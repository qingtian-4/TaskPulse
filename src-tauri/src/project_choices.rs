//! Project picker aliases are presentation groups, never new task/source identities.
use crate::monitoring::{self, SourceKind};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize)]
pub struct ProjectChoice {
    pub id: String,
    pub path: String,
    pub aliases: Vec<String>,
}

fn windows_path(raw: &str) -> Option<String> {
    let value = raw.strip_prefix(r"\\?\").unwrap_or(raw).replace('\\', "/");
    let bytes = value.as_bytes();
    if bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && &bytes[1..3] == b":/" {
        Some(format!(
            "{}{}",
            (bytes[0] as char).to_ascii_uppercase(),
            &value[1..]
        ))
    } else {
        None
    }
}

fn mounted_path(raw: &str) -> Option<String> {
    let tail = raw.strip_prefix("/mnt/")?;
    let bytes = tail.as_bytes();
    (bytes.len() > 1 && bytes[0].is_ascii_alphabetic() && bytes[1] == b'/').then(|| {
        format!(
            "{}:/{}",
            (bytes[0] as char).to_ascii_uppercase(),
            &tail[2..]
        )
    })
}

fn identity(
    raw: &str,
    windows: bool,
    resolve: &impl Fn(&str) -> Option<String>,
) -> (String, String) {
    if windows {
        if let Some(path) = windows_path(raw) {
            if let Some(real) = resolve(&path) {
                return (real.clone(), real);
            }
            // Some older clients prepended the Windows drive to a WSL mount
            // spelling. Collapse it only if the original is missing and the
            // corresponding local directory actually exists.
            if let Some(mounted) = mounted_path(&path[2..]) {
                if mounted[..1].eq_ignore_ascii_case(&path[..1]) {
                    if let Some(real) = resolve(&mounted) {
                        return (real.clone(), real);
                    }
                }
            }
            return (path.clone(), path);
        }
        if let Some(path) = mounted_path(raw) {
            if let Some(real) = resolve(&path) {
                return (real.clone(), real);
            }
        }
    } else if let Some(real) = resolve(raw) {
        return (real.clone(), real);
    }
    (raw.to_owned(), raw.to_owned())
}

fn group(
    source: &str,
    paths: Vec<String>,
    windows: bool,
    resolve: impl Fn(&str) -> Option<String>,
) -> Vec<ProjectChoice> {
    let mut groups: BTreeMap<String, ProjectChoice> = BTreeMap::new();
    for raw in paths.into_iter().filter(|p| !p.is_empty() && p != "*") {
        let (key, path) = identity(&raw, windows, &resolve);
        let row = groups.entry(key.clone()).or_insert_with(|| ProjectChoice {
            id: format!("{source}:{key}"),
            path,
            aliases: Vec::new(),
        });
        if !row.aliases.contains(&raw) {
            row.aliases.push(raw);
        }
    }
    groups.into_values().collect()
}

fn local_directory(path: &str) -> Option<String> {
    if path.contains(['*', '?', '\0']) {
        return None;
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;
        let drive = windows_path(path)?;
        let root: Vec<_> = format!("{}\\", &drive[..2])
            .encode_utf16()
            .chain(Some(0))
            .collect();
        // Never probe arbitrary UNC or mapped-network locations from metadata.
        if !matches!(unsafe { GetDriveTypeW(root.as_ptr()) }, 2 | 3) {
            return None;
        }
    }
    let path = std::path::Path::new(path);
    if !path.is_absolute() || !path.is_dir() {
        return None;
    }
    let real = path.canonicalize().ok()?.to_string_lossy().into_owned();
    #[cfg(windows)]
    {
        windows_path(&real)
    }
    #[cfg(not(windows))]
    {
        Some(real)
    }
}

#[tauri::command]
pub async fn get_project_choices(source_id: String) -> Vec<ProjectChoice> {
    tauri::async_runtime::spawn_blocking(move || {
        let settings = monitoring::get_monitoring_settings();
        let source = settings.sources.iter().find(|s| s.id == source_id);
        // The settings editor can show the default ZCode picker before saving
        // its draft. Only an existing source can select a custom database.
        let zcode_draft =
            source.is_none() && (source_id == "zcode" || source_id.starts_with("zcode:"));
        if source.is_none() && !zcode_draft {
            return Vec::new();
        }
        let local = zcode_draft
            || source.is_some_and(|s| {
                matches!(
                    s.kind,
                    SourceKind::Codex
                        | SourceKind::ClaudeCode
                        | SourceKind::DeepseekDesktop
                        | SourceKind::Opencode
                        | SourceKind::Workbuddy
                        | SourceKind::Zcode
                        | SourceKind::Hermes
                        | SourceKind::Doubao
                        | SourceKind::Scanner
                )
            });
        let mut paths = monitoring::project_paths(&source_id);
        if zcode_draft || source.is_some_and(|s| s.kind == SourceKind::Zcode) {
            // An isolated profile can read its explicitly configured fixture, but
            // must not fall back to the real user's ZCode index.
            let db = if let Some(source) = source.filter(|s| !s.location.is_empty()) {
                Some(std::path::PathBuf::from(&source.location))
            } else if std::env::var_os("TASKPULSE_HOME").is_none() {
                Some(crate::providers::zcode::default_db())
            } else {
                None
            };
            if let Some(db) = db {
                paths.extend(crate::providers::zcode::local_project_roots(&db));
                paths.sort_unstable();
                paths.dedup();
            }
        }
        group(&source_id, paths, local && cfg!(windows), |path| {
            local.then(|| local_directory(path)).flatten()
        })
    })
    .await
    .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn verified_windows_aliases_group_without_rewriting_the_inputs_or_merging_other_directories() {
        let paths = vec![
            r"D:\Project\agenttray",
            r"\\?\D:\Project\agenttray",
            "/mnt/d/project/agenttray",
            r"D:\mnt\d\Project\agenttray",
            r"D:\other\agenttray",
        ];
        let resolve = |p: &str| match p.to_ascii_lowercase().as_str() {
            "d:/project/agenttray" => Some("D:/Project/agenttray".into()),
            "d:/other/agenttray" => Some("D:/other/agenttray".into()),
            _ => None,
        };
        let rows = group(
            "codex",
            paths.iter().map(|p| (*p).to_owned()).collect(),
            true,
            resolve,
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows.iter()
                .find(|p| p.path == "D:/Project/agenttray")
                .unwrap()
                .aliases
                .len(),
            4
        );
    }
    #[test]
    fn existing_double_mount_and_case_sensitive_remote_projects_remain_distinct() {
        let rows = group(
            "local",
            vec!["D:/mnt/d/Project".into(), "D:/Project".into()],
            true,
            |p| Some(p.to_owned()),
        );
        assert_eq!(rows.len(), 2);
        let paths = vec!["/mnt/d/Project".into(), "/mnt/d/project".into()];
        let remote = group("ssh:host", paths.clone(), false, |_| None);
        let wsl = group("wsl:Ubuntu", paths, false, |_| None);
        assert_eq!(remote.len(), 2);
        assert_ne!(remote[0].id, wsl[0].id);
    }
}
