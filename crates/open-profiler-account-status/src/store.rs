use crate::model::{self, Account, Principal, Projection, StatusWrite};
use sqlx::any::{AnyPoolOptions, AnyRow};
use sqlx::{AnyConnection, AnyPool, ConnectOptions, Row};
use std::{borrow::Cow, sync::Once};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    Postgres,
    MariaDb,
    MySql,
}

impl Engine {
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "postgres" => Self::Postgres,
            "mariadb" => Self::MariaDb,
            "mysql" => Self::MySql,
            _ => return None,
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Fault {
    #[error("invalid request")]
    Invalid,
    #[error("account not found")]
    NotFound,
    #[error("scope forbidden")]
    Scope,
    #[error("action forbidden")]
    Action,
    #[error("stale revision")]
    Stale(Box<Account>),
    #[error("request ID conflict")]
    Conflict,
    #[error("projection revision conflict")]
    ProjectionConflict,
    #[error("authority unavailable")]
    Unavailable,
}

impl From<sqlx::Error> for Fault {
    fn from(_: sqlx::Error) -> Self {
        Self::Unavailable
    }
}

#[derive(Clone)]
pub struct Store {
    engine: Engine,
    pool: AnyPool,
}

#[derive(Debug, serde::Serialize)]
pub struct WriteResult {
    pub account: Account,
    pub outcome: &'static str,
    pub accepted_revision: String,
}

#[derive(Debug, serde::Serialize)]
pub struct AccountPage {
    pub accounts: Vec<Account>,
    pub next_cursor: Option<String>,
}

impl Store {
    pub async fn connect(engine: Engine, url: &str, local_fixture: bool) -> Result<Self, Fault> {
        let mut url = reqwest::Url::parse(url).map_err(|_| Fault::Invalid)?;
        if !matches!(
            (engine, url.scheme()),
            (Engine::Postgres, "postgres" | "postgresql")
                | (Engine::MariaDb | Engine::MySql, "mysql")
        ) || url.host_str().is_none()
            || url
                .query_pairs()
                .any(|(key, _)| matches!(key.as_ref(), "ssl-mode" | "sslmode" | "socket" | "host"))
        {
            return Err(Fault::Invalid);
        }
        if local_fixture && !matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]")) {
            return Err(Fault::Invalid);
        }
        let tls = match (engine, local_fixture) {
            (Engine::Postgres, true) => ("sslmode", "disable"),
            (Engine::Postgres, false) => ("sslmode", "verify-full"),
            (_, true) => ("ssl-mode", "DISABLED"),
            (_, false) => ("ssl-mode", "VERIFY_IDENTITY"),
        };
        url.query_pairs_mut().append_pair(tls.0, tls.1);
        static INSTALL: Once = Once::new();
        INSTALL.call_once(sqlx::any::install_default_drivers);
        let options: sqlx::any::AnyConnectOptions = url.as_str().parse()?;
        let pool = AnyPoolOptions::new()
            .max_connections(12)
            .acquire_timeout(std::time::Duration::from_secs(3))
            .after_connect(move |conn, _| {
                Box::pin(async move {
                    let statements = match engine {
                        Engine::Postgres => [
                            "SET SESSION CHARACTERISTICS AS TRANSACTION ISOLATION LEVEL READ COMMITTED",
                            "SET lock_timeout = '4s'",
                            "SET statement_timeout = '6s'",
                        ],
                        Engine::MariaDb => [
                            "SET SESSION TRANSACTION ISOLATION LEVEL READ COMMITTED",
                            "SET SESSION innodb_lock_wait_timeout = 4",
                            "SET SESSION max_statement_time = 6",
                        ],
                        Engine::MySql => [
                            "SET SESSION TRANSACTION ISOLATION LEVEL READ COMMITTED",
                            "SET SESSION innodb_lock_wait_timeout = 4",
                            "SET SESSION max_execution_time = 6000",
                        ],
                    };
                    for statement in statements {
                        sqlx::query(statement).execute(&mut *conn).await?;
                    }
                    Ok(())
                })
            })
            .connect_with(options.disable_statement_logging())
            .await?;
        let store = Self { engine, pool };
        let version: String = sqlx::query_scalar("SELECT VERSION()")
            .fetch_one(&store.pool)
            .await?;
        let vendor = version.to_ascii_lowercase();
        let matches = match engine {
            Engine::Postgres => vendor.starts_with("postgresql "),
            Engine::MariaDb => vendor.contains("mariadb"),
            Engine::MySql => !vendor.contains("mariadb") && !vendor.starts_with("postgresql "),
        };
        if !matches {
            return Err(Fault::Invalid);
        }
        store.writer().await?;
        Ok(store)
    }

    pub async fn migrate(&self) -> Result<(), Fault> {
        let migrations = match self.engine {
            Engine::Postgres => sqlx::migrate!("./migrations/postgres"),
            Engine::MariaDb | Engine::MySql => sqlx::migrate!("./migrations/mysql"),
        };
        migrations
            .run(&self.pool)
            .await
            .map_err(|_| Fault::Unavailable)?;
        Ok(())
    }

    pub async fn writer(&self) -> Result<(), Fault> {
        let mut connection = self.pool.acquire().await?;
        self.check_writer(&mut connection).await
    }

    async fn check_writer(&self, connection: &mut AnyConnection) -> Result<(), Fault> {
        let sql = match self.engine {
            Engine::Postgres => "SELECT CAST(CASE WHEN pg_is_in_recovery() OR current_setting('transaction_read_only') = 'on' THEN 0 ELSE 1 END AS BIGINT)",
            Engine::MariaDb => "SELECT CAST(CASE WHEN @@global.read_only = 0 AND @@session.tx_read_only = 0 THEN 1 ELSE 0 END AS SIGNED)",
            Engine::MySql => "SELECT CAST(CASE WHEN @@global.read_only = 0 AND @@session.transaction_read_only = 0 THEN 1 ELSE 0 END AS SIGNED)",
        };
        if sqlx::query_scalar::<_, i64>(sql)
            .fetch_one(connection)
            .await?
            != 1
        {
            return Err(Fault::Unavailable);
        }
        Ok(())
    }

    fn sql<'a>(&self, query: &'a str) -> Cow<'a, str> {
        if self.engine != Engine::Postgres || !query.contains('?') {
            return Cow::Borrowed(query);
        }
        let mut text = String::new();
        for (i, part) in query.split('?').enumerate() {
            if i != 0 {
                text.push_str(&format!("${i}"));
            }
            text.push_str(part);
        }
        Cow::Owned(text)
    }

    pub async fn publish(&self, company: &str, projection: Projection) -> Result<(), Fault> {
        if !model::key(company) || !projection.valid(model::now()) {
            return Err(Fault::Invalid);
        }
        let snapshot = serde_json::to_string(&projection).map_err(|_| Fault::Invalid)?;
        let mut tx = self.pool.begin().await?;
        self.check_writer(&mut tx).await?;
        let insert = match self.engine {
            Engine::Postgres => "INSERT INTO company_projection VALUES($1,0,0,'') ON CONFLICT(company_id) DO NOTHING",
            _ => "INSERT INTO company_projection VALUES(?,0,0,'') ON DUPLICATE KEY UPDATE company_id=company_id",
        };
        sqlx::query(insert).bind(company).execute(&mut *tx).await?;
        let statement = self.sql(
            "SELECT generation, snapshot FROM company_projection WHERE company_id=? FOR UPDATE",
        );
        let row = sqlx::query(&statement)
            .bind(company)
            .fetch_one(&mut *tx)
            .await?;
        let generation: i64 = row.try_get("generation")?;
        if generation >= projection.generation {
            if generation == projection.generation && snapshot_value(&row)? == snapshot {
                tx.commit().await?;
                return Ok(());
            }
            return Err(Fault::ProjectionConflict);
        }
        let insert_account = match self.engine {
            Engine::Postgres => "INSERT INTO company_account VALUES($1,$2,$3,NULL,NULL) ON CONFLICT(company_id,provider,account_ref) DO NOTHING",
            _ => "INSERT INTO company_account VALUES(?,?,?,NULL,NULL) ON DUPLICATE KEY UPDATE account_ref=account_ref",
        };
        for entry in &projection.accounts {
            sqlx::query(insert_account)
                .bind(company)
                .bind(&entry.account.provider)
                .bind(&entry.account.account_ref)
                .execute(&mut *tx)
                .await?;
        }
        let statement = self.sql("UPDATE company_projection SET generation=?, expires_at=?, snapshot=? WHERE company_id=?");
        sqlx::query(&statement)
            .bind(projection.generation)
            .bind(projection.expires_at)
            .bind(&snapshot)
            .bind(company)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Extend the active lease only while its published grants match the deployment policy.
    pub async fn renew_approved(
        &self,
        company: &str,
        approved: &Projection,
    ) -> Result<Option<(i64, i64)>, Fault> {
        let now = model::now();
        let mut tx = self.pool.begin().await?;
        self.check_writer(&mut tx).await?;
        let select = self.sql(
            "SELECT generation, expires_at, snapshot FROM company_projection WHERE company_id=? FOR UPDATE",
        );
        let row = sqlx::query(&select)
            .bind(company)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(Fault::ProjectionConflict)?;
        let mut current: Projection =
            serde_json::from_str(&snapshot_value(&row)?).map_err(|_| Fault::Unavailable)?;
        if current.generation != row.try_get::<i64, _>("generation")?
            || current.expires_at != row.try_get::<i64, _>("expires_at")?
            || !current.valid(now)
            || current.accounts != approved.accounts
            || current.grants != approved.grants
            || current.members != approved.members
        {
            return Err(Fault::ProjectionConflict);
        }
        let expires_at = now + 85_500;
        if current.expires_at >= expires_at {
            return Ok(None);
        }
        current.generation = current
            .generation
            .checked_add(1)
            .ok_or(Fault::ProjectionConflict)?;
        current.expires_at = expires_at;
        let snapshot = serde_json::to_string(&current).map_err(|_| Fault::Unavailable)?;
        let update = self.sql(
            "UPDATE company_projection SET generation=?, expires_at=?, snapshot=? WHERE company_id=?",
        );
        sqlx::query(&update)
            .bind(current.generation)
            .bind(expires_at)
            .bind(snapshot)
            .bind(company)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(Some((current.generation, expires_at)))
    }

    pub async fn list(
        &self,
        company: &str,
        principal: &Principal,
        provider: Option<&str>,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<AccountPage, Fault> {
        let mut tx = self.pool.begin().await?;
        self.check_writer(&mut tx).await?;
        // Hold the projection stable while reading the selected status rows.
        // Status writes and projection changes both lock this row first.
        let lock = match self.engine {
            Engine::Postgres => "SELECT expires_at,snapshot FROM company_projection WHERE company_id=? FOR SHARE",
            _ => "SELECT expires_at,snapshot FROM company_projection WHERE company_id=? LOCK IN SHARE MODE",
        };
        let row = sqlx::query(&self.sql(lock))
            .bind(company)
            .fetch_optional(&mut *tx)
            .await?;
        let scope = authorized(row.as_ref(), principal)?;
        let mut selected: Vec<_> = scope
            .grants
            .iter()
            .filter(|grant| &grant.principal == principal)
            .filter(|grant| provider.is_none_or(|value| value == grant.account.provider))
            .map(|grant| {
                let key = &grant.account;
                (format!("{}/{}", key.provider, key.account_ref), key)
            })
            .filter(|(token, _)| cursor.is_none_or(|cursor| token.as_str() > cursor))
            .collect();
        selected.sort_unstable_by(|left, right| left.0.cmp(&right.0));
        let more = selected.len() > limit;
        selected.truncate(limit);
        let mut accounts = Vec::with_capacity(selected.len());
        if !selected.is_empty() {
            let pairs = vec!["(?,?)"; selected.len()].join(",");
            let query = format!(
                "SELECT provider,account_ref,status,revision FROM company_account
                 WHERE company_id=? AND (provider,account_ref) IN ({pairs})"
            );
            let statement = self.sql(&query);
            let mut query = sqlx::query(&statement).bind(company);
            for (_, key) in &selected {
                query = query.bind(&key.provider).bind(&key.account_ref);
            }
            let rows = query.fetch_all(&mut *tx).await?;
            for row in &rows {
                let provider: String = row.try_get("provider")?;
                let reference: String = row.try_get("account_ref")?;
                accounts.push(account(row, &scope, company, &provider, &reference)?);
            }
            if accounts.len() != selected.len() {
                return Err(Fault::Unavailable);
            }
            accounts.sort_by_cached_key(|a| format!("{}/{}", a.provider, a.account_ref));
        }
        tx.commit().await?;
        let next_cursor = if more {
            selected.last().map(|(token, _)| token.clone())
        } else {
            None
        };
        Ok(AccountPage {
            accounts,
            next_cursor,
        })
    }

    pub async fn account(
        &self,
        company: &str,
        provider: &str,
        reference: &str,
        principal: &Principal,
        use_account: bool,
    ) -> Result<Account, Fault> {
        let mut connection = self.pool.acquire().await?;
        self.check_writer(&mut connection).await?;
        let statement = self.sql(
            "SELECT p.expires_at,p.snapshot,a.account_ref,a.status,a.revision
            FROM company_projection p LEFT JOIN company_account a
            ON a.company_id=p.company_id AND a.provider=? AND a.account_ref=? WHERE p.company_id=?",
        );
        let row = sqlx::query(&statement)
            .bind(provider)
            .bind(reference)
            .bind(company)
            .fetch_optional(&mut *connection)
            .await?;
        let scope = authorized(row.as_ref(), principal)?;
        let grant = scope
            .grant(principal, provider, reference)
            .ok_or(Fault::NotFound)?;
        if use_account && !grant.use_account {
            return Err(Fault::Action);
        }
        let row = row.ok_or(Fault::NotFound)?;
        if row.try_get::<Option<String>, _>("account_ref")?.is_none() {
            return Err(Fault::NotFound);
        }
        account(&row, &scope, company, provider, reference)
    }

    pub async fn write(
        &self,
        company: &str,
        provider: &str,
        reference: &str,
        principal: &Principal,
        request: StatusWrite,
    ) -> Result<WriteResult, Fault> {
        let expected = request
            .expected_revision
            .as_deref()
            .map(|value| {
                let number = value.parse::<i64>().map_err(|_| Fault::Invalid)?;
                if number < 1 || number.to_string() != value {
                    return Err(Fault::Invalid);
                }
                Ok(number)
            })
            .transpose()?;
        let desired = request.desired_status.name();
        let request_id = request.request_id.to_string();
        let actor = principal.fingerprint();
        let mut tx = self.pool.begin().await?;
        self.check_writer(&mut tx).await?;
        let statement = self.sql(
            "SELECT expires_at,snapshot FROM company_projection WHERE company_id=? FOR UPDATE",
        );
        let row = sqlx::query(&statement)
            .bind(company)
            .fetch_optional(&mut *tx)
            .await?;
        let scope = authorized(row.as_ref(), principal)?;
        let grant = scope
            .grant(principal, provider, reference)
            .ok_or(Fault::NotFound)?;
        if !grant.manage || grant.workload.is_some() {
            return Err(Fault::Action);
        }
        let statement = self.sql("SELECT status,revision FROM company_account WHERE company_id=? AND provider=? AND account_ref=? FOR UPDATE");
        let row = sqlx::query(&statement)
            .bind(company)
            .bind(provider)
            .bind(reference)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(Fault::NotFound)?;
        let mut current = account(&row, &scope, company, provider, reference)?;
        let old_status: Option<String> = row.try_get("status")?;
        let revision: Option<i64> = row.try_get("revision")?;
        let statement = self.sql(
            "SELECT desired_status,expected_revision,accepted_revision FROM accepted_status_write
            WHERE company_id=? AND provider=? AND account_ref=? AND actor=? AND request_id=?",
        );
        if let Some(prior) = sqlx::query(&statement)
            .bind(company)
            .bind(provider)
            .bind(reference)
            .bind(&actor)
            .bind(&request_id)
            .fetch_optional(&mut *tx)
            .await?
        {
            let same = prior.try_get::<String, _>("desired_status")? == desired
                && prior.try_get::<Option<i64>, _>("expected_revision")? == expected;
            if !same {
                return Err(Fault::Conflict);
            }
            let accepted: i64 = prior.try_get("accepted_revision")?;
            tx.commit().await?;
            return Ok(WriteResult {
                account: current,
                outcome: "replayed",
                accepted_revision: accepted.to_string(),
            });
        }
        if revision != expected {
            return Err(Fault::Stale(Box::new(current)));
        }
        let outcome;
        let accepted;
        if let Some(existing) = revision.filter(|_| old_status.as_deref() == Some(desired)) {
            outcome = "noop";
            accepted = existing;
        } else {
            outcome = "applied";
            accepted = revision
                .unwrap_or(0)
                .checked_add(1)
                .ok_or(Fault::Unavailable)?;
            let statement = self.sql("UPDATE company_account SET status=?,revision=? WHERE company_id=? AND provider=? AND account_ref=?");
            sqlx::query(&statement)
                .bind(desired)
                .bind(accepted)
                .bind(company)
                .bind(provider)
                .bind(reference)
                .execute(&mut *tx)
                .await?;
            let statement = self.sql("INSERT INTO status_transition VALUES(?,?,?,?,?,?,?,?,?)");
            sqlx::query(&statement)
                .bind(company)
                .bind(provider)
                .bind(reference)
                .bind(accepted)
                .bind(&actor)
                .bind(old_status.as_deref())
                .bind(desired)
                .bind(model::now())
                .bind(&request_id)
                .execute(&mut *tx)
                .await?;
            current.status = desired.into();
            current.revision = Some(accepted.to_string());
        }
        let statement = self.sql("INSERT INTO accepted_status_write VALUES(?,?,?,?,?,?,?,?)");
        sqlx::query(&statement)
            .bind(company)
            .bind(provider)
            .bind(reference)
            .bind(&actor)
            .bind(&request_id)
            .bind(desired)
            .bind(expected)
            .bind(accepted)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(WriteResult {
            account: current,
            outcome,
            accepted_revision: accepted.to_string(),
        })
    }
}

fn snapshot_value(row: &AnyRow) -> Result<String, Fault> {
    if let Ok(value) = row.try_get::<String, _>("snapshot") {
        return Ok(value);
    }
    String::from_utf8(row.try_get::<Vec<u8>, _>("snapshot")?).map_err(|_| Fault::Unavailable)
}

fn authorized(row: Option<&AnyRow>, principal: &Principal) -> Result<Projection, Fault> {
    let row = row.ok_or(Fault::Scope)?;
    let scope = serde_json::from_str::<Projection>(&snapshot_value(row)?)
        .map_err(|_| Fault::Unavailable)?;
    if !scope.members.contains(principal) {
        return Err(Fault::Scope);
    }
    if row.try_get::<i64, _>("expires_at")? <= model::now() {
        return Err(Fault::Unavailable);
    }
    Ok(scope)
}

fn account(
    row: &AnyRow,
    scope: &Projection,
    company: &str,
    provider: &str,
    reference: &str,
) -> Result<Account, Fault> {
    let entry = scope
        .accounts
        .iter()
        .find(|a| a.account.provider == provider && a.account.account_ref == reference)
        .ok_or(Fault::NotFound)?;
    let revision = row
        .try_get::<Option<i64>, _>("revision")?
        .map(|v| v.to_string());
    let status = row
        .try_get::<Option<String>, _>("status")?
        .unwrap_or_else(|| "unknown".into());
    Ok(Account {
        company_id: company.into(),
        provider: provider.into(),
        account_ref: reference.into(),
        status,
        revision,
        display_name: entry.display_name.clone(),
    })
}

pub fn correlation() -> String {
    Uuid::new_v4().to_string()
}
