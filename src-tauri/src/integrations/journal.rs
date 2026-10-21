//! Append-only facts + atomic reducer/cursor/outbox checkpoints. All paths are
//! derived from stable SHA-256 keys, never producer-provided filenames.
use super::{
    key, now_ms,
    schema::{Envelope, Event, MAX_INPUT},
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub const MAX_LINE: usize = MAX_INPUT + 8192;
pub const MAX_READ: usize = 256 * 1024;
pub const MAX_BATCH: usize = 128;
pub const SESSION_DISK_LIMIT: u64 = 32 * 1024 * 1024;
pub const SOURCE_DISK_LIMIT: u64 = 128 * 1024 * 1024;
pub const CHECKPOINT_LIMIT: usize = 16 * 1024 * 1024;
pub const OUTBOX_LIMIT: usize = 4 * 1024 * 1024;

pub fn backup(path: &Path, extension: &str) -> Result<(), String> {
    let target = path.with_extension(extension);
    if target.exists() {
        return Ok(());
    }
    let parent = path.parent().ok_or("invalid_backup_path")?;
    let mut temp =
        tempfile::NamedTempFile::new_in(parent).map_err(|_| "checkpoint_backup_failed")?;
    let mut input = File::open(path).map_err(|_| "checkpoint_backup_failed")?;
    std::io::copy(&mut input, temp.as_file_mut()).map_err(|_| "checkpoint_backup_failed")?;
    temp.as_file()
        .sync_all()
        .map_err(|_| "checkpoint_backup_failed")?;
    temp.persist_noclobber(&target)
        .map_err(|_| "checkpoint_backup_failed")?;
    Ok(())
}

struct LimitedWriter<W> {
    inner: W,
    bytes: usize,
    limit: usize,
}
impl<W: Write> Write for LimitedWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if buf.len() > self.limit.saturating_sub(self.bytes) {
            return Err(std::io::Error::other("state_budget_exceeded"));
        }
        self.inner.write_all(buf)?;
        self.bytes += buf.len();
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}
pub fn serialized_size(value: &impl Serialize, limit: usize) -> Result<usize, String> {
    let mut writer = LimitedWriter {
        inner: std::io::sink(),
        bytes: 0,
        limit,
    };
    serde_json::to_writer(&mut writer, value).map_err(|_| "state_budget_exceeded")?;
    Ok(writer.bytes)
}

pub fn root(home: &Path, source: &str) -> PathBuf {
    home.join(key(source))
}

pub fn lock(path: &Path, wait: Duration) -> Result<File, String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|_| "create_directory_failed")?;
    }
    if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err("symlink_rejected".into());
    }
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .map_err(|_| "lock_open_failed")?;
    let started = Instant::now();
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(std::fs::TryLockError::WouldBlock) if started.elapsed() < wait => {
                std::thread::sleep(Duration::from_millis(5))
            }
            Err(std::fs::TryLockError::WouldBlock) => return Err("lock_timeout".into()),
            Err(_) => return Err("lock_failed".into()),
        }
    }
}

pub fn atomic(path: &Path, value: &impl Serialize) -> Result<(), String> {
    atomic_bounded(path, value, usize::MAX)
}
pub fn atomic_bounded(path: &Path, value: &impl Serialize, limit: usize) -> Result<(), String> {
    let parent = path.parent().ok_or("invalid_state_path")?;
    std::fs::create_dir_all(parent).map_err(|_| "create_directory_failed")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|_| "state_temp_failed")?;
    let mut writer = LimitedWriter {
        inner: temporary.as_file_mut(),
        bytes: 0,
        limit,
    };
    serde_json::to_writer(&mut writer, value).map_err(|_| "state_serialize_or_budget_failed")?;
    temporary
        .as_file_mut()
        .flush()
        .map_err(|_| "state_flush_failed")?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|_| "state_sync_failed")?;
    temporary
        .persist(path)
        .map_err(|_| "state_replace_failed")?;
    Ok(())
}

