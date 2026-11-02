//! Build-channel state boundaries. Debug binaries never default to production state.
use sha2::{Digest, Sha256};
use std::path::{Component, Path, PathBuf};

pub fn isolated() -> bool {
    cfg!(debug_assertions) || std::env::var_os("TASKPULSE_HOME").is_some()
}

pub fn display_name() -> &'static str {
    if cfg!(debug_assertions) {
        "TaskPulse Dev"
    } else {
        "TaskPulse"
    }
}

// Resolve existing ancestors too: a not-yet-created profile under a junction
// or symlink must not resolve inside the production directory.
fn normalized(path: &Path) -> PathBuf {
    let mut current = path.to_path_buf();
    let mut suffix = Vec::new();
    let mut resolved = loop {
        if let Ok(path) = current.canonicalize() {
            break path;
        }
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) => {
                #[cfg(windows)]
                let reparse = {
                    use std::os::windows::fs::MetadataExt;
                    metadata.file_attributes() & 0x400 != 0
                };
                #[cfg(not(windows))]
                let reparse = false;
                assert!(
                    !metadata.file_type().is_symlink() && !reparse,
                    "Cannot resolve a link in the TaskPulse state path"
                );
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => panic!("Cannot inspect the TaskPulse state path"),
        }
        if let Some(name) = current.file_name() {
            suffix.push(name.to_os_string());
        }
        if !current.pop() {
            break path.to_path_buf();
        }
    };
    for name in suffix.into_iter().rev() {
        resolved.push(name);
    }
    if cfg!(any(windows, target_os = "macos")) {
        PathBuf::from(resolved.to_string_lossy().to_lowercase())
    } else {
        resolved
    }
}
fn validate_development_root(home: &Path, path: &Path) {
    let real = normalized(&home.join(".taskpulse"));
    let candidate = normalized(path);
    assert!(
        !candidate.starts_with(&real) && !real.starts_with(&candidate),
        "Development builds cannot use the production TaskPulse state directory"
    );
}

fn resolve_root(
    home: &Path,
    executable: &Path,
    explicit: Option<PathBuf>,
    development: bool,
) -> PathBuf {
    if let Some(path) = explicit {
        assert!(
            path.is_absolute()
                && path.parent().is_some()
                && !path.components().any(|c| matches!(c, Component::ParentDir)),
            "TASKPULSE_HOME must be an absolute non-root directory"
        );
        if development {
            #[cfg(windows)]
            assert!(
                !path.components().any(|component| match component {
                    Component::Normal(name) => {
                        let name = name.to_string_lossy();
                        name.ends_with('.') || name.ends_with(' ')
                    }
                    _ => false,
                }),
                "Ambiguous Windows state directory component"
            );
            validate_development_root(home, &path);
        }
        return path;
    }
    if !development {
        return home.join(".taskpulse");
    }
    // Stable through Rust rebuilds; different worktree/portable directories get
    // independent settings, checkpoints, GUI locks and browser storage.
    let dir = executable.parent().expect("Executable must have a parent");
    let dir = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    let key = dir.to_string_lossy();
    let key = if cfg!(windows) {
        key.to_lowercase()
    } else {
        key.into_owned()
    };
    let id = format!("{:x}", Sha256::digest(key.as_bytes()));
    let path = home.join(".taskpulse-dev").join(&id[..16]);
    validate_development_root(home, &path);
    path
}

fn validate_profile_children(root: &Path) {
    let canonical = normalized(root);
    for name in [
        "config.toml",
        "attention.json",
        "recent.json",
        "notifications.json",
        "notification-log.json",
        "messages.json",
        "gui.lock",
        "claude-events",
        "integrations",
        "host-sandbox",
        "webview2",
        "legacy-status",
    ] {
        assert!(
            normalized(&root.join(name)).starts_with(&canonical),
            "A linked state entry escapes the development profile"
        );
    }
}

