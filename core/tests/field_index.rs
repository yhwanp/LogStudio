//! Per-block field metadata must never change a result: counts, block
//! entries, pages, `locate` and exports equal the scan that reads every
//! block, and the field decision of each row is that of `fields_match`.
use logstudio_core::{
    dataset::{Dataset, Query},
    encoding, fields_index, parser,
    search::{Filter, Filters},
    source::{ByteLines, Source},
};
use serde_json::Value;
use std::{
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc},
    thread,
    time::{Duration, Instant},
};

fn wait(mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !done() {
        assert!(Instant::now() < deadline, "timed out");
        thread::sleep(Duration::from_millis(1));
    }
}
fn open(files: &[Vec<u8>]) -> (tempfile::TempDir, Arc<Dataset>) {
    let dir = tempfile::tempdir().unwrap();
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
    wait(|| d.index.lock().unwrap().block_fields.is_some());
    (dir, d)
}

/// Android and iOS rows over many blocks: common, rare, substring, case and
/// Unicode tags, iOS component tags, rows without tags, a rare PID, JSON and
/// stack trace bodies (one longer than two blocks, so some blocks hold no
/// entry row), malformed stamped rows, numbered rows, damaged UTF-8, blank
/// rows and CR/LF/CRLF.
fn workload(rows: usize) -> Vec<u8> {
    let tags = [
        "ActivityManager",
        "ActivityManager",
        "ActivityManager",
        "WifiService",
        "WIFISERVICE",
        "WifiScanner",
        "네트워크",
        "Full.Tag",
        "BluetoothGatt",
        "ActivityTaskManager",
    ];
    let levels = ["V", "D", "I", "W", "E", "F", "A"];
    let breaks: [&[u8]; 3] = [b"\n", b"\r\n", b"\r"];
    let mut out = b"preamble before any log\n\n".to_vec();
    let mut seed = 7u64;
    let mut next = |n: u64| {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (seed >> 33) % n
    };
    for i in 0..rows {
        let br = breaks[i / 17 % 3];
        let sec = i / 300 % 60;
        let row = |s: String| [s.into_bytes(), br.to_vec()].concat();
        let chunk: Vec<u8> = match i % 50 {
            0 => {
                let body = if i == rows / 2 { 4_000 } else { next(6) as usize };
                let mut v = row(format!(
                    "2026-09-30 21:55:{sec:02}.{i:06} Runner[101:9000] [DEBUG] [AlertsService] ["
                ));
                for k in 0..body {
                    v.extend(row(format!("    \"key{k}\": \"안내문구{i} 🙂\",")));
                }
                v.extend(row("]".into()));
                v
            }
            1 => {
                let mut v = row(format!(
                    "09-21 12:07:{sec:02}.386  102 {} E AndroidRuntime: crash {i}",
                    7 + next(3)
                ));
                for k in 0..next(4) {
                    v.extend(row(format!("\tat com.example.C{k}.run(C.java:{i})")));
                }
                v
            }
            2 => row(format!(
                "2026-09-30 21:55:{sec:02}.776 Runner[101:{}] [INFO] [SDK_UI] [Gallery{}] [SDK_UI] m {i}",
                9000 + next(3),
                next(3)
            )),
            3 => row(format!(
                "2026-09-30 21:55:{sec:02}.776 Runner[3870:bad] malformed {i}{}  tail of {i}",
                std::str::from_utf8(br).unwrap()
            )),
            4 => row(format!("{i:05}\t09-21 12:07:{sec:02}.386 101 9 W Num: numbered")),
            5 => row(format!(
                "  {i} 2026-09-30 21:55:{sec:02} numbered stamp, an entry only when recognized"
            )),
            6 => row(format!("2026-09-30 21:55:{sec:02}.776 Runner[103:5] plain no tag {i}")),
            7 => row(format!("2026-09-30 21:55:{sec:02}.776 [ERROR] [Comp] no process {i}")),
            8 => row(format!("09-21 12:07:{sec:02}.386 101 202 W  : empty tag {i}")),
            9 => {
                let mut v = format!("09-21 12:07:{sec:02}.386 104 11 I Bad").into_bytes();
                v.extend_from_slice(b"\xff\xfeTag: broken \xe2\x82");
                v.extend_from_slice(br);
                v
            }
            10 => row(String::new()),
            11 if i / 50 % 97 == 13 => {
                // A rare tag and PID, and a body longer than two blocks.
                let mut v = row(format!("09-21 12:07:{sec:02}.386 4242 4243 W RareTag: rare {i}"));
                if i / 50 == 13 {
                    v.extend(row(format!("09-21 12:07:{sec:02}.386 4242 4243 I HugeBody: start")));
                    for k in 0..12_000 {
                        v.extend(row(format!("  huge body line {k} 한글")));
                    }
                }
                v
            }
            _ => row(format!(
                "09-21 12:07:{sec:02}.{:03} {} {} {} {}: message {i} value={}",
                i % 1000,
                101 + next(4),
                20 + next(60),
                levels[next(7) as usize],
                tags[next(10) as usize],
                i * 7
            )),
        };
        out.extend(chunk);
    }
    out
}

