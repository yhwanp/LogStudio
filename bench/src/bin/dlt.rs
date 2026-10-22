// A DLT file or its text twin (`dlt-gen --twin`) through the engine API:
// first page, indexing and analysis, field filters after the analysis,
// searches (first page and completion, three times each), a middle page,
// locate and query cancellation. Times in seconds. Run the DLT file and the
// twin in separate processes, alternating; `/usr/bin/time -l` gives the
// peak RSS.
// Usage: dlt <path> [label]
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::Arc,
    thread::sleep,
    time::{Duration, Instant},
};
type E = Arc<logstudio_core::Engine>;
fn req(e: &E, v: Value) -> Value {
    e.request(v).unwrap()
}
fn st(e: &E, pane: &str) -> Value {
    req(e, json!({"action":"status","pane":pane}))
}
fn wait(mut f: impl FnMut() -> bool) {
    while !f() {
        sleep(Duration::from_micros(200));
    }
}
fn page(e: &E, id: u64, pane: &str, qid: u64, start: u64) -> Value {
    req(
        e,
        json!({"action":"page","pane":pane,"datasetId":id,"queryId":qid,"start":start,"count":200}),
    )
}
/// Time to the first page with a row (None if there is none) and to
/// completion, and the count.
fn run(e: &E, id: u64, qid: u64, pane: &str, f: &Value) -> (Option<f64>, f64, u64) {
    let t = Instant::now();
    req(
        e,
        json!({"action":"query","pane":pane,"datasetId":id,"queryId":qid,"filters":f}),
    );
    let mut first = None;
    loop {
        let s = st(e, pane);
        if first.is_none() && s["query"]["count"].as_u64().unwrap_or(0) > 0 {
            let p = page(e, id, pane, qid, 0);
            if !p["rows"].as_array().unwrap().is_empty() {
                first = Some(t.elapsed().as_secs_f64());
            }
        }
        if s["query"]["status"] != "running" {
            assert_eq!(s["query"]["status"], "complete");
            return (
                first,
                t.elapsed().as_secs_f64(),
                s["query"]["count"].as_u64().unwrap(),
            );
        }
        sleep(Duration::from_micros(200));
    }
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let path = PathBuf::from(&a[1]);
    let label = a
        .get(2)
        .cloned()
        .unwrap_or_else(|| path.file_name().unwrap().to_string_lossy().into());
    let e: E = Arc::new(logstudio_core::Engine::default());
    let id = e.begin_open();
    let t = Instant::now();
    e.open_files(id, std::slice::from_ref(&path)).unwrap();
    req(
        &e,
        json!({"action":"query","pane":"logs","datasetId":id,"queryId":1,"filters":{}}),
    );
    let mut first = None;
    while st(&e, "logs")["textComplete"] != true {
        if first.is_none()
            && !page(&e, id, "logs", 1, 0)["rows"]
                .as_array()
                .unwrap()
                .is_empty()
        {
            first = Some(t.elapsed().as_secs_f64());
        }
        sleep(Duration::from_micros(200));
    }
    let eof = t.elapsed().as_secs_f64();
    let s = st(&e, "logs");
    let total = s["total"].as_u64().unwrap();
    println!(
        "{label}\tformat {}\tbytes {}\trows {total}",
        s["format"], s["size"]
    );
    println!("{label}\tfirst page\t{:.4}", first.unwrap_or(eof));
    println!("{label}\tindex EOF\t{eof:.3}");
    wait(|| st(&e, "logs")["summaryComplete"] == true);
    println!("{label}\tanalysis done\t{:.3}", t.elapsed().as_secs_f64());
    // Field filters, for rows with fields: a twin's text rows have none.
    wait(|| st(&e, "logs")["query"]["status"] == "complete");
    let mid = page(&e, id, "logs", 1, total / 2)["rows"][0].clone();
    let mut qid = 10;
    if let (Some(date), Some(time)) = (mid["date"].as_str(), mid["time"].as_str()) {
        // A minute of rows from the middle of the log.
        let minute = format!("{date} {}", &time[..5]);
        let filters = [
            ("level E", json!({"levels":["E"]})),
            ("tag A07", json!({"tag":"A07"})),
            ("exclude ECU1", json!({"excludeTags":"ECU1"})),
            ("time minute", json!({"timeFrom":minute,"timeTo":minute})),
        ];
        for (name, f) in &filters {
            let (_, done, n) = run(&e, id, qid, "logs", f);
            println!("{label}\tfilter {name}\t{done:.3}\t{n}");
            qid += 1;
        }
    }
    let searches = [
        ("rare literal", json!({"query":"RarE_NEEDLE_77"})),
        ("common word", json!({"query":"STATE"})),
        (
            "regex",
            json!({"query":r"number \d+7 with","searchMode":"regex"}),
        ),
    ];
    for (name, f) in &searches {
        for round in 1..=3 {
            let (first, done, n) = run(&e, id, qid, "search", f);
            println!(
                "{label}\tsearch {name} #{round}\tfirst {}\tdone {done:.3}\t{n}",
                first.map_or("-".into(), |f| format!("{f:.4}"))
            );
            qid += 1;
        }
    }
    // Pages and locate in the common word's results.
    let (_, _, n) = run(&e, id, qid, "search", &json!({"query":"STATE"}));
    let t = Instant::now();
    let p = page(&e, id, "search", qid, n / 2);
    println!("{label}\tmiddle page\t{:.4}", t.elapsed().as_secs_f64());
    let line = p["rows"][0]["lineNumber"].as_u64().unwrap();
    let t = Instant::now();
    req(
        &e,
        json!({"action":"locate","pane":"search","datasetId":id,"queryId":qid,"lineNumber":line}),
    );
    println!("{label}\tlocate\t{:.4}", t.elapsed().as_secs_f64());
    qid += 1;
    // Cancelling a running full search.
    req(
        &e,
        json!({"action":"query","pane":"search","datasetId":id,"queryId":qid,"filters":{"query":r"(\w+ ){3}\d+9","searchMode":"regex"}}),
    );
    sleep(Duration::from_millis(30));
    let t = Instant::now();
    let c = req(
        &e,
        json!({"action":"cancel","pane":"search","datasetId":id,"queryId":qid}),
    );
    println!(
        "{label}\tcancel search\t{:.4}\t{}",
        t.elapsed().as_secs_f64(),
        c["query"]["status"]
    );
    e.close();
}
