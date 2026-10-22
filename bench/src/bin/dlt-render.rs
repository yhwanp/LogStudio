// Single-thread cost of reading a DLT file as rows, on the first 256 MiB in
// blocks of whole units of up to 128 KiB as the indexer would publish them:
// splitting into units, rendering rows (system time zone, as the app) and
// parsing rows back into fields. Prints the fastest of the rounds, input
// MB/s, rows/s, per-block render p50/p99 and the text/input size ratio.
// Usage: dlt-render <path.dlt> [rounds]
use logstudio_core::dlt::{self, Clock, Decoder};
use std::time::Instant;
const BLOCK: usize = 128 << 10;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let rounds: usize = a.get(2).map_or(5, |r| r.parse().unwrap());
    let mut data = std::fs::read(&a[1]).unwrap();
    data.truncate(256 << 20);
    // Units of the whole input, then blocks of whole units.
    let units: Vec<dlt::Unit> = dlt::units(&data, data.len()).collect();
    let mut blocks = vec![];
    let (mut start, mut end) = (0, 0);
    for u in &units {
        if u.start + u.len - start > BLOCK && end > start {
            blocks.push(start..end);
            start = end;
        }
        end = u.start + u.len;
    }
    if end > start {
        blocks.push(start..end);
    }
    let decoder = Decoder::new(Clock::local());
    let mb = data.len() as f64 / 1e6;
    let (mut split, mut render, mut parse) = (f64::MAX, f64::MAX, f64::MAX);
    let (mut text_bytes, mut rows, mut per_block) = (0usize, 0usize, vec![]);
    let mut out = Vec::with_capacity(4 * BLOCK);
    for _ in 0..rounds {
        let t = Instant::now();
        let n = dlt::units(&data, data.len()).count();
        split = split.min(t.elapsed().as_secs_f64());
        assert_eq!(n, units.len());
        let (mut times, mut bytes, mut lines) = (Vec::with_capacity(blocks.len()), 0, 0);
        let t = Instant::now();
        for b in &blocks {
            let s = Instant::now();
            out.clear();
            let starts = decoder.render(&data[b.clone()], b.start as u64, &mut out);
            times.push(s.elapsed().as_secs_f64());
            bytes += out.len();
            lines += starts.len() - 1;
        }
        let e = t.elapsed().as_secs_f64();
        if e < render {
            (render, text_bytes, rows, per_block) = (e, bytes, lines, times);
        }
        // Parsing rows back, as page and filter passes do per row; only the
        // parsing is timed.
        let (mut spent, mut levels) = (0.0, 0usize);
        for b in &blocks {
            out.clear();
            decoder.render(&data[b.clone()], b.start as u64, &mut out);
            let text = std::str::from_utf8(&out).unwrap();
            let s = Instant::now();
            for line in text.split_terminator('\n') {
                levels += dlt::parse_row(line).map_or(0, |r| r.level.len());
            }
            spent += s.elapsed().as_secs_f64();
        }
        std::hint::black_box(levels);
        parse = parse.min(spent);
    }
    per_block.sort_by(f64::total_cmp);
    let pct = |p: f64| per_block[((per_block.len() - 1) as f64 * p) as usize] * 1e3;
    println!(
        "input {:.1} MB, {} units, {} blocks, text/input {:.2}",
        mb,
        units.len(),
        blocks.len(),
        text_bytes as f64 / data.len() as f64
    );
    println!("split   {:7.1} MB/s  ({:.3} s)", mb / split, split);
    println!(
        "render  {:7.1} MB/s  {:.2} M rows/s  ({:.3} s)  block p50 {:.3} ms p99 {:.3} ms",
        mb / render,
        rows as f64 / render / 1e6,
        render,
        pct(0.5),
        pct(0.99)
    );
    println!("parse   {:7.1} ns/row", parse / rows as f64 * 1e9);
}
