use logstudio_core::{
    dataset::Dataset,
    search::{Filter, Filters},
    source::Source,
};
use std::{
    sync::{atomic::AtomicBool, atomic::Ordering},
    thread,
    time::{Duration, Instant},
};
fn wait(mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !f() {
        assert!(Instant::now() < deadline, "timed out");
        thread::sleep(Duration::from_millis(2));
    }
}
// Several 128 KiB blocks of mixed Android/iOS/numbered/raw rows and newlines.
fn mixed() -> String {
    let levels = ["INFO", "warn", "ınfo", "aſſert", "Notice", "E"];
    let breaks = ["\n", "\r\n", "\r"];
    (0..30_000)
        .map(|i| {
            let row = match i % 7 {
                0 => format!(
                    "09-21 12:07:30.386 {} 9 {} Tag{}: m {i}",
                    100 + i % 3,
                    ["V", "D", "I", "W", "E", "F", "A"][i / 7 % 7],
                    i % 4
                ),
                1 => format!("{i:05}\t09-21 12:07:30.386 101 9 W Num: numbered"),
                2 => format!(
                    "2026-09-30 21:55:53.776 Runner[101:9] [{}] [T] [c{}] [T] body",
                    levels[i % 6],
                    i % 3
                ),
                3 => format!("2026-09-30 21:55:53.776 Runner[102:9] plain {i}"),
                4 => String::new(),
                5 => format!("raw text {i}"),
                _ => format!("09-21 12:07:30.386 102 8 {} Tag: m", ["E", "I"][i % 2]),
            };
            row + breaks[i / 7 % 3]
        })
        .collect()
}
fn filters() -> Vec<Filters> {
    let mut out = vec![];
    for mode in [None, Some(false)] {
        for (levels, raw, pid) in [
            (vec!["E"], true, ""),
            (vec!["I", "W"], true, ""),
            (vec!["U"], true, ""),
            (vec!["V", "D", "I", "W", "E", "F", "A", "U"], false, ""),
            (vec!["A"], true, "101"),
            (vec!["F"], true, "999"),
        ] {
            out.push(Filters {
                levels: levels.into_iter().map(String::from).collect(),
                include_raw: raw,
                pid: pid.into(),
                recognize_line_numbers: mode,
                ..Filters::default()
            });
        }
    }
    out
}
fn counts(d: &std::sync::Arc<Dataset>, first_id: u64) -> Vec<(u64, String)> {
    filters()
        .into_iter()
        .enumerate()
        .map(|(i, f)| {
            let q = d
                .query(1, first_id + i as u64, Filter::new(f).unwrap())
                .unwrap();
            wait(|| q.json()["status"] == "complete");
            let page = d.page(&q, 3, 50).unwrap()["rows"].to_string();
            (q.json()["count"].as_u64().unwrap(), page)
        })
        .collect()
}
#[test]
fn analysis_level_table_matches_parsed_filter_counts() {
    let d = Dataset::new(1, Source::text(&mixed(), "mixed").unwrap());
    // Hold analysis so the first pass parses every row.
    d.pause_analysis.store(true, Ordering::Release);
    d.start(|_| {});
    wait(|| d.index.lock().unwrap().done);
    assert!(d.index.lock().unwrap().blocks.len() > 3);
    let parsed = counts(&d, 1);
    d.resume();
    wait(|| d.index.lock().unwrap().block_levels.is_some());
    let index = d.index.lock().unwrap();
    let table = index.block_levels.clone().unwrap();
    assert_eq!(table.len(), index.blocks.len());
    for (levels, block) in table.iter().zip(&index.blocks) {
        for variant in levels {
            assert_eq!(
                variant.slots.iter().map(|&n| n as usize).sum::<usize>(),
                block.rows
            );
        }
    }
    drop(index);
    assert_eq!(counts(&d, 100), parsed);
    d.close();
}
#[test]
fn parallel_scan_records_every_block_in_order() {
    let d = Dataset::new(1, Source::text(&mixed(), "mixed").unwrap());
    d.start(|_| {});
    let filter = Filters {
        tag: "c1".into(),
        ..Filters::default()
    };
    // Starts while indexing may still publish blocks.
    let q = d.query(1, 1, Filter::new(filter).unwrap()).unwrap();
    wait(|| q.json()["status"] == "complete");
    let blocks = d.index.lock().unwrap().blocks.clone();
    let (ends, entries) = {
        let r = q.result.lock().unwrap();
        (r.ends.clone(), r.entries.clone())
    };
    assert_eq!(ends.len(), blocks.len());
    assert_eq!(entries.len(), blocks.len());
    let stop = AtomicBool::new(false);
    let mut total = 0;
    for ((b, end), entry) in blocks.iter().zip(ends).zip(entries) {
        let data = d.source.read(b.start, (b.end - b.start) as usize).unwrap();
        let mask = q
            .filter
            .mask_from(&data, b.rows, &stop, entry)
            .unwrap()
            .bits;
        // Bits past the last row are padding.
        total += (0..b.rows)
            .filter(|i| mask[i / 8] & (1 << (i % 8)) != 0)
            .count() as u64;
        assert_eq!(end, total);
    }
    assert_eq!(q.json()["count"], total);
    assert!(total > 0);
    d.close();
}
#[test]
fn regex_rows_follow_cr_crlf_and_lf_boundaries() {
    let filter = Filter::new(Filters {
        query: "^b$".into(),
        search_mode: "regex".into(),
        ..Filters::default()
    })
    .unwrap();
    let data = b"a\r\nb\rb\nc\r\rb\r\n";
    let mask = filter.mask(data, 6, &AtomicBool::new(false)).unwrap();
    assert_eq!(mask, vec![0b10_0110]);
}
