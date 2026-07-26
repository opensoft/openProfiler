use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;
use walkdir::WalkDir;

const MANIFEST_VERSION: u32 = 1;
const ACTIVE_MARKER: &str = ".openprofiler-active.json";
const LEGACY_ACTIVE_MARKER: &str = ".profile-switcher-active.json";
const DESKTOP_ROLLBACK_CREDENTIAL: &str = ".openprofiler-desktop-auth.rollback.json";
const DESKTOP_ROLLBACK_MARKER: &str = ".openprofiler-desktop-rollback.json";
const MAX_CREDENTIAL_BYTES: u64 = 1024 * 1024;
const MAX_PROFILE_DIRECTORY_DEPTH: usize = 7;

#[derive(Debug, Error)]
pub enum ProfileError {
    #[error("could not determine the current user's home directory")]
    HomeUnavailable,

    #[error("could not read profile data at {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("could not write profile data at {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("invalid profile {profile}: {reason}")]
    InvalidProfile { profile: String, reason: String },

    #[error("profile directory escapes the configured profile root: {0}")]
    EscapedProfileRoot(PathBuf),

    #[error("unknown {provider} profile path: {profile_path}")]
    UnknownProfile {
        provider: Provider,
        profile_path: String,
    },

    #[error("{provider} profile {profile} does not contain a usable local credential")]
    CredentialUnavailable { provider: Provider, profile: String },

    #[error("refusing to activate through symlinked path: {0}")]
    UnsafeActivationPath(PathBuf),

    #[error("unsupported Codex desktop credential at {path}: {reason}")]
    UnsupportedDesktopCredential { path: PathBuf, reason: String },

    #[error("a Codex desktop rollback is already pending; keep or undo it before switching again")]
    DesktopRollbackPending,

    #[error("no Codex desktop rollback is available")]
    DesktopRollbackUnavailable,

    #[error("Codex desktop credential verification failed after activation")]
    DesktopVerificationFailed,

    #[error(
        "Codex desktop activation failed ({activation}); automatic rollback also failed ({rollback})"
    )]
    DesktopRecoveryFailed {
        activation: Box<ProfileError>,
        rollback: Box<ProfileError>,
    },
}

pub type Result<T> = std::result::Result<T, ProfileError>;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Codex,
    Claude,
}

impl std::fmt::Display for Provider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Codex => "Codex",
            Self::Claude => "Claude",
        })
    }
}

