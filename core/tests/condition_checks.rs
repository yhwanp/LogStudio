//! The logs pane's condition checks: for each listed condition, whether a
//! row of the log matches it, as one parsed row at a time decides, whatever
//! the tables can tell, after a replaced query, from what earlier queries
//! found, and while a file is followed.
use logstudio_core::{
    dataset::{Dataset, Query},
    dlt::{
        synth::{id, Arg, Message},
        Clock, Decoder,
    },
    encoding, fields_index, parser,
    search::{Condition, Filter, Filters},
    source::{ByteLines, Format, Source},
};
use serde_json::{json, Value};
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::{atomic::Ordering, Arc},
    thread,
    time::{Duration, Instant},
};

fn wait(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !done() {
        assert!(Instant::now() < deadline, "timed out: {what}");
        thread::sleep(Duration::from_millis(1));
    }
}
struct Rng(u64);
impl Rng {
    fn below(&mut self, n: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) as usize % n
    }
    fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }
}
fn write(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

/// Android and iOS rows over many blocks: levels, tags in several cases,
/// iOS component tags, rows without tags, a few PIDs and TIDs, minutes
/// 12:00-12:09, stack traces that cross blocks, numbered rows (entries
/// only when recognized), damaged UTF-8, blank rows, CR/LF/CRLF, and a
/// rare tag only near the end, so that finding it reads far.
fn text_workload(rows: usize, seed: u64) -> Vec<u8> {
    let tags = [
        "ActivityManager",
        "WifiService",
        "WIFISCAN",
        "네트워크",
        "Gatt",
    ];
    let levels = ["V", "D", "I", "W", "E", "F"];
    let breaks: [&[u8]; 3] = [b"\n", b"\r\n", b"\r"];
    let mut r = Rng(seed);
    let mut out = b"preamble before any log\n\n".to_vec();
    for i in 0..rows {
        let br = breaks[i / 13 % 3];
        let (min, sec) = (i / 3000 % 10, i / 50 % 60);
        let row = |s: String| [s.into_bytes(), br.to_vec()].concat();
        let chunk = match i % 40 {
            0 => {
                let mut v = row(format!(
                    "09-21 12:0{min}:{sec:02}.386  102 {} E AndroidRuntime: crash {i}",
                    7 + r.below(3)
                ));
                let lines = if i == rows / 3 { 9_000 } else { r.below(4) };
                for k in 0..lines {
                    v.extend(row(format!("\tat com.example.C{k}.run(C.java:{i})")));
                }
                v
            }
            1 => row(format!(
                "2026-09-30 12:0{min}:{sec:02}.776 Runner[101:{}] [INFO] [SDK_UI] [Comp{}] m {i}",
                9000 + r.below(3),
                r.below(3)
            )),
            2 => row(format!(
                "{i:05}\t09-21 12:0{min}:{sec:02}.386 101 9 W Num: numbered"
            )),
            3 => row(format!(
                "2026-09-30 12:0{min}:{sec:02}.776 Runner[103:5] plain {i}"
            )),
            4 => {
                let mut v = format!("09-21 12:0{min}:{sec:02}.386 104 11 I Bad").into_bytes();
                v.extend_from_slice(b"\xff\xfeTag: broken \xe2\x82");
                v.extend_from_slice(br);
                v
            }
            5 => row(String::new()),
            6 if i + 400 > rows => row(format!(
                "09-21 12:0{min}:{sec:02}.386 4242 4243 W RareTag: rare {i}"
            )),
            _ => row(format!(
                "09-21 12:0{min}:{sec:02}.{:03} {} {} {} {}: message {i}",
                i % 1000,
                101 + r.below(3),
                20 + r.below(40),
                levels[r.below(levels.len())],
                tags[r.below(tags.len())],
            )),
        };
        out.extend(chunk);
    }
    out
}
const TAGS: [&str; 9] = [
    "activity",
    "wifi",
    "네트워크",
    "comp",
    "raretag",
    "nothing",
    "num",
    "gatt",
    "bad",
];
const TIMES: [&str; 6] = [
    "12:03:10",
    "12:07",
    "13:00:00",
    "09-21 12:05:30",
    "2026-09-30 12:01:00",
    "12:09:59.5",
];
fn condition(r: &mut Rng) -> Condition {
    let mut c = Condition::default();
    if r.chance(50) {
        c.levels.retain(|_| r.chance(50));
    }
    if r.chance(45) {
        c.tag = TAGS[r.below(TAGS.len())].into();
    }
    if r.chance(20) {
        c.pid = ["101", "4242", "999", "103 104"][r.below(4)].into();
    }
    if r.chance(15) {
        c.tid = ["7", "9001", "55555"][r.below(3)].into();
    }
    if r.chance(25) {
        c.time_from = TIMES[r.below(TIMES.len())].into();
    }
    if r.chance(25) {
        c.time_to = TIMES[r.below(TIMES.len())].into();
    }
    c
}
/// A filter of one showing condition, None for one without criteria.
fn alone(c: &Condition, recognize: Option<bool>) -> Option<Filter> {
    let f = Filter::new(Filters {
        include: vec![c.clone()],
        recognize_line_numbers: recognize,
        ..Filters::default()
    })
    .unwrap();
    (!f.shows().is_empty()).then_some(f)
}
/// Whether a row of text files matches the condition, row by row.
fn text_oracle(files: &[Vec<u8>], c: &Condition, recognize: Option<bool>) -> Option<bool> {
    let f = alone(c, recognize)?;
    let recognize = recognize.unwrap_or(true);
    let mut line = 0;
    for data in files {
        for (_, bytes) in ByteLines::new(data) {
            line += 1;
            let raw = encoding::decode(bytes);
            let r = parser::parse_line(&raw, line, recognize);
            if r.parsed.is_some() && f.fields_match(&r) {
                return Some(true);
            }
        }
    }
    Some(false)
}
/// The query's states once none is pending.
fn settled(q: &Query) -> Value {
    let mut v = Value::Null;
    wait("checks", || {
        v = q.json()["conditions"].clone();
        let pending = |list: &Value| list.as_array().unwrap().iter().any(|s| s == "pending");
        !pending(&v["include"]) && !pending(&v["exclude"])
    });
    v
}
fn word(known: Option<bool>) -> Value {
    match known {
        None => Value::Null,
        Some(true) => json!("some"),
        Some(false) => json!("none"),
    }
}
/// Random filters on `d`, each condition's state against `oracle`, and the
/// same filter again answered at once from what the first found.
fn random_filters(
    d: &Arc<Dataset>,
    seed: u64,
    rounds: usize,
    oracle: &dyn Fn(&Condition, Option<bool>) -> Option<bool>,
) -> [usize; 2] {
    let mut r = Rng(seed);
    let mut id = 0;
    let (mut some, mut none) = (0, 0);
    for _ in 0..rounds {
        let filters = Filters {
            include: (0..r.below(4)).map(|_| condition(&mut r)).collect(),
            exclude: (0..r.below(3)).map(|_| condition(&mut r)).collect(),
            recognize_line_numbers: [None, Some(false)][r.below(2)],
            include_raw: r.chance(70),
            ..Filters::default()
        };
        id += 1;
        let q = d
            .query(0, id, Filter::new(filters.clone()).unwrap())
            .unwrap();
        // Without a condition of criteria there is nothing to check.
        let checked = filters
            .include
            .iter()
            .chain(&filters.exclude)
            .any(|c| alone(c, None).is_some());
        if !checked {
            assert!(q.json().get("conditions").is_none(), "{filters:?}");
            continue;
        }
        let states = settled(&q);
        for (list, conds) in [("include", &filters.include), ("exclude", &filters.exclude)] {
            for (k, c) in conds.iter().enumerate() {
                let expected = oracle(c, filters.recognize_line_numbers);
                assert_eq!(
                    states[list][k],
                    word(expected),
                    "{list} {k}: {c:?} in {filters:?}"
                );
                some += usize::from(expected == Some(true));
                none += usize::from(expected == Some(false));
            }
        }
        // Another query of the same conditions knows them at once.
        id += 1;
        let again = d
            .query(0, id, Filter::new(filters.clone()).unwrap())
            .unwrap();
        assert_eq!(again.json()["conditions"], states, "{filters:?}");
    }
    [some, none]
}

#[test]
fn checks_agree_with_rows_over_files() {
    let dir = tempfile::tempdir().unwrap();
    let files = [text_workload(30_000, 3), text_workload(12_000, 5)];
    let paths: Vec<PathBuf> = files
        .iter()
        .enumerate()
        .map(|(i, f)| write(dir.path(), &format!("{i}.log"), f))
        .collect();
    let d = Dataset::new(1, Source::granted_files(&paths).unwrap());
    d.start(|_| {});
    wait("index", || d.index.lock().unwrap().done);
    assert!(d.index.lock().unwrap().blocks.len() > 8);
    let [some, none] = random_filters(&d, 0x5eed, 80, &|c, recognize| {
        text_oracle(&files, c, recognize)
    });
    assert!(some > 30 && none > 30, "{some} {none}");
    // The shorthand fields are not checked, a listed condition beside them is.
    let shorthand = |v: Value| Filter::new(serde_json::from_value(v).unwrap()).unwrap();
    let q = d
        .query(
            0,
            1_000,
            shorthand(json!({"tag":"wifi","excludeTags":"gatt"})),
        )
        .unwrap();
    assert!(q.json().get("conditions").is_none());
    let q = d
        .query(
            0,
            1_001,
            shorthand(json!({"tag":"wifi","include":[{"tag":"nothing"}]})),
        )
        .unwrap();
    assert_eq!(settled(&q), json!({"include":["none"],"exclude":[]}));
    // The logs pane only is checked.
    let q = d
        .query(
            1,
            1,
            Filter::new(serde_json::from_value(json!({"include":[{"tag":"wifi"}]})).unwrap())
                .unwrap(),
        )
        .unwrap();
    assert!(q.json().get("conditions").is_none());
    d.close();
}

#[test]
fn checks_read_rows_of_a_dropped_field() {
    // Over 65,536 distinct TIDs drop the TID metadata.
    let mut log = vec![];
    for i in 0..70_000u32 {
        let tag = if i == 69_000 { "Late" } else { "Common" };
        writeln!(
            log,
            "09-21 12:07:{:02}.000 {} {} I {tag}: m {i}",
            i % 60,
            100 + i % 3,
            10_000 + i
        )
        .unwrap();
    }
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "tids.log", &log);
    let d = Dataset::new(1, Source::granted(&path).unwrap());
    d.start(|_| {});
    let f: Filters = serde_json::from_value(json!({
        "include": [{"tid":"79000"}, {"tid":"5"}, {"tid":"79000","tag":"late"}, {"tid":"10001","tag":"late"}],
        "exclude": [{"pid":"101","tid":"10001"}, {}]
    }))
    .unwrap();
    let q = d.query(0, 1, Filter::new(f).unwrap()).unwrap();
    let states = settled(&q);
    let kept = d
        .index
        .lock()
        .unwrap()
        .block_fields
        .as_ref()
        .unwrap()
        .kept();
    assert!(!kept[fields_index::TID] && kept[fields_index::PID]);
    assert_eq!(
        states,
        json!({"include":["some","none","some","none"],"exclude":["some",null]})
    );
    d.close();
}

