//! Rows without fields continue the log entry before them: field filters
//! decide them by that entry, also across block and analysis boundaries.
use logstudio_core::{
    dataset::{Dataset, Query},
    encoding, parser,
    search::{Filter, Filters},
    source::{ByteLines, Source},
};
use serde_json::Value;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
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
fn query(d: &Arc<Dataset>, id: u64, filters: &Filters) -> Arc<Query> {
    let q = d
        .query(0, id, Filter::new(filters.clone()).unwrap())
        .unwrap();
    wait(|| q.json()["status"] == "complete");
    q
}
fn all_rows(d: &Dataset, q: &Query) -> Vec<Value> {
    let count = q.json()["count"].as_u64().unwrap();
    let mut rows = vec![];
    while (rows.len() as u64) < count {
        let page = d.page(q, rows.len() as u64, 200).unwrap();
        let more = page["rows"].as_array().unwrap();
        assert!(!more.is_empty());
        rows.extend(more.iter().cloned());
    }
    rows
}
fn lines(rows: &[Value]) -> Vec<u64> {
    rows.iter()
        .map(|r| r["lineNumber"].as_u64().unwrap())
        .collect()
}

/// Expected rows, computed one physical row at a time from the documented
/// rule, and each continuation row's parsed owner line.
fn oracle(files: &[&[u8]], filters: &Filters) -> (Vec<u64>, Vec<(u64, Option<u64>)>) {
    let filter = Filter::new(filters.clone()).unwrap();
    let recognize = filters.recognizes_line_numbers();
    let (mut out, mut owners, mut line) = (vec![], vec![], 0u64);
    for data in files {
        // Per file: the decision and parsed line of the current entry.
        let mut entry: Option<(bool, Option<u64>)> = None;
        for (_, bytes) in ByteLines::new(data) {
            line += 1;
            let raw = encoding::decode(bytes);
            let r = parser::parse_line(&raw, line, recognize);
            let (pass, owner) = if r.parsed.is_some() || parser::starts_entry(&raw, recognize) {
                let decided = (filter.fields_match(&r), r.parsed.is_some().then_some(line));
                entry = Some(decided);
                (decided.0, None)
            } else {
                entry.unwrap_or((filter.admits_unparsed(), None))
            };
            let hit = filters.query.is_empty() || !filter.highlights(&raw).is_empty();
            if pass && hit {
                out.push(line);
                if r.parsed.is_none() {
                    owners.push((line, owner));
                }
            }
        }
    }
    (out, owners)
}

const SAMPLE: &str = "preamble before any log
2026-09-30 21:55:53.776261 [INFO] [AppDelegate] launched
2026-09-30 21:55:54.351980 [DEBUG] [AlertsService] [
  {
    \"body\": \"안내문구3\",
    \"schedule_window_at\": \"2026-10-02T09:00:00.000Z\",
    \"snooze_threshold\": \"today\"
  }
]
2026-09-30 21:55:55.000001 [ERROR] [Network] failed
2026-09-30 21:55:55.100 Runner[3870:bad] malformed
  detail of the malformed row
";

