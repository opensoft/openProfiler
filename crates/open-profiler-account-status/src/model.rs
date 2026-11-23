use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use uuid::Uuid;

pub fn key(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        && value != "."
        && value != ".."
}

pub fn provider(value: &str) -> bool {
    value.len() <= 32 && key(value)
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Principal {
    pub subject: String,
    pub client_id: String,
}

impl Principal {
    pub fn fingerprint(&self) -> String {
        let mut digest = Sha256::new();
        digest.update((self.subject.len() as u64).to_be_bytes());
        digest.update(self.subject.as_bytes());
        digest.update(self.client_id.as_bytes());
        format!("{:x}", digest.finalize())
    }

    pub fn valid(&self) -> bool {
        !self.subject.is_empty()
            && self.subject.len() <= 256
            && !self.client_id.is_empty()
            && self.client_id.len() <= 256
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AccountKey {
    pub provider: String,
    pub account_ref: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Registration {
    pub account: AccountKey,
    pub display_name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkloadScope {
    pub environment: String,
    pub factory_instance: String,
    pub role: String,
    pub host: Option<String>,
    pub persona: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Grant {
    pub principal: Principal,
    pub account: AccountKey,
    pub use_account: bool,
    pub manage: bool,
    pub workload: Option<WorkloadScope>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Projection {
    pub generation: i64,
    pub expires_at: i64,
    pub accounts: Vec<Registration>,
    pub grants: Vec<Grant>,
    pub members: Vec<Principal>,
}

impl Projection {
    pub fn valid(&self, now: i64) -> bool {
        if self.generation < 1
            || self.expires_at <= now
            || self.expires_at > now + 86_400
            || self.accounts.len() > 2_000
            || self.grants.len() > 8_000
        {
            return false;
        }
        let members: BTreeSet<_> = self
            .members
            .iter()
            .filter(|p| p.valid())
            .map(Principal::fingerprint)
            .collect();
        if members.len() != self.members.len() || members.len() > 8_000 {
            return false;
        }
        let mut accounts = BTreeSet::new();
        for entry in &self.accounts {
            if !provider(&entry.account.provider)
                || !key(&entry.account.account_ref)
                || entry
                    .display_name
                    .as_ref()
                    .is_some_and(|name| name.len() > 128 || name.chars().any(char::is_control))
                || !accounts.insert(&entry.account)
            {
                return false;
            }
        }
        let mut scopes = std::collections::BTreeMap::new();
        let mut bindings = BTreeSet::new();
        self.grants.iter().all(|grant| {
            let actor = grant.principal.fingerprint();
            if !members.contains(&actor)
                || !accounts.contains(&grant.account)
                || !bindings.insert((actor.clone(), &grant.account))
            {
                return false;
            }
            if let Some(scope) = &grant.workload {
                if grant.manage
                    || !key(&scope.environment)
                    || !key(&scope.factory_instance)
                    || !key(&scope.role)
                    || scope.host.as_deref().is_some_and(|v| !key(v))
                    || scope.persona.as_deref().is_some_and(|v| !key(v))
                {
                    return false;
                }
            }
            let scope = grant.workload.as_ref();
            scopes.entry(actor).or_insert(scope) == &scope
        })
    }

    pub fn grant(
        &self,
        principal: &Principal,
        provider: &str,
        account_ref: &str,
    ) -> Option<&Grant> {
        self.grants.iter().find(|g| {
            g.principal == *principal
                && g.account.provider == provider
                && g.account.account_ref == account_ref
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DesiredStatus {
    Active,
    Paused,
}

impl DesiredStatus {
    pub fn name(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatusWrite {
    pub desired_status: DesiredStatus,
    pub expected_revision: Option<String>,
    pub request_id: Uuid,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    Activation,
    DesktopSwitch,
    DesktopUndo,
    Job,
    Turn,
    AccountReselect,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdmissionRequest {
    pub operation_id: Uuid,
    pub operation_kind: OperationKind,
}

#[derive(Debug, Serialize)]
pub struct Account {
    pub company_id: String,
    pub provider: String,
    pub account_ref: String,
    pub status: String,
    pub revision: Option<String>,
    pub display_name: Option<String>,
}
