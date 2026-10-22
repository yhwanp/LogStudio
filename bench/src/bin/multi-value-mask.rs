// Times the per-block field pass alone: one thread masks every block of the
// first 256 MiB for each case, best of 3. Compiles against the baseline too.
use logstudio_bench::multi_value_cases::cases;
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
    let args: Vec<String> = std::env::args().collect();
    let all = args.get(2).is_some_and(|a| a == "all");
    let d = Dataset::new(1, Source::granted(std::path::Path::new(&args[1])).unwrap());
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
    for (label, f) in cases(all) {
        let filter = Filter::new(serde_json::from_value::<Filters>(f).unwrap()).unwrap();
        let (mut best, mut bits) = (f64::MAX, 0u64);
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
