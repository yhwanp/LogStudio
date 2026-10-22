// Times the engine (index, levels, search, field filters) and the raw block
// read path on one synthetic log, so a Windows run can be compared with macOS.
// The hypothesis under test (docs/LARGE_LOG_VALIDATION.md, "Windows 가설"): the
// per-block path `symlink_metadata` and handle metadata that `Source::read`
// does inside its mutex cost more on Windows, and one synchronous handle
// serializes reads even without the mutex.
//
//   read-lock <MiB> <dir> [runs]       generate <dir>/bench-<MiB>.log once, then measure
//   read-lock --file <path> [runs]     measure an existing UTF-8 log
use logstudio_core::source::{Source, BLOCK};
use serde_json::{json, Value};
use std::{
    fs::File,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    thread::{self, sleep},
    time::{Duration, Instant},
};
type E = Arc<logstudio_core::Engine>;
const THREADS: [usize; 4] = [1, 2, 4, 8];

/// Android threadtime 2 : iOS 1 with Korean and emoji; a rare marker every
/// 1,024 rows. LF line ends.
fn gen(path: &Path, mib: usize) -> io::Result<()> {
    let partial = path.with_extension("log.partial");
    let mut f = io::BufWriter::with_capacity(1 << 20, File::create(&partial)?);
    let tags = [
        "ActivityManager",
        "SDK_UI",
        "GalleryManager",
        "BluetoothGatt",
        "WifiService",
    ];
    let lv = ["V", "D", "I", "W", "E"];
    let (mut n, mut i) = (0usize, 0u64);
    while n < mib << 20 {
        let line = if i % 3 == 0 {
            format!("2026-09-30 21:55:53.{:06} Runner[{}:{}] [INFO] [{}] [Comp{}] message {} 한글 텍스트 payload=abcdef{}\n", i % 1000000, 100 + i % 50, 9000 + i % 200, tags[(i % 5) as usize], i % 7, i, i)
        } else if i % 1024 == 7 {
            format!(
                "09-21 12:07:30.386 {} {} E Tag: RarE_NEEDLE_{i} found\n",
                100 + i % 50,
                2000 + i % 300
            )
        } else {
            format!("09-21 12:07:{:02}.{:03} {} {} {} {}: some log message number {} with data value={} 🙂\n", i % 60, i % 1000, 100 + i % 50, 2000 + i % 300, lv[(i % 5) as usize], tags[(i % 5) as usize], i, i * 7)
        };
        f.write_all(line.as_bytes())?;
        n += line.len();
        i += 1;
    }
    f.flush()?;
    drop(f);
    std::fs::rename(partial, path)
}

fn st(e: &E, pane: &str) -> Value {
    e.request(json!({"action":"status","pane":pane})).unwrap()
}
fn req(e: &E, v: Value) -> Value {
    e.request(v).unwrap()
}
fn wait(mut f: impl FnMut() -> bool) {
    while !f() {
        sleep(Duration::from_micros(500));
    }
}
fn run(e: &E, id: u64, qid: u64, pane: &str, f: &Value) -> (f64, u64) {
    let t = Instant::now();
    req(
        e,
        json!({"action":"query","pane":pane,"datasetId":id,"queryId":qid,"filters":f}),
    );
    wait(|| st(e, pane)["query"]["status"] != "running");
    let s = st(e, pane);
    assert_eq!(s["query"]["status"], "complete", "{s}");
    (
        t.elapsed().as_secs_f64(),
        s["query"]["count"].as_u64().unwrap(),
    )
}
/// One pass through the engine API, as the app uses it. Seconds and counts.
fn engine(path: &Path, round: usize) {
    let e: E = Arc::new(logstudio_core::Engine::default());
    let id = e.begin_open();
    let t = Instant::now();
    e.open_file(id, path).unwrap();
    req(
        &e,
        json!({"action":"query","pane":"logs","datasetId":id,"queryId":1,"filters":{}}),
    );
    let mut first = None;
    while st(&e, "logs")["textComplete"] != true {
        let page = req(
            &e,
            json!({"action":"page","pane":"logs","datasetId":id,"queryId":1,"start":0,"count":200}),
        );
        if first.is_none() && page["rows"].as_array().is_some_and(|r| !r.is_empty()) {
            first = Some(t.elapsed().as_secs_f64());
        }
        sleep(Duration::from_micros(500));
    }
    let eof = t.elapsed().as_secs_f64();
    let rows = st(&e, "logs")["total"].as_u64().unwrap();
    println!("engine {round}\tfirst page\t{:.3}", first.unwrap_or(eof));
    println!("engine {round}\tindex EOF\t{eof:.3}\trows {rows}");
    let (s, n) = run(&e, id, 2, "logs", &json!({"levels":["E"]}));
    println!("engine {round}\tlevel E before analysis\t{s:.3}\t{n}");
    let t = Instant::now();
    wait(|| st(&e, "logs")["summaryComplete"] == true);
    println!(
        "engine {round}\tanalysis remaining\t{:.3}",
        t.elapsed().as_secs_f64()
    );
    let queries = [
        ("rare literal", "search", json!({"query":"rare_needle_"})),
        ("literal 한글", "search", json!({"query":"한글"})),
        (
            "regex",
            "search",
            json!({"query":"value=\\d{5}7 ","searchMode":"regex"}),
        ),
        ("level E after analysis", "logs", json!({"levels":["E"]})),
        ("pid 123", "logs", json!({"pid":"123"})),
        ("tag gallery", "logs", json!({"tag":"gallery"})),
    ];
    for (k, (label, pane, f)) in queries.iter().enumerate() {
        let (s, n) = run(&e, id, 10 + k as u64, pane, f);
        println!("engine {round}\t{label}\t{s:.3}\t{n}");
    }
    let t = Instant::now();
    e.close();
    println!("engine {round}\tclose\t{:.4}", t.elapsed().as_secs_f64());
}

