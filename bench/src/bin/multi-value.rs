// Opens one log through the engine API (4 scan workers) and prints, per
// filter case, the best and worst of 3 complete upper-pane queries and the
// count. Analysis finishes first so it does not compete for CPU.
use logstudio_bench::multi_value_cases::cases;
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::Arc,
    thread::sleep,
    time::{Duration, Instant},
};
type E = Arc<logstudio_core::Engine>;
fn st(e: &E) -> Value {
    e.request(json!({"action":"status","pane":"logs"})).unwrap()
}
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = PathBuf::from(&args[1]);
    let all = args.get(2).is_some_and(|a| a == "all");
    let e: E = Arc::new(logstudio_core::Engine::default());
    let id = e.begin_open();
    let t = Instant::now();
    e.open_file(id, &path).unwrap();
    e.request(json!({"action":"query","pane":"logs","datasetId":id,"queryId":1,"filters":{}}))
        .unwrap();
    while st(&e)["summaryComplete"] != true {
        sleep(Duration::from_millis(1));
    }
    let s = st(&e);
    println!(
        "bytes\t{}\trows {}\tready {:.3}",
        s["size"],
        s["total"],
        t.elapsed().as_secs_f64()
    );
    let mut qid = 10;
    for (label, f) in cases(all) {
        let (mut best, mut worst, mut count) = (f64::MAX, 0f64, 0);
        for _ in 0..3 {
            qid += 1;
            let t = Instant::now();
            e.request(
                json!({"action":"query","pane":"logs","datasetId":id,"queryId":qid,"filters":f}),
            )
            .unwrap();
            while st(&e)["query"]["status"] == "running" {
                sleep(Duration::from_micros(500));
            }
            let elapsed = t.elapsed().as_secs_f64();
            let s = st(&e);
            assert_eq!(s["query"]["status"], "complete", "{label}");
            count = s["query"]["count"].as_u64().unwrap();
            (best, worst) = (best.min(elapsed), worst.max(elapsed));
        }
        println!("{label}\t{best:.3}–{worst:.3}\t{count}");
    }
    e.close();
}
