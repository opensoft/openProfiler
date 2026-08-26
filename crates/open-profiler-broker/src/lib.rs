//! Credential custody and short-lived token minting for openProfiler.
//!
//! openProfiler holds long-lived model-provider credentials on the local
//! device and mints short-lived, scoped tokens on demand. It is deliberately
//! **not** a proxy: a consumer asks for a token, calls the provider with it
//! directly, and the broker is out of the request path.
//!
//! The declaration a consuming binding reads — argv shapes, stdin and stdout
//! contracts, exit codes, what a minted token carries, and the audit
//! obligation — is `docs/broker-cli.md`. This crate implements it.
//!
//! Two things this module will not do, and the reasons:
//!
//! - A credential never reaches this process through `argv` or an environment
//!   variable. Argv is world-readable on the platforms openProfiler runs on,
//!   and environment variables are inherited by every descendant process.
//! - A credential is written to exactly one file: the custody entry, at mode
//!   `0600` under a mode-`0700` root. The reference index, the audit log,
//!   every error and every log line are free of credential material by
//!   construction rather than by care.

pub mod cli;
pub mod ids;
pub mod secret;
pub mod store;
pub mod time;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use thiserror::Error;

pub use secret::Secret;
pub use store::{CustodyRecord, CustodyStore};

pub const SCHEMA_VERSION: u32 = 1;
pub const INTAKE_KIND: &str = "openprofiler_broker_intake";
pub const MINT_KIND: &str = "openprofiler_broker_mint";
pub const REVOCATION_KIND: &str = "openprofiler_broker_revocation";
pub const REFERENCE_LIST_KIND: &str = "openprofiler_broker_reference_list";
pub const AUDIT_RECORD_KIND: &str = "openprofiler_broker_audit_record";
pub const ERROR_KIND: &str = "openprofiler_broker_error";

pub const MIN_LIFETIME_SECONDS: u64 = 30;
pub const MAX_LIFETIME_SECONDS: u64 = 3_600;
pub const DEFAULT_LIFETIME_SECONDS: u64 = 300;

/// The identity this build issues under, absent an explicit `--issued-by`.
pub fn broker_identity() -> String {
    format!("openprofiler-broker/{}", env!("CARGO_PKG_VERSION"))
}

#[derive(Debug, Error)]
pub enum BrokerError {
    /// A malformed invocation. Carries the machine-readable code so a caller
    /// can tell an empty secret from an unknown flag.
    #[error("{message}")]
    Usage { code: &'static str, message: String },

    #[error("no credential is held under that reference")]
    UnknownReference,

    #[error("{0}")]
    CredentialUnusable(String),

    #[error("{0}")]
    NotImplemented(String),

    #[error("could not read the custody store at {path}: {message}")]
    CustodyRead { path: PathBuf, message: String },

    #[error("could not write the custody store at {path}: {message}")]
    CustodyWrite { path: PathBuf, message: String },

    #[error("refusing to follow a symlinked custody path: {0}")]
    UnsafeCustodyPath(PathBuf),

    #[error("could not determine the current user's home directory")]
    HomeUnavailable,

    #[error("could not write the result to standard output: {0}")]
    OutputUnavailable(String),
}

impl BrokerError {
    pub fn usage(message: impl Into<String>) -> Self {
        Self::Usage {
            code: "usage",
            message: message.into(),
        }
    }

    pub fn usage_coded(code: &'static str, message: impl Into<String>) -> Self {
        Self::Usage {
            code,
            message: message.into(),
        }
    }

    /// The `code` field of the error object written to standard error.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Usage { code, .. } => code,
            Self::UnknownReference => "unknown_reference",
            Self::CredentialUnusable(_) => "credential_unusable",
            Self::NotImplemented(_) => "not_implemented",
            Self::CustodyRead { .. } | Self::CustodyWrite { .. } | Self::UnsafeCustodyPath(_) => {
                "custody_unavailable"
            }
            Self::HomeUnavailable => "custody_unavailable",
            Self::OutputUnavailable(_) => "internal",
        }
    }

    /// The process exit code. Declared in `docs/broker-cli.md`; a consumer
    /// reads it to tell "nothing is configured" from "it broke".
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Usage { .. } => 2,
            Self::UnknownReference => 3,
            Self::CredentialUnusable(_) => 4,
            Self::NotImplemented(_) => 5,
            Self::CustodyRead { .. }
            | Self::CustodyWrite { .. }
            | Self::UnsafeCustodyPath(_)
            | Self::HomeUnavailable => 6,
            Self::OutputUnavailable(_) => 1,
        }
    }
}

