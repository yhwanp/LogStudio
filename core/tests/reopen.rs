use logstudio_core::{search::Filters, source::public_message, Engine};
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
fn status(e: &Engine) -> Value {
    e.request(json!({"action":"status"})).unwrap()
}
fn ready(e: &Engine, id: u64) {
    wait(|| {
        let s = status(e);
        s["datasetId"] == id && s["textComplete"] == true
    });
}
fn query(e: &Engine, id: u64, q: u64, filters: Filters) {
    e.request(json!({"action":"query","datasetId":id,"queryId":q,"filters":filters}))
        .unwrap();
    wait(|| status(e)["query"]["status"] == "complete");
}
fn lines(e: &Engine, id: u64, q: u64, wanted: Value) -> std::io::Result<Value> {
    e.request(json!({"action":"lines","datasetId":id,"queryId":q,"lineNumbers":wanted}))
}

#[test]
fn reopen_reads_the_same_files_in_order_and_releases_the_old_source() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (dir.path().join("b.log"), dir.path().join("a.log"));
    std::fs::write(&a, "a1\na2\n").unwrap();
    std::fs::write(&b, "b1\n").unwrap();
    let e = Arc::new(Engine::default());
    let first = e.begin_open();
    e.open_files(first, &[a.clone(), b.clone()]).unwrap();
    ready(&e, first);
    let grants = e.grants(first).unwrap();
    assert_eq!(
        grants.iter().map(|g| g.path.clone()).collect::<Vec<_>>(),
        [a.clone(), b.clone()]
    );
    assert_eq!(grants.iter().map(|g| g.size).collect::<Vec<_>>(), [6, 3]);
    assert!(grants.iter().all(|g| g.modified.is_some()));

    let second = e.begin_open();
    e.reopen(second, first).unwrap();
    ready(&e, second);
    let s = status(&e);
    assert_eq!(s["name"], "b.log 외 1개");
    assert_eq!(s["total"], 3);
    assert_eq!(s["files"][0]["name"], "b.log");
    // Validation released the rollback source, so its ID names nothing.
    wait(|| e.grants(first).is_none());
    assert!(e.grants(second).is_some());
    let gone = e.reopen(e.begin_open(), first).unwrap_err();
    assert_eq!(
        public_message(&gone, ""),
        "Nothing to reopen; select the file again"
    );
    e.close();
}

#[test]
#[cfg(unix)]
fn a_changed_file_reopens_with_its_new_content() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("grow.log");
    std::fs::write(&path, "one\ntwo\n").unwrap();
    let e = Arc::new(Engine::default());
    let first = e.begin_open();
    e.open_file(first, &path).unwrap();
    ready(&e, first);
    query(&e, first, 1, Filters::default());
    use std::io::Write;
    // An appended row keeps the opened rows readable; a rewrite does not.
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(b"three\n")
        .unwrap();
    let kept = e
        .request(json!({"action":"page","datasetId":first,"queryId":1}))
        .unwrap();
    assert_eq!(kept["rows"].as_array().unwrap().len(), 2);
    std::fs::write(&path, "uno\ntwo\nthree\n").unwrap();
    let changed = e
        .request(json!({"action":"page","datasetId":first,"queryId":1}))
        .unwrap_err();
    assert_eq!(
        public_message(&changed, ""),
        "Source changed; reopen the file"
    );
    let second = e.begin_open();
    e.reopen(second, first).unwrap();
    ready(&e, second);
    assert_eq!(status(&e)["total"], 3);
    assert_eq!(e.grants(second).unwrap()[0].size, 14);
    e.close();
}

#[test]
fn pasted_text_cannot_be_reopened() {
    let e = Arc::new(Engine::default());
    let id = e.begin_open();
    e.open_text(id, "text\n", "pasted").unwrap();
    ready(&e, id);
    assert!(e.grants(id).is_none());
    let error = e.reopen(e.begin_open(), id).unwrap_err();
    assert_eq!(
        public_message(&error, ""),
        "Nothing to reopen; select the file again"
    );
    // The failed reopen leaves the open text in place.
    assert_eq!(status(&e)["datasetId"], id);
    e.close();
}

#[test]
fn lines_read_rows_by_physical_number_whatever_the_filter() {
    let text = "09-21 12:07:30.386 007 009 I Tag: first\n{\n09-21 12:07:31.000 007 009 E Other: second\nraw tail\n";
    let e = Arc::new(Engine::default());
    let id = e.begin_open();
    e.open_text(id, text, "rows").unwrap();
    ready(&e, id);
    query(
        &e,
        id,
        1,
        Filters {
            levels: vec!["E".into()],
            ..Filters::default()
        },
    );
    let page = lines(&e, id, 1, json!([1, 2, 4, 9])).unwrap();
    let rows = page["rows"].as_array().unwrap();
    assert_eq!(page["next"], 3, "line 9 is past the end");
    assert_eq!(
        rows.iter()
            .map(|r| r["lineNumber"].clone())
            .collect::<Vec<_>>(),
        [json!(1), json!(2), json!(4)]
    );
    assert_eq!(rows[0]["kind"], "parsed");
    assert_eq!(rows[0]["tag"], "Tag");
    assert_eq!(rows[0]["pid"], "007");
    assert_eq!(rows[1]["raw"], "{");
    assert_eq!(rows[1]["owner"]["lineNumber"], 1);
    assert_eq!(rows[2]["owner"]["level"], "E");
    assert!(rows.iter().all(|r| r.get("highlights").is_none()));
    for bad in [
        json!([2, 1]),
        json!([0]),
        json!([1, 1]),
        json!("1"),
        json!((1..=201).collect::<Vec<u64>>()),
    ] {
        let error = lines(&e, id, 1, bad).unwrap_err();
        assert_eq!(public_message(&error, ""), "Invalid lineNumbers");
    }
    assert_eq!(
        public_message(&lines(&e, id, 2, json!([1])).unwrap_err(), ""),
        "Stale query"
    );
    e.close();
}

#[test]
fn lines_answer_a_prefix_within_the_wire_budget() {
    let long = format!("09-21 12:07:30.386 007 009 I Tag: {}\n", "가".repeat(5000));
    let e = Arc::new(Engine::default());
    let id = e.begin_open();
    e.open_text(id, &long.repeat(200), "long").unwrap();
    ready(&e, id);
    query(&e, id, 1, Filters::default());
    let all: Vec<u64> = (1..=200).collect();
    let page = lines(&e, id, 1, json!(all)).unwrap();
    let next = page["next"].as_u64().unwrap() as usize;
    assert!(next > 0 && next < 200, "{next}");
    assert_eq!(page["rows"].as_array().unwrap().len(), next);
    assert_eq!(page["rows"][0]["truncated"], true);
    let rest = lines(&e, id, 1, json!(all[next..])).unwrap();
    assert_eq!(rest["rows"][0]["lineNumber"], next as u64 + 1);
    e.close();
}

#[test]
fn combined_rows_name_their_file() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (dir.path().join("1.log"), dir.path().join("2.log"));
    std::fs::write(&a, "a1\na2").unwrap();
    std::fs::write(&b, "b1\n").unwrap();
    let e = Arc::new(Engine::default());
    let id = e.begin_open();
    e.open_files(id, &[a, b]).unwrap();
    ready(&e, id);
    query(&e, id, 1, Filters::default());
    let page = lines(&e, id, 1, json!([2, 3])).unwrap();
    assert_eq!(page["rows"][0]["raw"], "a2");
    assert_eq!(page["rows"][1]["file"], 1);
    assert_eq!(page["rows"][1]["fileLine"], 1);
    e.close();
}
