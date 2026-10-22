//! DLT files opened as datasets: rows are decoded messages, compared with
//! rendering the whole file at once, across blocks, before and after the
//! analysis, and in multi-file opens.
use logstudio_core::{
    dataset::{Dataset, ExportKind, Exported, Query},
    dlt::{
        self,
        synth::{id, msin, with_id, Arg, Message},
        Clock, Decoder,
    },
    order,
    search::{Filter, Filters},
    source::Source,
    Engine,
};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
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
fn write(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}
fn open(path: &Path) -> Arc<Dataset> {
    let d = Dataset::with_dlt_clock(1, Source::granted(path).unwrap(), Clock::Fixed(0));
    d.start(|_| {});
    d
}
/// Rows of the whole file rendered at once, as the oracle.
fn rendered(raw: &[u8]) -> Vec<String> {
    let mut out = vec![];
    Decoder::new(Clock::Fixed(0)).render(raw, 0, &mut out);
    String::from_utf8(out)
        .unwrap()
        .split_terminator('\n')
        .map(String::from)
        .collect()
}

const APIDS: [&str; 3] = ["NAV", "AUD", "SYS"];
/// Messages over several blocks: every level, three APIDs, nine CTIDs, two
/// ECUs, Korean text, `needle <i>` every 97 messages, non-verbose and
/// control messages, other bytes, version 2 data and a cut last message.
fn workload() -> Vec<u8> {
    let mut out = vec![];
    for i in 0..8000u32 {
        if i == 3000 {
            out.extend_from_slice(b"\x00\x13garbage between messages\xff");
        }
        if i == 5000 {
            out.extend_from_slice(b"DLT\x02version two data");
        }
        let text = match i % 7 {
            0 => format!("경로 재탐색 {i} 🙂"),
            _ if i % 97 == 0 => format!("found needle {i}"),
            _ => format!("value changed to {i}"),
        };
        let apid = APIDS[(i % 3) as usize];
        let ctid = format!("CTX{}", i % 9 + 1);
        let mut m = Message::log(
            (i % 6 + 1) as u8,
            apid,
            &ctid,
            &[Arg::Utf8(&text), Arg::Uint(i.into(), 4)],
        );
        m.seconds = 1_773_480_413 + i / 10;
        m.micros = (i % 10) * 100_000;
        m.count = i as u8;
        m.ecu = Some(id(["ECU1", "ECU2"][(i % 2) as usize]));
        m.timestamp = Some(i * 1000);
        match i % 50 {
            11 => {
                m.ext = Some((msin(false, 0, 4), 0, id(apid), id(&ctid)));
                m.payload = with_id(false, i, b"AB");
            }
            23 => {
                m.ext = Some((msin(false, 3, 2), 0, id("DA1"), id("DC1")));
                m.payload = with_id(false, 3, &[0, 1, 2]);
            }
            _ => {}
        }
        m.write(&mut out);
    }
    let cut = Message::log(4, "CUT", "END", &[Arg::Ascii(b"cut short")]).bytes();
    out.extend_from_slice(&cut[..cut.len() - 4]);
    out
}
/// Expected row numbers of `filters` over the rendered rows.
fn expected(rows: &[String], filters: &Filters) -> Vec<u64> {
    let lower = |s: &str| s.to_lowercase();
    let terms = |list: &str| -> Vec<String> {
        list.split(',')
            .map(|t| lower(t.trim()))
            .filter(|t| !t.is_empty())
            .collect()
    };
    let (tags, excluded) = (terms(&filters.tag), terms(&filters.exclude_tags));
    let regex = (filters.search_mode == "regex").then(|| {
        regex::RegexBuilder::new(&filters.query)
            .case_insensitive(!filters.case_sensitive)
            .build()
            .unwrap()
    });
    let fielded = filters.levels.len() != 8 || !tags.is_empty() || !filters.time_from.is_empty();
    rows.iter()
        .enumerate()
        .filter(|(_, raw)| {
            let fields = match dlt::parse_row(raw) {
                Some(r) => {
                    let row_tags: Vec<String> = [r.ecu, r.apid, r.ctid]
                        .into_iter()
                        .filter(|t| !dlt::absent(t))
                        .map(lower)
                        .collect();
                    let stamp = format!("{} {}", r.date, r.time);
                    filters.levels.iter().any(|l| l == r.level)
                        && (tags.is_empty()
                            || row_tags.iter().any(|t| tags.iter().any(|x| t.contains(x))))
                        && !row_tags
                            .iter()
                            .any(|t| excluded.iter().any(|x| t.contains(x)))
                        && (filters.time_from.is_empty()
                            || (stamp.as_str() >= filters.time_from.as_str()
                                && stamp[..filters.time_to.len()] <= *filters.time_to))
                }
                None => filters.include_raw && !fielded,
            };
            let text = filters.query.is_empty()
                || match &regex {
                    Some(re) => re.is_match(raw),
                    None if filters.case_sensitive => raw.contains(&filters.query),
                    None => lower(raw).contains(&lower(&filters.query)),
                };
            fields && text
        })
        .map(|(i, _)| i as u64 + 1)
        .collect()
}
fn workload_filters() -> Vec<Filters> {
    let base = Filters::default;
    vec![
        base(),
        Filters {
            levels: vec!["E".into(), "F".into()],
            ..base()
        },
        Filters {
            levels: vec!["U".into()],
            ..base()
        },
        Filters {
            tag: "nav".into(),
            ..base()
        },
        Filters {
            tag: "ecu2, ctx9".into(),
            ..base()
        },
        Filters {
            exclude_tags: "ctx3,AUD".into(),
            ..base()
        },
        Filters {
            time_from: "2026-03-14 09:27:00".into(),
            time_to: "2026-03-14 09:27:30".into(),
            ..base()
        },
        Filters {
            query: "NEEDLE 5".into(),
            ..base()
        },
        Filters {
            query: "경로 재탐색 7".into(),
            tag: "sys".into(),
            ..base()
        },
        Filters {
            query: r"needle \d*3\b".into(),
            search_mode: "regex".into(),
            ..base()
        },
        Filters {
            query: "needle 1".into(),
            case_sensitive: true,
            ..base()
        },
        Filters {
            include_raw: false,
            ..base()
        },
    ]
}
fn check(d: &Arc<Dataset>, rows: &[String], first_id: u64) {
    for (k, filters) in workload_filters().iter().enumerate() {
        let q = query(d, first_id + k as u64, filters);
        let want = expected(rows, filters);
        assert!(!want.is_empty(), "{filters:?}");
        assert_eq!(q.json()["count"], want.len() as u64, "{filters:?}");
        assert_eq!(lines(&all_rows(d, &q)), want, "{filters:?}");
    }
}

