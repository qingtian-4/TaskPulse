use super::*;
use serde_json::json;

fn var(mut n: u64, b: &mut Vec<u8>) {
    while n >= 128 {
        b.push(n as u8 | 128);
        n >>= 7;
    }
    b.push(n as u8);
}
fn bytes(value: &[u8], b: &mut Vec<u8>) {
    var(value.len() as u64, b);
    b.extend(value);
}
fn v8(v: &Value, b: &mut Vec<u8>) {
    match v {
        Value::Null => b.push(b'0'),
        Value::Bool(v) => b.push(if *v { b'T' } else { b'F' }),
        Value::Number(v) => {
            b.push(b'N');
            b.extend(v.as_f64().unwrap().to_le_bytes());
        }
        Value::String(s) => {
            b.push(b'S');
            bytes(s.as_bytes(), b);
        }
        Value::Object(o) => {
            b.push(b'o');
            for (k, v) in o {
                v8(&json!(k), b);
                v8(v, b);
            }
            b.push(b'{');
            var(o.len() as u64, b);
        }
        _ => panic!("fixture schema"),
    }
}
fn checkpoint() -> Vec<u8> {
    let mut b = vec![255, 21, 254];
    b.extend([0; 12]);
    b.extend([255, 15]);
    v8(
        &json!({"version":2,"state":{"mainTaskDataMap":{"turn":{"sessionId":"turn","createTime":1793074433000u64,"receivedMessages":{"msg":{"conversationId":"123","contentStatus":2,"extra":{"stage":2,"chunk_seq":1,"index":1}}}}}}}),
        &mut b,
    );
    b
}
fn key(index: u8) -> Vec<u8> {
    let mut k = vec![0, 2, 1, index, 1, 30];
    for c in b"default-chat-im-protocol-store" {
        k.extend([0, *c]);
    }
    k
}
fn log_record(b: &[u8]) -> Vec<u8> {
    assert!(b.len() < 32761);
    let mut out = crc(std::iter::once(1).chain(b.iter().copied()))
        .to_le_bytes()
        .to_vec();
    out.extend((b.len() as u16).to_le_bytes());
    out.push(1);
    out.extend(b);
    out
}
fn batch(sequence: u64, rows: &[(Vec<u8>, Option<Vec<u8>>)]) -> Vec<u8> {
    let mut b = sequence.to_le_bytes().to_vec();
    b.extend((rows.len() as u32).to_le_bytes());
    for (k, v) in rows {
        b.push(u8::from(v.is_some()));
        bytes(k, &mut b);
        if let Some(v) = v {
            bytes(v, &mut b);
        }
    }
    log_record(&b)
}
fn db() -> (tempfile::TempDir, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join(DATABASE);
    fs::create_dir(&p).unwrap();
    fs::write(p.join("CURRENT"), b"MANIFEST-000001\n").unwrap();
    fs::write(p.join("MANIFEST-000001"), log_record(&[2, 3])).unwrap();
    (d, p)
}
fn inline() -> Vec<u8> {
    let mut v = vec![1];
    v.extend(checkpoint());
    v
}

fn serialized(body: &[u8]) -> Vec<u8> {
    let mut bytes = checkpoint()[..17].to_vec();
    bytes.extend(body);
    bytes
}

#[test]
fn shared_objects_replay_without_renumbering_later_references() {
    // root=0, a=1, a.nested=2, c=3; replaying a must not allocate IDs 3/4.
    let body=b"o\"\x01ao\"\x06nestedo\"\x01xI\x02{\x01{\x01\"\x01b^\x01\"\x01co\"\x01yI\x04{\x01\"\x01d^\x03{\x04";
    let value = decode(&serialized(body)).unwrap();
    assert_eq!(value["a"], value["b"]);
    assert_eq!(value["c"], value["d"]);
    assert_eq!(value["b"]["nested"]["x"], 1);
    assert_eq!(value["d"]["y"], 2);
}

