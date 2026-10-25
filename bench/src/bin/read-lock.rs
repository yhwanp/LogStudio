// Times the engine (index, levels, search, field filters) and the raw block
// read path on one synthetic log, so a Windows run can be compared with macOS.
// The hypothesis under test (docs/LARGE_LOG_VALIDATION.md, "Windows 가설"): the
// per-block path `symlink_metadata` and handle metadata that `Source::read`
// does inside its mutex cost more on Windows, and one synchronous handle
// serializes reads even without the mutex.
//
//   read-lock <MiB> <dir> [runs] [sections]    generate <dir>/bench-<MiB>.log once, then measure
//   read-lock --file <path> [runs] [sections]  measure an existing UTF-8 log
//
// `sections` is a comma-separated subset of `reads` (the block read path),
// `engine` (queries after opening), `restore` (a field filter that starts
// with the file, as restored work does) and `search` (searches five times
// each); all by default. A file larger than
// the OS file cache keeps, such as 8 GiB on a 24 GiB Mac, measures reads from
// the disk: the cache cannot hold it between passes.
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
/// Seconds to the first counted result (completion when none) and to
/// completion, and the count.
fn run(e: &E, id: u64, qid: u64, pane: &str, f: &Value) -> (f64, f64, u64) {
    let t = Instant::now();
    req(
        e,
        json!({"action":"query","pane":pane,"datasetId":id,"queryId":qid,"filters":f}),
    );
    let mut first = None;
    wait(|| {
        let q = &st(e, pane)["query"];
        if first.is_none() && q["count"].as_u64().is_some_and(|n| n > 0) {
            first = Some(t.elapsed().as_secs_f64());
        }
        q["status"] != "running"
    });
    let done = t.elapsed().as_secs_f64();
    let s = st(e, pane);
    assert_eq!(s["query"]["status"], "complete", "{s}");
    (
        first.unwrap_or(done),
        done,
        s["query"]["count"].as_u64().unwrap(),
    )
}
/// Opens `path` with the logs pane query `filters`, as a reopened file's
/// restored work starts it, and polls the first page until indexing ends.
/// Returns the engine, dataset id and seconds to the first row and to EOF.
fn open(path: &Path, filters: &Value) -> (E, u64, f64, f64) {
    let e: E = Arc::new(logstudio_core::Engine::default());
    let id = e.begin_open();
    let t = Instant::now();
    e.open_file(id, path).unwrap();
    req(
        &e,
        json!({"action":"query","pane":"logs","datasetId":id,"queryId":1,"filters":filters}),
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
    (e, id, first.unwrap_or(eof), eof)
}
/// One pass through the engine API, as the app uses it. Each query line
/// gives seconds to the first counted result, to completion, and the count.
fn engine(path: &Path, round: usize) {
    let (e, id, first, eof) = open(path, &json!({}));
    let rows = st(&e, "logs")["total"].as_u64().unwrap();
    println!("engine {round}\tfirst page\t{first:.3}");
    println!("engine {round}\tindex EOF\t{eof:.3}\trows {rows}");
    let (f, s, n) = run(&e, id, 2, "logs", &json!({"levels":["E"]}));
    println!("engine {round}\tlevel E before analysis\t{f:.3}\t{s:.3}\t{n}");
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
        // Common queries within a field filter: only iOS rows (none of
        // them E) have the first, E rows the second.
        (
            "level E + common hidden literal",
            "search",
            json!({"levels":["E"],"query":"한글 텍스트"}),
        ),
        (
            "level E + common shown literal",
            "search",
            json!({"levels":["E"],"query":"some log message"}),
        ),
        (
            "time 12:07:10-20",
            "logs",
            json!({"timeFrom":"12:07:10","timeTo":"12:07:20"}),
        ),
    ];
    for (k, (label, pane, f)) in queries.iter().enumerate() {
        let (first, s, n) = run(&e, id, 10 + k as u64, pane, f);
        println!("engine {round}\t{label}\t{first:.3}\t{s:.3}\t{n}");
    }
    // Cancelling a search that reads every block, mid-file.
    req(
        &e,
        json!({"action":"query","pane":"search","datasetId":id,"queryId":100,"filters":{"query":"rare_needle_"}}),
    );
    sleep(Duration::from_millis(50));
    let t = Instant::now();
    let c = req(
        &e,
        json!({"action":"cancel","pane":"search","datasetId":id,"queryId":100}),
    );
    println!(
        "engine {round}\tcancel search\t{:.4}\t{}",
        t.elapsed().as_secs_f64(),
        c["query"]["status"]
    );
    let t = Instant::now();
    e.close();
    println!("engine {round}\tclose\t{:.4}", t.elapsed().as_secs_f64());
}
/// Searches that read every block and filters that the tables decide, five
/// times each after the analysis, to compare with less noise than single
/// queries.
fn searches(path: &Path, round: usize) {
    let (e, id, _, _) = open(path, &json!({}));
    wait(|| st(&e, "logs")["summaryComplete"] == true);
    let mut qid = 10;
    for (label, pane, f) in [
        ("rare literal", "search", json!({"query":"rare_needle_"})),
        ("literal 한글", "search", json!({"query":"한글"})),
        (
            "regex",
            "search",
            json!({"query":"value=\\d{5}7 ","searchMode":"regex"}),
        ),
        ("level E", "logs", json!({"levels":["E"]})),
        ("pid 123", "logs", json!({"pid":"123"})),
        ("tag gallery", "logs", json!({"tag":"gallery"})),
    ] {
        let times: Vec<String> = (0..5)
            .map(|_| {
                qid += 1;
                format!("{:.3}", run(&e, id, qid, pane, &f).1)
            })
            .collect();
        println!("search {round}\t{label}\t{}", times.join(" "));
    }
    e.close();
}
/// Reopening a file whose saved work restores a field filter: its logs pane
/// query starts with the file, before the level analysis.
fn restore(path: &Path, round: usize) {
    for (label, filters) in [
        ("level E", json!({"levels":["E"]})),
        ("tag gallery", json!({"tag":"gallery"})),
    ] {
        let t = Instant::now();
        let (e, _, first, eof) = open(path, &filters);
        wait(|| st(&e, "logs")["query"]["status"] != "running");
        let s = st(&e, "logs");
        assert_eq!(s["query"]["status"], "complete", "{s}");
        let counted = t.elapsed().as_secs_f64();
        wait(|| st(&e, "logs")["summaryComplete"] == true);
        let summarized = t.elapsed().as_secs_f64();
        println!(
            "restore {round}\t{label}\tfirst row {first:.3}\tindex EOF {eof:.3}\tcount {counted:.3}\tlevels {summarized:.3}\t{}",
            s["query"]["count"]
        );
        e.close();
    }
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
    let sections = rest
        .get(1)
        .map_or("reads,engine,restore,search", String::as_str);
    let section = |name: &str| sections.split(',').any(|s| s == name);
    let cpus = thread::available_parallelism().map_or(0, |n| n.get());
    let size = std::fs::metadata(&path).unwrap().len();
    println!(
        "environment\t{} {}\t{cpus} logical CPUs\tfile {size} bytes\tblock {BLOCK} bytes",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    // The first read pass also warms the OS file cache for the other
    // sections, unless the file is larger than the cache keeps.
    for round in 1..=runs {
        if section("reads") {
            reads(&path, round);
        }
        if section("engine") {
            engine(&path, round);
        }
        if section("restore") {
            restore(&path, round);
        }
        if section("search") {
            searches(&path, round);
        }
    }
}
