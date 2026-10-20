//! Only built-in recipes can edit known host files, after a fingerprint-bound
//! preview/confirmation. Imported manifests never grant execution/write rights.
use super::{bridge, journal, key, manifest, now_ms};
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex},
};

#[derive(Debug, Clone, Serialize)]
pub struct Preview {
    pub token: String,
    pub source_id: String,
    pub path: String,
    pub action: String,
    pub entries: Vec<Value>,
    pub decoded_command: String,
    pub notes: Vec<String>,
}
struct Pending {
    public: Preview,
    before: Option<Vec<u8>>,
    after: Vec<u8>,
    binding: String,
    path: PathBuf,
    commands: BTreeSet<String>,
    expires: u64,
}
static PREVIEWS: LazyLock<Mutex<BTreeMap<String, Pending>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

fn read(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match std::fs::metadata(path) {
        Ok(m) if m.len() > 1024 * 1024 => Err("host_config_too_large".into()),
        Ok(m) if !m.is_file() => Err("host_config_not_file".into()),
        Ok(_) => std::fs::read(path)
            .map(Some)
            .map_err(|_| "host_config_unreadable".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("host_config_unreadable".into()),
    }
}

fn location(adapter: &str) -> Result<PathBuf, String> {
    let relative = match adapter {
        "kimi-code-cli" => ".kimi-code/config.toml",
        "trae-cn-ide" => ".trae-cn/hooks.json",
        "zcode-desktop" => ".zcode/cli/config.json",
        _ => return Err("no_verified_install_recipe_export_only".into()),
    };
    // Isolation has its own *host* root as well, not just TaskPulse state.
    let home = if std::env::var_os("TASKPULSE_HOME").is_some() {
        crate::providers::remote_tmux::config::config_dir().join("host-sandbox")
    } else {
        dirs_next::home_dir().ok_or("home_unavailable")?
    };
    Ok(home.join(relative))
}

fn owned_path(source: &str) -> PathBuf {
    journal::root(&super::home(), source).join("hook-ownership.json")
}
fn ownership(source: &str) -> BTreeSet<String> {
    std::fs::read(owned_path(source))
        .ok()
        .filter(|b| b.len() < 64 * 1024)
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}
fn quoted(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

pub fn hook_command(exe: &Path, source: &str, adapter: &str) -> (String, String) {
    use base64::Engine;
    let script=format!("$tpInput = [Console]::In.ReadToEnd(); $OutputEncoding = [Text.UTF8Encoding]::new($false); $tpInput | & {} --integration-host-event --source {} --adapter {}; exit 0",quoted(&exe.to_string_lossy()),quoted(source),quoted(adapter));
    let bytes: Vec<_> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    (
        format!(
            "powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand {}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        ),
        script,
    )
}

fn recipes(adapter: &str, command: &str, exe: &Path, source: &str) -> Result<Vec<Value>, String> {
    let m = manifest::load(adapter)?;
    let mut events: BTreeSet<_> = m.events.keys().cloned().collect();
    events.extend(m.rules.iter().map(|r| r.event.clone()));
    Ok(events.into_iter().map(|event|match adapter{
        "kimi-code-cli"=>json!({"event":event,"command":command,"timeout":10}),
        "trae-cn-ide"=>json!({"event":event,"group":{"matcher":"","hooks":[{"type":"command","command":command,"timeout":10}]}}),
        "zcode-desktop"=>json!({"event":event,"group":{"matcher":"","hooks":[{"type":"process","command":exe,"args":["--integration-host-event","--source",source,"--adapter",adapter],"timeoutMs":10000}]}}),
        _=>Value::Null,
    }).collect())
}

fn own_hook(hook: &Value, commands: &BTreeSet<String>, source: &str, adapter: &str) -> bool {
    hook.get("command")
        .and_then(Value::as_str)
        .is_some_and(|c| commands.contains(c))
        || hook.get("args")
            == Some(&json!([
                "--integration-host-event",
                "--source",
                source,
                "--adapter",
                adapter
            ]))
}

fn merge_json(
    before: Option<&[u8]>,
    entries: &[Value],
    commands: &BTreeSet<String>,
    source: &str,
    adapter: &str,
    remove: bool,
) -> Result<Vec<u8>, String> {
    let mut value: Value = match before {
        Some(bytes) => {
            serde_json::from_slice(bytes).map_err(|_| "host_json_or_jsonc_unsupported_no_change")?
        }
        None => json!({}),
    };
    let root = value.as_object_mut().ok_or("host_config_not_object")?;
    if adapter == "trae-cn-ide" && root.get("version").is_some_and(|v| v.as_u64() != Some(1)) {
        return Err("unsupported_host_config_version".into());
    }
    if remove && !root.contains_key("hooks") {
        return serde_json::to_vec_pretty(&value).map_err(|_| "host_serialize_failed".into());
    }
    let hooks = root
        .entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("host_hooks_invalid")?;
    let target = if adapter == "zcode-desktop" {
        // Do not enable a user's disabled global Hook policy. New setups can
        // explicitly enable it in the host; this preview does not change policy.
        hooks
            .entry("events")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .ok_or("host_hook_events_invalid")?
    } else {
        hooks
    };
    for entry in entries {
        let event = entry["event"].as_str().ok_or("recipe_invalid")?;
        if !target.contains_key(event) && remove {
            continue;
        }
        let groups = target
            .entry(event)
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .ok_or("host_hook_groups_invalid")?;
        groups.retain_mut(|group| {
            if let Some(nested) = group.get_mut("hooks").and_then(Value::as_array_mut) {
                let owned = nested
                    .iter()
                    .any(|h| own_hook(h, commands, source, adapter));
                if owned {
                    nested.retain(|h| !own_hook(h, commands, source, adapter));
                    return !nested.is_empty();
                }
            }
            true
        });
        if !remove {
            groups.push(entry["group"].clone());
        }
        if groups.is_empty() {
            target.remove(event);
        }
    }
    serde_json::to_vec_pretty(&value).map_err(|_| "host_serialize_failed".into())
}

fn merge_toml(
    before: Option<&[u8]>,
    entries: &[Value],
    commands: &BTreeSet<String>,
    remove: bool,
) -> Result<Vec<u8>, String> {
    let text = std::str::from_utf8(before.unwrap_or(b"")).map_err(|_| "host_toml_not_utf8")?;
    let mut doc: toml_edit::DocumentMut =
        text.parse().map_err(|_| "host_toml_invalid_no_change")?;
    if remove && !doc.contains_key("hooks") {
        return Ok(text.as_bytes().to_vec());
    }
    let tables = doc
        .entry("hooks")
        .or_insert(toml_edit::Item::ArrayOfTables(
            toml_edit::ArrayOfTables::new(),
        ))
        .as_array_of_tables_mut()
        .ok_or("host_hooks_not_array_of_tables")?;
    tables.retain(|table| {
        !table
            .get("command")
            .and_then(toml_edit::Item::as_str)
            .is_some_and(|c| commands.contains(c))
    });
    if !remove {
        for entry in entries {
            let mut table = toml_edit::Table::new();
            table["event"] = toml_edit::value(entry["event"].as_str().ok_or("recipe_invalid")?);
            table["command"] = toml_edit::value(entry["command"].as_str().ok_or("recipe_invalid")?);
            table["timeout"] = toml_edit::value(10);
            tables.push(table);
        }
    }
    if tables.is_empty() {
        doc.remove("hooks");
    }
    Ok(doc.to_string().into_bytes())
}
pub fn prepare(source_id: &str, remove: bool) -> Result<Preview, String> {
    let source = bridge::configured_source(source_id)?;
    let config = source
        .integration
        .as_ref()
        .ok_or("integration_not_configured")?;
    let m = manifest::load(&config.adapter_id)?;
    if manifest::builtin(&m.adapter_id).is_none() {
        return Err("imported_manifest_cannot_install_hooks".into());
    }
    let exe = std::env::current_exe().map_err(|_| "executable_unavailable")?;
    let temporary = exe.components().any(|part| {
        ["codex-check", "debug", "ci-release", "release"]
            .iter()
            .any(|name| part.as_os_str() == *name)
    });
    if temporary && std::env::var_os("TASKPULSE_HOME").is_none() {
        return Err("install_taskpulse_at_persistent_path_before_installing_hooks".into());
    }
    let path = location(&m.adapter_id)?;
    if std::fs::symlink_metadata(&path).is_ok_and(|meta| meta.file_type().is_symlink()) {
        return Err("host_symlink_rejected".into());
    }
    let before = read(&path)?;
    let (command, decoded) = hook_command(&exe, source_id, &m.adapter_id);
    let entries = recipes(&m.adapter_id, &command, &exe, source_id)?;
    let mut commands = ownership(source_id);
    commands.insert(command);
    let after = if m.adapter_id == "kimi-code-cli" {
        merge_toml(before.as_deref(), &entries, &commands, remove)?
    } else {
        merge_json(
            before.as_deref(),
            &entries,
            &commands,
            source_id,
            &m.adapter_id,
            remove,
        )?
    };
    let binding = key(&serde_json::to_string(&json!([
        source,
        m.adapter_version,
        key(&serde_json::to_string(&m).map_err(|_| "manifest_invalid")?)
    ]))
    .map_err(|_| "preview_binding_failed")?);
    let nonce = tempfile::NamedTempFile::new().map_err(|_| "preview_nonce_failed")?;
    let token = key(&format!(
        "{}:{}:{}",
        now_ms(),
        nonce.path().display(),
        binding
    ));
    let public=Preview{token:token.clone(),source_id:source_id.into(),path:path.display().to_string(),action:if remove{"remove"}else{"install"}.into(),entries:entries.clone(),decoded_command:decoded,
        notes:vec!["仅显示 TaskPulse 自有条目，不返回完整配置或凭据；确认前不写宿主文件。".into(),"需要宿主重新加载/新建会话。ZCode 全局 hooks.enabled 不会被本工具改动，请在宿主中确认开关。".into(),"能力是文档/合成测试等级，不表示该宿主版本已实测。撤销只移除登记的 TaskPulse 条目。".into()]};
    let mut pending = PREVIEWS.lock().map_err(|_| "preview_lock_failed")?;
    pending.retain(|_, p| p.expires > now_ms());
    if pending.len() >= 16 {
        return Err("too_many_previews".into());
    }
    pending.insert(
        token,
        Pending {
            public: public.clone(),
            before,
            after,
            binding,
            path,
            commands,
            expires: now_ms() + 300_000,
        },
    );
    Ok(public)
}

pub fn apply(token: &str, confirmed: bool) -> Result<(), String> {
    if !confirmed {
        return Err("explicit_confirmation_required".into());
    }
    let pending = PREVIEWS
        .lock()
        .map_err(|_| "preview_lock_failed")?
        .remove(token)
        .ok_or("preview_missing_or_used")?;
    if pending.expires < now_ms() {
        return Err("preview_expired".into());
    }
    let source = bridge::configured_source(&pending.public.source_id)?;
    let m = manifest::load(
        &source
            .integration
            .as_ref()
            .ok_or("integration_not_configured")?
            .adapter_id,
    )?;
    let binding = key(&serde_json::to_string(&json!([
        source,
        m.adapter_version,
        key(&serde_json::to_string(&m).map_err(|_| "manifest_invalid")?)
    ]))
    .map_err(|_| "preview_binding_failed")?);
    if binding != pending.binding || read(&pending.path)? != pending.before {
        return Err("preview_conflict_regenerate_required".into());
    }
    let parent = pending.path.parent().ok_or("invalid_host_path")?;
    std::fs::create_dir_all(parent).map_err(|_| "host_directory_failed")?;
    let _lock = journal::lock(
        &pending.path.with_extension("taskpulse.lock"),
        std::time::Duration::from_millis(500),
    )?;
    if read(&pending.path)? != pending.before {
        return Err("preview_conflict_regenerate_required".into());
    }
    if let Some(before) = &pending.before {
        use std::io::Write;
        let backup = pending
            .path
            .with_extension("pre-taskpulse-integrations.bak");
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(backup)
        {
            Ok(mut file) => {
                file.write_all(before).map_err(|_| "backup_failed")?;
                file.sync_all().map_err(|_| "backup_failed")?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err("backup_failed".into()),
        }
    }
    // Register ownership BEFORE host write. A crash cannot strand unremovable
    // entries; a registry-only crash leaves harmless uninstalled registrations.
    journal::atomic(&owned_path(&pending.public.source_id), &pending.commands)?;
    use std::io::Write;
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|_| "host_temp_failed")?;
    temporary
        .write_all(&pending.after)
        .map_err(|_| "host_write_failed")?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|_| "host_sync_failed")?;
    temporary
        .persist(&pending.path)
        .map_err(|_| "host_replace_failed")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn trae_install_remove_are_idempotent_and_preserve_later_foreign_hooks() {
        let exe = Path::new("C:\\Fixture 中文 & team's app\\agent-tray.exe");
        let (command, decoded) = hook_command(exe, "source-a", "trae-cn-ide");
        assert!(decoded.contains("team''s app"));
        let entries = recipes("trae-cn-ide", &command, exe, "source-a").unwrap();
        let owned = BTreeSet::from([command]);
        let original = json!({"version":1,"env":{"proxy":"synthetic"},"model":"synthetic-model","statusLine":{"command":"keep"},"hooks":{"Stop":[{"hooks":[{"command":"user-hook"}]}]}});
        let first = merge_json(
            Some(&serde_json::to_vec(&original).unwrap()),
            &entries,
            &owned,
            "source-a",
            "trae-cn-ide",
            false,
        )
        .unwrap();
        let second = merge_json(
            Some(&first),
            &entries,
            &owned,
            "source-a",
            "trae-cn-ide",
            false,
        )
        .unwrap();
        assert_eq!(first, second);
        let mut later: Value = serde_json::from_slice(&second).unwrap();
        later["hooks"]["Stop"]
            .as_array_mut()
            .unwrap()
            .push(json!({"hooks":[{"command":"later-user-hook"}]}));
        let removed = merge_json(
            Some(&serde_json::to_vec(&later).unwrap()),
            &entries,
            &owned,
            "source-a",
            "trae-cn-ide",
            true,
        )
        .unwrap();
        let result: Value = serde_json::from_slice(&removed).unwrap();
        assert_eq!(result["env"], original["env"]);
        assert_eq!(result["statusLine"], original["statusLine"]);
        assert_eq!(result["hooks"]["Stop"].as_array().unwrap().len(), 2);
        assert!(!String::from_utf8(removed.clone())
            .unwrap()
            .contains("integration-host-event"));
        assert_eq!(
            removed,
            merge_json(
                Some(&removed),
                &entries,
                &owned,
                "source-a",
                "trae-cn-ide",
                true
            )
            .unwrap()
        );
    }
    #[test]
    fn kimi_toml_preserves_comments_fields_and_only_uses_documented_properties() {
        let exe = Path::new("C:\\Fixture App\\agent-tray.exe");
        let (command, _) = hook_command(exe, "source", "kimi-code-cli");
        let entries = recipes("kimi-code-cli", &command, exe, "source").unwrap();
        let owned = BTreeSet::from([command]);
        let original=b"# keep this comment\nmodel = 'synthetic-model'\n\n[[hooks]]\nevent = 'Stop'\ncommand = 'foreign'\n";
        let added = merge_toml(Some(original), &entries, &owned, false).unwrap();
        assert!(String::from_utf8_lossy(&added).contains("# keep this comment"));
        let parsed: toml::Value = toml::from_str(std::str::from_utf8(&added).unwrap()).unwrap();
        for entry in parsed["hooks"].as_array().unwrap() {
            assert!(entry
                .as_table()
                .unwrap()
                .keys()
                .all(|k| ["event", "command", "timeout", "matcher"].contains(&k.as_str())));
        }
        let removed = merge_toml(Some(&added), &entries, &owned, true).unwrap();
        let parsed: toml::Value = toml::from_str(std::str::from_utf8(&removed).unwrap()).unwrap();
        assert_eq!(parsed["model"].as_str(), Some("synthetic-model"));
        assert_eq!(parsed["hooks"].as_array().unwrap().len(), 1);
        assert!(String::from_utf8_lossy(&removed).contains("# keep this comment"));
    }
    #[test]
    fn zcode_direct_exec_does_not_change_disabled_hook_policy_or_other_sources() {
        let exe = Path::new("C:\\Fixture\\agent-tray.exe");
        let entries = recipes("zcode-desktop", "not-used", exe, "a").unwrap();
        let other = recipes("zcode-desktop", "not-used", exe, "b").unwrap();
        let owned = BTreeSet::new();
        let initial = json!({"hooks":{"enabled":false,"events":{}},"unknown":"keep"});
        let first = merge_json(
            Some(&serde_json::to_vec(&initial).unwrap()),
            &entries,
            &owned,
            "a",
            "zcode-desktop",
            false,
        )
        .unwrap();
        let both = merge_json(Some(&first), &other, &owned, "b", "zcode-desktop", false).unwrap();
        let removed: Value = serde_json::from_slice(
            &merge_json(Some(&both), &entries, &owned, "a", "zcode-desktop", true).unwrap(),
        )
        .unwrap();
        assert_eq!(removed["hooks"]["enabled"], false);
        assert_eq!(removed["unknown"], "keep");
        assert_eq!(
            removed["hooks"]["events"]["Stop"].as_array().unwrap().len(),
            1
        );
        assert!(merge_json(
            Some(b"{ // JSONC }"),
            &entries,
            &owned,
            "a",
            "zcode-desktop",
            false
        )
        .is_err());
    }
    #[test]
    fn apply_without_confirmation_or_unknown_preview_never_writes() {
        assert!(apply("not-a-token", false).is_err());
        assert!(apply("not-a-token", true).is_err());
    }
}