const APIDS: [&str; 3] = ["NAV", "AUD", "SYS"];
/// DLT messages over several blocks: every level, three APIDs, CTIDs, two
/// ECUs, a rare APID near the end and a storage time of 10 per second.
fn dlt_workload() -> Vec<u8> {
    let mut out = vec![];
    for i in 0..40_000u32 {
        let apid = if i == 39_000 {
            "RARE"
        } else {
            APIDS[(i % 3) as usize]
        };
        let ctid = format!("C{}", i % 7);
        let mut m = Message::log(
            (i % 6 + 1) as u8,
            apid,
            &ctid,
            &[
                Arg::Utf8(&format!("value {i} 경로")),
                Arg::Uint(i.into(), 4),
            ],
        );
        m.seconds = 1_773_480_413 + i / 10;
        m.micros = (i % 10) * 100_000;
        m.ecu = Some(id(["ECU1", "HU"][(i / 5 % 2) as usize]));
        m.write(&mut out);
    }
    out
}

#[test]
fn dlt_checks_agree_with_rendered_rows() {
    let raw = dlt_workload();
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "a.dlt", &raw);
    let d = Dataset::with_dlt_clock(1, Source::granted(&path).unwrap(), Clock::Fixed(0));
    d.start(|_| {});
    let mut text = vec![];
    Decoder::new(Clock::Fixed(0)).render(&raw, 0, &mut text);
    let rows: Vec<String> = String::from_utf8(text)
        .unwrap()
        .split_terminator('\n')
        .map(String::from)
        .collect();
    let mut r = Rng(0xd17);
    let tags = ["NAV", "aud", "ECU1", "hu", "C3", "RARE", "NONE", "c"];
    // 09:26:53 to 10:33:33 UTC.
    let times = [
        "09:30",
        "10:33:30",
        "11:00",
        "09:26:53.5",
        "2026-03-14 10:00",
    ];
    let mut id = 0;
    let (mut some, mut none) = (0, 0);
    for _ in 0..60 {
        let c = |r: &mut Rng| {
            let mut c = Condition::default();
            if r.chance(50) {
                c.levels.retain(|_| r.chance(50));
            }
            if r.chance(60) {
                c.tag = tags[r.below(tags.len())].into();
            }
            if r.chance(10) {
                c.pid = "1".into();
            }
            if r.chance(30) {
                c.time_from = times[r.below(times.len())].into();
            }
            if r.chance(30) {
                c.time_to = times[r.below(times.len())].into();
            }
            c
        };
        let filters = Filters {
            include: (0..1 + r.below(3)).map(|_| c(&mut r)).collect(),
            exclude: (0..r.below(3)).map(|_| c(&mut r)).collect(),
            ..Filters::default()
        };
        id += 1;
        let q = d
            .query(0, id, Filter::new(filters.clone()).unwrap())
            .unwrap();
        let states = settled(&q);
        for (list, conds) in [("include", &filters.include), ("exclude", &filters.exclude)] {
            for (k, c) in conds.iter().enumerate() {
                let expected = alone(c, None).map(|f| {
                    rows.iter().enumerate().any(|(i, row)| {
                        let r = parser::parse_in(Format::Dlt, row, i as u64 + 1, true);
                        r.parsed.is_some() && f.fields_match(&r)
                    })
                });
                assert_eq!(states[list][k], word(expected), "{list} {k}: {c:?}");
                some += usize::from(expected == Some(true));
                none += usize::from(expected == Some(false));
            }
        }
    }
    assert!(some > 20 && none > 20, "{some} {none}");
    d.close();
}