/// Positional read on a shared handle, the way `Source::read` reads.
#[cfg(unix)]
fn read_at(f: &File, buf: &mut [u8], at: u64) -> io::Result<usize> {
    std::os::unix::fs::FileExt::read_at(f, buf, at)
}
#[cfg(windows)]
fn read_at(f: &File, buf: &mut [u8], at: u64) -> io::Result<usize> {
    std::os::windows::fs::FileExt::seek_read(f, buf, at)
}
fn read_full(f: &File, buf: &mut [u8], mut at: u64) -> io::Result<()> {
    let mut done = 0;
    while done < buf.len() {
        let n = read_at(f, &mut buf[done..], at)?;
        if n == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        done += n;
        at += n as u64;
    }
    Ok(())
}
/// Reads every 128 KiB block once with `threads` workers taking blocks in
/// turn; returns seconds.
fn pass(size: u64, threads: usize, read: impl Fn(usize, u64, usize) + Sync) -> f64 {
    let blocks = size.div_ceil(BLOCK as u64) as usize;
    let next = AtomicUsize::new(0);
    let t = Instant::now();
    thread::scope(|s| {
        for worker in 0..threads {
            let (next, read) = (&next, &read);
            s.spawn(move || loop {
                let k = next.fetch_add(1, Ordering::Relaxed);
                if k >= blocks {
                    break;
                }
                let at = k as u64 * BLOCK as u64;
                read(worker, at, (size - at).min(BLOCK as u64) as usize);
            });
        }
    });
    t.elapsed().as_secs_f64()
}
fn report(label: &str, threads: usize, size: u64, secs: f64) {
    let blocks = size.div_ceil(BLOCK as u64) as f64;
    println!(
        "{label}\t{threads} threads\t{:.2} GB/s\t{:.1} µs/block",
        size as f64 / secs / 1e9,
        secs * 1e6 / blocks
    );
}
fn per_call(n: usize, mut f: impl FnMut()) -> f64 {
    let t = Instant::now();
    for _ in 0..n {
        f();
    }
    t.elapsed().as_secs_f64() * 1e6 / n as f64
}
fn reads(path: &Path, round: usize) {
    let size = std::fs::metadata(path).unwrap().len();
    // The work `Source::read` adds to each block besides the read itself.
    let file = File::open(path).unwrap();
    let path_meta = per_call(20_000, || {
        std::fs::symlink_metadata(path).unwrap();
    });
    let handle_meta = per_call(20_000, || {
        file.metadata().unwrap();
    });
    println!("read {round}\tpath symlink_metadata\t{path_meta:.2} µs/call");
    println!("read {round}\thandle metadata\t{handle_meta:.2} µs/call");
    println!(
        "read {round}\tchecks per block (1 path + 2 handle)\t{:.2} µs",
        path_meta + 2.0 * handle_meta
    );
    drop(file);
    // The engine path: one mutex around checks and the read of each block.
    let source = Source::granted(path).unwrap();
    pass(size, 4, |_, at, len| drop(source.read(at, len).unwrap()));
    for threads in THREADS {
        let secs = pass(size, threads, |_, at, len| {
            drop(source.read(at, len).unwrap())
        });
        report(
            &format!("read {round}\tSource::read (mutex + checks)"),
            threads,
            size,
            secs,
        );
    }
    drop(source);
    // Without the mutex and checks: one shared handle, then one per worker.
    let shared = File::open(path).unwrap();
    for threads in THREADS {
        let secs = pass(size, threads, |_, at, len| {
            let mut buf = vec![0; len];
            read_full(&shared, &mut buf, at).unwrap();
        });
        report(
            &format!("read {round}\tshared handle, no lock"),
            threads,
            size,
            secs,
        );
    }
    for threads in THREADS {
        let own: Vec<File> = (0..threads).map(|_| File::open(path).unwrap()).collect();
        let secs = pass(size, threads, |worker, at, len| {
            let mut buf = vec![0; len];
            read_full(&own[worker], &mut buf, at).unwrap();
        });
        report(
            &format!("read {round}\thandle per worker, no lock"),
            threads,
            size,
            secs,
        );
    }
}
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (path, rest) = match args.first().map(String::as_str) {
        Some("--file") => (
            PathBuf::from(args.get(1).expect("--file <path>")),
            &args[2.min(args.len())..],
        ),
        Some(mib) => {
            let mib: usize = mib
                .parse()
                .expect("<MiB> <dir> [runs] or --file <path> [runs]");
            let dir = Path::new(args.get(1).expect("<MiB> <dir> [runs]"));
            std::fs::create_dir_all(dir).unwrap();
            let path = dir.join(format!("bench-{mib}.log"));
            if !path.exists() {
                let t = Instant::now();
                gen(&path, mib).unwrap();
                println!(
                    "generated\t{}\t{:.1}s",
                    path.display(),
                    t.elapsed().as_secs_f64()
                );
            }
            (path, &args[2.min(args.len())..])
        }
        None => panic!("<MiB> <dir> [runs] or --file <path> [runs]"),
    };
    let runs: usize = rest.first().map_or(2, |r| r.parse().expect("runs"));
    let cpus = thread::available_parallelism().map_or(0, |n| n.get());
    let size = std::fs::metadata(&path).unwrap().len();
    println!(
        "environment\t{} {}\t{cpus} logical CPUs\tfile {size} bytes\tblock {BLOCK} bytes",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    // The first read pass also warms the OS file cache for both sections.
    for round in 1..=runs {
        reads(&path, round);
        engine(&path, round);
    }
}