pub fn state_root() -> PathBuf {
    static ROOT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    ROOT.get_or_init(|| {
        let root = resolve_root(
            &dirs_next::home_dir().expect("Cannot locate the TaskPulse user home"),
            &std::env::current_exe().expect("Cannot locate the TaskPulse executable"),
            std::env::var_os("TASKPULSE_HOME").map(PathBuf::from),
            cfg!(debug_assertions),
        );
        if cfg!(debug_assertions) {
            validate_profile_children(&root);
        }
        root
    })
    .clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn development_and_production_and_worktrees_do_not_share_state() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let a = home.join("worktree-a/target/debug/agent-tray");
        let b = home.join("worktree-b/target/debug/agent-tray");
        let dev = resolve_root(home, &a, None, true);
        assert!(dev.starts_with(home.join(".taskpulse-dev")));
        assert_eq!(dev, resolve_root(home, &a, None, true));
        assert_ne!(dev, resolve_root(home, &b, None, true));
        assert_eq!(resolve_root(home, &a, None, false), home.join(".taskpulse"));
        let explicit = home.join("fixture");
        for development in [true, false] {
            assert_eq!(
                resolve_root(home, &a, Some(explicit.clone()), development),
                explicit
            );
        }
    }
    #[test]
    fn malformed_or_production_development_overrides_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        for bad in [
            PathBuf::from("relative"),
            dir.path().join("nested/../state"),
            dir.path().join(".taskpulse"),
            dir.path().join(".taskpulse/nested"),
        ] {
            assert!(std::panic::catch_unwind(|| resolve_root(
                dir.path(),
                &dir.path().join("app"),
                Some(bad),
                true
            ))
            .is_err());
        }
    }
    #[test]
    #[cfg(unix)]
    fn development_root_rejects_a_symlink_into_production() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".taskpulse")).unwrap();
        std::os::unix::fs::symlink(
            dir.path().join(".taskpulse"),
            dir.path().join(".taskpulse-dev"),
        )
        .unwrap();
        assert!(std::panic::catch_unwind(|| resolve_root(
            dir.path(),
            &dir.path().join("app"),
            None,
            true
        ))
        .is_err());
    }
    #[test]
    #[cfg(unix)]
    fn dangling_production_symlink_cannot_become_a_development_profile() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("absent-store");
        std::os::unix::fs::symlink(&target, dir.path().join(".taskpulse")).unwrap();
        assert!(std::panic::catch_unwind(|| resolve_root(
            dir.path(),
            &dir.path().join("app"),
            Some(target),
            true
        ))
        .is_err());
    }
    #[test]
    #[cfg(windows)]
    fn win32_aliases_cannot_select_production_state() {
        let dir = tempfile::tempdir().unwrap();
        for suffix in [
            ".taskpulse.",
            ".taskpulse ",
            ".taskpulse./nested",
            ".TASKPULSE",
        ] {
            assert!(std::panic::catch_unwind(|| resolve_root(
                dir.path(),
                &dir.path().join("app"),
                Some(dir.path().join(suffix)),
                true
            ))
            .is_err());
        }
    }
    #[test]
    #[cfg(any(windows, target_os = "macos"))]
    fn case_aliases_of_missing_production_roots_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        assert!(std::panic::catch_unwind(|| resolve_root(
            dir.path(),
            &dir.path().join("app"),
            Some(dir.path().join(".TASKPULSE")),
            true
        ))
        .is_err());
    }
    #[test]
    #[cfg(unix)]
    fn linked_journal_cannot_escape_a_clean_development_profile() {
        let dir = tempfile::tempdir().unwrap();
        let dev = dir.path().join("dev");
        let real = dir.path().join(".taskpulse/claude-events");
        std::fs::create_dir(&dev).unwrap();
        std::fs::create_dir_all(&real).unwrap();
        validate_profile_children(&dev);
        std::os::unix::fs::symlink(&real, dev.join("claude-events")).unwrap();
        assert!(std::panic::catch_unwind(|| validate_profile_children(&dev)).is_err());
    }
}
