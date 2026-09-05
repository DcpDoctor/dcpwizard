use dcpwizard_core::preferences::Preferences;

#[tauri::command]
pub fn load_preferences() -> Result<Preferences, String> {
    dcpwizard_core::preferences::load_preferences().map_err(|error| error.to_string())
}

#[tauri::command]
pub fn save_preferences(preferences: Preferences) -> Result<(), String> {
    dcpwizard_core::preferences::save_preferences(&preferences).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn reset_preferences() -> Result<Preferences, String> {
    dcpwizard_core::preferences::reset_preferences().map_err(|error| error.to_string())
}
