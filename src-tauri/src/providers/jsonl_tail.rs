//! Append-only JSONL reader. Partial UTF-8/JSON lines survive the next refresh;
//! truncation/replacement resets the caller's parser instead of merging turns.
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::SystemTime;

pub const READ_BUDGET: usize = 256 * 1024;
pub const LINE_BUDGET: usize = 4 * 1024 * 1024;

/// Obtained inside Linux, not from the UNC redirector's synthetic timestamps.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct LinuxFileStamp {
    pub dev: u64,
    pub ino: u64,
    pub len: u64,
    pub modified_ns: u64,
    pub changed_ns: u64,
}

#[derive(Default)]
pub struct JsonlTail {
    ignored_envelopes: &'static [&'static str],
    oversized: Option<EnvelopeProbe>,
    offset: u64,
    signature: Option<(SystemTime, u64)>,
    created: Option<SystemTime>,
    anchor: Vec<u8>,
    pending: Vec<u8>,
    discarding: bool,
    gap: bool,
    pub bytes_read_total: u64,
    pub resets: u64,
    pub validation_bytes_total: u64,
    external_stamp: Option<LinuxFileStamp>,
    previous_linux_stamp: Option<LinuxFileStamp>,
    read_failed: bool,
    last_reset: Option<&'static str>,
    last_error: Option<String>,
    head: Vec<u8>,
}

pub struct TailBatch {
    pub reset: bool,
    pub body: String,
    pub caught_up: bool,
    pub gap: bool,
    pub bytes_read: usize,
}

