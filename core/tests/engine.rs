use logstudio_core::{
    dataset::Dataset,
    search::{Filter, Filters},
    source::{ByteLines, Source},
    Engine,
};
use serde_json::{json, Value};
use std::{
    sync::Arc,
    thread,
    time::{Duration, Instant},
};
fn wait(mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(4);
    while !f() {
        assert!(Instant::now() < deadline, "timed out");
        thread::sleep(Duration::from_millis(2));
    }
}
fn request(e: &Engine, d: u64, q: u64, action: &str, more: Value) -> Value {
    let mut v = json!({"datasetId":d,"queryId":q,"action":action});
    for (k, vv) in more.as_object().unwrap() {
        v[k] = vv.clone();
    }
    e.request(v).unwrap()
}
#[test]
fn physical_and_two_panes() {
    let e = Arc::new(Engine::default());
    let id = e.begin_open();
    e.open_text(id,"09-21 12:07:30.386 007 009 I 한글.Tag: 🙂 COMMON\r\n\r2026-09-30 21:55:53.776261 [INFO] [SDK_UI] [GalleryManager] common\nraw COMMON","test").unwrap();
    wait(|| e.request(json!({"action":"status"})).unwrap()["textComplete"] == true);
    for (pane, filter) in [
        ("logs", Filters::default()),
        (
            "search",
            Filters {
                tag: "gallery".into(),
                query: "COMMON".into(),
                ..Filters::default()
            },
        ),
    ] {
        request(&e, id, 1, "query", json!({"pane":pane,"filters":filter}));
    }
    wait(|| {
        e.request(json!({"action":"status","pane":"search"}))
            .unwrap()["query"]["status"]
            == "complete"
    });
    let a = request(&e, id, 1, "page", json!({"pane":"search"}));
    // The raw last row continues the GalleryManager entry and its tag.
    assert_eq!(a["count"], 2);
    assert_eq!(a["rows"][0]["lineNumber"], 3);
    assert_eq!(a["rows"][0]["tags"], json!(["SDK_UI", "GalleryManager"]));
    assert_eq!(a["rows"][1]["lineNumber"], 4);
    assert_eq!(a["rows"][1]["owner"]["lineNumber"], 3);
    assert_eq!(a["rows"][1]["owner"]["tag"], "SDK_UI");
    wait(|| {
        e.request(json!({"action":"status","pane":"logs"})).unwrap()["query"]["status"]
            == "complete"
    });
    let b = request(&e, id, 1, "page", json!({}));
    assert_eq!(b["rows"].as_array().unwrap().len(), 4);
    assert_eq!(b["rows"][1]["kind"], "blank");
    assert_eq!(
        request(&e, id, 1, "range", json!({"lineNumber":1}))["text"],
        "09-21 12:07:30.386 007 009 I 한글.Tag: 🙂 COMMON"
    );
    e.close();
    assert_eq!(
        e.request(json!({"action":"status"})).unwrap()["empty"],
        true
    );
}
#[test]
fn rollback_and_reopen() {
    let e = Arc::new(Engine::default());
    let id = e.begin_open();
    e.open_text(id, "valid\n", "one").unwrap();
    wait(|| e.request(json!({"action":"status"})).unwrap()["textComplete"] == true);
    let next = e.begin_open();
    e.open_text(next, "bad\0", "invalid").unwrap();
    wait(|| e.request(json!({"action":"status"})).unwrap()["datasetId"] == id);
    e.close();
    let last = e.begin_open();
    e.open_text(last, "new", "two").unwrap();
    wait(|| e.request(json!({"action":"status"})).unwrap()["textComplete"] == true);
    e.close();
}
#[test]
fn validation_event_waits_for_opened_notification() {
    use std::sync::{mpsc, Mutex};
    let e = Arc::new(Engine::default());
    let (entered, entered_rx) = mpsc::channel();
    let (release, release_rx) = mpsc::channel();
    let release_rx = Mutex::new(release_rx);
    let (terminal, terminal_rx) = mpsc::channel();
    e.observe(move |event| {
        if event["status"] == "opened" {
            entered.send(()).unwrap();
            release_rx.lock().unwrap().recv().unwrap();
        } else {
            terminal.send(event).unwrap();
        }
    });
    let id = e.begin_open();
    let engine = e.clone();
    let open = thread::spawn(move || engine.open_text(id, "bad\0", "invalid"));
    entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let early = terminal_rx.recv_timeout(Duration::from_millis(50));
    release.send(()).unwrap();
    open.join().unwrap().unwrap();
    assert!(
        early.is_err(),
        "validation cannot precede the opened notification"
    );
    let event = terminal_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(event["status"], "rollback");
    assert_eq!(event["datasetId"], id);
    e.close();
}

