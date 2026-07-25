use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};
use thiserror::Error;
use walkdir::WalkDir;

const MANIFEST_VERSION: u32 = 1;

#[derive(Debug, Error)]
pub enum ProfileError {
    #[error("could not determine the current user's home directory")]
    HomeUnavailable,

    #[error("could not read profile metadata at {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("could not parse profile metadata at {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("unsupported profile manifest version {0}")]
    UnsupportedVersion(u32),

    #[error("invalid profile {profile}: {reason}")]
    InvalidProfile { profile: String, reason: String },

    #[error("profile directory escapes the configured profile root: {0}")]
    EscapedProfileRoot(PathBuf),

    #[error("unknown Codex profile: {0}")]
    UnknownProfile(String),
}

pub type Result<T> = std::result::Result<T, ProfileError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveryConfig {
    pub manifest_path: PathBuf,
    pub profiles_home: PathBuf,
}

impl DiscoveryConfig {
    pub fn from_env() -> Result<Self> {
        let home = dirs::home_dir().ok_or(ProfileError::HomeUnavailable)?;
        let config_home = env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"));

        let manifest_path = env::var_os("CODEX_PROFILES_MANIFEST")
            .or_else(|| env::var_os("CHATGPT_PROFILES_MANIFEST"))
            .map(PathBuf::from)
            .unwrap_or_else(|| config_home.join("workbenches/openai-profiles.json"));

        let profiles_home = env::var_os("CODEX_PROFILES_HOME")
            .or_else(|| env::var_os("CHATGPT_PROFILES_HOME"))
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".chatgpt-profiles"));

        Ok(Self {
            manifest_path,
            profiles_home,
        })
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProfileInventory {
    pub manifest_path: PathBuf,
    pub profiles_home: PathBuf,
    pub profiles: Vec<CodexProfile>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodexProfile {
    pub name: String,
    pub email: String,
    pub family: String,
    pub aliases: Vec<String>,
    pub profile_path: String,
    pub status: String,
    pub configured: bool,
    pub credential_present: bool,
    pub source: ProfileSource,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ProfileSource {
    Manifest,
    ProfileMetadata,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CodexAction {
    Login,
    Status,
    Launch,
    Logout,
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
    email: String,
    family: String,
    #[serde(default)]
    aliases: Vec<String>,
    profile_path: Option<String>,
    status: Option<String>,
}

pub fn discover_codex_profiles(config: &DiscoveryConfig) -> Result<ProfileInventory> {
    let mut profiles = BTreeMap::<String, CodexProfile>::new();

    if config.manifest_path.is_file() {
        let manifest_text = read_text(&config.manifest_path)?;
        let manifest: ProfileManifest =
            serde_json::from_str(&manifest_text).map_err(|source| ProfileError::Parse {
                path: config.manifest_path.clone(),
                source,
            })?;

        if manifest.version != MANIFEST_VERSION {
            return Err(ProfileError::UnsupportedVersion(manifest.version));
        }

        for profile in manifest.profiles {
            let resolved = resolve_profile(profile, config, ProfileSource::Manifest)?;
            profiles.insert(resolved.name.to_lowercase(), resolved);
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
            let metadata_text = read_text(&metadata_path)?;
            let mut profile: ManifestProfile =
                serde_json::from_str(&metadata_text).map_err(|source| ProfileError::Parse {
                    path: metadata_path.clone(),
                    source,
                })?;

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

            let resolved = resolve_profile(profile, config, ProfileSource::ProfileMetadata)?;
            profiles
                .entry(resolved.name.to_lowercase())
                .or_insert(resolved);
        }
    }

    Ok(ProfileInventory {
        manifest_path: config.manifest_path.clone(),
        profiles_home: config.profiles_home.clone(),
        profiles: profiles.into_values().collect(),
    })
}

pub fn build_codex_command(
    inventory: &ProfileInventory,
    requested: &str,
    action: CodexAction,
) -> Result<String> {
    let profile = inventory
        .profiles
        .iter()
        .find(|profile| {
            profile.name.eq_ignore_ascii_case(requested)
                || profile
                    .aliases
                    .iter()
                    .any(|alias| alias.eq_ignore_ascii_case(requested))
        })
        .ok_or_else(|| ProfileError::UnknownProfile(requested.to_string()))?;

    let profile_name = shell_quote(&profile.name);
    let command = match action {
        CodexAction::Login => format!("pcodex login {profile_name}"),
        CodexAction::Status => format!("pcodex status {profile_name}"),
        CodexAction::Launch => format!("pcodex {profile_name}"),
        CodexAction::Logout => format!("pcodex logout {profile_name}"),
    };

    Ok(command)
}

fn resolve_profile(
    profile: ManifestProfile,
    config: &DiscoveryConfig,
    source: ProfileSource,
) -> Result<CodexProfile> {
    validate_required(&profile)?;

    let profile_path = profile
        .profile_path
        .clone()
        .unwrap_or_else(|| profile.name.clone());
    validate_profile_path(&profile.name, &profile_path)?;

    let profile_root = config.profiles_home.join("profiles");
    let profile_dir = profile_root.join(&profile_path);
    if profile_dir.exists() {
        let canonical_root = profile_root
            .canonicalize()
            .map_err(|source| ProfileError::Read {
                path: profile_root.clone(),
                source,
            })?;
        let canonical_profile =
            profile_dir
                .canonicalize()
                .map_err(|source| ProfileError::Read {
                    path: profile_dir.clone(),
                    source,
                })?;
        if !canonical_profile.starts_with(&canonical_root) {
            return Err(ProfileError::EscapedProfileRoot(profile_dir));
        }
    }

    let configured = profile_dir.is_dir();
    let credential_present = configured
        && fs::symlink_metadata(profile_dir.join("auth.json"))
            .map(|metadata| metadata.file_type().is_file() && metadata.len() > 0)
            .unwrap_or(false);

    Ok(CodexProfile {
        name: profile.name,
        email: profile.email,
        family: profile.family,
        aliases: profile.aliases,
        profile_path,
        status: profile.status.unwrap_or_else(|| "active".to_string()),
        configured,
        credential_present,
        source,
    })
}

fn validate_required(profile: &ManifestProfile) -> Result<()> {
    for (field, value) in [
        ("name", profile.name.as_str()),
        ("email", profile.email.as_str()),
        ("family", profile.family.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(ProfileError::InvalidProfile {
                profile: profile.name.clone(),
                reason: format!("{field} must not be empty"),
            });
        }
    }

    validate_identifier(&profile.name, "name", &profile.name)?;
    validate_identifier(&profile.name, "family", &profile.family)?;
    for alias in &profile.aliases {
        validate_identifier(&profile.name, "alias", alias)?;
    }

    Ok(())
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

fn read_text(path: &Path) -> Result<String> {
    fs::read_to_string(path).map_err(|source| ProfileError::Read {
        path: path.to_path_buf(),
        source,
    })
}

fn shell_quote(value: &str) -> String {
    if !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-_./".contains(character))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\"'\"'"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};
    use std::io::Write;
    use tempfile::TempDir;

    fn config(temp: &TempDir) -> DiscoveryConfig {
        DiscoveryConfig {
            manifest_path: temp.path().join("config/openai-profiles.json"),
            profiles_home: temp.path().join("profiles-home"),
        }
    }

    fn write_file(path: &Path, value: &str) {
        fs::create_dir_all(path.parent().expect("test path has parent")).unwrap();
        fs::write(path, value).unwrap();
    }

    #[test]
    fn discovers_manifest_profiles_without_reading_credentials() {
        let temp = TempDir::new().unwrap();
        let config = config(&temp);
        write_file(
            &config.manifest_path,
            r#"{
              "version": 1,
              "profiles": [
                {
                  "name": "work-chatgpt-1",
                  "profilePath": "opensoft/team/work-chatgpt-1",
                  "family": "opensoft",
                  "email": "developer@opensoft.example",
                  "aliases": ["work1"]
                },
                {
                  "name": "personal-chatgpt-1",
                  "family": "personal",
                  "email": "developer@example.com"
                }
              ]
            }"#,
        );

        let work_dir = config
            .profiles_home
            .join("profiles/opensoft/team/work-chatgpt-1");
        fs::create_dir_all(&work_dir).unwrap();
        let mut auth = File::create(work_dir.join("auth.json")).unwrap();
        auth.write_all(b"not-read-by-profile-switcher").unwrap();

        let inventory = discover_codex_profiles(&config).unwrap();
        assert_eq!(inventory.profiles.len(), 2);

        let work = inventory
            .profiles
            .iter()
            .find(|profile| profile.name == "work-chatgpt-1")
            .unwrap();
        assert!(work.configured);
        assert!(work.credential_present);

        let personal = inventory
            .profiles
            .iter()
            .find(|profile| profile.name == "personal-chatgpt-1")
            .unwrap();
        assert!(!personal.configured);
        assert!(!personal.credential_present);
    }

    #[test]
    fn rejects_profile_path_traversal() {
        let temp = TempDir::new().unwrap();
        let config = config(&temp);
        write_file(
            &config.manifest_path,
            r#"{
              "version": 1,
              "profiles": [{
                "name": "escape",
                "profilePath": "../escape",
                "family": "opensoft",
                "email": "developer@opensoft.example"
              }]
            }"#,
        );

        let error = discover_codex_profiles(&config).unwrap_err();
        assert!(matches!(error, ProfileError::InvalidProfile { .. }));
    }

    #[test]
    fn rejects_option_like_profile_names_and_aliases() {
        let temp = TempDir::new().unwrap();
        let config = config(&temp);
        write_file(
            &config.manifest_path,
            r#"{
              "version": 1,
              "profiles": [{
                "name": "safe-profile",
                "family": "opensoft",
                "email": "developer@opensoft.example",
                "aliases": ["--option"]
              }]
            }"#,
        );

        let error = discover_codex_profiles(&config).unwrap_err();
        assert!(matches!(error, ProfileError::InvalidProfile { .. }));
        assert!(error.to_string().contains("alias must start"));
    }

    #[test]
    fn discovers_profile_metadata_when_manifest_is_absent() {
        let temp = TempDir::new().unwrap();
        let config = config(&temp);
        let profile_dir = config.profiles_home.join("profiles/opensoft/team/team-001");
        write_file(
            &profile_dir.join(".profile.json"),
            r#"{
              "name": "team-001",
              "family": "opensoft",
              "email": "team@opensoft.example",
              "aliases": ["team"]
            }"#,
        );

        let inventory = discover_codex_profiles(&config).unwrap();
        assert_eq!(inventory.profiles.len(), 1);
        assert_eq!(inventory.profiles[0].profile_path, "opensoft/team/team-001");
        assert_eq!(inventory.profiles[0].source, ProfileSource::ProfileMetadata);
    }

    #[test]
    fn generates_commands_from_canonical_names_and_aliases() {
        let inventory = ProfileInventory {
            manifest_path: PathBuf::from("/tmp/manifest"),
            profiles_home: PathBuf::from("/tmp/profiles"),
            profiles: vec![CodexProfile {
                name: "work-chatgpt-1".to_string(),
                email: "developer@opensoft.example".to_string(),
                family: "opensoft".to_string(),
                aliases: vec!["work1".to_string()],
                profile_path: "opensoft/team/work-chatgpt-1".to_string(),
                status: "active".to_string(),
                configured: true,
                credential_present: true,
                source: ProfileSource::Manifest,
            }],
        };

        assert_eq!(
            build_codex_command(&inventory, "work1", CodexAction::Launch).unwrap(),
            "pcodex work-chatgpt-1"
        );
        assert_eq!(
            build_codex_command(&inventory, "WORK1", CodexAction::Status).unwrap(),
            "pcodex status work-chatgpt-1"
        );
    }

    #[test]
    fn quotes_nonstandard_profile_names() {
        assert_eq!(shell_quote("team alpha's"), "'team alpha'\"'\"'s'");
    }

    #[test]
    fn preserves_unknown_profile_spelling_in_errors() {
        let inventory = ProfileInventory {
            manifest_path: PathBuf::from("/tmp/manifest"),
            profiles_home: PathBuf::from("/tmp/profiles"),
            profiles: Vec::new(),
        };

        let error =
            build_codex_command(&inventory, "Missing-Profile", CodexAction::Launch).unwrap_err();
        assert_eq!(error.to_string(), "unknown Codex profile: Missing-Profile");
    }
}
