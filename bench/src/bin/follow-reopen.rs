// Opens a log with the measurement's two queries and prints when indexing,
// both queries and the level analysis finish. Uses only API the core had
// before following, so it also builds against a baseline core:
//   follow-reopen <path>
use logstudio_core::{
    dataset::Dataset,
    search::{Filter, Filters},
    source::Source,
};
use serde_json::{json, Value};
use std::{
    path::Path,
    thread::sleep,
    time::{Duration, Instant},
};
fn wire(v: Value) -> Filter {
    Filter::new(serde_json::from_value::<Filters>(v).unwrap()).unwrap()
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let t = Instant::now();
    let d = Dataset::new(1, Source::granted(Path::new(&a[1])).unwrap());
    d.start(|_| {});
    let q = [
        d.query(0, 1, wire(json!({"levels":["W","E"]}))).unwrap(),
        d.query(1, 1, wire(json!({"query":"rare_needle"}))).unwrap(),
    ];
    let (mut index, mut scans) = (None, None);
    loop {
        if index.is_none() && d.index.lock().unwrap().done {
            index = Some(t.elapsed().as_secs_f64());
        }
        if scans.is_none() && q.iter().all(|q| q.json()["status"] == "complete") {
            scans = Some(t.elapsed().as_secs_f64());
        }
        if index.is_some() && scans.is_some() && d.status(0)["summaryComplete"] == true {
            break;
        }
        sleep(Duration::from_micros(200));
    }
    let levels = t.elapsed().as_secs_f64();
    println!(
        "reopen index={:.4} queries={:.4} levels={levels:.4} counts=({}, {}, {})",
        index.unwrap(),
        scans.unwrap(),
        q[0].json()["count"],
        q[1].json()["count"],
        d.index.lock().unwrap().total
    );
    d.close();
}
