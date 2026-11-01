# valrust

A small log-structured key-value store in Rust. Keys and values live in an ordered in-memory index (a `BTreeMap`). Every write is appended to an on-disk log and synced before it returns, so the store survives restarts and crashes. The log is the only copy of the data on disk, and `compact()` rewrites it to its smallest form.

A learning project, not for production use. The design is close to [Bitcask](https://riak.com/assets/bitcask-intro.pdf), except that values are kept in memory too.

## Contents

- [Usage](#usage)
  - [Library](#library)
  - [CLI](#cli)
- [Design](#design)
  - [Write path](#write-path)
  - [Recovery](#recovery)
  - [Compaction](#compaction)
  - [Log format](#log-format)
  - [Layers](#layers)
- [Measurements](#measurements)
- [Guarantees and limits](#guarantees-and-limits)
- [Tests](#tests)
- [Background](#background)

## Usage

### Library

```rust
use valrust::kv_store::KvStore;

let mut db = KvStore::open("data.log")?;
db.set(b"name", b"valrust")?;
assert_eq!(db.get(b"name"), Some(b"valrust".as_slice()));
db.delete(b"name")?;

// entries with start <= key < end, in sorted order
for (k, v) in db.scan_range(b"a", b"d") {
    // ...
}

// shrink the log to one entry per live key
db.compact()?;
```

Keys and values are arbitrary bytes. `open` creates the log if it does not exist.

Keys are ordered bytewise (lexicographically). `scan_range(start, end)` returns the entries with `start <= key < end`, sorted by key. It returns an empty list if `start >= end`.

### CLI

```
cargo run -- set <key> <value>
cargo run -- get <key>
cargo run -- delete <key>
cargo run -- compact         # rewrite the log as one Set per live key
cargo run -- dump            # print every log entry, repairs nothing
cargo run -- chop <n>        # cut n bytes off the log (simulate a crash)
cargo run -- flip <pos>      # flip one byte (simulate damage)
```

```
$ cargo run -- set name valrust
$ cargo run -- set lang rust
$ cargo run -- delete lang
$ cargo run -- dump
0: Set(name => valrust)
1: Set(lang => rust)
2: Delete(lang)
entries: 3, valid bytes: 58, file bytes: 58, tail: Clean
$ cargo run -- compact
log: 58 -> 24 bytes
```

## Design

The log on disk is the database: it is the only durable copy of the data. The ordered map in memory is the working copy, and it can always be rebuilt by replaying the log, never the other way round.

The log format does not depend on the in-memory structure. The map started as a `HashMap` and became a `BTreeMap` without any change to the log.

### Write path

`set` and `delete` do two things, in a fixed order:

1. Append the operation to the log and call `sync_all`.
2. Apply the operation to the map.

If step 1 fails, the function returns early and the map is untouched, so memory never holds a change the log doesn't. If the process dies between the two steps, the change is in the log and is applied on the next `open`. Reads (`get`, `scan_range`) only touch the map.

Both normal writes and recovery go through the same `apply` function, so the two paths can't behave differently.

### Recovery

`open` first deletes a leftover `<log>.tmp` file, if there is one (see [Compaction](#compaction)). It then reads the log from the start with `scan`, which stops at one of three outcomes:

| Outcome                                                         | Meaning              | What `open` does                                                       |
| --------------------------------------------------------------- | -------------------- | ---------------------------------------------------------------------- |
| End of file at an entry boundary                                | clean log            | replays all entries                                                    |
| End of file in the middle of an entry                           | crash during a write | truncates the file to the end of the last complete entry, then replays |
| A complete entry fails its checksum or has an impossible length | damage               | returns `CorruptedData`, changes nothing                               |

Truncating a cut-off entry is what makes later writes safe. Without it, new entries would land after the broken bytes and never be read back.

An entry that was cut off was never acknowledged, because `set` only returns `Ok` after the sync. Dropping it breaks no promise.

### Compaction

The log records every operation ever made, so it grows with the history, not with the data. `compact()` rewrites it as one `Set` per live key. Overwritten values and deleted keys disappear, and replaying the new log gives the identical map.

It never modifies the real log in place:

1. Write every live key to `<log>.tmp` and `sync_all` it.
2. `rename` the temp file over the log. A rename within one directory is atomic.
3. `sync` the parent directory so the rename itself survives power loss.

| Crash happens                   | Real log       | Temp file        | What `open` sees         |
| ------------------------------- | -------------- | ---------------- | ------------------------ |
| during step 1                   | old, untouched | partial          | old log, temp deleted    |
| after step 1, before the rename | old, untouched | complete, unused | old log, temp deleted    |
| around the rename               | old or new     | gone or leftover | either log, both correct |
| after step 3                    | new            | gone             | new log                  |

The old and new logs replay to the same map, so landing on either one is correct. The compacted log holds no `Delete` entries: it contains one entry per live key, so there is nothing older for a delete to cancel.

Compaction is manual and blocks the caller while it runs.

### Log format

Big-endian. The CRC covers `len` and `payload`, so a damaged length is detected too.

```
entry   = [crc: u32][len: u32][payload: len bytes]
payload = Set    [1][key_len: u32][key][value]
        | Delete [2][key]
```

`len` is capped at 16 MB. The cap is checked before any allocation, so a corrupted length can't trigger a huge allocation, and it is also enforced on write, so the log can never contain an entry it refuses to read.

### Layers

- `file.rs`: entries only. It frames bytes, checksums them, appends them, scans them back, writes a whole new log (`write_new_log`), and syncs a directory (`sync_parent_dir`). It knows nothing about what the payload means.
- `kv_store.rs`: defines the operations (`Op`), encodes them into payloads, owns the ordered map, and implements `open`, `set`, `delete`, `get`, `scan_range` and `compact`.

## Measurements

One key overwritten 1,000,000 times, release build:

```
cargo run --release --example measure 1000000
```

|                | log size     | `open()` |
| -------------- | ------------ | -------- |
| before compact | 25,888,890 B | 98.8 ms  |
| after compact  | 26 B         | 9.8 µs   |

`compact()` itself took 2.1 ms.

`open` time grows with the number of writes ever made, not with the amount of live data, and compaction removes that history. This is the best case for compaction: a log where every key is written once shrinks very little. The numbers are from a single warm run (the file was just written, so it sits in the OS page cache), so expect a few milliseconds of variation.

## Guarantees and limits

- Once `set` or `delete` returns `Ok`, the entry has been synced to disk.
- If the log ends in a partial entry (a crash during a write), `open` truncates it. Everything before it is kept.
- If a complete entry fails its checksum, `open` returns `CorruptedData`. It does not skip or repair.
- `compact` is crash-safe: at every point either the old log or the new log is complete and valid.
- Entries larger than 16 MB are rejected on write.
- Keys are sorted bytewise, and `scan_range` uses the half-open interval `[start, end)`.
- All keys and values are held in memory, so the data set must fit in RAM.
- Startup replays the whole log, so `open` takes time proportional to the log size. Compaction keeps that bounded by the live data.
- Compaction is manual, and blocks while it runs.
- One process per log file. There is no locking.
- Every write opens the file and syncs it, which is safe but slow.
- Directory sync after compaction works on Linux and macOS, not Windows.
- Crash behavior is tested by truncating and corrupting the file, not by power-loss testing.

## Tests

```
cargo test
```

The tests cover log framing, recovery from torn and corrupted logs, store behavior across reopen, range scans, compaction, and recovery from an interrupted compaction.

## Background

The design is a simplified [Bitcask](https://riak.com/assets/bitcask-intro.pdf): an append-only log that is also the data file, plus an in-memory index. Bitcask keeps only file offsets in memory and reads values from disk, so it can hold more data than RAM. This project keeps the values in memory too, which is simpler.

The git tag `bst-unbalanced` holds an experiment with a hand-written binary search tree as the index, checked against `BTreeMap` with a randomized test. It was not merged, because `BTreeMap` is balanced and faster.
