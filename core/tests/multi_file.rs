use logstudio_core::{search::Filters, Engine};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
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

// File ends must close rows: `a` ends with a lone CR that the LF starting `c`
// must not join, `b` is empty, and `c` ends without a terminator.
const FILES: [(&str, &[u8]); 4] = [
    (
        "a.log",
        b"06-18 19:24:00.000  1 2 I TagA: a1\r\n06-18 19:24:01.000  1 2 E TagA: a2\r",
    ),
    ("b.log", b""),
    ("c.log", b"\n06-18 19:24:02.000  3 4 E TagC: c1"),
    ("d.log", b"06-18 19:24:03.000  5 6 W TagD: d1\n"),
];

fn write(dir: &std::path::Path) -> Vec<PathBuf> {
    FILES
        .iter()
        .map(|(name, bytes)| {
            let path = dir.join(name);
            std::fs::write(&path, bytes).unwrap();
            path
        })
        .collect()
}

fn open(paths: &[PathBuf]) -> (Arc<Engine>, u64) {
    let e = Arc::new(Engine::default());
    let id = e.begin_open();
    e.open_files(id, paths).unwrap();
    wait(|| e.request(json!({"action":"status"})).unwrap()["textComplete"] == true);
    (e, id)
}

fn query(e: &Engine, id: u64, pane: &str, query_id: u64, filters: Filters) -> Value {
    e.request(
        json!({"datasetId":id,"queryId":query_id,"action":"query","pane":pane,"filters":filters}),
    )
    .unwrap();
    wait(|| {
        e.request(json!({"action":"status","pane":pane})).unwrap()["query"]["status"] == "complete"
    });
    e.request(json!({"datasetId":id,"queryId":query_id,"action":"page","pane":pane,"start":0}))
        .unwrap()
}

#[test]
fn files_open_in_order_as_one_log_without_joining_rows() {
    let dir = tempfile::tempdir().unwrap();
    let (e, id) = open(&write(dir.path()));
    let status = e.request(json!({"action":"status"})).unwrap();
    assert_eq!(status["name"], "a.log 외 3개");
    assert_eq!(status["total"], 5);
    assert_eq!(
        status["size"],
        FILES.iter().map(|f| f.1.len()).sum::<usize>()
    );
    let files: Vec<(String, u64)> = status["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            (
                f["name"].as_str().unwrap().into(),
                f["firstLine"].as_u64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        files,
        [
            ("a.log".into(), 1),
            ("b.log".into(), 3),
            ("c.log".into(), 3),
            ("d.log".into(), 5)
        ]
    );

    let page = query(&e, id, "logs", 1, Filters::default());
    let rows: Vec<(u64, &str, u64, u64)> = page["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["lineNumber"].as_u64().unwrap(),
                r["raw"].as_str().unwrap(),
                r["file"].as_u64().unwrap(),
                r["fileLine"].as_u64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            (1, "06-18 19:24:00.000  1 2 I TagA: a1", 0, 1),
            (2, "06-18 19:24:01.000  1 2 E TagA: a2", 0, 2),
            (3, "", 2, 1),
            (4, "06-18 19:24:02.000  3 4 E TagC: c1", 2, 2),
            (5, "06-18 19:24:03.000  5 6 W TagD: d1", 3, 1),
        ]
    );

    // Search, locate and the raw detail cover every file.
    let found = query(
        &e,
        id,
        "search",
        2,
        Filters {
            query: "tagd".into(),
            ..Filters::default()
        },
    );
    assert_eq!(found["count"], 1);
    assert_eq!(found["rows"][0]["file"], 3);
    let detail = e
        .request(
            json!({"datasetId":id,"queryId":2,"action":"range","pane":"search","lineNumber":5}),
        )
        .unwrap();
    assert_eq!(detail["text"], "06-18 19:24:03.000  5 6 W TagD: d1");
    assert_eq!(
        (detail["file"].as_u64(), detail["fileLine"].as_u64()),
        (Some(3), Some(1))
    );
    let located = e
        .request(json!({"datasetId":id,"queryId":1,"action":"locate","pane":"logs","lineNumber":4}))
        .unwrap();
    assert_eq!(located["index"], 3);
}

#[test]
fn export_keeps_original_bytes_and_separates_unterminated_file_ends() {
    let dir = tempfile::tempdir().unwrap();
    let (e, id) = open(&write(dir.path()));
    query(&e, id, "logs", 1, Filters::default());
    let out = dir.path().join("out.log");
    let never = AtomicBool::new(false);
    let result = e.export("logs", id, 1, &out, &never, |_, _| {}).unwrap();
    // `a` keeps its CR; `c`'s unterminated last row gets one LF before `d`.
    let mut expected = [FILES[0].1, FILES[2].1].concat();
    expected.push(b'\n');
    expected.extend(FILES[3].1);
    assert_eq!(std::fs::read(&out).unwrap(), expected);
    assert_eq!(result, json!({"rows":5,"bytes":expected.len()}));

    // None of the open files may be the export destination.
    let err = e
        .export("logs", id, 1, &dir.path().join("c.log"), &never, |_, _| {})
        .unwrap_err();
    assert_eq!(err.to_string(), "Choose a file other than the open log");
}