#[test]
fn a_replaced_query_stops_and_the_next_reuses_what_was_found() {
    let files = [text_workload(60_000, 9)];
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "a.log", &files[0]);
    let d = Dataset::new(1, Source::granted(&path).unwrap());
    d.start(|_| {});
    wait("analysis", || {
        d.index.lock().unwrap().block_fields.is_some()
    });
    // Times outside the log need every block read.
    let slow: Filters = serde_json::from_value(json!({
        "include": [{"timeFrom":"13:00","timeTo":"13:30"}, {"tag":"raretag"}],
        "exclude": [{"tag":"wifi","timeFrom":"14:00"}]
    }))
    .unwrap();
    let first = d.query(0, 1, Filter::new(slow.clone()).unwrap()).unwrap();
    let other: Filters = serde_json::from_value(json!({"exclude":[{"tag":"nothing"}]})).unwrap();
    let second = d.query(0, 2, Filter::new(other).unwrap()).unwrap();
    let frozen = first.json()["conditions"].clone();
    assert_eq!(settled(&second), json!({"include":[],"exclude":["none"]}));
    thread::sleep(Duration::from_millis(50));
    assert_eq!(
        first.json()["conditions"],
        frozen,
        "a replaced query's check stopped"
    );
    let third = d.query(0, 3, Filter::new(slow.clone()).unwrap()).unwrap();
    let states = settled(&third);
    for (list, conds) in [("include", &slow.include), ("exclude", &slow.exclude)] {
        for (k, c) in conds.iter().enumerate() {
            assert_eq!(states[list][k], word(text_oracle(&files, c, None)), "{c:?}");
        }
    }
    let fourth = d.query(0, 4, Filter::new(slow).unwrap()).unwrap();
    assert_eq!(fourth.json()["conditions"], states);
    d.close();
}

