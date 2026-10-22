// Times `Filter::dlt_mask` alone, the header decision of DLT blocks without
// a search query: one thread over the first 256 MiB of a `dlt-gen android`
// file, best of 5, as ns per message, for 1, 8 and 32 conditions. The
// call compiles against the baseline too.
// Usage: dlt-mask <path.dlt>
use logstudio_core::{
    dataset::Dataset,
    dlt::{Clock, Decoder},
    search::{Filter, Filters},
    source::Source,
};
use serde_json::{json, Value};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Instant,
};
fn main() {
    let path = std::path::PathBuf::from(std::env::args().nth(1).unwrap());
    let d = Dataset::with_dlt_clock(1, Source::granted(&path).unwrap(), Clock::Fixed(0));
    d.pause_analysis.store(true, Ordering::Release);
    d.start(|_| {});
    while !d.index.lock().unwrap().done {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let blocks: Vec<_> = d
        .index
        .lock()
        .unwrap()
        .blocks
        .iter()
        .copied()
        .take(2048)
        .collect();
    let data: Vec<Vec<u8>> = blocks
        .iter()
        .map(|b| d.source.read(b.start, (b.end - b.start) as usize).unwrap())
        .collect();
    let rows: usize = blocks.iter().map(|b| b.rows).sum();
    let decoder = Decoder::new(Clock::Fixed(0));
    let stop = AtomicBool::new(false);
    // Rows are from 09:26:53 UTC; none at 03:xx.
    let none = |k: usize| json!({"timeFrom": format!("03:{:02}:00", k % 60), "timeTo": format!("03:{:02}:59", k % 60)});
    let tagged = |k: usize| json!({"tag": format!("A{:02}", k % 40), "timeFrom": "04:00", "timeTo": "04:59"});
    for n in [1, 8, 32] {
        let cases: Vec<(&str, Value)> = vec![
            (
                "time hides, no row",
                json!({ "exclude": (0..n).map(none).collect::<Vec<_>>() }),
            ),
            (
                "tag+time hides, no row",
                json!({ "exclude": (0..n).map(tagged).collect::<Vec<_>>() }),
            ),
            (
                "tag shows",
                json!({ "include": (0..n).map(|k| json!({"tag": format!("A{:02}", k % 40)})).collect::<Vec<_>>() }),
            ),
        ];
        for (label, f) in cases {
            let filter = Filter::new(serde_json::from_value::<Filters>(f).unwrap()).unwrap();
            let (mut best, mut count) = (f64::MAX, 0);
            for _ in 0..5 {
                let t = Instant::now();
                count = 0;
                for (b, raw) in blocks.iter().zip(&data) {
                    let m = filter.dlt_mask(&decoder, raw, b.rows, &stop).unwrap();
                    count += (0..b.rows)
                        .filter(|i| m.bits[i / 8] & (1 << (i % 8)) != 0)
                        .count();
                }
                best = best.min(t.elapsed().as_secs_f64());
            }
            println!(
                "{label}\t{n}\t{:.1} ns/message\t{count}",
                best * 1e9 / rows as f64
            );
        }
    }
    d.close();
}
