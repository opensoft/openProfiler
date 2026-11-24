use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
#[cfg(any(windows, test))]
use opensoft_open_profiler_core::codex_profile_paths_for_account;
use opensoft_open_profiler_core::{profile_policy_path, DiscoveryConfig, ProfileError, Provider};
use reqwest::{blocking::Client, StatusCode, Url};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    io::{self, BufRead, BufReader, Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    sync::Mutex,
    thread,
    time::{Duration, Instant},
};
use tauri::{Manager, State};
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    company_id: String,
    service_origin: String,
    authorization_endpoint: String,
    token_endpoint: String,
    client_id: String,
    #[serde(default)]
    profiles: Vec<Binding>,
    #[serde(default)]
    managed_roots: Vec<ManagedRoot>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Binding {
    provider: Provider,
    path: String,
    account_ref: String,
    codex_account_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ManagedRoot {
    provider: Provider,
    path: String,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ConfigFile {
    Single(Config),
    Catalog(CompanyCatalog),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CompanyCatalog {
    companies: Vec<Config>,
}

#[derive(Default)]
struct Session {
    company: usize,
    generation: u64,
    token: Option<String>,
    sign_in_url: Option<Url>,
}

pub(crate) struct CompanyState {
    settings: Result<Vec<Config>, String>,
    session: Mutex<Session>,
    pub activation: Mutex<()>,
}

impl CompanyState {
    pub fn from_env(local_data: &Path) -> Self {
        Self {
            settings: load_settings(
                env::var_os("OPENPROFILER_COMPANY_CONFIG").map(PathBuf::from),
                local_data,
            ),
            session: Mutex::default(),
            activation: Mutex::new(()),
        }
    }

    fn configs(&self) -> Result<&[Config], String> {
        self.settings.as_deref().map_err(Clone::clone)
    }

    fn config(&self) -> Result<Option<&Config>, String> {
        let index = self
            .session
            .lock()
            .map_err(|_| "Company session unavailable")?
            .company;
        Ok(self.configs()?.get(index))
    }

    fn select(&self, company_id: &str) -> Result<(), String> {
        let index = self
            .configs()?
            .iter()
            .position(|entry| entry.company_id == company_id)
            .ok_or("The selected company is not configured")?;
        let mut session = self
            .session
            .lock()
            .map_err(|_| "Company session unavailable")?;
        if session.company != index {
            session.company = index;
            session.generation += 1;
            session.token = None;
            session.sign_in_url = None;
        }
        Ok(())
    }

    pub fn authorize(
        &self,
        discovery: &DiscoveryConfig,
        provider: Provider,
        path: &str,
        codex_id: Option<&str>,
        kind: &str,
    ) -> Result<String, String> {
        let configs = self.configs()?;
        let resolved =
            profile_policy_path(discovery, provider, path).map_err(|error| error.to_string())?;
        let mut association = None;
        for config in configs {
            let managed = config.manages_source(discovery, provider, path, &resolved)?;
            if managed && !config.matches_managed_source(discovery, provider, path, &resolved) {
                return Err(
                    "A company-managed profile must be selected by its enrolled path".into(),
                );
            }
            if let Some(binding) = config.binding(provider, path, codex_id, managed)? {
                association = Some((config, binding));
            }
        }
        if let Some((config, binding)) = association {
            self.check(config, provider, &binding.account_ref, kind)?;
        }
        Ok(resolved)
    }

    #[cfg(windows)]
    pub fn authorize_undo(
        &self,
        discovery: &DiscoveryConfig,
        codex_id: Option<&str>,
    ) -> Result<(), String> {
        let configs = self.configs()?;
        let Some(codex_id) = codex_id else {
            return Ok(());
        };
        for config in configs {
            if let Some(binding) = config.profiles.iter().find(|binding| {
                binding.provider == Provider::Codex
                    && binding.codex_account_id.as_deref() == Some(codex_id)
            }) {
                return self.check(
                    config,
                    Provider::Codex,
                    &binding.account_ref,
                    "desktop_undo",
                );
            }
        }
        verify_local_undo(configs, discovery, codex_id)
    }

    fn bearer(&self, company_id: &str) -> Result<(u64, String), String> {
        let session = self
            .session
            .lock()
            .map_err(|_| "Company session unavailable")?;
        if !self
            .configs()?
            .get(session.company)
            .is_some_and(|entry| entry.company_id == company_id)
        {
            return Err("Select the company that owns the account and sign in".into());
        }
        let token = session
            .token
            .clone()
            .ok_or("Sign in to your company before using this account")?;
        Ok((session.generation, token))
    }

    fn verify_session(&self, generation: u64, status: StatusCode) -> Result<(), String> {
        let mut session = self
            .session
            .lock()
            .map_err(|_| "Company session unavailable")?;
        if session.generation != generation {
            return Err("The company session changed".into());
        }
        if status == StatusCode::UNAUTHORIZED {
            session.generation += 1;
            session.token = None;
            session.sign_in_url = None;
            return Err("Company session expired. Sign in again".into());
        }
        Ok(())
    }

    fn check(
        &self,
        config: &Config,
        provider: Provider,
        key: &str,
        kind: &str,
    ) -> Result<(), String> {
        let (generation, token) = self.bearer(&config.company_id)?;
        let id = Uuid::new_v4();
        let route = format!(
            "{}/{}/{key}/admissions",
            config.accounts_url(),
            provider_name(provider)
        );
        let response = client()?
            .post(route)
            .bearer_auth(token)
            .json(&serde_json::json!({"operation_id":id,"operation_kind":kind}))
            .send()
            .map_err(|_| "Company status is unavailable")?;
        self.verify_session(generation, response.status())?;
        if !response.status().is_success() {
            return Err("Company account eligibility could not be verified".into());
        }
        let response: Admission = decode(response)?;
        self.verify_session(generation, StatusCode::OK)?;
        if response.operation_id != id
            || response.operation_kind != kind
            || response.account.company_id != config.company_id
            || response.account.provider != provider_name(provider)
            || response.account.account_ref != key
        {
            return Err("Company status returned a mismatched decision".into());
        }
        if response.decision == "allow"
            && response.account.status == "active"
            && response
                .account
                .revision
                .as_ref()
                .is_some_and(|v| revision(v))
        {
            Ok(())
        } else if response.decision == "paused" {
            Err("This company account is paused".into())
        } else {
            Err("This company account is unavailable for new work".into())
        }
    }
}

fn load_settings(explicit: Option<PathBuf>, local_data: &Path) -> Result<Vec<Config>, String> {
    let enrollment = local_data.join("company-enrolled");
    let config = if let Some(path) = explicit {
        path
    } else {
        let default = local_data.join("company-accounts.json");
        if !path_present(&default)? {
            return if path_present(&enrollment)? {
                Err("Company configuration unavailable for this enrolled installation".into())
            } else {
                Ok(Vec::new())
            };
        }
        default
    };

    fs::create_dir_all(local_data).map_err(|_| "Company enrollment could not be saved")?;
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(enrollment)
    {
        Ok(marker) => marker
            .sync_all()
            .map_err(|_| "Company enrollment could not be saved")?,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (),
        Err(_) => return Err("Company enrollment could not be saved".into()),
    }

    let mut bytes = Vec::new();
    fs::File::open(config)
        .map_err(|_| "Company configuration unavailable")?
        .take(256 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Company configuration unavailable")?;
    if bytes.len() > 256 * 1024 {
        return Err("Company configuration is too large".into());
    }
    parse_configs(&bytes)
}

fn path_present(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err("Company enrollment could not be read".into()),
    }
}

#[cfg(any(windows, test))]
fn verify_local_undo(
    configs: &[Config],
    discovery: &DiscoveryConfig,
    codex_id: &str,
) -> Result<(), String> {
    if !configs.iter().any(|config| {
        config
            .managed_roots
            .iter()
            .any(|root| root.provider == Provider::Codex)
    }) {
        return Ok(());
    }

    let error = "The previous GPT login has no verified company account association";
    let matches = codex_profile_paths_for_account(discovery, codex_id);
    if matches.is_empty() {
        return Err(error.into());
    }
    for (path, resolved) in matches {
        for config in configs {
            if config.manages_source(discovery, Provider::Codex, &path, &resolved)? {
                return Err(error.into());
            }
        }
    }
    Ok(())
}

fn parse_configs(data: &[u8]) -> Result<Vec<Config>, String> {
    let entries = match serde_json::from_slice(data).map_err(|_| "Invalid company configuration")? {
        ConfigFile::Single(config) => vec![config],
        ConfigFile::Catalog(catalog) => catalog.companies,
    };
    if entries.is_empty() || entries.len() > 32 {
        return Err("Invalid company directory size".into());
    }
    for (index, entry) in entries.iter().enumerate() {
        entry.validate()?;
        for prior in &entries[..index] {
            let overlap = entry.managed_roots.iter().any(|left| {
                prior.managed_roots.iter().any(|right| {
                    left.matches(right.provider, &right.path)
                        || right.matches(left.provider, &left.path)
                })
            });
            let shared_codex = entry
                .profiles
                .iter()
                .filter_map(|b| b.codex_account_id.as_ref())
                .any(|id| {
                    prior
                        .profiles
                        .iter()
                        .any(|b| b.codex_account_id.as_ref() == Some(id))
                });
            if entry.company_id == prior.company_id || overlap || shared_codex {
                return Err("Company configurations have overlapping account associations".into());
            }
        }
    }
    Ok(entries)
}

impl Config {
    fn validate(&self) -> Result<(), String> {
        let service = endpoint(&self.service_origin)?;
        let authorize = endpoint(&self.authorization_endpoint)?;
        let token = endpoint(&self.token_endpoint)?;
        if service.path() != "/"
            || service.query().is_some()
            || authorize.origin() != token.origin()
            || authorize.query().is_some()
            || token.query().is_some()
            || !identifier(&self.company_id)
            || self.client_id.is_empty()
            || self.client_id.len() > 128
        {
            return Err("Invalid company service or identity configuration".into());
        }
        for (index, binding) in self.profiles.iter().enumerate() {
            if !identifier(&binding.account_ref)
                || !profile_path(&binding.path)
                || !self
                    .managed_roots
                    .iter()
                    .any(|root| root.matches(binding.provider, &binding.path))
                || (binding.provider == Provider::Codex
                    && binding
                        .codex_account_id
                        .as_deref()
                        .is_none_or(str::is_empty))
                || self.profiles[..index].iter().any(|prior| {
                    prior.provider == binding.provider
                        && (same_path(&prior.path, &binding.path)
                            || (binding.provider == Provider::Codex
                                && prior.codex_account_id == binding.codex_account_id
                                && prior.account_ref != binding.account_ref))
                })
            {
                return Err("Invalid or ambiguous company profile binding".into());
            }
        }
        if self
            .managed_roots
            .iter()
            .any(|root| !profile_path(&root.path))
        {
            return Err("Invalid company-managed profile root".into());
        }
        Ok(())
    }

    fn manages_source(
        &self,
        discovery: &DiscoveryConfig,
        provider: Provider,
        path: &str,
        resolved: &str,
    ) -> Result<bool, String> {
        for root in self
            .managed_roots
            .iter()
            .filter(|root| root.provider == provider)
        {
            if root.matches(provider, path) || root.matches(provider, resolved) {
                return Ok(true);
            }
            // A declared root can itself be a symlink or junction. Its real
            // subtree is also managed, including when selected by a local path.
            match profile_policy_path(discovery, provider, &root.path) {
                Ok(real_root) if is_inside(resolved, &real_root) => return Ok(true),
                Ok(_) => (),
                Err(ProfileError::Read { source, .. })
                    if source.kind() == io::ErrorKind::NotFound => {}
                Err(_) => return Err("Company-managed profile root could not be verified".into()),
            }
        }
        Ok(false)
    }

    fn matches_managed_source(
        &self,
        discovery: &DiscoveryConfig,
        provider: Provider,
        path: &str,
        resolved: &str,
    ) -> bool {
        if same_path(path, resolved) {
            return true;
        }
        self.managed_roots.iter().any(|root| {
            if !root.matches(provider, path) {
                return false;
            }
            let Ok(real_root) = profile_policy_path(discovery, provider, &root.path) else {
                return false;
            };
            // A declared root may be an alias. The path below it still has to
            // identify the same profile after resolving that root.
            let suffix: PathBuf = Path::new(path)
                .components()
                .skip(Path::new(&root.path).components().count())
                .collect();
            Path::new(&real_root)
                .join(suffix)
                .to_str()
                .is_some_and(|expected| same_path(expected, resolved))
        })
    }

    fn binding(
        &self,
        provider: Provider,
        path: &str,
        codex_id: Option<&str>,
        managed: bool,
    ) -> Result<Option<&Binding>, String> {
        if let Some(binding) = self
            .profiles
            .iter()
            .find(|item| item.provider == provider && same_path(&item.path, path))
        {
            if provider == Provider::Codex && binding.codex_account_id.as_deref() != codex_id {
                return Err(
                    "The local Codex login does not match the enrolled company account".into(),
                );
            }
            return Ok(Some(binding));
        }
        if managed {
            return Err("Company-managed profile has no verified account association".into());
        }
        if provider == Provider::Codex {
            return Ok(self.profiles.iter().find(|item| {
                item.provider == provider
                    && codex_id.is_some()
                    && item.codex_account_id.as_deref() == codex_id
            }));
        }
        Ok(None)
    }

    fn accounts_url(&self) -> String {
        format!(
            "{}/v1/companies/{}/accounts",
            self.service_origin.trim_end_matches('/'),
            self.company_id
        )
    }
}

impl ManagedRoot {
    fn matches(&self, provider: Provider, profile: &str) -> bool {
        self.provider == provider && is_inside(profile, &self.path)
    }
}

fn is_inside(profile: &str, root: &str) -> bool {
    let mut profile = Path::new(profile).components();
    Path::new(root).components().all(|part| {
        profile
            .next()
            .is_some_and(|value| same_component(part, value))
    })
}

fn same_path(left: &str, right: &str) -> bool {
    let mut right = Path::new(right).components();
    Path::new(left).components().all(|part| {
        right
            .next()
            .is_some_and(|value| same_component(part, value))
    }) && right.next().is_none()
}

fn same_component(left: std::path::Component<'_>, right: std::path::Component<'_>) -> bool {
    if cfg!(windows) {
        left.as_os_str().eq_ignore_ascii_case(right.as_os_str())
    } else {
        left == right
    }
}

fn profile_path(value: &str) -> bool {
    !value.is_empty() && value.len() <= 400 && value.split('/').all(identifier)
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}
fn revision(value: &str) -> bool {
    value
        .parse::<i64>()
        .is_ok_and(|n| n > 0 && n.to_string() == value)
}
fn provider_name(provider: Provider) -> &'static str {
    match provider {
        Provider::Codex => "codex",
        Provider::Claude => "claude",
    }
}
fn endpoint(value: &str) -> Result<Url, String> {
    let url = Url::parse(value).map_err(|_| "Invalid company endpoint")?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err("Company endpoint must be HTTPS".into());
    }
    Ok(url)
}
fn client() -> Result<Client, String> {
    Client::builder()
        .timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Company network client unavailable".into())
}

fn decode<T: serde::de::DeserializeOwned>(
    response: reqwest::blocking::Response,
) -> Result<T, String> {
    let mut data = Vec::new();
    response
        .take(256 * 1024 + 1)
        .read_to_end(&mut data)
        .map_err(|_| "Company response unavailable")?;
    if data.len() > 256 * 1024 {
        return Err("Company response exceeds its limit".into());
    }
    serde_json::from_slice(&data).map_err(|_| "Invalid company response".into())
}

#[derive(Deserialize, Serialize)]
struct Account {
    company_id: String,
    provider: String,
    account_ref: String,
    status: String,
    revision: Option<String>,
    display_name: Option<String>,
}
#[derive(Deserialize)]
struct Admission {
    account: Account,
    operation_id: Uuid,
    operation_kind: String,
    decision: String,
}
#[derive(Deserialize)]
struct Page {
    accounts: Vec<Account>,
    next_cursor: Option<String>,
}
#[derive(Serialize)]
pub(crate) struct CompanyView {
    company_id: Option<String>,
    companies: Vec<String>,
    signed_in: bool,
    accounts: Vec<Account>,
    bindings: Vec<BindingView>,
}
#[derive(Serialize)]
struct BindingView {
    provider: Provider,
    path: String,
    account_ref: String,
}

#[tauri::command]
pub(crate) async fn company_accounts(app: tauri::AppHandle) -> Result<CompanyView, String> {
    tauri::async_runtime::spawn_blocking(move || accounts(&app.state::<CompanyState>()))
        .await
        .map_err(|_| "Company inventory unavailable".to_string())?
}

fn accounts(state: &CompanyState) -> Result<CompanyView, String> {
    let config = state.config()?;
    let mut view = CompanyView {
        company_id: config.map(|config| config.company_id.clone()),
        companies: state
            .configs()?
            .iter()
            .map(|entry| entry.company_id.clone())
            .collect(),
        signed_in: state
            .session
            .lock()
            .map_err(|_| "Company session unavailable")?
            .token
            .is_some(),
        accounts: Vec::new(),
        bindings: Vec::new(),
    };
    let Some(config) = config.filter(|_| view.signed_in) else {
        return Ok(view);
    };
    let (generation, token) = state.bearer(&config.company_id)?;
    let client = client()?;
    let mut cursor = None;
    for _ in 0..32 {
        let mut request = client.get(config.accounts_url()).bearer_auth(&token);
        if let Some(cursor) = &cursor {
            request = request.query(&[("cursor", cursor)]);
        }
        let response = request
            .send()
            .map_err(|_| "Company status is unavailable")?;
        state.verify_session(generation, response.status())?;
        if !response.status().is_success() {
            return Err("Company accounts are unavailable".into());
        }
        let page: Page = decode(response)?;
        if page.accounts.iter().any(|a| {
            a.company_id != config.company_id
                || !identifier(&a.account_ref)
                || !matches!(
                    (a.status.as_str(), a.revision.as_deref()),
                    ("unknown", None) | ("active" | "paused", Some(_))
                )
                || a.revision.as_ref().is_some_and(|v| !revision(v))
        }) {
            return Err("Company accounts returned an invalid record".into());
        }
        view.accounts.extend(page.accounts);
        if page.next_cursor.is_none() {
            state.verify_session(generation, StatusCode::OK)?;
            view.bindings = config
                .profiles
                .iter()
                .filter(|b| {
                    view.accounts.iter().any(|a| {
                        a.provider == provider_name(b.provider) && a.account_ref == b.account_ref
                    })
                })
                .map(|b| BindingView {
                    provider: b.provider,
                    path: b.path.clone(),
                    account_ref: b.account_ref.clone(),
                })
                .collect();
            return Ok(view);
        }
        if page.next_cursor == cursor {
            return Err("Company account pagination repeated a page".into());
        }
        cursor = page.next_cursor;
    }
    Err("Company account inventory exceeds its limit".into())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Edit {
    company_id: String,
    provider: String,
    account_ref: String,
    desired_status: String,
    expected_revision: Option<String>,
    request_id: Uuid,
}
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum EditOutcome {
    Updated,
    Unchanged,
    StaleRevision,
    RequestIdConflict,
}

#[derive(Serialize)]
pub(crate) struct EditResult {
    outcome: EditOutcome,
}

#[derive(Deserialize)]
struct WriteAcknowledgement {
    account: Account,
    outcome: String,
    accepted_revision: String,
}

#[derive(Deserialize)]
struct WriteConflict {
    code: String,
    account: Option<Account>,
}

const UNCERTAIN_WRITE: &str = "The save result is unavailable. Retry the same change to check it";

impl Edit {
    fn matches(&self, account: &Account) -> bool {
        self.company_id == account.company_id
            && self.provider == account.provider
            && self.account_ref == account.account_ref
    }

    fn acknowledgement(&self, ack: WriteAcknowledgement) -> Result<EditOutcome, String> {
        if !self.matches(&ack.account)
            || !matches!(ack.account.status.as_str(), "active" | "paused")
            || !revision(&ack.accepted_revision)
        {
            return Err(UNCERTAIN_WRITE.into());
        }
        let Some(current) = ack
            .account
            .revision
            .as_deref()
            .filter(|value| revision(value))
        else {
            return Err(UNCERTAIN_WRITE.into());
        };
        let accepted = Some(ack.accepted_revision.as_str());
        let expected = self.expected_revision.as_deref();
        let next_revision = expected
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or_default()
            .checked_add(1)
            .map(|value| value.to_string());
        let effect = if accepted == expected {
            EditOutcome::Unchanged
        } else if accepted == next_revision.as_deref() {
            EditOutcome::Updated
        } else {
            return Err(UNCERTAIN_WRITE.into());
        };
        let applied = current == ack.accepted_revision && ack.account.status == self.desired_status;
        let replayed = applied
            || (current.len(), current)
                > (ack.accepted_revision.len(), ack.accepted_revision.as_str());
        match (ack.outcome.as_str(), effect) {
            ("applied", EditOutcome::Updated) if applied => Ok(EditOutcome::Updated),
            ("noop", EditOutcome::Unchanged) if applied => Ok(EditOutcome::Unchanged),
            ("replayed", outcome) if replayed => Ok(outcome),
            _ => Err(UNCERTAIN_WRITE.into()),
        }
    }

    fn conflict(&self, conflict: WriteConflict) -> Result<EditOutcome, String> {
        match (conflict.code.as_str(), conflict.account) {
            ("stale_revision", Some(account))
                if self.matches(&account)
                    && account.revision != self.expected_revision
                    && match (account.status.as_str(), account.revision.as_deref()) {
                        ("unknown", None) => true,
                        ("active" | "paused", Some(value)) => revision(value),
                        _ => false,
                    } =>
            {
                Ok(EditOutcome::StaleRevision)
            }
            ("request_id_conflict", None) => Ok(EditOutcome::RequestIdConflict),
            _ => Err(UNCERTAIN_WRITE.into()),
        }
    }
}
#[tauri::command]
pub(crate) async fn company_set_status(
    app: tauri::AppHandle,
    edit: Edit,
) -> Result<EditResult, String> {
    tauri::async_runtime::spawn_blocking(move || set_status(&app.state::<CompanyState>(), edit))
        .await
        .map_err(|_| "Company status write unavailable".to_string())?
}

fn set_status(state: &CompanyState, edit: Edit) -> Result<EditResult, String> {
    let config = state.config()?.ok_or("Company service is not configured")?;
    if edit.company_id != config.company_id {
        return Err("The company session changed".into());
    }
    if !identifier(&edit.provider)
        || !identifier(&edit.account_ref)
        || !matches!(edit.desired_status.as_str(), "active" | "paused")
        || edit
            .expected_revision
            .as_ref()
            .is_some_and(|v| !revision(v))
    {
        return Err("Invalid account edit".into());
    }
    let (generation, token) = state.bearer(&config.company_id)?;
    let response = client()?
        .put(format!(
            "{}/{}/{}/status",
            config.accounts_url(),
            edit.provider,
            edit.account_ref
        ))
        .bearer_auth(token)
        .json(&serde_json::json!({"desired_status":edit.desired_status,
            "expected_revision":edit.expected_revision,"request_id":edit.request_id}))
        .send()
        .map_err(|_| UNCERTAIN_WRITE)?;
    state.verify_session(generation, response.status())?;
    let outcome = match response.status() {
        StatusCode::OK => edit.acknowledgement(decode(response).map_err(|_| UNCERTAIN_WRITE)?)?,
        StatusCode::CONFLICT => edit.conflict(decode(response).map_err(|_| UNCERTAIN_WRITE)?)?,
        StatusCode::FORBIDDEN | StatusCode::NOT_FOUND => {
            return Err("You do not have permission to change this account".into());
        }
        _ => return Err(UNCERTAIN_WRITE.into()),
    };
    state.verify_session(generation, StatusCode::OK)?;
    Ok(EditResult { outcome })
}

#[tauri::command]
pub(crate) fn company_select(
    state: State<'_, CompanyState>,
    company_id: String,
) -> Result<(), String> {
    state.select(&company_id)
}

#[tauri::command]
pub(crate) fn company_sign_out(state: State<'_, CompanyState>) -> Result<(), String> {
    let mut session = state
        .session
        .lock()
        .map_err(|_| "Company session unavailable")?;
    session.generation += 1;
    session.token = None;
    session.sign_in_url = None;
    Ok(())
}

#[tauri::command]
pub(crate) async fn company_sign_in(app: tauri::AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || sign_in(&app.state::<CompanyState>()))
        .await
        .map_err(|_| "Company sign-in unavailable".to_string())?
}

#[tauri::command]
pub(crate) async fn company_reopen_sign_in(app: tauri::AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<CompanyState>();
        let url = state
            .session
            .lock()
            .map_err(|_| "Company session unavailable")?
            .sign_in_url
            .clone()
            .ok_or(
                "The sign-in page is no longer available. Start sign-in again in openProfiler.",
            )?;
        open_browser(&url)
    })
    .await
    .map_err(|_| "Company sign-in unavailable".to_string())?
}

