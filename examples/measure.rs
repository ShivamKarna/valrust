use std::fs;
use std::time::Instant;

use walrus::file::LogEntry;
use walrus::kv_store::{KvStore, Op};

fn main() {
    let n: u32 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(100_000);

    let path = "measure.log";
    let _ = fs::remove_file(path);

    // Build the whole log in memory and write it once.
    // Calling db.set() n times would fsync n times, which takes minutes.
    let mut bytes = Vec::new();
    for i in 0..n {
        let op = Op::Set {
            key: b"counter".to_vec(),
            val: i.to_string().into_bytes(),
        };
        bytes.extend_from_slice(&LogEntry::new(op.encode()).serialize());
    }
    fs::write(path, &bytes).unwrap();

    let size = fs::metadata(path).unwrap().len();

    let t = Instant::now();
    let db = KvStore::open(path).unwrap();
    let open_time = t.elapsed();

    println!("writes to the same key : {n}");
    println!("live keys              : 1");
    println!("log size               : {size} bytes");
    println!("open() time            : {open_time:?}");
    println!(
        "final value            : {}",
        String::from_utf8_lossy(db.get(b"counter").unwrap())
    );

    fs::remove_file(path).unwrap();
}