#[test]
fn cancelled_open_reports_candidate_identity_and_restores_previous_source() {
    use std::sync::mpsc;
    let e = Arc::new(Engine::default());
    let id = e.begin_open();
    e.open_text(id, "valid\n", "previous").unwrap();
    wait(|| e.request(json!({"action":"status"})).unwrap()["textComplete"] == true);
    let (events, rx) = mpsc::channel();
    e.observe(move |event| {
        let _ = events.send(event);
    });
    let candidate = e.begin_open();
    e.open_text(candidate, &"raw\n".repeat(4 * 1024 * 1024), "candidate")
        .unwrap();
    e.cancel_open(candidate);
    let opened = rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(opened["status"], "opened");
    let rollback = rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(rollback["status"], "rollback");
    assert_eq!(rollback["datasetId"], candidate);
    assert_eq!(
        e.request(json!({"action":"status"})).unwrap()["datasetId"],
        id
    );
    e.close();
}
#[test]
fn newline_boundaries() {
    for bytes in [b"".as_slice(), b"\n", b"\r", b"\r\n", b"a\r\nb\rc\n\n"] {
        let data = std::str::from_utf8(bytes).unwrap();
        let d = Dataset::new(1, Source::text(data, "test").unwrap());
        d.start(|_| {});
        wait(|| d.index.lock().unwrap().done);
        assert_eq!(
            d.index.lock().unwrap().total,
            ByteLines::new(bytes).count() as u64
        );
        d.close();
    }
}
#[test]
fn bounded_pages_replacement_cancel() {
    let text = "09-21 12:07:30.386 007 009 I Tag: COMMON 한글\n".repeat(60000);
    let d = Dataset::new(1, Source::text(&text, "test").unwrap());
    d.start(|_| {});
    let q = d
        .query(0, 1, Filter::new(Filters::default()).unwrap())
        .unwrap();
    wait(|| q.json()["status"] == "complete");
    assert_eq!(q.json()["count"], 60000);
    let p = d.page(&q, 59900, 200).unwrap();
    assert_eq!(p["rows"].as_array().unwrap().len(), 100);
    assert_eq!(p["rows"][0]["lineNumber"], 59901);
    let q = d
        .query(
            1,
            1,
            Filter::new(Filters {
                query: "no-match".into(),
                ..Filters::default()
            })
            .unwrap(),
        )
        .unwrap();
    q.cancel();
    let q = d
        .query(
            1,
            2,
            Filter::new(Filters {
                query: "한글".into(),
                ..Filters::default()
            })
            .unwrap(),
        )
        .unwrap();
    wait(|| q.json()["status"] == "complete");
    assert_eq!(q.json()["count"], 60000);
    assert!(d
        .query(1, 1, Filter::new(Filters::default()).unwrap())
        .is_err());
    d.close();
}
#[test]
fn grants_and_regex() {
    let temp = tempfile::NamedTempFile::new().unwrap().into_temp_path();
    let source = Source::granted(&temp).unwrap();
    assert_eq!(source.size, 0);
    assert!(Filter::new(Filters {
        query: "(?=x)".into(),
        search_mode: "regex".into(),
        ..Filters::default()
    })
    .is_err());
    assert!(Filter::new(Filters {
        pid: "１２".into(),
        ..Filters::default()
    })
    .is_err());
    let filter = Filter::new(Filters {
        query: "🙂".into(),
        ..Filters::default()
    })
    .unwrap();
    assert_eq!(filter.highlights("a🙂b"), vec![[1, 3]]);
}
#[test]
fn chunk_frontiers_and_invalid_data() {
    for delimiter in ["\r", "\n", "\r\n"] {
        let text = "x".repeat(logstudio_core::source::BLOCK - 1) + delimiter + "한글";
        let d = Dataset::new(1, Source::text(&text, "boundary").unwrap());
        d.start(|_| {});
        wait(|| d.index.lock().unwrap().done);
        let q = d
            .query(0, 1, Filter::new(Filters::default()).unwrap())
            .unwrap();
        wait(|| q.json()["status"] == "complete");
        let p = d.page(&q, 1, 200).unwrap();
        assert_eq!(p["rows"][0]["raw"], "한글");
        d.close();
    }
    let d = Dataset::new(
        1,
        Source::text(&("x".repeat(logstudio_core::source::MAX_LINE + 1)), "long").unwrap(),
    );
    d.start(|_| {});
    wait(|| !d.index.lock().unwrap().error.is_empty());
    d.close();
    let mut f = tempfile::NamedTempFile::new().unwrap();
    use std::io::Write;
    f.write_all(&[0, b'\n']).unwrap();
    let path = f.into_temp_path();
    let d = Dataset::new(1, Source::granted(&path).unwrap());
    d.start(|_| {});
    wait(|| !d.index.lock().unwrap().error.is_empty());
    d.close();
}
#[test]
#[cfg(unix)]
fn source_mutation_and_symlink() {
    let mut f = tempfile::NamedTempFile::new().unwrap();
    use std::io::Write;
    f.write_all(b"one").unwrap();
    let source = Source::granted(f.path()).unwrap();
    // Appended bytes, as a log being written gets, leave the granted ones
    // readable; only following reads further.
    f.write_all(b"two").unwrap();
    assert_eq!(source.read(0, 3).unwrap(), b"one");
    assert!(source.read(0, 6).is_err());
    // Any other change needs reopening.
    std::fs::write(f.path(), b"changed").unwrap();
    assert!(source.read(0, 3).is_err());
    source.close();
    assert!(source.read(0, 0).is_err());
    #[cfg(unix)]
    {
        let dir = tempfile::tempdir().unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(f.path(), &link).unwrap();
        assert!(Source::granted(&link).is_err());
    }
}
// A live log's writer keeps writing while the file is open in the app, and
// following reads what it appends.
#[test]
#[cfg(windows)]
fn windows_grant_shares_source_with_its_writer_and_close_releases_the_file() {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    use std::io::Write;
    file.write_all(b"one").unwrap();
    let path = file.into_temp_path();
    let source = Source::granted(&path).unwrap();
    assert_eq!(source.read(0, 3).unwrap(), b"one");
    let mut writer = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    writer.write_all(b" two").unwrap();
    assert_eq!(source.read(0, 3).unwrap(), b"one");
    assert_eq!(source.grow().unwrap(), Some(7));
    assert_eq!(source.read(0, 7).unwrap(), b"one two");
    drop(writer);
    source.close();
    assert!(source.read(0, 0).is_err());
    std::fs::write(&path, b"changed").unwrap();
    let reopened = Source::granted(&path).unwrap();
    assert_eq!(reopened.read(0, 7).unwrap(), b"changed");
    reopened.close();
    std::fs::remove_file(&path).unwrap();
}
#[test]
fn preview_budgets_and_empty_filtered_page() {
    let row = "09-21 12:07:30.386 007 009 I Tag: ".to_owned() + &"🙂".repeat(20000) + "\n";
    let d = Dataset::new(1, Source::text(&row.repeat(20), "long").unwrap());
    d.start(|_| {});
    wait(|| d.index.lock().unwrap().done);
    let q = d
        .query(0, 1, Filter::new(Filters::default()).unwrap())
        .unwrap();
    wait(|| q.json()["status"] == "complete");
    let p = d.page(&q, 0, 200).unwrap();
    assert!(serde_json::to_vec(&p).unwrap().len() < 513 * 1024);
    assert_eq!(p["rows"][0]["truncated"], true);
    let r = d.range(&q, 1, 65535).unwrap();
    assert!(r["text"].as_str().unwrap().len() <= 65536);
    assert!(r["startByte"].as_u64().unwrap() <= 65535);
    assert!(65535 - r["startByte"].as_u64().unwrap() < 4);
    let q = d
        .query(
            0,
            2,
            Filter::new(Filters {
                pid: "999".into(),
                ..Filters::default()
            })
            .unwrap(),
        )
        .unwrap();
    assert!(d.page(&q, 0, 200).unwrap()["rows"]
        .as_array()
        .unwrap()
        .is_empty());
    d.close();
}
#[test]
fn repeated_cancel_close_is_bounded() {
    for _ in 0..15 {
        let d = Dataset::new(
            1,
            Source::text(&"raw no match\n".repeat(40000), "cancel").unwrap(),
        );
        d.start(|_| {});
        for id in 1..8 {
            let q = d
                .query(
                    1,
                    id,
                    Filter::new(Filters {
                        query: "absent".into(),
                        ..Filters::default()
                    })
                    .unwrap(),
                )
                .unwrap();
            q.cancel();
        }
        d.close();
        assert!(d.stop.load(std::sync::atomic::Ordering::Acquire));
    }
}
#[test]
fn prefix_summary_cache_and_identity() {
    let d=Dataset::new(1,Source::text("001 09-21 12:07:30.386 007 009 I Tag: body\n002 2026-09-30 21:55:53.776261 [INFO] [Tag] body\n","prefix").unwrap());
    d.start(|_| {});
    wait(|| d.index.lock().unwrap().summary.is_some());
    let q = d
        .query(
            0,
            1,
            Filter::new(Filters {
                recognize_line_numbers: Some(true),
                ..Filters::default()
            })
            .unwrap(),
        )
        .unwrap();
    assert!(q.filter.identity());
    wait(|| q.json()["status"] == "complete");
    assert_eq!(d.status(0)["summary"]["parsed"], 2);
    assert_eq!(d.status(0)["summary"]["counts"]["I"], 2);
    assert_eq!(
        d.page(&q, 0, 200).unwrap()["rows"][0]["sourceLineNumber"],
        "001"
    );
    d.close();
}
#[test]
fn automatic_prefix_filters_pages_and_summary() {
    // Numbered and unnumbered rows coexist; numeric prose and stack rows stay raw.
    let raw = "0001 09-21 12:07:30.386 007 009 I Tag: 한글 🙂\r\n09-21 12:07:30.386 007 009 I Tag: body\r0003 2026-09-30 21:55:53.776261 Runner[007:009] [INFO] [Tag] body\n42 ordinary message\n004 at stack.frame\n005 2026-09-30 21:55:53.776261 [ambiguous] body";
    let d = Dataset::new(1, Source::text(raw, "mixed").unwrap());
    d.start(|_| {});
    wait(|| d.index.lock().unwrap().summary.is_some());
    // Raw rows after a matching entry continue it. A recognized numbered row
    // with a malformed stamp (row 6) opens its own orphan entry. Without
    // prefix recognition the numbered first row is an orphan and rows 3-6
    // continue row 2.
    for (id, mode, expected, parsed) in [
        (1, None, 5, 3),
        (2, Some(false), 5, 1),
        (3, Some(true), 5, 3),
    ] {
        let q = d
            .query(
                0,
                id,
                Filter::new(Filters {
                    recognize_line_numbers: mode,
                    pid: "007".into(),
                    tid: "009".into(),
                    tag: "Tag".into(),
                    ..Filters::default()
                })
                .unwrap(),
            )
            .unwrap();
        wait(|| q.json()["status"] == "complete");
        assert_eq!(q.json()["count"], expected);
        assert_eq!(d.status(0)["summary"]["parsed"], parsed);
        let page = d.page(&q, 0, 1).unwrap();
        assert_eq!(page["rows"].as_array().unwrap().len(), 1);
        if mode != Some(false) {
            assert_eq!(page["rows"][0]["sourceLineNumber"], "0001");
            assert_eq!(page["rows"][0]["raw"], raw.lines().next().unwrap());
            assert_eq!(d.page(&q, 1, 1).unwrap()["rows"][0]["lineNumber"], 2);
        }
    }
    let q = d
        .query(0, 4, Filter::new(Filters::default()).unwrap())
        .unwrap();
    wait(|| q.json()["status"] == "complete");
    assert!(q.filter.identity());
    let page = d.page(&q, 0, 200).unwrap();
    for row in &page["rows"].as_array().unwrap()[3..] {
        assert_eq!(row["kind"], "raw");
        assert!(row.get("sourceLineNumber").is_none());
    }
    d.close();
}

