#[derive(serde::Serialize)]
pub struct BuildInfo {
    process_id: u32,
    channel: &'static str,
    state_directory: String,
    version: &'static str,
    commit: &'static str,
    modified: bool,
    profile: &'static str,
    executable: String,
}

#[tauri::command]
pub fn get_build_info() -> BuildInfo {
    BuildInfo {
        process_id: std::process::id(),
        channel: if cfg!(debug_assertions) {
            "development"
        } else {
            "release"
        },
        state_directory: crate::runtime::state_root().display().to_string(),
        version: env!("CARGO_PKG_VERSION"),
        commit: env!("TASKPULSE_BUILD_SHA"),
        modified: env!("TASKPULSE_BUILD_DIRTY") == "true",
        profile: env!("TASKPULSE_BUILD_PROFILE"),
        executable: std::env::current_exe()
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
    }
}
