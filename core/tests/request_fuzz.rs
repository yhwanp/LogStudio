//! The IPC boundary: random requests, well formed or not, must never panic
//! the engine or leave it stuck, since a panic there fails the desktop
//! command and a hang leaves the page waiting. Errors are fine. A text log
//! with CR, LF and CRLF rows and a COVESA DLT sample are driven alike.
use logstudio_core::Engine;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
}
fn wait(mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !f() {
        assert!(Instant::now() < deadline, "timed out");
        thread::sleep(Duration::from_millis(2));
    }
}
/// A number-like value: in range, at the edges, out of range, or not a
/// number at all.
fn number(r: &mut Rng, span: u64) -> Value {
    match r.below(12) {
        0 => json!(0),
        1 => json!(1),
        2 => json!(span),
        3 => json!(span + 1),
        4 => json!(u64::MAX),
        5 => json!(-1),
        6 => json!(1.5),
        7 => json!("7"),
        8 => Value::Null,
        9 => json!([1]),
        _ => json!(r.below(span as usize + 2)),
    }
}
fn filters(r: &mut Rng) -> Value {
    let long = "x".repeat(5000);
    let words = [
        "",
        "a",
        "ERROR",
        "한글",
        "(",
        "\\",
        "[",
        ".*",
        "1 2",
        "7,",
        " , ",
        "ECU1",
        long.as_str(),
        "(?i)crash",
        "a{99999}",
        "(x{1000}){1000}",
        "[[:alpha:]]+",
        "\\p{Han}",
        "(?-u:\\xff)",
        "(?P<n>a)\\k<n>",
        "(?=a)",
        "12:07",
        "2026-09-30 21:55:30",
        "09-21 12:07:30.5",
        "25:99",
    ];
    let word = |r: &mut Rng| words[r.below(words.len())].to_string();
    let condition = |r: &mut Rng| {
        let mut c = json!({});
        if r.chance(40) {
            c["levels"] = match r.below(4) {
                0 => json!(["E", "W"]),
                1 => json!([]),
                2 => json!(["Q"]),
                _ => json!("E"),
            };
        }
        for k in ["tag", "pid", "tid", "timeFrom", "timeTo"] {
            if r.chance(30) {
                c[k] = if r.chance(85) {
                    json!(word(r))
                } else {
                    number(r, 10)
                };
            }
        }
        c
    };
    let mut f = json!({});
    for k in [
        "tag",
        "excludeTags",
        "pid",
        "tid",
        "query",
        "timeFrom",
        "timeTo",
    ] {
        if r.chance(25) {
            f[k] = if r.chance(85) {
                json!(word(r))
            } else {
                number(r, 10)
            };
        }
    }
    if r.chance(30) {
        f["levels"] = json!(["V", "I"]);
    }
    if r.chance(30) {
        f["include"] = Value::Array((0..r.below(4)).map(|_| condition(r)).collect());
    }
    if r.chance(30) {
        f["exclude"] = Value::Array((0..r.below(3)).map(|_| condition(r)).collect());
    }
    if r.chance(20) {
        f["searchMode"] = json!(["literal", "regex", "glob"][r.below(3)]);
    }
    if r.chance(20) {
        f["caseSensitive"] = json!(r.chance(50));
    }
    if r.chance(20) {
        f["includeRaw"] = json!(r.chance(50));
    }
    if r.chance(20) {
        f["recognizeLineNumbers"] =
            [Value::Null, json!(true), json!(false), json!(1)][r.below(4)].clone();
    }
    if r.chance(5) {
        return number(r, 3);
    }
    f
}
fn drive(e: &Arc<Engine>, id: u64, seed: u64, rounds: usize) {
    let mut r = Rng(seed);
    wait(|| e.request(json!({"action":"status"})).unwrap()["complete"] == true);
    let total = e.request(json!({"action":"status"})).unwrap()["total"]
        .as_u64()
        .unwrap();
    assert!(total > 0);
    let mut query_ids = [1u64, 1u64];
    for pane in 0..2 {
        let name = ["logs", "search"][pane];
        e.request(json!({"datasetId":id,"queryId":1,"action":"query","pane":name,"filters":{}}))
            .unwrap();
    }
    let actions = [
        "status", "query", "page", "locate", "range", "record", "lines", "cancel", "follow",
        "close?", "", "export",
    ];
    let (mut located, mut outcomes) = (0, [0usize; 3]);
    let mut errors: BTreeMap<String, usize> = BTreeMap::new();
    for _ in 0..rounds {
        let pane = r.below(2);
        let mut v = json!({});
        let action = actions[r.below(actions.len())];
        if action == "close?" {
            // Closing would end the run; an unknown action is the point here.
            v["action"] = json!("closed");
        } else if r.chance(97) {
            v["action"] = json!(action);
        } else {
            v["action"] = number(&mut r, 3);
        }
        match r.below(20) {
            0 => {}
            1 => v["pane"] = json!("other"),
            2 => v["pane"] = json!(1),
            _ => v["pane"] = json!(["logs", "search"][pane]),
        }
        match r.below(16) {
            0 => {}
            1 => v["datasetId"] = json!(id + 1),
            2 => v["datasetId"] = json!("x"),
            _ => v["datasetId"] = json!(id),
        }
        match r.below(16) {
            0 => {}
            1 => v["queryId"] = json!(query_ids[pane] + 1),
            2 => v["queryId"] = json!(0),
            _ => v["queryId"] = json!(query_ids[pane]),
        }
        if action == "query" {
            if r.chance(70) {
                v["queryId"] = json!(query_ids[pane] + 1);
            }
            if r.chance(95) {
                v["filters"] = filters(&mut r);
            }
        }
        if action == "follow" {
            v["follow"] =
                [json!(true), json!(false), json!("yes"), Value::Null][r.below(4)].clone();
        }
        for (k, span) in [
            ("start", total),
            ("count", 200),
            ("lineNumber", total),
            ("startByte", 1 << 20),
            ("searchQueryId", query_ids[1]),
        ] {
            if r.chance(50) {
                v[k] = number(&mut r, span);
            }
        }
        if r.chance(50) {
            let n = r.below(6);
            let mut lines: Vec<Value> = (0..n).map(|_| number(&mut r, total)).collect();
            if r.chance(70) {
                lines = (0..n).map(|_| json!(1 + r.below(total as usize))).collect();
                lines.sort_by_key(|v| v.as_u64());
                if r.chance(80) {
                    lines.dedup();
                }
            }
            if r.chance(5) {
                lines = vec![json!(1); 201];
            }
            v["lineNumbers"] = Value::Array(lines);
        }
        // The pane the engine reads: the default when the key is absent.
        let name = v.get("pane").and_then(Value::as_str).unwrap_or("logs");
        if action == "locate" && matches!(name, "logs" | "search") {
            // Let the pane's query finish first, so that the result index
            // can be cross-checked; a query over this small log completes
            // within milliseconds.
            for _ in 0..500 {
                let status = e.request(json!({"action":"status","pane":name})).unwrap();
                if status["query"]["status"] != "running" {
                    break;
                }
                thread::sleep(Duration::from_millis(1));
            }
        }
        let started = Instant::now();
        let reply = e.request(v.clone());
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "request took {:?}: {v}",
            started.elapsed()
        );
        if let (Err(e), "locate") = (&reply, action) {
            *errors.entry(e.to_string()).or_insert(0) += 1;
        }
        if let Ok(reply) = reply {
            // A query that the engine took replaces the pane's current one;
            // a rejected query leaves the previous query in place.
            if action == "query" && reply["query"].is_object() {
                if let Some(taken) = v["queryId"].as_u64() {
                    query_ids[usize::from(name == "search")] = taken;
                }
            }
            if action == "page" {
                if let Some(rows) = reply["rows"].as_array() {
                    let count = v["count"].as_u64().unwrap_or(200).min(200) as usize;
                    assert!(rows.len() <= count, "{v} -> {} rows", rows.len());
                }
            }
            if action == "lines" {
                if let Some(rows) = reply["rows"].as_array() {
                    let wanted: Vec<u64> = v["lineNumbers"]
                        .as_array()
                        .map_or(vec![], |a| a.iter().filter_map(Value::as_u64).collect());
                    let got: Vec<u64> = rows
                        .iter()
                        .map(|r| r["lineNumber"].as_u64().unwrap())
                        .collect();
                    assert!(got.windows(2).all(|w| w[0] < w[1]), "{v} -> {got:?}");
                    assert!(got.iter().all(|l| wanted.contains(l)), "{v} -> {got:?}");
                }
            }
            // A located row is the row its result index pages to, and a row
            // that is no result is passed by the row at that index.
            if action == "locate" {
                outcomes[match () {
                    _ if reply["pending"] == true => 0,
                    _ if reply["incomplete"] == true => 1,
                    _ => 2,
                }] += 1;
            }
            if action == "locate" && reply["pending"] == false && reply["incomplete"] != true {
                let status = e.request(json!({"action":"status","pane":name})).unwrap();
                if status["query"]["status"] == "complete" && status["query"]["id"] == v["queryId"]
                {
                    let (line, index) = (
                        reply["lineNumber"].as_u64().unwrap(),
                        reply["index"].as_u64().unwrap(),
                    );
                    let page = e
                        .request(json!({"action":"page","pane":name,"datasetId":id,"queryId":v["queryId"],"start":index,"count":1}))
                        .unwrap();
                    let at = page["rows"][0]["lineNumber"].as_u64();
                    located += 1;
                    if reply["found"] == true {
                        assert_eq!(at, Some(line), "{v} -> {reply} but {page}");
                    } else {
                        assert!(at.is_none_or(|at| at > line), "{v} -> {reply} but {page}");
                    }
                }
            }
        }
    }
    // How many locates land on a completed query depends on how fast the
    // background queries run (about 140 of 6000 here, 116 on the Windows
    // runner), so the floor only proves the cross-check ran at all.
    assert!(
        located > rounds / 200,
        "{located} locate results cross-checked; pending/incomplete/decided {outcomes:?}, errors {errors:?}"
    );
    // The engine still answers after all that.
    let status = e.request(json!({"action":"status"})).unwrap();
    assert_eq!(status["datasetId"], id, "{status}");
    assert_eq!(status["status"], "ready", "{status}");
}

