// Following a growing log against reopening it, through the core API.
// Times are seconds unless named otherwise. Inputs go to a temporary folder:
//   follow gen <path> <MiB>
//   follow grow <path> <append MiB> follow|reopen [poll ms]
//   follow idle <path> <seconds> on|off
//   follow poll <path> <calls>
use logstudio_core::{
    dataset::{Dataset, Query},
    search::{Filter, Filters},
    source::Source,
};
use serde_json::{json, Value};
use std::{
    io::Write,
    path::Path,
    sync::{atomic::Ordering, Arc},
    thread::sleep,
    time::{Duration, Instant},
};

/// Android 2 : iOS 1 rows with Korean and emoji; a rare marker every 1,024
/// rows. Returns the bytes and the next row number.
fn rows(mut i: u64, bytes: usize) -> (Vec<u8>, u64) {
    let tags = [
        "ActivityManager",
        "SDK_UI",
        "GalleryManager",
        "BluetoothGatt",
        "WifiService",
    ];
    let lv = ["V", "D", "I", "W", "E"];
    let mut out = Vec::with_capacity(bytes + 512);
    while out.len() < bytes {
        let line = if i.is_multiple_of(3) {
            format!("2026-09-30 21:55:53.{:06} Runner[{}:{}] [INFO] [{}] [Comp{}] message {} 한글 텍스트 payload=abcdef{}", i % 1_000_000, 100 + i % 50, 9000 + i % 200, tags[(i % 5) as usize], i % 7, i, i)
        } else if i % 1024 == 7 {
            format!(
                "09-21 12:07:30.386 {} {} E Tag: rare_needle_{i} found",
                100 + i % 50,
                2000 + i % 300
            )
        } else {
            format!("09-21 12:07:{:02}.{:03} {} {} {} {}: some log message number {} with data value={} 🙂", i % 60, i % 1000, 100 + i % 50, 2000 + i % 300, lv[(i % 5) as usize], tags[(i % 5) as usize], i, i * 7)
        };
        out.extend_from_slice(line.as_bytes());
        out.push(b'\n');
        i += 1;
    }
    (out, i)
}
fn wire(v: Value) -> Filter {
    Filter::new(serde_json::from_value::<Filters>(v).unwrap()).unwrap()
}
fn wait(mut f: impl FnMut() -> bool) {
    while !f() {
        sleep(Duration::from_micros(200));
    }
}
fn open(path: &Path, poll: u64) -> Arc<Dataset> {
    let d = Dataset::new(1, Source::granted(path).unwrap());
    d.follow_poll_ms.store(poll, Ordering::Relaxed);
    d.start(|_| {});
    d
}
/// The upper pane filters levels W,E; the lower pane searches a rare literal.
fn queries(d: &Arc<Dataset>) -> [Arc<Query>; 2] {
    [
        d.query(0, 1, wire(json!({"levels":["W","E"]}))).unwrap(),
        d.query(1, 1, wire(json!({"query":"rare_needle"}))).unwrap(),
    ]
}
fn caught_up(d: &Dataset, q: &Query) -> bool {
    let blocks = d.index.lock().unwrap().blocks.len();
    q.json()["caughtUp"] == true && q.result.lock().unwrap().ends.len() == blocks
}
fn done(q: &Query) -> bool {
    q.json()["status"] == "complete"
}
fn analyzed(d: &Dataset) -> bool {
    let i = d.index.lock().unwrap();
    i.analyzed == i.blocks.len()
}
fn append(path: &Path, data: &[u8]) {
    let mut f = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    f.write_all(data).unwrap();
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let path = Path::new(&a[2]);
    match a[1].as_str() {
        "gen" => {
            let mib: usize = a[3].parse().unwrap();
            let mut f = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
            let (mut written, mut i) = (0usize, 0u64);
            while written < mib << 20 {
                let (chunk, next) = rows(i, (64 << 20).min((mib << 20) - written));
                f.write_all(&chunk).unwrap();
                written += chunk.len();
                i = next;
            }
            f.flush().unwrap();
            println!("gen bytes={written} rows={i}");
        }
        "grow" => {
            let mib: usize = a[3].parse().unwrap();
            let poll: u64 = a.get(5).map_or(1, |p| p.parse().unwrap());
            let base = std::fs::metadata(path).unwrap().len();
            // Appended in 1 MiB writes, so the harness holds little of it.
            let chunks: Vec<(u64, usize)> = (0..mib as u64)
                .map(|k| (1_000_000_000 + k * 10_000, 1 << 20))
                .collect();
            let data = |k: usize| rows(chunks[k].0, chunks[k].1).0;
            let size: u64 = (0..chunks.len()).map(|k| data(k).len() as u64).sum();
            if a[4] == "follow" {
                let d = open(path, poll);
                let q = queries(&d);
                wait(|| {
                    d.index.lock().unwrap().done
                        && done(&q[0])
                        && done(&q[1])
                        && d.status(0)["summaryComplete"] == true
                });
                let before = d.index.lock().unwrap().total;
                d.follow(true).unwrap();
                wait(|| caught_up(&d, &q[0]) && caught_up(&d, &q[1]));
                let t = Instant::now();
                (0..chunks.len()).for_each(|k| append(path, &data(k)));
                let write = t.elapsed().as_secs_f64();
                let len = base + size;
                let (mut first, mut index, mut scans) = (None, None, None);
                while !(scans.is_some() && analyzed(&d)) {
                    let i = d.index.lock().unwrap();
                    let (total, bytes) = (i.total, i.bytes);
                    drop(i);
                    if first.is_none() && total > before {
                        first = Some(t.elapsed().as_secs_f64());
                    }
                    if index.is_none() && bytes == len {
                        index = Some(t.elapsed().as_secs_f64());
                    }
                    if scans.is_none()
                        && index.is_some()
                        && caught_up(&d, &q[0])
                        && caught_up(&d, &q[1])
                    {
                        scans = Some(t.elapsed().as_secs_f64());
                    }
                    sleep(Duration::from_micros(200));
                }
                let levels = t.elapsed().as_secs_f64();
                let counts = (
                    q[0].json()["count"].clone(),
                    q[1].json()["count"].clone(),
                    d.index.lock().unwrap().total,
                );
                let off = Instant::now();
                d.follow(false).unwrap();
                wait(|| done(&q[0]) && done(&q[1]) && d.status(0)["summaryComplete"] == true);
                let finish = off.elapsed().as_secs_f64();
                println!("follow append_mib={mib} poll_ms={poll} write={write:.4} first_row={:.4} index={:.4} queries={:.4} levels={levels:.4} off_final={finish:.4} counts={counts:?}", first.unwrap(), index.unwrap(), scans.unwrap());
                d.close();
            } else {
                (0..chunks.len()).for_each(|k| append(path, &data(k)));
                let t = Instant::now();
                let d = open(path, poll);
                let q = queries(&d);
                wait(|| d.index.lock().unwrap().done);
                let index = t.elapsed().as_secs_f64();
                wait(|| done(&q[0]) && done(&q[1]));
                let scans = t.elapsed().as_secs_f64();
                wait(|| d.status(0)["summaryComplete"] == true);
                let levels = t.elapsed().as_secs_f64();
                let counts = (
                    q[0].json()["count"].clone(),
                    q[1].json()["count"].clone(),
                    d.index.lock().unwrap().total,
                );
                println!("reopen append_mib={mib} index={index:.4} queries={scans:.4} levels={levels:.4} counts={counts:?}");
                d.close();
            }
            std::fs::OpenOptions::new()
                .write(true)
                .open(path)
                .unwrap()
                .set_len(base)
                .unwrap();
        }
        "idle" => {
            let secs: u64 = a[3].parse().unwrap();
            let d = open(path, 500);
            let q = queries(&d);
            wait(|| done(&q[0]) && done(&q[1]) && d.status(0)["summaryComplete"] == true);
            if a[4] == "on" {
                d.follow(true).unwrap();
            }
            println!("idle {} for {secs}s", a[4]);
            sleep(Duration::from_secs(secs));
            d.close();
        }
        "poll" => {
            let calls: u32 = a[3].parse().unwrap();
            let d = open(path, 3_600_000);
            wait(|| d.index.lock().unwrap().done);
            d.follow(true).unwrap();
            let t = Instant::now();
            for _ in 0..calls {
                assert!(d.source.grow().unwrap().is_none());
            }
            println!(
                "poll calls={calls} per_call_us={:.2}",
                t.elapsed().as_secs_f64() * 1e6 / calls as f64
            );
            d.close();
        }
        _ => panic!("unknown mode"),
    }
}
