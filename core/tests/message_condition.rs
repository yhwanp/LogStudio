//! Message conditions: a condition's text the row's message contains,
//! ignoring case as the search does, as a string or a pattern. A DLT row's
//! message is its payload, read from rendered rows.
use logstudio_core::{
    dataset::{Dataset, Query},
    dlt::{
        synth::{id, Arg, Message},
        Clock, Decoder,
    },
    parser,
    search::{Filter, Filters},
    source::{Format, Source},
};
use serde_json::{json, Value};
use std::{
    path::Path,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

fn filter(v: Value) -> Filter {
    Filter::new(serde_json::from_value(v).unwrap()).unwrap()
}
fn shown(f: &Filter, raw: &str) -> bool {
    f.fields_match(&parser::parse_line(raw, 1, true))
}
fn wait(mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !done() {
        assert!(Instant::now() < deadline, "timed out");
        thread::sleep(Duration::from_millis(1));
    }
}

const ANDROID: &str = "06-18 19:24:05.300  1201 1208 W ActivityManager: Frame skipped 3 (main)";
const IOS: &str = "2026-06-18 19:24:05.300 Runner[101:9000] [INFO] [Net] [Comp] ÉCOLE 연결 실패";

#[test]
fn a_message_condition_reads_the_message_only_ignoring_case() {
    let show = |message: &str| filter(json!({"include":[{"message":message}]}));
    assert!(shown(&show("frame SKIPPED"), ANDROID));
    assert!(
        !shown(&show("ActivityManager"), ANDROID),
        "the tag is not the message"
    );
    assert!(!shown(&show("19:24"), ANDROID), "nor the time");
    // Unicode simple case folding, as the search; Korean has no case.
    assert!(shown(&show("école"), IOS));
    assert!(shown(&show("연결"), IOS));
    assert!(
        !shown(&show("Runner"), IOS),
        "the process is not the message"
    );
    // A string is literal; a pattern follows the search's grammar.
    assert!(!shown(&show("skipped ."), ANDROID));
    assert!(shown(&show("(main)"), ANDROID));
    let pattern = |p: &str| filter(json!({"include":[{"message":p,"messageRegex":true}]}));
    assert!(shown(&pattern(r"skipped \d+ \(main\)$"), ANDROID));
    assert!(
        !shown(&pattern(r"(?-i)FRAME"), ANDROID),
        "an inline flag can ask for case"
    );
    assert!(shown(&pattern(r"^frame"), ANDROID));
    // A hiding condition hides rows whose message contains the text.
    let hide = filter(json!({"exclude":[{"message":"skipped"}]}));
    assert!(!shown(&hide, ANDROID) && shown(&hide, IOS));
    // With other criteria every one must match.
    let both = filter(json!({"include":[{"levels":["E"],"message":"skipped"}]}));
    assert!(!shown(&both, ANDROID));
    // Rows without fields match no condition.
    assert!(!shown(&show("skipped"), "  frame skipped in a raw row"));
}

#[test]
fn message_conditions_take_a_normal_form_and_report_bad_patterns() {
    let f = filter(
        json!({"include":[{"message":"","messageRegex":true},{"message":" spaced ","messageRegex":false}]}),
    );
    let empty = &f.fields.include[0];
    assert!(
        empty.message.is_empty() && !empty.message_regex,
        "an empty message has no mode"
    );
    assert_eq!(
        f.fields.include[1].message, " spaced ",
        "a message is used as typed"
    );
    assert_eq!(f.shows().len(), 1);
    assert!(!f.only_fields() && !f.only_levels());
    let error = Filter::new(
        serde_json::from_value::<Filters>(json!({"exclude":[{"message":"(","messageRegex":true}]}))
            .unwrap(),
    )
    .err()
    .unwrap()
    .to_string();
    assert!(
        error.starts_with("Invalid or unsupported message pattern: "),
        "{error}"
    );
    let long = serde_json::from_value::<Filters>(json!({"include":[{"message":"x".repeat(4001)}]}))
        .unwrap();
    assert_eq!(
        Filter::new(long).err().unwrap().to_string(),
        "Filter budget exceeded"
    );
    // A string with a line break matches no message.
    assert!(!shown(
        &filter(json!({"include":[{"message":"a\nb"}]})),
        ANDROID
    ));
}

fn query(d: &Arc<Dataset>, id: u64, v: Value) -> Arc<Query> {
    let q = d.query(0, id, filter(v)).unwrap();
    wait(|| q.json()["status"] == "complete");
    q
}
fn lines(d: &Dataset, q: &Query) -> Vec<u64> {
    let count = q.json()["count"].as_u64().unwrap();
    let mut out = vec![];
    while (out.len() as u64) < count {
        let page = d.page(q, out.len() as u64, 200).unwrap();
        let rows = page["rows"].as_array().unwrap();
        assert!(!rows.is_empty());
        out.extend(rows.iter().map(|r| r["lineNumber"].as_u64().unwrap()));
    }
    out
}

#[test]
fn continuation_rows_follow_their_entry_across_blocks() {
    let mut log = String::new();
    for i in 0..20_000 {
        let message = if i % 5 == 0 {
            "crash in worker"
        } else {
            "tick"
        };
        log.push_str(&format!(
            "06-18 19:24:{:02}.000  101 7 E App: {message} {i}\n",
            i % 60
        ));
        if i % 5 == 0 {
            log.push_str("\tat com.example.Worker.run(Worker.java:1)\n");
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.log");
    std::fs::write(&path, &log).unwrap();
    let d = Dataset::new(1, Source::granted(&path).unwrap());
    d.start(|_| {});
    wait(|| d.index.lock().unwrap().done);
    assert!(d.index.lock().unwrap().blocks.len() > 3);
    let shows = lines(&d, &query(&d, 1, json!({"include":[{"message":"CRASH"}]})));
    assert_eq!(shows.len(), 8_000, "each crash and its continuation row");
    let hides = lines(&d, &query(&d, 2, json!({"exclude":[{"message":"crash"}]})));
    assert_eq!(hides.len(), 16_000);
    // A message only a continuation row has matches nothing.
    let q = query(&d, 3, json!({"include":[{"message":"Worker.java"}]}));
    assert_eq!(q.json()["count"], 0);
    d.close();
}

#[test]
fn dlt_payloads_decide_message_conditions() {
    let mut raw = vec![];
    for i in 0..30_000u32 {
        let text = if i % 11 == 0 {
            format!("경로 재탐색 {i}")
        } else {
            format!("value {i}")
        };
        let mut m = Message::log(
            (i % 6 + 1) as u8,
            ["NAV", "AUD"][(i % 2) as usize],
            "CTX",
            &[Arg::Utf8(&text)],
        );
        m.seconds = 1_773_480_413 + i / 10;
        m.ecu = Some(id("ECU1"));
        m.write(&mut raw);
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.dlt");
    std::fs::write(&path, &raw).unwrap();
    let d = Dataset::with_dlt_clock(
        1,
        Source::granted(Path::new(&path)).unwrap(),
        Clock::Fixed(0),
    );
    d.start(|_| {});
    wait(|| d.index.lock().unwrap().done);
    let mut text = vec![];
    Decoder::new(Clock::Fixed(0)).render(&raw, 0, &mut text);
    let rows: Vec<String> = String::from_utf8(text)
        .unwrap()
        .split_terminator('\n')
        .map(String::from)
        .collect();
    let mut id = 0;
    for v in [
        json!({"include":[{"message":"경로"}]}),
        json!({"exclude":[{"message":"VALUE 1"}]}),
        json!({"include":[{"tag":"nav","message":r"value \d*8$","messageRegex":true}]}),
        json!({"include":[{"levels":["E"]},{"message":"재탐색 99"}],"exclude":[{"tag":"aud","message":"value"}]}),
    ] {
        let f = filter(v.clone());
        let want: Vec<u64> = rows
            .iter()
            .enumerate()
            .filter(|(i, row)| {
                f.fields_match(&parser::parse_in(Format::Dlt, row, *i as u64 + 1, true))
            })
            .map(|(i, _)| i as u64 + 1)
            .collect();
        assert!(
            !want.is_empty() && want.len() < rows.len(),
            "{v} {:?}",
            &rows[..3]
        );
        id += 1;
        let q = query(&d, id, v.clone());
        assert_eq!(lines(&d, &q), want, "{v}");
        // The logs pane also learns whether each condition has rows.
        wait(|| !q.json()["conditions"].to_string().contains("pending"));
    }
    d.close();
}
