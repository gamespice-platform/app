//! Start on boot. Off until the user turns it on; the OS entry is the state.

use tauri::AppHandle;
use tauri_plugin_autostart::ManagerExt;

/// Whether the app is registered to start when the user logs in.
#[tauri::command]
pub fn autostart_get(app: AppHandle) -> Result<bool, String> {
    app.autolaunch().is_enabled().map_err(|e| e.to_string())
}

/// Register or remove the login entry, and say what the OS now holds.
#[tauri::command]
pub fn autostart_set(app: AppHandle, enabled: bool) -> Result<bool, String> {
    let launch = app.autolaunch();

    if enabled {
        launch.enable().map_err(|e| e.to_string())?;
    } else {
        launch.disable().map_err(|e| e.to_string())?;
    }

    launch.is_enabled().map_err(|e| e.to_string())
}