#[test]
fn dlt_rows_and_filters_across_blocks_before_and_after_analysis() {
    let dir = tempfile::tempdir().unwrap();
    let raw = workload();
    let path = write(dir.path(), "workload.dlt", &raw);
    let rows = rendered(&raw);
    let d = Dataset::with_dlt_clock(1, Source::granted(&path).unwrap(), Clock::Fixed(0));
    d.pause_analysis.store(true, Ordering::Release);
    d.start(|_| {});
    wait(|| d.index.lock().unwrap().done);
    let blocks = d.index.lock().unwrap().blocks.len();
    assert!(blocks > 4, "{blocks} blocks");
    let status = d.status(0);
    assert_eq!(status["format"], "dlt");
    assert_eq!(status["followable"], false);
    assert_eq!(status["total"], rows.len() as u64);
    assert_eq!(status["size"], raw.len() as u64);
    assert_eq!(status["indexedBytes"], raw.len() as u64);
    assert_eq!(
        status["dltIssues"],
        json!({"garbage":1,"garbageBytes":27,"truncated":1,"unsupported":1})
    );
    // Every row as rendered at once.
    let q = query(&d, 1, &Filters::default());
    let page_rows = all_rows(&d, &q);
    let raws: Vec<&str> = page_rows
        .iter()
        .map(|r| r["raw"].as_str().unwrap())
        .collect();
    assert_eq!(raws, rows);
    check(&d, &rows, 10);
    d.resume();
    wait(|| d.index.lock().unwrap().block_fields.is_some());
    check(&d, &rows, 100);
    let summary = &d.status(0)["summary"];
    assert_eq!(summary["total"], rows.len() as u64);
    assert_eq!(summary["raw"], 3);
    assert_eq!(summary["parsed"], 8000);
    // `include_raw` keeps the three issue rows, and only they are unparsed.
    let issues: Vec<&String> = rows.iter().filter(|r| r.starts_with(dlt::ISSUE)).collect();
    assert_eq!(issues.len(), 3);
    d.close();
}
#[test]
fn page_rows_carry_dlt_fields_and_offsets() {
    let dir = tempfile::tempdir().unwrap();
    let raw = workload();
    let path = write(dir.path(), "w.dlt", &raw);
    let d = open(&path);
    wait(|| d.status(0)["complete"] == true);
    let q = query(
        &d,
        1,
        &Filters {
            query: "재탐색 7 ".into(),
            ..Filters::default()
        },
    );
    let rows = all_rows(&d, &q);
    let r = &rows[0];
    let line = r["raw"].as_str().unwrap();
    assert_eq!(r["kind"], "parsed");
    assert_eq!(r["level"], "E");
    assert_eq!(r["tag"], "AUD");
    assert_eq!((r["pid"].as_str(), r["tid"].as_str()), (Some(""), Some("")));
    assert_eq!(r["tags"], json!(["ECU2", "AUD", "CTX8"]));
    assert_eq!(
        r["dlt"],
        json!({"uptime":"0.7000","count":"7","ecu":"ECU2","apid":"AUD","ctid":"CTX8","session":"4242","kind":"E"})
    );
    assert_eq!(r["message"], "경로 재탐색 7 🙂 7");
    let utf16 = |s: &str| s.encode_utf16().count();
    for key in ["uptime", "ecu", "apid", "ctid", "message"] {
        let value = if key == "message" {
            r["message"].as_str().unwrap()
        } else {
            r["dlt"][key].as_str().unwrap()
        };
        let at = line.find(&format!(" {value}")).unwrap() + 1;
        assert_eq!(r["fieldOffsets"][key], utf16(&line[..at]) as u64, "{key}");
    }
    assert!(r["fieldOffsets"].get("pid").is_none());
    // Highlights are UTF-16 ranges in the row text.
    let at = utf16(&line[..line.find("재탐색 7 ").unwrap()]) as u64;
    assert_eq!(r["highlights"], json!([[at, at + 6]]));
    // The source byte of a DLT row is its message's offset in the file.
    let starts: Vec<u32> = dlt::units(&raw, raw.len())
        .map(|u| u.start as u32)
        .collect();
    assert_eq!(
        r["sourceStartByte"],
        starts[r["lineNumber"].as_u64().unwrap() as usize - 1] as u64
    );
    // Issue rows are rows without fields that open their own entry.
    let q = query(
        &d,
        2,
        &Filters {
            query: "버전 2".into(),
            ..Filters::default()
        },
    );
    let issue = &all_rows(&d, &q)[0];
    assert_eq!(issue["kind"], "raw");
    assert!(issue.get("owner").is_none());
    assert!(issue["raw"]
        .as_str()
        .unwrap()
        .starts_with("!! 지원하지 않는 DLT 버전 2 데이터 20바이트 · 파일 위치 "));
    d.close();
}
#[test]
fn long_rows_detail_record_and_lines() {
    let dir = tempfile::tempdir().unwrap();
    let mut raw = Message::log(4, "BIG", "RAW", &[Arg::Ascii(b"before")]).bytes();
    let data: Vec<u8> = (0..65_000u32).map(|i| i as u8).collect();
    Message::log(3, "BIG", "RAW", &[Arg::Raw(&data)]).write(&mut raw);
    Message::log(4, "BIG", "RAW", &[Arg::Ascii(b"after")]).write(&mut raw);
    let path = write(dir.path(), "big.dlt", &raw);
    let d = open(&path);
    wait(|| d.status(0)["complete"] == true);
    let q = query(&d, 1, &Filters::default());
    let row = &rendered(&raw)[1];
    assert!(row.len() > 190_000);
    let first = d.range(&q, 2, 0).unwrap();
    assert_eq!(first["totalBytes"], row.len() as u64);
    assert_eq!(first["complete"], false);
    assert_eq!(
        first["text"].as_str().unwrap(),
        &row[..first["nextByte"].as_u64().unwrap() as usize]
    );
    let starts: Vec<u64> = dlt::units(&raw, raw.len())
        .map(|u| u.start as u64)
        .collect();
    assert_eq!(first["sourceStartByte"], starts[1]);
    // A DLT row is a log entry of its own.
    let record = d.record(&q, 2, 0).unwrap();
    assert_eq!(
        (record["first"].as_u64(), record["last"].as_u64()),
        (Some(2), Some(2))
    );
    assert_eq!(record["recordComplete"], true);
    let lines = d.lines(&q, &[1, 3]).unwrap();
    let got: Vec<&str> = lines["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["message"].as_str().unwrap())
        .collect();
    assert_eq!(got, ["before", "after"]);
    d.close();
}
#[test]
fn several_dlt_files_open_and_order_together() {
    let dir = tempfile::tempdir().unwrap();
    let early = Message {
        seconds: 1_000,
        ..Message::log(4, "A", "C", &[Arg::Ascii(b"early")])
    }
    .bytes();
    let late = Message {
        seconds: 2_000,
        ..Message::log(4, "A", "C", &[Arg::Ascii(b"late")])
    }
    .bytes();
    let a = write(dir.path(), "a.dlt", &late);
    let empty = write(dir.path(), "empty.dlt", b"");
    let b = write(
        dir.path(),
        "b.dlt",
        &[early.as_slice(), b"\x00\x00"].concat(),
    );
    let (paths, by) = order::arrange(vec![a.clone(), empty.clone(), b.clone()]);
    assert_eq!(paths, [b.clone(), a.clone(), empty.clone()]);
    assert_eq!(by.map(|b| b.as_str()), Some("time"));
    let d = Dataset::with_dlt_clock(1, Source::granted_files(&paths).unwrap(), Clock::Fixed(0));
    d.start(|_| {});
    wait(|| d.status(0)["complete"] == true);
    let status = d.status(0);
    assert_eq!(status["format"], "dlt");
    assert_eq!(status["files"][1]["firstLine"], 3);
    let q = query(&d, 1, &Filters::default());
    let rows = all_rows(&d, &q);
    let raws: Vec<&str> = rows.iter().map(|r| r["raw"].as_str().unwrap()).collect();
    assert_eq!(
        raws[0],
        "1970-01-01 00:16:40.589793 1234.5678 0 ECU1 A C 4242 I early"
    );
    // The issue row of the second unit names its offset in its own file.
    assert_eq!(
        raws[1],
        format!(
            "!! DLT 메시지가 아닌 데이터 2바이트 · 파일 위치 {}: 00 00",
            early.len()
        )
    );
    assert!(raws[2].ends_with(" late"));
    assert_eq!(
        (rows[2]["file"].as_u64(), rows[2]["fileLine"].as_u64()),
        (Some(1), Some(1))
    );
    d.close();
}
#[test]
fn dlt_and_text_do_not_open_together() {
    let dir = tempfile::tempdir().unwrap();
    let a = write(dir.path(), "a.dlt", &Message::log(4, "A", "C", &[]).bytes());
    let t = write(dir.path(), "t.log", b"09-21 12:07:30.386 1 2 I T: x\n");
    let e = Source::granted_files(&[a.clone(), t]).err().unwrap();
    assert_eq!(
        e.to_string(),
        "DLT files and text logs cannot be opened together"
    );
    // An empty file joins either format; a DLT file is never followed.
    let empty = write(dir.path(), "e.log", b"");
    let s = Source::granted_files(&[empty, a.clone()]).unwrap();
    assert!(!s.followable());
    let d = open(&a);
    wait(|| d.status(0)["complete"] == true);
    assert_eq!(
        d.follow(true).unwrap_err().to_string(),
        "DLT files cannot be followed"
    );
    d.close();
}
#[test]
fn files_without_messages_fail_to_open() {
    let dir = tempfile::tempdir().unwrap();
    for (name, bytes, error) in [
        (
            "garbage.dlt",
            b"DLT\x01 but nothing else".to_vec(),
            "A selected DLT file contains no DLT messages",
        ),
        (
            "v2.dlt",
            b"DLT\x02\x00\x00\x00\x00\x00\x00 version two".to_vec(),
            "DLT version 2 files are not supported",
        ),
    ] {
        let path = write(dir.path(), name, &bytes);
        let d = Dataset::with_dlt_clock(1, Source::granted(&path).unwrap(), Clock::Fixed(0));
        let (tx, rx) = mpsc::channel();
        d.start(move |valid| tx.send(valid).unwrap());
        assert!(!rx.recv_timeout(Duration::from_secs(10)).unwrap(), "{name}");
        assert_eq!(d.index.lock().unwrap().error, error);
        d.close();
    }
    // Opening stops when the dataset stops.
    let path = write(dir.path(), "ok.dlt", &workload());
    let d = Dataset::with_dlt_clock(1, Source::granted(&path).unwrap(), Clock::Fixed(0));
    d.stop.store(true, Ordering::Release);
    let (tx, rx) = mpsc::channel();
    d.start(move |valid| tx.send(valid).unwrap());
    assert!(!rx.recv_timeout(Duration::from_secs(10)).unwrap());
    assert_eq!(d.index.lock().unwrap().error, "Opening cancelled");
    d.close();
}
#[test]
fn text_logs_never_read_rows_as_dlt() {
    // A rendered DLT row in a text log keeps the text reading.
    let row = "2026-03-14 09:26:53.589793 1234.5678 0 ECU1 NAV RTE 4242 W Route recalculated\n";
    let d = Dataset::new(1, Source::text(row, "text").unwrap());
    d.start(|_| {});
    wait(|| d.status(0)["complete"] == true);
    assert_eq!(d.status(0)["format"], "text");
    let q = query(&d, 1, &Filters::default());
    let r = &all_rows(&d, &q)[0];
    assert!(r.get("dlt").is_none());
    d.close();
}

/// Native page rows of a DLT file for the mounted UI test.
/// `LOGSTUDIO_WRITE_FIXTURE=1` rewrites the fixture from the current core.
#[test]
fn page_rows_for_the_ui_fixture() {
    let dir = tempfile::tempdir().unwrap();
    let mut raw = vec![];
    let mut m = Message::log(
        3,
        "NAV",
        "RTE",
        &[
            Arg::Ascii(b"Route recalculated:"),
            Arg::Uint(3, 4),
            Arg::Ascii(b"km"),
        ],
    );
    m.write(&mut raw);
    m = Message::log(
        4,
        "HMI",
        "UI",
        &[Arg::Utf8("경로 안내 시작 🙂"), Arg::Bool(true)],
    );
    m.ecu = Some(id("HU"));
    m.count = 1;
    m.micros = 600_000;
    m.write(&mut raw);
    m = Message::log(
        2,
        "AUD",
        "MIX",
        &[Arg::Ascii(b"Route volume error"), Arg::F64(-3.5)],
    );
    m.count = 2;
    m.timestamp = None;
    m.session = None;
    m.write(&mut raw);
    raw.extend_from_slice(b"\x00\x13route?");
    m = Message::log(4, "", "", &[]);
    m.ext = Some((msin(false, 3, 2), 0, id("DA1"), id("DC1")));
    m.payload = with_id(false, 3, &[0, 7, 1]);
    m.count = 3;
    m.write(&mut raw);
    let path = write(dir.path(), "fixture.dlt", &raw);
    let d = open(&path);
    wait(|| d.status(0)["complete"] == true);
    let filters = Filters {
        query: "route".into(),
        ..Filters::default()
    };
    let page = d.page(&query(&d, 1, &filters), 0, 200).unwrap();
    let all = d.page(&query(&d, 2, &Filters::default()), 0, 200).unwrap();
    if std::env::var_os("LOGSTUDIO_WRITE_FIXTURE").is_some() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../tests/dlt-page.json");
        let text =
            serde_json::to_string_pretty(&json!({"rows": all["rows"], "search": page["rows"]}))
                .unwrap();
        std::fs::write(path, text + "\n").unwrap();
    }
    let golden: Value = serde_json::from_str(include_str!("../../tests/dlt-page.json")).unwrap();
    assert_eq!(
        all["rows"], golden["rows"],
        "native fields consumed by the UI fixture"
    );
    assert_eq!(page["rows"], golden["search"]);
    assert_eq!(lines(page["rows"].as_array().unwrap()), vec![1, 3]);
    d.close();
}

