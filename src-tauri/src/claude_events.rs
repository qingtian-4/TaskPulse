//! Ordered, replayable observational Claude hook envelopes. No approval policy
//! is changed. Writers serialize with an OS lock; the latest JSON is retained
//! only as a compatibility checkpoint, never as the event queue.
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

const MAX_ENVELOPE_BYTES: usize = 60 * 1024;
const MAX_JOURNAL_BYTES: u64 = 8 * 1024 * 1024;

pub fn record(dir: &Path, input: &Value, at: u64, root: &Path) -> std::io::Result<Value> {
    let id = input
        .get("session_id")
        .and_then(Value::as_str)
        .filter(|id| {
            !id.is_empty()
                && id.len() <= 160
                && id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_".contains(c))
        })
        .ok_or_else(|| std::io::Error::other("invalid session ID"))?;
    let event = input
        .get("hook_event_name")
        .and_then(Value::as_str)
        .filter(|name| crate::claude_integration::EVENTS.contains(name))
        .ok_or_else(|| std::io::Error::other("unknown hook event"))?;
    std::fs::create_dir_all(dir)?;
    let lock = std::fs::File::options()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(dir.join(format!("{id}.lock")))?;
    lock.lock()?;
    let path = dir.join(format!("{id}.jsonl"));
    let mut journal = std::fs::File::options()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)?;
    // Recover an interrupted final append before admitting another writer.
    let length = journal.metadata()?.len();
    journal.seek(SeekFrom::Start(length.saturating_sub(64 * 1024)))?;
    let mut tail = Vec::new();
    journal.read_to_end(&mut tail)?;
    let boundary = tail
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map(|i| i + 1)
        .unwrap_or(0);
    if boundary < tail.len() {
        journal.set_len(length - tail.len() as u64 + boundary as u64)?;
    }
    let last = tail[..boundary]
        .split(|b| *b == b'\n')
        .rev()
        .find_map(|line| serde_json::from_slice::<Value>(line).ok());
    let previous = last
        .or_else(|| {
            std::fs::read(dir.join(format!("{id}.json")))
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        })
        .unwrap_or_default();
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
    journal.seek(SeekFrom::End(0))?;
    journal.write_all(&bytes)?;
    journal.sync_all()?;
    // A checkpoint failure cannot lose the already durable journal event.
    let mut temporary = tempfile::NamedTempFile::new_in(dir)?;
    temporary.write_all(&bytes)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(dir.join(format!("{id}.json")))
        .map_err(|e| e.error)?;
    if journal.metadata()?.len() > MAX_JOURNAL_BYTES {
        // Every v3 envelope is a complete reducer checkpoint: pending calls,
        // approvals, turn start, sequence and correlation fingerprints survive.
        // Persist the checkpoint before atomically replacing the old journal;
        // a failed replacement keeps the full journal, never a truncated file.
        drop(journal);
        let mut compact = tempfile::NamedTempFile::new_in(dir)?;
        compact.write_all(&bytes)?;
        compact.as_file().sync_all()?;
        compact.persist(path).map_err(|e| e.error)?;
    }
    Ok(payload)
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
