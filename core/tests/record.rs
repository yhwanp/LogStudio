//! `record` returns the whole log entry a physical row belongs to: the
//! parsed entry row and the rows that continue it, in bounded windows.
use logstudio_core::{
    dataset::{Dataset, Query, RECORD_BYTES},
    encoding, parser,
    search::{Filter, Filters},
    source::{ByteLines, Source},
};
use serde_json::Value;
use std::{
    path::PathBuf,
    sync::{atomic::Ordering, Arc},
    thread,
    time::{Duration, Instant},
};

fn wait(mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !done() {
        assert!(Instant::now() < deadline, "timed out");
        thread::sleep(Duration::from_millis(1));
    }
}
fn open(source: Source) -> Arc<Dataset> {
    let d = Dataset::new(1, source);
    d.start(|_| {});
    wait(|| d.status(0)["complete"] == true);
    d
}
fn query(d: &Arc<Dataset>, id: u64, filters: Filters) -> Arc<Query> {
    let q = d.query(0, id, Filter::new(filters).unwrap()).unwrap();
    wait(|| q.json()["status"] == "complete");
    q
}
/// The whole record text, read window by window like the detail view.
fn read(d: &Dataset, q: &Query, line: u64) -> (Value, String) {
    let (mut text, mut start, mut first) = (String::new(), 0, None);
    loop {
        let w = d.record(q, line, start).unwrap();
        assert_eq!(w["lineNumber"], line);
        assert_eq!(w["startByte"], start as u64);
        assert!(w["nextByte"].as_u64().unwrap() as usize - start <= 65536);
        text += w["text"].as_str().unwrap();
        first.get_or_insert(w.clone());
        if w["complete"] == true {
            assert_eq!(w["nextByte"], w["totalBytes"]);
            return (first.unwrap(), text);
        }
        start = w["nextByte"].as_u64().unwrap() as usize;
    }
}
fn span(w: &Value) -> (u64, u64) {
    (w["first"].as_u64().unwrap(), w["last"].as_u64().unwrap())
}

/// Each row's record, one physical row at a time: a parsed row opens an
/// entry that the following rows without fields continue in the same file;
/// a malformed stamped row and the rows before a file's first entry are
/// their own records.
fn oracle(files: &[&[u8]], recognize: bool) -> Vec<(u64, u64)> {
    let mut out: Vec<(u64, u64)> = vec![];
    let mut line = 0u64;
    for data in files {
        let mut entry: Option<usize> = None;
        for (_, bytes) in ByteLines::new(data) {
            line += 1;
            let raw = encoding::decode(bytes);
            let parsed = parser::parse_line(&raw, line, recognize).parsed.is_some();
            if parsed {
                entry = Some(out.len());
                out.push((line, line));
            } else if parser::starts_entry(&raw, recognize) {
                entry = None;
                out.push((line, line));
            } else if let Some(e) = entry {
                let first = out[e].0;
                for r in &mut out[e..] {
                    r.1 = line;
                }
                out.push((first, line));
            } else {
                out.push((line, line));
            }
        }
    }
    out
}
fn rows_text(data: &[u8], first: u64, last: u64) -> String {
    ByteLines::new(data)
        .skip(first as usize - 1)
        .take((last - first + 1) as usize)
        .map(|(_, b)| encoding::decode(b).into_owned())
        .collect::<Vec<_>>()
        .join("\n")
}

const SAMPLE: &str = "preamble before any log
2026-09-30 21:55:53.776261 [INFO] [AppDelegate] launched
2026-09-30 21:55:54.351980 [DEBUG] [AlertsService] [\r
  {\r
    \"body\": \"안내문구3\",\r
    \"snooze_threshold\": \"today\"\r
  }\r
]
2026-09-30 21:55:55.000001 [ERROR] [Network] failed
2026-09-30 21:55:55.100 Runner[3870:bad] malformed
  detail of the malformed row