/// Expected rows by the documented rule, one physical row at a time.
fn oracle(files: &[Vec<u8>], filters: &Filters) -> Vec<u64> {
    let filter = Filter::new(filters.clone()).unwrap();
    let recognize = filters.recognizes_line_numbers();
    let (mut out, mut line) = (vec![], 0u64);
    for data in files {
        let mut entry = None;
        for (_, bytes) in ByteLines::new(data) {
            line += 1;
            let raw = encoding::decode(bytes);
            let r = parser::parse_line(&raw, line, recognize);
            let pass = if r.parsed.is_some() || parser::starts_entry(&raw, recognize) {
                entry = Some(filter.fields_match(&r));
                entry.unwrap()
            } else {
                entry.unwrap_or(filter.admits_unparsed())
            };
            if pass && (filters.query.is_empty() || filter.row_matches(&raw)) {
                out.push(line);
            }
        }
    }
    out
}

fn filters() -> Vec<Filters> {
    let f = |json: Value| -> Filters { serde_json::from_value(json).unwrap() };
    let mut out = vec![];
    for recognize in [Value::Null, Value::Bool(false)] {
        for mut v in [
            serde_json::json!({"tag":"activitymanager"}),
            serde_json::json!({"tag":"RareTag"}),
            serde_json::json!({"tag":"wifi"}),
            serde_json::json!({"tag":"네트워크"}),
            serde_json::json!({"tag":"AlertsService"}),
            serde_json::json!({"tag":"gallery1"}),
            serde_json::json!({"tag":"HugeBody"}),
            serde_json::json!({"tag":"bad"}),
            serde_json::json!({"tag":"num"}),
            serde_json::json!({"tag":"a"}),
            serde_json::json!({"tag":"zzz-nothing"}),
            serde_json::json!({"excludeTags":"activitymanager,wifi"}),
            serde_json::json!({"excludeTags":"alertsservice","includeRaw":false}),
            serde_json::json!({"excludeTags":"qqq-nothing"}),
            serde_json::json!({"excludeTags":"comp"}),
            serde_json::json!({"pid":"101"}),
            serde_json::json!({"pid":"4242"}),
            serde_json::json!({"pid":"103"}),
            serde_json::json!({"tid":"9000"}),
            serde_json::json!({"tid":"8"}),
            serde_json::json!({"tid":"202","levels":["W"]}),
            serde_json::json!({"tag":"activitymanager","levels":["E"]}),
            serde_json::json!({"pid":"101","levels":["W","E"],"includeRaw":false}),
            serde_json::json!({"tag":"sdk","levels":["U","I"]}),
            serde_json::json!({"tag":"wifi","pid":"102"}),
            serde_json::json!({"tag":"activity","excludeTags":"activitymanager"}),
            serde_json::json!({"tag":"alertsservice","query":"안내문구"}),
            serde_json::json!({"pid":"102","query":"crash"}),
            serde_json::json!({"excludeTags":"activitymanager","query":"body"}),
            serde_json::json!({"tag":"rare","timeFrom":"12:07:10","timeTo":"12:07:40"}),
            serde_json::json!({"pid":"4242","query":"huge body line 59","caseSensitive":true}),
            serde_json::json!({"tid":"7","query":"c[12]\\.run","searchMode":"regex"}),
            // Comma-separated lists accept any listed value.
            serde_json::json!({"tag":"wifi, RareTag, zzz-nothing"}),
            serde_json::json!({"pid":"101, 4242 9999"}),
            serde_json::json!({"tid":"202,8","levels":["W","E"]}),
            serde_json::json!({"tag":"activity,sdk","excludeTags":"activitymanager"}),
            serde_json::json!({"pid":"101,103","query":"crash"}),
            // Several conditions: tags or levels, hidden levels of a tag,
            // two fields, rare values, a query and a time range.
            serde_json::json!({"include":[{"tag":"activitymanager"},{"levels":["E","F"]}]}),
            serde_json::json!({"exclude":[{"tag":"activitymanager","levels":["V","D","I"]}]}),
            serde_json::json!({"include":[{"levels":["E"]}],"exclude":[{"levels":["E"],"tag":"wifi"}]}),
            serde_json::json!({"include":[{"tag":"RareTag"},{"tag":"zzz-nothing"}]}),
            serde_json::json!({"include":[{"tag":"wifi"},{"pid":"101"}]}),
            serde_json::json!({"include":[{"tag":"sdk"},{"tid":"9000"}],"exclude":[{"pid":"4242"}]}),
            serde_json::json!({"tag":"wifi","include":[{"tag":"alertsservice","levels":["W"]}]}),
            serde_json::json!({"include":[{"tag":"activity"},{"pid":"102"}],"query":"crash"}),
            serde_json::json!({"include":[{"tag":"rare","timeFrom":"12:07:10","timeTo":"12:07:40"},{"levels":["F"]}]}),
            serde_json::json!({"exclude":[{"levels":["V"]},{"tag":"comp"}],"includeRaw":false}),
            serde_json::json!({"include":[{"levels":[]}]}),
        ] {
            v["recognizeLineNumbers"] = recognize.clone();
            out.push(f(v));
        }
    }
    out
}