impl JsonlTail {
    pub fn set_linux_stamp(&mut self, stamp: LinuxFileStamp) {
        self.external_stamp = Some(stamp);
    }
    /// Only the owning adapter may declare envelopes that contain no lifecycle
    /// evidence. Unknown/invalid oversized records still leave a permanent gap.
    pub fn ignoring_envelopes(names: &'static [&'static str]) -> Self {
        Self {
            ignored_envelopes: names,
            ..Self::default()
        }
    }
    /// More than one bounded buffer can be folded on the independent worker.
    /// This is a resource budget, never a lifecycle or approval timeout.
    pub fn drain(
        &mut self,
        path: &Path,
        mut fold: impl FnMut(&TailBatch),
    ) -> std::io::Result<bool> {
        let began = std::time::Instant::now();
        let mut bytes = 0usize;
        loop {
            let batch = self.read(path)?;
            bytes += batch.bytes_read;
            fold(&batch);
            if batch.caught_up
                || batch.bytes_read == 0
                || bytes >= 16 * 1024 * 1024
                || began.elapsed() >= std::time::Duration::from_millis(500)
            {
                return Ok(batch.caught_up && !batch.gap);
            }
        }
    }
    pub fn trusted(&self) -> bool {
        !self.read_failed
            && !self.gap
            && !self.discarding
            && self.pending.is_empty()
            && self.signature.is_some_and(|(_, len)| self.offset == len)
    }
    pub fn retained_bytes(&self) -> usize {
        self.pending.capacity() + self.anchor.capacity() + self.head.capacity()
    }
    pub fn diagnostics(&self) -> serde_json::Value {
        serde_json::json!({"length":self.signature.map(|(_,len)|len),"offset":self.offset,
            "reset_count":self.resets,"last_reset_reason":self.last_reset,"error_code":self.last_error,
            "cumulative_bytes":self.bytes_read_total,"validation_bytes":self.validation_bytes_total,"caught_up":self.trusted(),"gap":self.gap,
            "linux_identity":self.previous_linux_stamp.is_some()})
    }
    pub fn read_failed(&self) -> bool {
        self.read_failed
    }
    pub fn has_gap(&self) -> bool {
        self.gap
    }
    pub fn size_raced(&self) -> bool {
        self.last_error.as_deref() == Some("size_race")
    }
    pub fn read(&mut self, path: &Path) -> std::io::Result<TailBatch> {
        let result = self.read_inner(path);
        self.read_failed = result.is_err();
        self.last_error = result.as_ref().err().map(|e| {
            if e.kind() == std::io::ErrorKind::WouldBlock && e.raw_os_error().is_none() {
                "size_race".into()
            } else {
                format!("{:?}:{}", e.kind(), e.raw_os_error().unwrap_or(0))
            }
        });
        result
    }
    fn read_inner(&mut self, path: &Path) -> std::io::Result<TailBatch> {
        let mut file = File::open(path)?;
        let metadata = file.metadata()?;
        // UNC creation/mtime may both mirror Linux mtime, or drift without a
        // write. Only WSL's explicitly supplied Linux stat overrides them.
        // A size race is NOT permission to publish a historical terminal.
        if self
            .external_stamp
            .as_ref()
            .is_some_and(|s| s.len != metadata.len())
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "wsl_rollout_size_race",
            ));
        }
        let signature = if let Some(s) = &self.external_stamp {
            (
                SystemTime::UNIX_EPOCH + std::time::Duration::from_nanos(s.modified_ns),
                s.len,
            )
        } else {
            (metadata.modified()?, metadata.len())
        };
        let created = self
            .external_stamp
            .is_none()
            .then(|| metadata.created().ok())
            .flatten();
        let linux_replaced = self
            .previous_linux_stamp
            .as_ref()
            .zip(self.external_stamp.as_ref())
            .is_some_and(|(old, new)| old.dev != new.dev || old.ino != new.ino);
        let rewritten = self
            .previous_linux_stamp
            .as_ref()
            .zip(self.external_stamp.as_ref())
            .is_some_and(|(old, new)| old.len == new.len && old != new);
        let mut reason = if self.signature.is_none() {
            Some("initial_or_io_restart")
        } else if signature.1 < self.offset {
            Some("truncated")
        } else if linux_replaced {
            Some("linux_inode_replaced")
        } else if created != self.created {
            Some("creation_changed")
        } else if rewritten
            || self.external_stamp.is_none()
                && self
                    .signature
                    .is_some_and(|old| old != signature && old.1 == signature.1)
        {
            Some("same_length_rewrite")
        } else {
            None
        };
        // Validate content anchors even on unchanged Linux stat. This catches
        // tail overwrites and replacement races without trusting UNC times.
        if reason.is_none() && !self.anchor.is_empty() {
            file.seek(SeekFrom::Start(self.offset - self.anchor.len() as u64))?;
            let mut anchor = vec![0; self.anchor.len()];
            file.read_exact(&mut anchor)?;
            file.seek(SeekFrom::Start(0))?;
            let mut head = vec![0; self.head.len()];
            file.read_exact(&mut head)?;
            self.validation_bytes_total += (anchor.len() + head.len()) as u64;
            if anchor != self.anchor || head != self.head {
                reason = Some("content_anchor_changed");
            }
        }
        if reason.is_none() && self.signature == Some(signature) && self.offset == signature.1 {
            self.previous_linux_stamp = self.external_stamp.clone();
            return Ok(TailBatch {
                reset: false,
                body: String::new(),
                caught_up: self.pending.is_empty() && !self.discarding,
                gap: self.gap,
                bytes_read: 0,
            });
        }
        let reset = reason.is_some();
        let offset = if reset { 0 } else { self.offset };
        file.seek(SeekFrom::Start(offset))?;
        if reset {
            self.resets += 1;
            self.last_reset = reason;
            self.pending = Vec::new();
            self.discarding = false;
            self.oversized = None;
            self.gap = false;
        }
        // An I/O error after a partial read must force a parser reset next
        // time, rather than append the same bytes twice to pending state.
        self.signature = None;
        let mut body = String::new();
        let mut bytes_read = 0;
        let mut chunk = [0u8; 16 * 1024];
        while bytes_read < READ_BUDGET {
            let remaining = signature.1.saturating_sub(offset + bytes_read as u64);
            let available = chunk
                .len()
                .min(READ_BUDGET - bytes_read)
                .min(remaining as usize);
            if available == 0 {
                break;
            }
            let read = file.read(&mut chunk[..available])?;
            if read == 0 {
                break;
            }
            bytes_read += read;
            self.bytes_read_total = self.bytes_read_total.saturating_add(read as u64);
            for part in chunk[..read].split_inclusive(|b| *b == b'\n') {
                let ended = part.last() == Some(&b'\n');
                if self.discarding {
                    if let Some(probe) = &mut self.oversized {
                        probe.feed(part);
                    }
                } else {
                    if self.pending.len() + part.len() > LINE_BUDGET {
                        let mut probe = EnvelopeProbe::default();
                        probe.feed(&self.pending);
                        probe.feed(part);
                        self.oversized = Some(probe);
                        self.pending = Vec::new();
                        self.discarding = true;
                    } else {
                        let needed = self.pending.len() + part.len();
                        if needed > self.pending.capacity() {
                            let target = needed
                                .max(self.pending.capacity().saturating_mul(2))
                                .min(LINE_BUDGET);
                            self.pending.reserve_exact(target - self.pending.len());
                        }
                        self.pending.extend_from_slice(part);
                    }
                }
                if ended {
                    if self.discarding {
                        self.gap |= !self
                            .oversized
                            .take()
                            .is_some_and(|p| p.ignored(self.ignored_envelopes));
                    } else {
                        self.consume_line(&mut body);
                    }
                    self.discarding = false;
                }
            }
        }
        self.offset = offset + bytes_read as u64;
        // Some versions write a complete final JSON object without a newline.
        if self.offset == signature.1
            && !self.pending.is_empty()
            && serde_json::from_slice::<serde::de::IgnoredAny>(&self.pending).is_ok()
        {
            self.consume_line(&mut body);
        }
        file.seek(SeekFrom::Start(self.offset.saturating_sub(64)))?;
        let mut anchor = vec![0; self.offset.min(64) as usize];
        file.read_exact(&mut anchor)?;
        self.anchor = anchor;
        file.seek(SeekFrom::Start(0))?;
        let mut head = vec![0; self.offset.min(64) as usize];
        file.read_exact(&mut head)?;
        self.validation_bytes_total += (self.anchor.len() + head.len()) as u64;
        self.head = head;
        self.created = created;
        self.previous_linux_stamp = self.external_stamp.clone();
        self.signature = Some(signature);
        Ok(TailBatch {
            reset,
            body,
            caught_up: self.offset == signature.1 && self.pending.is_empty() && !self.discarding,
            gap: self.gap,
            bytes_read,
        })
    }
    fn consume_line(&mut self, body: &mut String) {
        if self.pending.iter().all(u8::is_ascii_whitespace) {
            self.pending = Vec::new();
            return;
        }
        if let Ok(line) = std::str::from_utf8(&self.pending) {
            if serde_json::from_str::<serde::de::IgnoredAny>(line).is_ok() {
                body.push_str(line);
                if !line.ends_with('\n') {
                    body.push('\n');
                }
            } else {
                self.gap = true;
            }
        } else {
            self.gap = true;
        }
        self.pending = Vec::new();
    }
}

