//! Bounded, read-only projection of one Chromium IndexedDB record. Never opens
//! LevelDB for writing or takes its LOCK; active MANIFEST files and sequence
//! numbers select the current checkpoint, including tombstones and blob values.
use serde_json::{Map, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::SystemTime,
};
#[cfg(test)]
#[path = "storage_tests.rs"]
mod tests;

pub type Result<T> = std::result::Result<T, &'static str>;
const LIMIT: usize = 32 * 1024 * 1024;
const RECORD: &str = "default-chat-im-protocol-store";
pub const DATABASE: &str = "chrome_doubao-chat_0.indexeddb.leveldb";

struct Input<'a> {
    b: &'a [u8],
    p: usize,
}
impl<'a> Input<'a> {
    fn new(b: &'a [u8]) -> Self {
        Self { b, p: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.p.checked_add(n).ok_or("bounds")?;
        let v = self.b.get(self.p..end).ok_or("incomplete")?;
        self.p = end;
        Ok(v)
    }
    fn byte(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn var(&mut self) -> Result<u64> {
        let mut v = 0;
        for shift in (0..64).step_by(7) {
            let b = self.byte()?;
            if shift == 63 && b > 1 {
                return Err("varint");
            }
            v |= u64::from(b & 127) << shift;
            if b < 128 {
                return Ok(v);
            }
        }
        Err("varint")
    }
    fn size(&mut self) -> Result<usize> {
        let n = self.var()?;
        if n > LIMIT as u64 {
            return Err("budget");
        }
        Ok(n as usize)
    }
    fn slice(&mut self) -> Result<&'a [u8]> {
        let n = self.size()?;
        self.take(n)
    }
    fn fixed(&mut self, n: usize) -> Result<u64> {
        let mut b = [0; 8];
        b[..n].copy_from_slice(self.take(n)?);
        Ok(u64::from_le_bytes(b))
    }
}

fn crc(bytes: impl Iterator<Item = u8>) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        std::array::from_fn(|i| {
            let mut n = i as u32;
            for _ in 0..8 {
                n = (n >> 1) ^ (0x82f63b78 & (0u32.wrapping_sub(n & 1)));
            }
            n
        })
    });
    let mut crc = !0u32;
    for b in bytes {
        crc = (crc >> 8) ^ table[((crc ^ u32::from(b)) & 255) as usize];
    }
    (!crc).rotate_right(15).wrapping_add(0xa282ead8)
}

fn snappy(b: &[u8]) -> Result<Vec<u8>> {
    let mut r = Input::new(b);
    let len = r.size()?;
    let mut out = Vec::new();
    while r.p < b.len() {
        let tag = r.byte()?;
        if tag & 3 == 0 {
            let mut n = (tag >> 2) as usize;
            if n >= 60 {
                n = r.fixed(n - 59)? as usize;
            }
            n = n.checked_add(1).ok_or("budget")?;
            if n > len.saturating_sub(out.len()) {
                return Err("snappy size");
            }
            out.extend_from_slice(r.take(n)?);
        } else {
            let (n, offset) = match tag & 3 {
                1 => (
                    4 + ((tag >> 2) & 7) as usize,
                    ((tag as usize & 224) << 3) | r.byte()? as usize,
                ),
                2 => (1 + (tag >> 2) as usize, r.fixed(2)? as usize),
                _ => (1 + (tag >> 2) as usize, r.fixed(4)? as usize),
            };
            if offset == 0 || offset > out.len() || n > len.saturating_sub(out.len()) {
                return Err("snappy copy");
            }
            for _ in 0..n {
                out.push(out[out.len() - offset]);
            }
        }
    }
    if out.len() != len {
        return Err("snappy size");
    }
    Ok(out)
}

