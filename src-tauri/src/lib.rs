use opensoft_open_profiler_core::{
    activate_profile as activate, discover_profiles, ActivationResult, DiscoveryConfig,
    ProfileInventory, Provider,
};

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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![list_profiles, activate_profile])
        .run(tauri::generate_context!())
        .expect("error while running openProfiler");
}
