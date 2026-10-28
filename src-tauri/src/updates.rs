//! Signed application updates. Checking never downloads, installs or restarts.
use serde::Serialize;
use std::{
    sync::{Mutex, OnceLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter, WebviewWindow};
use tauri_plugin_updater::{Update, UpdaterExt};

#[derive(Clone, Default, Serialize)]
pub struct UpdateStatus {
    pub phase: String,
    pub current_version: String,
    pub version: Option<String>,
    pub notes: Option<String>,
    pub checked_at: Option<u64>,
    pub downloaded: u64,
    pub total: Option<u64>,
    pub message: Option<String>,
    pub can_install: bool,
}
#[derive(Default)]
struct State {
    status: UpdateStatus,
    pending: Option<Update>,
    busy: bool,
}
static STATE: OnceLock<Mutex<State>> = OnceLock::new();
fn state() -> &'static Mutex<State> {
    STATE.get_or_init(|| {
        Mutex::new(State {
            status: UpdateStatus {
                phase: "idle".into(),
                current_version: env!("CARGO_PKG_VERSION").into(),
                can_install: install_supported(),
                ..Default::default()
            },
            ..Default::default()
        })
    })
}
fn install_supported() -> bool {
    if !cfg!(all(feature = "custom-protocol", not(debug_assertions)))
        || std::env::var_os("TASKPULSE_HOME").is_some()
    {
        return false;
    }
    #[cfg(target_os = "windows")]
    {
        matches!(
            tauri::utils::platform::bundle_type(),
            Some(tauri::utils::config::BundleType::Nsis | tauri::utils::config::BundleType::Msi)
        )
    }
    #[cfg(target_os = "macos")]
    {
        let Some(exe) = std::env::current_exe()
            .ok()
            .and_then(|p| p.canonicalize().ok())
        else {
            return false;
        };
        let Some(root) = macos_bundle_root(&exe) else {
            return false;
        };
        let Ok(plist::Value::Dictionary(info)) =
            plist::Value::from_file(root.join("Contents/Info.plist"))
        else {
            return false;
        };
        info.get("CFBundleIdentifier")
            .and_then(plist::Value::as_string)
            == Some("app.taskpulse.desktop")
            && info
                .get("CFBundleExecutable")
                .and_then(plist::Value::as_string)
                == exe.file_name().and_then(|s| s.to_str())
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        false
    }
}
#[cfg(any(test, target_os = "macos"))]
fn macos_bundle_root(exe: &std::path::Path) -> Option<std::path::PathBuf> {
    let executable = exe.canonicalize().ok()?;
    if !executable.is_file() {
        return None;
    }
    let macos = executable.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    if macos.file_name()? != "MacOS"
        || contents.file_name()? != "Contents"
        || bundle.extension()? != "app"
    {
        return None;
    }
    Some(bundle.to_path_buf())
}

