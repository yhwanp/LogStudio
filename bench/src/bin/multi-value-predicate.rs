// Times `Filter::fields_match` alone on rows parsed in advance: the parsed
// rows of the first 64 MiB, best of 5, as ns per row. Parsing and decoding
// dominate the full field pass, so this isolates the per-row list cost.
// Compiles against the baseline too.
use logstudio_bench::multi_value_cases::cases;
use logstudio_core::{
    dataset::Dataset,
    encoding, parser,
    search::{Filter, Filters},
    source::{ByteLines, Source},
};
use std::{sync::atomic::Ordering, time::Instant};
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
        .take(512)
        .collect();
    let texts: Vec<String> = blocks
        .iter()
        .map(|b| {
            encoding::decode(&d.source.read(b.start, (b.end - b.start) as usize).unwrap())
                .into_owned()
        })
        .collect();
    let mut rows = vec![];
    for t in &texts {
        for (offset, bytes) in ByteLines::new(t.as_bytes()) {
            let r = parser::parse_line(&t[offset..offset + bytes.len()], 1, true);
            if r.parsed.is_some() {
                rows.push(r);
            }
        }
    }
    println!("parsed rows\t{}", rows.len());
    for (label, f) in cases(all) {
        let filter = Filter::new(serde_json::from_value::<Filters>(f).unwrap()).unwrap();
        let (mut best, mut count) = (f64::MAX, 0);
        for _ in 0..5 {
            let t = Instant::now();
            count = rows.iter().filter(|r| filter.fields_match(r)).count();
            best = best.min(t.elapsed().as_secs_f64());
        }
        println!(
            "{label}\t{:.1} ns/row\t{count}",
            best * 1e9 / rows.len() as f64
        );
    }
    d.close();
}