struct Observed {
    count: u64,
    ends: Vec<u64>,
    entries: Vec<bool>,
    pages: Vec<String>,
    locates: Vec<String>,
    export: Vec<u8>,
}
fn observe(d: &Arc<Dataset>, id: u64, filters: &Filters) -> Observed {
    let q: Arc<Query> = d
        .query(0, id, Filter::new(filters.clone()).unwrap())
        .unwrap();
    wait(|| q.json()["status"] != "running");
    results(d, &q)
}
fn results(d: &Arc<Dataset>, q: &Arc<Query>) -> Observed {
    assert_eq!(q.json()["status"], "complete");
    let count = q.json()["count"].as_u64().unwrap();
    let (ends, entries) = {
        let r = q.result.lock().unwrap();
        (r.ends.clone(), r.entries.clone())
    };
    let pages = [0, 1, count / 3, count / 2, count.saturating_sub(3)]
        .iter()
        .map(|&start| d.page(q, start, 200).unwrap()["rows"].to_string())
        .collect();
    let total = d.status(0)["total"].as_u64().unwrap();
    let locates = [1, 2, total / 5, total / 2, total - 1, total]
        .iter()
        .map(|&line| {
            let mut found = d.locate(q, line).unwrap();
            found.as_object_mut().unwrap().remove("queryId");
            found.to_string()
        })
        .collect();
    let mut export = vec![];
    d.export(q, &mut export, &AtomicBool::new(false), |_, _| {})
        .unwrap();
    Observed {
        count,
        ends,
        entries,
        pages,
        locates,
        export,
    }
}
/// Blocks the field metadata decides without reading, and all blocks.
fn decided(d: &Dataset, filters: &Filters) -> (usize, usize) {
    let (table, fields) = {
        let index = d.index.lock().unwrap();
        (
            index.block_levels.clone().unwrap(),
            index.block_fields.clone().unwrap(),
        )
    };
    let filter = Filter::new(filters.clone()).unwrap();
    let variant = usize::from(filters.recognizes_line_numbers());
    let Some(plan) = fields.plan(&filter) else {
        return (0, table.len());
    };
    let n = (0..table.len())
        .filter(|&i| plan.decide(i, variant, &table[i][variant]).is_some())
        .count();
    (n, table.len())
}
/// Runs every filter with and without the field table and compares.
fn check(d: &Arc<Dataset>, files: &[Vec<u8>]) -> Vec<(String, usize, usize)> {
    let mut id = 0;
    let mut usage = vec![];
    for filters in filters() {
        id += 1;
        let with = observe(d, id, &filters);
        let table = d.index.lock().unwrap().block_fields.take();
        id += 1;
        let without = observe(d, id, &filters);
        d.index.lock().unwrap().block_fields = table;
        let label = format!("{filters:?}");
        assert_eq!(with.count, without.count, "{label}");
        assert_eq!(with.ends, without.ends, "{label}");
        assert_eq!(with.entries, without.entries, "{label}");
        assert_eq!(with.pages, without.pages, "{label}");
        assert_eq!(with.locates, without.locates, "{label}");
        assert_eq!(with.export, without.export, "{label}");
        assert_eq!(with.count, oracle(files, &filters).len() as u64, "{label}");
        let (n, blocks) = decided(d, &filters);
        usage.push((label, n, blocks));
    }
    usage
}
fn by(usage: &[(String, usize, usize)], needle: &str) -> (usize, usize) {
    let (_, n, blocks) = usage
        .iter()
        .find(|(l, _, _)| l.contains(needle))
        .unwrap_or_else(|| {
            panic!(
                "{needle} is in none of {:#?}",
                usage.iter().map(|u| &u.0).collect::<Vec<_>>()
            )
        });
    (*n, *blocks)
}

