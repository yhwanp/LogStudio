//! A filter as large as the page lets one be is a query like any other. The
//! page counts UTF-16 units and the engine bytes, and JSON adds escapes, so
//! Korean text and text of quotes, backslashes and control characters are
//! the cases to hold.
use logstudio_core::{
    search::{Filter, Filters, MAX_CONDITIONS},
    Engine,
};
use serde_json::{json, Value};
use std::{
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

fn wait(mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !f() {
        assert!(Instant::now() < deadline, "timed out");
        thread::sleep(Duration::from_millis(2));
    }
}
/// `n` UTF-16 units: `text` repeated and cut, `n` in all.
fn units(text: &str, n: usize) -> String {
    let all: Vec<u16> = text.encode_utf16().cycle().take(n).collect();
    String::from_utf16(&all).unwrap()
}
/// An open log, whose file lasts as long.
struct Log {
    e: Arc<Engine>,
    id: u64,
    _dir: tempfile::TempDir,
}
impl Drop for Log {
    fn drop(&mut self) {
        self.e.close();
    }
}
fn engine(text: &str) -> Log {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.log");
    std::fs::write(&path, text).unwrap();
    let e = Arc::new(Engine::default());
    let id = e.begin_open();
    e.open_files(id, &[path]).unwrap();
    wait(|| {
        let s = e.request(json!({"action":"status"})).unwrap();
        s["datasetId"] == id && s["textComplete"] == true
    });
    Log { e, id, _dir: dir }
}
fn run(e: &Engine, id: u64, qid: u64, filters: Value) -> Value {
    e.request(json!({"action":"query","datasetId":id,"queryId":qid,"filters":filters}))
        .unwrap();
    let mut q = Value::Null;
    wait(|| {
        q = e.request(json!({"action":"status"})).unwrap()["query"].take();
        q["status"] == "complete"
    });
    q
}
fn error(filters: Value) -> String {
    Filter::new(serde_json::from_value::<Filters>(filters).unwrap())
        .err()
        .expect("refused")
        .to_string()
}

#[test]
fn every_message_the_page_may_hide_fits_in_one_query() {
    for text in ["abc ", "가나다라", "\"\\", "\u{1}\t", "한🙂"] {
        // Every hidden message is a row of the log, beside a row that stays.
        let messages: Vec<String> = (0..MAX_CONDITIONS)
            .map(|i| format!("{i:03}{}", units(text, 997)))
            .collect();
        let mut log = String::from("06-18 19:24:00.000  1 2 I Tag: stays\n");
        for m in &messages {
            log += &format!("06-18 19:24:01.000  1 2 I Tag: {m}\n");
        }
        let Log { e, id, .. } = &engine(&log);
        let exclude: Vec<Value> = messages.iter().map(|m| json!({"message":m})).collect();
        let q = run(e, *id, 1, json!({"include":[],"exclude":exclude}));
        assert_eq!(q["count"], 1, "{text:?}");
    }
}

#[test]
fn every_field_at_the_page_bound_fits_in_one_query() {
    // As the page sends a condition, escapes and all.
    for text in ["a", "가", "\"", "\\", "\u{1}", "🙂"] {
        for regex in [false, true] {
            // Patterns are whole ones: an even run of backslashes is.
            let id_text = units(text, 1000);
            let condition = json!({
                "levels":["V","D","I","W","E","F","A","U"],
                "tag":units(text, 4000),"tagRegex":regex,
                "pid":id_text,"pidRegex":true,"tid":id_text,"tidRegex":true,
                "ecu":id_text,"ecuRegex":regex,"apid":id_text,"apidRegex":regex,
                "ctid":id_text,"ctidRegex":regex,
                "timeFrom":format!("06-18{}19:24", "\u{3000}".repeat(54)),"timeTo":"",
                "message":units(text, 1000),"messageRegex":regex,
            });
            let Log { e, id, .. } = &engine("06-18 19:24:00.000  1 2 I Tag: stays\n");
            let q = run(
                e,
                *id,
                1,
                json!({"include":vec![condition; MAX_CONDITIONS],"query":units(text, 1000)}),
            );
            assert_eq!(q["count"], 0, "{text:?}");
        }
    }
}

#[test]
fn a_filter_over_its_bounds_says_which_one_and_leaves_the_query_in_place() {
    for (key, n) in [
        ("tag", 12_001),
        ("pid", 4001),
        ("tid", 4001),
        ("ecu", 4001),
        ("apid", 4001),
        ("ctid", 4001),
        ("message", 4001),
        ("timeFrom", 193),
        ("timeTo", 193),
    ] {
        let name = if key.starts_with("time") { "time" } else { key };
        let name = match name {
            "tag" | "message" | "time" => name.to_string(),
            id => id.to_uppercase(),
        };
        let mut c = json!({});
        c[key] = json!("1".repeat(n));
        assert_eq!(
            error(json!({"exclude":[c]})),
            format!("Filter budget exceeded: {name}")
        );
    }
    assert_eq!(
        error(json!({"include":vec![json!({"tag":"a"}); MAX_CONDITIONS + 1]})),
        "Filter budget exceeded: conditions"
    );
    assert_eq!(
        error(json!({"query":"x".repeat(4001)})),
        "Filter budget exceeded: query"
    );
    let Log { e, id, .. } = &engine("06-18 19:24:00.000  1 2 I Tag: stays\n");
    run(e, *id, 1, json!({"include":[{"tag":"Tag"}]}));
    let refused = e.request(json!({"action":"query","datasetId":id,"queryId":2,"filters":{"exclude":[{"message":"x".repeat(4001)}]}}));
    assert_eq!(
        refused.err().unwrap().to_string(),
        "Filter budget exceeded: message"
    );
    let s = e.request(json!({"action":"status"})).unwrap();
    assert_eq!(s["query"]["id"], 1);
    assert_eq!(s["query"]["count"], 1);
}

#[test]
fn only_a_query_filter_is_outside_the_request_bound() {
    let Log { e, id, .. } = &engine("06-18 19:24:00.000  1 2 I Tag: stays\n");
    run(e, *id, 1, json!({}));
    let long = "x".repeat(20_000);
    for request in [
        json!({"action":"status","note":long}),
        json!({"action":"page","datasetId":id,"queryId":1,"filters":{"query":long}}),
        json!({"action":"query","datasetId":id,"queryId":2,"filters":{},"note":long}),
        json!([long]),
        json!(long),
    ] {
        assert_eq!(
            e.request(request).err().unwrap().to_string(),
            "Request budget exceeded"
        );
    }
    // A query without a filter is refused as before, not taken as an empty one.
    assert!(e
        .request(json!({"action":"query","datasetId":id,"queryId":2}))
        .is_err());
}