fn log_records(b: &[u8], mut visit: impl FnMut(&[u8]) -> Result<()>) -> Result<()> {
    let mut partial: Option<Vec<u8>> = None;
    for block in b.chunks(32768) {
        let mut r = Input::new(block);
        while block.len() - r.p >= 7 {
            let checksum = r.fixed(4)? as u32;
            let len = r.fixed(2)? as usize;
            let kind = r.byte()?;
            if kind == 0 && len == 0 {
                break;
            }
            let data = r.take(len)?;
            if crc(std::iter::once(kind).chain(data.iter().copied())) != checksum {
                return Err("log checksum");
            }
            match kind {
                1 if partial.is_none() => visit(data)?,
                2 if partial.is_none() => partial = Some(data.to_vec()),
                3 | 4 => {
                    let p = partial.as_mut().ok_or("log fragments")?;
                    if p.len() + len > LIMIT {
                        return Err("budget");
                    }
                    p.extend_from_slice(data);
                    if kind == 4 {
                        visit(p)?;
                        partial = None;
                    }
                }
                _ => return Err("log fragments"),
            }
        }
        if block[r.p..].iter().any(|b| *b != 0) {
            return Err("log tail");
        }
    }
    if partial.is_some() {
        return Err("incomplete log");
    }
    Ok(())
}

fn entries(b: &[u8], mut visit: impl FnMut(&[u8], &[u8]) -> Result<()>) -> Result<()> {
    let count = Input::new(
        b.get(b.len().checked_sub(4).ok_or("block")?..)
            .ok_or("block")?,
    )
    .fixed(4)? as usize;
    let end = b
        .len()
        .checked_sub(
            count
                .checked_add(1)
                .and_then(|n| n.checked_mul(4))
                .ok_or("block")?,
        )
        .ok_or("block")?;
    let mut r = Input::new(&b[..end]);
    let mut key = Vec::new();
    while r.p < end {
        let shared = r.size()?;
        let fresh = r.size()?;
        let size = r.size()?;
        if shared > key.len() {
            return Err("block key");
        }
        key.truncate(shared);
        key.extend_from_slice(r.take(fresh)?);
        visit(&key, r.take(size)?)?;
    }
    Ok(())
}

fn table_block(b: &[u8], handle: &mut Input<'_>) -> Result<Vec<u8>> {
    let offset = handle.size()?;
    let size = handle.size()?;
    let mut r = Input::new(b.get(offset..).ok_or("block offset")?);
    let data = r.take(size)?;
    let kind = r.byte()?;
    let checksum = r.fixed(4)? as u32;
    if crc(data.iter().copied().chain(std::iter::once(kind))) != checksum {
        return Err("table checksum");
    }
    match kind {
        0 => Ok(data.to_vec()),
        1 => snappy(data),
        _ => Err("table compression"),
    }
}

