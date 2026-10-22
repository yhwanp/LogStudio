// Times the per-block field pass alone: one thread masks every block of the
// first 256 MiB for each filter. The call compiles against the baseline too.
use logstudio_core::{
    dataset::Dataset,
    search::{Filter, Filters},
    source::Source,
};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Instant,
};
fn main() {
    let path = std::path::PathBuf::from(std::env::args().nth(1).unwrap());
    let d = Dataset::new(1, Source::granted(&path).unwrap());
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
    let stop = AtomicBool::new(false);
    let cases: Vec<(&str, Filters)> = vec![
        (
            "level E",
            Filters {
                levels: vec!["E".into()],
                ..Filters::default()
            },
        ),
        (
            "tag",
            Filters {
                tag: "alertsservice".into(),
                ..Filters::default()
            },
        ),
        (
            "time",
            Filters {
                time_from: "21:55:10".into(),
                time_to: "21:55:20".into(),
                ..Filters::default()
            },
        ),
        (
            "query+tag",
            Filters {
                query: "안내문구3".into(),
                tag: "AlertsService".into(),
                ..Filters::default()
            },
        ),
    ];
    for (label, f) in cases {
        let filter = Filter::new(f).unwrap();
        let mut best = f64::MAX;
        let mut bits = 0u64;
        for _ in 0..3 {
            let t = Instant::now();
            bits = 0;
            for (b, bytes) in blocks.iter().zip(&data) {
                let m = filter.mask(bytes, b.rows, &stop).unwrap();
                bits += (0..b.rows)
                    .filter(|i| m[i / 8] & (1 << (i % 8)) != 0)
                    .count() as u64;
            }
            best = best.min(t.elapsed().as_secs_f64());
        }
        println!("{label}\t{best:.3}\t{bits}");
    }
    d.close();
}
