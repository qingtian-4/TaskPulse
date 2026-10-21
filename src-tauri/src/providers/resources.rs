//! Bounded, payload-free runtime evidence. Buffer estimates are not RSS.
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Mutex, OnceLock},
};
static PARSERS: OnceLock<Mutex<BTreeMap<String, Value>>> = OnceLock::new();
pub fn record(kind: &str, root: &Path, metrics: Value) {
    let id = format!(
        "{}:{}",
        kind,
        &crate::integrations::key(&root.to_string_lossy())[..12]
    );
    let mut map = PARSERS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if map.len() < 128 || map.contains_key(&id) {
        map.insert(id, metrics);
    }
}
fn is_link(m: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if m.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    m.file_type().is_symlink()
}
pub fn directory_usage(root: &Path) -> Value {
    let mut dirs = vec![root.to_path_buf()];
    let mut files = 0;
    let mut bytes = 0u64;
    let mut scanned = 0;
    let mut complete = true;
    while let Some(dir) = dirs.pop() {
        if std::fs::symlink_metadata(&dir).is_ok_and(|m| is_link(&m)) {
            complete = false;
            continue;
        }
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => {
                complete = false;
                continue;
            }
        };
        for entry in entries {
            scanned += 1;
            if scanned > 10000 {
                return json!({"bytes_at_least":bytes,"files":files,"complete":false});
            }
            match entry.and_then(|e| std::fs::symlink_metadata(e.path()).map(|m| (e, m))) {
                Ok((entry, m)) if is_link(&m) => {
                    let _ = entry;
                    complete = false;
                }
                Ok((entry, m)) if m.is_dir() => dirs.push(entry.path()),
                Ok((_, m)) => {
                    bytes = bytes.saturating_add(m.len());
                    files += 1;
                }
                Err(_) => complete = false,
            }
        }
    }
    json!({"bytes_at_least":bytes,"files":files,"complete":complete})
}
fn quota_pressure(root: &Path) -> Value {
    let mut sources = Vec::new();
    let mut complete = true;
    match std::fs::read_dir(root) {
        Ok(entries) => {
            for (index, entry) in entries.enumerate() {
                if index >= 64 {
                    complete = false;
                    break;
                }
                let Ok(entry) = entry else {
                    complete = false;
                    continue;
                };
                let Ok(meta) = std::fs::symlink_metadata(entry.path()) else {
                    complete = false;
                    continue;
                };
                if !meta.is_dir() || is_link(&meta) {
                    continue;
                }
                let source_name = entry.file_name().to_string_lossy().into_owned();
                if source_name.len() != 64 || !source_name.bytes().all(|b| b.is_ascii_hexdigit()) {
                    continue;
                }
                let mut journal_bytes = 0u64;
                let mut largest = 0u64;
                let mut source_complete = true;
                match std::fs::read_dir(entry.path().join("events")) {
                    Ok(events) => {
                        for (n, event) in events.enumerate() {
                            if n >= 1024 {
                                source_complete = false;
                                break;
                            }
                            let Ok(event) = event else {
                                source_complete = false;
                                continue;
                            };
                            if event.path().extension().is_none_or(|e| e != "jsonl") {
                                continue;
                            }
                            match std::fs::symlink_metadata(event.path()) {
                                Ok(m) if m.is_file() && !is_link(&m) => {
                                    journal_bytes = journal_bytes.saturating_add(m.len());
                                    largest = largest.max(m.len());
                                }
                                _ => source_complete = false,
                            }
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(_) => source_complete = false,
                }
                sources.push(json!({"source_hash":source_name,"journal_bytes_at_least":journal_bytes,"largest_session_journal":largest,"state":directory_usage(&entry.path().join("state")),"near_quota":journal_bytes>=crate::integrations::journal::SOURCE_DISK_LIMIT*9/10 || largest>=crate::integrations::journal::SESSION_DISK_LIMIT*9/10,"complete":source_complete}));
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => complete = false,
    }
    json!({"sources":sources,"complete":complete})
}
#[cfg(windows)]
pub fn process_memory() -> Value {
    #[repr(C)]
    struct Counters {
        cb: u32,
        faults: u32,
        peak_working: usize,
        working: usize,
        peak_paged: usize,
        paged: usize,
        peak_nonpaged: usize,
        nonpaged: usize,
        pagefile: usize,
        peak_pagefile: usize,
        private: usize,
    }
    #[link(name = "psapi")]
    extern "system" {
        fn GetProcessMemoryInfo(
            process: *mut std::ffi::c_void,
            counters: *mut Counters,
            size: u32,
        ) -> i32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> *mut std::ffi::c_void;
    }
    let mut counters: Counters = unsafe { std::mem::zeroed() };
    counters.cb = std::mem::size_of::<Counters>() as u32;
    if unsafe {
        GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut counters,
            std::mem::size_of::<Counters>() as u32,
        )
    } == 0
    {
        return json!({"available":false});
    }
    json!({"available":true,"private_bytes":counters.private,"working_set_bytes":counters.working,"peak_working_set_bytes":counters.peak_working,"includes_webview_children":false})
}
#[cfg(not(windows))]
pub fn process_memory() -> Value {
    json!({"available":false,"reason":"use OS process sampler on this platform"})
}
pub fn snapshot() -> Value {
    let parsers = PARSERS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let (writes, bytes) = crate::integrations::journal::checkpoint_stats();
    let usage = directory_usage(&crate::integrations::home());
    json!({"parser_instances":parsers,"checkpoint_writes":writes,"checkpoint_bytes":bytes,"integration_disk":usage,
        "source_journal_budget_bytes":crate::integrations::journal::SOURCE_DISK_LIMIT,"session_journal_budget_bytes":crate::integrations::journal::SESSION_DISK_LIMIT,
        "quota_pressure":quota_pressure(&crate::integrations::home()),"process_memory":process_memory(),"note":"Current process counters only. Buffer estimates are not process RSS. Disk scan is bounded; source-level receive/gap diagnoses remain in integration_diagnose."})
}
#[tauri::command]
pub async fn get_resource_diagnostics() -> Value {
    tauri::async_runtime::spawn_blocking(snapshot)
        .await
        .unwrap_or_else(|_| json!({"error":"diagnostics_worker_failed"}))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quota_scan_never_claims_completeness_after_an_entry_budget_cutoff() {
        let dir = tempfile::tempdir().unwrap();
        for n in 0..70 {
            std::fs::create_dir(dir.path().join(format!("{n:064x}"))).unwrap();
        }
        assert_eq!(quota_pressure(dir.path())["complete"], false);
        let dir2 = tempfile::tempdir().unwrap();
        for n in 0..70 {
            std::fs::create_dir(dir2.path().join(format!("unrelated-{n}"))).unwrap();
        }
        assert_eq!(quota_pressure(dir2.path())["complete"], false);
    }
    #[test]
    fn diagnostics_do_not_expose_paths_or_payloads_and_disk_usage_is_read_only() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("evidence"), b"1234").unwrap();
        let usage = directory_usage(dir.path());
        assert_eq!(usage["bytes_at_least"], 4);
        assert_eq!(std::fs::read(dir.path().join("evidence")).unwrap(), b"1234");
        record("synthetic", dir.path(), json!({"entries":3}));
        let snapshot = snapshot();
        assert!(!snapshot
            .to_string()
            .contains(&dir.path().to_string_lossy().to_string()));
        #[cfg(windows)]
        assert_eq!(process_memory()["available"], true);
    }
}