#[test]
fn metadata_keeps_every_result() {
    let data = workload(26_000);
    let files = vec![data];
    let (_dir, d) = open(&files);
    let (table, fields) = {
        let index = d.index.lock().unwrap();
        (
            index.block_levels.clone().unwrap(),
            index.block_fields.clone().unwrap(),
        )
    };
    assert!(table.len() > 12, "{} blocks", table.len());
    assert_eq!(fields.kept(), [true; 3]);
    // Each field counts every row except orphan rows, in both variants.
    for (i, t) in table.iter().enumerate() {
        for (v, rows) in t.iter().enumerate() {
            let parsed: u64 = rows.slots[..8].iter().map(|&n| u64::from(n)).sum();
            for f in [fields_index::PID, fields_index::TID, fields_index::TAGS] {
                assert_eq!(fields.counted_rows(f, i, v), Some(parsed), "{i} {v} {f}");
            }
        }
    }
    // Some block holds only rows that continue an earlier block's entry.
    assert!(table
        .iter()
        .any(|t| t[0].tail.is_none() && t[0].owner.is_some()));
    let usage = check(&d, &files);
    // Levels and one field decide every block without reading.
    for needle in [
        "tag: \"activitymanager\", exclude_tags: \"\"",
        "tag: \"RareTag\"",
        "exclude_tags: \"activitymanager,wifi\"",
        "pid: \"101\", tid: \"\", include: [], exclude: [], query: \"\"",
        "tid: \"9000\"",
        "tag: \"activity\", exclude_tags: \"activitymanager\"",
    ] {
        let (n, blocks) = by(&usage, needle);
        assert_eq!(n, blocks, "{needle}");
    }
    // A rare value skips the blocks without it, also with a query.
    let (n, blocks) = by(&usage, "huge body line 59");
    assert!(n > 0 && n < blocks, "{n}/{blocks}");
    // Several conditions over levels and one field decide every block.
    for needle in [
        "tag: \"activitymanager\", pid: \"\", tid: \"\", time_from: \"\", time_to: \"\" }, Condition { levels: [\"E\", \"F\"]",
        "exclude: [Condition { levels: [\"V\", \"D\", \"I\"], tag: \"activitymanager\"",
        "exclude: [Condition { levels: [\"E\"], tag: \"wifi\"",
        "tag: \"RareTag\", pid: \"\", tid: \"\", time_from: \"\", time_to: \"\" }, Condition",
        "include: [Condition { levels: [], tag",
    ] {
        let (n, blocks) = by(&usage, needle);
        assert_eq!(n, blocks, "{needle}");
    }
    // Two fields cannot be counted together, but blocks where no showing
    // condition can match are skipped.
    let (n, blocks) = by(&usage, "tag: \"sdk\", pid");
    assert!(n < blocks, "{n}/{blocks}");
    d.close();
}

