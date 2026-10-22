// Opens one log through the engine API and prints timings (seconds), counts
// and a digest of each result, so a baseline and a change can be compared.
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::Arc,
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
    let path = PathBuf::from(std::env::args().nth(1).unwrap());
    let e: E = Arc::new(logstudio_core::Engine::default());
    let id = e.begin_open();
    let t = Instant::now();
    e.open_file(id, &path).unwrap();
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
    let s = st(&e, "logs");
    println!("bytes\t{}", s["size"]);
    println!("first page\t{:.3}", first.unwrap_or(eof));
    println!("index EOF\t{eof:.3}\trows {}", s["total"]);
    // Field filters before analysis mask every block.
    for (k, f) in [json!({"levels":["E"]}), json!({"tag":"alertsservice"})]
        .iter()
        .enumerate()
    {
        let (t, n) = run(&e, id, 2 + k as u64, "logs", f);
        println!("before analysis {f}\t{t:.3}\t{n}");
    }
    let t = Instant::now();
    wait(|| st(&e, "logs")["summaryComplete"] == true);
    println!("analysis remaining\t{:.3}", t.elapsed().as_secs_f64());
    let s = st(&e, "logs")["summary"].clone();
    println!(
        "summary\tparsed {}\traw {}\tblank {}\tcontinuation {}",
        s["parsed"], s["raw"], s["blank"], s["continuation"]
    );
    let cases = [
        ("logs", json!({"levels":["E"]})),
        ("logs", json!({"levels":["D","I"],"includeRaw":false})),
        ("logs", json!({"tag":"alertsservice"})),
        (
            "logs",
            json!({"excludeTags":"alertsservice,androidruntime"}),
        ),
        ("logs", json!({"pid":"3870"})),
        ("logs", json!({"timeFrom":"21:55:10","timeTo":"21:55:20"})),
        ("logs", json!({"includeRaw":false})),
        ("search", json!({"query":"rare_needle_"})),
        ("search", json!({"query":"rare_needle_","levels":["E"]})),
        ("search", json!({"query":"안내문구3","tag":"AlertsService"})),
        (
            "search",
            json!({"query":"C[0-9]\\.run","searchMode":"regex","pid":"120"}),
        ),
        ("search", json!({"query":"한글","levels":["W"]})),
    ];
    for (i, (pane, f)) in cases.iter().enumerate() {
        let qid = 10 + i as u64;
        let (t, n) = run(&e, id, qid + 100, pane, f);
        let t2 = Instant::now();
        let mut digest = 0u64;
        let mut owners = 0;
        for start in [0, n / 2, n.saturating_sub(200)] {
            let page = req(
                &e,
                json!({"action":"page","pane":pane,"datasetId":id,"queryId":qid + 100,"start":start,"count":200}),
            );
            for r in page["rows"].as_array().unwrap() {
                digest = digest
                    .wrapping_mul(31)
                    .wrapping_add(r["lineNumber"].as_u64().unwrap());
                owners += usize::from(r.get("owner").is_some());
            }
        }
        println!(
            "{pane} {f}\t{t:.3}\t{n}\t3 pages {:.4}\tdigest {digest:x}\towners {owners}",
            t2.elapsed().as_secs_f64()
        );
    }
    e.close();
}