#[cfg(test)]
pub fn append(
    home: &Path,
    source: &str,
    event: Event,
    evidence: &str,
    adapter_version: &str,
) -> Result<Envelope, String> {
    event.validate(&event.agent_id)?;
    let session = event.session_id.clone();
    append_using(
        home,
        source,
        &session,
        evidence,
        adapter_version,
        |_, _, _| Ok((event, None)),
    )
}

pub fn append_using(
    home: &Path,
    source: &str,
    session: &str,
    evidence: &str,
    adapter_version: &str,
    make: impl FnOnce(
        Option<&Envelope>,
        u64,
        &str,
    ) -> Result<(Event, Option<serde_json::Value>), String>,
) -> Result<Envelope, String> {
    let dir = root(home, source).join("events");
    let _session_guard = lock(
        &root(home, source)
            .join("writers")
            .join(format!("{}.lock", key(session))),
        Duration::from_secs(2),
    )?;
    std::fs::create_dir_all(&dir).map_err(|_| "create_directory_failed")?;
    let path = dir.join(format!("{}.jsonl", key(session)));
    if std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err("symlink_rejected".into());
    }
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|_| "journal_open_failed")?;
    let length = file
        .metadata()
        .map_err(|_| "journal_metadata_failed")?
        .len();
    file.seek(SeekFrom::Start(
        length.saturating_sub((MAX_LINE * 2) as u64),
    ))
    .map_err(|_| "journal_seek_failed")?;
    let mut tail = vec![];
    file.read_to_end(&mut tail)
        .map_err(|_| "journal_read_failed")?;
    let boundary = tail.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
    if boundary < tail.len() {
        file.set_len(length - tail.len() as u64 + boundary as u64)
            .map_err(|_| "partial_tail_recovery_failed")?;
    }
    let previous = tail[..boundary]
        .split(|b| *b == b'\n')
        .rev()
        .find(|line| !line.is_empty())
        .map(serde_json::from_slice::<Envelope>)
        .transpose()
        .map_err(|_| "journal_tail_corrupt_export_before_repair")?;
    if previous
        .as_ref()
        .is_some_and(|e| e.source_instance_id != source || e.event.session_id != session)
    {
        return Err("journal_identity_mismatch".into());
    }
    let epoch = if let Some(e) = &previous {
        e.journal_epoch.clone()
    } else {
        let nonce = tempfile::NamedTempFile::new_in(&dir).map_err(|_| "epoch_failed")?;
        key(&format!(
            "{}:{}:{}",
            now_ms(),
            std::process::id(),
            nonce.path().display()
        ))
    };
    let sequence = previous.as_ref().map_or(Ok(1), |e| {
        e.sequence.checked_add(1).ok_or("sequence_overflow")
    })?;
    let (event, context) = make(previous.as_ref(), sequence, &epoch)?;
    event.validate(&event.agent_id)?;
    if event.session_id != session {
        return Err("journal_identity_mismatch".into());
    }
    let envelope = Envelope {
        version: 1,
        source_instance_id: source.into(),
        sequence,
        journal_epoch: epoch,
        received_at: now_ms(),
        adapter_version: adapter_version.into(),
        evidence: evidence.into(),
        event,
        context,
    };
    let mut bytes = serde_json::to_vec(&envelope).map_err(|_| "serialize_failed")?;
    bytes.push(b'\n');
    if bytes.len() > MAX_LINE {
        return Err("line_too_large".into());
    }
    // Serialize quota admission and the visible append, not the slow fsync.
    // Independent sessions can flush concurrently without starving each other;
    // their own OS lock still protects sequence/tail recovery until durability.
    let quota_guard = lock(
        &root(home, source).join("writer.lock"),
        Duration::from_secs(2),
    )?;
    let total = std::fs::read_dir(&dir)
        .map_err(|_| "quota_read_failed")?
        .try_fold(0u64, |sum, entry| {
            entry
                .and_then(|e| e.metadata())
                .map(|m| sum.saturating_add(m.len()))
        })
        .map_err(|_| "quota_read_failed")?;
    if length + bytes.len() as u64 > SESSION_DISK_LIMIT
        || total + bytes.len() as u64 > SOURCE_DISK_LIMIT
    {
        return Err("journal_quota_export_before_reset".into());
    }
    file.seek(SeekFrom::End(0))
        .map_err(|_| "journal_seek_failed")?;
    file.write_all(&bytes)
        .map_err(|_| "journal_append_failed")?;
    drop(quota_guard);
    file.sync_all().map_err(|_| "journal_sync_failed")?;
    Ok(envelope)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Cursor {
    pub offset: u64,
    pub epoch: String,
    pub sequence: u64,
    pub anchor_start: u64,
    pub anchor_hash: String,
}
pub struct Batch {
    pub events: Vec<Envelope>,
    pub cursor: Cursor,
    pub corrupt: usize,
    pub backlog: bool,
    pub generation_changed: bool,
    pub partial: bool,
    pub event_cursors: Vec<Cursor>,
}

