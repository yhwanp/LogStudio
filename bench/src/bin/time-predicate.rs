// Times `Filter::fields_match` alone on rows parsed in advance, for 1 to 32
// time conditions: the parsed rows of the first 64 MiB of a
// `field-filters-gen` log, best of 7, as ns per row. Time conditions read
// the row's date and time; tag conditions, which do not, are the contrast.
// Compiles against the baseline too.
// Usage: time-predicate <path>
use logstudio_core::{
    dataset::Dataset,
    parser,
    search::{Filter, Filters},
    source::{ByteLines, Source},
};
use serde_json::{json, Value};
use std::{sync::atomic::Ordering, time::Instant};
fn main() {
    let path = std::env::args().nth(1).unwrap();
    let d = Dataset::new(1, Source::granted(std::path::Path::new(&path)).unwrap());
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
            logstudio_core::encoding::decode(
                &d.source.read(b.start, (b.end - b.start) as usize).unwrap(),
            )
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
    // No row is at 13:xx; Android rows are at 12:07, iOS rows at 21:55:53.
    let none = |k: usize| json!({"timeFrom": format!("13:{:02}:00", k % 60), "timeTo": format!("13:{:02}:59", k % 60)});
    for n in [1, 2, 4, 8, 16, 32] {
        let cases: Vec<(&str, Value)> = vec![
            (
                "time hides, no row",
                json!({ "exclude": (0..n).map(none).collect::<Vec<_>>() }),
            ),
            (
                "tag hides, no row",
                json!({ "exclude": (0..n).map(|k| json!({"tag": format!("NoSuchTag{k}")})).collect::<Vec<_>>() }),
            ),
            (
                "time shows, first has Android rows",
                json!({ "include": std::iter::once(json!({"timeFrom":"12:07:00","timeTo":"12:07:59"})).chain((1..n).map(none)).collect::<Vec<_>>() }),
            ),
            (
                "time shows + tag hides",
                json!({ "include": (0..n.div_ceil(2)).map(none).collect::<Vec<_>>(), "exclude": (0..n / 2).map(|k| json!({"tag": format!("NoSuchTag{k}")})).collect::<Vec<_>>() }),
            ),
        ];
        for (label, f) in cases {
            let filter = Filter::new(serde_json::from_value::<Filters>(f).unwrap()).unwrap();
            let (mut best, mut count) = (f64::MAX, 0);
            for _ in 0..7 {
                let t = Instant::now();
                count = rows.iter().filter(|r| filter.fields_match(r)).count();
                best = best.min(t.elapsed().as_secs_f64());
            }
            println!(
                "{label}\t{n}\t{:.1} ns/row\t{count}",
                best * 1e9 / rows.len() as f64
            );
        }
    }
    d.close();
}
