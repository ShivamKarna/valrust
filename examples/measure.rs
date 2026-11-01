use std::fs;
use std::time::Instant;

use valrust::file::LogEntry;
use valrust::kv_store::{KvStore, Op};

fn main() {
    let n: u32 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(100_000);

    let path = "measure.log";
    let _ = fs::remove_file(path);

    let mut bytes = Vec::new();
    for i in 0..n {
        let op = Op::Set {
            key: b"counter".to_vec(),
            val: i.to_string().into_bytes(),
        };
        bytes.extend_from_slice(&LogEntry::new(op.encode()).serialize());
    }
    fs::write(path, &bytes).unwrap();

    let size_before = fs::metadata(path).unwrap().len();
    let t = Instant::now();
    let mut db = KvStore::open(path).unwrap();
    let open_before = t.elapsed();

    let t = Instant::now();
    db.compact().unwrap();
    let compact_time = t.elapsed();
    drop(db);

    let size_after = fs::metadata(path).unwrap().len();
    let t = Instant::now();
    let db = KvStore::open(path).unwrap();
    let open_after = t.elapsed();

    println!("writes to the same key : {n}");
    println!("before: {size_before} bytes, open() {open_before:?}");
    println!("compact() took         : {compact_time:?}");
    println!("after : {size_after} bytes, open() {open_after:?}");
    println!(
        "final value            : {}",
        String::from_utf8_lossy(db.get(b"counter").unwrap())
    );

    fs::remove_file(path).unwrap();
}