#[derive(Clone, PartialEq)]
struct Stamp {
    path: PathBuf,
    len: u64,
    modified: SystemTime,
}
fn stamp(p: &Path) -> Result<Stamp> {
    let m = fs::symlink_metadata(p).map_err(|_| "read metadata")?;
    if !m.is_file() || m.file_type().is_symlink() || m.len() > LIMIT as u64 {
        return Err("file budget/type");
    }
    Ok(Stamp {
        path: p.to_owned(),
        len: m.len(),
        modified: m.modified().map_err(|_| "file time")?,
    })
}
#[derive(Default)]
pub struct Reader {
    stamps: Vec<Stamp>,
    cached: Option<Value>,
}
impl Reader {
    pub fn read(&mut self, root: &Path) -> Result<Value> {
        if let Some(cached) = self.cached.as_ref().filter(|_| {
            !self.stamps.is_empty()
                && self
                    .stamps
                    .iter()
                    .all(|s| stamp(&s.path).ok().as_ref() == Some(s))
        }) {
            return Ok(cached.clone());
        }
        let mut files = Files::default();
        let db = root.join(DATABASE);
        let current = files.read(&db.join("CURRENT"))?;
        let manifest = std::str::from_utf8(&current)
            .map_err(|_| "manifest name")?
            .trim();
        if !manifest
            .strip_prefix("MANIFEST-")
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|c| c.is_ascii_digit()))
        {
            return Err("manifest name");
        }
        let mut tables = BTreeSet::new();
        let mut log = 0;
        let mut prev = 0;
        log_records(&files.read(&db.join(manifest))?, |b| {
            let mut r = Input::new(b);
            while r.p < b.len() {
                match r.var()? {
                    1 => {
                        r.slice()?;
                    }
                    2 => log = r.var()?,
                    3 | 4 => {
                        r.var()?;
                    }
                    5 => {
                        r.var()?;
                        r.slice()?;
                    }
                    6 => {
                        r.var()?;
                        tables.remove(&r.var()?);
                    }
                    7 => {
                        r.var()?;
                        tables.insert(r.var()?);
                        r.var()?;
                        r.slice()?;
                        r.slice()?;
                    }
                    9 => prev = r.var()?,
                    _ => return Err("manifest version"),
                }
            }
            Ok(())
        })?;
        let mut records = BTreeMap::<Vec<u8>, (u64, Option<Vec<u8>>)>::new();
        let mut accept = |key: &[u8], seq: u64, kind: u8, value: &[u8]| -> Result<()> {
            if !checkpoint_key(key)? {
                return Ok(());
            }
            if kind > 1 {
                return Err("record type");
            }
            if records.get(key).is_none_or(|(old, _)| *old < seq) {
                records.insert(key.to_vec(), (seq, (kind == 1).then(|| value.to_vec())));
            }
            Ok(())
        };
        let mut budget = 0;
        for id in tables {
            let b = files.read(&db.join(format!("{id:06}.ldb")))?;
            let footer = b
                .get(b.len().checked_sub(48).ok_or("table footer")?..)
                .ok_or("table footer")?;
            if footer[40..] != 0xdb4775248b80fb57u64.to_le_bytes() {
                return Err("table magic");
            }
            let mut f = Input::new(footer);
            f.var()?;
            f.var()?;
            let index = table_block(&b, &mut f)?;
            entries(&index, |_, handle| {
                let block = table_block(&b, &mut Input::new(handle))?;
                budget += block.len();
                if budget > LIMIT * 2 {
                    return Err("table budget");
                }
                entries(&block, |key, value| {
                    let split = key.len().checked_sub(8).ok_or("internal key")?;
                    let packed = Input::new(&key[split..]).fixed(8)?;
                    accept(&key[..split], packed >> 8, packed as u8, value)
                })
            })?;
        }
        for id in [prev, log].into_iter().filter(|v| *v != 0) {
            log_records(&files.read(&db.join(format!("{id:06}.log")))?, |b| {
                let mut r = Input::new(b);
                let seq = r.fixed(8)?;
                let count = r.fixed(4)?;
                if count > 1_000_000 {
                    return Err("batch budget");
                }
                for i in 0..count {
                    let kind = r.byte()?;
                    let key = r.slice()?;
                    let value = if kind == 1 {
                        r.slice()?
                    } else if kind == 0 {
                        &[]
                    } else {
                        return Err("batch type");
                    };
                    accept(key, seq.checked_add(i).ok_or("sequence")?, kind, value)?;
                }
                if r.p != b.len() {
                    return Err("batch tail");
                }
                Ok(())
            })?;
        }
        let active: Vec<_> = records
            .iter()
            .filter(|(key, (_, v))| key[3] == 1 && v.is_some())
            .collect();
        if active.len() != 1 {
            return Err("checkpoint unavailable/ambiguous");
        }
        let (key, (_, Some(value))) = active[0] else {
            return Err("checkpoint unavailable");
        };
        let mut r = Input::new(value);
        r.var()?;
        let mut bytes = value[r.p..].to_vec();
        if bytes.starts_with(&[255, 17, 1]) {
            let mut wrapper = Input::new(&bytes[3..]);
            let size = wrapper.size()?;
            let index = wrapper.var()?;
            if index != 0 || wrapper.p != wrapper.b.len() {
                return Err("blob wrapper");
            }
            let mut blob_key = key.to_vec();
            blob_key[3] = 3;
            let Some((_, Some(blob))) = records.get(&blob_key) else {
                return Err("blob metadata");
            };
            let mut b = Input::new(blob);
            if b.byte()? != 0 {
                return Err("blob kind");
            }
            let number = b.var()?;
            let mime_len = b.size()?;
            b.take(mime_len.checked_mul(2).ok_or("blob mime")?)?;
            if b.var()? != size as u64 || b.p != blob.len() {
                return Err("blob size");
            }
            let path = root
                .join("chrome_doubao-chat_0.indexeddb.blob")
                .join(format!("{:x}/{:02x}/{:x}", key[1], number >> 8, number));
            bytes = files.read(&path)?;
            if bytes.len() != size {
                return Err("blob size");
            }
        }
        let value = decode(&bytes)?;
        if files
            .stamps
            .iter()
            .any(|s| stamp(&s.path).ok().as_ref() != Some(s))
        {
            return Err("concurrent write");
        }
        // Cache ONLY normalized task facts, never messages/prompts/account data.
        let value = super::project_checkpoint(&value)?;
        self.stamps = files.stamps;
        self.cached = Some(value.clone());
        Ok(value)
    }
}
#[derive(Default)]
struct Files {
    stamps: Vec<Stamp>,
    bytes: u64,
}
impl Files {
    fn read(&mut self, p: &Path) -> Result<Vec<u8>> {
        let s = stamp(p)?;
        self.bytes += s.len;
        if self.bytes > (LIMIT * 4) as u64 {
            return Err("snapshot budget");
        }
        let mut out = Vec::new();
        fs::File::open(p)
            .map_err(|_| "open")?
            .take(LIMIT as u64 + 1)
            .read_to_end(&mut out)
            .map_err(|_| "read")?;
        if out.len() as u64 != s.len {
            return Err("concurrent write");
        }
        self.stamps.push(s);
        Ok(out)
    }
}
fn checkpoint_key(key: &[u8]) -> Result<bool> {
    // This supported schema uses one-byte DB/store/index IDs. Unknown layouts
    // cannot select an unrelated record by a string match alone.
    if key.len() < 6
        || key[0] != 0
        || key[1] == 0
        || key[2] == 0
        || ![1, 3].contains(&key[3])
        || key[4] != 1
    {
        return Ok(false);
    }
    let mut r = Input::new(&key[5..]);
    let n = r.size()?;
    if n != RECORD.len() {
        return Ok(false);
    }
    let text = r.take(n * 2)?;
    Ok(r.p == r.b.len()
        && text
            .as_chunks::<2>()
            .0
            .iter()
            .zip(RECORD.bytes())
            .all(|(c, b)| *c == [0, b]))
}