fn notify(app: &AppHandle) {
    let _ = app.emit("application-update", get_update_status());
}
#[tauri::command]
pub fn get_update_status() -> UpdateStatus {
    state()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .status
        .clone()
}
fn management(window: &WebviewWindow) -> Result<(), String> {
    if window.label() == "management" {
        Ok(())
    } else {
        Err("请在设置中管理应用更新".into())
    }
}
#[tauri::command]
pub async fn check_application_update(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<UpdateStatus, String> {
    management(&window)?;
    check(&app).await;
    Ok(get_update_status())
}
fn allowed_download(update: &Update) -> bool {
    let url = &update.download_url;
    url.scheme() == "https"
        && url.host_str() == Some("github.com")
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && url.path().starts_with(&format!(
            "/qingtian-4/TaskPulse/releases/download/v{}/TaskPulse_",
            update.version
        ))
        && !update.signature.trim().is_empty()
}
async fn check(app: &AppHandle) {
    {
        let mut state = state().lock().unwrap_or_else(|e| e.into_inner());
        if state.busy {
            return;
        }
        state.busy = true;
        state.status.phase = "checking".into();
        state.status.message = None;
    }
    notify(app);
    let result = match app
        .updater_builder()
        .timeout(Duration::from_secs(30))
        .build()
    {
        Ok(updater) => updater.check().await,
        Err(error) => Err(error),
    };
    {
        let mut state = state().lock().unwrap_or_else(|e| e.into_inner());
        state.busy = false;
        state.status.checked_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|d| d.as_millis() as u64);
        match result {
            Ok(Some(mut update)) if allowed_download(&update) => {
                update.timeout = Some(Duration::from_secs(300));
                state.status.version = Some(update.version.clone());
                state.status.notes = update
                    .body
                    .as_ref()
                    .map(|text| text.chars().take(4000).collect());
                state.status.phase = "available".into();
                state.pending = Some(update);
            }
            Ok(None) => {
                state.status.phase = "current".into();
                state.status.version = None;
                state.status.notes = None;
                state.pending = None;
            }
            _ => {
                state.status.phase = "error".into();
                state.status.message =
                    Some("暂时无法取得有效的更新信息，请稍后重试或查看 GitHub Releases。".into());
                state.pending = None;
                state.status.version = None;
                state.status.notes = None;
            }
        }
    }
    notify(app);
}
#[tauri::command]
pub async fn install_application_update(
    app: AppHandle,
    window: WebviewWindow,
    version: String,
) -> Result<(), String> {
    management(&window)?;
    if !install_supported() {
        return Err("请使用已安装的 TaskPulse 管理更新".into());
    }
    let update = {
        let mut state = state().lock().unwrap_or_else(|e| e.into_inner());
        if state.busy {
            return Err("更新操作正在进行".into());
        }
        let update = state
            .pending
            .clone()
            .filter(|u| u.version == version && allowed_download(u))
            .ok_or("请先检查更新并确认版本")?;
        state.busy = true;
        state.status.phase = "downloading".into();
        state.status.downloaded = 0;
        state.status.total = None;
        state.status.message = None;
        update
    };
    notify(&app);
    let result = update
        .download(
            |chunk, total| {
                {
                    let mut state = state().lock().unwrap_or_else(|e| e.into_inner());
                    state.status.downloaded += chunk as u64;
                    state.status.total = total;
                }
                notify(&app);
            },
            || {
                state()
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .status
                    .phase = "verifying".into();
                notify(&app);
            },
        )
        .await;
    let result = match result {
        Ok(bytes) => {
            // download() has verified the signature before any replacement.
            state()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .status
                .phase = "installing".into();
            notify(&app);
            update.install(bytes)
        }
        Err(error) => Err(error),
    };
    if result.is_ok() {
        app.restart();
    }
    {
        let mut state = state().lock().unwrap_or_else(|e| e.into_inner());
        state.busy = false;
        state.pending = None;
        state.status.phase = "error".into();
        state.status.message =
            Some("更新未完成。未通过签名校验的安装包不会运行，请重新检查更新。".into());
    }
    notify(&app);
    Err("更新未完成，请重新检查更新".into())
}
pub fn start(app: AppHandle) {
    if std::env::var_os("TASKPULSE_HOME").is_some() {
        return;
    }
    // One scheduler for the application, independent of settings-window lifetime.
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(60));
        let settings = crate::monitoring::get_monitoring_settings();
        if !settings.onboarding_complete || !settings.auto_check_updates {
            continue;
        }
        let status = get_update_status();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let interval = if status.phase == "error" {
            60 * 60 * 1000
        } else {
            24 * 60 * 60 * 1000
        };
        if status
            .checked_at
            .is_none_or(|last| now.saturating_sub(last) >= interval)
        {
            tauri::async_runtime::block_on(check(&app));
        }
    });
}

#[cfg(test)]
mod tests {
    use base64::Engine;
    #[test]
    fn unpackaged_paths_never_become_macos_replacement_targets() {
        let dir = tempfile::tempdir().unwrap();
        let bare = dir.path().join("agent-tray");
        std::fs::write(&bare, b"fixture").unwrap();
        assert!(super::macos_bundle_root(&bare).is_none());
        let app = dir.path().join("TaskPulse.app");
        let exe = app.join("Contents/MacOS/agent-tray");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, b"fixture").unwrap();
        assert_eq!(
            super::macos_bundle_root(&exe),
            Some(app.canonicalize().unwrap())
        );
        let misleading = dir.path().join("Other/Contents/MacOS/agent-tray");
        std::fs::create_dir_all(misleading.parent().unwrap()).unwrap();
        std::fs::write(&misleading, b"fixture").unwrap();
        assert!(super::macos_bundle_root(&misleading).is_none());
    }

    #[test]
    #[ignore = "requires signed bundle files in TASKPULSE_VERIFY_UPDATE_DIR; never installs"]
    fn signed_release_artifacts_match_embedded_key() {
        let dir = std::path::PathBuf::from(
            std::env::var_os("TASKPULSE_VERIFY_UPDATE_DIR").expect("signed asset directory"),
        );
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let decode = |text: &str| {
            String::from_utf8(
                base64::engine::general_purpose::STANDARD
                    .decode(text.trim())
                    .unwrap(),
            )
            .unwrap()
        };
        let key = minisign_verify::PublicKey::decode(&decode(
            config["plugins"]["updater"]["pubkey"].as_str().unwrap(),
        ))
        .unwrap();
        let mut count = 0;
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|s| s != "sig") {
                continue;
            }
            let signature = minisign_verify::Signature::decode(&decode(
                &std::fs::read_to_string(&path).unwrap(),
            ))
            .unwrap();
            let bytes = std::fs::read(path.with_extension("")).unwrap();
            let signed_version = signature
                .trusted_comment()
                .split('\t')
                .find_map(|field| field.strip_prefix("version:"));
            assert_eq!(
                signed_version,
                Some(env!("CARGO_PKG_VERSION")),
                "signed version must match this application release"
            );
            key.verify(&bytes, &signature, true)
                .expect("release signature must match compiled public key");
            let mut corrupt = bytes.clone();
            corrupt.push(0);
            assert!(key.verify(&corrupt, &signature, true).is_err());
            count += 1;
        }
        assert!(count > 0, "no signatures were checked");
        println!("Verified {count} signed artifacts and corruption rejection");
    }
}