#[test]
fn metadata_keeps_results_across_files() {
    let data = workload(9_000);
    // Split inside a JSON body, after a lone CR and inside a stack trace, so
    // files start with rows that must not continue the previous file.
    let at = |needle: &[u8], from: usize| {
        from + data[from..]
            .windows(needle.len())
            .position(|w| w == needle)
            .unwrap()
    };
    let a = at("\"key1\"".as_bytes(), data.len() / 4);
    let b = at(b"\tat com", data.len() / 2);
    let c = at(b"\r", data.len() * 3 / 4) + 1;
    let files = vec![
        data[..a].to_vec(),
        data[a..b].to_vec(),
        data[b..c].to_vec(),
        data[c..].to_vec(),
    ];
    let (_dir, d) = open(&files);
    check(&d, &files);
    d.close();
}

#[test]
fn dropped_field_falls_back_to_reading() {
    // More distinct TIDs than a dictionary keeps; tags stay few.
    let mut data = vec![];
    for i in 0..(fields_index::MAX_VALUES + 3_000) {
        data.extend(
            format!(
                "09-21 12:07:30.386 {} {} {} Tag{}: m {i}\n",
                100 + i % 7,
                1_000_000 + i,
                ["I", "W", "E"][i % 3],
                i % 5
            )
            .into_bytes(),
        );
        if i % 9 == 0 {
            data.extend(b"  continued\n");
        }
    }
    let files = vec![data];
    let (_dir, d) = open(&files);
    let fields = d.index.lock().unwrap().block_fields.clone().unwrap();
    assert_eq!(fields.kept(), [true, false, true]);
    let mut id = 0;
    for json in [
        serde_json::json!({"tid":"1000005"}),
        serde_json::json!({"tid":"1000005","tag":"tag0"}),
        serde_json::json!({"tag":"tag1","levels":["W"]}),
        serde_json::json!({"pid":"103"}),
        serde_json::json!({"excludeTags":"tag2"}),
    ] {
        let filters: Filters = serde_json::from_value(json).unwrap();
        id += 1;
        let with = observe(&d, id, &filters);
        let table = d.index.lock().unwrap().block_fields.take();
        id += 1;
        let without = observe(&d, id, &filters);
        d.index.lock().unwrap().block_fields = table;
        assert_eq!(with.ends, without.ends, "{filters:?}");
        assert_eq!(with.entries, without.entries, "{filters:?}");
        assert_eq!(with.pages, without.pages, "{filters:?}");
        assert_eq!(with.count, oracle(&files, &filters).len() as u64);
        let (n, blocks) = decided(&d, &filters);
        if filters.tid.is_empty() {
            // Fields with metadata still count every block.
            assert_eq!(n, blocks, "{filters:?}");
        } else if filters.tag.is_empty() {
            assert_eq!(n, 0, "{filters:?}");
        }
    }
    d.close();
}

