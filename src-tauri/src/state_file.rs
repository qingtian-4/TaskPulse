//! Fail-closed loading of user-owned durable state. Errors contain no paths.
use std::io::{self, Write};
use std::path::Path;

#[derive(Debug, Clone, Copy)]
pub enum StateError {
    Read,
    Invalid,
    Version,
    Backup,
}

impl StateError {
    pub fn message(self) -> &'static str {
        match self {
            Self::Read => "无法读取状态文件；已阻止覆盖，请保全文件并修复后重启",
            Self::Invalid => "状态文件损坏；已阻止覆盖，请保全文件并修复后重启",
            Self::Version => "状态版本不受支持；已阻止覆盖，请使用兼容版本",
            Self::Backup => "迁移备份未确认成功；原文件保留，请检查备份后重启",
        }
    }
}

pub fn read(
    path: &Path,
    max_version: u64,
    allow_unversioned: bool,
) -> Result<Option<(serde_json::Value, Vec<u8>)>, StateError> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            // A dangling link is evidence too, not a fresh installation.
            return match std::fs::symlink_metadata(path) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
                _ => Err(StateError::Read),
            };
        }
        Err(_) => return Err(StateError::Read),
    };
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| StateError::Invalid)?;
    if !value.is_object() {
        return Err(StateError::Invalid);
    }
    match value.get("version") {
        Some(version) if version.as_u64().is_some_and(|v| v > 0 && v <= max_version) => {}
        None if allow_unversioned => {}
        _ => return Err(StateError::Version),
    }
    Ok(Some((value, bytes)))
}

/// Atomically create the one-time backup, or verify an existing exact copy.
/// A directory, partial file or unrelated backup is never a successful backup.
pub fn backup(path: &Path, bytes: &[u8]) -> Result<(), StateError> {
    let attempt = || -> io::Result<()> {
        match std::fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_file() => {
                return if std::fs::read(path)? == bytes {
                    Ok(())
                } else {
                    Err(io::Error::other("backup differs"))
                };
            }
            Ok(_) => return Err(io::Error::other("backup is not a regular file")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let mut file = tempfile::NamedTempFile::new_in(path.parent().unwrap_or(Path::new(".")))?;
        file.write_all(bytes)?;
        file.as_file().sync_all()?;
        file.persist_noclobber(path).map_err(|error| error.error)?;
        Ok(())
    };
    attempt().map_err(|_| StateError::Backup)
}
