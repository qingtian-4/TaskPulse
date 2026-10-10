//! Append-only JSONL reader. Partial UTF-8/JSON lines survive the next refresh;
//! truncation/replacement resets the caller's parser instead of merging turns.
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::SystemTime;

#[derive(Default)]
pub struct JsonlTail {
    offset: u64,
    signature: Option<(SystemTime, u64)>,
    created: Option<SystemTime>,
    anchor: Vec<u8>,
    pending: Vec<u8>,
}

pub struct TailBatch {
    pub reset: bool,
    pub body: String,
    #[cfg(test)]
    pub bytes_read: usize,
}

impl JsonlTail {
    pub fn read(&mut self, path: &Path) -> std::io::Result<TailBatch> {
        let mut file = File::open(path)?;
        let metadata = file.metadata()?;
        let signature = (metadata.modified()?, metadata.len());
        if self.signature == Some(signature) {
            return Ok(TailBatch {
                reset: false,
                body: String::new(),
                #[cfg(test)]
                bytes_read: 0,
            });
        }
        let created = metadata.created().ok();
        let mut reset = self.signature.is_none()
            || metadata.len() < self.offset
            || created != self.created
            || self.signature.is_some_and(|old| old.1 == metadata.len());
        if !reset && !self.anchor.is_empty() {
            file.seek(SeekFrom::Start(self.offset - self.anchor.len() as u64))?;
            let mut anchor = vec![0; self.anchor.len()];
            file.read_exact(&mut anchor)?;
            reset = anchor != self.anchor;
        }
        let offset = if reset { 0 } else { self.offset };
        file.seek(SeekFrom::Start(offset))?;
        let mut appended = Vec::new();
        file.read_to_end(&mut appended)?;
        let bytes_read = appended.len();
        if reset {
            self.pending.clear();
        }
        self.offset = offset + bytes_read as u64;
        self.pending.extend_from_slice(&appended);
        let mut end = self
            .pending
            .iter()
            .rposition(|b| *b == b'\n')
            .map_or(0, |i| i + 1);
        // Some versions write a complete final JSON object without a newline.
        if serde_json::from_slice::<serde_json::Value>(&self.pending[end..]).is_ok() {
            end = self.pending.len();
        }
        let body = String::from_utf8_lossy(&self.pending[..end]).into_owned();
        self.pending.drain(..end);
        file.seek(SeekFrom::Start(self.offset.saturating_sub(64)))?;
        let mut anchor = vec![0; self.offset.min(64) as usize];
        file.read_exact(&mut anchor)?;
        self.anchor = anchor;
        self.created = created;
        self.signature = Some(signature);
        Ok(TailBatch {
            reset,
            body,
            #[cfg(test)]
            bytes_read,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
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