/// What the analysis published: summaries, the level table, and the field
/// table's size and its decisions for every filter.
#[derive(Debug, PartialEq)]
struct Published {
    summary: Value,
    prefix: Value,
    levels: Vec<logstudio_core::dataset::BlockLevels>,
    values: [Option<usize>; 3],
    bytes: [usize; 3],
    decided: Vec<(usize, usize)>,
}
fn published(d: &Dataset) -> Published {
    let index = d.index.lock().unwrap();
    let fields = index.block_fields.clone().unwrap();
    let published = Published {
        summary: index.summary.clone().unwrap(),
        prefix: index.prefix_summary.clone().unwrap(),
        levels: index.block_levels.as_deref().unwrap().clone(),
        values: fields.values(),
        bytes: fields.bytes(),
        decided: vec![],
    };
    drop(index);
    Published {
        decided: filters().iter().map(|f| decided(d, f)).collect(),
        ..published
    }
}
/// A dataset of `files` whose logs pane query of `filters` starts before
/// the file is read, as a reopened file's restored filter does.
fn open_with(
    files: &[Vec<u8>],
    filters: &Filters,
) -> (tempfile::TempDir, Arc<Dataset>, Arc<Query>) {
    let dir = tempfile::tempdir().unwrap();
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
    let q = d
        .query(0, 1, Filter::new(filters.clone()).unwrap())
        .unwrap();
    d.start(|_| {});
    (dir, d, q)
}
/// Whether a query parses every row it scans, so that its scan analyzes
/// the blocks too.
fn parses_every_row(filters: &Filters) -> bool {
    let filter = Filter::new(filters.clone()).unwrap();
    filter.matcher.is_none() && filter.needs_fields()
}

#[test]
fn filter_at_open_analyzes_what_it_reads() {
    let data = workload(26_000);
    let at = data.len() / 3
        + data[data.len() / 3..]
            .iter()
            .position(|&b| b == b'\n')
            .unwrap()
        + 1;
    // Every filter over one file, every third over two.
    for (files, every) in [
        (vec![data.clone()], 1),
        (vec![data[..at].to_vec(), data[at..].to_vec()], 3),
    ] {
        let (_dir, reference) = open(&files);
        wait(|| reference.status(0)["summaryComplete"] == true);
        let expected = published(&reference);
        for (k, filters) in filters().iter().enumerate().step_by(every) {
            let (_dir, d, q) = open_with(&files, filters);
            wait(|| q.json()["status"] != "running");
            let label = format!("{filters:?}");
            // The scan published the analysis before it completed.
            if parses_every_row(filters) {
                assert_eq!(d.status(0)["summaryComplete"], true, "{label}");
            }
            let got = results(&d, &q);
            let want = observe(&reference, 100 + k as u64, filters);
            assert_eq!(got.count, want.count, "{label}");
            assert_eq!(got.ends, want.ends, "{label}");
            assert_eq!(got.entries, want.entries, "{label}");
            assert_eq!(got.pages, want.pages, "{label}");
            assert_eq!(got.locates, want.locates, "{label}");
            assert_eq!(got.export, want.export, "{label}");
            wait(|| d.status(0)["summaryComplete"] == true);
            // Every tables comparison runs all filters; a few bound the time.
            if k % 7 == 0 {
                assert_eq!(published(&d), expected, "{label}");
            }
            d.close();
        }
        reference.close();
    }
}