#[test]
fn json_body_follows_its_log_entry() {
    let d = Dataset::new(1, Source::text(SAMPLE, "sample").unwrap());
    d.start(|_| {});
    wait(|| d.status(0)["complete"] == true);
    let summary = d.status(0)["summary"].clone();
    assert_eq!(summary["parsed"], 3);
    // Rows 4-9 continue AlertsService; rows 1, 11 and 12 are orphans.
    assert_eq!(summary["continuation"], 6);
    let mut id = 0;
    for (filters, expected) in [
        (
            Filters {
                tag: "alertsservice".into(),
                ..Filters::default()
            },
            (3..=9).collect::<Vec<u64>>(),
        ),
        (
            Filters {
                levels: vec!["D".into()],
                ..Filters::default()
            },
            (3..=9).collect(),
        ),
        (
            Filters {
                time_from: "21:55:54".into(),
                time_to: "21:55:54.9".into(),
                ..Filters::default()
            },
            (3..=9).collect(),
        ),
        // Excluding the tag removes the whole entry, not only its first row.
        (
            Filters {
                exclude_tags: "Alerts".into(),
                include_raw: false,
                ..Filters::default()
            },
            vec![2, 10],
        ),
        // The query still matches physical rows.
        (
            Filters {
                tag: "AlertsService".into(),
                query: "안내문구3".into(),
                ..Filters::default()
            },
            vec![5],
        ),
        (
            Filters {
                levels: vec!["E".into()],
                query: "안내문구3".into(),
                ..Filters::default()
            },
            vec![],
        ),
        // Orphans keep the "include raw" rule, including after a malformed
        // row that starts its own entry.
        (
            Filters {
                include_raw: false,
                ..Filters::default()
            },
            (2..=10).collect(),
        ),
        (Filters::default(), (1..=12).collect()),
    ] {
        id += 1;
        let q = query(&d, id, &filters);
        let rows = all_rows(&d, &q);
        assert_eq!(lines(&rows), expected, "{filters:?}");
        assert_eq!(
            expected,
            oracle(&[SAMPLE.as_bytes()], &filters).0,
            "{filters:?}"
        );
        for row in &rows {
            let line = row["lineNumber"].as_u64().unwrap();
            if row["kind"] == "parsed" || !(4..=9).contains(&line) {
                assert!(row.get("owner").is_none(), "{row}");
            } else {
                assert_eq!(row["owner"]["lineNumber"], 3);
                assert_eq!(row["owner"]["tag"], "AlertsService");
                assert_eq!(row["owner"]["level"], "D");
                assert_eq!(row["owner"]["time"], "21:55:54.351980");
            }
        }
    }
    d.close();
}

/// Several 128 KiB blocks: iOS entries with JSON bodies, one body longer
/// than three blocks, Android stack traces, malformed stamped rows, blank
/// rows, numbered rows, CR/LF/CRLF and Unicode.
fn workload() -> String {
    let breaks = ["\n", "\r\n", "\r"];
    let mut out = String::from("orphan 1\n\norphan 3\r\n");
    for i in 0..9_000usize {
        let br = breaks[i % 3];
        match i % 6 {
            0 => {
                let body = if i == 3_000 { 9_000 } else { i % 5 };
                out += &format!(
                    "2026-09-30 21:55:{:02}.{:06} [DEBUG] [AlertsService] [{br}",
                    i % 60,
                    i
                );
                for k in 0..body {
                    out += &format!("    \"key{k}\": \"안내문구{i} 🙂\",{br}");
                }
                out += &format!("]{br}");
            }
            1 => {
                out += &format!(
                    "09-21 12:07:30.386  {} 9 E AndroidRuntime: crash {i}{br}",
                    100 + i % 3
                );
                for k in 0..i % 4 {
                    out += &format!("\tat com.example.C{k}.run(C.java:{i}){br}");
                }
            }
            2 => {
                out += &format!(
                    "2026-09-30 21:55:53.776 Runner[101:9] [INFO] [Tag{}] m{br}",
                    i % 4
                )
            }
            3 => {
                out += &format!(
                    "2026-09-30 21:55:53.776 Runner[101:bad] malformed {i}{br}  tail of {i}{br}"
                )
            }
            4 => out += &format!("{i:05}\t09-21 12:07:30.386 101 9 W Num: numbered{br}{br}"),
            _ => out += &format!("09-21 12:07:30.386 102 8 I Tag: m{br}plain continuation {i}{br}"),
        }
    }
    out
}
fn workload_filters() -> Vec<Filters> {
    let mut out = vec![];
    for recognize in [None, Some(false)] {
        let base = Filters {
            recognize_line_numbers: recognize,
            ..Filters::default()
        };
        for f in [
            Filters {
                tag: "AlertsService".into(),
                ..base.clone()
            },
            Filters {
                levels: vec!["D".into()],
                ..base.clone()
            },
            Filters {
                levels: vec!["E".into(), "W".into()],
                include_raw: false,
                ..base.clone()
            },
            Filters {
                levels: vec!["U".into(), "I".into()],
                ..base.clone()
            },
            Filters {
                exclude_tags: "alerts,androidruntime".into(),
                ..base.clone()
            },
            Filters {
                pid: "101".into(),
                ..base.clone()
            },
            Filters {
                time_from: "21:55:10".into(),
                time_to: "21:55:20".into(),
                ..base.clone()
            },
            Filters {
                include_raw: false,
                ..base.clone()
            },
            Filters {
                tag: "alerts".into(),
                query: "안내문구3000".into(),
                ..base.clone()
            },
            Filters {
                levels: vec!["E".into()],
                query: r"C[12]\.run".into(),
                search_mode: "regex".into(),
                ..base.clone()
            },
            Filters {
                query: "continuation".into(),
                ..base.clone()
            },
        ] {
            out.push(f);
        }
    }
    out
}
fn check_workload(d: &Arc<Dataset>, files: &[&[u8]], first_id: u64) -> Vec<Vec<u64>> {
    let mut results = vec![];
    for (k, filters) in workload_filters().iter().enumerate() {
        let q = query(d, first_id + k as u64, filters);
        let (expected, owners) = oracle(files, filters);
        let rows = all_rows(d, &q);
        assert_eq!(q.json()["count"], expected.len() as u64, "{filters:?}");
        assert_eq!(lines(&rows), expected, "{filters:?}");
        // Owners of continuation rows, across blocks too.
        let got: Vec<(u64, Option<u64>)> = rows
            .iter()
            .filter(|r| r["kind"] != "parsed")
            .map(|r| {
                (
                    r["lineNumber"].as_u64().unwrap(),
                    r["owner"]["lineNumber"].as_u64(),
                )
            })
            .collect();
        assert_eq!(got, owners, "{filters:?}");
        // Locating a row agrees with the page order.
        for (index, line) in expected.iter().enumerate().step_by(97) {
            let found = d.locate(&q, *line).unwrap();
            assert_eq!(found["found"], true);
            assert_eq!(found["index"], index as u64);
        }
        // Export writes the same rows.
        let mut out = vec![];
        let (rows_written, _) = d
            .export(&q, &mut out, &AtomicBool::new(false), |_, _| {})
            .unwrap();
        assert_eq!(rows_written, expected.len() as u64, "{filters:?}");
        results.push(expected);
    }
    results
}