#[test]
fn a_single_file_has_no_file_markers_and_selection_is_validated() {
    let dir = tempfile::tempdir().unwrap();
    let paths = write(dir.path());
    let (e, id) = open(&paths[..1]);
    let page = query(&e, id, "logs", 1, Filters::default());
    assert!(page["rows"][0].get("file").is_none());
    assert_eq!(
        e.request(json!({"action":"status"})).unwrap()["name"],
        "a.log"
    );

    let e = Arc::new(Engine::default());
    for (paths, message) in [
        (vec![], "Select at least one log file"),
        (vec![paths[0].clone(); 65], "Select 1 to 64 log files"),
        (
            vec![paths[0].clone(), paths[2].clone(), paths[0].clone()],
            "The same file was selected more than once",
        ),
        (
            vec![paths[0].clone(), dir.path().join("missing.log")],
            "Selected file was not found; select it again",
        ),
    ] {
        let id = e.begin_open();
        assert_eq!(
            e.open_files(id, &paths).unwrap_err().to_string(),
            message,
            "{paths:?}"
        );
    }
}

#[test]
fn a_change_to_any_file_is_detected() {
    let dir = tempfile::tempdir().unwrap();
    let paths = write(dir.path());
    let (e, id) = open(&paths);
    query(&e, id, "logs", 1, Filters::default());
    std::fs::write(&paths[3], b"06-18 19:24:03.000  5 6 W TagD: changed d1\n").unwrap();
    let err = e
        .request(json!({"datasetId":id,"queryId":1,"action":"range","pane":"logs","lineNumber":5}))
        .unwrap_err();
    assert_eq!(err.to_string(), "Source changed; reopen the file");
}

#[test]
fn files_larger_than_a_block_keep_their_own_row_numbers() {
    let dir = tempfile::tempdir().unwrap();
    let row = |file: &str, n: usize| format!("06-18 19:24:00.000  1 2 I {file}: row {n}\n");
    let paths: Vec<PathBuf> = [("x.log", 9000), ("y.log", 7000)]
        .iter()
        .map(|(name, rows)| {
            let path = dir.path().join(name);
            let text: String = (1..=*rows).map(|n| row(name, n)).collect();
            std::fs::write(&path, text).unwrap();
            path
        })
        .collect();
    let (e, id) = open(&paths);
    assert_eq!(
        e.request(json!({"action":"status"})).unwrap()["total"],
        16000
    );
    let found = query(
        &e,
        id,
        "search",
        1,
        Filters {
            query: "y.log: row 7000".into(),
            ..Filters::default()
        },
    );
    assert_eq!(found["count"], 1);
    assert_eq!(found["rows"][0]["lineNumber"], 16000);
    for (line, file, file_line, text) in [
        (9000, 0, 9000, row("x.log", 9000)),
        (9001, 1, 1, row("y.log", 1)),
        (16000, 1, 7000, row("y.log", 7000)),
    ] {
        let page = e
            .request(json!({"datasetId":id,"queryId":1,"action":"range","pane":"search","lineNumber":line}))
            .unwrap();
        assert_eq!(page["text"], text.trim_end());
        assert_eq!(
            (page["file"].as_u64(), page["fileLine"].as_u64()),
            (Some(file), Some(file_line))
        );
    }
}

// Windows shares open files with their writers (a live log) and with
// renames and deletes (rotation); the test above covers change detection.
#[cfg(windows)]
#[test]
fn every_file_stays_writable_and_removable_while_open() {
    let dir = tempfile::tempdir().unwrap();
    let paths = write(dir.path());
    let (e, id) = open(&paths);
    query(&e, id, "logs", 1, Filters::default());
    for path in &paths {
        std::fs::OpenOptions::new().append(true).open(path).unwrap();
    }
    let moved = dir.path().join("moved.log");
    std::fs::rename(&paths[0], &moved).unwrap();
    std::fs::remove_file(&moved).unwrap();
    e.close();
}

// Sparse files make the sizes without writing data; opening reads nothing.
// Windows extends files without sparseness, so this stays on Unix.
#[cfg(unix)]
#[test]
fn only_each_file_is_bounded_not_the_combined_size() {
    use logstudio_core::source::{Source, MAX_FILE};
    let dir = tempfile::tempdir().unwrap();
    let sized = |name: &str, len: u64| {
        let path = dir.path().join(name);
        std::fs::File::create(&path).unwrap().set_len(len).unwrap();
        path
    };
    let parts = [sized("a.log", MAX_FILE), sized("b.log", MAX_FILE / 8)];
    let source = Source::granted_files(&parts).unwrap();
    assert_eq!(source.size, MAX_FILE + MAX_FILE / 8);
    assert_eq!(source.parts()[1].offset, MAX_FILE);
    let over = [sized("c.log", 1), sized("d.log", MAX_FILE + 1)];
    assert_eq!(
        Source::granted_files(&over).err().unwrap().to_string(),
        "File changed or exceeds 8 GiB"
    );
}