#[test]
fn analysis_hands_over_and_resumes() {
    let files = vec![workload(26_000)];
    let (_dir, reference) = open(&files);
    wait(|| reference.status(0)["summaryComplete"] == true);
    let expected = published(&reference);
    let tag: Filters = serde_json::from_value(serde_json::json!({"tag":"wifi"})).unwrap();
    // A pass hands its state to a scan that parses every row, before its
    // first block or after the scan delivered some, or the scan ends first;
    // either way the analysis completes, also once a pause ends.
    for (paused, delay) in [(true, 0), (true, 50), (false, 0), (false, 2), (false, 10)] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("0.log");
        std::fs::write(&path, &files[0]).unwrap();
        let d = Dataset::new(1, Source::granted(&path).unwrap());
        d.pause_analysis
            .store(paused, std::sync::atomic::Ordering::Release);
        d.start(|_| {});
        wait(|| d.status(0)["textComplete"] == true);
        thread::sleep(Duration::from_millis(delay));
        let q = d.query(0, 1, Filter::new(tag.clone()).unwrap()).unwrap();
        wait(|| q.json()["status"] != "running");
        d.resume();
        wait(|| d.status(0)["summaryComplete"] == true);
        assert_eq!(published(&d), expected, "{paused} {delay}");
        assert_eq!(results(&d, &q).count, oracle(&files, &tag).len() as u64);
        d.close();
    }
    // A scan replaced midway leaves its blocks to the analysis, also when a
    // search query replaces it, and to a scan that parses every row again.
    for next in [
        serde_json::json!({"query":"message"}),
        serde_json::json!({"levels":["E"]}),
        serde_json::json!({}),
    ] {
        let (_dir, d, q) = open_with(&files, &tag);
        wait(|| q.result.lock().unwrap().ends.len() >= 3);
        let next: Filters = serde_json::from_value(next).unwrap();
        let replaced = d.query(0, 2, Filter::new(next.clone()).unwrap()).unwrap();
        wait(|| replaced.json()["status"] != "running");
        wait(|| d.status(0)["summaryComplete"] == true);
        assert_eq!(published(&d), expected, "{next:?}");
        assert_eq!(
            results(&d, &replaced).count,
            oracle(&files, &next).len() as u64,
            "{next:?}"
        );
        d.close();
    }
}

/// A followed file whose rows arrive after the first analysis pass: a
/// capture's file is empty when it opens, and later rows may hold values
/// the first pass's dictionary lacks. Field filters then match the rows as
/// one parsed row at a time decides them.
#[test]
fn followed_rows_outside_the_dictionary() {
    let row = |i: usize, tag: &str| {
        format!(
            "10-24 13:57:{:02}.000  4321  4321 I {tag}: row {i}\n",
            i % 60
        )
    };
    for (first, later) in [
        (vec![], vec!["NotesUi", "InputDispatcher"]),
        (vec!["WifiA"], vec!["WifiB", "Other"]),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("0.log");
        let head: String = first.iter().enumerate().map(|(i, t)| row(i, t)).collect();
        std::fs::write(&path, &head).unwrap();
        let d = Dataset::new(1, Source::granted(&path).unwrap());
        d.follow_poll_ms
            .store(2, std::sync::atomic::Ordering::Relaxed);
        d.start(|_| {});
        wait(|| d.status(0)["summaryComplete"] == true);
        d.follow(true).unwrap();
        let mut log = head.into_bytes();
        for k in 0..12 {
            let more = row(first.len() + k, later[k % later.len()]);
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap();
            std::io::Write::write_all(&mut f, more.as_bytes()).unwrap();
            log.extend(more.into_bytes());
            let rows = (first.len() + k + 1) as u64;
            wait(|| d.index.lock().unwrap().total == rows);
        }
        wait(|| {
            let index = d.index.lock().unwrap();
            index.analyzed == index.blocks.len()
        });
        let files = vec![log];
        for (id, json) in [
            serde_json::json!({"exclude":[{"tag":"InputDispatcher"}]}),
            serde_json::json!({"include":[{"tag":"wifi"}]}),
            serde_json::json!({"tag":"wifi"}),
            serde_json::json!({"excludeTags":"other"}),
        ]
        .into_iter()
        .enumerate()
        {
            let filters: Filters = serde_json::from_value(json).unwrap();
            let q = d
                .query(0, 1 + id as u64, Filter::new(filters.clone()).unwrap())
                .unwrap();
            wait(|| {
                q.json()["caughtUp"] == true
                    && q.result.lock().unwrap().ends.len() == d.index.lock().unwrap().blocks.len()
            });
            assert_eq!(
                q.json()["count"].as_u64().unwrap(),
                oracle(&files, &filters).len() as u64,
                "{first:?} {filters:?}"
            );
        }
        d.close();
    }
}
