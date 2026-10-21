//! Append-only JSONL reader. Partial UTF-8/JSON lines survive the next refresh;
//! truncation/replacement resets the caller's parser instead of merging turns.
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::SystemTime;

pub const READ_BUDGET: usize = 256 * 1024;
pub const LINE_BUDGET: usize = 4 * 1024 * 1024;

#[derive(Default)]
pub struct JsonlTail {
    offset: u64,
    signature: Option<(SystemTime, u64)>,
    created: Option<SystemTime>,
    anchor: Vec<u8>,
    pending: Vec<u8>,
    discarding: bool,
    gap: bool,
    pub bytes_read_total: u64,
}

pub struct TailBatch {
    pub reset: bool,
    pub body: String,
    pub caught_up: bool,
    pub gap: bool,
    #[cfg(test)]
    pub bytes_read: usize,
}

impl JsonlTail {
    pub fn trusted(&self) -> bool {
        !self.gap
            && !self.discarding
            && self.pending.is_empty()
            && self.signature.is_some_and(|(_, len)| self.offset == len)
    }
    pub fn retained_bytes(&self) -> usize {
        self.pending.capacity() + self.anchor.capacity()
    }
    pub fn read(&mut self, path: &Path) -> std::io::Result<TailBatch> {
        let mut file = File::open(path)?;
        let metadata = file.metadata()?;
        let signature = (metadata.modified()?, metadata.len());
        if self.signature == Some(signature) && self.offset == metadata.len() {
            return Ok(TailBatch {
                reset: false,
                body: String::new(),
                caught_up: self.pending.is_empty() && !self.discarding,
                gap: self.gap,
                #[cfg(test)]
                bytes_read: 0,
            });
        }
        let created = metadata.created().ok();
        let mut reset = self.signature.is_none()
            || metadata.len() < self.offset
            || created != self.created
            || self
                .signature
                .is_some_and(|old| old != signature && old.1 == metadata.len());
        if !reset && !self.anchor.is_empty() {
            file.seek(SeekFrom::Start(self.offset - self.anchor.len() as u64))?;
            let mut anchor = vec![0; self.anchor.len()];
            file.read_exact(&mut anchor)?;
            reset = anchor != self.anchor;
        }
        let offset = if reset { 0 } else { self.offset };
        file.seek(SeekFrom::Start(offset))?;
        if reset {
            self.pending = Vec::new();
            self.discarding = false;
            self.gap = false;
        }
        // An I/O error after a partial read must force a parser reset next
        // time, rather than append the same bytes twice to pending state.
        self.signature = None;
        let mut body = String::new();
        let mut bytes_read = 0;
        let mut chunk = [0u8; 16 * 1024];
        while bytes_read < READ_BUDGET {
            let available = chunk.len().min(READ_BUDGET - bytes_read);
            let read = file.read(&mut chunk[..available])?;
            if read == 0 {
                break;
            }
            bytes_read += read;
            for part in chunk[..read].split_inclusive(|b| *b == b'\n') {
                let ended = part.last() == Some(&b'\n');
                if !self.discarding {
                    if self.pending.len() + part.len() > LINE_BUDGET {
                        self.pending = Vec::new();
                        self.discarding = true;
                        self.gap = true;
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
                    if !self.discarding {
                        self.consume_line(&mut body);
                    }
                    self.discarding = false;
                }
            }
        }
        self.offset = offset + bytes_read as u64;
        self.bytes_read_total = self.bytes_read_total.saturating_add(bytes_read as u64);
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
        self.created = created;
        self.signature = Some(signature);
        Ok(TailBatch {
            reset,
            body,
            caught_up: self.offset == signature.1 && self.pending.is_empty() && !self.discarding,
            gap: self.gap,
            #[cfg(test)]
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
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
}