#[test]
fn followed_rows_change_what_conditions_find() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(
        dir.path(),
        "f.log",
        &(0..5_000)
            .flat_map(|i| {
                format!("09-21 12:07:{:02}.000 101 7 I Early: row {i}\n", i % 60).into_bytes()
            })
            .collect::<Vec<_>>(),
    );
    let d = Dataset::new(1, Source::granted(&path).unwrap());
    d.follow_poll_ms.store(2, Ordering::Relaxed);
    d.start(|_| {});
    wait("analysis", || {
        d.index.lock().unwrap().block_fields.is_some()
    });
    d.follow(true).unwrap();
    let f: Filters = serde_json::from_value(json!({
        "include": [{"tag":"early"}, {"tag":"partialtag"}],
        "exclude": [{"tag":"late"}, {"tag":"late","timeFrom":"13:00"}]
    }))
    .unwrap();
    let q = d.query(0, 1, Filter::new(f).unwrap()).unwrap();
    assert_eq!(
        settled(&q),
        json!({"include":["some","none"],"exclude":["none","none"]})
    );
    let append = |bytes: &[u8]| {
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        f.write_all(bytes).unwrap();
    };
    append(b"09-21 13:00:01.000 101 7 W Late: arrived\n");
    wait("late rows", || {
        let v = q.json()["conditions"].clone();
        v["exclude"] == json!(["some", "some"])
    });
    // A last row without a line ending is replaced once completed.
    append(b"09-21 13:00:02.000 101 7 I Partial");
    thread::sleep(Duration::from_millis(30));
    append(b"Tag: done\n");
    wait("completed row", || {
        q.json()["conditions"]["include"] == json!(["some", "some"])
    });
    assert_eq!(q.json()["conditions"]["exclude"], json!(["some", "some"]));
    d.follow(false).unwrap();
    d.close();
}
