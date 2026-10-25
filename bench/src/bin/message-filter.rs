// Times message conditions through the engine API, as the logs pane runs
// them, beside the filters they are compared with: a level the tables decide,
// a time range and a search that read every block. Each query reports its
// completion and, for listed conditions, when the logs pane knows whether
// each has rows. Rows come from `field-filters-gen`.
//
//   message-filter <log> [runs]
use serde_json::{json, Value};
use std::{
    path::Path,
    sync::Arc,
    thread::sleep,
    time::{Duration, Instant},
};
type E = Arc<logstudio_core::Engine>;

fn st(e: &E) -> Value {
    e.request(json!({"action":"status","pane":"logs"})).unwrap()
}
fn wait(mut f: impl FnMut() -> bool) {
    while !f() {
        sleep(Duration::from_micros(500));
    }
}
fn pending(s: &Value) -> bool {
    s["query"]["conditions"].to_string().contains("pending")
}
/// Seconds to complete, rows, and seconds until no condition is pending.
fn run(e: &E, id: u64, qid: u64, f: &Value) -> (f64, u64, f64) {
    let t = Instant::now();
    e.request(json!({"action":"query","pane":"logs","datasetId":id,"queryId":qid,"filters":f}))
        .unwrap();
    wait(|| st(e)["query"]["status"] != "running");
    let done = t.elapsed().as_secs_f64();
    let s = st(e);
    assert_eq!(s["query"]["status"], "complete", "{s}");
    wait(|| !pending(&st(e)));
    (
        done,
        s["query"]["count"].as_u64().unwrap(),
        t.elapsed().as_secs_f64(),
    )
}
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = Path::new(args.first().expect("<log> [runs]"));
    let runs: usize = args.get(1).map_or(2, |r| r.parse().expect("runs"));
    let filters = [
        ("level E (tables)", json!({"levels":["E"]})),
        (
            "time 1 min",
            json!({"timeFrom":"12:07:00","timeTo":"12:07:59"}),
        ),
        ("search rare (no fields)", json!({"query":"RareFieldTag"})),
        (
            "show message 한글 (1/3 rows)",
            json!({"include":[{"message":"한글"}]}),
        ),
        (
            "show message none",
            json!({"include":[{"message":"zzz-nothing"}]}),
        ),
        (
            "show message none, as a pattern",
            json!({"include":[{"message":"zzz-nothing","messageRegex":true}]}),
        ),
        (
            "hide message rare",
            json!({"exclude":[{"message":"rare field row"}]}),
        ),
        (
            "hide message rare, as a pattern",
            json!({"exclude":[{"message":"rare field row","messageRegex":true}]}),
        ),
        (
            "hide message value= (2/3 rows)",
            json!({"exclude":[{"message":"VALUE="}]}),
        ),
        (
            "hide message regex",
            json!({"exclude":[{"message":r"value=\d+7 ","messageRegex":true}]}),
        ),
        (
            "show tag rare + message",
            json!({"include":[{"tag":"RareFieldTag","message":"rare"}]}),
        ),
        (
            "level E + hide message",
            json!({"levels":["E"],"exclude":[{"message":"한글"}]}),
        ),
    ];
    for round in 1..=runs {
        let e: E = Arc::new(logstudio_core::Engine::default());
        let id = e.begin_open();
        let t = Instant::now();
        e.open_file(id, path).unwrap();
        wait(|| st(&e)["summaryComplete"] == true);
        println!(
            "round {round}\tindex and analysis\t{:.3}\trows {}",
            t.elapsed().as_secs_f64(),
            st(&e)["total"]
        );
        for (k, (label, f)) in filters.iter().enumerate() {
            let (done, n, checked) = run(&e, id, 10 + k as u64, f);
            println!("round {round}\t{label}\t{done:.3} s\t{n} rows\tchecked {checked:.3} s");
        }
        e.close();
    }
}
