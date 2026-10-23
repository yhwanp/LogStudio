// Ways to find the conditions of a filter that match no row, through the
// core's public API. A condition matches no row when no log entry passes it;
// continuation rows follow their entry and orphan rows pass none, so only
// entry rows decide. Methods, each over sets of 1, 8 and 32 conditions:
//   table  the block level and field tables alone, per condition. One they
//          cannot decide (a time range, two tested fields, a dropped field)
//          stays open with the blocks that may hold a match.
//   first  table, then per open condition a scan of its open blocks with
//          the pane's worker count that stops at its first match.
//   joint  table, then one scan of the open conditions' blocks: each block
//          is masked once with every open condition, and again per
//          condition only when that mask has a row.
//   count  a count query per condition (what showing counts would need);
//          sets of at most 8 conditions.
// The sets: conditions the tables decide or that need rows, with or without
// matching rows, and a mix. The methods' answers are compared.
// `rows` times the field decision per parsed row on one thread (first
// 64 MiB of a text log): as the query's scan makes it, and also learning
// every condition's own match, the extra work of collecting condition bits
// with the query. `beside` times a query that reads every block alone and
// while a joint scan of 32 conditions without rows runs, with and without
// the scan yielding to it, and cancelling a joint scan after 100 ms.
// `engine` opens the log again for each set and runs a logs pane query
// hiding its conditions once the analysis is done: the first rows, the
// query's completion and when the engine's own check knows every condition
// (`conditions` in the query state; a core without it shows "-"), then the
// same filter again and with one condition replaced. `overhead` alternates,
// on one dataset, a logs pane query (checked) and a search pane query (not
// checked) hiding the same kind of fresh conditions that need rows, and
// times each query's completion; each checked query's check ends first.
// Usage: zero-conditions <path> [methods] [rounds]; methods are
// comma-separated (default table,first,joint,count,rows,beside). Times in
// seconds; `/usr/bin/time -l` gives the peak RSS.
use logstudio_core::{
    dataset::{Block, BlockLevels, Dataset, Query},
    dlt::{Clock, Decoder},
    encoding,
    fields_index::FieldIndex,
    parser,
    search::{Condition, Filter, Filters},
    source::{ByteLines, Format, Source},
};
use serde_json::{json, Value};
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::*},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
const TAGS: [&str; 5] = [
    "ActivityManager",
    "SDK_UI",
    "GalleryManager",
    "BluetoothGatt",
    "WifiService",
];
const LV: [&str; 5] = ["V", "D", "I", "W", "E"];
const SETS: [&str; 5] = [
    "table-none",
    "table-some",
    "rows-none",
    "rows-some",
    "mixed",
];
/// Condition `k` of one kind, for `field-filters-gen` text or `dlt-gen`
/// android DLT. `table-*` the tables decide, `rows-*` need rows; `*-none`
/// match no row.
fn kind(dlt: bool, name: &str, k: usize) -> Value {
    let name = if name == "mixed" { SETS[k % 4] } else { name };
    match (dlt, name) {
        (false, "table-none") => match k % 4 {
            0 => json!({"tag": format!("NoSuchTag{k}")}),
            1 => json!({"pid": (9000 + k).to_string()}),
            2 => json!({"tid": (7000 + k).to_string()}),
            // Android rows give tag j level j, iOS rows level I.
            _ => {
                let a = k % 5;
                let other: Vec<usize> = (0..5).filter(|&l| l != a && l != 2).collect();
                json!({"tag": TAGS[a], "levels": [LV[other[k / 4 % other.len()]]]})
            }
        },
        (false, "table-some") => match k % 3 {
            0 => json!({"pid": (100 + k % 50).to_string()}),
            1 => json!({"tid": (2000 + k % 300).to_string()}),
            _ => json!({"tag": TAGS[k % 5], "levels": [LV[k % 5]]}),
        },
        (false, "rows-none") => match k % 3 {
            // PID 100 + j logs only tag j % 5, in every block.
            0 => {
                let a = k % 5;
                let j = (5 * (k / 3) + a + 1) % 50;
                json!({"tag": TAGS[a], "pid": (100 + j).to_string()})
            }
            1 => {
                json!({"timeFrom": format!("13:{:02}:00", k % 60), "timeTo": format!("13:{:02}:59", k % 60)})
            }
            // The rare PID's rows are at 12:07:30, in few blocks.
            _ => {
                json!({"pid": "4242", "timeFrom": format!("12:0{}:00", k % 7), "timeTo": format!("12:0{}:59", k % 7)})
            }
        },
        (false, "rows-some") => match k % 2 {
            0 => {
                let a = k % 5;
                let j = (5 * (k / 2) + a) % 50;
                json!({"tag": TAGS[a], "pid": (100 + j).to_string()})
            }
            // Android rows are at 12:07:SS, SS = row % 60 not divisible by 3.
            _ => {
                let s = (3 * k + 1) % 60;
                json!({"timeFrom": format!("12:07:{s:02}"), "timeTo": format!("12:07:{s:02}")})
            }
        },
        // APIDs A00-A39, CTIDs C000-C199, ECUs ECU1, HU and TCU, levels F
        // to V; rows from 09:26:53 UTC, 1,000 per second; no PID or TID.
        (true, "table-none") => match k % 3 {
            0 => json!({"tag": format!("A{}", 40 + k)}),
            1 => json!({"pid": (k + 1).to_string()}),
            _ => {
                let (ecu, level) = (["ECU1", "HU", "TCU"][k / 3 % 3], ["A", "U"][k % 2]);
                json!({"tag": ecu, "levels": [level]})
            }
        },
        (true, "table-some") => match k % 3 {
            0 => json!({"tag": format!("A{:02}", k % 40)}),
            1 => json!({"tag": format!("C{:03}", k * 7 % 200)}),
            _ => {
                let (ecu, level) = (["ECU1", "HU", "TCU"][k / 3 % 3], ["E", "W", "I"][k / 9 % 3]);
                json!({"tag": ecu, "levels": [level]})
            }
        },
        (true, "rows-none") => match k % 2 {
            0 => {
                json!({"timeFrom": format!("03:{:02}:00", k % 60), "timeTo": format!("03:{:02}:59", k % 60)})
            }
            _ => {
                json!({"tag": format!("A{:02}", k % 40), "timeFrom": format!("04:{:02}:00", k % 60), "timeTo": format!("04:{:02}:59", k % 60)})
            }
        },
        (true, "rows-some") => match k % 2 {
            0 => {
                json!({"timeFrom": format!("09:30:{:02}", k % 60), "timeTo": format!("09:30:{:02}", k % 60)})
            }
            _ => {
                json!({"tag": format!("A{:02}", k % 40), "timeFrom": "09:30:00", "timeTo": format!("09:3{}:59", 1 + k % 8)})
            }
        },
        _ => unreachable!(),
    }
}
fn conditions(dlt: bool, set: &str, n: usize) -> Vec<Condition> {
    (0..n)
        .map(|k| serde_json::from_value(kind(dlt, set, k)).unwrap())
        .collect()
}
#[derive(Clone, Debug, PartialEq)]
enum Answer {
    Nothing,
    Matches,
    /// Undecided: blocks that may hold a match.
    Open(Vec<u32>),
}
struct Log {
    d: Arc<Dataset>,
    dlt: Option<Decoder>,
    blocks: Vec<Block>,
    levels: Arc<Vec<BlockLevels>>,
    fields: Arc<FieldIndex>,
    workers: usize,
}
impl Log {
    /// A filter showing the rows that pass one of `conds`.
    fn filter(&self, conds: &[Condition]) -> Filter {
        Filter::new(Filters {
            include: conds.to_vec(),
            ..Filters::default()
        })
        .unwrap()
        .with_format(self.d.format)
    }
    /// What the tables say of a filter's one showing condition, as the scan
    /// reads them: no row of a selected level, levels alone, field metadata.
    fn table(&self, f: &Filter) -> Answer {
        let variant = usize::from(f.fields.recognizes_line_numbers());
        let plan = self.fields.plan(f);
        let slots = f.level_slots();
        let mut open = vec![];
        for i in 0..self.blocks.len() {
            let Some(l) = self.levels.get(i) else {
                open.push(i as u32);
                continue;
            };
            let rows = &l[variant];
            let possible: u64 = (0..9)
                .filter(|s| slots & (1 << s) != 0)
                .map(|s| u64::from(rows.slots[s]))
                .sum();
            if possible == 0 {
                continue;
            }
            if f.only_levels() {
                return Answer::Matches;
            }
            match plan.as_ref().and_then(|p| p.decide(i, variant, rows)) {
                Some((n, _)) if n > 0 => return Answer::Matches,
                Some(_) => {}
                None => open.push(i as u32),
            }
        }
        if open.is_empty() {
            Answer::Nothing
        } else {
            Answer::Open(open)
        }
    }
    /// Whether an entry row of block `i` passes the filter's showing
    /// conditions; rows continuing an earlier block's entry do not.
    fn hit(&self, f: &Filter, i: usize, stop: &AtomicBool) -> bool {
        let b = self.blocks[i];
        let raw = self
            .d
            .source
            .read(b.start, (b.end - b.start) as usize)
            .unwrap();
        let bits = match &self.dlt {
            Some(dec) => f.dlt_mask(dec, &raw, b.rows, stop).unwrap().bits,
            None => f.mask_from(&raw, b.rows, stop, false, false).unwrap().bits,
        };
        // A stopped mask may be partial.
        !stop.load(Acquire) && (0..b.rows).any(|r| bits[r / 8] & (1 << (r % 8)) != 0)
    }
    /// Scans `open` blocks of a one-condition filter until a match.
    fn first(&self, f: &Filter, open: &[u32], read: &AtomicUsize) -> bool {
        let (next, found) = (AtomicUsize::new(0), AtomicBool::new(false));
        thread::scope(|s| {
            for _ in 0..self.workers {
                s.spawn(|| loop {
                    let k = next.fetch_add(1, Relaxed);
                    if found.load(Acquire) || k >= open.len() {
                        return;
                    }
                    read.fetch_add(1, Relaxed);
                    if self.hit(f, open[k] as usize, &found) {
                        found.store(true, Release);
                    }
                });
            }
        });
        found.into_inner()
    }
    /// One scan for open conditions `(index in conds, open blocks)` until
    /// each has a match; sets `stop` when all have one, and stops when
    /// another sets it. While `yield_to` runs, workers wait between blocks.
    /// Returns which have a match.
    fn joint(
        &self,
        conds: &[Condition],
        open: &[(usize, Vec<u32>)],
        stop: &AtomicBool,
        yield_to: Option<&Query>,
        read: &AtomicUsize,
    ) -> Vec<bool> {
        let mut blocks: Vec<u32> = open.iter().flat_map(|(_, b)| b.iter().copied()).collect();
        blocks.sort_unstable();
        blocks.dedup();
        let singles: Vec<Filter> = open
            .iter()
            .map(|(c, _)| self.filter(&conds[*c..=*c]))
            .collect();
        let found: Vec<AtomicBool> = open.iter().map(|_| AtomicBool::new(false)).collect();
        // Open conditions without a match yet, and the filter showing them.
        let together = |left: Vec<usize>| {
            let list: Vec<Condition> = left.iter().map(|&m| conds[open[m].0].clone()).collect();
            let f = (!list.is_empty()).then(|| self.filter(&list));
            Arc::new((left, f))
        };
        let group = Mutex::new(together((0..open.len()).collect()));
        let left = AtomicUsize::new(open.len());
        let next = AtomicUsize::new(0);
        if open.is_empty() {
            stop.store(true, Release);
        }
        thread::scope(|s| {
            for _ in 0..self.workers {
                s.spawn(|| loop {
                    if let Some(q) = yield_to {
                        while !stop.load(Acquire) && running(q) {
                            thread::park_timeout(Duration::from_millis(10));
                        }
                    }
                    let k = next.fetch_add(1, Relaxed);
                    if stop.load(Acquire) || k >= blocks.len() {
                        return;
                    }
                    let i = blocks[k];
                    let g = group.lock().unwrap().clone();
                    let here: Vec<usize> = g
                        .0
                        .iter()
                        .copied()
                        .filter(|&m| !found[m].load(Acquire) && open[m].1.binary_search(&i).is_ok())
                        .collect();
                    let Some(f) = g.1.as_ref().filter(|_| !here.is_empty()) else {
                        continue;
                    };
                    read.fetch_add(1, Relaxed);
                    if !self.hit(f, i as usize, stop) {
                        continue;
                    }
                    let mut changed = false;
                    for m in here {
                        // Workers may find the same condition at once.
                        if !found[m].load(Acquire)
                            && self.hit(&singles[m], i as usize, stop)
                            && !found[m].swap(true, AcqRel)
                        {
                            changed = true;
                            if left.fetch_sub(1, AcqRel) == 1 {
                                stop.store(true, Release);
                            }
                        }
                    }
                    if changed {
                        let mut g = group.lock().unwrap();
                        let rest =
                            g.0.iter()
                                .copied()
                                .filter(|&m| !found[m].load(Acquire))
                                .collect();
                        *g = together(rest);
                    }
                });
            }
        });
        found.iter().map(|f| f.load(Acquire)).collect()
    }
}
fn running(q: &Query) -> bool {
    q.json()["status"] == "running"
}
fn wait(f: impl Fn() -> bool) {
    while !f() {
        thread::sleep(Duration::from_micros(200));
    }
}
/// This process's resident set size from `ps`, and the bytes its malloc
/// zones hold in use (macOS `malloc_zone_statistics`).
#[cfg(target_os = "macos")]
fn rss() -> String {
    #[repr(C)]
    #[derive(Default)]
    struct Stats {
        blocks_in_use: u32,
        size_in_use: usize,
        max_size_in_use: usize,
        size_allocated: usize,
    }
    extern "C" {
        fn malloc_zone_statistics(zone: *mut std::ffi::c_void, stats: *mut Stats);
    }
    let mut s = Stats::default();
    unsafe { malloc_zone_statistics(std::ptr::null_mut(), &mut s) };
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .unwrap();
    format!(
        "rss {} KiB heap {} KiB",
        String::from_utf8_lossy(&out.stdout).trim(),
        s.size_in_use >> 10
    )
}
#[cfg(not(target_os = "macos"))]
fn rss() -> String {
    "rss unavailable (macOS only)".into()
}
fn summary(answers: &[Answer]) -> String {
    let n = |a: &Answer| {
        answers
            .iter()
            .filter(|b| std::mem::discriminant(*b) == std::mem::discriminant(a))
            .count()
    };
    format!(
        "none {}\tsome {}\topen {}",
        n(&Answer::Nothing),
        n(&Answer::Matches),
        n(&Answer::Open(vec![]))
    )
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let path = Path::new(&a[1]);
    let methods: Vec<&str> = a
        .get(2)
        .map_or("table,first,joint,count,rows,beside", |s| s.as_str())
        .split(',')
        .collect();
    let rounds: usize = a.get(3).map_or(2, |r| r.parse().unwrap());
    let label = path.file_name().unwrap().to_string_lossy().into_owned();
    let d = Dataset::with_dlt_clock(1, Source::granted(path).unwrap(), Clock::Fixed(0));
    let dlt = d.format == Format::Dlt;
    let t = Instant::now();
    d.start(|_| {});
    wait(|| d.index.lock().unwrap().done);
    let eof = t.elapsed().as_secs_f64();
    wait(|| d.index.lock().unwrap().block_fields.is_some());
    let (blocks, levels, fields) = {
        let i = d.index.lock().unwrap();
        (
            i.blocks.clone(),
            i.block_levels.clone().unwrap(),
            i.block_fields.clone().unwrap(),
        )
    };
    println!(
        "{label}\tindex {eof:.3}\tanalysis {:.3}\trows {}\tblocks {}\ttable bytes {:?} kept {:?}\t{}",
        t.elapsed().as_secs_f64() - eof,
        d.index.lock().unwrap().total,
        blocks.len(),
        fields.bytes(),
        fields.kept(),
        rss()
    );
    let max = if dlt { 8 } else { 4 };
    let log = Log {
        dlt: dlt.then(|| Decoder::new(Clock::Fixed(0))),
        d: d.clone(),
        blocks,
        levels,
        fields,
        workers: thread::available_parallelism()
            .map_or(1, |n| n.get() / 2)
            .clamp(1, max),
    };
    let ids = AtomicU64::new(0);
    let id = || ids.fetch_add(1, Relaxed) + 1;
    let search = ["table", "first", "joint", "count"];
    for round in 0..rounds {
        if !methods.iter().any(|m| search.contains(m)) {
            break;
        }
        for set in SETS {
            for n in [1, 8, 32] {
                let conds = conditions(dlt, set, n);
                let mut seen: Vec<(&str, Vec<bool>)> = vec![];
                let mut tables = None;
                // Each round runs the methods in another order.
                let mut order: Vec<&str> = search
                    .iter()
                    .copied()
                    .filter(|m| methods.contains(m))
                    .collect();
                if round % 2 == 1 {
                    order.reverse();
                }
                for m in order {
                    let read = AtomicUsize::new(0);
                    let t = Instant::now();
                    let (answers, extra): (Vec<Answer>, String) = match m {
                        "table" => {
                            let a: Vec<Answer> = conds
                                .iter()
                                .map(|c| log.table(&log.filter(std::slice::from_ref(c))))
                                .collect();
                            (a, String::new())
                        }
                        "first" => {
                            let a = conds
                                .iter()
                                .map(|c| {
                                    let f = log.filter(std::slice::from_ref(c));
                                    match log.table(&f) {
                                        Answer::Open(b) if log.first(&f, &b, &read) => {
                                            Answer::Matches
                                        }
                                        Answer::Open(_) => Answer::Nothing,
                                        a => a,
                                    }
                                })
                                .collect();
                            (a, String::new())
                        }
                        "joint" => {
                            let mut a: Vec<Answer> = conds
                                .iter()
                                .map(|c| log.table(&log.filter(std::slice::from_ref(c))))
                                .collect();
                            let decided = t.elapsed().as_secs_f64();
                            let open: Vec<(usize, Vec<u32>)> = a
                                .iter()
                                .enumerate()
                                .filter_map(|(k, x)| match x {
                                    Answer::Open(b) => Some((k, b.clone())),
                                    _ => None,
                                })
                                .collect();
                            let union = {
                                let mut u: Vec<u32> =
                                    open.iter().flat_map(|(_, b)| b.iter().copied()).collect();
                                u.sort_unstable();
                                u.dedup();
                                u.len()
                            };
                            let stop = AtomicBool::new(false);
                            let found = log.joint(&conds, &open, &stop, None, &read);
                            for ((k, _), f) in open.iter().zip(found) {
                                a[*k] = if f { Answer::Matches } else { Answer::Nothing };
                            }
                            (a, format!("\ttables {decided:.4}\topen blocks {union}"))
                        }
                        _ => {
                            if n > 8 {
                                continue;
                            }
                            let a = conds
                                .iter()
                                .map(|c| {
                                    let q = d
                                        .query(1, id(), log.filter(std::slice::from_ref(c)))
                                        .unwrap();
                                    wait(|| !running(&q));
                                    assert_eq!(q.json()["status"], "complete");
                                    if q.json()["count"].as_u64().unwrap() > 0 {
                                        Answer::Matches
                                    } else {
                                        Answer::Nothing
                                    }
                                })
                                .collect();
                            (a, String::new())
                        }
                    };
                    let took = t.elapsed().as_secs_f64();
                    println!(
                        "{label}\t{set}\t{n}\t{m}\t{round}\t{took:.4}\t{}\tread {}{extra}",
                        summary(&answers),
                        read.load(Relaxed)
                    );
                    if m == "table" {
                        tables = Some(answers);
                    } else {
                        seen.push((m, answers.iter().map(|x| *x == Answer::Matches).collect()));
                    }
                }
                let equal = seen.windows(2).all(|w| w[0].1 == w[1].1)
                    && tables.as_ref().is_none_or(|t| {
                        seen.iter().all(|(_, s)| {
                            t.iter().zip(s).all(|(a, &m)| match a {
                                Answer::Open(_) => true,
                                a => (*a == Answer::Matches) == m,
                            })
                        })
                    });
                let some = seen.first().map(|(_, s)| s.iter().filter(|&&m| m).count());
                println!("{label}\t{set}\t{n}\tcheck\t{round}\t\tequal {equal}\tmatching {some:?}");
                if !equal {
                    for (k, c) in conds.iter().enumerate() {
                        let each: Vec<_> = seen.iter().map(|(m, s)| (*m, s[k])).collect();
                        println!("{label}\t{set}\t{n}\tcondition {k}\t{c:?}\t{each:?}");
                    }
                }
                assert!(equal, "{set} {n}");
            }
        }
    }
    if methods.contains(&"rows") && !dlt {
        rows(&log);
    }
    if methods.contains(&"overhead") {
        overhead(&log, dlt, &label, rounds);
    }
    if methods.contains(&"engine") {
        d.close();
        engine(path, dlt, &label);
        println!("{label}\tend\t{}", rss());
        return;
    }
    if methods.contains(&"beside") {
        beside(&log, dlt, rounds, &id);
    }
    println!("{label}\tend\t{}", rss());
    d.close();
}
/// The field decision per parsed row of the first 64 MiB, one thread, best
/// of 5: `Filter::fields_match`, the same short-circuit loop over the
/// conditions, and the loop that also learns each condition's own match.
fn rows(log: &Log) {
    let texts: Vec<String> = log
        .blocks
        .iter()
        .take(512)
        .map(|b| {
            encoding::decode(
                &log.d
                    .source
                    .read(b.start, (b.end - b.start) as usize)
                    .unwrap(),
            )
            .into_owned()
        })
        .collect();
    let mut records = vec![];
    for t in &texts {
        for (offset, bytes) in ByteLines::new(t.as_bytes()) {
            let r = parser::parse_in(Format::Text, &t[offset..offset + bytes.len()], 1, true);
            if r.parsed.is_some() {
                records.push(r);
            }
        }
    }
    let parsed: Vec<&parser::Parsed> = records.iter().map(|r| r.parsed.as_ref().unwrap()).collect();
    println!("rows\tparsed rows\t{}", parsed.len());
    for n in [1, 8, 32] {
        let none = conditions(false, "table-none", n);
        let some = conditions(false, "table-some", n);
        let rows_none = conditions(false, "rows-none", n);
        let one_some: Vec<Condition> = some[..1].iter().chain(&none[1..]).cloned().collect();
        let half = n.div_ceil(2);
        let cases: Vec<(&str, Vec<Condition>, Vec<Condition>)> = vec![
            ("hide none", vec![], none.clone()),
            ("hide none (time, 2 fields)", vec![], rows_none.clone()),
            ("show some", some.clone(), vec![]),
            ("show 1 some + rest none", one_some, vec![]),
            (
                "show some + hide none",
                some[..half].to_vec(),
                none[..n - half].to_vec(),
            ),
        ];
        for (name, include, exclude) in cases {
            let f = Filter::new(Filters {
                include,
                exclude,
                ..Filters::default()
            })
            .unwrap();
            let (shows, hides) = (f.shows(), f.hides());
            let best = |run: &dyn Fn() -> (usize, Vec<bool>)| {
                let (mut best, mut out) = (f64::MAX, (0, vec![]));
                for _ in 0..5 {
                    let t = Instant::now();
                    out = run();
                    best = best.min(t.elapsed().as_secs_f64());
                }
                (best * 1e9 / parsed.len() as f64, out)
            };
            let (core, (c0, _)) =
                best(&|| (records.iter().filter(|r| f.fields_match(r)).count(), vec![]));
            let (short, (c1, _)) = best(&|| {
                let pass = |p: &parser::Parsed| {
                    (shows.is_empty() || shows.iter().any(|c| c.matches(p)))
                        && !hides.iter().any(|c| c.matches(p))
                };
                (parsed.iter().filter(|p| pass(p)).count(), vec![])
            });
            let (bits, (c2, found)) = best(&|| {
                let mut found = vec![false; shows.len() + hides.len()];
                let mut count = 0;
                for p in &parsed {
                    let mut shown = shows.is_empty();
                    for (k, c) in shows.iter().enumerate() {
                        if (shown && found[k]) || !c.matches(p) {
                            continue;
                        }
                        (shown, found[k]) = (true, true);
                    }
                    let mut hidden = false;
                    for (k, c) in hides.iter().enumerate() {
                        let k = shows.len() + k;
                        if (found[k] && (hidden || !shown)) || !c.matches(p) {
                            continue;
                        }
                        (hidden, found[k]) = (true, true);
                    }
                    count += usize::from(shown && !hidden);
                }
                (count, found)
            });
            assert!(c0 == c1 && c1 == c2, "{name}");
            println!(
                "rows\t{name}\t{n}\tfields_match {core:.1} ns/row\tshort-circuit {short:.1}\twith bits {bits:.1}\tpassed {c0}\tconditions matched {}/{}",
                found.iter().filter(|&&x| x).count(),
                found.len()
            );
        }
    }
    // The whole per-block pass of a query that reads every block, for scale.
    let f = log.filter(&conditions(false, "mixed", 32));
    let stop = AtomicBool::new(false);
    let mut best = f64::MAX;
    for _ in 0..3 {
        let t = Instant::now();
        for b in log.blocks.iter().take(512) {
            let raw = log
                .d
                .source
                .read(b.start, (b.end - b.start) as usize)
                .unwrap();
            let _ = f.mask_from(&raw, b.rows, &stop, true, false).unwrap();
        }
        best = best.min(t.elapsed().as_secs_f64());
    }
    let total: usize = log.blocks.iter().take(512).map(|b| b.rows).sum();
    println!(
        "rows\tblock pass (read, decode, parse, decide), 32 mixed\t{:.1} ns/row",
        best * 1e9 / total as f64
    );
}
/// A query that reads every block alone and beside a joint scan, and a
/// joint scan's cancellation.
fn beside(log: &Log, dlt: bool, rounds: usize, id: &dyn Fn() -> u64) {
    let query: Filters = serde_json::from_value(if dlt {
        json!({"include":[{"timeFrom":"09:30:00","timeTo":"10:30:00"}]})
    } else {
        json!({"include":[{"tag":"GalleryManager"},{"pid":"123"}]})
    })
    .unwrap();
    let conds = conditions(dlt, "rows-none", 32);
    let open: Vec<(usize, Vec<u32>)> = conds
        .iter()
        .enumerate()
        .filter_map(
            |(k, c)| match log.table(&log.filter(std::slice::from_ref(c))) {
                Answer::Open(b) => Some((k, b)),
                _ => None,
            },
        )
        .collect();
    let run_query = || {
        let t = Instant::now();
        let q = log
            .d
            .query(0, id(), Filter::new(query.clone()).unwrap())
            .unwrap();
        let mut first = None;
        while running(&q) {
            if first.is_none() && q.json()["count"].as_u64().unwrap() > 0 {
                let rows = log.d.page(&q, 0, 200).unwrap()["rows"]
                    .as_array()
                    .unwrap()
                    .len();
                if rows > 0 {
                    first = Some(t.elapsed().as_secs_f64());
                }
            }
            thread::sleep(Duration::from_micros(200));
        }
        assert_eq!(q.json()["status"], "complete");
        (q, first.unwrap_or(f64::NAN), t.elapsed().as_secs_f64())
    };
    let joint = |yield_to: Option<&Query>| {
        let (stop, read) = (AtomicBool::new(false), AtomicUsize::new(0));
        let t = Instant::now();
        let found = log.joint(&conds, &open, &stop, yield_to, &read);
        assert!(found.iter().all(|f| !f));
        t.elapsed().as_secs_f64()
    };
    for round in 0..rounds {
        let (q, first, done) = run_query();
        let count = q.json()["count"].as_u64().unwrap();
        println!("beside\t{round}\tquery alone\tfirst rows {first:.4}\tcomplete {done:.4}\tcount {count}");
        println!(
            "beside\t{round}\tjoint alone ({} open)\t{:.4}",
            open.len(),
            joint(None)
        );
        for yields in [false, true] {
            let slot: Mutex<Option<Arc<Query>>> = Mutex::new(None);
            let (mut first, mut done, mut scan) = (0.0, 0.0, 0.0);
            let t = Instant::now();
            thread::scope(|s| {
                let h = s.spawn(|| {
                    if yields {
                        // As the analysis does: wait for the pane's query to exist.
                        wait(|| slot.lock().unwrap().is_some());
                        let q = slot.lock().unwrap().clone().unwrap();
                        joint(Some(&q));
                    } else {
                        joint(None);
                    }
                    t.elapsed().as_secs_f64()
                });
                let started = Instant::now();
                let q = log
                    .d
                    .query(0, id(), Filter::new(query.clone()).unwrap())
                    .unwrap();
                *slot.lock().unwrap() = Some(q.clone());
                let mut f = None;
                while running(&q) {
                    if f.is_none()
                        && q.json()["count"].as_u64().unwrap() > 0
                        && !log.d.page(&q, 0, 200).unwrap()["rows"]
                            .as_array()
                            .unwrap()
                            .is_empty()
                    {
                        f = Some(started.elapsed().as_secs_f64());
                    }
                    thread::sleep(Duration::from_micros(200));
                }
                (first, done) = (f.unwrap_or(f64::NAN), started.elapsed().as_secs_f64());
                scan = h.join().unwrap();
            });
            println!(
                "beside\t{round}\twith joint{}\tfirst rows {first:.4}\tcomplete {done:.4}\tjoint ends {scan:.4}",
                if yields { " yielding" } else { "" }
            );
        }
        let (stop, read) = (AtomicBool::new(false), AtomicUsize::new(0));
        let mut latency = 0.0;
        thread::scope(|s| {
            let h = s.spawn(|| {
                log.joint(&conds, &open, &stop, None, &read);
                Instant::now()
            });
            thread::sleep(Duration::from_millis(100));
            let t = Instant::now();
            stop.store(true, Release);
            latency = h.join().unwrap().duration_since(t).as_secs_f64();
        });
        println!(
            "beside\t{round}\tjoint cancel after 100 ms\t{:.2} ms\tread {}",
            latency * 1e3,
            read.load(Relaxed)
        );
    }
}
/// The logs pane query of a filter on a fresh dataset: first rows, its
/// completion and when its check knows every condition, in seconds.
fn engine(path: &Path, dlt: bool, label: &str) {
    for set in SETS {
        for n in [1, 8, 32] {
            let d = Dataset::with_dlt_clock(1, Source::granted(path).unwrap(), Clock::Fixed(0));
            d.start(|_| {});
            wait(|| d.index.lock().unwrap().block_fields.is_some());
            let run = |id: u64, filters: &Filters| {
                let t = Instant::now();
                let q = d
                    .query(0, id, Filter::new(filters.clone()).unwrap())
                    .unwrap();
                let (mut first, mut done, mut known) = (None, None, None);
                loop {
                    let v = q.json();
                    let at = t.elapsed().as_secs_f64();
                    if first.is_none()
                        && v["count"].as_u64().unwrap() > 0
                        && !d.page(&q, 0, 200).unwrap()["rows"]
                            .as_array()
                            .unwrap()
                            .is_empty()
                    {
                        first = Some(at);
                    }
                    if done.is_none() && v["status"] != "running" {
                        assert_eq!(v["status"], "complete");
                        done = Some(at);
                    }
                    let states = v.get("conditions").map(|c| {
                        let all: Vec<&Value> = ["include", "exclude"]
                            .iter()
                            .flat_map(|l| c[*l].as_array().unwrap())
                            .collect();
                        (
                            all.iter().all(|s| *s != "pending"),
                            all.iter().filter(|s| **s == "none").count(),
                        )
                    });
                    if known.is_none() && states.is_some_and(|(all, _)| all) {
                        known = Some((at, states.unwrap().1));
                    }
                    if done.is_some() && (known.is_some() || states.is_none()) {
                        break;
                    }
                    thread::sleep(Duration::from_micros(200));
                }
                let s = |x: Option<f64>| x.map_or("-".to_string(), |x| format!("{x:.4}"));
                format!(
                    "first rows {}\tcomplete {}\tknown {}\tnone {}",
                    s(first),
                    s(done),
                    s(known.map(|k| k.0)),
                    known.map_or("-".to_string(), |k| k.1.to_string())
                )
            };
            let filters = Filters {
                exclude: conditions(dlt, set, n),
                ..Filters::default()
            };
            let mut changed = filters.clone();
            changed.exclude[n - 1] =
                serde_json::from_value(json!({"tag": format!("FreshTag{n}")})).unwrap();
            for (id, name, f) in [
                (1, "new", &filters),
                (2, "again", &filters),
                (3, "one changed", &changed),
            ] {
                println!("{label}\t{set}\t{n}\tengine {name}\t{}", run(id, f));
            }
            // A query replacing one whose check still reads blocks.
            let fresh = Filters {
                exclude: filters
                    .exclude
                    .iter()
                    .map(|c| Condition {
                        time_to: "23:59:58".into(),
                        ..c.clone()
                    })
                    .collect(),
                ..Filters::default()
            };
            let q = d.query(0, 4, Filter::new(fresh).unwrap()).unwrap();
            wait(|| !running(&q));
            thread::sleep(Duration::from_millis(20));
            let pending = q.json()["conditions"]["exclude"]
                .as_array()
                .map_or(0, |a| a.iter().filter(|s| *s == "pending").count());
            let t = Instant::now();
            let other = Filters {
                exclude: changed.exclude.clone(),
                ..Filters::default()
            };
            d.query(0, 5, Filter::new(other).unwrap()).unwrap();
            println!(
                "{label}\t{set}\t{n}\tengine replace\t{:.3} ms\tpending before {pending}",
                t.elapsed().as_secs_f64() * 1e3
            );
            d.close();
        }
    }
}
/// Completion of queries with and without a check beside them.
fn overhead(log: &Log, dlt: bool, label: &str, rounds: usize) {
    let mut id = 1_000_000;
    for n in [1, 8, 32] {
        let (mut checked, mut alone) = (vec![], vec![]);
        for round in 0..rounds * 4 {
            // Fresh conditions, so that the check has work every time.
            let exclude: Vec<Condition> = conditions(dlt, "rows-none", n)
                .into_iter()
                .map(|c| Condition {
                    time_to: format!("23:59:{:02}", round % 60),
                    ..c
                })
                .collect();
            let f = Filters {
                exclude,
                ..Filters::default()
            };
            for pane in [round % 2, 1 - round % 2] {
                id += 1;
                let t = Instant::now();
                let q = log
                    .d
                    .query(pane, id, Filter::new(f.clone()).unwrap())
                    .unwrap();
                wait(|| !running(&q));
                let took = t.elapsed().as_secs_f64();
                if pane == 0 {
                    checked.push(took);
                    wait(|| {
                        let c = &q.json()["conditions"]["exclude"];
                        c.as_array()
                            .is_none_or(|a| a.iter().all(|s| s != "pending"))
                    });
                } else {
                    alone.push(took);
                }
            }
        }
        let range = |v: &mut Vec<f64>| {
            v.sort_by(|a, b| a.partial_cmp(b).unwrap());
            format!(
                "{:.4}-{:.4} median {:.4}",
                v[0],
                v[v.len() - 1],
                v[v.len() / 2]
            )
        };
        println!(
            "{label}\toverhead\t{n}\tchecked {}\tunchecked {}",
            range(&mut checked),
            range(&mut alone)
        );
    }
}
