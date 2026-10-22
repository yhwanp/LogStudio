// What ordering a picked or dropped multi-selection costs before it opens.
// For each case: N rotated files (`main.log.N-1` .. `main.log`) of S MiB, given
// newest first. `plain` opens them in the given order; `arranged` first runs
// `order::arrange` (which reverses them) and then opens. Runs alternate.
// Timings are from the call until `open_files` returns (the page then gets
// `opened`) and until the first page has rows (first usable display).
// Layout `first`: every row is a log row. `late`: the first and last 64 KiB
// hold no log row, so every row of both ends is parsed and rejected (name
// order decides).
use logstudio_core::{order, Engine};
use serde_json::json;
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
    thread::sleep,
    time::{Duration, Instant},
};

fn write(dir: &Path, files: usize, mib: usize, late: bool) -> Vec<PathBuf> {
    (0..files)
        .map(|k| {
            // A higher rotation number is older: file k starts k hours earlier.
            let name = if k == 0 { "main.log".to_string() } else { format!("main.log.{k}") };
            let path = dir.join(name);
            let mut out = std::io::BufWriter::new(std::fs::File::create(&path).unwrap());
            let filler = format!("{}\n", "x".repeat(99));
            let fill = |out: &mut std::io::BufWriter<std::fs::File>| {
                for _ in 0..order::HEAD.div_ceil(filler.len()) {
                    out.write_all(filler.as_bytes()).unwrap();
                }
                order::HEAD.div_ceil(filler.len()) * filler.len()
            };
            let mut written = if late { fill(&mut out) } else { 0 };
            let hour = 23 - k % 24;
            let mut n = 0u64;
            while written < mib << 20 {
                let row = format!(
                    "06-{:02} {hour:02}:{:02}:{:02}.{:03}  1234  5678 I Tag{}: row {n} payload value={}\n",
                    18 - k / 24,
                    n / 60_000 % 60,
                    n / 1000 % 60,
                    n % 1000,
                    n % 7,
                    n * 31
                );
                out.write_all(row.as_bytes()).unwrap();
                written += row.len();
                n += 1;
            }
            if late {
                fill(&mut out);
            }
            path
        })
        .collect()
}
fn open(paths: &[PathBuf]) -> (f64, f64) {
    let e = Arc::new(Engine::default());
    let id = e.begin_open();
    let t = Instant::now();
    e.open_files(id, paths).unwrap();
    let opened = t.elapsed().as_secs_f64();
    e.request(json!({"action":"query","pane":"logs","datasetId":id,"queryId":1,"filters":{}}))
        .unwrap();
    loop {
        let page = e
            .request(json!({"action":"page","pane":"logs","datasetId":id,"queryId":1,"start":0,"count":200}))
            .unwrap();
        if !page["rows"].as_array().unwrap().is_empty() {
            break;
        }
        sleep(Duration::from_micros(200));
    }
    let first = t.elapsed().as_secs_f64();
    e.close();
    (opened, first)
}
fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}
fn ms(v: &[f64]) -> String {
    let (lo, hi) = (
        v.iter().copied().fold(f64::MAX, f64::min),
        v.iter().copied().fold(0.0, f64::max),
    );
    format!(
        "{:.2} ms ({:.2}–{:.2})",
        median(v.to_vec()) * 1e3,
        lo * 1e3,
        hi * 1e3
    )
}
fn main() {
    let base = PathBuf::from(std::env::args().nth(1).expect("temp dir"));
    let runs = 9;
    println!("case\tplain open\tplain first rows\tarrange alone\tarranged open\tarranged first rows\tbasis");
    for (files, mib, late) in [
        (2, 64, false),
        (8, 16, false),
        (64, 4, false),
        (64, 4, true),
    ] {
        let dir = base.join(format!(
            "{files}x{mib}-{}",
            if late { "late" } else { "first" }
        ));
        std::fs::create_dir_all(&dir).unwrap();
        // Given newest first, so arranging reverses the order.
        let given = write(&dir, files, mib, late);
        let (mut po, mut pf, mut aa, mut ao, mut af, mut basis) =
            (vec![], vec![], vec![], vec![], vec![], None);
        open(&given); // Warm-up: the files were just written and are cached.
        for _ in 0..runs {
            let (o, f) = open(&given);
            po.push(o);
            pf.push(f);
            let t = Instant::now();
            let (paths, by) = order::arrange(given.clone());
            let arranged = t.elapsed().as_secs_f64();
            let (o, f) = open(&paths);
            aa.push(arranged);
            ao.push(arranged + o);
            af.push(arranged + f);
            basis = by;
        }
        println!(
            "{files} × {mib} MiB {}\t{}\t{}\t{}\t{}\t{}\t{basis:?}",
            if late { "late" } else { "first" },
            ms(&po),
            ms(&pf),
            ms(&aa),
            ms(&ao),
            ms(&af)
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
