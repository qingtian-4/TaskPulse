//! Ordered, replayable observational Claude hook envelopes. No approval policy
//! is changed. Writers serialize with an OS lock; the latest JSON is retained
//! only as a compatibility checkpoint, never as the event queue.
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::time::Duration;

pub fn default_dir() -> std::path::PathBuf {
    crate::runtime::state_root().join("claude-events")
}

const MAX_ENVELOPE_BYTES: usize = 60 * 1024;
const MAX_JOURNAL_BYTES: u64 = 8 * 1024 * 1024;

const MAX_DIRECTORY_BYTES: u64 = 128 * 1024 * 1024;
const MAX_SESSIONS: usize = 512;
const STAGING_PREFIX: &str = ".taskpulse-claude-write-";
pub(crate) const MAX_DIRECTORY_ENTRIES: usize = 4096;
pub(crate) const DEGRADED_MARKER: &str = ".admission-degraded";

pub fn record(dir: &Path, input: &Value, at: u64, root: &Path) -> io::Result<Value> {
    record_bounded(dir, input, at, root, MAX_DIRECTORY_BYTES, MAX_SESSIONS)
}

fn record_bounded(
    dir: &Path,
    input: &Value,
    at: u64,
    root: &Path,
    byte_limit: u64,
    session_limit: usize,
) -> io::Result<Value> {
    let result = record_inner(dir, input, at, root, byte_limit, session_limit);
    if result.is_err() {
        // The bridge is fail open. A durable local flag prevents a dropped
        // event from looking like a healthy source after the GUI restarts.
        let _ = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dir.join(DEGRADED_MARKER))
            .and_then(|file| file.sync_all());
    }
    result
}

#[derive(Default)]
struct Footprint {
    journal: u64,
    checkpoint: u64,
}

impl Footprint {
    fn reserved_bytes(&self) -> u64 {
        // Keep space for an atomic replacement of one full event and its
        // compatibility checkpoint even when the shared quota is full.
        self.journal
            .max(MAX_ENVELOPE_BYTES as u64)
            .saturating_add(self.checkpoint.max(MAX_ENVELOPE_BYTES as u64))
    }
}

fn inventory(dir: &Path) -> io::Result<BTreeMap<String, Footprint>> {
    let mut sessions: BTreeMap<String, Footprint> = BTreeMap::new();
    for (i, entry) in std::fs::read_dir(dir)?.enumerate() {
        if i == MAX_DIRECTORY_ENTRIES {
            return Err(io::Error::other(
                "Claude event directory entry budget exhausted",
            ));
        }
        let entry = entry?;
        let path = entry.path();
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(STAGING_PREFIX))
        {
            // Only this writer uses this reserved prefix, and no other
            // writer can have an active staging file under the global lock.
            if entry.file_type()?.is_file() {
                std::fs::remove_file(&path)?;
            }
            continue;
        }
        let Some(ext @ ("jsonl" | "json" | "lock")) = path.extension().and_then(|s| s.to_str())
        else {
            continue;
        };
        let Some(id) = path
            .file_stem()
            .and_then(|s| s.to_str())
            .filter(|id| valid_id(id))
        else {
            continue;
        };
        // Windows already returned these attributes during enumeration.
        // DirEntry metadata, like symlink_metadata, does not follow links.
        let meta = entry.metadata()?;
        if !meta.is_file() {
            return Err(io::Error::other("Claude event path is not a regular file"));
        }
        let session = sessions.entry(session_key(id)).or_default();
        match ext {
            "jsonl" => session.journal = session.journal.saturating_add(meta.len()),
            "json" => session.checkpoint = session.checkpoint.saturating_add(meta.len()),
            _ => {}
        }
    }
    Ok(sessions)
}