fn export(d: &Dataset, q: &Query, kind: ExportKind) -> (Vec<u8>, Exported) {
    let mut out = vec![];
    let e = d
        .export_as(q, kind, &mut out, &AtomicBool::new(false), |_, _| {})
        .unwrap();
    (out, e)
}
#[test]
fn exports_write_messages_or_rows() {
    let dir = tempfile::tempdir().unwrap();
    let raw = workload();
    let path = write(dir.path(), "w.dlt", &raw);
    let d = open(&path);
    wait(|| d.status(0)["complete"] == true);
    let units: Vec<dlt::Unit> = dlt::units(&raw, raw.len()).collect();
    let messages: Vec<u8> = units
        .iter()
        .filter(|u| u.kind == dlt::Kind::Message)
        .flat_map(|u| raw[u.start..u.start + u.len].iter().copied())
        .collect();
    // Every row: the messages, without the three units of other bytes.
    let q = query(&d, 1, &Filters::default());
    let (out, e) = export(&d, &q, ExportKind::Original);
    assert_eq!(out, messages);
    assert_eq!(
        e,
        Exported {
            rows: 8000,
            bytes: messages.len() as u64,
            skipped: 3
        }
    );
    // As text: the rows as shown, each with LF.
    let (out, e) = export(&d, &q, ExportKind::Text);
    let rows = rendered(&raw);
    assert_eq!(String::from_utf8(out).unwrap(), rows.join("\n") + "\n");
    assert_eq!((e.rows, e.skipped), (rows.len() as u64, 0));
    // A filtered export opens again as the same rows.
    let filters = Filters {
        tag: "nav".into(),
        query: "needle".into(),
        ..Filters::default()
    };
    let q = query(&d, 2, &filters);
    let shown: Vec<String> = all_rows(&d, &q)
        .iter()
        .map(|r| r["raw"].as_str().unwrap().to_string())
        .collect();
    let (out, e) = export(&d, &q, ExportKind::Original);
    assert_eq!((e.rows, e.skipped), (shown.len() as u64, 0));
    assert_eq!(rendered(&out), shown);
    d.close();
    // A file of messages only exports to itself.
    let clean: Vec<u8> = (0..500)
        .flat_map(|i| Message::log(4, "A", "C", &[Arg::Uint(i, 4)]).bytes())
        .collect();
    let path = write(dir.path(), "clean.dlt", &clean);
    let d = open(&path);
    wait(|| d.status(0)["complete"] == true);
    let (out, _) = export(&d, &query(&d, 1, &Filters::default()), ExportKind::Original);
    assert_eq!(out, clean);
    d.close();
    // Text logs have no row text apart from their bytes.
    let d = Dataset::new(
        1,
        Source::text("09-21 12:07:30.386 1 2 I T: x\n", "t").unwrap(),
    );
    d.start(|_| {});
    wait(|| d.status(0)["complete"] == true);
    let q = query(&d, 1, &Filters::default());
    let mut sink = vec![];
    let e = d.export_as(
        &q,
        ExportKind::Text,
        &mut sink,
        &AtomicBool::new(false),
        |_, _| {},
    );
    assert_eq!(e.unwrap_err().to_string(), "Invalid export format");
    d.close();
}
#[test]
fn engine_exports_through_a_temporary_file() {
    let dir = tempfile::tempdir().unwrap();
    let raw = workload();
    let path = write(dir.path(), "w.dlt", &raw);
    let e = Arc::new(Engine::default());
    let id = e.begin_open();
    e.open_file(id, &path).unwrap();
    wait(|| e.request(json!({"action":"status"})).unwrap()["complete"] == true);
    e.request(json!({"datasetId":id,"queryId":1,"action":"query","pane":"logs","filters":{"levels":["E"]}}))
        .unwrap();
    wait(|| {
        e.request(json!({"action":"status","pane":"logs"})).unwrap()["query"]["status"]
            == "complete"
    });
    let never = AtomicBool::new(false);
    let out = dir.path().join("errors.dlt");
    let v = e
        .export_as("logs", id, 1, &out, ExportKind::Original, &never, |_, _| {})
        .unwrap();
    let count =
        e.request(json!({"action":"status","pane":"logs"})).unwrap()["query"]["count"].clone();
    assert_eq!(v["rows"], count);
    assert!(v.get("skipped").is_none());
    assert_eq!(v["bytes"], std::fs::metadata(&out).unwrap().len());
    let txt = dir.path().join("errors.txt");
    let v = e
        .export_as("logs", id, 1, &txt, ExportKind::Text, &never, |_, _| {})
        .unwrap();
    assert_eq!(v["rows"], count);
    // Writing over the open file and a cancelled export leave no file.
    let err = e
        .export_as(
            "logs",
            id,
            1,
            &path,
            ExportKind::Original,
            &never,
            |_, _| {},
        )
        .unwrap_err();
    assert_eq!(err.to_string(), "Choose a file other than the open log");
    let cancelled = dir.path().join("cancelled.dlt");
    let err = e
        .export_as(
            "logs",
            id,
            1,
            &cancelled,
            ExportKind::Original,
            &AtomicBool::new(true),
            |_, _| {},
        )
        .unwrap_err();
    assert_eq!(err.to_string(), "Export cancelled");
    let names: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|f| f.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        !names
            .iter()
            .any(|n| n.contains("cancelled") || n.contains("partial")),
        "{names:?}"
    );
    e.close();
}
