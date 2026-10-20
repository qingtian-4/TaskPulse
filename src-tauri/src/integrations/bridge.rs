use super::{
    journal, manifest,
    schema::{self, Event, Kind},
};
use serde_json::{json, Value};
use std::io::Read;

pub fn configured_source(id: &str) -> Result<crate::monitoring::SourceConfig, String> {
    if !schema::identifier(id) {
        return Err("invalid_source_id".into());
    }
    let path = crate::providers::remote_tmux::config::config_path();
    if std::fs::metadata(&path)
        .map_err(|_| "config_missing")?
        .len()
        > 2 * 1024 * 1024
    {
        return Err("config_too_large".into());
    }
    let text = std::fs::read_to_string(path).map_err(|_| "config_unreadable")?;
    let value: toml::Value = toml::from_str(&text).map_err(|_| "config_invalid")?;
    let settings: crate::monitoring::DashboardSettings = value
        .get("dashboard")
        .cloned()
        .ok_or("source_registry_missing")?
        .try_into()
        .map_err(|_| "source_registry_invalid")?;
    if !settings.onboarding_complete {
        return Err("source_not_confirmed".into());
    }
    settings
        .sources
        .into_iter()
        .find(|s| s.id == id && s.kind == crate::monitoring::SourceKind::GenericEvent)
        .ok_or_else(|| "source_not_configured".into())
}

pub fn receive(source_id: &str, adapter: &str, host: bool, bytes: &[u8]) -> Result<(), String> {
    if bytes.len() > schema::MAX_INPUT {
        return Err("input_too_large".into());
    }
    let source = configured_source(source_id)?;
    if !source.enabled {
        return Err("source_disabled".into());
    }
    let config = source
        .integration
        .as_ref()
        .ok_or("integration_not_configured")?;
    if config.adapter_id != adapter || config.transport != "local" {
        return Err("adapter_or_transport_mismatch".into());
    }
    let m = manifest::load(adapter)?;
    if host
        && !m.supported_versions.is_empty()
        && !config
            .host_version
            .as_ref()
            .is_some_and(|v| m.supported_versions.contains(v))
    {
        return Err("host_version_not_supported".into());
    }
    if m.agent_id != config.agent_id {
        return Err("agent_binding_mismatch".into());
    }
    let input: Value = serde_json::from_slice(bytes).map_err(|_| "invalid_json")?;
    if !host {
        let event: Event = serde_json::from_value(input).map_err(|_| "event_schema_error")?;
        event.validate(&config.agent_id)?;
        if event.kind == Kind::Test {
            // Separate diagnostic channel, never a task/journal lifecycle.
            return journal::atomic(
                &journal::root(&super::home(), source_id).join("connection-test.json"),
                &json!({"test":true,"received_at":super::now_ms(),"accepted":true}),
            );
        }
        journal::append(
            &super::home(),
            source_id,
            event,
            "self_reported",
            &m.adapter_version,
        )?;
    } else {
        let session = m.text(&input, "session").ok_or("host_session_missing")?;
        if !schema::identifier(session) {
            return Err("invalid_session_id".into());
        }
        let evidence = if manifest::builtin(adapter).is_some() {
            "documented_hook"
        } else {
            "self_reported"
        };
        journal::append_using(
            &super::home(),
            source_id,
            session,
            evidence,
            &m.adapter_version,
            |previous, seq, epoch| manifest::map(&input, &m, previous, seq, epoch),
        )?;
    }
    Ok(())
}

/// No raw input, command, token, prompt or tool output is written to diagnostics.
pub fn diagnostic(source: &str, code: &str) {
    let result = json!({"at":super::now_ms(),"code":code.chars().filter(|c|c.is_ascii_alphanumeric()||*c=='_').take(96).collect::<String>()});
    let _ = journal::atomic(
        &journal::root(&super::home(), source).join("last-error.json"),
        &result,
    );
}

