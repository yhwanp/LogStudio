use logstudio_core::{
    dataset::Dataset,
    encoding, parser,
    search::{Filter, Filters},
    source::Source,
};
use serde_json::{json, Value};
use std::{
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

const TAGGED: &str = "2026-09-15 10:47:35.134 Runner[3870:481516] [Memory] heap used = 58.36 MB, remaining = 5824.17 MB";
const UNTAGGED: &str =
    "2026-09-15 10:47:35.184 Runner[3870:481490] Shader cache pool loaded: 0x1004e7a10";
fn wait(mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !done() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
}
fn query(d: &Arc<Dataset>, id: u64, filters: Filters) -> Arc<logstudio_core::dataset::Query> {
    let q = d.query(0, id, Filter::new(filters).unwrap()).unwrap();
    wait(|| q.json()["status"] == "complete");
    q
}

#[test]
fn exact_user_examples_keep_context_and_empty_tag_without_inventing_level() {
    for (raw, physical, tag, tid, message) in [
        (
            TAGGED,
            3,
            "Memory",
            "481516",
            "heap used = 58.36 MB, remaining = 5824.17 MB",
        ),
        (
            UNTAGGED,
            4,
            "",
            "481490",
            "Shader cache pool loaded: 0x1004e7a10",
        ),
    ] {
        let row = parser::parse_line(raw, physical, false).json(usize::MAX);
        assert_eq!(row["kind"], "parsed");
        assert_eq!(row["raw"], raw);
        assert_eq!(row["lineNumber"], physical);
        assert_eq!(row["sourceLineNumber"], Value::Null);
        assert_eq!(row["date"], "2026-09-15");
        assert_eq!(row["year"], "2026");
        assert_eq!(
            row["time"],
            if physical == 3 {
                "10:47:35.134"
            } else {
                "10:47:35.184"
            }
        );
        assert_eq!(row["process"], "Runner");
        assert_eq!(row["pid"], "3870");
        assert_eq!(row["tid"], tid);
        assert_eq!(row["tag"], tag);
        assert_eq!(row["message"], message);
        assert_eq!(row["level"], "U");
        assert_eq!(row["levelText"], Value::Null);
        assert_eq!(
            row["tags"],
            if tag.is_empty() {
                json!([])
            } else {
                json!([tag])
            }
        );
    }
    for prefix in ["4 ", "0004\t", " \t0004 ", "999999999999999999999999\t"] {
        let raw = format!("{prefix}{UNTAGGED}");
        let on = parser::parse_line(&raw, 91, true);
        assert_eq!(on.kind, "parsed");
        assert_eq!(on.line, 91);
        assert_eq!(on.raw, raw);
        assert_eq!(on.source_number, Some(parser::trim(prefix)));
        assert_eq!(on.parsed.unwrap().tag, "");
        assert!(parser::has_source_prefix(&raw));
        let off = parser::parse_line(&raw, 91, false);
        assert_eq!(off.kind, "raw");
        assert_eq!(off.source_number, None);
    }
    let inferred = parser::parse_line(
        "2026-09-15 10:47:35.184 Runner[007:009] ERROR WARNING debug loaded",
        8,
        false,
    )
    .json(usize::MAX);
    assert_eq!(inferred["level"], "U");
    assert_eq!(inferred["pid"], "007");
    assert_eq!(inferred["tid"], "009");
    let explicit = parser::parse_line(
        "2026-09-15 10:47:35.184 Runner[007:009] [INFO] cache pool loaded",
        9,
        false,
    )
    .json(usize::MAX);
    assert_eq!(explicit["level"], "I");
    assert_eq!(explicit["levelText"], "INFO");
    assert_eq!(explicit["tag"], "");
    assert_eq!(explicit["message"], "cache pool loaded");
}

#[test]
fn optional_tag_does_not_accept_malformed_context_or_brackets() {
    for raw in [
        "2026-09-15 10:47:35.184 plain continuation",
        "2026-09-15 10:47:35.184 [Memory] plain continuation",
        "2026-09-15 10:47:35.184 [INFO] missing component",
        "2026-09-15 10:47:35.184 Runner[3870:abc] cache pool loaded",
        "2026-09-15 10:47:35.184 Runner[abc:481490] cache pool loaded",
        "2026-09-15 10:47:35.184 Runner[3870:] cache pool loaded",
        "2026-09-15 10:47:35.184 Runner[3870] cache pool loaded",
        "2026-09-15 10:47:35.184 [3870:481490] cache pool loaded",
        "2026-09-15 10:47:35.184 Runner[3870:481490]cache pool loaded",
        "2026-09-15 10:47:35.18 Runner[3870:481490] cache pool loaded",
        "2026-09-15 10:47:35.184 Runner[3870:481490] ",
        "2026-09-15 10:47:35.184 Runner[3870:481490] [] cache pool loaded",
        "2026-09-15 10:47:35.184 Runner[3870:481490] [ ] cache pool loaded",
        "2026-09-15 10:47:35.184 Runner[3870:481490] [Memory cache pool loaded",
        "2026-09-15 10:47:35.184 Runner[3870:481490] [[Memory]] cache pool loaded",
        "2026-09-15 10:47:35.184 Runner[3870:481490] [INFO] [] cache pool loaded",
        "2026-09-15 10:47:35.184 Runner[3870:481490] body\ncontinuation",
        "2026-09-15 10:47:35.184 Runner[3870:481490] body\u{2028}continuation",
    ] {
        for original in [raw.to_owned(), format!("0004 {raw}")] {
            let row = parser::parse_line(&original, 4, true);
            assert_eq!(row.kind, "raw", "{original}");
            assert_eq!(row.raw, original);
            assert_eq!(row.source_number, None);
            assert!(!parser::has_source_prefix(&original));
        }
    }
}

#[test]
fn native_pages_filters_offsets_search_detail_and_summary_are_consistent() {
    let mut bytes = format!("ordinary raw\n\n{TAGGED}\r\n{UNTAGGED}\r0005\t{UNTAGGED}\n2026-09-15 10:47:35.184 Runner[3870:bad] malformed\n0007 {UNTAGGED} ").into_bytes();
    bytes.extend_from_slice(b"\xff \xef\xbf\xbd");
    bytes.extend_from_slice(" 🙂".as_bytes());
    let file = tempfile::NamedTempFile::new().unwrap().into_temp_path();
    std::fs::write(&file, &bytes).unwrap();
    let source = Source::granted(&file).unwrap();
    assert_eq!(source.read(0, bytes.len()).unwrap(), bytes);
    let d = Dataset::new(1, source);
    d.start(|_| {});
    wait(|| d.index.lock().unwrap().summary.is_some());
    let mut id = 1;
    let q = query(
        &d,
        id,
        Filters {
            query: "Runner|Shader".into(),
            search_mode: "regex".into(),
            pid: "3870".into(),
            include_raw: false,
            ..Filters::default()
        },
    );
    let page = d.page(&q, 0, 200).unwrap();
    let golden: Value =
        serde_json::from_str(include_str!("../../tests/ios-optional-tag-page.json")).unwrap();
    assert_eq!(
        page["rows"], golden["rows"],
        "native fields consumed by the mounted UI fixture"
    );
    assert_eq!(page["count"], 4);
    assert_eq!(
        page["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["lineNumber"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        vec![3, 4, 5, 7]
    );
    for row in page["rows"].as_array().unwrap() {
        let raw = row["raw"].as_str().unwrap();
        let line = row["lineNumber"].as_u64().unwrap();
        assert_eq!(row["kind"], "parsed");
        assert_eq!(row["process"], "Runner");
        assert_eq!(row["pid"], "3870");
        if line != 3 {
            assert_eq!(row["tag"], "");
            assert_eq!(row["tags"], json!([]));
            assert!(row["fieldOffsets"].get("tag").is_none());
            assert!(row["fieldDamage"].get("tag").is_none());
        }
        for key in ["date", "time", "process", "pid", "tid", "message"] {
            let text = row[key].as_str().unwrap();
            let at = if key == "message" {
                raw.rfind(text).unwrap()
            } else {
                raw.find(text).unwrap()
            };
            assert_eq!(
                row["fieldOffsets"][key],
                raw[..at].encode_utf16().count(),
                "{line}: {key}"
            );
        }
        let process = row["fieldOffsets"]["process"].as_u64().unwrap();
        assert!(row["highlights"]
            .as_array()
            .unwrap()
            .contains(&json!([process, process + 6])));
        let detail = d.range(&q, line, 0).unwrap();
        assert_eq!(detail["text"], raw);
        assert_eq!(detail["highlights"], row["highlights"]);
        let start = row["sourceStartByte"].as_u64().unwrap();
        let len = row["rawBytes"].as_u64().unwrap() as usize;
        let original = d.source.read(start, len).unwrap();
        assert_eq!(encoding::decode(&original), raw);
        assert_eq!(detail["sourceStartByte"], start);
        assert_eq!(detail["nextByte"], len);
        assert_eq!(detail["totalBytes"], len);
        if line == 7 {
            assert_eq!(row["encodingDamage"]["bytes"], 1);
            assert_eq!(row["fieldDamage"]["message"]["bytes"], 1);
            assert!(row["fieldDamage"].get("process").is_none());
            assert_eq!(detail["encodingDamage"]["bytes"], 1);
            assert!(raw.ends_with("� � 🙂"));
            assert_eq!(row["sourceLineNumber"], "0007");
        }
    }
    let summary = d.status(0)["summary"].clone();
    assert_eq!(summary["parsed"], 4);
    assert_eq!(summary["raw"], 2);
    assert_eq!(summary["blank"], 1);
    assert_eq!(summary["counts"]["U"], 4);
    if let Ok(path) = std::env::var("LOGSTUDIO_IOS_PAGE_EVIDENCE") {
        std::fs::write(path, serde_json::to_vec_pretty(&page).unwrap()).unwrap();
    }
    for (filters, count) in [
        (
            Filters {
                tag: "memory".into(),
                ..Filters::default()
            },
            1,
        ),
        (
            Filters {
                tag: "Runner".into(),
                ..Filters::default()
            },
            0,
        ),
        (
            Filters {
                tag: "Shader".into(),
                ..Filters::default()
            },
            0,
        ),
        (
            Filters {
                tid: "481490".into(),
                ..Filters::default()
            },
            3,
        ),
        (
            Filters {
                pid: "261".into(),
                ..Filters::default()
            },
            0,
        ),
        (
            Filters {
                query: "Shader cache pool".into(),
                ..Filters::default()
            },
            3,
        ),
        (
            Filters {
                levels: vec!["U".into()],
                include_raw: false,
                ..Filters::default()
            },
            4,
        ),
        (
            Filters {
                levels: vec!["I".into()],
                ..Filters::default()
            },
            0,
        ),
        (
            Filters {
                recognize_line_numbers: Some(false),
                pid: "3870".into(),
                ..Filters::default()
            },
            // Unrecognized, numbered row 5 continues row 4's entry.
            3,
        ),
    ] {
        id += 1;
        let q = query(&d, id, filters);
        assert_eq!(q.json()["count"], count);
    }
    assert_eq!(d.status(0)["summary"]["parsed"], 2);
    assert_eq!(d.status(0)["summary"]["counts"]["U"], 2);
    d.close();
}