fn session_key(id: &str) -> String {
    if cfg!(windows) {
        id.to_ascii_lowercase()
    } else {
        id.to_owned()
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 160
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_".contains(c))
}

fn lock(path: &Path) -> io::Result<std::fs::File> {
    crate::integrations::journal::lock(path, Duration::from_secs(1)).map_err(io::Error::other)
}

fn record_inner(
    dir: &Path,
    input: &Value,
    at: u64,
    root: &Path,
    byte_limit: u64,
    session_limit: usize,
) -> io::Result<Value> {
    let id = input
        .get("session_id")
        .and_then(Value::as_str)
        .filter(|id| valid_id(id))
        .ok_or_else(|| std::io::Error::other("invalid session ID"))?;
    let event = input
        .get("hook_event_name")
        .and_then(Value::as_str)
        .filter(|name| crate::claude_integration::EVENTS.contains(name))
        .ok_or_else(|| std::io::Error::other("unknown hook event"))?;
    // Non-permission notifications have no reducer effect and cannot serve
    // as the sole record of a compacted execution.
    if event == "Notification" && input["notification_type"].as_str() != Some("permission_prompt") {
        return Ok(Value::Null);
    }
    std::fs::create_dir_all(dir)?;
    let dir = dir.canonicalize()?;
    let dir = dir.as_path();
    // Serialize the inventory and admission across all hook processes. Keep
    // the old per-session lock for compatibility with existing executables.
    let _capacity_guard = lock(&dir.join(".admission.lock"))?;
    let sessions = inventory(dir)?;
    let key = session_key(id);
    let admitted = sessions.get(&key);
    let total = sessions.values().fold(0u64, |sum, entry| {
        sum.saturating_add(entry.reserved_bytes())
    });
    let default_reservation = Footprint::default().reserved_bytes();
    if admitted.is_none()
        && (sessions.len() >= session_limit
            || total.saturating_add(default_reservation) > byte_limit)
    {
        return Err(io::Error::other(
            "Claude journal capacity exhausted; existing sessions retained",
        ));
    }
    let _session_guard = lock(&dir.join(format!("{id}.lock")))?;
    let path = dir.join(format!("{id}.jsonl"));
    let mut journal = match std::fs::File::options().read(true).write(true).open(&path) {
        Ok(file) => Some(file),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let (last, valid_length) = match journal.as_mut() {
        Some(file) => last_record(file, id)?,
        None => (None, 0),
    };
    let checkpoint = read_checkpoint(&dir.join(format!("{id}.json")));
    let previous = if let Some(event) = last {
        if let Ok(checkpoint) = &checkpoint {
            let journal_seq = event["sequence"].as_u64();
            let checkpoint_seq = checkpoint["sequence"].as_u64();
            if checkpoint["session_id"].as_str() == Some(id)
                && (checkpoint_seq > journal_seq
                    || checkpoint_seq == journal_seq && checkpoint != &event)
            {
                return Err(io::Error::other(
                    "Claude journal rewound or diverged from its checkpoint",
                ));
            }
        }
        event
    } else {
        match checkpoint {
            Ok(event) => {
                if !event.is_object()
                    || event.get("session_id").is_some() && event["session_id"].as_str() != Some(id)
                {
                    return Err(io::Error::other("Claude checkpoint identity mismatch"));
                }
                event
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Value::Null,
            Err(error) => return Err(error),
        }
    };
    if !valid_checkpoint(&previous) {
        return Err(io::Error::other("Claude reducer checkpoint is damaged"));
    }
    let sequence = previous
        .get("sequence")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| std::io::Error::other("event sequence overflow"))?;
    let at = at.max(
        previous
            .get("observed_at")
            .and_then(Value::as_u64)
            .unwrap_or(0),
    );
    let event_id = format!("{id}:{sequence}");
    let text = |key: &str| previous.get(key).and_then(Value::as_str).map(str::to_owned);
    let mut turn = text("turn_key").unwrap_or_default();
    let mut turn_start = previous.get("turn_started_at").and_then(Value::as_u64);
    let mut calls: BTreeMap<String, String> = previous
        .get("open_calls")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let mut pending: Vec<Value> = previous
        .get("pending_requests")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if pending.is_empty() && previous.get("pending_requests").is_none() {
        if let Some(request) = text("pending_request_id") {
            pending.push(json!({"id":request,"call_id":text("pending_call_id")}));
        }
    }
    let call = input
        .get("tool_use_id")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let mut resolved = Vec::new();
    if event == "UserPromptSubmit" {
        turn = format!("prompt:{event_id}");
        turn_start = Some(at);
        calls.clear();
        pending.clear();
    } else if event == "PreToolUse" {
        if let Some(call) = &call {
            calls.insert(call.clone(), tool_signature(input));
        }
    } else if event == "PermissionRequest"
        || event == "Notification"
            && input.get("notification_type").and_then(Value::as_str) == Some("permission_prompt")
    {
        // PermissionRequest lacks tool_use_id. Match only a UNIQUE preceding
        // tool/input signature, never whichever parallel tool ran last.
        if event != "Notification" || pending.is_empty() {
            let signature = tool_signature(input);
            let matching: Vec<_> = calls.iter().filter(|(_, s)| **s == signature).collect();
            let permission_call = if event == "PermissionRequest" && matching.len() == 1 {
                Some(matching[0].0.clone())
            } else {
                None
            };
            let repeated = permission_call.as_ref().is_some_and(|call| {
                pending
                    .iter()
                    .any(|p| p.get("call_id").and_then(Value::as_str) == Some(call))
            });
            if !repeated {
                pending
                    .push(json!({"id":format!("approval:{event_id}"),"call_id":permission_call}));
            }
        }
    } else if matches!(event, "PostToolUse" | "PostToolUseFailure") {
        if let Some(call) = &call {
            pending.retain(|p| {
                if p.get("call_id").and_then(Value::as_str) == Some(call) {
                    if let Some(id) = p.get("id").and_then(Value::as_str) {
                        resolved.push(id.to_owned());
                    }
                    false
                } else {
                    true
                }
            });
            calls.remove(call);
        }
    } else if matches!(event, "Stop" | "StopFailure") {
        pending.clear();
        calls.clear();
    }
    let permission = pending.first().and_then(|p| p.get("id")).cloned();
    let permission_call = pending.first().and_then(|p| p.get("call_id")).cloned();
    let request_ids: Vec<_> = pending
        .iter()
        .filter_map(|p| p.get("id").cloned())
        .collect();
    let activity = matches!(event, "PreToolUse" | "PermissionRequest")
        .then(|| crate::privacy::tool_activity(input));
    let notification_type = (input.get("notification_type").and_then(Value::as_str)
        == Some("permission_prompt"))
    .then_some("permission_prompt");
    let payload = json!({"v":3,"session_id":id,"hook_event_name":event,"observed_at":at,
        "event_id":event_id,"sequence":sequence,"turn_key":turn,"turn_started_at":turn_start,
        "config_root_key":crate::privacy::config_root_key(root),
        "cwd":input.get("cwd").and_then(Value::as_str),"tool_use_id":call,
        "notification_type":notification_type,
        "request_id":permission,"resolved_request_id":resolved.first(),"resolved_request_ids":resolved,
        "open_calls":calls,"pending_requests":pending,"pending_request_ids":request_ids,
        "pending_request_id":permission,"pending_call_id":permission_call,
        "activity":activity});
    let mut bytes = serde_json::to_vec(&payload)?;
    bytes.push(b'\n');
    if bytes.len() > MAX_ENVELOPE_BYTES {
        return Err(std::io::Error::other(
            "Claude event checkpoint exceeds capacity",
        ));
    }
    let current = admitted.map_or(0, Footprint::reserved_bytes);
    let other = total.saturating_sub(current);
    let appended_length = valid_length.saturating_add(bytes.len() as u64);
    let projected = |length| {
        other.saturating_add(
            Footprint {
                journal: length,
                checkpoint: bytes.len() as u64,
            }
            .reserved_bytes(),
        )
    };
    let compact = appended_length > MAX_JOURNAL_BYTES || projected(appended_length) > byte_limit;
    let final_length = if compact {
        bytes.len() as u64
    } else {
        appended_length
    };
    // Legacy directories can already be over quota. An admitted session can
    // still publish a complete checkpoint that does not increase its charge.
    if projected(final_length) > byte_limit
        && (admitted.is_none() || projected(final_length) > total)
    {
        return Err(io::Error::other(
            "Claude journal capacity exhausted; existing sessions retained",
        ));
    }
    if compact {
        // The replacement is itself a complete replay checkpoint. Never
        // truncate a journal to make room, or evict another session.
        drop(journal);
        persist(&path, &bytes)?;
    } else {
        let mut journal = match journal {
            Some(file) => file,
            None => std::fs::File::options()
                .write(true)
                .create_new(true)
                .open(&path)?,
        };
        if journal.metadata()?.len() != valid_length {
            journal.set_len(valid_length)?;
        }
        journal.seek(SeekFrom::Start(valid_length))?;
        journal.write_all(&bytes)?;
        journal.sync_all()?;
    }
    // A compatibility checkpoint failure cannot lose the durable journal.
    persist(&dir.join(format!("{id}.json")), &bytes)?;
    Ok(payload)
}

pub(crate) fn valid_checkpoint(event: &Value) -> bool {
    if event["v"].as_u64().is_none_or(|v| v < 3) {
        return true; // Legacy checkpoints have no complete reducer state.
    }
    let Some(calls) = event["open_calls"].as_object() else {
        return false;
    };
    let Some(requests) = event["pending_requests"].as_array() else {
        return false;
    };
    let Some(ids) = event["pending_request_ids"].as_array() else {
        return false;
    };
    event["sequence"].as_u64().is_some_and(|seq| seq > 0)
        && calls.values().all(Value::is_string)
        && ids.len() == requests.len()
        && requests.iter().zip(ids).all(|(request, id)| {
            id.is_string()
                && request["id"] == *id
                && (request["call_id"].is_string() || request["call_id"].is_null())
        })
        && ids.first().unwrap_or(&Value::Null) == &event["pending_request_id"]
}

fn persist(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut temporary = tempfile::Builder::new()
        .prefix(STAGING_PREFIX)
        .tempfile_in(path.parent().expect("journal parent"))?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

pub(crate) fn read_checkpoint(path: &Path) -> io::Result<Value> {
    let meta = std::fs::symlink_metadata(path)?;
    if !meta.is_file() || meta.len() > MAX_ENVELOPE_BYTES as u64 {
        return Err(io::Error::other(
            "Claude checkpoint exceeds read budget or is not a regular file",
        ));
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(MAX_ENVELOPE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_ENVELOPE_BYTES {
        return Err(io::Error::other("Claude checkpoint exceeds read budget"));
    }
    serde_json::from_slice(&bytes).map_err(io::Error::other)
}

fn last_record(file: &mut std::fs::File, id: &str) -> io::Result<(Option<Value>, u64)> {
    let length = file.metadata()?.len();
    // One full previous line plus the possible interrupted append.
    let start = length.saturating_sub(2 * MAX_ENVELOPE_BYTES as u64);
    file.seek(SeekFrom::Start(start))?;
    let mut tail = Vec::new();
    file.take(length - start).read_to_end(&mut tail)?;
    if tail.len() as u64 != length - start {
        return Err(io::Error::other("Claude journal changed during recovery"));
    }
    let Some(newline) = tail.iter().rposition(|b| *b == b'\n') else {
        return if length < MAX_ENVELOPE_BYTES as u64 {
            Ok((None, 0))
        } else {
            Err(io::Error::other("Claude journal tail is corrupt"))
        };
    };
    let last_start = tail[..newline]
        .iter()
        .rposition(|b| *b == b'\n')
        .map_or(0, |i| i + 1);
    let event: Value = serde_json::from_slice(&tail[last_start..newline])?;
    if event["session_id"].as_str() != Some(id) || event["sequence"].as_u64().is_none() {
        return Err(io::Error::other(
            "Claude journal tail identity or sequence is invalid",
        ));
    }
    Ok((Some(event), start + newline as u64 + 1))
}

// Persist only a fingerprint for correlation, not arbitrary tool inputs.
fn tool_signature(input: &Value) -> String {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    input
        .get("tool_name")
        .unwrap_or(&Value::Null)
        .to_string()
        .hash(&mut hash);
    input
        .get("tool_input")
        .unwrap_or(&Value::Null)
        .to_string()
        .hash(&mut hash);
    format!("{:016x}", hash.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compacted_journal_preserves_pending_requests_and_monotonic_sequence() {
        let dir = tempfile::tempdir().unwrap();
        let input =
            json!({"session_id":"s","hook_event_name":"PermissionRequest","tool_name":"Bash"});
        let first = record(dir.path(), &input, 1, dir.path()).unwrap();
        let path = dir.path().join("s.jsonl");
        // Large valid history ending in the current checkpoint.
        let line = serde_json::to_vec(&first).unwrap();
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        while file.metadata().unwrap().len() <= MAX_JOURNAL_BYTES {
            file.write_all(&line).unwrap();
            file.write_all(b"\n").unwrap();
        }
        drop(file);
        let next = record(
            dir.path(),
            &json!({"session_id":"s","hook_event_name":"PreToolUse","tool_use_id":"unrelated"}),
            2,
            dir.path(),
        )
        .unwrap();
        assert_eq!(next["sequence"], 2);
        assert_eq!(next["pending_request_ids"], first["pending_request_ids"]);
        assert!(std::fs::metadata(path).unwrap().len() < MAX_ENVELOPE_BYTES as u64);
        let restarted = record(
            dir.path(),
            &json!({"session_id":"s","hook_event_name":"Stop"}),
            3,
            dir.path(),
        )
        .unwrap();
        assert_eq!(restarted["sequence"], 3);
        assert_eq!(restarted["pending_request_ids"], json!([]));
    }
    #[test]
    fn shared_byte_capacity_compacts_existing_sessions_and_recovers_after_torn_write() {
        let dir = tempfile::tempdir().unwrap();
        let send = |id: &str, event: &str, call: Option<&str>, at| {
            record_bounded(
                dir.path(),
                &json!({"session_id":id,"hook_event_name":event,"tool_name":"Bash","tool_use_id":call}),
                at,
                dir.path(),
                4 * MAX_ENVELOPE_BYTES as u64,
                10,
            )
        };
        send("a", "PreToolUse", Some("call"), 1).unwrap();
        let pending = send("a", "PermissionRequest", None, 2).unwrap();
        let other = send("b", "PermissionRequest", None, 3).unwrap();
        let other_path = dir.path().join("b.jsonl");
        let other_before = std::fs::read(&other_path).unwrap();
        assert!(send("c", "PermissionRequest", None, 4).is_err());
        assert!(!dir.path().join("c.lock").exists());
        assert!(dir.path().join(DEGRADED_MARKER).is_file());
        let path = dir.path().join("a.jsonl");
        // Fill the session's entire reservation with valid records, then
        // interrupt the next append and leave a lagging compatibility file.
        let mut line = serde_json::to_vec(&pending).unwrap();
        line.resize(MAX_ENVELOPE_BYTES - 1, b' ');
        line.push(b'\n');
        std::fs::write(&path, [&line[..], b"{\"torn"].concat()).unwrap();
        std::fs::remove_file(dir.path().join("a.json")).unwrap();
        let next = send("a", "PreToolUse", Some("unrelated"), 5).unwrap();
        assert_eq!(next["sequence"], 3);
        assert_eq!(next["pending_request_ids"], pending["pending_request_ids"]);
        assert_eq!(std::fs::read_to_string(path).unwrap().lines().count(), 1);
        for i in 6..46 {
            let next = send("a", "PreToolUse", Some("unrelated"), i).unwrap();
            assert_eq!(next["pending_request_ids"], pending["pending_request_ids"]);
        }
        let resolved = send("a", "PostToolUse", Some("call"), 47).unwrap();
        assert_eq!(resolved["resolved_request_id"], pending["request_id"]);
        assert_eq!(resolved["pending_request_ids"], json!([]));
        assert_eq!(std::fs::read(other_path).unwrap(), other_before);
        assert_eq!(read_checkpoint(&dir.path().join("b.json")).unwrap(), other);
    }

    #[test]
    fn concurrent_new_sessions_cannot_exceed_admission_and_corruption_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let send = |id: &str| {
            record_bounded(
                dir.path(),
                &json!({"session_id":id,"hook_event_name":"PermissionRequest"}),
                1,
                dir.path(),
                MAX_DIRECTORY_BYTES,
                2,
            )
        };
        let admitted: Vec<_> = std::thread::scope(|scope| {
            (0..8)
                .map(|i| {
                    let send = &send;
                    scope.spawn(move || {
                        let id = format!("s{i}");
                        send(&id).ok().map(|_| id)
                    })
                })
                .collect::<Vec<_>>()
                .into_iter()
                .filter_map(|thread| thread.join().unwrap())
                .collect()
        });
        assert_eq!(admitted.len(), 2);
        assert_eq!(inventory(dir.path()).unwrap().len(), 2);
        let id = &admitted[0];
        let path = dir.path().join(format!("{id}.jsonl"));
        let mut bytes = std::fs::read(&path).unwrap();
        bytes.extend(b"{invalid complete record}\n");
        std::fs::write(&path, &bytes).unwrap();
        assert!(send(id).is_err());
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }

    #[test]
    fn r04_untrusted_tool_payloads_never_enter_new_journal_or_checkpoint_activity() {
        let dir = tempfile::tempdir().unwrap();
        let secret = "token-command-private-file-prompt-sentinel";
        let root = dir.path().join(secret);
        for (n, tool) in ["Bash", "Read", secret].iter().enumerate() {
            let input = json!({"session_id":"s","hook_event_name":"PreToolUse","tool_use_id":format!("call-{n}"),
                "tool_name":tool,"tool_input":{"command":secret,"file_path":secret,"token":secret},
                "prompt":secret,"tool_response":secret,"notification_type":secret,"cwd":"/public-project"});
            let payload = record(dir.path(), &input, n as u64, &root).unwrap();
            assert_eq!(payload["activity"], crate::privacy::tool_activity(&input));
            assert_eq!(payload["cwd"], "/public-project"); // Necessary local scope metadata.
            assert!(payload.get("config_dir").is_none());
            assert_eq!(
                payload["config_root_key"],
                crate::privacy::config_root_key(&root)
            );
            for name in ["s.jsonl", "s.json"] {
                assert!(!std::fs::read_to_string(dir.path().join(name))
                    .unwrap()
                    .contains(secret));
            }
        }
    }

    #[test]
    fn r04_legacy_activity_is_not_copied_forward_and_permission_correlation_survives() {
        let dir = tempfile::tempdir().unwrap();
        let secret = "legacy-command-secret";
        std::fs::write(
            dir.path().join("s.json"),
            serde_json::to_vec(&json!({
                "v":2,"sequence":1,"activity":secret,"config_dir":secret
            }))
            .unwrap(),
        )
        .unwrap();
        let send =
            |name: &str, call: Option<&str>, at| {
                record(dir.path(), &json!({
            "session_id":"s","hook_event_name":name,"tool_name":"Bash","tool_use_id":call,
            "tool_input":{"command":secret}
        }), at, Path::new("/public-root")).unwrap()
            };
        send("PreToolUse", Some("call"), 2);
        let pending = send("PermissionRequest", None, 3);
        assert_eq!(pending["pending_call_id"], "call");
        assert!(!std::fs::read_to_string(dir.path().join("s.json"))
            .unwrap()
            .contains(secret));
        let resolved = send("PostToolUse", Some("call"), 4);
        assert_eq!(resolved["resolved_request_id"], pending["request_id"]);
        assert!(resolved["pending_request_ids"]
            .as_array()
            .unwrap()
            .is_empty());
        assert!(!std::fs::read_to_string(dir.path().join("s.jsonl"))
            .unwrap()
            .contains(secret));
    }
    #[test]
    fn parallel_permissions_match_inputs_not_last_tool_and_resolve_independently() {
        let dir = tempfile::tempdir().unwrap();
        let send = |event: &str, call: Option<&str>, command: &str, at| {
            record(dir.path(),
            &json!({"session_id":"s","hook_event_name":event,"tool_name":"Bash","tool_use_id":call,
                "tool_input":{"command":command}}), at, Path::new("/claude")).unwrap()
        };
        send("UserPromptSubmit", None, "", 1);
        send("PreToolUse", Some("a"), "command-a", 2);
        send("PreToolUse", Some("b"), "command-b", 3);
        let a = send("PermissionRequest", None, "command-a", 4);
        assert_eq!(a["pending_call_id"], "a");
        let b = send("PermissionRequest", None, "command-b", 5);
        assert_eq!(b["pending_request_ids"].as_array().unwrap().len(), 2);
        let returned = send("PostToolUseFailure", Some("b"), "", 6);
        assert_eq!(returned["request_id"], a["request_id"]);
        assert_eq!(returned["pending_request_ids"].as_array().unwrap().len(), 1);
        assert!(send("PostToolUse", Some("a"), "", 7)["pending_request_ids"]
            .as_array()
            .unwrap()
            .is_empty());
    }
    pub(crate) fn event(dir: &Path, name: &str, call: Option<&str>, at: u64) -> Value {
        record(
            dir,
            &json!({"session_id":"s","hook_event_name":name,"cwd":"/project",
            "tool_name":"Bash","tool_use_id":call,"notification_type":"permission_prompt"}),
            at,
            Path::new("/claude"),
        )
        .unwrap()
    }
    #[test]
    fn requests_are_distinct_and_delayed_notifications_and_old_results_cannot_resolve_the_next() {
        let dir = tempfile::tempdir().unwrap();
        event(dir.path(), "UserPromptSubmit", None, 1);
        event(dir.path(), "PreToolUse", Some("call-a"), 2);
        let a = event(dir.path(), "PermissionRequest", None, 3);
        assert_eq!(
            a["request_id"],
            event(dir.path(), "Notification", None, 4)["request_id"]
        );
        event(dir.path(), "PostToolUse", Some("call-a"), 5);
        event(dir.path(), "PreToolUse", Some("call-b"), 6);
        let b = event(dir.path(), "PermissionRequest", None, 7);
        assert_ne!(a["request_id"], b["request_id"]);
        assert_eq!(
            b["request_id"],
            event(dir.path(), "PermissionRequest", None, 8)["request_id"]
        );
        let late = event(dir.path(), "PostToolUse", Some("call-a"), 9);
        assert!(late["resolved_request_id"].is_null());
        assert_eq!(late["pending_request_id"], b["request_id"]);
        let rejected_tool = event(dir.path(), "PostToolUseFailure", Some("call-b"), 10);
        assert_eq!(rejected_tool["resolved_request_id"], b["request_id"]);
        assert!(rejected_tool["pending_request_id"].is_null());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("s.jsonl"))
                .unwrap()
                .lines()
                .count(),
            10
        );
    }
    #[test]
    fn concurrent_writers_have_one_monotonic_journal_and_partial_append_is_recovered() {
        let dir = tempfile::tempdir().unwrap();
        std::thread::scope(|scope| {
            for _ in 0..12 {
                let path = dir.path();
                scope.spawn(move || {
                    event(path, "PreToolUse", Some("call"), 1);
                });
            }
        });
        let records: Vec<Value> = std::fs::read_to_string(dir.path().join("s.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(records.len(), 12);
        for (i, record) in records.iter().enumerate() {
            assert_eq!(record["sequence"].as_u64(), Some(i as u64 + 1));
        }
        let mut file = std::fs::File::options()
            .append(true)
            .open(dir.path().join("s.jsonl"))
            .unwrap();
        file.write_all(b"{\"half").unwrap();
        drop(file);
        assert_eq!(
            event(dir.path(), "Stop", None, 2)["sequence"].as_u64(),
            Some(13)
        );
        assert!(std::fs::read_to_string(dir.path().join("s.jsonl"))
            .unwrap()
            .lines()
            .all(|line| serde_json::from_str::<Value>(line).is_ok()));
    }
}