impl Provider {
    fn credential_name(self) -> &'static str {
        match self {
            Self::Codex => "auth.json",
            Self::Claude => ".credentials.json",
        }
    }

    fn default_profiles_home(self, home: &Path) -> PathBuf {
        match self {
            Self::Codex => home.join(".chatgpt-profiles"),
            Self::Claude => home.join(".claude-profiles"),
        }
    }

    fn default_active_home(self, home: &Path) -> PathBuf {
        match self {
            Self::Codex => home.join(".codex"),
            Self::Claude => home.join(".claude"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderConfig {
    pub provider: Provider,
    pub manifest_path: PathBuf,
    pub profiles_home: PathBuf,
    pub active_home: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveryConfig {
    pub providers: Vec<ProviderConfig>,
}

impl DiscoveryConfig {
    pub fn from_env() -> Result<Self> {
        let home = dirs::home_dir().ok_or(ProfileError::HomeUnavailable)?;
        let config_home = env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"));
        let default_codex_profiles_home = Provider::Codex.default_profiles_home(&home);
        let default_codex_manifest = config_home.join("workbenches/openai-profiles.json");
        let default_claude_profiles_home = Provider::Claude.default_profiles_home(&home);
        let default_claude_manifest = config_home.join("workbenches/claude-profiles.json");
        let codex_profiles_home_override = env::var_os("CODEX_PROFILES_HOME")
            .or_else(|| env::var_os("CHATGPT_PROFILES_HOME"))
            .map(PathBuf::from);
        let codex_manifest_override = env::var_os("CODEX_PROFILES_MANIFEST")
            .or_else(|| env::var_os("CHATGPT_PROFILES_MANIFEST"))
            .map(PathBuf::from);
        let claude_profiles_home_override = env::var_os("CLAUDE_PROFILES_HOME").map(PathBuf::from);
        let claude_manifest_override = env::var_os("CLAUDE_PROFILES_MANIFEST").map(PathBuf::from);
        #[cfg(windows)]
        let (discovered_codex_profiles_home, discovered_codex_manifest) =
            if codex_profiles_home_override.is_some()
                || default_codex_profiles_home.join("profiles").is_dir()
            {
                (
                    default_codex_profiles_home.clone(),
                    default_codex_manifest.clone(),
                )
            } else {
                discover_wsl_defaults(Provider::Codex).unwrap_or_else(|| {
                    (
                        default_codex_profiles_home.clone(),
                        default_codex_manifest.clone(),
                    )
                })
            };
        #[cfg(not(windows))]
        let (discovered_codex_profiles_home, discovered_codex_manifest) = (
            default_codex_profiles_home.clone(),
            default_codex_manifest.clone(),
        );
        #[cfg(windows)]
        let (discovered_claude_profiles_home, discovered_claude_manifest) =
            if claude_profiles_home_override.is_some()
                || default_claude_profiles_home.join("profiles").is_dir()
            {
                (
                    default_claude_profiles_home.clone(),
                    default_claude_manifest.clone(),
                )
            } else {
                discover_wsl_defaults(Provider::Claude).unwrap_or_else(|| {
                    (
                        default_claude_profiles_home.clone(),
                        default_claude_manifest.clone(),
                    )
                })
            };
        #[cfg(not(windows))]
        let (discovered_claude_profiles_home, discovered_claude_manifest) = (
            default_claude_profiles_home.clone(),
            default_claude_manifest.clone(),
        );

        let codex = ProviderConfig {
            provider: Provider::Codex,
            manifest_path: codex_manifest_override.unwrap_or(discovered_codex_manifest),
            profiles_home: codex_profiles_home_override.unwrap_or(discovered_codex_profiles_home),
            active_home: env::var_os("OPENPROFILER_CODEX_ACTIVE_HOME")
                .or_else(|| env::var_os("PROFILE_SWITCHER_CODEX_ACTIVE_HOME"))
                .map(PathBuf::from)
                .unwrap_or_else(|| Provider::Codex.default_active_home(&home)),
        };

        let claude = ProviderConfig {
            provider: Provider::Claude,
            manifest_path: claude_manifest_override.unwrap_or(discovered_claude_manifest),
            profiles_home: claude_profiles_home_override.unwrap_or(discovered_claude_profiles_home),
            active_home: env::var_os("OPENPROFILER_CLAUDE_ACTIVE_HOME")
                .or_else(|| env::var_os("PROFILE_SWITCHER_CLAUDE_ACTIVE_HOME"))
                .map(PathBuf::from)
                .unwrap_or_else(|| Provider::Claude.default_active_home(&home)),
        };

        Ok(Self {
            providers: vec![codex, claude],
        })
    }
}

#[cfg(windows)]
fn discover_wsl_defaults(provider: Provider) -> Option<(PathBuf, PathBuf)> {
    let mut candidates = Vec::new();
    for distribution in wsl_distribution_names() {
        for wsl_root in [r"\\wsl.localhost", r"\\wsl$"] {
            let home_root = PathBuf::from(format!(r"{wsl_root}\{distribution}\home"));
            let Ok(users) = fs::read_dir(home_root) else {
                continue;
            };
            let mut found_in_distribution = false;
            for user in users.filter_map(std::result::Result::ok) {
                let user_home = user.path();
                let profiles_home = provider.default_profiles_home(&user_home);
                if profiles_home.join("profiles").is_dir() {
                    candidates.push((
                        profiles_home,
                        user_home.join(".config/workbenches").join(match provider {
                            Provider::Codex => "openai-profiles.json",
                            Provider::Claude => "claude-profiles.json",
                        }),
                    ));
                    if candidates.len() > 1 {
                        return None;
                    }
                    found_in_distribution = true;
                }
            }
            if found_in_distribution {
                break;
            }
        }
    }

    candidates.sort();
    candidates.dedup();
    (candidates.len() == 1).then(|| candidates.remove(0))
}

#[cfg(windows)]
fn wsl_distribution_names() -> Vec<String> {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let Some(system_root) = env::var_os("SystemRoot") else {
        return Vec::new();
    };
    let wsl_executable = PathBuf::from(system_root).join("System32").join("wsl.exe");
    let mut command = std::process::Command::new(wsl_executable);
    command
        .args(["--list", "--quiet"])
        .creation_flags(CREATE_NO_WINDOW);
    let Ok(output) = command.output() else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }

    parse_wsl_distribution_names(&output.stdout)
}

#[cfg(windows)]
fn parse_wsl_distribution_names(bytes: &[u8]) -> Vec<String> {
    decode_wsl_output(bytes)
        .lines()
        .map(str::trim)
        .filter(|name| {
            !name.is_empty()
                && *name != "."
                && *name != ".."
                && !name.contains('\\')
                && !name.contains('/')
                && !name.chars().any(char::is_control)
        })
        .map(str::to_owned)
        .collect()
}

#[cfg(windows)]
fn decode_wsl_output(bytes: &[u8]) -> String {
    let has_utf16_bom = bytes.starts_with(&[0xff, 0xfe]);
    let has_utf16_line_ending = bytes
        .windows(4)
        .any(|window| window == [b'\r', 0, b'\n', 0])
        || bytes.windows(2).any(|window| window == [b'\n', 0]);
    let has_utf8_line_ending = bytes.contains(&b'\n') && !has_utf16_line_ending;

    if has_utf16_bom || has_utf16_line_ending {
        return decode_utf16_le(bytes, has_utf16_bom);
    }
    if has_utf8_line_ending {
        return String::from_utf8_lossy(bytes).into_owned();
    }

    let utf8 = std::str::from_utf8(bytes).ok();
    let utf16 = bytes
        .len()
        .is_multiple_of(2)
        .then(|| decode_utf16_le(bytes, false));
    match (utf8, utf16) {
        (Some(utf8), Some(utf16)) => {
            let utf8_penalty = decoding_penalty(utf8);
            let utf16_penalty = decoding_penalty(&utf16);
            if utf8_penalty < utf16_penalty {
                utf8.to_owned()
            } else {
                // Native wsl.exe emits UTF-16LE. Prefer it when both strict
                // decodings are equally plausible and no line ending exists.
                utf16
            }
        }
        (Some(utf8), None) => utf8.to_owned(),
        (None, Some(utf16)) => utf16,
        (None, None) => String::from_utf8_lossy(bytes).into_owned(),
    }
}

#[cfg(windows)]
fn decode_utf16_le(bytes: &[u8], has_bom: bool) -> String {
    let start = usize::from(has_bom) * 2;
    let units = bytes[start..]
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect::<Vec<_>>();
    String::from_utf16_lossy(&units)
}

#[cfg(windows)]
fn decoding_penalty(value: &str) -> usize {
    value
        .chars()
        .filter(|character| {
            *character == '\u{fffd}'
                || (*character != '\r' && *character != '\n' && character.is_control())
        })
        .count()
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProfileInventory {
    pub stores: Vec<ProviderStore>,
    pub profiles: Vec<Profile>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderStore {
    pub provider: Provider,
    pub manifest_path: PathBuf,
    pub profiles_home: PathBuf,
    pub active_home: PathBuf,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub provider: Provider,
    pub name: String,
    pub email: String,
    pub family: String,
    pub aliases: Vec<String>,
    pub profile_path: String,
    pub status: String,
    pub configured: bool,
    pub credential_present: bool,
    pub active: bool,
    pub source: ProfileSource,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ProfileSource {
    Manifest,
    ProfileMetadata,
    ProfileDirectory,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ActivationResult {
    pub provider: Provider,
    pub profile: String,
    pub restart_required: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodexDesktopStatus {
    pub file_activation_supported: bool,
    pub credential_store: String,
    pub eligible_profile_paths: Vec<String>,
    pub active_profile_paths: Vec<String>,
    pub rollback_available: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodexDesktopActivationResult {
    pub profile: String,
    pub outgoing_profiles_updated: usize,
    pub rollback_available: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodexDesktopRollbackResult {
    pub restored_previous_credential: bool,
}

#[derive(Debug, Deserialize)]
struct ProfileManifest {
    version: u32,
    #[serde(default)]
    profiles: Vec<ManifestProfile>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManifestProfile {
    name: String,
    #[serde(default)]
    email: String,
    #[serde(default)]
    family: String,
    #[serde(default)]
    aliases: Vec<String>,
    profile_path: Option<String>,
    status: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ActiveMarker {
    schema_version: u32,
    provider: Provider,
    profile: String,
    profile_path: String,
    credential_size: u64,
    active_modified_unix_secs: u64,
    active_modified_subsec_nanos: u32,
}

#[derive(Debug, Deserialize)]
struct CodexCredentialBundle {
    auth_mode: String,
    tokens: CodexCredentialTokens,
}

#[derive(Debug, Deserialize)]
struct CodexCredentialTokens {
    access_token: String,
    account_id: String,
    id_token: String,
    refresh_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CodexCredentialIdentity {
    account_id: String,
}

#[derive(Debug)]
struct CodexDesktopCredentialState {
    credential_store: String,
    target_exists: bool,
    active_identity: Option<CodexCredentialIdentity>,
    file_activation_supported: bool,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DesktopRollbackMarker {
    schema_version: u32,
    previous_credential_present: bool,
    selected_profile_path: String,
    backup_size: Option<u64>,
    backup_modified_unix_secs: Option<u64>,
    backup_modified_subsec_nanos: Option<u32>,
}

pub fn discover_profiles(config: &DiscoveryConfig) -> ProfileInventory {
    let mut stores = Vec::new();
    let mut all_profiles = Vec::new();

    for provider_config in &config.providers {
        let (profiles, issues) = discover_provider(provider_config);
        stores.push(ProviderStore {
            provider: provider_config.provider,
            manifest_path: provider_config.manifest_path.clone(),
            profiles_home: provider_config.profiles_home.clone(),
            active_home: provider_config.active_home.clone(),
            issues,
        });
        all_profiles.extend(profiles);
    }

    all_profiles.sort_by(|left, right| {
        left.provider
            .cmp(&right.provider)
            .then_with(|| left.family.to_lowercase().cmp(&right.family.to_lowercase()))
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
            .then_with(|| left.profile_path.cmp(&right.profile_path))
    });

    ProfileInventory {
        stores,
        profiles: all_profiles,
    }
}

pub fn activate_profile(
    config: &DiscoveryConfig,
    provider: Provider,
    requested_profile_path: &str,
) -> Result<ActivationResult> {
    let provider_config = config
        .providers
        .iter()
        .find(|item| item.provider == provider)
        .ok_or_else(|| ProfileError::UnknownProfile {
            provider,
            profile_path: requested_profile_path.to_string(),
        })?;

    validate_profile_path(requested_profile_path, requested_profile_path)?;
    let inventory = discover_profiles(config);
    let profile = inventory
        .profiles
        .iter()
        .find(|profile| {
            profile.provider == provider && profile.profile_path == requested_profile_path
        })
        .ok_or_else(|| ProfileError::UnknownProfile {
            provider,
            profile_path: requested_profile_path.to_string(),
        })?;

    if !profile.credential_present {
        return Err(ProfileError::CredentialUnavailable {
            provider,
            profile: profile.name.clone(),
        });
    }

    let profile_root = provider_config.profiles_home.join("profiles");
    let profile_dir = checked_profile_dir(&profile_root, &profile.profile_path)?;
    let source = profile_dir.join(provider.credential_name());
    ensure_regular_nonempty_file(&source).ok_or_else(|| ProfileError::CredentialUnavailable {
        provider,
        profile: profile.name.clone(),
    })?;

    ensure_safe_active_home(&provider_config.active_home)?;
    let target = provider_config.active_home.join(provider.credential_name());
    reject_symlink(&target)?;
    atomic_copy(&source, &target, provider, &profile.name)?;
    let (credential_size, active_modified_unix_secs, active_modified_subsec_nanos) =
        credential_stamp(&target).ok_or_else(|| ProfileError::CredentialUnavailable {
            provider,
            profile: profile.name.clone(),
        })?;

    let marker = ActiveMarker {
        schema_version: 1,
        provider,
        profile: profile.name.clone(),
        profile_path: profile.profile_path.clone(),
        credential_size,
        active_modified_unix_secs,
        active_modified_subsec_nanos,
    };
    let marker_bytes =
        serde_json::to_vec_pretty(&marker).map_err(|source| ProfileError::Write {
            path: provider_config.active_home.join(ACTIVE_MARKER),
            source: io::Error::new(io::ErrorKind::InvalidData, source),
        })?;
    atomic_write(
        &provider_config.active_home.join(ACTIVE_MARKER),
        &marker_bytes,
    )?;

    Ok(ActivationResult {
        provider,
        profile: profile.name.clone(),
        restart_required: true,
    })
}

pub fn codex_desktop_status(config: &DiscoveryConfig, desktop_home: &Path) -> CodexDesktopStatus {
    let inventory = discover_profiles(config);
    codex_desktop_status_from_inventory(config, desktop_home, &inventory)
}

pub fn codex_desktop_status_from_inventory(
    config: &DiscoveryConfig,
    desktop_home: &Path,
    inventory: &ProfileInventory,
) -> CodexDesktopStatus {
    let Some(provider_config) = provider_config(config, Provider::Codex) else {
        return CodexDesktopStatus {
            file_activation_supported: false,
            credential_store: "unknown".to_string(),
            eligible_profile_paths: Vec::new(),
            active_profile_paths: Vec::new(),
            rollback_available: desktop_rollback_pending(desktop_home),
            message: "Codex profile discovery is not configured".to_string(),
        };
    };

    let credential_state = codex_desktop_credential_state(desktop_home);
    let profiles = codex_profile_credentials_from_inventory(inventory, provider_config);
    let mut eligible_profile_paths = profiles
        .iter()
        .map(|profile| profile.profile_path.clone())
        .collect::<Vec<_>>();
    let mut active_profile_paths = credential_state
        .active_identity
        .as_ref()
        .map(|identity| {
            profiles
                .iter()
                .filter(|profile| profile.identity == *identity)
                .map(|profile| profile.profile_path.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    eligible_profile_paths.sort();
    eligible_profile_paths.dedup();
    active_profile_paths.sort();
    active_profile_paths.dedup();

    let message = match (
        credential_state.credential_store.as_str(),
        credential_state.file_activation_supported,
        credential_state.target_exists,
    ) {
        ("keyring", _, _) => {
            "Windows Credential Manager is configured; use the supported ChatGPT sign-in flow"
                .to_string()
        }
        ("auto" | "default", false, false) => {
            "No file-based desktop login was found, so Credential Manager cannot be ruled out"
                .to_string()
        }
        ("file" | "auto" | "default", false, true) => {
            "The desktop credential is not a complete reusable ChatGPT login".to_string()
        }
        ("unknown", _, _) => {
            "The desktop credential-store mode could not be established".to_string()
        }
        (_, true, _) if eligible_profile_paths.is_empty() => {
            "No reusable ChatGPT profile credentials are available".to_string()
        }
        _ => "File-based GPT app profile switching is ready".to_string(),
    };

    CodexDesktopStatus {
        file_activation_supported: credential_state.file_activation_supported,
        credential_store: credential_state.credential_store,
        eligible_profile_paths,
        active_profile_paths,
        rollback_available: desktop_rollback_pending(desktop_home),
        message,
    }
}

pub fn activate_codex_desktop_profile(
    config: &DiscoveryConfig,
    requested_profile_path: &str,
    desktop_home: &Path,
) -> Result<CodexDesktopActivationResult> {
    validate_profile_path(requested_profile_path, requested_profile_path)?;
    if desktop_rollback_pending(desktop_home) {
        return Err(ProfileError::DesktopRollbackPending);
    }

    let provider_config =
        provider_config(config, Provider::Codex).ok_or_else(|| ProfileError::UnknownProfile {
            provider: Provider::Codex,
            profile_path: requested_profile_path.to_string(),
        })?;
    let profiles = codex_profile_credentials(config, provider_config);
    let selected = profiles
        .iter()
        .find(|profile| profile.profile_path == requested_profile_path)
        .ok_or_else(|| ProfileError::CredentialUnavailable {
            provider: Provider::Codex,
            profile: requested_profile_path.to_string(),
        })?;

    let credential_state = codex_desktop_credential_state(desktop_home);
    if !credential_state.file_activation_supported {
        let reason = match (
            credential_state.credential_store.as_str(),
            credential_state.target_exists,
        ) {
            ("keyring", _) => {
                "Windows Credential Manager is configured; use the supported ChatGPT sign-in flow"
                    .to_string()
            }
            ("auto" | "default", false) => {
                "No file-based desktop login was found, so Credential Manager cannot be ruled out"
                    .to_string()
            }
            ("file" | "auto" | "default", true) => {
                "The desktop credential is not a complete reusable ChatGPT login".to_string()
            }
            _ => "The desktop credential-store mode could not be established".to_string(),
        };
        return Err(ProfileError::UnsupportedDesktopCredential {
            path: desktop_home.join(Provider::Codex.credential_name()),
            reason,
        });
    }

    ensure_safe_active_home(desktop_home)?;
    let target = desktop_home.join(Provider::Codex.credential_name());
    let rollback = desktop_home.join(DESKTOP_ROLLBACK_CREDENTIAL);
    let marker_path = desktop_home.join(DESKTOP_ROLLBACK_MARKER);
    reject_symlink(&target)?;
    reject_symlink(&rollback)?;
    reject_symlink(&marker_path)?;
    remove_regular_file_if_present(&rollback)?;

    let previous_credential_present = credential_state.active_identity.is_some();
    let (backup_size, backup_modified_unix_secs, backup_modified_subsec_nanos) =
        if previous_credential_present {
            atomic_copy(&target, &rollback, Provider::Codex, "desktop rollback")?;
            let rollback_identity = read_codex_credential_identity(&rollback)?;
            if Some(&rollback_identity) != credential_state.active_identity.as_ref() {
                return Err(ProfileError::DesktopVerificationFailed);
            }
            let (size, modified_secs, modified_nanos) =
                credential_stamp(&rollback).ok_or(ProfileError::DesktopRollbackUnavailable)?;
            (Some(size), Some(modified_secs), Some(modified_nanos))
        } else {
            (None, None, None)
        };

    let mut outgoing_profiles_updated = 0;
    if let Some(active_identity) = credential_state.active_identity {
        for profile in profiles
            .iter()
            .filter(|profile| profile.identity == active_identity)
        {
            atomic_copy(
                &rollback,
                &profile.credential_path,
                Provider::Codex,
                &profile.name,
            )?;
            outgoing_profiles_updated += 1;
        }
    }

    let marker = DesktopRollbackMarker {
        schema_version: 1,
        previous_credential_present,
        selected_profile_path: selected.profile_path.clone(),
        backup_size,
        backup_modified_unix_secs,
        backup_modified_subsec_nanos,
    };
    let marker_bytes =
        serde_json::to_vec_pretty(&marker).map_err(|source| ProfileError::Write {
            path: marker_path.clone(),
            source: io::Error::new(io::ErrorKind::InvalidData, source),
        })?;
    atomic_write(&marker_path, &marker_bytes)?;

    let activation_result = (|| {
        atomic_copy(
            &selected.credential_path,
            &target,
            Provider::Codex,
            &selected.name,
        )?;
        let installed_identity = read_codex_credential_identity(&target)?;
        if installed_identity != selected.identity {
            return Err(ProfileError::DesktopVerificationFailed);
        }
        Ok(())
    })();

    if let Err(error) = activation_result {
        return match rollback_codex_desktop_profile(desktop_home) {
            Ok(_) => Err(error),
            Err(rollback) => Err(ProfileError::DesktopRecoveryFailed {
                activation: Box::new(error),
                rollback: Box::new(rollback),
            }),
        };
    }

    Ok(CodexDesktopActivationResult {
        profile: selected.name.clone(),
        outgoing_profiles_updated,
        rollback_available: true,
    })
}

pub fn rollback_codex_desktop_profile(desktop_home: &Path) -> Result<CodexDesktopRollbackResult> {
    let marker_path = desktop_home.join(DESKTOP_ROLLBACK_MARKER);
    let marker = read_desktop_rollback_marker(&marker_path)?;
    let target = desktop_home.join(Provider::Codex.credential_name());
    let rollback = desktop_home.join(DESKTOP_ROLLBACK_CREDENTIAL);

    if marker.previous_credential_present {
        let expected_stamp = (
            marker.backup_size,
            marker.backup_modified_unix_secs,
            marker.backup_modified_subsec_nanos,
        );
        let actual_stamp = credential_stamp(&rollback)
            .map(|(size, secs, nanos)| (Some(size), Some(secs), Some(nanos)));
        if actual_stamp != Some(expected_stamp) {
            return Err(ProfileError::DesktopRollbackUnavailable);
        }
        read_codex_credential_identity(&rollback)?;
        atomic_copy(&rollback, &target, Provider::Codex, "desktop rollback")?;
    } else if target.exists() {
        remove_regular_file(&target)?;
    }

    remove_regular_file_if_present(&rollback)?;
    remove_regular_file(&marker_path)?;
    Ok(CodexDesktopRollbackResult {
        restored_previous_credential: marker.previous_credential_present,
    })
}

pub fn confirm_codex_desktop_profile(desktop_home: &Path) -> Result<()> {
    let marker_path = desktop_home.join(DESKTOP_ROLLBACK_MARKER);
    read_desktop_rollback_marker(&marker_path)?;
    remove_regular_file_if_present(&desktop_home.join(DESKTOP_ROLLBACK_CREDENTIAL))?;
    remove_regular_file(&marker_path)
}

#[derive(Debug, Clone)]
struct CodexProfileCredential {
    name: String,
    profile_path: String,
    credential_path: PathBuf,
    identity: CodexCredentialIdentity,
}

fn provider_config(config: &DiscoveryConfig, provider: Provider) -> Option<&ProviderConfig> {
    config
        .providers
        .iter()
        .find(|provider_config| provider_config.provider == provider)
}

fn codex_profile_credentials(
    config: &DiscoveryConfig,
    provider_config: &ProviderConfig,
) -> Vec<CodexProfileCredential> {
    let inventory = discover_profiles(config);
    codex_profile_credentials_from_inventory(&inventory, provider_config)
}

fn codex_profile_credentials_from_inventory(
    inventory: &ProfileInventory,
    provider_config: &ProviderConfig,
) -> Vec<CodexProfileCredential> {
    let profile_root = provider_config.profiles_home.join("profiles");
    inventory
        .profiles
        .iter()
        .filter(|profile| profile.provider == Provider::Codex && profile.credential_present)
        .filter_map(|profile| {
            let profile_dir = checked_profile_dir(&profile_root, &profile.profile_path).ok()?;
            let credential_path = profile_dir.join(Provider::Codex.credential_name());
            let identity = read_codex_credential_identity(&credential_path).ok()?;
            Some(CodexProfileCredential {
                name: profile.name.clone(),
                profile_path: profile.profile_path.clone(),
                credential_path,
                identity,
            })
        })
        .collect()
}

fn codex_desktop_credential_state(desktop_home: &Path) -> CodexDesktopCredentialState {
    let credential_store = configured_credential_store(desktop_home);
    let target = desktop_home.join(Provider::Codex.credential_name());
    let target_exists = match fs::symlink_metadata(&target) {
        Ok(_) => true,
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(_) => true,
    };
    let active_identity = read_codex_credential_identity(&target).ok();
    let file_activation_supported = match credential_store.as_str() {
        "file" => !target_exists || active_identity.is_some(),
        "keyring" => false,
        "auto" | "default" => active_identity.is_some(),
        _ => false,
    };

    CodexDesktopCredentialState {
        credential_store,
        target_exists,
        active_identity,
        file_activation_supported,
    }
}

fn configured_credential_store(desktop_home: &Path) -> String {
    let config_path = desktop_home.join("config.toml");
    match fs::symlink_metadata(&config_path) {
        Ok(metadata) if metadata.file_type().is_file() => {}
        Ok(_) => return "unknown".to_string(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return "default".to_string(),
        Err(_) => return "unknown".to_string(),
    }
    let config = match read_text(&config_path) {
        Ok(config) => config,
        Err(_) => return "unknown".to_string(),
    };

    config
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with('#'))
        .find_map(|line| {
            let (key, value) = line.split_once('=')?;
            (key.trim() == "cli_auth_credentials_store")
                .then(|| value.split('#').next().unwrap_or_default().trim())
        })
        .map(|value| value.trim_matches(['"', '\'']).to_ascii_lowercase())
        .filter(|value| matches!(value.as_str(), "file" | "keyring" | "auto"))
        .unwrap_or_else(|| "default".to_string())
}

fn read_codex_credential_identity(path: &Path) -> Result<CodexCredentialIdentity> {
    let mut file = open_file_no_follow(path)?;
    let metadata = file.metadata().map_err(|source| ProfileError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_CREDENTIAL_BYTES {
        return Err(ProfileError::UnsupportedDesktopCredential {
            path: path.to_path_buf(),
            reason: "expected a non-empty credential file smaller than 1 MiB".to_string(),
        });
    }

    let mut text = String::new();
    file.read_to_string(&mut text)
        .map_err(|source| ProfileError::Read {
            path: path.to_path_buf(),
            source,
        })?;
    let credential: CodexCredentialBundle = serde_json::from_str(&text).map_err(|source| {
        ProfileError::UnsupportedDesktopCredential {
            path: path.to_path_buf(),
            reason: format!("invalid credential JSON: {source}"),
        }
    })?;

    if credential.auth_mode != "chatgpt" {
        return Err(ProfileError::UnsupportedDesktopCredential {
            path: path.to_path_buf(),
            reason: "only complete ChatGPT OAuth credentials can be reused".to_string(),
        });
    }
    if [
        credential.tokens.access_token.as_str(),
        credential.tokens.account_id.as_str(),
        credential.tokens.id_token.as_str(),
        credential.tokens.refresh_token.as_str(),
    ]
    .iter()
    .any(|value| value.trim().is_empty())
    {
        return Err(ProfileError::UnsupportedDesktopCredential {
            path: path.to_path_buf(),
            reason: "the ChatGPT OAuth credential bundle is incomplete".to_string(),
        });
    }

    Ok(CodexCredentialIdentity {
        account_id: credential.tokens.account_id,
    })
}

fn desktop_rollback_pending(desktop_home: &Path) -> bool {
    match fs::symlink_metadata(desktop_home.join(DESKTOP_ROLLBACK_MARKER)) {
        Ok(_) => true,
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(_) => true,
    }
}

fn read_desktop_rollback_marker(path: &Path) -> Result<DesktopRollbackMarker> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => {}
        Ok(_) => return Err(ProfileError::UnsafeActivationPath(path.to_path_buf())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(ProfileError::DesktopRollbackUnavailable);
        }
        Err(source) => {
            return Err(ProfileError::Read {
                path: path.to_path_buf(),
                source,
            });
        }
    }
    let text = read_text(path)?;
    let marker: DesktopRollbackMarker =
        serde_json::from_str(&text).map_err(|source| ProfileError::Read {
            path: path.to_path_buf(),
            source: io::Error::new(io::ErrorKind::InvalidData, source),
        })?;
    if marker.schema_version != 1 {
        return Err(ProfileError::DesktopRollbackUnavailable);
    }
    Ok(marker)
}

fn discover_provider(config: &ProviderConfig) -> (Vec<Profile>, Vec<String>) {
    let mut profiles = BTreeMap::<String, Profile>::new();
    let mut issues = Vec::new();
    let active_marker = read_active_marker(config);

    if config.manifest_path.is_file() {
        match read_manifest(&config.manifest_path) {
            Ok(manifest) if manifest.version == MANIFEST_VERSION => {
                for profile in manifest.profiles {
                    match resolve_profile(profile, config, ProfileSource::Manifest, &active_marker)
                    {
                        Ok(profile) => {
                            profiles.insert(normalized_path_key(&profile.profile_path), profile);
                        }
                        Err(error) => issues.push(error.to_string()),
                    }
                }
            }
            Ok(manifest) => issues.push(format!(
                "unsupported profile manifest version {} at {}",
                manifest.version,
                config.manifest_path.display()
            )),
            Err(error) => issues.push(error.to_string()),
        }
    }

    let profile_root = config.profiles_home.join("profiles");
    if profile_root.is_dir() {
        let mut entries = WalkDir::new(&profile_root)
            .min_depth(1)
            .max_depth(MAX_PROFILE_DIRECTORY_DEPTH)
            .follow_links(false)
            .into_iter();

        while let Some(entry) = entries.next() {
            let Ok(entry) = entry else {
                continue;
            };
            if !entry.file_type().is_dir() {
                continue;
            }

            let profile_dir = entry.into_path();
            let metadata_path = profile_dir.join(".profile.json");
            let credential_path = profile_dir.join(config.provider.credential_name());
            let has_metadata = is_regular_file(&metadata_path);
            let has_credential = ensure_regular_nonempty_file(&credential_path).is_some();
            if !has_metadata && !has_credential {
                continue;
            }

            // A profile owns everything beneath its directory. Avoid walking
            // provider caches, histories, logs, and databases over WSL UNC.
            entries.skip_current_dir();

            if has_metadata {
                let result = read_manifest_profile(&metadata_path).and_then(|mut profile| {
                    if profile.profile_path.is_none() {
                        let relative = profile_dir
                            .strip_prefix(&profile_root)
                            .map_err(|_| ProfileError::EscapedProfileRoot(profile_dir.clone()))?;
                        profile.profile_path = Some(relative_profile_path(relative));
                    }
                    resolve_profile(
                        profile,
                        config,
                        ProfileSource::ProfileMetadata,
                        &active_marker,
                    )
                });

                match result {
                    Ok(profile) => {
                        profiles
                            .entry(normalized_path_key(&profile.profile_path))
                            .or_insert(profile);
                    }
                    Err(error) => issues.push(error.to_string()),
                }
            }

            if !has_credential {
                continue;
            }
            let Ok(relative) = profile_dir.strip_prefix(&profile_root) else {
                continue;
            };
            let profile_path = relative_profile_path(relative);
            let key = normalized_path_key(&profile_path);
            if profiles.contains_key(&key) {
                continue;
            }

            let name = relative
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("profile")
                .to_string();
            let family = relative
                .components()
                .next()
                .and_then(|component| match component {
                    Component::Normal(value) => value.to_str(),
                    _ => None,
                })
                .filter(|value| *value != name)
                .unwrap_or("local")
                .to_string();
            let active = marker_matches(
                &active_marker,
                config.provider,
                &profile_path,
                &config.active_home.join(config.provider.credential_name()),
            );

            profiles.insert(
                key,
                Profile {
                    provider: config.provider,
                    name,
                    email: String::new(),
                    family,
                    aliases: Vec::new(),
                    profile_path,
                    status: "active".to_string(),
                    configured: true,
                    credential_present: ensure_regular_nonempty_file(&credential_path).is_some(),
                    active,
                    source: ProfileSource::ProfileDirectory,
                },
            );
        }
    }

    (profiles.into_values().collect(), issues)
}

fn resolve_profile(
    profile: ManifestProfile,
    config: &ProviderConfig,
    source: ProfileSource,
    active_marker: &Option<ActiveMarker>,
) -> Result<Profile> {
    validate_identifier(&profile.name, "name", &profile.name)?;
    if !profile.family.is_empty() {
        validate_identifier(&profile.name, "family", &profile.family)?;
    }
    for alias in &profile.aliases {
        validate_identifier(&profile.name, "alias", alias)?;
    }

    let profile_path = profile
        .profile_path
        .clone()
        .unwrap_or_else(|| profile.name.clone());
    validate_profile_path(&profile.name, &profile_path)?;

    let profile_root = config.profiles_home.join("profiles");
    let profile_dir = if profile_root.join(&profile_path).exists() {
        checked_profile_dir(&profile_root, &profile_path)?
    } else {
        profile_root.join(&profile_path)
    };
    let configured = profile_dir.is_dir();
    let credential_present = configured
        && ensure_regular_nonempty_file(&profile_dir.join(config.provider.credential_name()))
            .is_some();
    let active = marker_matches(
        active_marker,
        config.provider,
        &profile_path,
        &config.active_home.join(config.provider.credential_name()),
    );

    Ok(Profile {
        provider: config.provider,
        name: profile.name,
        email: profile.email,
        family: if profile.family.is_empty() {
            "local".to_string()
        } else {
            profile.family
        },
        aliases: profile.aliases,
        profile_path,
        status: profile.status.unwrap_or_else(|| "active".to_string()),
        configured,
        credential_present,
        active,
        source,
    })
}

fn read_manifest(path: &Path) -> Result<ProfileManifest> {
    let text = read_text(path)?;
    serde_json::from_str(&text).map_err(|source| ProfileError::Read {
        path: path.to_path_buf(),
        source: io::Error::new(io::ErrorKind::InvalidData, source),
    })
}

fn read_manifest_profile(path: &Path) -> Result<ManifestProfile> {
    let text = read_text(path)?;
    serde_json::from_str(&text).map_err(|source| ProfileError::Read {
        path: path.to_path_buf(),
        source: io::Error::new(io::ErrorKind::InvalidData, source),
    })
}

fn read_active_marker(config: &ProviderConfig) -> Option<ActiveMarker> {
    [ACTIVE_MARKER, LEGACY_ACTIVE_MARKER]
        .into_iter()
        .find_map(|marker_name| {
            let path = config.active_home.join(marker_name);
            let mut file = open_file_no_follow(&path).ok()?;
            let mut text = String::new();
            file.read_to_string(&mut text).ok()?;
            let marker: ActiveMarker = serde_json::from_str(&text).ok()?;
            (marker.schema_version == 1 && marker.provider == config.provider).then_some(marker)
        })
}

fn marker_matches(
    marker: &Option<ActiveMarker>,
    provider: Provider,
    profile_path: &str,
    active_credential: &Path,
) -> bool {
    marker.as_ref().is_some_and(|marker| {
        marker.provider == provider
            && marker.profile_path == profile_path
            && credential_stamp(active_credential).is_some_and(
                |(size, modified_secs, modified_nanos)| {
                    size == marker.credential_size
                        && modified_secs == marker.active_modified_unix_secs
                        && modified_nanos == marker.active_modified_subsec_nanos
                },
            )
    })
}

fn checked_profile_dir(profile_root: &Path, profile_path: &str) -> Result<PathBuf> {
    let profile_dir = profile_root.join(profile_path);
    let canonical_root = profile_root
        .canonicalize()
        .map_err(|source| ProfileError::Read {
            path: profile_root.to_path_buf(),
            source,
        })?;
    let canonical_profile = profile_dir
        .canonicalize()
        .map_err(|source| ProfileError::Read {
            path: profile_dir.clone(),
            source,
        })?;
    if !canonical_profile.starts_with(&canonical_root) {
        return Err(ProfileError::EscapedProfileRoot(profile_dir));
    }
    Ok(canonical_profile)
}

fn validate_identifier(profile: &str, field: &str, value: &str) -> Result<()> {
    let mut characters = value.chars();
    let valid = characters
        .next()
        .is_some_and(|character| character.is_ascii_alphanumeric())
        && characters
            .all(|character| character.is_ascii_alphanumeric() || "-_.".contains(character));

    if !valid {
        return Err(ProfileError::InvalidProfile {
            profile: profile.to_string(),
            reason: format!(
                "{field} must start with an ASCII letter or number and contain only letters, numbers, hyphens, underscores, or dots"
            ),
        });
    }
    Ok(())
}

fn validate_profile_path(profile: &str, profile_path: &str) -> Result<()> {
    let path = Path::new(profile_path);
    let valid = !profile_path.trim().is_empty()
        && !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)));

    if !valid {
        return Err(ProfileError::InvalidProfile {
            profile: profile.to_string(),
            reason: "profilePath must be a relative path without dot components".to_string(),
        });
    }
    Ok(())
}

fn ensure_regular_nonempty_file(path: &Path) -> Option<()> {
    fs::symlink_metadata(path)
        .ok()
        .filter(|metadata| metadata.file_type().is_file() && metadata.len() > 0)
        .map(|_| ())
}

fn is_regular_file(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_file())
        .unwrap_or(false)
}

fn credential_stamp(path: &Path) -> Option<(u64, u64, u32)> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if !metadata.file_type().is_file() || metadata.len() == 0 {
        return None;
    }
    let modified = metadata.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    Some((metadata.len(), modified.as_secs(), modified.subsec_nanos()))
}

fn reject_symlink(path: &Path) -> Result<()> {
    if fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(ProfileError::UnsafeActivationPath(path.to_path_buf()));
    }
    Ok(())
}

fn ensure_safe_active_home(path: &Path) -> Result<()> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if !metadata.file_type().is_dir() {
            return Err(ProfileError::UnsafeActivationPath(path.to_path_buf()));
        }
    } else {
        fs::create_dir_all(path).map_err(|source| ProfileError::Write {
            path: path.to_path_buf(),
            source,
        })?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|source| {
            ProfileError::Write {
                path: path.to_path_buf(),
                source,
            }
        })?;
    }
    Ok(())
}

fn atomic_copy(source: &Path, target: &Path, provider: Provider, profile: &str) -> Result<()> {
    reject_symlink(target)?;
    let mut input = open_file_no_follow(source)?;
    if !input
        .metadata()
        .map(|metadata| metadata.is_file() && metadata.len() > 0)
        .unwrap_or(false)
    {
        return Err(ProfileError::CredentialUnavailable {
            provider,
            profile: profile.to_string(),
        });
    }

    let temp = temporary_path(target);
    let mut output = secure_create(&temp)?;
    io::copy(&mut input, &mut output).map_err(|source| ProfileError::Write {
        path: temp.clone(),
        source,
    })?;
    output.sync_all().map_err(|source| ProfileError::Write {
        path: temp.clone(),
        source,
    })?;
    drop(output);

    replace_file(&temp, target).inspect_err(|_| {
        let _ = fs::remove_file(&temp);
    })
}

fn open_file_no_follow(path: &Path) -> Result<File> {
    reject_symlink(path)?;
    let mut options = OpenOptions::new();
    options.read(true);

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }

    #[cfg(windows)]
    {
        use std::mem::size_of;
        use std::os::windows::fs::OpenOptionsExt;
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            FileAttributeTagInfo, GetFileInformationByHandleEx, FILE_ATTRIBUTE_REPARSE_POINT,
            FILE_ATTRIBUTE_TAG_INFO, FILE_FLAG_OPEN_REPARSE_POINT,
        };

        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
        let file = options.open(path).map_err(|source| ProfileError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        let mut information = FILE_ATTRIBUTE_TAG_INFO {
            FileAttributes: 0,
            ReparseTag: 0,
        };
        let success = unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileAttributeTagInfo,
                (&mut information as *mut FILE_ATTRIBUTE_TAG_INFO).cast(),
                size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
            )
        };
        if success == 0 {
            return Err(ProfileError::Read {
                path: path.to_path_buf(),
                source: io::Error::last_os_error(),
            });
        }
        if information.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(ProfileError::UnsafeActivationPath(path.to_path_buf()));
        }
        Ok(file)
    }

    #[cfg(not(windows))]
    options.open(path).map_err(|source| ProfileError::Read {
        path: path.to_path_buf(),
        source,
    })
}

fn atomic_write(target: &Path, value: &[u8]) -> Result<()> {
    reject_symlink(target)?;
    let temp = temporary_path(target);
    let mut output = secure_create(&temp)?;
    output
        .write_all(value)
        .and_then(|_| output.write_all(b"\n"))
        .and_then(|_| output.sync_all())
        .map_err(|source| ProfileError::Write {
            path: temp.clone(),
            source,
        })?;
    drop(output);
    replace_file(&temp, target).inspect_err(|_| {
        let _ = fs::remove_file(&temp);
    })
}

fn secure_create(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path).map_err(|source| ProfileError::Write {
        path: path.to_path_buf(),
        source,
    })
}

fn temporary_path(target: &Path) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let name = target
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("profile");
    target.with_file_name(format!(
        ".{name}.openprofiler-{}-{nonce}.tmp",
        std::process::id()
    ))
}

#[cfg(not(windows))]
fn replace_file(source: &Path, target: &Path) -> Result<()> {
    fs::rename(source, target).map_err(|source| ProfileError::Write {
        path: target.to_path_buf(),
        source,
    })
}

#[cfg(windows)]
fn replace_file(source: &Path, target: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, ReplaceFileW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
        REPLACEFILE_WRITE_THROUGH,
    };

    let source_wide: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let target_wide: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    if target.exists() {
        let success = unsafe {
            ReplaceFileW(
                target_wide.as_ptr(),
                source_wide.as_ptr(),
                std::ptr::null(),
                REPLACEFILE_WRITE_THROUGH,
                std::ptr::null(),
                std::ptr::null(),
            )
        };
        if success == 0 {
            return Err(ProfileError::Write {
                path: target.to_path_buf(),
                source: io::Error::last_os_error(),
            });
        }
        return Ok(());
    }

    let success = unsafe {
        MoveFileExW(
            source_wide.as_ptr(),
            target_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if success == 0 {
        return Err(ProfileError::Write {
            path: target.to_path_buf(),
            source: io::Error::last_os_error(),
        });
    }
    Ok(())
}

fn normalized_path_key(path: &str) -> String {
    path.replace('\\', "/").to_lowercase()
}

fn relative_profile_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn read_text(path: &Path) -> Result<String> {
    let mut file = open_file_no_follow(path)?;
    let mut text = String::new();
    file.read_to_string(&mut text)
        .map_err(|source| ProfileError::Read {
            path: path.to_path_buf(),
            source,
        })?;
    Ok(text)
}

fn remove_regular_file(path: &Path) -> Result<()> {
    reject_symlink(path)?;
    let metadata = fs::symlink_metadata(path).map_err(|source| ProfileError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    if !metadata.file_type().is_file() {
        return Err(ProfileError::UnsafeActivationPath(path.to_path_buf()));
    }
    fs::remove_file(path).map_err(|source| ProfileError::Write {
        path: path.to_path_buf(),
        source,
    })
}

fn remove_regular_file_if_present(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => remove_regular_file(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(ProfileError::Read {
            path: path.to_path_buf(),
            source,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn config(temp: &TempDir) -> DiscoveryConfig {
        let home = temp.path();
        DiscoveryConfig {
            providers: vec![
                ProviderConfig {
                    provider: Provider::Codex,
                    manifest_path: home.join("config/openai-profiles.json"),
                    profiles_home: home.join(".chatgpt-profiles"),
                    active_home: home.join(".codex"),
                },
                ProviderConfig {
                    provider: Provider::Claude,
                    manifest_path: home.join("config/claude-profiles.json"),
                    profiles_home: home.join(".claude-profiles"),
                    active_home: home.join(".claude"),
                },
            ],
        }
    }

    fn write_file(path: &Path, value: &str) {
        fs::create_dir_all(path.parent().expect("test path has parent")).unwrap();
        fs::write(path, value).unwrap();
    }

    fn codex_auth(account_id: &str, version: &str) -> String {
        format!(
            r#"{{"auth_mode":"chatgpt","tokens":{{"access_token":"access-{version}","account_id":"{account_id}","id_token":"id-{version}","refresh_token":"refresh-{version}"}}}}"#
        )
    }

    fn provider_config(config: &DiscoveryConfig, provider: Provider) -> &ProviderConfig {
        config
            .providers
            .iter()
            .find(|item| item.provider == provider)
            .unwrap()
    }

    #[cfg(windows)]
    #[test]
    fn parses_utf16_wsl_distribution_names_safely() {
        let bytes = "Ubuntu-24.04\r\ndocker-desktop\r\n..\\escape\r\n"
            .encode_utf16()
            .flat_map(|unit| unit.to_le_bytes())
            .collect::<Vec<_>>();

        assert_eq!(
            parse_wsl_distribution_names(&bytes),
            vec!["Ubuntu-24.04", "docker-desktop"]
        );
    }

    #[cfg(windows)]
    #[test]
    fn parses_utf8_wsl_distribution_names() {
        assert_eq!(
            parse_wsl_distribution_names(b"Ubuntu\r\nUbuntu-Preview\r\n"),
            vec!["Ubuntu", "Ubuntu-Preview"]
        );
    }

    #[cfg(windows)]
    #[test]
    fn decodes_utf16_wsl_name_without_null_bytes() {
        let bytes = "䅂"
            .encode_utf16()
            .flat_map(|unit| unit.to_le_bytes())
            .collect::<Vec<_>>();

        assert_eq!(decode_wsl_output(&bytes), "䅂");
    }

    #[test]
    fn discovers_codex_and_claude_profiles() {
        let temp = TempDir::new().unwrap();
        let config = config(&temp);
        let codex = provider_config(&config, Provider::Codex);
        let claude = provider_config(&config, Provider::Claude);

        write_file(
            &codex.manifest_path,
            r#"{"version":1,"profiles":[{"name":"work","profilePath":"company/work","family":"company","email":"me@example.com","aliases":["w"]}]}"#,
        );
        write_file(
            &claude.manifest_path,
            r#"{"version":1,"profiles":[{"name":"personal","family":"personal","email":"me@home.example"}]}"#,
        );
        write_file(
            &codex.profiles_home.join("profiles/company/work/auth.json"),
            "codex-secret",
        );
        write_file(
            &claude
                .profiles_home
                .join("profiles/personal/.credentials.json"),
            "claude-secret",
        );

        let inventory = discover_profiles(&config);
        assert_eq!(inventory.stores.len(), 2);
        assert_eq!(inventory.profiles.len(), 2);
        assert!(inventory
            .profiles
            .iter()
            .any(|profile| profile.provider == Provider::Codex && profile.credential_present));
        assert!(inventory
            .profiles
            .iter()
            .any(|profile| profile.provider == Provider::Claude && profile.credential_present));
    }

    #[test]
    fn discovers_credential_directories_without_a_manifest() {
        let temp = TempDir::new().unwrap();
        let config = config(&temp);
        let codex = provider_config(&config, Provider::Codex);
        write_file(
            &codex
                .profiles_home
                .join("profiles/client/account-one/auth.json"),
            "codex-secret",
        );

        let inventory = discover_profiles(&config);
        let profile = inventory.profiles.first().unwrap();
        assert_eq!(profile.name, "account-one");
        assert_eq!(profile.family, "client");
        assert_eq!(profile.source, ProfileSource::ProfileDirectory);
    }

    #[test]
    fn stops_walking_when_a_profile_directory_is_found() {
        let temp = TempDir::new().unwrap();
        let config = config(&temp);
        let codex = provider_config(&config, Provider::Codex);
        let profile_dir = codex.profiles_home.join("profiles/client/account-one");
        write_file(
            &profile_dir.join(".profile.json"),
            r#"{"name":"account-one","family":"client"}"#,
        );
        write_file(&profile_dir.join("auth.json"), "codex-secret");
        write_file(
            &profile_dir.join("cache/nested/.profile.json"),
            r#"{"name":"should-not-be-scanned","family":"cache"}"#,
        );
        write_file(&profile_dir.join("cache/nested/auth.json"), "cached-secret");

        let inventory = discover_profiles(&config);
        assert_eq!(inventory.profiles.len(), 1);
        assert_eq!(inventory.profiles[0].name, "account-one");
        assert_eq!(inventory.profiles[0].profile_path, "client/account-one");
    }

    #[test]
    fn empty_credential_placeholder_does_not_hide_nested_profiles() {
        let temp = TempDir::new().unwrap();
        let config = config(&temp);
        let codex = provider_config(&config, Provider::Codex);
        let family_dir = codex.profiles_home.join("profiles/client");
        write_file(&family_dir.join("auth.json"), "");
        write_file(&family_dir.join("account-one/auth.json"), "codex-secret");

        let inventory = discover_profiles(&config);
        assert_eq!(inventory.profiles.len(), 1);
        assert_eq!(inventory.profiles[0].name, "account-one");
        assert_eq!(inventory.profiles[0].profile_path, "client/account-one");
        assert!(inventory.profiles[0].credential_present);
    }

    #[test]
    fn malformed_provider_manifest_does_not_hide_other_provider() {
        let temp = TempDir::new().unwrap();
        let config = config(&temp);
        let codex = provider_config(&config, Provider::Codex);
        let claude = provider_config(&config, Provider::Claude);
        write_file(&codex.manifest_path, "{not-json");
        write_file(
            &claude
                .profiles_home
                .join("profiles/personal/.credentials.json"),
            "claude-secret",
        );

        let inventory = discover_profiles(&config);
        assert_eq!(inventory.profiles.len(), 1);
        assert_eq!(inventory.profiles[0].provider, Provider::Claude);
        assert!(!inventory.stores[0].issues.is_empty());
    }

    #[test]
    fn activates_credentials_atomically_and_marks_the_profile() {
        let temp = TempDir::new().unwrap();
        let config = config(&temp);
        let codex = provider_config(&config, Provider::Codex);
        write_file(
            &codex
                .profiles_home
                .join("profiles/company/work/.profile.json"),
            r#"{"name":"work","profilePath":"company/work","family":"company","email":"me@example.com"}"#,
        );
        write_file(
            &codex.profiles_home.join("profiles/company/work/auth.json"),
            "codex-secret",
        );
        write_file(&codex.active_home.join("auth.json"), "old-secret");

        let result = activate_profile(&config, Provider::Codex, "company/work").unwrap();
        assert_eq!(result.profile, "work");
        assert_eq!(
            fs::read_to_string(codex.active_home.join("auth.json")).unwrap(),
            "codex-secret"
        );
        assert!(codex.active_home.join(ACTIVE_MARKER).is_file());
        assert!(!codex.active_home.join(LEGACY_ACTIVE_MARKER).exists());

        let inventory = discover_profiles(&config);
        assert!(inventory.profiles[0].active);

        write_file(&codex.active_home.join("auth.json"), "external-change");
        let inventory = discover_profiles(&config);
        assert!(!inventory.profiles[0].active);
    }

    #[test]
    fn recognizes_legacy_active_marker() {
        let temp = TempDir::new().unwrap();
        let config = config(&temp);
        let codex = provider_config(&config, Provider::Codex);
        write_file(
            &codex.profiles_home.join("profiles/work/auth.json"),
            "codex-secret",
        );

        activate_profile(&config, Provider::Codex, "work").unwrap();
        fs::rename(
            codex.active_home.join(ACTIVE_MARKER),
            codex.active_home.join(LEGACY_ACTIVE_MARKER),
        )
        .unwrap();

        let inventory = discover_profiles(&config);
        assert_eq!(inventory.profiles.len(), 1);
        assert!(inventory.profiles[0].active);
    }

    #[test]
    fn reports_file_based_desktop_profiles_without_exposing_credentials() {
        let temp = TempDir::new().unwrap();
        let config = config(&temp);
        let codex = provider_config(&config, Provider::Codex);
        write_file(
            &codex.profiles_home.join("profiles/work/auth.json"),
            &codex_auth("account-work", "work"),
        );
        write_file(
            &codex.profiles_home.join("profiles/personal/auth.json"),
            "not-a-complete-oauth-bundle",
        );
        write_file(
            &codex.active_home.join("auth.json"),
            &codex_auth("account-work", "desktop"),
        );

        let status = codex_desktop_status(&config, &codex.active_home);
        assert!(status.file_activation_supported);
        assert_eq!(status.credential_store, "default");
        assert_eq!(status.eligible_profile_paths, vec!["work"]);
        assert_eq!(status.active_profile_paths, vec!["work"]);
        assert!(!status.rollback_available);
        assert!(!format!("{status:?}").contains("access-work"));
        assert!(!format!("{status:?}").contains("refresh-work"));
    }

    #[test]
    fn refuses_desktop_switching_when_keyring_is_authoritative() {
        let temp = TempDir::new().unwrap();
        let config = config(&temp);
        let codex = provider_config(&config, Provider::Codex);
        write_file(
            &codex.profiles_home.join("profiles/work/auth.json"),
            &codex_auth("account-work", "work"),
        );
        write_file(
            &codex.active_home.join("auth.json"),
            &codex_auth("account-work", "desktop"),
        );
        write_file(
            &codex.active_home.join("config.toml"),
            "cli_auth_credentials_store = \"keyring\"",
        );

        let status = codex_desktop_status(&config, &codex.active_home);
        assert!(!status.file_activation_supported);
        assert_eq!(status.credential_store, "keyring");
        assert!(matches!(
            activate_codex_desktop_profile(&config, "work", &codex.active_home),
            Err(ProfileError::UnsupportedDesktopCredential { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn refuses_desktop_switching_when_credential_store_config_is_unsafe() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new().unwrap();
        let config = config(&temp);
        let codex = provider_config(&config, Provider::Codex);
        write_file(
            &codex.profiles_home.join("profiles/work/auth.json"),
            &codex_auth("account-work", "work"),
        );
        write_file(
            &codex.active_home.join("auth.json"),
            &codex_auth("account-work", "desktop"),
        );
        symlink(
            temp.path().join("missing-config.toml"),
            codex.active_home.join("config.toml"),
        )
        .unwrap();

        let status = codex_desktop_status(&config, &codex.active_home);
        assert_eq!(status.credential_store, "unknown");
        assert!(!status.file_activation_supported);
        assert!(matches!(
            activate_codex_desktop_profile(&config, "work", &codex.active_home),
            Err(ProfileError::UnsupportedDesktopCredential { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn refuses_to_ignore_a_dangling_symlink_during_secure_cleanup() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new().unwrap();
        let dangling = temp.path().join("rollback.json");
        symlink(temp.path().join("missing-auth.json"), &dangling).unwrap();

        assert!(matches!(
            remove_regular_file_if_present(&dangling),
            Err(ProfileError::UnsafeActivationPath(path)) if path == dangling
        ));
    }

    #[cfg(unix)]
    #[test]
    fn treats_a_symlinked_rollback_marker_as_an_unsafe_pending_transaction() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new().unwrap();
        let marker = temp.path().join(DESKTOP_ROLLBACK_MARKER);
        symlink(temp.path().join("missing-marker.json"), &marker).unwrap();

        assert!(desktop_rollback_pending(temp.path()));
        assert!(matches!(
            rollback_codex_desktop_profile(temp.path()),
            Err(ProfileError::UnsafeActivationPath(path)) if path == marker
        ));
        assert!(matches!(
            confirm_codex_desktop_profile(temp.path()),
            Err(ProfileError::UnsafeActivationPath(path)) if path == marker
        ));
    }

    #[test]
    fn switches_desktop_credentials_saves_refresh_and_rolls_back() {
        let temp = TempDir::new().unwrap();
        let config = config(&temp);
        let codex = provider_config(&config, Provider::Codex);
        let outgoing_profile = codex.profiles_home.join("profiles/work/auth.json");
        let selected_profile = codex.profiles_home.join("profiles/personal/auth.json");
        let desktop_auth = codex.active_home.join("auth.json");
        write_file(
            &outgoing_profile,
            &codex_auth("account-work", "stale-profile"),
        );
        write_file(
            &selected_profile,
            &codex_auth("account-personal", "selected"),
        );
        write_file(
            &desktop_auth,
            &codex_auth("account-work", "refreshed-desktop"),
        );

        let result =
            activate_codex_desktop_profile(&config, "personal", &codex.active_home).unwrap();
        assert_eq!(result.profile, "personal");
        assert_eq!(result.outgoing_profiles_updated, 1);
        assert!(result.rollback_available);
        assert_eq!(
            read_codex_credential_identity(&outgoing_profile)
                .unwrap()
                .account_id,
            "account-work"
        );
        assert!(fs::read_to_string(outgoing_profile)
            .unwrap()
            .contains("refreshed-desktop"));
        assert_eq!(
            read_codex_credential_identity(&desktop_auth)
                .unwrap()
                .account_id,
            "account-personal"
        );
        assert!(desktop_rollback_pending(&codex.active_home));
        assert!(matches!(
            activate_codex_desktop_profile(&config, "work", &codex.active_home),
            Err(ProfileError::DesktopRollbackPending)
        ));

        let rollback = rollback_codex_desktop_profile(&codex.active_home).unwrap();
        assert!(rollback.restored_previous_credential);
        assert_eq!(
            read_codex_credential_identity(&desktop_auth)
                .unwrap()
                .account_id,
            "account-work"
        );
        assert!(!desktop_rollback_pending(&codex.active_home));
    }

    #[test]
    fn confirms_desktop_activation_and_discards_rollback() {
        let temp = TempDir::new().unwrap();
        let config = config(&temp);
        let codex = provider_config(&config, Provider::Codex);
        write_file(
            &codex.profiles_home.join("profiles/work/auth.json"),
            &codex_auth("account-work", "work"),
        );
        write_file(
            &codex.profiles_home.join("profiles/personal/auth.json"),
            &codex_auth("account-personal", "personal"),
        );
        write_file(
            &codex.active_home.join("auth.json"),
            &codex_auth("account-work", "desktop"),
        );

        activate_codex_desktop_profile(&config, "personal", &codex.active_home).unwrap();
        confirm_codex_desktop_profile(&codex.active_home).unwrap();

        assert!(!desktop_rollback_pending(&codex.active_home));
        assert!(matches!(
            rollback_codex_desktop_profile(&codex.active_home),
            Err(ProfileError::DesktopRollbackUnavailable)
        ));
        assert_eq!(
            read_codex_credential_identity(&codex.active_home.join("auth.json"))
                .unwrap()
                .account_id,
            "account-personal"
        );
    }

    #[test]
    fn ignores_an_orphaned_desktop_backup_without_a_rollback_marker() {
        let temp = TempDir::new().unwrap();
        let config = config(&temp);
        let codex = provider_config(&config, Provider::Codex);
        write_file(
            &codex.profiles_home.join("profiles/work/auth.json"),
            &codex_auth("account-work", "work"),
        );
        write_file(
            &codex.profiles_home.join("profiles/personal/auth.json"),
            &codex_auth("account-personal", "personal"),
        );
        write_file(
            &codex.active_home.join("auth.json"),
            &codex_auth("account-work", "desktop"),
        );
        write_file(
            &codex.active_home.join(DESKTOP_ROLLBACK_CREDENTIAL),
            &codex_auth("orphaned-account", "orphaned"),
        );

        let status = codex_desktop_status(&config, &codex.active_home);
        assert!(!status.rollback_available);
        activate_codex_desktop_profile(&config, "personal", &codex.active_home).unwrap();
        assert!(desktop_rollback_pending(&codex.active_home));
    }

    #[test]
    fn refuses_missing_credentials_and_path_traversal() {
        let temp = TempDir::new().unwrap();
        let config = config(&temp);
        let codex = provider_config(&config, Provider::Codex);
        write_file(
            &codex
                .profiles_home
                .join("profiles/company/work/.profile.json"),
            r#"{"name":"work","profilePath":"company/work","family":"company"}"#,
        );

        assert!(matches!(
            activate_profile(&config, Provider::Codex, "company/work"),
            Err(ProfileError::CredentialUnavailable { .. })
        ));
        assert!(matches!(
            activate_profile(&config, Provider::Codex, "../escape"),
            Err(ProfileError::InvalidProfile { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlinked_active_credential() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new().unwrap();
        let config = config(&temp);
        let codex = provider_config(&config, Provider::Codex);
        write_file(
            &codex.profiles_home.join("profiles/work/auth.json"),
            "codex-secret",
        );
        fs::create_dir_all(&codex.active_home).unwrap();
        let outside = temp.path().join("outside");
        write_file(&outside, "preserve-me");
        symlink(&outside, codex.active_home.join("auth.json")).unwrap();

        assert!(matches!(
            activate_profile(&config, Provider::Codex, "work"),
            Err(ProfileError::UnsafeActivationPath(_))
        ));
        assert_eq!(fs::read_to_string(outside).unwrap(), "preserve-me");
    }

    #[cfg(unix)]
    #[test]
    fn refuses_a_symlinked_atomic_copy_destination() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new().unwrap();
        let source = temp.path().join("source-auth.json");
        let target = temp.path().join("profile/auth.json");
        let outside = temp.path().join("outside-auth.json");
        write_file(&source, "new-secret");
        write_file(&outside, "preserve-me");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        symlink(&outside, &target).unwrap();

        assert!(matches!(
            atomic_copy(&source, &target, Provider::Codex, "work"),
            Err(ProfileError::UnsafeActivationPath(path)) if path == target
        ));
        assert_eq!(fs::read_to_string(outside).unwrap(), "preserve-me");
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlinked_source_credential() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new().unwrap();
        let config = config(&temp);
        let codex = provider_config(&config, Provider::Codex);
        let source = codex.profiles_home.join("profiles/work/auth.json");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        let outside = temp.path().join("outside-source");
        write_file(&outside, "do-not-copy");
        symlink(&outside, &source).unwrap();

        assert!(discover_profiles(&config).profiles.is_empty());
        assert!(matches!(
            open_file_no_follow(&source),
            Err(ProfileError::UnsafeActivationPath(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn ignores_symlinked_active_marker() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new().unwrap();
        let config = config(&temp);
        let codex = provider_config(&config, Provider::Codex);
        write_file(
            &codex.profiles_home.join("profiles/work/auth.json"),
            "codex-secret",
        );
        write_file(&codex.active_home.join("auth.json"), "codex-secret");

        let outside = temp.path().join("outside-marker");
        write_file(
            &outside,
            r#"{"schemaVersion":1,"provider":"codex","profile":"work","profilePath":"work","credentialSize":12,"activeModifiedUnixSecs":0,"activeModifiedSubsecNanos":0}"#,
        );
        symlink(&outside, codex.active_home.join(ACTIVE_MARKER)).unwrap();

        let inventory = discover_profiles(&config);
        assert_eq!(inventory.profiles.len(), 1);
        assert!(!inventory.profiles[0].active);
    }

    #[test]
    fn rejects_option_like_identifiers_from_metadata() {
        let temp = TempDir::new().unwrap();
        let config = config(&temp);
        let codex = provider_config(&config, Provider::Codex);
        write_file(
            &codex.profiles_home.join("profiles/safe/.profile.json"),
            r#"{"name":"safe","family":"local","aliases":["--option"]}"#,
        );

        let inventory = discover_profiles(&config);
        assert!(inventory.profiles.is_empty());
        assert!(inventory.stores[0].issues[0].contains("alias must start"));
    }
}