";

#[test]
fn a_row_shows_its_whole_entry() {
    let d = open(Source::text(SAMPLE, "sample").unwrap());
    let q = query(
        &d,
        1,
        Filters {
            query: "안내문구3".into(),
            ..Filters::default()
        },
    );
    let body = rows_text(SAMPLE.as_bytes(), 3, 8);
    for line in 3..=8 {
        let (w, text) = read(&d, &q, line);
        assert_eq!(span(&w), (3, 8), "{line}");
        assert_eq!(w["recordComplete"], true);
        assert_eq!(text, body);
        // Rows are joined by LF; CRLF stays a source detail.
        assert!(!text.contains('\r'));
        assert_eq!(w["totalBytes"], body.len() as u64);
        let start = SAMPLE.find("2026-09-30 21:55:54").unwrap();
        assert_eq!(w["sourceStartByte"], start as u64);
        // The match is highlighted in its own row, in UTF-16 units.
        let at = body.find("안내문구3").unwrap();
        let units = body[..at].encode_utf16().count();
        assert_eq!(w["highlights"], serde_json::json!([[units, units + 5]]));
    }
    // A single-row entry, an orphan, a malformed stamped row and the row
    // that continues it are each their own record.
    for (line, expected) in [
        (2, (2, 2)),
        (9, (9, 9)),
        (1, (1, 1)),
        (10, (10, 10)),
        (11, (11, 11)),
    ] {
        let (w, text) = read(&d, &q, line);
        assert_eq!(span(&w), expected, "{line}");
        assert_eq!(text, rows_text(SAMPLE.as_bytes(), line, line));
    }
    assert!(d.record(&q, 3, body.len() + 1).is_err());
    d.close();
}

/// iOS JSON bodies (one longer than three 128 KiB blocks), Android stack
/// traces, malformed stamped rows, numbered rows, CR/LF/CRLF and Unicode.
fn workload() -> String {
    let breaks = ["\n", "\r\n", "\r"];
    let mut out = String::from("orphan 1\n\norphan 3\r\n");
    for i in 0..14_000usize {
        let br = breaks[i % 3];
        match i % 5 {
            0 => {
                let body = if i == 3_000 { 9_000 } else { i % 5 + i % 7 };
                out += &format!(
                    "2026-09-30 21:55:{:02}.{i:06} [DEBUG] [Notice] [{br}",
                    i % 60
                );
                for k in 0..body {
                    out += &format!("    \"key{k}\": \"안내문구{i} 🙂\",{br}");
                }
                out += &format!("]{br}");
            }
            1 => {
                out += &format!("09-21 12:07:30.386  101 9 E AndroidRuntime: crash {i}{br}");
                for k in 0..i % 4 {
                    out += &format!("\tat com.example.C{k}.run(C.java:{i}){br}");
                }
            }
            2 => {
                out +=
                    &format!("2026-09-30 21:55:53.776 Runner[101:bad] malformed {i}{br}  tail{br}")
            }
            3 => out += &format!("{i:05}\t09-21 12:07:30.386 101 9 W Num: numbered{br}{br}"),
            _ => out += &format!("09-21 12:07:30.386 102 8 I Tag: m{br}plain continuation {i}{br}"),
        }
    }
    out
}