fn decode(b: &[u8]) -> Result<Value> {
    let uncompressed;
    let b = if b.starts_with(&[255, 17, 2]) {
        uncompressed = snappy(&b[3..])?;
        &uncompressed
    } else {
        b
    };
    // Blink v21 envelope: trailer offset (8 BE bytes) + trailer size (4 BE).
    if b.len() < 17 || b[..3] != [255, 21, 254] || b[15..17] != [255, 15] {
        return Err("serialization version");
    }
    let mut parser = V8 {
        r: Input::new(&b[17..]),
        nodes: 0,
        string_bytes: 0,
        objects: Vec::new(),
        object_offsets: BTreeMap::new(),
    };
    let value = parser.value(0)?;
    if parser.r.p != parser.r.b.len() {
        return Err("serialization tail");
    }
    Ok(value)
}
struct V8<'a> {
    r: Input<'a>,
    nodes: usize,
    string_bytes: usize,
    // Reparse completed object spans instead of retaining a clone of every
    // ancestor. Replays reuse IDs and share the same expansion budgets.
    objects: Vec<(usize, bool)>,
    object_offsets: BTreeMap<usize, usize>,
}
impl V8<'_> {
    fn value(&mut self, depth: usize) -> Result<Value> {
        self.nodes += 1;
        if depth > 48 || self.nodes > 200_000 {
            return Err("serialization budget");
        }
        let mut t = self.r.byte()?;
        while t == 0 {
            t = self.r.byte()?;
        }
        let offset = self.r.p - 1;
        Ok(match t {
            b'"' | b'S' | b'c' => {
                let b = self.r.slice()?;
                let s = if t == b'c' {
                    if b.len() % 2 != 0 {
                        return Err("utf16");
                    }
                    String::from_utf16(
                        &b.as_chunks::<2>()
                            .0
                            .iter()
                            .map(|c| u16::from_le_bytes([c[0], c[1]]))
                            .collect::<Vec<_>>(),
                    )
                    .map_err(|_| "utf16")?
                } else if t == b'S' {
                    std::str::from_utf8(b).map_err(|_| "utf8")?.to_owned()
                } else {
                    b.iter().map(|b| char::from(*b)).collect()
                };
                self.string_bytes += s.len();
                if self.string_bytes > LIMIT * 2 {
                    return Err("serialization string budget");
                }
                Value::String(s)
            }
            b'I' => {
                let n = self.r.var()?;
                if n > u32::MAX as u64 {
                    return Err("int");
                }
                Value::from(((n >> 1) as i64) ^ -((n & 1) as i64))
            }
            b'U' => Value::from(self.r.var()?),
            b'N' => {
                let n = f64::from_bits(self.r.fixed(8)?);
                // V8 can store NaN/Infinity in optional statistics. Match the
                // JSON projection (null); required identity/time/status fields
                // still undergo their own validation, never coerce to zero.
                if !n.is_finite() {
                    Value::Null
                } else if n.fract() == 0.0 && n.abs() <= 9_007_199_254_740_991.0 {
                    Value::from(n as i64)
                } else {
                    Value::from(n)
                }
            }
            b'T' => Value::Bool(true),
            b'F' => Value::Bool(false),
            b'0' | b'_' | b'-' => Value::Null,
            b'^' => {
                let id = self.r.size()?;
                let start = self
                    .objects
                    .get(id)
                    .filter(|(_, complete)| *complete)
                    .map(|(start, _)| *start)
                    .ok_or("invalid/cyclic object reference")?;
                let resume = self.r.p;
                self.r.p = start;
                let value = self.value(depth + 1);
                self.r.p = resume;
                value?
            }
            b'o' | b'a' | b'A' => {
                let id = if let Some(id) = self.object_offsets.get(&offset) {
                    *id
                } else {
                    let id = self.objects.len();
                    self.objects.push((offset, false));
                    self.object_offsets.insert(offset, id);
                    id
                };
                let length = if t != b'o' { self.r.size()? } else { 0 };
                let end = match t {
                    b'o' => b'{',
                    b'a' => b'@',
                    _ => b'$',
                };
                let mut map = Map::new();
                if t == b'A' {
                    for i in 0..length {
                        map.insert(i.to_string(), self.value(depth + 1)?);
                    }
                }
                let mut properties = 0;
                while self.r.b.get(self.r.p).copied() != Some(end) {
                    let key = self.value(depth + 1)?;
                    let key = match key {
                        Value::String(s) => s,
                        Value::Number(n) => n.to_string(),
                        _ => return Err("object key"),
                    };
                    map.insert(key, self.value(depth + 1)?);
                    properties += 1;
                }
                self.r.byte()?;
                if self.r.var()? != properties {
                    return Err("object length");
                }
                if t != b'o' && self.r.size()? != length {
                    return Err("array length");
                }
                self.objects[id].1 = true;
                Value::Object(map)
            }
            _ => return Err("unsupported serialization tag"),
        })
    }
}
