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
const ACTIVE_MARKER: &str = ".profile-switcher-active.json";

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

        let codex = ProviderConfig {
            provider: Provider::Codex,
            manifest_path: env::var_os("CODEX_PROFILES_MANIFEST")
                .or_else(|| env::var_os("CHATGPT_PROFILES_MANIFEST"))
                .map(PathBuf::from)
                .unwrap_or_else(|| config_home.join("workbenches/openai-profiles.json")),
            profiles_home: env::var_os("CODEX_PROFILES_HOME")
                .or_else(|| env::var_os("CHATGPT_PROFILES_HOME"))
                .map(PathBuf::from)
                .unwrap_or_else(|| Provider::Codex.default_profiles_home(&home)),
            active_home: env::var_os("PROFILE_SWITCHER_CODEX_ACTIVE_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| Provider::Codex.default_active_home(&home)),
        };

        let claude = ProviderConfig {
            provider: Provider::Claude,
            manifest_path: env::var_os("CLAUDE_PROFILES_MANIFEST")
                .map(PathBuf::from)
                .unwrap_or_else(|| config_home.join("workbenches/claude-profiles.json")),
            profiles_home: env::var_os("CLAUDE_PROFILES_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| Provider::Claude.default_profiles_home(&home)),
            active_home: env::var_os("PROFILE_SWITCHER_CLAUDE_ACTIVE_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| Provider::Claude.default_active_home(&home)),
        };

        Ok(Self {
            providers: vec![codex, claude],
        })
    }
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
    pub active_credential_path: PathBuf,
    pub restart_required: bool,
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
    active_modified_unix_nanos: u64,
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
    atomic_copy(&source, &target)?;
    let (credential_size, active_modified_unix_nanos) =
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
        active_modified_unix_nanos,
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
        active_credential_path: target,
        restart_required: true,
    })
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
        for entry in WalkDir::new(&profile_root)
            .min_depth(2)
            .max_depth(8)
            .follow_links(false)
            .into_iter()
            .filter_map(std::result::Result::ok)
            .filter(|entry| entry.file_type().is_file() && entry.file_name() == ".profile.json")
        {
            let metadata_path = entry.into_path();
            let result = read_manifest_profile(&metadata_path).and_then(|mut profile| {
                if profile.profile_path.is_none() {
                    let parent =
                        metadata_path
                            .parent()
                            .ok_or_else(|| ProfileError::InvalidProfile {
                                profile: profile.name.clone(),
                                reason: "metadata file has no parent directory".to_string(),
                            })?;
                    let relative = parent
                        .strip_prefix(&profile_root)
                        .map_err(|_| ProfileError::EscapedProfileRoot(parent.to_path_buf()))?;
                    profile.profile_path = Some(relative.to_string_lossy().into_owned());
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

        for entry in WalkDir::new(&profile_root)
            .min_depth(2)
            .max_depth(8)
            .follow_links(false)
            .into_iter()
            .filter_map(std::result::Result::ok)
            .filter(|entry| {
                entry.file_type().is_file()
                    && entry.file_name() == config.provider.credential_name()
            })
        {
            let credential_path = entry.into_path();
            let Some(parent) = credential_path.parent() else {
                continue;
            };
            let Ok(relative) = parent.strip_prefix(&profile_root) else {
                continue;
            };
            let profile_path = relative.to_string_lossy().into_owned();
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
    let path = config.active_home.join(ACTIVE_MARKER);
    let text = fs::read_to_string(path).ok()?;
    let marker: ActiveMarker = serde_json::from_str(&text).ok()?;
    (marker.schema_version == 1 && marker.provider == config.provider).then_some(marker)
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
            && credential_stamp(active_credential).is_some_and(|(size, modified)| {
                size == marker.credential_size && modified == marker.active_modified_unix_nanos
            })
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

fn credential_stamp(path: &Path) -> Option<(u64, u64)> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if !metadata.file_type().is_file() || metadata.len() == 0 {
        return None;
    }
    let modified = metadata
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some((metadata.len(), u64::try_from(modified).ok()?))
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

fn atomic_copy(source: &Path, target: &Path) -> Result<()> {
    let mut input = File::open(source).map_err(|source_error| ProfileError::Read {
        path: source.to_path_buf(),
        source: source_error,
    })?;
    if !input
        .metadata()
        .map(|metadata| metadata.is_file() && metadata.len() > 0)
        .unwrap_or(false)
    {
        return Err(ProfileError::CredentialUnavailable {
            provider: if source.file_name().and_then(|value| value.to_str()) == Some("auth.json") {
                Provider::Codex
            } else {
                Provider::Claude
            },
            profile: source
                .parent()
                .and_then(Path::file_name)
                .and_then(|value| value.to_str())
                .unwrap_or("profile")
                .to_string(),
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
        ".{name}.profile-switcher-{}-{nonce}.tmp",
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
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let source_wide: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let target_wide: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
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

fn read_text(path: &Path) -> Result<String> {
    let mut file = File::open(path).map_err(|source| ProfileError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let mut text = String::new();
    file.read_to_string(&mut text)
        .map_err(|source| ProfileError::Read {
            path: path.to_path_buf(),
            source,
        })?;
    Ok(text)
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

    fn provider_config(config: &DiscoveryConfig, provider: Provider) -> &ProviderConfig {
        config
            .providers
            .iter()
            .find(|item| item.provider == provider)
            .unwrap()
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

        let inventory = discover_profiles(&config);
        assert!(inventory.profiles[0].active);

        write_file(&codex.active_home.join("auth.json"), "external-change");
        let inventory = discover_profiles(&config);
        assert!(!inventory.profiles[0].active);
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