fn sign_in(state: &CompanyState) -> Result<(), String> {
    let (company, generation) = {
        let mut session = state
            .session
            .lock()
            .map_err(|_| "Company session unavailable")?;
        session.generation += 1;
        session.token = None;
        (session.company, session.generation)
    };
    let config = state
        .configs()?
        .get(company)
        .ok_or("Company service is not configured")?;
    let token = browser_login(
        config,
        |url| {
            let mut session = state
                .session
                .lock()
                .map_err(|_| "Company session unavailable")?;
            if session.generation != generation {
                return Err("Company sign-in was cancelled".into());
            }
            session.sign_in_url = Some(url.clone());
            Ok(())
        },
        || {
            state
                .session
                .lock()
                .is_ok_and(|s| s.generation == generation)
        },
    );
    let mut session = state
        .session
        .lock()
        .map_err(|_| "Company session unavailable")?;
    if session.generation != generation {
        return Err("The company session changed".into());
    }
    session.sign_in_url = None;
    session.token = Some(token?);
    Ok(())
}

fn open_browser(url: &Url) -> Result<(), String> {
    open::that_detached(url.as_str()).map_err(|_| "Could not open the system browser".to_string())
}

fn browser_login(
    config: &Config,
    remember_url: impl FnOnce(&Url) -> Result<(), String>,
    current: impl Fn() -> bool,
) -> Result<String, String> {
    let listener =
        TcpListener::bind("127.0.0.1:0").map_err(|_| "Could not open the login callback")?;
    listener
        .set_nonblocking(true)
        .map_err(|_| "Could not open the login callback")?;
    let callback = format!(
        "http://{}/callback",
        listener
            .local_addr()
            .map_err(|_| "Could not open the login callback")?
    );
    let state = Uuid::new_v4().to_string();
    let verifier = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let mut url = endpoint(&config.authorization_endpoint)?;
    url.query_pairs_mut().extend_pairs([
        ("client_id", config.client_id.as_str()),
        ("response_type", "code"),
        ("scope", "openid"),
        ("code_challenge_method", "S256"),
        ("code_challenge", challenge.as_str()),
        ("state", state.as_str()),
        ("redirect_uri", callback.as_str()),
    ]);
    remember_url(&url)?;
    open_browser(&url)?;
    let deadline = Instant::now() + Duration::from_secs(120);
    let code = loop {
        if !current() {
            return Err("Company sign-in was cancelled".into());
        }
        if Instant::now() >= deadline {
            return Err(
                "Company sign-in timed out. Start sign-in again to open a new browser tab.".into(),
            );
        }
        let mut stream = match listener.accept() {
            Ok((stream, _)) => stream,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(100));
                continue;
            }
            Err(_) => return Err("The company sign-in callback is unavailable".into()),
        };
        let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
        let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));
        match read_callback(&mut stream, &state) {
            Some(Callback::Code(code)) => {
                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\nReturn to the openProfiler window where you started this sign-in.");
                break code;
            }
            Some(Callback::Rejected) => {
                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\nCompany sign-in did not complete. Return to openProfiler to try again.");
                return Err("The identity provider declined sign-in. Try signing in again.".into());
            }
            None => {
                let _ = stream.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\nThis tab does not match the current sign-in. Return to openProfiler and reopen the browser.");
            }
        }
    };
    #[derive(Deserialize)]
    struct Token {
        access_token: String,
        token_type: String,
    }
    let response = client()?
        .post(endpoint(&config.token_endpoint)?)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("client_id", config.client_id.as_str()),
            ("code_verifier", verifier.as_str()),
            ("redirect_uri", callback.as_str()),
        ])
        .send()
        .map_err(|_| "Company token exchange failed")?;
    if !response.status().is_success() {
        return Err("Company token exchange failed".into());
    }
    let grant: Token = decode(response)?;
    if !grant.token_type.eq_ignore_ascii_case("bearer")
        || grant.access_token.is_empty()
        || grant.access_token.len() > 16 * 1024
    {
        return Err("Invalid company login response".into());
    }
    Ok(grant.access_token)
}