/// Bounded lexical discriminator for adapter-declared irrelevant envelopes.
/// Root fields may occur after megabytes of nested history. Never search text,
/// retain history, trust duplicate discriminators, or filter unknown events.
#[derive(Default)]
struct EnvelopeProbe {
    depth: usize,
    in_string: bool,
    escaped: bool,
    key: bool,
    type_value: bool,
    capture: bool,
    text: Vec<u8>,
    name: Option<String>,
    ambiguous: bool,
    object: bool,
}
impl EnvelopeProbe {
    fn feed(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            if self.in_string {
                if self.capture && self.text.len() < 256 {
                    self.text.push(byte);
                } else if self.capture {
                    self.ambiguous = true;
                }
                if self.escaped {
                    self.escaped = false;
                    continue;
                }
                if byte == b'\\' {
                    self.escaped = true;
                    continue;
                }
                if byte == b'"' {
                    self.in_string = false;
                    if self.capture {
                        let value = serde_json::from_slice::<String>(&self.text).ok();
                        if self.key {
                            self.type_value = value.as_deref() == Some("type");
                            if self.type_value && self.name.is_some() {
                                self.ambiguous = true;
                            }
                            self.key = false;
                        } else if self.type_value {
                            if self.name.is_some() {
                                self.ambiguous = true;
                            }
                            self.name = value;
                            self.type_value = false;
                        }
                    }
                }
                continue;
            }
            match byte {
                b'"' => {
                    self.in_string = true;
                    self.capture = self.depth == 1 && (self.key || self.type_value);
                    self.text.clear();
                    if self.capture {
                        self.text.push(b'"');
                    }
                }
                b'{' | b'[' => {
                    if self.depth == 0 {
                        self.object = byte == b'{';
                        self.key = self.object;
                    }
                    self.depth = self.depth.saturating_add(1);
                }
                b'}' | b']' => self.depth = self.depth.saturating_sub(1),
                b',' if self.depth == 1 => {
                    self.key = true;
                    self.type_value = false;
                }
                _ => {}
            }
        }
    }
    fn ignored(&self, allowed: &[&str]) -> bool {
        self.object
            && self.depth == 0
            && !self.in_string
            && !self.ambiguous
            && self
                .name
                .as_deref()
                .is_some_and(|name| allowed.contains(&name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    #[test]
    fn lifecycle_oversized_filter_uses_only_unique_root_kind_and_remains_opt_in() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("large.jsonl");
        for kind in ["compacted", "event_msg"] {
            let row = serde_json::json!({"payload":{"type":"compacted","text":"x".repeat(LINE_BUDGET+1)},"type":kind});
            std::fs::write(&path, format!("{row}\n{{\"next\":true}}\n")).unwrap();
            let mut tail = JsonlTail::ignoring_envelopes(&["compacted"]);
            for _ in 0..30 {
                if tail.read(&path).unwrap().caught_up {
                    break;
                }
            }
            assert_eq!(tail.trusted(), kind == "compacted");
            assert!(tail.retained_bytes() < 1024);
            let mut normal = JsonlTail::default();
            for _ in 0..30 {
                if normal.read(&path).unwrap().caught_up {
                    break;
                }
            }
            assert!(!normal.trusted());
        }
        let mut probe = EnvelopeProbe::default();
        probe.feed(br#"{"type":"compacted","type":"event_msg","payload":{}}"#);
        assert!(!probe.ignored(&["compacted"]));
    }
    #[test]
    fn f04_eight_readers_release_consumed_eight_mib_buffers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("large.jsonl");
        let line = format!("{{\"payload\":\"{}\"}}\n", "x".repeat(1009));
        assert_eq!(line.len(), 1024);
        std::fs::write(&path, line.repeat(8192)).unwrap();
        let mut readers: Vec<_> = (0..8).map(|_| JsonlTail::default()).collect();
        let mut largest_batch = 0;
        for tail in &mut readers {
            let mut consumed = 0;
            while consumed < 8 * 1024 * 1024 {
                let batch = tail.read(&path).unwrap();
                assert!(!batch.body.is_empty());
                largest_batch = largest_batch.max(batch.body.len());
                consumed += batch.body.len();
            }
        }
        let capacity: usize = readers.iter().map(|t| t.pending.capacity()).sum();
        eprintln!(
            "8x8MiB retained Vec capacity={capacity}; largest returned batch={largest_batch}"
        );
        assert_eq!(capacity, 0);
        assert!(largest_batch <= 512 * 1024);
    }
    #[test]
    fn f04_budget_crossing_unicode_final_record_and_corruption_are_conservative() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tail.jsonl");
        let record = format!("{{\"text\":\"{}中文\"}}", "a".repeat(READ_BUDGET - 11));
        std::fs::write(&path, &record).unwrap();
        let mut tail = JsonlTail::default();
        let first = tail.read(&path).unwrap();
        assert!(!first.caught_up);
        assert!(first.body.is_empty());
        let final_record = tail.read(&path).unwrap();
        assert!(final_record.caught_up && !final_record.gap);
        assert_eq!(final_record.body.trim(), record);
        assert_eq!(tail.pending.capacity(), 0);
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        writeln!(file, "\n{{broken}}").unwrap();
        file.write_all(
            format!(
                "{{\"huge\":\"{}\"}}\n{{\"end\":true}}\n",
                "a".repeat(LINE_BUDGET + 1)
            )
            .as_bytes(),
        )
        .unwrap();
        let mut found_end = false;
        for _ in 0..30 {
            let batch = tail.read(&path).unwrap();
            found_end |= batch.body.contains("\"end\":true");
            assert!(tail.pending.capacity() <= LINE_BUDGET);
            if batch.caught_up {
                assert!(batch.gap);
                break;
            }
        }
        assert!(found_end);
        assert!(!tail.trusted());
        std::fs::write(&path, b"{\"new\":true}\n").unwrap();
        assert!(tail.read(&path).unwrap().reset);
        assert!(tail.trusted());
    }
    #[test]
    fn append_partial_unicode_and_replacement() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(&path, b"{\"type\":\"start\"}\n").unwrap();
        let mut tail = JsonlTail::default();
        assert!(tail.read(&path).unwrap().reset);
        assert_eq!(tail.read(&path).unwrap().bytes_read, 0);
        let record = "{\"title\":\"重命名\"}\n".as_bytes();
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        file.write_all(&record[..12]).unwrap();
        assert!(tail.read(&path).unwrap().body.is_empty());
        file.write_all(&record[12..]).unwrap();
        let next = tail.read(&path).unwrap();
        assert!(!next.reset);
        assert_eq!(next.bytes_read, record.len() - 12);
        assert!(next.body.contains("重命名"));
        std::fs::write(&path, b"{\"type\":\"end\"}\n").unwrap();
        assert!(tail.read(&path).unwrap().reset);
    }

    fn stamp(path: &Path, version: u64) -> LinuxFileStamp {
        LinuxFileStamp {
            dev: 1,
            ino: 42,
            len: path.metadata().unwrap().len(),
            modified_ns: version,
            changed_ns: version,
        }
    }

    #[test]
    fn wsl_continuity_unchanged_content_with_unc_time_drift_is_not_replayed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(&path, "{\"event\":\"running\"}\n").unwrap();
        let mut tail = JsonlTail::default();
        tail.set_linux_stamp(stamp(&path, 100));
        tail.read(&path).unwrap();
        let initial_bytes = tail.bytes_read_total;
        for n in 1..=4 {
            // Simulate the UNC redirector changing timestamps without changing
            // the Linux inode, contents or Linux timestamps.
            File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_times(
                    std::fs::FileTimes::new()
                        .set_modified(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(n)),
                )
                .unwrap();
            tail.set_linux_stamp(stamp(&path, 100));
            assert!(!tail.read(&path).unwrap().reset);
            assert!(tail.trusted());
        }
        assert_eq!(tail.resets, 1);
        assert_eq!(tail.bytes_read_total, initial_bytes);
    }

    #[test]
    fn wsl_continuity_linux_identity_rewrite_truncation_and_read_failure() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log.jsonl");
        let content = format!("{{\"text\":\"{}\"}}\n", "x".repeat(512));
        std::fs::write(&path, &content).unwrap();
        let mut tail = JsonlTail::default();
        tail.set_linux_stamp(stamp(&path, 100));
        assert!(tail.read(&path).unwrap().reset);
        // Same length, identical head/tail but a changed middle: Linux ctime
        // detects an in-place rewrite that sampled content anchors cannot.
        let mut rewritten = content.clone();
        rewritten.replace_range(200..328, &"y".repeat(128));
        std::fs::write(&path, rewritten).unwrap();
        tail.set_linux_stamp(stamp(&path, 200));
        assert!(tail.read(&path).unwrap().reset);
        let mut replaced = stamp(&path, 200);
        replaced.ino = 43;
        tail.set_linux_stamp(replaced);
        assert!(tail.read(&path).unwrap().reset);
        std::fs::write(&path, "{}\n").unwrap();
        tail.set_linux_stamp(stamp(&path, 300));
        assert!(tail.read(&path).unwrap().reset);
        // Even if stat timestamps were unexpectedly preserved, the cursor
        // anchor catches a tail overwrite. An append is incremental.
        std::fs::write(&path, "[]\n").unwrap();
        assert!(tail.read(&path).unwrap().reset);
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"{}\n")
            .unwrap();
        tail.set_linux_stamp(stamp(&path, 400));
        let append = tail.read(&path).unwrap();
        assert!(!append.reset);
        assert_eq!(append.bytes_read, 3);
        std::fs::remove_file(&path).unwrap();
        assert!(tail.read(&path).is_err());
        assert!(!tail.trusted());
        // Size races are uncertainty, never an authoritative old terminal.
        std::fs::write(&path, "{}\n").unwrap();
        assert!(tail.read(&path).is_err());
        assert!(!tail.trusted());
        tail.set_linux_stamp(stamp(&path, 500));
        assert!(tail.read(&path).unwrap().caught_up);
        assert!(tail.trusted());
    }

    #[test]
    fn wsl_continuity_large_fixed_log_hot_polls_read_zero_payload_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("large.jsonl");
        let line = format!("{{\"value\":\"{}\"}}\n", "x".repeat(2000));
        std::fs::write(&path, line.repeat(2500)).unwrap();
        let mut tail = JsonlTail::default();
        tail.set_linux_stamp(stamp(&path, 1));
        let mut legacy = JsonlTail::default();
        while !legacy.drain(&path, |_| {}).unwrap() {}
        let legacy_bytes = legacy.bytes_read_total;
        let cold = std::time::Instant::now();
        while !tail.drain(&path, |_| {}).unwrap() {}
        let bytes = tail.bytes_read_total;
        let cold_ms = cold.elapsed().as_millis();
        let hot = std::time::Instant::now();
        let mut fixed_hot_us = 0;
        let mut legacy_hot_us = 0;
        let validation_before = tail.validation_bytes_total;
        for n in 1..=10 {
            File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_times(
                    std::fs::FileTimes::new()
                        .set_modified(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(n)),
                )
                .unwrap();
            let timer = std::time::Instant::now();
            assert!(tail.drain(&path, |_| {}).unwrap());
            fixed_hot_us += timer.elapsed().as_micros();
            let timer = std::time::Instant::now();
            while !legacy.drain(&path, |_| {}).unwrap() {}
            legacy_hot_us += timer.elapsed().as_micros();
        }
        println!("{{\"fixture_bytes\":{bytes},\"cold_ms\":{cold_ms},\"comparison_10_ms\":{},\"fixed_hot_us\":{fixed_hot_us},\"legacy_hot_us\":{legacy_hot_us},\"hot_payload_bytes\":{},\"resets\":{},\"legacy_hot_payload_bytes\":{},\"legacy_resets\":{},\"hot_validation_bytes\":{}}}",hot.elapsed().as_millis(),tail.bytes_read_total-bytes,tail.resets,legacy.bytes_read_total-legacy_bytes,legacy.resets,tail.validation_bytes_total-validation_before);
        assert_eq!(tail.bytes_read_total, bytes);
        assert_eq!(tail.resets, 1);
    }
}
