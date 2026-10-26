#[derive(serde::Serialize)]
pub struct BuildInfo {
    version: &'static str,
    commit: &'static str,
    modified: bool,
    profile: &'static str,
    executable: String,
}

#[tauri::command]
pub fn get_build_info() -> BuildInfo {
    BuildInfo {
        version: env!("CARGO_PKG_VERSION"),
        commit: env!("TASKPULSE_BUILD_SHA"),
        modified: env!("TASKPULSE_BUILD_DIRTY") == "true",
        profile: env!("TASKPULSE_BUILD_PROFILE"),
        executable: std::env::current_exe()
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
    }
}