#[test]
fn entries_span_blocks_before_and_after_analysis() {
    let text = workload();
    let d = Dataset::new(1, Source::text(&text, "workload").unwrap());
    // First pass: no level table, so every block is masked and the entry
    // owners come from the bounded look-back.
    d.pause_analysis.store(true, Ordering::Release);
    d.start(|_| {});
    wait(|| d.index.lock().unwrap().done);
    let blocks = d.index.lock().unwrap().blocks.len();
    assert!(blocks > 8, "{blocks} blocks");
    let before = check_workload(&d, &[text.as_bytes()], 1);
    d.resume();
    wait(|| d.index.lock().unwrap().block_levels.is_some());
    let table = d.index.lock().unwrap().block_levels.clone().unwrap();
    // Some block starts inside an entry that began in an earlier block.
    assert!(table.iter().any(|t| t[0].owner.is_some()));
    let after = check_workload(&d, &[text.as_bytes()], 1000);
    assert_eq!(before, after);
    d.close();
}

#[test]
fn a_file_never_continues_the_previous_file() {
    let dir = tempfile::tempdir().unwrap();
    let files: [&[u8]; 3] = [
        b"2026-09-30 21:55:54.351980 [DEBUG] [AlertsService] [\n  {\"a\": 1}\n",
        b"  {\"b\": 2}\n]\n2026-09-30 21:55:55.000 [ERROR] [Network] x\n  stack\n",
        b"\n09-21 12:07:30.386  1 2 I Tag: m\r\n\tat a.b(C.java:1)",
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
    let d = Dataset::new(1, Source::granted_files(&paths).unwrap());
    d.start(|_| {});
    wait(|| d.status(0)["complete"] == true);
    let q = query(
        &d,
        1,
        &Filters {
            tag: "AlertsService".into(),
            ..Filters::default()
        },
    );
    assert_eq!(lines(&all_rows(&d, &q)), vec![1, 2]);
    let q = query(
        &d,
        2,
        &Filters {
            include_raw: false,
            ..Filters::default()
        },
    );
    assert_eq!(lines(&all_rows(&d, &q)), vec![1, 2, 5, 6, 8, 9]);
    for (k, filters) in workload_filters().iter().enumerate() {
        let q = query(&d, 10 + k as u64, filters);
        assert_eq!(
            lines(&all_rows(&d, &q)),
            oracle(&files, filters).0,
            "{filters:?}"
        );
    }
    d.close();
}