#[test]
fn prefix_mode_wire_defaults_and_manual_compatibility() {
    for (wire, expected) in [
        ("{}", None),
        (r#"{"recognizeLineNumbers":null}"#, None),
        (r#"{"recognizeLineNumbers":true}"#, Some(true)),
        (r#"{"recognizeLineNumbers":false}"#, Some(false)),
    ] {
        let f: Filters = serde_json::from_str(wire).unwrap();
        assert_eq!(f.recognize_line_numbers, expected);
        assert_eq!(f.recognizes_line_numbers(), expected != Some(false));
    }
}

#[test]
fn automatic_prefix_keeps_original_detail_search_and_uncertain_rows() {
    let rows = [
        "0001 09-21 12:07:30.386 007 009 I Tag: 42 original 한글 🙂",
        "0002 2026-09-30 21:55:53.776261 Runner[007:009] [INFO] [Tag] body",
        "09-21 12:07:30.386 007 009 I Tag: 123 numeric message",
        "42 ordinary message",
        "004 at stack.frame",
        "0006 2026-09-30 21:55:53.776261 [ambiguous] body",
        "0007 09-21 12:07:30.386 007 009 I Tag without colon",
    ];
    let d = Dataset::new(
        1,
        Source::text(&rows.join("\r\n"), "mixed-prefixes").unwrap(),
    );
    d.start(|_| {});
    wait(|| d.index.lock().unwrap().summary.is_some());
    for (id, mode, parsed) in [(1, None, 3), (2, Some(false), 1), (3, Some(true), 3)] {
        let q = d
            .query(
                0,
                id,
                Filter::new(Filters {
                    recognize_line_numbers: mode,
                    ..Filters::default()
                })
                .unwrap(),
            )
            .unwrap();
        wait(|| q.json()["status"] == "complete");
        assert_eq!(d.status(0)["summary"]["parsed"], parsed);
        let page = d.page(&q, 0, 200).unwrap();
        assert_eq!(page["rows"].as_array().unwrap().len(), rows.len());
        for (i, raw) in rows.iter().enumerate() {
            assert_eq!(page["rows"][i]["raw"], *raw);
            assert_eq!(page["rows"][i]["lineNumber"], i + 1);
            assert_eq!(d.range(&q, i as u64 + 1, 0).unwrap()["text"], *raw);
            if i >= 3 {
                assert_eq!(page["rows"][i]["kind"], "raw");
                assert!(page["rows"][i].get("sourceLineNumber").is_none());
            }
        }
    }
    let q = d
        .query(
            1,
            1,
            Filter::new(Filters {
                query: "0001".into(),
                ..Filters::default()
            })
            .unwrap(),
        )
        .unwrap();
    wait(|| q.json()["status"] == "complete");
    assert_eq!(q.json()["count"], 1);
    let page = d.page(&q, 0, 200).unwrap();
    assert_eq!(page["rows"][0]["sourceLineNumber"], "0001");
    assert_eq!(page["rows"][0]["raw"], rows[0]);
    assert_eq!(page["rows"][0]["highlights"], json!([[0, 4]]));
    assert_eq!(d.range(&q, 1, 0).unwrap()["highlights"], json!([[0, 4]]));
    d.close();
}