pub fn read_batch(path: &Path, old: &Cursor) -> Result<Batch, String> {
    if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err("symlink_rejected".into());
    }
    let mut file = File::open(path).map_err(|_| "journal_open_failed")?;
    let length = file
        .metadata()
        .map_err(|_| "journal_metadata_failed")?
        .len();
    if length > SESSION_DISK_LIMIT {
        return Err("journal_read_limit".into());
    }
    let mut cursor = old.clone();
    let mut generation_changed = length < cursor.offset;
    if !generation_changed && cursor.offset > cursor.anchor_start {
        let anchor_size = (cursor.offset - cursor.anchor_start) as usize;
        if anchor_size > MAX_LINE {
            generation_changed = true;
        } else {
            let mut anchor = vec![0; anchor_size];
            file.seek(SeekFrom::Start(cursor.anchor_start))
                .map_err(|_| "journal_seek_failed")?;
            generation_changed = file.read_exact(&mut anchor).is_err()
                || key(&String::from_utf8_lossy(&anchor)) != cursor.anchor_hash;
        }
    }
    if generation_changed {
        cursor = Cursor::default();
    }
    file.seek(SeekFrom::Start(cursor.offset))
        .map_err(|_| "journal_seek_failed")?;
    let mut bytes = vec![];
    file.take(MAX_READ as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| "journal_read_failed")?;
    let mut events = vec![];
    let mut corrupt = 0;
    let mut consumed = 0;
    let mut partial = false;
    let mut event_cursors = vec![];
    for line in bytes.split_inclusive(|b| *b == b'\n') {
        if events.len() + corrupt >= MAX_BATCH {
            break;
        }
        if line.last() != Some(&b'\n') {
            partial = true;
            if line.len() > MAX_LINE {
                return Err("journal_oversized_or_partial_line".into());
            }
            break;
        }
        if line.len() > MAX_LINE {
            return Err("journal_oversized_line".into());
        }
        let mut valid_event = false;
        match serde_json::from_slice::<Envelope>(line) {
            Ok(e) if e.version == 1 => {
                if cursor.epoch.is_empty() {
                    cursor.epoch = e.journal_epoch.clone();
                }
                if cursor.epoch != e.journal_epoch || e.sequence <= cursor.sequence {
                    return Err("journal_generation_or_sequence_conflict".into());
                }
                cursor.sequence = e.sequence;
                events.push(e);
                valid_event = true;
            }
            _ => {
                corrupt += 1;
            }
        }
        cursor.anchor_start = cursor.offset + consumed as u64;
        cursor.anchor_hash = key(&String::from_utf8_lossy(line));
        consumed += line.len();
        if valid_event {
            let mut next = cursor.clone();
            next.offset += consumed as u64;
            event_cursors.push(next);
        }
    }
    cursor.offset += consumed as u64;
    let backlog =
        cursor.offset < length && bytes.len() >= MAX_READ || events.len() + corrupt >= MAX_BATCH;
    Ok(Batch {
        events,
        cursor,
        corrupt,
        backlog,
        generation_changed,
        partial,
        event_cursors,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event(id: &str) -> Event {
        serde_json::from_value(serde_json::json!({"schema_version":1,"agent_id":"fixture","session_id":"../会话:a","event_id":id,"kind":"turn.started","turn_id":id})).unwrap()
    }
    #[test]
    fn journal_partial_tail_is_recovered_and_state_cursor_is_not_advanced_early() {
        let tmp = tempfile::tempdir().unwrap();
        let first = append(
            tmp.path(),
            "../../source",
            event("one"),
            "self_reported",
            "v1",
        )
        .unwrap();
        let path = root(tmp.path(), "../../source")
            .join("events")
            .join(format!("{}.jsonl", key("../会话:a")));
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(b"{partial").unwrap();
        drop(file);
        let partial = read_batch(&path, &Cursor::default()).unwrap();
        assert!(partial.partial);
        assert_eq!(partial.events.len(), 1);
        let second = append(
            tmp.path(),
            "../../source",
            event("two"),
            "self_reported",
            "v1",
        )
        .unwrap();
        assert_eq!(second.sequence, 2);
        assert_eq!(first.journal_epoch, second.journal_epoch);
        let batch = read_batch(&path, &Cursor::default()).unwrap();
        assert_eq!(batch.events.len(), 2);
        assert_eq!(
            read_batch(&path, &Cursor::default()).unwrap().events.len(),
            2
        ); // replay before atomic checkpoint
        assert!(read_batch(&path, &batch.cursor).unwrap().events.is_empty());
        std::fs::write(&path, b"").unwrap();
        let fresh = append(
            tmp.path(),
            "../../source",
            event("three"),
            "self_reported",
            "v1",
        )
        .unwrap();
        assert_ne!(fresh.journal_epoch, first.journal_epoch);
        let replaced = read_batch(&path, &batch.cursor).unwrap();
        assert!(replaced.generation_changed);
        assert_eq!(replaced.events[0].event.event_id, "three");
    }
    #[test]
    fn lock_wait_is_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lock");
        let _guard = lock(&path, Duration::from_millis(20)).unwrap();
        assert!(lock(&path, Duration::from_millis(20)).is_err());
    }

    #[test]
    fn bad_complete_line_and_same_size_or_larger_replacement_are_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let first = append(dir.path(), "source", event("a"), "self_reported", "v1").unwrap();
        let path = root(dir.path(), "source")
            .join("events")
            .join(format!("{}.jsonl", key("../会话:a")));
        let mut second = first.clone();
        second.sequence = 2;
        second.event = event("b");
        let mut bytes = serde_json::to_vec(&first).unwrap();
        bytes.extend_from_slice(b"\n{broken-complete-line}\n");
        bytes.extend(serde_json::to_vec(&second).unwrap());
        bytes.push(b'\n');
        std::fs::write(&path, bytes).unwrap();
        let batch = read_batch(&path, &Cursor::default()).unwrap();
        assert_eq!(batch.events.len(), 2);
        assert_eq!(batch.corrupt, 1);
        assert!(!batch.partial);
        let mut replacement = first;
        replacement.journal_epoch = "new-generation".into();
        replacement.event = event("replacement");
        replacement.event.payload.title = Some("x".repeat(1800));
        let mut bytes = serde_json::to_vec(&replacement).unwrap();
        bytes.push(b'\n');
        assert!(bytes.len() as u64 > batch.cursor.offset);
        std::fs::write(&path, bytes).unwrap();
        let rewritten = read_batch(&path, &batch.cursor).unwrap();
        assert!(rewritten.generation_changed);
        assert_eq!(rewritten.events[0].event.event_id, "replacement");
    }
}