#[test]
fn random_requests_on_a_text_log_never_panic() {
    let mut text = String::new();
    for i in 0..3000 {
        let br = ["\n", "\r\n", "\r"][i % 3];
        text += &match i % 9 {
            0 => format!("09-21 12:07:{:02}.{:03}  101  {} E AndroidRuntime: crash {i}{br}\tat com.example.C.run(C.java:{i}){br}", i % 60, i % 1000, 7 + i % 3),
            1 => format!("2026-09-30 21:55:{:02}.776 Runner[101:9000] [INFO] [SDK_UI] [Gallery] 한글 {i}{br}", i % 60),
            2 => format!("2026-09-30 21:55:{:02}.776 Runner[3870:bad] malformed {i}{br}  tail{br}", i % 60),
            3 => format!("{i:05}\t09-21 12:07:{:02}.386 101 9 W Num: numbered{br}", i % 60),
            4 => br.to_string(),
            5 => format!("09-21 12:07:{:02}.386 104 11 I Bad\u{fffd}Tag: broken 🙂{br}", i % 60),
            _ => format!("09-21 12:07:{:02}.{:03} {} {} {} {}: message {i} value={}{br}", i % 60, i % 1000, 101 + i % 4, 20 + i % 60, ["V", "D", "I", "W", "E", "F", "A"][i % 7], ["ActivityManager", "WifiService", "네트워크"][i % 3], i * 7),
        };
    }
    let e = Arc::new(Engine::default());
    let id = e.begin_open();
    e.open_text(id, &text, "fuzz").unwrap();
    drive(&e, id, 0x9E37_79B9_7F4A_7C15, 6000);
    e.close();
}

#[test]
fn random_requests_on_a_dlt_file_never_panic() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../tests/fixtures/covesa/dlt-daemon-testfile_extended.dlt");
    let e = Arc::new(Engine::default());
    let id = e.begin_open();
    e.open_file(id, &path).unwrap();
    drive(&e, id, 0xD1B5_4A32_D192_ED03, 4000);
    e.close();
}
