//! Only the two application-owned About links may leave the management WebView.
use serde::Deserialize;

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectLink {
    Project,
    License,
}

impl ProjectLink {
    fn url(self) -> &'static str {
        match self {
            Self::Project => "https://github.com/qingtian-4/TaskPulse",
            Self::License => "https://github.com/qingtian-4/TaskPulse/blob/main/LICENSE",
        }
    }
}

#[cfg(windows)]
fn open(url: &str) -> Result<(), String> {
    use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};
    let verb: Vec<u16> = "open\0".encode_utf16().collect();
    let target: Vec<u16> = url.encode_utf16().chain(Some(0)).collect();
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            target.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    if result as isize > 32 {
        Ok(())
    } else {
        Err("无法打开系统浏览器，请复制项目链接后重试".into())
    }
}

#[cfg(unix)]
fn open(url: &str) -> Result<(), String> {
    let executable = if cfg!(target_os = "macos") {
        "/usr/bin/open"
    } else {
        "xdg-open"
    };
    let result =
        crate::process::output(executable, &[url], std::time::Duration::from_secs(5), 4096)
            .map_err(|_| "无法打开系统浏览器，请复制项目链接后重试".to_string())?;
    result
        .status
        .success()
        .then_some(())
        .ok_or_else(|| "无法打开系统浏览器，请复制项目链接后重试".to_string())
}

#[tauri::command]
pub async fn open_project_link(target: ProjectLink) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || open(target.url()))
        .await
        .map_err(|_| "打开项目链接未完成".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_are_fixed_and_arbitrary_urls_or_commands_are_rejected() {
        assert_eq!(
            ProjectLink::Project.url(),
            "https://github.com/qingtian-4/TaskPulse"
        );
        assert!(ProjectLink::License.url().ends_with("/blob/main/LICENSE"));
        for value in [
            "https://example.com",
            "file:///private",
            "cmd",
            "project;bad",
        ] {
            assert!(serde_json::from_value::<ProjectLink>(serde_json::json!(value)).is_err());
        }
    }
}
