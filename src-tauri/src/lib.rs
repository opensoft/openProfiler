#[cfg(windows)]
use opensoft_open_profiler_core::{
    activate_codex_desktop_profile as activate_desktop,
    codex_desktop_status as inspect_desktop_credentials,
    confirm_codex_desktop_profile as confirm_desktop,
    rollback_codex_desktop_profile as rollback_desktop, CodexDesktopActivationResult,
    CodexDesktopStatus,
};
use opensoft_open_profiler_core::{
    activate_profile as activate, discover_profiles, ActivationResult, DiscoveryConfig,
    ProfileInventory, Provider,
};
use serde::Serialize;
#[cfg(windows)]
use std::env;
#[cfg(windows)]
use std::path::PathBuf;

#[cfg(windows)]
mod windows_desktop;

#[tauri::command]
fn list_profiles() -> Result<ProfileInventory, String> {
    let config = DiscoveryConfig::from_env().map_err(|error| error.to_string())?;
    Ok(discover_profiles(&config))
}

#[tauri::command]
fn activate_profile(provider: Provider, profile_path: String) -> Result<ActivationResult, String> {
    let config = DiscoveryConfig::from_env().map_err(|error| error.to_string())?;
    activate(&config, provider, &profile_path).map_err(|error| error.to_string())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DesktopAppStatus {
    platform_supported: bool,
    installed: bool,
    running: bool,
    file_activation_supported: bool,
    credential_store: String,
    eligible_profile_paths: Vec<String>,
    active_profile_paths: Vec<String>,
    rollback_available: bool,
    message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DesktopActivationOutcome {
    profile: String,
    outgoing_profiles_updated: usize,
    rollback_available: bool,
    relaunched: bool,
}

#[tauri::command]
fn desktop_app_status() -> Result<DesktopAppStatus, String> {
    #[cfg(not(windows))]
    {
        Ok(DesktopAppStatus {
            platform_supported: false,
            installed: false,
            running: false,
            file_activation_supported: false,
            credential_store: "unavailable".to_string(),
            eligible_profile_paths: Vec::new(),
            active_profile_paths: Vec::new(),
            rollback_available: false,
            message: "GPT app switching requires the native Windows openProfiler app".to_string(),
        })
    }

    #[cfg(windows)]
    {
        let config = DiscoveryConfig::from_env().map_err(|error| error.to_string())?;
        let home = desktop_home(&config)?;
        let credential_status = inspect_desktop_credentials(&config, &home);
        let app = windows_desktop::inspect();
        let message = if !app.installed {
            "The Windows ChatGPT desktop app was not detected".to_string()
        } else {
            credential_status.message.clone()
        };
        Ok(desktop_status_view(credential_status, app, message))
    }
}

#[tauri::command]
fn activate_codex_desktop_profile(
    profile_path: String,
) -> Result<DesktopActivationOutcome, String> {
    #[cfg(not(windows))]
    {
        let _ = profile_path;
        Err("GPT app switching requires the native Windows openProfiler app".to_string())
    }

    #[cfg(windows)]
    {
        let config = DiscoveryConfig::from_env().map_err(|error| error.to_string())?;
        let home = desktop_home(&config)?;
        let app = windows_desktop::inspect();
        if !app.installed {
            return Err("The Windows ChatGPT desktop app was not detected".to_string());
        }

        windows_desktop::stop_gracefully().map_err(|error| error.to_string())?;
        let result = activate_desktop(&config, &profile_path, &home);
        let activation = match result {
            Ok(activation) => activation,
            Err(error) => {
                if app.running {
                    let _ = windows_desktop::launch(&app.app_user_model_id);
                }
                return Err(error.to_string());
            }
        };
        windows_desktop::launch(&app.app_user_model_id).map_err(|error| {
            format!(
                "The profile was switched and can be rolled back, but ChatGPT could not be relaunched: {error}"
            )
        })?;
        Ok(desktop_activation_outcome(activation, true))
    }
}

#[tauri::command]
fn rollback_codex_desktop_profile() -> Result<DesktopActivationOutcome, String> {
    #[cfg(not(windows))]
    {
        Err("GPT app switching requires the native Windows openProfiler app".to_string())
    }

    #[cfg(windows)]
    {
        let config = DiscoveryConfig::from_env().map_err(|error| error.to_string())?;
        let home = desktop_home(&config)?;
        let app = windows_desktop::inspect();
        windows_desktop::stop_gracefully().map_err(|error| error.to_string())?;
        let result = rollback_desktop(&home);
        match result {
            Ok(_) => {
                let relaunched =
                    app.installed && windows_desktop::launch(&app.app_user_model_id).is_ok();
                Ok(DesktopActivationOutcome {
                    profile: String::new(),
                    outgoing_profiles_updated: 0,
                    rollback_available: false,
                    relaunched,
                })
            }
            Err(error) => {
                if app.running {
                    let _ = windows_desktop::launch(&app.app_user_model_id);
                }
                Err(error.to_string())
            }
        }
    }
}

#[tauri::command]
fn confirm_codex_desktop_profile() -> Result<(), String> {
    #[cfg(not(windows))]
    {
        Err("GPT app switching requires the native Windows openProfiler app".to_string())
    }

    #[cfg(windows)]
    {
        let config = DiscoveryConfig::from_env().map_err(|error| error.to_string())?;
        let home = desktop_home(&config)?;
        confirm_desktop(&home).map_err(|error| error.to_string())
    }
}

#[cfg(windows)]
fn desktop_home(config: &DiscoveryConfig) -> Result<PathBuf, String> {
    if let Some(path) = env::var_os("OPENPROFILER_CODEX_DESKTOP_HOME") {
        return Ok(PathBuf::from(path));
    }
    config
        .providers
        .iter()
        .find(|provider| provider.provider == Provider::Codex)
        .map(|provider| provider.active_home.clone())
        .ok_or_else(|| "Codex profile discovery is not configured".to_string())
}

#[cfg(windows)]
fn desktop_status_view(
    status: CodexDesktopStatus,
    app: windows_desktop::DesktopApp,
    message: String,
) -> DesktopAppStatus {
    DesktopAppStatus {
        platform_supported: true,
        installed: app.installed,
        running: app.running,
        file_activation_supported: status.file_activation_supported,
        credential_store: status.credential_store,
        eligible_profile_paths: status.eligible_profile_paths,
        active_profile_paths: status.active_profile_paths,
        rollback_available: status.rollback_available,
        message,
    }
}

#[cfg(windows)]
fn desktop_activation_outcome(
    result: CodexDesktopActivationResult,
    relaunched: bool,
) -> DesktopActivationOutcome {
    DesktopActivationOutcome {
        profile: result.profile,
        outgoing_profiles_updated: result.outgoing_profiles_updated,
        rollback_available: result.rollback_available,
        relaunched,
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            list_profiles,
            activate_profile,
            desktop_app_status,
            activate_codex_desktop_profile,
            rollback_codex_desktop_profile,
            confirm_codex_desktop_profile
        ])
        .run(tauri::generate_context!())
        .expect("error while running openProfiler");
}
