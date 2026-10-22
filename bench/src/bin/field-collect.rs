// Single-thread cost of the analysis pass per row, without and with field
// collection, alternating rounds over the same 128 KiB blocks (first 256 MiB
// of the file); prints the fastest round of each. The loop mirrors
// `Dataset::analyze`'s worker: parse with and without line number
// recognition and classify entry rows, then optionally `Collector`.
// Usage: field-collect <log path> [rounds]
use logstudio_core::{encoding, fields_index::Collector, parser, source::ByteLines};
use std::time::Instant;
fn opens(r: &parser::Record, recognize: bool) -> Option<usize> {
    match &r.parsed {
        Some(p) => parser::LEVELS.iter().position(|l| *l == p.level),
        None => parser::starts_entry(r.raw, recognize).then_some(8),
    }
}
fn pass(blocks: &[&[u8]], collect: bool) -> (f64, usize, usize) {
    let t = Instant::now();
    let (mut rows, mut kept) = (0, 0);
    for data in blocks {
        let text = encoding::decode(data);
        let mut values = Collector::default();
        for (offset, bytes) in ByteLines::new(text.as_bytes()) {
            let raw = &text[offset..offset + bytes.len()];
            let r = parser::parse_line(raw, 1, false);
            let o = opens(&r, false);
            // `parse_numbered_line` is crate-private; this is its result.
            let numbered = if r.parsed.is_none() {
                Some(parser::parse_line(raw, 1, true)).filter(|n| n.parsed.is_some())
            } else {
                None
            };
            let o1 = opens(numbered.as_ref().unwrap_or(&r), true);
            rows += 1;
            kept += usize::from(o.is_some()) + usize::from(o1.is_some());
            if collect {
                values.add(&r, o, numbered.as_ref(), o1);
            }
        }
        std::hint::black_box(values.finish());
    }
    (t.elapsed().as_secs_f64(), rows, kept)
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let rounds: usize = a.get(2).map_or(5, |r| r.parse().unwrap());
    let mut data = std::fs::read(&a[1]).unwrap();
    data.truncate(256 << 20);
    let mut blocks = vec![];
    let mut at = 0;
    while at < data.len() {
        let end = (at + (128 << 10)).min(data.len());
        let end = memchr_last(&data[at..end]).map_or(end, |i| at + i + 1);
        blocks.push(&data[at..end]);
        at = end;
    }
    let (mut best, mut rows) = ([f64::MAX; 2], 0);
    for round in 0..rounds {
        for collect in if round % 2 == 0 {
            [false, true]
        } else {
            [true, false]
        } {
            let (t, n, _) = pass(&blocks, collect);
            best[usize::from(collect)] = best[usize::from(collect)].min(t);
            rows = n;
        }
    }
    let ns = |t: f64| t * 1e9 / rows as f64;
    println!(
        "rows {rows}\tparse only {:.3}s {:.1} ns/row\twith fields {:.3}s {:.1} ns/row\textra {:.1} ns/row ({:+.1}%)",
        best[0],
        ns(best[0]),
        best[1],
        ns(best[1]),
        ns(best[1]) - ns(best[0]),
        (best[1] / best[0] - 1.0) * 100.0
    );
}
fn memchr_last(b: &[u8]) -> Option<usize> {
    b.iter().rposition(|&c| c == b'\n')
}
