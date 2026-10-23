use logstudio_core::{search::Filters, Engine};
use serde_json::{json, Value};
use std::{
    sync::{atomic::AtomicBool, Arc},
    thread,
    time::{Duration, Instant},
};

fn wait(mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !done() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
}
fn request(e: &Engine, d: u64, q: u64, action: &str, more: Value) -> Value {
    let mut v = json!({"datasetId":d,"queryId":q,"action":action});
    for (k, vv) in more.as_object().unwrap() {
        v[k] = vv.clone();
    }
    e.request(v).unwrap()
}
fn query(e: &Engine, d: u64, pane: &str, id: u64, filters: Filters) -> Vec<Value> {
    request(e, d, id, "query", json!({"pane":pane,"filters":filters}));
    wait(|| request(e, d, id, "status", json!({"pane":pane}))["query"]["status"] == "complete");
    request(e, d, id, "page", json!({"pane":pane}))["rows"]
        .as_array()
        .unwrap()
        .clone()
}
fn lines(rows: &[Value]) -> Vec<u64> {
    rows.iter()
        .map(|r| r["lineNumber"].as_u64().unwrap())
        .collect()
}

// iOS rows whose UTF-8 was read as Mac OS Roman ("인사" as "Ïù∏ÏÇ¨", "안녕"
// as "ÏïàÎÖï") show, search and filter as the original text, CRLF rows and
// damage marks included, while exports keep the source bytes.
const LOG: &[u8] = "2026-09-15 10:47:35.134 Maps[3870:481516] [Ïù∏ÏÇ¨] Greeting(text : ÏïàÎÖï, city : ÎåÄÏ†Ñ) \u{f8ff}üôÇ\r\n\
2026-09-15 10:47:35.135 Maps[3870:481516] [Cache] café Grüße “quoted” – 5 °C\n\
2026-09-15 10:47:35.136 Maps[3870:481516] [Ïù∏ÏÇ¨] bad \u{ff}"
    .as_bytes();

#[test]
fn mac_roman_mojibake_shows_searches_and_filters_as_its_text() {
    // U+00FF is "ÿ" (0xD8 in Mac OS Roman); the row's damage is a real
    // invalid byte placed after it.
    let log = [LOG, b"\xff ", "ÏïàÎÖï\n".as_bytes()].concat();
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("ios.log");
    std::fs::write(&source, &log).unwrap();
    let e = Arc::new(Engine::default());
    let id = e.begin_open();
    e.open_file(id, &source).unwrap();
    wait(|| e.request(json!({"action":"status"})).unwrap()["textComplete"] == true);

    let all = query(&e, id, "logs", 1, Filters::default());
    assert_eq!(
        all[0]["raw"],
        "2026-09-15 10:47:35.134 Maps[3870:481516] [인사] Greeting(text : 안녕, city : 대전) 🙂"
    );
    assert_eq!(all[0]["tag"], "인사");
    assert_eq!(all[0]["message"], "Greeting(text : 안녕, city : 대전) 🙂");
    assert!(all[0].get("encodingDamaged").is_none());
    assert_eq!(
        all[1]["raw"],
        "2026-09-15 10:47:35.135 Maps[3870:481516] [Cache] café Grüße “quoted” – 5 °C",
        "ordinary accented text stays"
    );
    // "ÿ" before the invalid byte stays: it starts no valid run.
    assert_eq!(
        all[2]["raw"],
        "2026-09-15 10:47:35.136 Maps[3870:481516] [인사] bad ÿ\u{fffd} 안녕"
    );
    assert_eq!(all[2]["encodingDamaged"], true);
    let at = "2026-09-15 10:47:35.136 Maps[3870:481516] [인사] bad ÿ"
        .encode_utf16()
        .count();
    let mut mask = vec![0u8; at / 8 + 1];
    mask[at / 8] = 1 << (at % 8);
    let hex: String = mask.iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(all[2]["encodingDamage"]["mask"], hex);

    // Literal and regex searches match the repaired text and highlight it.
    let found = query(
        &e,
        id,
        "search",
        2,
        Filters {
            query: "안녕".into(),
            ..Filters::default()
        },
    );
    assert_eq!(lines(&found), vec![1, 3]);
    let at = "2026-09-15 10:47:35.134 Maps[3870:481516] [인사] Greeting(text : "
        .encode_utf16()
        .count();
    assert_eq!(found[0]["highlights"], json!([[at, at + 2]]));
    let regex = query(
        &e,
        id,
        "search",
        3,
        Filters {
            query: "안녕.+대전\\) 🙂$".into(),
            search_mode: "regex".into(),
            ..Filters::default()
        },
    );
    assert_eq!(lines(&regex), vec![1]);
    let garbled = query(
        &e,
        id,
        "search",
        4,
        Filters {
            query: "ÏïàÎÖï".into(),
            ..Filters::default()
        },
    );
    assert!(garbled.is_empty(), "searches read the displayed text");

    // Tag filters and the field metadata read the repaired tag too.
    let tagged = query(
        &e,
        id,
        "logs",
        5,
        Filters {
            tag: "인사".into(),
            ..Filters::default()
        },
    );
    assert_eq!(lines(&tagged), vec![1, 3]);

    // Details name a repair; exports keep the source bytes.
    let detail = request(&e, id, 5, "range", json!({"pane":"logs","lineNumber":1}));
    assert_eq!(detail["text"], all[0]["raw"]);
    assert_eq!(detail["encodingRepaired"], true);
    let record = request(&e, id, 5, "record", json!({"pane":"logs","lineNumber":1}));
    assert_eq!(record["encodingRepaired"], true);
    let plain = request(&e, id, 5, "range", json!({"pane":"logs","lineNumber":2}));
    assert!(plain.get("encodingRepaired").is_none());
    let out = dir.path().join("out.log");
    e.export("logs", id, 5, &out, &AtomicBool::new(false), |_, _| {})
        .unwrap();
    let rows: Vec<&[u8]> = log.split_inclusive(|&b| b == b'\n').collect();
    assert_eq!(std::fs::read(&out).unwrap(), [rows[0], rows[2]].concat());
    e.close();
}

