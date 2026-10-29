// Copied into a disposable CI source tree only. Never linked by production main.rs.
use serde_json::json;
use std::{fs, io::Write, path::PathBuf, time::Duration};
use tauri::{Listener, Manager};

fn write(root: &std::path::Path, name: &str, value: serde_json::Value) {
    let mut temp = tempfile::NamedTempFile::new_in(root).unwrap();
    temp.write_all(serde_json::to_string_pretty(&value).unwrap().as_bytes())
        .unwrap();
    temp.persist(root.join(name)).unwrap();
}

pub fn record_entry(stage: &str) {
    let args: Vec<_> = std::env::args().collect();
    let Some(index) = args.iter().position(|a| a == "--upgrade-probe-root") else {
        return;
    };
    let root = PathBuf::from(&args[index + 1]);
    if !root.join("ci-owned.json").is_file() {
        return;
    }
    write(
        &root,
        &format!(
            "entry-{}-{}-{stage}.json",
            env!("CARGO_PKG_VERSION"),
            std::process::id()
        ),
        json!({
            "stage": stage, "version": env!("CARGO_PKG_VERSION"), "pid": std::process::id(),
            "ci_environment_present": std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true"),
            "exe": std::env::current_exe().ok(),
            "taskpulse_home_override": std::env::var_os("TASKPULSE_HOME").is_some()
        }),
    );
}

pub fn start(app: tauri::AppHandle) {
    record_entry("setup");
    let args: Vec<_> = std::env::args().collect();
    let Some(index) = args.iter().position(|a| a == "--upgrade-probe-root") else {
        return;
    };
    assert_eq!(std::env::var("GITHUB_ACTIONS").as_deref(), Ok("true"));
    assert!(std::env::var_os("TASKPULSE_HOME").is_none());
    let root = PathBuf::from(&args[index + 1]).canonicalize().unwrap();
    assert!(root.join("ci-owned.json").is_file());
    std::thread::spawn(move || {
        let event_root = root.clone();
        app.listen("application-update", move |event| {
            let mut file = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(event_root.join("events.jsonl"))
                .unwrap();
            writeln!(file, "{}", event.payload()).unwrap();
        });
        tauri::async_runtime::block_on(crate::tray::open_management(
            app.clone(),
            Some("updates".into()),
        ))
        .unwrap();
        let window = app.get_webview_window("management").unwrap();
        #[cfg(windows)]
        let elevated = {
            #[link(name = "shell32")]
            extern "system" {
                fn IsUserAnAdmin() -> i32;
            }
            unsafe { IsUserAnAdmin() != 0 }
        };
        #[cfg(not(windows))]
        let elevated = unsafe { libc::geteuid() == 0 };
        assert!(!elevated, "installed app must run without elevation");
        // Do not replay the old process's install command after updater restart.
        let mut last = fs::read(root.join("control.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .and_then(|v| v["id"].as_u64())
            .unwrap_or(0);
        std::thread::sleep(Duration::from_secs(2));
        write(
            &root,
            &format!(
                "boot-{}-{}.json",
                env!("CARGO_PKG_VERSION"),
                std::process::id()
            ),
            json!({
                "pid": std::process::id(), "version": env!("CARGO_PKG_VERSION"),
                "exe": std::env::current_exe().unwrap().canonicalize().unwrap(),
                "elevated": elevated, "status": crate::updates::get_update_status(),
                "bundle": format!("{:?}", tauri::utils::platform::bundle_type()),
                "settings": crate::monitoring::get_monitoring_settings()
            }),
        );
        loop {
            std::thread::sleep(Duration::from_millis(100));
            let Some(command) = fs::read(root.join("control.json"))
                .ok()
                .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            else {
                continue;
            };
            let id = command["id"].as_u64().unwrap();
            if id <= last {
                continue;
            }
            last = id;
            let action = command["action"].as_str().unwrap();
            let version = command["version"].as_str().unwrap_or("").to_owned();
            let result: Result<(), String> = match action {
                "check" => tauri::async_runtime::block_on(
                    crate::updates::check_application_update(app.clone(), window.clone()),
                )
                .map(|_| ()),
                "install" => {
                    tauri::async_runtime::block_on(crate::updates::install_application_update(
                        app.clone(),
                        window.clone(),
                        version,
                    ))
                }
                "popup-denied" => {
                    let popup = app.get_webview_window("popup").unwrap();
                    tauri::async_runtime::block_on(crate::updates::install_application_update(
                        app.clone(),
                        popup,
                        version,
                    ))
                }
                "exit" => {
                    app.exit(0);
                    return;
                }
                _ => panic!("unknown test action"),
            };
            write(
                &root,
                &format!("response-{id}.json"),
                json!({
                    "id": id, "action": action, "ok": result.is_ok(), "error": result.err(),
                    "status": crate::updates::get_update_status(), "pid": std::process::id(),
                    "dashboard": crate::watcher::get_dashboard_snapshot()
                }),
            );
        }
    });
}
