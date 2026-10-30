//! App presentation preferences. Never changes task identity or source scopes.
use crate::monitoring::DashboardSettings;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, WebviewWindow};
#[cfg(not(windows))]
use tauri_plugin_autostart::ManagerExt;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub theme: String,
    pub language: String,
    pub show_on_start: bool,
    pub show_model: bool,
    pub show_elapsed: bool,
    pub panel_density: String,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            theme: "dark".into(),
            language: "zh-CN".into(),
            show_on_start: false,
            show_model: true,
            show_elapsed: true,
            panel_density: "comfortable".into(),
        }
    }
}
impl Preferences {
    pub fn fresh() -> Self {
        Self {
            theme: "system".into(),
            language: "system".into(),
            ..Self::default()
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if !matches!(self.theme.as_str(), "system" | "light" | "dark") {
            return Err("Invalid appearance".into());
        }
        if !matches!(self.language.as_str(), "system" | "zh-CN" | "en") {
            return Err("Invalid language".into());
        }
        if !matches!(self.panel_density.as_str(), "comfortable" | "compact") {
            return Err("Invalid panel density".into());
        }
        Ok(())
    }
}

/// A separate compare-and-swap baseline lets a preference save preserve source drafts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreferenceValues {
    pub preferences: Preferences,
    pub default_view: String,
    pub recent_days: u16,
    pub notifications: bool,
    pub sound: bool,
    pub notification_mode: String,
    pub auto_check_updates: bool,
    pub auto_discover_local: bool,
}
impl From<&DashboardSettings> for PreferenceValues {
    fn from(s: &DashboardSettings) -> Self {
        Self {
            preferences: s.preferences.clone(),
            default_view: s.default_view.clone(),
            recent_days: s.recent_days,
            notifications: s.notifications,
            sound: s.sound,
            notification_mode: s.notification_mode.clone(),
            auto_check_updates: s.auto_check_updates,
            auto_discover_local: s.auto_discover_local,
        }
    }
}
impl PreferenceValues {
    pub fn apply(self, s: &mut DashboardSettings) {
        s.preferences = self.preferences;
        s.default_view = self.default_view;
        s.recent_days = self.recent_days;
        s.notifications = self.notifications;
        s.sound = self.sound;
        s.notification_mode = self.notification_mode;
        s.auto_check_updates = self.auto_check_updates;
        s.auto_discover_local = self.auto_discover_local;
    }
}
#[tauri::command]
pub fn save_preferences(
    app: AppHandle,
    values: PreferenceValues,
    expected: PreferenceValues,
) -> Result<DashboardSettings, String> {
    crate::monitoring::save_preference_values(app, values, expected)
}

#[derive(Serialize)]
pub struct AutostartStatus {
    pub enabled: bool,
    pub available: bool,
}
fn startup_available() -> bool {
    cfg!(all(feature = "custom-protocol", not(debug_assertions)))
        && std::env::var_os("TASKPULSE_HOME").is_none()
}
#[tauri::command]
pub fn get_autostart(app: AppHandle) -> Result<AutostartStatus, String> {
    // Isolated smoke profiles must never register their disposable executable.
    if !startup_available() {
        return Ok(AutostartStatus {
            enabled: false,
            available: false,
        });
    }
    #[cfg(windows)]
    let enabled = crate::windows_startup::enabled(
        &app.package_info().name,
        &std::env::current_exe().map_err(|e| e.to_string())?,
    );
    #[cfg(not(windows))]
    let enabled = app.autolaunch().is_enabled().map_err(|e| e.to_string());
    enabled
        .map(|enabled| AutostartStatus {
            enabled,
            available: true,
        })
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub fn set_autostart(
    app: AppHandle,
    window: WebviewWindow,
    enabled: bool,
) -> Result<AutostartStatus, String> {
    if window.label() != "management" || !startup_available() {
        return Err("Open startup settings in the installed TaskPulse application".into());
    }
    #[cfg(windows)]
    crate::windows_startup::set_enabled(
        &app.package_info().name,
        &std::env::current_exe().map_err(|e| e.to_string())?,
        enabled,
    )?;
    #[cfg(not(windows))]
    (if enabled {
        app.autolaunch().enable()
    } else {
        app.autolaunch().disable()
    })
    .map_err(|e| e.to_string())?;
    get_autostart(app)
}

pub fn repair_startup_command(_app: &AppHandle) {
    #[cfg(windows)]
    if startup_available() {
        let result = std::env::current_exe()
            .map_err(|e| e.to_string())
            .and_then(|exe| crate::windows_startup::repair(&_app.package_info().name, &exe));
        if let Err(error) = result {
            log::warn!("Login startup command repair: {error}");
        }
    }
}

pub fn apply_theme(app: &AppHandle, p: &Preferences) {
    let theme = match p.theme.as_str() {
        "light" => Some(tauri::Theme::Light),
        "dark" => Some(tauri::Theme::Dark),
        _ => None,
    };
    app.set_theme(theme);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_preferences_keep_the_current_presentation() {
        let p: Preferences = serde_json::from_str("{}").unwrap();
        assert_eq!(p, Preferences::default());
        assert_eq!(p.theme, "dark");
        assert_eq!(p.language, "zh-CN");
        assert_eq!(Preferences::fresh().theme, "system");
    }
    #[test]
    fn bad_preferences_are_rejected_before_persistence() {
        let mut p = Preferences::default();
        p.theme = "neon".into();
        assert!(p.validate().is_err());
        assert!(serde_json::from_str::<Preferences>(r#"{"show_model":"false"}"#).is_err());
    }
    #[test]
    fn applying_preferences_preserves_sources_and_onboarding() {
        let mut s = crate::monitoring::from_legacy(&Default::default(), true);
        let sources = s.sources.clone();
        let mut values = PreferenceValues::from(&s);
        values.preferences.show_model = false;
        values.apply(&mut s);
        assert_eq!(sources, s.sources);
        assert!(s.onboarding_complete);
        assert!(!s.preferences.show_model);
    }
}