pub type Result<T> = std::result::Result<T, BrokerError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthKind {
    ApiKey,
    Oauth,
}

impl AuthKind {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "api_key" => Ok(Self::ApiKey),
            "oauth" => Ok(Self::Oauth),
            other => Err(BrokerError::usage(format!(
                "unknown --auth-kind {other:?}; expected api_key or oauth"
            ))),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ApiKey => "api_key",
            Self::Oauth => "oauth",
        }
    }

    /// What the provider will accept the minted token as.
    pub fn token_type(self) -> &'static str {
        match self {
            // The stored API key, verbatim: the provider's own credential,
            // presented however that provider expects a key to be presented.
            Self::ApiKey => "api_key",
            // A provider-issued access token, presented as a bearer token.
            Self::Oauth => "bearer",
        }
    }

    /// Who actually enforces the expiry and the scope of a minted token.
    ///
    /// This is the honest half of the design and it is machine-readable so a
    /// consumer never has to infer it. For a plain API key there is no
    /// provider mechanism to shorten the life of the credential, so the
    /// declared expiry is broker bookkeeping that the consumer honors, and the
    /// declared scope describes what the token was issued for rather than a
    /// limit the provider will apply.
    pub fn enforcement(self) -> Enforcement {
        match self {
            Self::ApiKey => Enforcement {
                expiry: Enforced::BrokerBookkeeping,
                scope: Enforced::Declared,
            },
            Self::Oauth => Enforcement {
                expiry: Enforced::Provider,
                scope: Enforced::Provider,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Enforced {
    /// The broker declares it and the consumer honors it. The provider does
    /// not know about it and will not apply it.
    BrokerBookkeeping,
    /// Recorded as the issuance intent. Not narrowed by anything.
    Declared,
    /// The provider itself applies it.
    Provider,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Enforcement {
    pub expiry: Enforced,
    pub scope: Enforced,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditEvent {
    Intake,
    Mint,
    Revoke,
}

/// One line of `<broker home>/audit/broker-audit.jsonl`.
///
/// The mint is the auditable event; provider calls made under one minted token
/// are its children and the broker neither sees nor records them. No token
/// material appears here — not the token, not a prefix, not a hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditRecord {
    pub schema_version: u32,
    pub kind: String,
    pub audit_ref: String,
    pub event: AuditEvent,
    pub recorded_at: String,
    pub reference: String,
    pub binding: String,
    pub provider: String,
    pub auth_kind: AuthKind,
    pub scope: Vec<String>,
    pub issued_by: String,
    pub approved_by: String,
    pub expires_at: Option<String>,
    pub enforcement: Enforcement,
    pub retry_of: Option<String>,
}

/// A non-secret index row. Safe to read, log, quote and commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferenceEntry {
    pub reference: String,
    pub binding: String,
    pub provider: String,
    pub auth_kind: AuthKind,
    pub label: Option<String>,
    pub created_at: String,
    pub max_lifetime_seconds: u64,
    pub issued_by: String,
    pub approved_by: String,
    pub mint_count: u64,
    pub last_minted_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IntakeReceipt {
    pub schema_version: u32,
    pub kind: String,
    pub reference: String,
    pub binding: String,
    pub provider: String,
    pub auth_kind: AuthKind,
    pub label: Option<String>,
    pub created_at: String,
    pub max_lifetime_seconds: u64,
    pub issued_by: String,
    pub approved_by: String,
    pub audit_ref: String,
}

/// The one value the broker emits that carries credential material, emitted by
/// exactly one command.
#[derive(Debug, Serialize)]
pub struct MintedToken {
    pub schema_version: u32,
    pub kind: String,
    pub reference: String,
    pub binding: String,
    pub provider: String,
    pub auth_kind: AuthKind,
    pub token: Secret,
    pub token_type: &'static str,
    pub issued_at: String,
    pub expires_at: String,
    pub expires_in_seconds: u64,
    pub scope: Vec<String>,
    pub issued_by: String,
    pub approved_by: String,
    pub audit_ref: String,
    pub retry_of: Option<String>,
    pub enforcement: Enforcement,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Revocation {
    pub schema_version: u32,
    pub kind: String,
    pub reference: String,
    pub binding: String,
    pub provider: String,
    pub auth_kind: AuthKind,
    pub revoked: bool,
    pub revoked_at: String,
    pub audit_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReferenceListing {
    pub schema_version: u32,
    pub kind: String,
    pub references: Vec<ReferenceEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntakeRequest {
    pub binding: String,
    pub provider: String,
    pub auth_kind: AuthKind,
    pub label: Option<String>,
    pub issued_by: String,
    pub approved_by: String,
    pub max_lifetime_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MintRequest {
    pub reference: String,
    pub scope: Vec<String>,
    pub lifetime_seconds: Option<u64>,
    pub retry_of: Option<String>,
}

/// Takes a secret into custody and returns a non-secret reference.
///
/// The secret arrives already read from standard input; this function is the
/// only place it is written down, and it is written to one mode-`0600` file.
pub fn intake(
    store: &CustodyStore,
    request: IntakeRequest,
    secret: Secret,
) -> Result<IntakeReceipt> {
    if request.auth_kind == AuthKind::Oauth {
        return Err(oauth_not_implemented(
            "an OAuth grant is not taken on standard input; \
             the browser authorization flow is declared in docs/broker-cli.md \
             and `authorize` is not built yet",
        ));
    }
    if secret.is_empty() {
        return Err(BrokerError::usage_coded(
            "empty_secret",
            "standard input held no credential; intake reads the secret from stdin",
        ));
    }
    if secret.as_str().is_none() {
        return Err(BrokerError::usage_coded(
            "invalid_secret_encoding",
            "the credential on standard input is not valid UTF-8",
        ));
    }

    let now = time::now_unix_seconds();
    let created_at = time::format_rfc3339_utc(now);
    let reference = ids::new_reference();
    let audit_ref = ids::new_audit_ref();

    let record = CustodyRecord::new(
        reference.clone(),
        request.binding.clone(),
        request.provider.clone(),
        request.auth_kind,
        request.label.clone(),
        created_at.clone(),
        request.max_lifetime_seconds,
        request.issued_by.clone(),
        request.approved_by.clone(),
        secret,
    );
    store.put(&record)?;
    store.upsert_index_entry(record.to_reference_entry())?;
    store.append_audit(&AuditRecord {
        schema_version: SCHEMA_VERSION,
        kind: AUDIT_RECORD_KIND.to_string(),
        audit_ref: audit_ref.clone(),
        event: AuditEvent::Intake,
        recorded_at: created_at.clone(),
        reference: reference.clone(),
        binding: request.binding.clone(),
        provider: request.provider.clone(),
        auth_kind: request.auth_kind,
        scope: Vec::new(),
        issued_by: request.issued_by.clone(),
        approved_by: request.approved_by.clone(),
        // No token exists at intake, so there is no expiry to record.
        expires_at: None,
        enforcement: request.auth_kind.enforcement(),
        retry_of: None,
    })?;

    Ok(IntakeReceipt {
        schema_version: SCHEMA_VERSION,
        kind: INTAKE_KIND.to_string(),
        reference,
        binding: request.binding,
        provider: request.provider,
        auth_kind: request.auth_kind,
        label: request.label,
        created_at,
        max_lifetime_seconds: request.max_lifetime_seconds,
        issued_by: request.issued_by,
        approved_by: request.approved_by,
        audit_ref,
    })
}

/// Issues a token for a held credential.
///
/// Cheap and repeatable by design: on the `api_key` path this is one local
/// read and two small local writes, with no network and no provider round
/// trip. That is what makes the ruled mid-turn behaviour — re-mint and retry
/// once — affordable.
pub fn mint(store: &CustodyStore, request: MintRequest) -> Result<MintedToken> {
    let record = store.get(&request.reference)?;
    if record.auth_kind == AuthKind::Oauth {
        return Err(oauth_not_implemented(
            "minting from a stored OAuth grant needs the provider refresh exchange, \
             which is declared in docs/broker-cli.md and not built yet",
        ));
    }
    if record.secret.is_empty() {
        return Err(BrokerError::CredentialUnusable(
            "the custody entry holds no credential material".to_string(),
        ));
    }

    // A larger request is clamped rather than refused; the response states the
    // effective lifetime and the consumer honors that.
    let lifetime = request
        .lifetime_seconds
        .unwrap_or(record.max_lifetime_seconds)
        .min(record.max_lifetime_seconds)
        .clamp(MIN_LIFETIME_SECONDS, MAX_LIFETIME_SECONDS);

    let now = time::now_unix_seconds();
    let issued_at = time::format_rfc3339_utc(now);
    let expires_at = time::format_rfc3339_utc(now.saturating_add(lifetime as i64));
    let audit_ref = ids::new_audit_ref();

    let mut entry = record.to_reference_entry();
    let previous = store
        .read_index()?
        .into_iter()
        .find(|existing| existing.reference == record.reference);
    entry.mint_count = previous.map(|existing| existing.mint_count).unwrap_or(0) + 1;
    entry.last_minted_at = Some(issued_at.clone());
    store.upsert_index_entry(entry)?;

    store.append_audit(&AuditRecord {
        schema_version: SCHEMA_VERSION,
        kind: AUDIT_RECORD_KIND.to_string(),
        audit_ref: audit_ref.clone(),
        event: AuditEvent::Mint,
        recorded_at: issued_at.clone(),
        reference: record.reference.clone(),
        binding: record.binding.clone(),
        provider: record.provider.clone(),
        auth_kind: record.auth_kind,
        scope: request.scope.clone(),
        issued_by: record.issued_by.clone(),
        approved_by: record.approved_by.clone(),
        expires_at: Some(expires_at.clone()),
        enforcement: record.auth_kind.enforcement(),
        retry_of: request.retry_of.clone(),
    })?;

    Ok(MintedToken {
        schema_version: SCHEMA_VERSION,
        kind: MINT_KIND.to_string(),
        reference: record.reference,
        binding: record.binding,
        provider: record.provider,
        auth_kind: record.auth_kind,
        // Provider-native: the credential the provider itself accepts. A
        // broker-issued token the provider was pre-configured to trust would
        // put the broker back in the request path it was taken out of.
        token: record.secret,
        token_type: record.auth_kind.token_type(),
        issued_at,
        expires_at,
        expires_in_seconds: lifetime,
        scope: request.scope,
        issued_by: record.issued_by,
        approved_by: record.approved_by,
        audit_ref,
        retry_of: request.retry_of,
        enforcement: record.auth_kind.enforcement(),
    })
}

/// Destroys a held credential. The audit trail is not destroyed with it —
/// revocation appends to it.
pub fn revoke(store: &CustodyStore, reference: &str) -> Result<Revocation> {
    let record = store.get(reference)?;
    store.remove(reference)?;
    store.remove_index_entry(reference)?;

    let revoked_at = time::format_rfc3339_utc(time::now_unix_seconds());
    let audit_ref = ids::new_audit_ref();
    store.append_audit(&AuditRecord {
        schema_version: SCHEMA_VERSION,
        kind: AUDIT_RECORD_KIND.to_string(),
        audit_ref: audit_ref.clone(),
        event: AuditEvent::Revoke,
        recorded_at: revoked_at.clone(),
        reference: record.reference.clone(),
        binding: record.binding.clone(),
        provider: record.provider.clone(),
        auth_kind: record.auth_kind,
        scope: Vec::new(),
        issued_by: record.issued_by.clone(),
        approved_by: record.approved_by.clone(),
        expires_at: None,
        enforcement: record.auth_kind.enforcement(),
        retry_of: None,
    })?;

    Ok(Revocation {
        schema_version: SCHEMA_VERSION,
        kind: REVOCATION_KIND.to_string(),
        reference: record.reference,
        binding: record.binding,
        provider: record.provider,
        auth_kind: record.auth_kind,
        revoked: true,
        revoked_at,
        audit_ref,
    })
}

/// The non-secret reference index. Never opens a custody file: reading a
/// listing is not a reason to page a secret into this process.
pub fn list(store: &CustodyStore) -> Result<ReferenceListing> {
    Ok(ReferenceListing {
        schema_version: SCHEMA_VERSION,
        kind: REFERENCE_LIST_KIND.to_string(),
        references: store.read_index()?,
    })
}

fn oauth_not_implemented(detail: &str) -> BrokerError {
    BrokerError::NotImplemented(format!(
        "the OAuth path is declared but not built: {detail}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const SECRET: &str = "sk-live-must-not-appear-anywhere";

    fn store(temp: &TempDir) -> CustodyStore {
        CustodyStore::at(temp.path().join("broker"))
    }

    fn request(auth_kind: AuthKind) -> IntakeRequest {
        IntakeRequest {
            binding: "anthropic-default".to_string(),
            provider: "anthropic".to_string(),
            auth_kind,
            label: Some("team key".to_string()),
            issued_by: broker_identity(),
            approved_by: "brett@opensoft.one".to_string(),
            max_lifetime_seconds: DEFAULT_LIFETIME_SECONDS,
        }
    }

    fn take(store: &CustodyStore) -> IntakeReceipt {
        intake(
            store,
            request(AuthKind::ApiKey),
            Secret::from_string(SECRET.to_string()),
        )
        .unwrap()
    }

    #[test]
    fn intake_stores_the_secret_and_returns_only_a_reference() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        let receipt = take(&store);

        assert!(ids::is_valid_reference(&receipt.reference));
        assert!(ids::is_valid_audit_ref(&receipt.audit_ref));
        assert_eq!(receipt.auth_kind, AuthKind::ApiKey);
        assert_eq!(receipt.max_lifetime_seconds, DEFAULT_LIFETIME_SECONDS);
        let rendered = serde_json::to_string(&receipt).unwrap();
        assert!(!rendered.contains(SECRET));
    }

    #[test]
    fn intake_refuses_an_empty_secret() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        let error = intake(&store, request(AuthKind::ApiKey), Secret::new(Vec::new()))
            .expect_err("empty stdin is refused");
        assert_eq!(error.code(), "empty_secret");
        assert_eq!(error.exit_code(), 2);
        assert!(!store.root().join("custody").exists());
    }

    #[test]
    fn intake_refuses_a_non_utf8_secret() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        let error = intake(
            &store,
            request(AuthKind::ApiKey),
            Secret::new(vec![0xff, 0xfe]),
        )
        .expect_err("a non-UTF-8 credential is refused");
        assert_eq!(error.code(), "invalid_secret_encoding");
        assert_eq!(error.exit_code(), 2);
    }

    #[test]
    fn intake_of_an_oauth_grant_refuses_honestly_before_reading_anything() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        let error = intake(
            &store,
            request(AuthKind::Oauth),
            Secret::from_string(SECRET.to_string()),
        )
        .expect_err("the OAuth path is staged");
        assert_eq!(error.code(), "not_implemented");
        assert_eq!(error.exit_code(), 5);
        assert!(error.to_string().contains("authorize"));
        assert!(!store.root().exists());
    }

    #[test]
    fn mint_returns_the_provider_native_credential_with_declared_expiry() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        let receipt = take(&store);

        let minted = mint(
            &store,
            MintRequest {
                reference: receipt.reference.clone(),
                scope: vec!["messages:write".to_string()],
                lifetime_seconds: None,
                retry_of: None,
            },
        )
        .unwrap();

        assert_eq!(minted.token.as_str(), Some(SECRET));
        assert_eq!(minted.token_type, "api_key");
        assert_eq!(minted.expires_in_seconds, DEFAULT_LIFETIME_SECONDS);
        assert_eq!(minted.enforcement.expiry, Enforced::BrokerBookkeeping);
        assert_eq!(minted.enforcement.scope, Enforced::Declared);
        assert_eq!(minted.scope, vec!["messages:write".to_string()]);
        assert_eq!(minted.approved_by, "brett@opensoft.one");
        assert!(minted.issued_at < minted.expires_at);
    }

    #[test]
    fn mint_clamps_a_lifetime_beyond_the_credentials_maximum() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        let receipt = take(&store);

        let minted = mint(
            &store,
            MintRequest {
                reference: receipt.reference,
                scope: Vec::new(),
                lifetime_seconds: Some(MAX_LIFETIME_SECONDS * 10),
                retry_of: None,
            },
        )
        .unwrap();
        assert_eq!(minted.expires_in_seconds, DEFAULT_LIFETIME_SECONDS);
    }

    #[test]
    fn mint_is_repeatable_and_records_a_retry_visibly() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        let receipt = take(&store);

        let first = mint(
            &store,
            MintRequest {
                reference: receipt.reference.clone(),
                scope: Vec::new(),
                lifetime_seconds: None,
                retry_of: None,
            },
        )
        .unwrap();
        let second = mint(
            &store,
            MintRequest {
                reference: receipt.reference.clone(),
                scope: Vec::new(),
                lifetime_seconds: None,
                retry_of: Some(first.audit_ref.clone()),
            },
        )
        .unwrap();

        assert_ne!(first.audit_ref, second.audit_ref);
        assert_eq!(second.token.as_str(), Some(SECRET));
        assert_eq!(second.retry_of, Some(first.audit_ref.clone()));

        let audit = store.read_audit().unwrap();
        let mints: Vec<_> = audit
            .iter()
            .filter(|record| record.event == AuditEvent::Mint)
            .collect();
        assert_eq!(mints.len(), 2);
        assert_eq!(mints[1].retry_of, Some(first.audit_ref));

        let entry = list(&store).unwrap().references.remove(0);
        assert_eq!(entry.mint_count, 2);
        assert!(entry.last_minted_at.is_some());
    }

    #[test]
    fn mint_refuses_an_unknown_reference() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        let error = mint(
            &store,
            MintRequest {
                reference: ids::new_reference(),
                scope: Vec::new(),
                lifetime_seconds: None,
                retry_of: None,
            },
        )
        .expect_err("an unheld reference cannot mint");
        assert_eq!(error.code(), "unknown_reference");
        assert_eq!(error.exit_code(), 3);
    }

    #[test]
    fn mint_refuses_a_stored_oauth_grant() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        // The library supports the record shape; only the refresh exchange is
        // missing, so the refusal is proven against a stored grant rather than
        // assumed from the intake refusal.
        let reference = ids::new_reference();
        store
            .put(&CustodyRecord::new(
                reference.clone(),
                "anthropic-subscription".to_string(),
                "anthropic".to_string(),
                AuthKind::Oauth,
                None,
                time::format_rfc3339_utc(time::now_unix_seconds()),
                DEFAULT_LIFETIME_SECONDS,
                broker_identity(),
                "brett@opensoft.one".to_string(),
                Secret::from_string("refresh-grant".to_string()),
            ))
            .unwrap();

        let error = mint(
            &store,
            MintRequest {
                reference,
                scope: Vec::new(),
                lifetime_seconds: None,
                retry_of: None,
            },
        )
        .expect_err("the OAuth refresh exchange is staged");
        assert_eq!(error.code(), "not_implemented");
        assert_eq!(error.exit_code(), 5);
    }

    #[test]
    fn the_oauth_kind_declares_provider_enforcement() {
        let enforcement = AuthKind::Oauth.enforcement();
        assert_eq!(enforcement.expiry, Enforced::Provider);
        assert_eq!(enforcement.scope, Enforced::Provider);
        assert_eq!(AuthKind::Oauth.token_type(), "bearer");
    }

    #[test]
    fn revoke_destroys_custody_and_keeps_the_audit_trail() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        let receipt = take(&store);

        let revocation = revoke(&store, &receipt.reference).unwrap();
        assert!(revocation.revoked);
        assert_eq!(revocation.reference, receipt.reference);

        assert!(list(&store).unwrap().references.is_empty());
        assert!(matches!(
            mint(
                &store,
                MintRequest {
                    reference: receipt.reference.clone(),
                    scope: Vec::new(),
                    lifetime_seconds: None,
                    retry_of: None,
                }
            ),
            Err(BrokerError::UnknownReference)
        ));

        let audit = store.read_audit().unwrap();
        assert_eq!(audit.len(), 2);
        assert_eq!(audit[0].event, AuditEvent::Intake);
        assert_eq!(audit[1].event, AuditEvent::Revoke);
    }

    #[test]
    fn revoking_twice_says_so_rather_than_pretending() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        let receipt = take(&store);
        revoke(&store, &receipt.reference).unwrap();
        let error = revoke(&store, &receipt.reference).expect_err("nothing is left to revoke");
        assert_eq!(error.exit_code(), 3);
    }

    #[test]
    fn every_audit_record_carries_the_four_accountability_fields() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        let receipt = take(&store);
        mint(
            &store,
            MintRequest {
                reference: receipt.reference.clone(),
                scope: vec!["messages:write".to_string()],
                lifetime_seconds: None,
                retry_of: None,
            },
        )
        .unwrap();
        revoke(&store, &receipt.reference).unwrap();

        let audit = store.read_audit().unwrap();
        assert_eq!(audit.len(), 3);
        for record in &audit {
            assert!(!record.issued_by.is_empty(), "issued_by is mandatory");
            assert!(!record.approved_by.is_empty(), "approved_by is mandatory");
            assert!(ids::is_valid_audit_ref(&record.audit_ref));
            assert_eq!(record.kind, AUDIT_RECORD_KIND);
            assert_eq!(record.schema_version, SCHEMA_VERSION);
        }
        // The mint is the moment a token — and therefore an expiry — exists.
        let mint_record = audit
            .iter()
            .find(|record| record.event == AuditEvent::Mint)
            .unwrap();
        assert!(mint_record.expires_at.is_some());
        assert!(audit[0].expires_at.is_none());
    }

    #[test]
    fn no_audit_record_carries_token_material() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        let receipt = take(&store);
        mint(
            &store,
            MintRequest {
                reference: receipt.reference,
                scope: Vec::new(),
                lifetime_seconds: None,
                retry_of: None,
            },
        )
        .unwrap();

        let raw = std::fs::read_to_string(store.audit_path()).unwrap();
        assert!(!raw.contains(SECRET));
        let index = std::fs::read_to_string(store.index_path()).unwrap();
        assert!(!index.contains(SECRET));
    }

    #[test]
    fn the_secret_lives_in_exactly_one_file() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        let receipt = take(&store);
        mint(
            &store,
            MintRequest {
                reference: receipt.reference.clone(),
                scope: Vec::new(),
                lifetime_seconds: None,
                retry_of: None,
            },
        )
        .unwrap();

        let holders = files_containing(temp.path(), SECRET);
        assert_eq!(holders.len(), 1, "found {holders:?}");
        assert_eq!(
            holders[0],
            store
                .custody_directory()
                .join(format!("{}.json", receipt.reference))
        );

        revoke(&store, &receipt.reference).unwrap();
        assert!(files_containing(temp.path(), SECRET).is_empty());
    }

    fn files_containing(root: &std::path::Path, needle: &str) -> Vec<PathBuf> {
        let mut found = Vec::new();
        let mut pending = vec![root.to_path_buf()];
        while let Some(directory) = pending.pop() {
            for entry in std::fs::read_dir(&directory).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    pending.push(path);
                } else if std::fs::read(&path)
                    .map(|bytes| String::from_utf8_lossy(&bytes).contains(needle))
                    .unwrap_or(false)
                {
                    found.push(path);
                }
            }
        }
        found.sort();
        found
    }

    #[test]
    fn error_codes_and_exit_codes_match_the_declaration() {
        assert_eq!(BrokerError::usage("x").exit_code(), 2);
        assert_eq!(BrokerError::usage("x").code(), "usage");
        assert_eq!(BrokerError::UnknownReference.exit_code(), 3);
        assert_eq!(
            BrokerError::CredentialUnusable("x".to_string()).exit_code(),
            4
        );
        assert_eq!(BrokerError::NotImplemented("x".to_string()).exit_code(), 5);
        assert_eq!(BrokerError::HomeUnavailable.exit_code(), 6);
        assert_eq!(BrokerError::HomeUnavailable.code(), "custody_unavailable");
        assert_eq!(
            BrokerError::UnsafeCustodyPath(PathBuf::from("/x")).exit_code(),
            6
        );
    }

    #[test]
    fn auth_kinds_parse_and_render() {
        assert_eq!(AuthKind::parse("api_key").unwrap(), AuthKind::ApiKey);
        assert_eq!(AuthKind::parse("oauth").unwrap(), AuthKind::Oauth);
        assert_eq!(AuthKind::ApiKey.as_str(), "api_key");
        let error = AuthKind::parse("basic").expect_err("only two kinds exist");
        assert_eq!(error.exit_code(), 2);
    }
}