#[test]
fn records_span_blocks_before_and_after_analysis() {
    let text = workload();
    let d = Dataset::new(1, Source::text(&text, "workload").unwrap());
    d.pause_analysis.store(true, Ordering::Release);
    d.start(|_| {});
    wait(|| d.index.lock().unwrap().done);
    assert!(d.index.lock().unwrap().blocks.len() > 8);
    let mut id = 0;
    for analysed in [false, true] {
        if analysed {
            d.resume();
            wait(|| d.index.lock().unwrap().block_levels.is_some());
        }
        for recognize in [None, Some(false)] {
            id += 1;
            let q = query(
                &d,
                id,
                Filters {
                    recognize_line_numbers: recognize,
                    ..Filters::default()
                },
            );
            let expected = oracle(&[text.as_bytes()], recognize.unwrap_or(true));
            let long = expected.iter().map(|r| r.1 - r.0).max().unwrap();
            assert!(long > 9_000, "{long}");
            for (i, want) in expected.iter().enumerate().step_by(149) {
                let line = i as u64 + 1;
                let w = d.record(&q, line, 0).unwrap();
                assert_eq!(span(&w), *want, "line {line} analysed {analysed}");
                assert_eq!(w["recordComplete"], true);
            }
            // The longest entry, read in windows from a row in its middle.
            let first = expected.iter().find(|r| r.1 - r.0 == long).unwrap().0;
            let (w, all) = read(&d, &q, first + long / 2);
            assert_eq!(span(&w), (first, first + long));
            assert_eq!(all, rows_text(text.as_bytes(), first, first + long));
        }
    }
    d.close();
}

#[test]
fn a_long_entry_keeps_its_first_mebibyte() {
    let mut text = String::from("09-21 12:07:30.386 1 2 E Tag: start\n");
    let row = format!("\tat {}\n", "x".repeat(250));
    let rows = 2 * RECORD_BYTES / row.len();
    for _ in 0..rows {
        text += &row;
    }
    text += "09-21 12:07:31.000 1 2 I Tag: next\n";
    let d = open(Source::text(&text, "long").unwrap());
    let q = query(&d, 1, Filters::default());
    let (w, all) = read(&d, &q, 2);
    let (first, last) = span(&w);
    assert_eq!(first, 1);
    assert_eq!(w["recordComplete"], false);
    assert!(all.len() <= RECORD_BYTES && all.len() + row.len() > RECORD_BYTES);
    assert_eq!(all, rows_text(text.as_bytes(), 1, last));
    // A row past the limit shows on its own.
    let beyond = rows as u64;
    let w = d.record(&q, beyond, 0).unwrap();
    assert_eq!(span(&w), (beyond, beyond));
    assert_eq!(w["recordComplete"], true);
    d.close();
}

#[test]
fn records_stay_in_their_file_and_map_damage_to_source_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let files: [&[u8]; 2] = [
        b"2026-09-30 21:55:54.351980 [DEBUG] [AlertsService] [\r\n  {\"a\": \"\xff\"}\r\n",
        b"  {\"b\": 2}\n]\n2026-09-30 21:55:55.000 [ERROR] [Network] x\n  stack\n",
    ];
    let paths: Vec<PathBuf> = files
        .iter()
        .enumerate()
        .map(|(i, bytes)| {
            let path = dir.path().join(format!("{i}.log"));
            std::fs::write(&path, bytes).unwrap();
            path
        })
        .collect();
    let d = open(Source::granted_files(&paths).unwrap());
    let q = query(&d, 1, Filters::default());
    let (w, text) = read(&d, &q, 2);
    assert_eq!(span(&w), (1, 2));
    assert_eq!(w["file"], 0);
    assert_eq!(w["fileLine"], 1);
    assert!(text.ends_with("{\"a\": \"\u{fffd}\"}"));
    let sample = &w["encodingDamage"]["samples"][0];
    let at = files[0].iter().position(|&b| b == 0xff).unwrap();
    assert_eq!(sample["sourceStartByte"], at as u64);
    assert_eq!(sample["hex"], "FF");
    // The next file's leading rows are orphans; its entry is its own.
    assert_eq!(span(&d.record(&q, 3, 0).unwrap()), (3, 3));
    let w = d.record(&q, 6, 0).unwrap();
    assert_eq!(span(&w), (5, 6));
    assert_eq!(w["file"], 1);
    assert_eq!(w["fileLine"], 3);
    assert_eq!(oracle(&files, true)[5], (5, 6));
    d.close();
}
