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

// CRLF, CR and LF rows, a damaged UTF-8 byte and a final row without a
// terminator; exports must keep every byte of each selected row.
const LOG: &[u8] = b"06-18 19:24:00.000  1 2 I Keep: one\r\n06-18 19:24:00.050  1 2 D chatty  : drop\r06-18 19:24:00.100  1 2 W Keep: bad \xff byte\nraw line\n06-18 19:24:00.200  1 2 E Keep: last";

fn open(dir: &std::path::Path) -> (Arc<Engine>, u64, std::path::PathBuf) {
    let source = dir.join("source.log");
    std::fs::write(&source, LOG).unwrap();
    let e = Arc::new(Engine::default());
    let id = e.begin_open();
    e.open_file(id, &source).unwrap();
    wait(|| e.request(json!({"action":"status"})).unwrap()["textComplete"] == true);
    (e, id, source)
}

fn query(e: &Engine, id: u64, query_id: u64, filters: Filters) {
    e.request(
        json!({"datasetId":id,"queryId":query_id,"action":"query","pane":"logs","filters":filters}),
    )
    .unwrap();
    wait(|| {
        e.request(json!({"action":"status","pane":"logs"})).unwrap()["query"]["status"]
            == "complete"
    });
}

#[test]
fn export_writes_original_bytes_and_line_endings_of_result_rows() {
    let dir = tempfile::tempdir().unwrap();
    let (e, id, source) = open(dir.path());
    let out = dir.path().join("out.log");
    let never = AtomicBool::new(false);
    query(
        &e,
        id,
        1,
        Filters {
            exclude_tags: "chatty".into(),
            ..Filters::default()
        },
    );
    let mut seen = vec![];
    let result = e
        .export("logs", id, 1, &out, &never, |rows, bytes| {
            seen.push((rows, bytes))
        })
        .unwrap();
    let expected: &[u8] = b"06-18 19:24:00.000  1 2 I Keep: one\r\n06-18 19:24:00.100  1 2 W Keep: bad \xff byte\nraw line\n06-18 19:24:00.200  1 2 E Keep: last";
    assert_eq!(std::fs::read(&out).unwrap(), expected);
    assert_eq!(result, json!({"rows":4,"bytes":expected.len()}));
    assert_eq!(seen.last(), Some(&(4, expected.len() as u64)));

    // Overwrites an existing destination only after a complete write.
    query(
        &e,
        id,
        2,
        Filters {
            levels: vec!["D".into()],
            ..Filters::default()
        },
    );
    e.export("logs", id, 2, &out, &never, |_, _| {}).unwrap();
    assert_eq!(
        std::fs::read(&out).unwrap(),
        b"06-18 19:24:00.050  1 2 D chatty  : drop\r"
    );

    // The open log is never a destination; stale identities are rejected.
    let refused = e
        .export("logs", id, 2, &source, &never, |_, _| {})
        .unwrap_err();
    assert_eq!(refused.to_string(), "Choose a file other than the open log");
    assert_eq!(std::fs::read(&source).unwrap(), LOG);
    assert!(e
        .export("logs", id + 1, 2, &out, &never, |_, _| {})
        .is_err());
    assert!(
        e.export("logs", id, 1, &out, &never, |_, _| {}).is_err(),
        "replaced query"
    );

    // Cancellation leaves neither a partial nor a changed destination.
    let cancelled = AtomicBool::new(true);
    let error = e
        .export(
            "logs",
            id,
            2,
            &dir.path().join("new.log"),
            &cancelled,
            |_, _| {},
        )
        .unwrap_err();
    assert_eq!(error.to_string(), "Export cancelled");
    let names: Vec<Value> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|f| json!(f.unwrap().file_name().to_string_lossy()))
        .collect();
    assert_eq!(names.len(), 2, "{names:?}");
    e.close();
}

#[test]
fn export_requires_completed_results() {
    let dir = tempfile::tempdir().unwrap();
    let (e, id, _) = open(dir.path());
    let never = AtomicBool::new(false);
    // A query that has not been requested for the pane cannot be exported.
    assert!(e
        .export("logs", id, 9, &dir.path().join("x.log"), &never, |_, _| {})
        .is_err());
    assert!(e
        .export("other", id, 9, &dir.path().join("x.log"), &never, |_, _| {})
        .is_err());
    e.close();
}