// Only iOS logs are repaired: an Android file keeps its text, merged files
// are decided one by one, and pasted iOS text repairs as a file does.
#[test]
fn only_ios_logs_are_repaired() {
    let ios = "2026-09-15 10:47:35.134 Maps[3870:481516] [Ïù∏ÏÇ¨] text : ÏïàÎÖï\n";
    let android = "09-15 10:47:35.135  3870  3870 I Maps: text : ÏïàÎÖï\n";
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (dir.path().join("a.log"), dir.path().join("b.log"));
    std::fs::write(&a, ios).unwrap();
    std::fs::write(&b, android).unwrap();
    let e = Arc::new(Engine::default());
    let raws = |e: &Engine, d: u64| -> Vec<Value> {
        query(e, d, "logs", 1, Filters::default())
            .iter()
            .map(|r| r["raw"].clone())
            .collect()
    };
    let open = |paths: &[std::path::PathBuf]| {
        let id = e.begin_open();
        e.open_files(id, paths).unwrap();
        wait(|| e.request(json!({"action":"status"})).unwrap()["textComplete"] == true);
        id
    };
    let id = open(std::slice::from_ref(&b));
    assert_eq!(raws(&e, id), vec![json!(android.trim_end())]);
    let found = query(
        &e,
        id,
        "search",
        2,
        Filters {
            query: "안녕".into(),
            ..Filters::default()
        },
    );
    assert!(found.is_empty(), "Android rows are searched as they are");

    let id = open(&[a.clone(), b.clone()]);
    let repaired = "2026-09-15 10:47:35.134 Maps[3870:481516] [인사] text : 안녕";
    assert_eq!(
        raws(&e, id),
        vec![json!(repaired), json!(android.trim_end())]
    );
    let found = query(
        &e,
        id,
        "search",
        2,
        Filters {
            query: "안녕".into(),
            ..Filters::default()
        },
    );
    assert_eq!(lines(&found), vec![1]);

    let id = e.begin_open();
    e.open_text(id, ios, "pasted").unwrap();
    wait(|| e.request(json!({"action":"status"})).unwrap()["textComplete"] == true);
    assert_eq!(raws(&e, id), vec![json!(repaired)]);
    e.close();
}
