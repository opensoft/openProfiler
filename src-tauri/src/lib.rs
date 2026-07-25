use opensoft_profile_core::{
    build_codex_command as build_command, discover_codex_profiles, CodexAction, DiscoveryConfig,
    ProfileInventory,
};

#[tauri::command]
fn list_codex_profiles() -> Result<ProfileInventory, String> {
    let config = DiscoveryConfig::from_env().map_err(|error| error.to_string())?;
    discover_codex_profiles(&config).map_err(|error| error.to_string())
}

#[tauri::command]
fn build_codex_command(profile: String, action: CodexAction) -> Result<String, String> {
    let config = DiscoveryConfig::from_env().map_err(|error| error.to_string())?;
    let inventory = discover_codex_profiles(&config).map_err(|error| error.to_string())?;
    build_command(&inventory, &profile, action).map_err(|error| error.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            list_codex_profiles,
            build_codex_command
        ])
        .run(tauri::generate_context!())
        .expect("error while running Opensoft Profile Switcher");
}