#[test]
fn shared_arrays_and_reference_expansion_are_bounded() {
    let value = decode(&serialized(b"o\"\x01aA\x01I\x02$\x00\x01\"\x01b^\x01{\x02")).unwrap();
    assert_eq!(value["a"], value["b"]);
    assert_eq!(value["b"]["0"], 1);
    for body in [b"o\"\x01x^\x00{\x01".as_slice(), b"o\"\x01x^\x7f{\x01"] {
        assert!(decode(&serialized(body)).is_err());
    }
    let mut body = b"o\"\x010o{\x00".to_vec();
    for id in 1..20u8 {
        body.extend([
            b'"',
            1,
            b'A' + id,
            b'o',
            b'"',
            1,
            b'a',
            b'^',
            id,
            b'"',
            1,
            b'b',
            b'^',
            id,
            b'{',
            2,
        ]);
    }
    body.extend([b'{', 20]);
    assert!(decode(&serialized(&body)).is_err());
}

#[test]
fn optional_nonfinite_v8_numbers_do_not_discard_the_checkpoint() {
    for n in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut body = b"o\"\x01nN".to_vec();
        body.extend(n.to_le_bytes());
        body.extend(b"{\x01");
        assert_eq!(decode(&serialized(&body)).unwrap()["n"], Value::Null);
    }
    let mut invalid = decode(&checkpoint()).unwrap();
    invalid["state"]["mainTaskDataMap"]["turn"]["createTime"] = Value::Null;
    assert!(super::super::project_checkpoint(&invalid).is_err());
}

#[test]
#[ignore = "read-only historical inline execution checkpoints; counts only, no task import"]
fn live_doubao_reference_checkpoints() {
    let root = super::super::default_root().join(DATABASE);
    let mut parsed = 0;
    let mut active = 0;
    for entry in fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|s| s.to_str()) != Some("log") {
            continue;
        }
        let b = fs::read(path).unwrap();
        log_records(&b, |batch| {
            let mut r = Input::new(batch);
            r.fixed(8)?;
            let count = r.fixed(4)?;
            for _ in 0..count {
                let kind = r.byte()?;
                let key = r.slice()?;
                let value = if kind == 1 { r.slice()? } else { &[] };
                if key.get(3) != Some(&1) || !checkpoint_key(key)? || value.is_empty() {
                    continue;
                }
                let mut v = Input::new(value);
                v.var()?;
                let data = &value[v.p..];
                if !data.starts_with(&[255, 21, 254]) {
                    continue;
                }
                let decoded = decode(data)?;
                let rows = super::super::project_checkpoint(&decoded)?;
                parsed += 1;
                active += rows
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|row| row["state"] == "running")
                    .count();
            }
            Ok(())
        })
        .unwrap();
    }
    println!("real inline checkpoints parsed={parsed}; running observations={active}");
    assert!(
        parsed > 0 && active > 0,
        "Need real execution checkpoints for this opt-in regression"
    );
}

