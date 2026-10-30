#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod attention;
mod build_info;
mod claude_events;
mod claude_integration;
mod domain;
mod focus;
mod focusers;
mod heuristics;
mod history;
mod instance;
mod integrations;
mod localization;
mod message_center;
mod monitoring;
mod notification_log;
mod notifier;
mod preferences;
mod privacy;
mod process;
mod providers;
mod scanner;
mod source_catalog;
mod state_file;
mod store;
mod task_notice;
mod transition;
mod tray;
mod updates;
mod watcher;
#[cfg(windows)]
mod windows_startup;

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::Manager;
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

fn main() {
    env_logger::init();
    if let Some(code) = integrations::mcp::cli(&std::env::args().collect::<Vec<_>>()) {
        std::process::exit(code);
    }
    if let Some(code) = integrations::bridge::cli(&std::env::args().collect::<Vec<_>>()) {
        std::process::exit(code);
    }
    if std::env::args().any(|arg| arg == "--version") {
        println!("TaskPulse {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if std::env::args().any(|arg| arg == "--claude-event") {
        if let Err(error) = claude_integration::receive_event() {
            log::warn!("TaskPulse event bridge: {error}");
        }
        // Monitoring must never block or inject context into the Agent.
        return;
    }
    if std::env::args().any(|arg| arg == "--diagnose") {
        watcher::diagnose();
        return;
    }
    if std::env::args().any(|arg| arg == "--test-notification") {
        match notifier::send_test_notification("app.taskpulse.desktop") {
            Ok(()) => println!("Notification accepted by the native Windows/desktop API."),
            Err(error) => {
                eprintln!("Notification failed: {error}");
                std::process::exit(1);
            }
        }
        return;
    }

    let instance = match instance::Instance::acquire(&providers::remote_tmux::config::config_dir())
    {
        Ok(Some(instance)) => instance,
        Ok(None) => return,
        Err(error) => {
            log::error!("Cannot acquire TaskPulse GUI lock: {error}");
            std::process::exit(1);
        }
    };

    // Log panics from any thread before the default handler.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("PANIC: {}", info);
        default_hook(info);
    }));

    let builder = tauri::Builder::default();
    let builder = if std::env::var_os("TASKPULSE_HOME").is_some() {
        builder
    } else {
        builder.plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(tauri_plugin_window_state::StateFlags::POSITION)
                .build(),
        )
    };
    #[cfg(not(windows))]
    let builder = builder.plugin(tauri_plugin_autostart::init(
        tauri_plugin_autostart::MacosLauncher::LaunchAgent,
        None,
    ));
    builder
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(move |app| {
            instance.listen(app.handle().clone());
            app.manage(instance);
            let handle = app.handle().clone();
            let settings = monitoring::initialize();
            let first_setup = !settings.onboarding_complete;
            preferences::apply_theme(app.handle(), &settings.preferences);
            preferences::repair_startup_command(app.handle());
            monitoring::start_auto_discovery(app.handle().clone());
            updates::start(app.handle().clone());

            // Spawn watcher on a dedicated OS thread (blocking loop)
            let h_watch = handle.clone();
            std::thread::spawn(move || watcher::watch(h_watch));

            // Build tray menu (required for AppIndicator on GNOME to show icon;
            // GNOME AppIndicator does NOT support direct click events — only menu)
            let h_show = handle.clone();
            let h_hide = handle.clone();
            let show_item = MenuItem::with_id(app, "show", "显示任务面板", true, None::<&str>)?;
            let pin_item = MenuItem::with_id(app, "pin", "固定任务面板", true, None::<&str>)?;
            let hide_item = MenuItem::with_id(app, "hide", "隐藏任务面板", true, None::<&str>)?;
            let test_notification_item =
                MenuItem::with_id(app, "test-notification", "发送测试提醒", true, None::<&str>)?;
            let separator = PredefinedMenuItem::separator(app)?;
            let quit_item = MenuItem::with_id(app, "quit", "退出 TaskPulse", true, None::<&str>)?;
            app.manage(localization::TrayLabels(vec![
                show_item.clone(),
                pin_item.clone(),
                hide_item.clone(),
                test_notification_item.clone(),
                quit_item.clone(),
            ]));
            localization::refresh_native(app.handle());
            let menu = Menu::with_items(
                app,
                &[
                    &show_item,
                    &pin_item,
                    &hide_item,
                    &test_notification_item,
                    &separator,
                    &quit_item,
                ],
            )?;

            TrayIconBuilder::with_id("main")
                .icon(tauri::image::Image::from_bytes(include_bytes!(
                    "../icons/tray-offline.png"
                ))?)
                .tooltip("TaskPulse · 正在采集，状态待更新")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event({
                    let h_pin = handle.clone();
                    move |app, event| match event.id.as_ref() {
                        "show" => tray::toggle_popup(&h_show),
                        "pin" => tray::pin_popup(&h_pin),
                        "hide" => tray::hide_popup(&h_hide),
                        "test-notification" => {
                            let handle = app.clone();
                            tauri::async_runtime::spawn(async move {
                                if let Err(error) = notifier::test_notification(handle, None).await
                                {
                                    log::error!("Test notification failed: {error}");
                                }
                            });
                        }
                        "quit" => app.exit(0),
                        _ => {}
                    }
                })
                .on_tray_icon_event(move |_tray, event| {
                    // Works on KDE/Windows/macOS — left-click toggles popup directly
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        tray::toggle_popup(&handle);
                    }
                })
                .build(app)?;
            watcher::emit_latest(app.handle());

            // Global shortcut: Ctrl+Shift+A toggles the popup
            let h_shortcut = app.handle().clone();
            let shortcut = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::KeyA);
            app.handle().plugin(
                tauri_plugin_global_shortcut::Builder::new()
                    .with_handler(move |_app, sc, event| {
                        if sc == &shortcut && event.state() == ShortcutState::Pressed {
                            tray::toggle_popup(&h_shortcut);
                        }
                    })
                    .build(),
            )?;
            if let Err(error) = app.global_shortcut().register(shortcut) {
                log::warn!("Ctrl+Shift+A unavailable; use the tray icon: {error}");
            }

            // Useful for first launch and desktop smoke verification.
            if first_setup
                || settings.preferences.show_on_start
                || std::env::args().any(|arg| arg == "--show")
            {
                tray::show_popup(app.handle());
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            build_info::get_build_info,
            source_catalog::get_source_catalog,
            preferences::save_preferences,
            preferences::get_autostart,
            preferences::set_autostart,
            updates::get_update_status,
            updates::check_application_update,
            updates::install_application_update,
            integrations::api::integration_catalog,
            integrations::api::integration_environment,
            integrations::api::integration_installations,
            integrations::api::integration_import,
            integrations::api::integration_preview,
            integrations::api::integration_apply,
            integrations::api::integration_diagnose,
            integrations::api::integration_test,
            integrations::api::integration_export,
            focus::focus_terminal,
            watcher::get_agents,
            watcher::get_dashboard_snapshot,
            watcher::get_archived_attention,
            watcher::get_hosts,
            watcher::get_providers,
            providers::codex::get_codex_quota,
            providers::resources::get_resource_diagnostics,
            watcher::request_refresh,
            watcher::get_status_dir,
            watcher::update_task_attention,
            watcher::open_task,
            watcher::get_selected_task,
            monitoring::get_monitoring_settings,
            monitoring::get_monitoring_error,
            monitoring::save_monitoring_settings,
            monitoring::discover_sources,
            monitoring::list_discovered_sessions,
            watcher::install_hooks,
            claude_integration::configure_claude_events,
            notifier::test_notification,
            notifier::get_notification_error,
            notification_log::get_notification_log,
            message_center::get_message_center,
            message_center::mark_messages_seen,
            task_notice::get_task_notice,
            task_notice::get_pending_task_prompts,
            task_notice::reopen_task_prompt,
            task_notice::dismiss_task_notice,
            task_notice::next_task_notice,
            task_notice::ack_task_notice,
            task_notice::open_task_notice,
            tray::toggle_pin,
            tray::get_pinned,
            tray::resize_dashboard,
            tray::open_management,
            tray::close_popup
        ])
        .build(tauri::generate_context!())
        .expect("TaskPulse failed to build")
        .run(|_app, event| {
            // Prevent exit when the last window is hidden — this is a tray-only app.
            // Only block window-close exits (code == None), not explicit quit (code == Some).
            if let tauri::RunEvent::ExitRequested { code, api, .. } = event {
                if code.is_none() {
                    api.prevent_exit();
                }
            }
        });
}