#[derive(Debug, PartialEq, Eq)]
enum Callback {
    Code(String),
    Rejected,
}

fn read_callback(request: impl Read, expected_state: &str) -> Option<Callback> {
    let mut line = String::new();
    BufReader::new(request)
        .take(8192)
        .read_line(&mut line)
        .ok()?;
    if !line.ends_with('\n') {
        return None;
    }
    parse_callback(line.trim_end(), expected_state)
}

fn parse_callback(request: &str, expected_state: &str) -> Option<Callback> {
    let mut parts = request.split_whitespace();
    if parts.next()? != "GET" {
        return None;
    }
    let relative = parts.next()?;
    if !relative.starts_with("/callback?") {
        return None;
    }
    let url = Url::parse(&format!("http://localhost{relative}")).ok()?;
    let mut state = None;
    let mut code = None;
    let mut error = None;
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "state" if state.is_none() => state = Some(value),
            "code" if code.is_none() => code = Some(value),
            "error" if error.is_none() => error = Some(value),
            "state" | "code" | "error" => return None,
            _ => (),
        }
    }
    if state.as_deref() != Some(expected_state) {
        return None;
    }
    match (code, error) {
        (Some(code), None) if !code.is_empty() && code.len() <= 4096 => {
            Some(Callback::Code(code.into_owned()))
        }
        (None, Some(error)) if !error.is_empty() => Some(Callback::Rejected),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMPANY: &[u8] = br#"{
        "company_id": "company",
        "service_origin": "https://status.example",
        "authorization_endpoint": "https://login.example/authorize",
        "token_endpoint": "https://login.example/token",
        "client_id": "openprofiler",
        "managed_roots": [{"provider":"codex","path":"managed"}]
    }"#;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path =
                env::temp_dir().join(format!("openprofiler-desktop-test-{}", Uuid::new_v4()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn discovery(&self) -> DiscoveryConfig {
            DiscoveryConfig {
                providers: vec![opensoft_open_profiler_core::ProviderConfig {
                    provider: Provider::Codex,
                    manifest_path: self.0.join("profiles.json"),
                    profiles_home: self.0.clone(),
                    active_home: self.0.join("active"),
                }],
            }
        }

        fn profile(&self, path: &str, account: &str) -> PathBuf {
            let directory = self.0.join("profiles").join(path);
            fs::create_dir_all(&directory).unwrap();
            fs::write(directory.join("auth.json"), serde_json::to_vec(&serde_json::json!({
                "auth_mode":"chatgpt", "tokens": {
                    "account_id":account, "access_token":"test", "id_token":"test", "refresh_token":"test"
                },
            })).unwrap()).unwrap();
            directory
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn missing_config_stays_blocked_after_enrollment() {
        let tmp = TestDirectory::new();
        assert!(load_settings(None, &tmp.0).unwrap().is_empty());
        let config = tmp.0.join("explicit-company.json");
        fs::write(&config, COMPANY).unwrap();
        assert_eq!(
            load_settings(Some(config.clone()), &tmp.0).unwrap().len(),
            1
        );
        assert!(load_settings(None, &tmp.0).is_err());
        fs::remove_file(&config).unwrap();
        assert!(load_settings(Some(config), &tmp.0).is_err());
        fs::write(tmp.0.join("company-accounts.json"), COMPANY).unwrap();
        assert_eq!(load_settings(None, &tmp.0).unwrap().len(), 1);
    }

    #[test]
    fn local_undo_requires_a_known_backup_outside_managed_roots() {
        let tmp = TestDirectory::new();
        tmp.profile("local", "local-account");
        tmp.profile("managed/enrolled", "managed-account");
        let config = parse_configs(COMPANY).unwrap();
        assert!(verify_local_undo(&config, &tmp.discovery(), "local-account").is_ok());
        assert!(verify_local_undo(&config, &tmp.discovery(), "managed-account").is_err());
        assert!(verify_local_undo(&config, &tmp.discovery(), "unknown-account").is_err());
        tmp.profile("managed/duplicate-local", "local-account");
        assert!(verify_local_undo(&config, &tmp.discovery(), "local-account").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn alternate_paths_into_managed_roots_are_classified_managed() {
        let tmp = TestDirectory::new();
        let enrolled = tmp.profile("managed/enrolled", "managed-account");
        let local = tmp.profile("outside", "local-account");
        let config = parse_configs(COMPANY).unwrap().remove(0);
        let discovery = tmp.discovery();
        std::os::unix::fs::symlink(enrolled, tmp.0.join("profiles/alias")).unwrap();
        let resolved = profile_policy_path(&discovery, Provider::Codex, "alias").unwrap();
        assert!(config
            .manages_source(&discovery, Provider::Codex, "alias", &resolved)
            .unwrap());
        assert!(!same_path("alias", &resolved));

        fs::rename(tmp.0.join("profiles/managed"), tmp.0.join("profiles/old")).unwrap();
        std::os::unix::fs::symlink(local, tmp.0.join("profiles/managed")).unwrap();
        assert!(config
            .manages_source(&discovery, Provider::Codex, "outside", "outside")
            .unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn enrolled_path_can_resolve_through_a_linked_managed_root() {
        let tmp = TestDirectory::new();
        let real = tmp.profile("stored/enrolled", "managed-account");
        let different = tmp.profile("other", "different-account");
        let discovery = tmp.discovery();
        std::os::unix::fs::symlink(real.parent().unwrap(), tmp.0.join("profiles/managed")).unwrap();
        std::os::unix::fs::symlink(different, tmp.0.join("profiles/stored/redirect")).unwrap();
        let mut config = parse_configs(COMPANY).unwrap().remove(0);
        let enrolled = "managed/enrolled";
        config.profiles.push(Binding {
            provider: Provider::Codex,
            path: enrolled.into(),
            account_ref: "approved-account".into(),
            codex_account_id: Some("managed-account".into()),
        });
        assert!(config.validate().is_ok());
        assert!(config
            .binding(Provider::Codex, enrolled, Some("managed-account"), true)
            .unwrap()
            .is_some());
        let resolved = profile_policy_path(&discovery, Provider::Codex, enrolled).unwrap();
        assert!(config.matches_managed_source(&discovery, Provider::Codex, enrolled, &resolved));
        let redirected = "managed/redirect";
        let resolved = profile_policy_path(&discovery, Provider::Codex, redirected).unwrap();
        assert!(!config.matches_managed_source(&discovery, Provider::Codex, redirected, &resolved));
        assert!(config
            .binding(
                Provider::Codex,
                "stored/enrolled",
                Some("managed-account"),
                true
            )
            .is_err());
    }

    #[cfg(windows)]
    #[test]
    fn managed_paths_accept_native_separator_and_windows_case() {
        assert!(same_path(r"Managed\Enrolled", "managed/enrolled"));
        assert!(is_inside(r"Managed\Enrolled", "managed"));
        assert!(!is_inside(r"Managed-Other\Enrolled", "managed"));
    }

    #[test]
    fn validates_status_acknowledgements_and_distinguishes_conflicts() {
        let edit = Edit {
            company_id: "company".into(),
            provider: "codex".into(),
            account_ref: "profile".into(),
            desired_status: "paused".into(),
            expected_revision: Some("1".into()),
            request_id: Uuid::new_v4(),
        };
        let account = |name: &str, status: &str, revision: &str| Account {
            company_id: "company".into(),
            provider: "codex".into(),
            account_ref: name.into(),
            status: status.into(),
            revision: Some(revision.into()),
            display_name: None,
        };
        let ack = |name, status, revision, outcome: &str| WriteAcknowledgement {
            account: account(name, status, revision),
            outcome: outcome.into(),
            accepted_revision: "2".into(),
        };
        assert_eq!(
            edit.acknowledgement(ack("profile", "paused", "2", "applied")),
            Ok(EditOutcome::Updated)
        );
        assert_eq!(
            edit.acknowledgement(ack("profile", "active", "3", "replayed")),
            Ok(EditOutcome::Updated)
        );
        assert!(edit
            .acknowledgement(ack("other", "paused", "2", "applied"))
            .is_err());
        assert!(edit
            .acknowledgement(ack("profile", "active", "2", "applied"))
            .is_err());
        assert_eq!(
            edit.conflict(WriteConflict {
                code: "stale_revision".into(),
                account: Some(account("profile", "paused", "2")),
            }),
            Ok(EditOutcome::StaleRevision)
        );
        assert_eq!(
            edit.conflict(WriteConflict {
                code: "request_id_conflict".into(),
                account: None,
            }),
            Ok(EditOutcome::RequestIdConflict)
        );
        assert!(edit
            .conflict(WriteConflict {
                code: "stale_revision".into(),
                account: None,
            })
            .is_err());
    }

    #[test]
    fn callback_requires_the_exact_path_and_state() {
        assert_eq!(
            parse_callback("GET /callback?code=one&state=ok HTTP/1.1", "ok"),
            Some(Callback::Code("one".into()))
        );
        assert_eq!(
            parse_callback("GET /callback?code=one&state=wrong HTTP/1.1", "ok"),
            None
        );
        assert_eq!(
            parse_callback("GET /callback?code=one&state=ok&state=ok HTTP/1.1", "ok"),
            None
        );
        assert_eq!(
            parse_callback("GET /another?code=one&state=ok HTTP/1.1", "ok"),
            None
        );
    }

    #[test]
    fn reads_a_callback_split_across_multiple_reads() {
        let first = b"GET /callback?code=second&sta";
        let rest = b"te=window-b HTTP/1.1\r\nHost: 127.0.0.1\r\n";
        assert_eq!(
            read_callback(first.as_slice().chain(rest.as_slice()), "window-b"),
            Some(Callback::Code("second".into()))
        );
        assert_eq!(
            read_callback(first.as_slice().chain(rest.as_slice()), "window-a"),
            None
        );
    }

    #[test]
    fn reports_provider_denials_only_for_the_matching_attempt() {
        let denied = "GET /callback?error=access_denied&state=second HTTP/1.1";
        assert_eq!(parse_callback(denied, "second"), Some(Callback::Rejected));
        assert_eq!(parse_callback(denied, "first"), None);
        assert_eq!(
            parse_callback(
                "GET /callback?code=one&error=denied&state=second HTTP/1.1",
                "second"
            ),
            None
        );
    }

    #[test]
    fn managed_root_requires_a_full_path_component() {
        let root = ManagedRoot {
            provider: Provider::Claude,
            path: "company".into(),
        };
        assert!(root.matches(Provider::Claude, "company/claude-03"));
        assert!(!root.matches(Provider::Claude, "company-copy/claude-03"));
    }
}