/// Hook mode: bounded stdin wait, silent fail-open; validation/diagnose modes
/// have real failure exit codes and machine-readable JSON on stdout.
pub fn cli(args: &[String]) -> Option<i32> {
    let hook = args
        .iter()
        .any(|a| a == "--integration-event" || a == "--integration-host-event");
    let validate = args.iter().any(|a| a == "--integration-validate");
    let diagnose = args.iter().any(|a| a == "--integration-diagnose");
    if !hook && !validate && !diagnose {
        return None;
    }
    let argument = |name: &str| {
        args.windows(2)
            .find(|w| w[0] == name)
            .map(|w| w[1].as_str())
    };
    let source = argument("--source").unwrap_or("unbound");
    if diagnose {
        let result = diagnose_source(source);
        println!(
            "{}",
            match &result {
                Ok(v) => v.clone(),
                Err(e) => json!({"ok":false,"error":e}),
            }
        );
        return Some(if result.is_ok() { 0 } else { 1 });
    }
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = vec![];
        let result = std::io::stdin()
            .take((schema::MAX_INPUT + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| "stdin_read_failed".to_string())
            .and_then(|_| {
                if bytes.len() > schema::MAX_INPUT {
                    Err("input_too_large".into())
                } else {
                    Ok(bytes)
                }
            });
        let _ = tx.send(result);
    });
    let input = rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap_or_else(|_| Err("stdin_timeout".into()));
    let result = input.and_then(|bytes| {
        if validate {
            let event: Event = serde_json::from_slice(&bytes).map_err(|_| "event_schema_error")?;
            event.validate(&event.agent_id)
        } else {
            receive(
                source,
                argument("--adapter").ok_or("adapter_argument_required")?,
                args.iter().any(|a| a == "--integration-host-event"),
                &bytes,
            )
        }
    });
    if validate {
        println!(
            "{}",
            match &result {
                Ok(()) => json!({"valid":true,"source_bound":false}),
                Err(e) => json!({"valid":false,"error":e}),
            }
        );
        return Some(if result.is_ok() { 0 } else { 1 });
    }
    if let Err(code) = result {
        diagnostic(source, &code);
    }
    Some(0)
}

pub fn diagnose_source(source: &str) -> Result<Value, String> {
    let config = configured_source(source)?;
    let dir = journal::root(&super::home(), source);
    let mut events = 0;
    let mut received = None;
    let mut sampled_kinds = std::collections::BTreeSet::new();
    if let Ok(entries) = std::fs::read_dir(dir.join("events")) {
        for path in entries
            .take(4096)
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
        {
            // Only tail envelopes and counts; never return their payloads.
            let mut file = std::fs::File::open(path).map_err(|_| "journal_unreadable")?;
            use std::io::{Seek, SeekFrom};
            let len = file.metadata().map_err(|_| "journal_unreadable")?.len();
            file.seek(SeekFrom::Start(
                len.saturating_sub((journal::MAX_LINE * 2) as u64),
            ))
            .map_err(|_| "journal_unreadable")?;
            let mut bytes = vec![];
            file.take((journal::MAX_LINE * 2) as u64)
                .read_to_end(&mut bytes)
                .map_err(|_| "journal_unreadable")?;
            if let Some(e) = bytes
                .split(|b| *b == b'\n')
                .rev()
                .find_map(|b| serde_json::from_slice::<schema::Envelope>(b).ok())
            {
                events += e.sequence;
                received = Some(received.unwrap_or(0u64).max(e.received_at));
                sampled_kinds.insert(
                    serde_json::to_value(e.event.kind)
                        .map_err(|_| "diagnostic_serialize_failed")?
                        .as_str()
                        .unwrap_or("unknown")
                        .to_owned(),
                );
            }
        }
    }
    let last_error = std::fs::read(dir.join("last-error.json"))
        .ok()
        .filter(|b| b.len() < 1024)
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok());
    let test = std::fs::read(dir.join("connection-test.json"))
        .ok()
        .filter(|b| b.len() < 1024)
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok());
    Ok(
        json!({"ok":true,"source":source,"enabled":config.enabled,"state":if !config.enabled{"disabled"}else if last_error.as_ref().and_then(|e:&Value|e.get("at")).and_then(Value::as_u64).is_some_and(|at|received.is_none_or(|r|at>=r)){"diagnostic"}else if received.is_some(){"connected"}else{"waiting_first_event"},"events":events,"last_received_at":received,"sampled_latest_event_kinds":sampled_kinds,"last_error":last_error,"test":test,"quiet_is_failure":false}),
    )
}