#[test]
fn checksum_truncation_and_unsupported_values_fail_closed() {
    let b = log_record(b"abc");
    assert!(log_records(&b, |v| {
        assert_eq!(v, b"abc");
        Ok(())
    })
    .is_ok());
    for n in 1..b.len() {
        assert!(log_records(&b[..n], |_| Ok(())).is_err());
    }
    let mut corrupted = b;
    corrupted[7] ^= 1;
    assert!(log_records(&corrupted, |_| Ok(())).is_err());
    assert!(decode(&[255, 17, 3]).is_err());
    assert!(snappy(&[1, 1, 0]).is_err());
    assert!(snappy(&[255; 10]).is_err());
}
#[test]
fn live_wal_precedence_tombstone_rotation_and_cache_invalidation() {
    let (d, p) = db();
    fs::write(p.join("000003.log"), batch(10, &[(key(1), Some(inline()))])).unwrap();
    let mut r = Reader::default();
    let one = r.read(d.path()).unwrap();
    assert_eq!(one[0]["conversation"], "123");
    assert_eq!(r.read(d.path()).unwrap(), one);
    fs::write(p.join("MANIFEST-000002"), log_record(&[2, 4])).unwrap();
    fs::write(p.join("000004.log"), batch(11, &[(key(1), None)])).unwrap();
    fs::write(p.join("CURRENT"), b"MANIFEST-000002\n").unwrap();
    assert!(r.read(d.path()).is_err());
    fs::write(p.join("000004.log"), batch(12, &[(key(1), Some(inline()))])).unwrap();
    assert_eq!(r.read(d.path()).unwrap(), one);
}
#[test]
fn external_compressed_blob_is_bound_to_exact_record() {
    let (d, p) = db();
    let raw = checkpoint();
    let mut compressed = vec![255, 17, 2];
    var(raw.len() as u64, &mut compressed);
    compressed.push(61 << 2);
    compressed.extend(((raw.len() - 1) as u16).to_le_bytes());
    compressed.extend(raw);
    let dir = d.path().join("chrome_doubao-chat_0.indexeddb.blob/2/0a");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("abc"), &compressed).unwrap();
    let mut meta = vec![0];
    var(0xabc, &mut meta);
    var(0, &mut meta);
    var(compressed.len() as u64, &mut meta);
    let mut wrapper = vec![1, 255, 17, 1];
    var(compressed.len() as u64, &mut wrapper);
    var(0, &mut wrapper);
    fs::write(
        p.join("000003.log"),
        batch(1, &[(key(1), Some(wrapper)), (key(3), Some(meta))]),
    )
    .unwrap();
    let mut r = Reader::default();
    assert_eq!(r.read(d.path()).unwrap()[0]["state"], "running");
    fs::write(dir.join("abc"), b"truncated").unwrap();
    assert!(r.read(d.path()).is_err());
}
#[test]
fn compacted_table_and_newer_wal_are_reconciled_by_sequence() {
    let (d, p) = db();
    fn block(rows: &[(Vec<u8>, Vec<u8>)]) -> Vec<u8> {
        let mut b = Vec::new();
        for (k, v) in rows {
            var(0, &mut b);
            var(k.len() as u64, &mut b);
            var(v.len() as u64, &mut b);
            b.extend(k);
            b.extend(v);
        }
        b.extend(0u32.to_le_bytes());
        b.extend(1u32.to_le_bytes());
        b
    }
    fn add(b: &mut Vec<u8>, raw: Vec<u8>) -> Vec<u8> {
        let mut handle = Vec::new();
        var(b.len() as u64, &mut handle);
        var(raw.len() as u64, &mut handle);
        let checksum = crc(raw.iter().copied().chain(std::iter::once(0)));
        b.extend(raw);
        b.push(0);
        b.extend(checksum.to_le_bytes());
        handle
    }
    let mut internal = key(1);
    internal.extend(((7u64 << 8) | 1).to_le_bytes());
    let mut table = Vec::new();
    let handle = add(&mut table, block(&[(internal.clone(), inline())]));
    let index = add(&mut table, block(&[(internal, handle)]));
    let mut footer = vec![0, 0];
    footer.extend(index);
    footer.resize(40, 0);
    footer.extend(0xdb4775248b80fb57u64.to_le_bytes());
    table.extend(footer);
    fs::write(p.join("000005.ldb"), table).unwrap();
    fs::write(
        p.join("MANIFEST-000001"),
        log_record(&[2, 3, 7, 0, 5, 0, 0, 0]),
    )
    .unwrap();
    fs::write(p.join("000003.log"), []).unwrap();
    let mut r = Reader::default();
    assert_eq!(r.read(d.path()).unwrap()[0]["session"], "turn");
    fs::write(p.join("000003.log"), batch(8, &[(key(1), None)])).unwrap();
    assert!(r.read(d.path()).is_err());
}
