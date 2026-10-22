// Opens the given files as one log through the engine API and prints timings
// (seconds) and counts. `--each` opens every file alone and prints its counts.
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc},
    thread::sleep,
    time::{Duration, Instant},
};
type E = Arc<logstudio_core::Engine>;
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
fn open(paths: &[PathBuf]) -> (E, u64, f64, f64) {
    let e: E = Arc::new(logstudio_core::Engine::default());
    let id = e.begin_open();
    let t = Instant::now();
    e.open_files(id, paths).unwrap();
    req(
        &e,
        json!({"action":"query","pane":"logs","datasetId":id,"queryId":1,"filters":{}}),
    );
    let mut first = None;
    while st(&e, "logs")["textComplete"] != true {
        if first.is_none() && !req(&e, json!({"action":"page","pane":"logs","datasetId":id,"queryId":1,"start":0,"count":200}))["rows"].as_array().unwrap().is_empty() {
            first = Some(t.elapsed().as_secs_f64());
        }
        sleep(Duration::from_micros(500));
    }
    let eof = t.elapsed().as_secs_f64();
    (e, id, first.unwrap_or(eof), eof)
}
fn queries() -> Vec<(&'static str, &'static str, Value)> {
    vec![
        (
            "rare literal (all files)",
            "search",
            json!({"query":"rare_needle_"}),
        ),
        (
            "rare literal (file 3 only)",
            "search",
            json!({"query":"rare_needle_f2_"}),
        ),
        ("literal 한글", "search", json!({"query":"한글"})),
        (
            "regex",
            "search",
            json!({"query":"value=\\d{5}7 ","searchMode":"regex"}),
        ),
        ("level E", "logs", json!({"levels":["E"]})),
        ("tag gallery", "logs", json!({"tag":"gallery"})),
    ]
}
fn run(e: &E, id: u64, qid: u64, pane: &str, f: &Value) -> (f64, u64) {
    let t = Instant::now();
    req(
        e,
        json!({"action":"query","pane":pane,"datasetId":id,"queryId":qid,"filters":f}),
    );
    wait(|| st(e, pane)["query"]["status"] != "running");
    let s = st(e, pane);
    assert_eq!(s["query"]["status"], "complete");
    (
        t.elapsed().as_secs_f64(),
        s["query"]["count"].as_u64().unwrap(),
    )
}
fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let each = args.first().is_some_and(|a| a == "--each");
    if each {
        args.remove(0);
    }
    let paths: Vec<PathBuf> = args.iter().map(PathBuf::from).collect();
    if each {
        for p in &paths {
            let (e, id, _, eof) = open(std::slice::from_ref(p));
            let total = st(&e, "logs")["total"].as_u64().unwrap();
            wait(|| st(&e, "logs")["summaryComplete"] == true);
            let counts: Vec<u64> = queries()
                .iter()
                .enumerate()
                .map(|(i, (_, pane, f))| run(&e, id, 10 + i as u64, pane, f).1)
                .collect();
            println!(
                "each\t{}\tindex EOF {eof:.3}\trows {total}\tcounts {counts:?}",
                p.file_name().unwrap().to_string_lossy()
            );
            e.close();
        }
        return;
    }
    let (e, id, first, eof) = open(&paths);
    let s = st(&e, "logs");
    println!("files\t{}\tbytes {}", paths.len(), s["size"]);
    println!("first page\t{first:.3}");
    println!("index EOF\t{eof:.3}\trows {}", s["total"]);
    println!(
        "file first rows\t{:?}",
        s["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["firstLine"].as_u64().unwrap())
            .collect::<Vec<_>>()
    );
    // A field filter before analysis completes, then the remaining analysis.
    let (t, n) = run(&e, id, 2, "logs", &json!({"levels":["E"]}));
    println!("level E before analysis\t{t:.3}\t{n}");
    let t = Instant::now();
    wait(|| st(&e, "logs")["summaryComplete"] == true);
    println!("analysis remaining\t{:.3}", t.elapsed().as_secs_f64());
    let mut counts = vec![];
    for (i, (label, pane, f)) in queries().iter().enumerate() {
        let qid = 10 + i as u64;
        let (t, n) = run(&e, id, qid, pane, f);
        counts.push(n);
        // Pages at the start, middle and end of the result; the last page
        // starts at the final result, so its row is the result's last row.
        let t2 = Instant::now();
        let mut last = Value::Null;
        for start in [0, n / 2, n.saturating_sub(1)] {
            last = req(
                &e,
                json!({"action":"page","pane":pane,"datasetId":id,"queryId":qid,"start":start,"count":200}),
            );
        }
        let row = last["rows"]
            .as_array()
            .unwrap()
            .last()
            .cloned()
            .unwrap_or(Value::Null);
        println!(
            "{label}\t{t:.3}\t{n}\t3 pages {:.3}\tlast result file {} line {} combined {}",
            t2.elapsed().as_secs_f64(),
            row["file"],
            row["fileLine"],
            row["lineNumber"]
        );
    }
    println!("counts\t{counts:?}");
    // Locate and read the last physical row through the unfiltered pane.
    req(
        &e,
        json!({"action":"query","pane":"logs","datasetId":id,"queryId":30,"filters":{}}),
    );
    wait(|| st(&e, "logs")["query"]["status"] != "running");
    let total = st(&e, "logs")["total"].as_u64().unwrap();
    let t = Instant::now();
    let at = req(
        &e,
        json!({"action":"locate","pane":"logs","datasetId":id,"queryId":30,"lineNumber":total}),
    );
    let range = req(
        &e,
        json!({"action":"range","pane":"logs","datasetId":id,"queryId":30,"lineNumber":total,"startByte":0}),
    );
    println!(
        "locate+range last row\t{:.3}\tindex {}\tfile {} line {}",
        t.elapsed().as_secs_f64(),
        at["index"],
        range["file"],
        range["fileLine"]
    );
    // Export one file's rare rows; the bytes must be that file's marker rows.
    let out = std::env::temp_dir().join("multi-file-bench-export.log");
    let (t, n) = run(&e, id, 31, "search", &json!({"query":"rare_needle_f3_"}));
    let t2 = Instant::now();
    let r = e
        .export("search", id, 31, &out, &AtomicBool::new(false), |_, _| {})
        .unwrap();
    let written = std::fs::read_to_string(&out).unwrap();
    let ok = written.lines().count() as u64 == n
        && written.lines().all(|l| l.contains("RarE_NEEDLE_F3_"));
    println!(
        "export file 4 rare rows\tsearch {t:.3}\texport {:.3}\t{r}\tverified {ok}",
        t2.elapsed().as_secs_f64()
    );
    let _ = std::fs::remove_file(&out);
    // Cancel a whole-source regex scan shortly after it starts.
    req(
        &e,
        json!({"action":"query","pane":"search","datasetId":id,"queryId":40,"filters":{"query":"payload=abcdef\\d+9$","searchMode":"regex"}}),
    );
    sleep(Duration::from_millis(100));
    let t = Instant::now();
    req(
        &e,
        json!({"action":"cancel","pane":"search","datasetId":id,"queryId":40}),
    );
    let s = st(&e, "search")["query"].clone();
    println!(
        "cancel after 100 ms\t{:.4}\tstatus {}\tscanned {}",
        t.elapsed().as_secs_f64(),
        s["status"],
        s["scannedBytes"]
    );
    // Close while both panes scan.
    req(
        &e,
        json!({"action":"query","pane":"search","datasetId":id,"queryId":41,"filters":{"query":"payload=abcdef\\d+9$","searchMode":"regex"}}),
    );
    req(
        &e,
        json!({"action":"query","pane":"logs","datasetId":id,"queryId":42,"filters":{"tag":"wifi"}}),
    );
    sleep(Duration::from_millis(100));
    let t = Instant::now();
    e.close();
    println!("close while scanning\t{:.4}", t.elapsed().as_secs_f64());
}
