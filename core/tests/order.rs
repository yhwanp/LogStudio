use logstudio_core::{
    order::{arrange, By, HEAD},
    search::Filters,
    Engine,
};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
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
fn write(dir: &Path, files: &[(&str, &str)]) -> Vec<PathBuf> {
    files
        .iter()
        .map(|(name, text)| {
            let path = dir.join(name);
            std::fs::write(&path, text).unwrap();
            path
        })
        .collect()
}
fn names(paths: &[PathBuf]) -> Vec<String> {
    paths
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into())
        .collect()
}
fn open(paths: &[PathBuf]) -> (Arc<Engine>, u64) {
    let e = Arc::new(Engine::default());
    let id = e.begin_open();
    e.open_files(id, paths).unwrap();
    wait(|| e.request(json!({"action":"status"})).unwrap()["textComplete"] == true);
    (e, id)
}
fn rows(e: &Engine, id: u64) -> Vec<String> {
    e.request(json!({"datasetId":id,"queryId":1,"action":"query","pane":"logs","filters":Filters::default()}))
        .unwrap();
    wait(|| e.request(json!({"action":"status"})).unwrap()["query"]["status"] == "complete");
    let page: Value = e
        .request(json!({"datasetId":id,"queryId":1,"action":"page","pane":"logs","start":0}))
        .unwrap();
    page["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["raw"].as_str().unwrap().into())
        .collect()
}

#[test]
fn files_open_in_time_order_and_status_lists_them_so() {
    let dir = tempfile::tempdir().unwrap();
    // Delivered newest first, as a picker might; the rotated names agree.
    let given = write(
        dir.path(),
        &[
            ("main.log", "06-18 10:00:00.000  1 2 I Now: c\n"),
            (
                "main.log.2",
                "--------- beginning of main\n06-16 10:00:00.000  1 2 I Old: a\n",
            ),
            ("main.log.1", "000001 06-17 10:00:00.000  1 2 I Mid: b\n"),
        ],
    );
    let (paths, by) = arrange(given.clone());
    assert_eq!(by, Some(By::Time));
    assert_eq!(names(&paths), ["main.log.2", "main.log.1", "main.log"]);
    let (e, id) = open(&paths);
    let status = e.request(json!({"action":"status"})).unwrap();
    assert_eq!(status["name"], "main.log.2 외 2개");
    let listed: Vec<&str> = status["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["name"].as_str().unwrap())
        .collect();
    assert_eq!(listed, ["main.log.2", "main.log.1", "main.log"]);
    assert_eq!(
        rows(&e, id),
        [
            "--------- beginning of main",
            "06-16 10:00:00.000  1 2 I Old: a",
            "000001 06-17 10:00:00.000  1 2 I Mid: b",
            "06-18 10:00:00.000  1 2 I Now: c",
        ]
    );

    // An order that already holds is reported as unchanged.
    assert_eq!(arrange(paths.clone()), (paths, None));
    assert_eq!(arrange(given[..1].to_vec()), (given[..1].to_vec(), None));
}

#[test]
fn a_reboot_between_files_keeps_their_order() {
    let dir = tempfile::tempdir().unwrap();
    // Rows from before the clock was set fill more than a head.
    let boot = |early: &str, set: &str| {
        let row = format!("{early}  0  0 I init: boot step\n");
        row.repeat(HEAD / row.len() + 1) + &format!("{set}  1 2 I Clock: set\n")
    };
    let given = write(
        dir.path(),
        &[
            (
                "logcat_2026-03-11 08_30_00 2.txt",
                "03-11 08:30:00.000  1 2 I Run: d\n",
            ),
            (
                "logcat_2026-03-11 07_30_00 1.txt",
                &boot("01-01 09:00:02.000", "03-11 07:30:20.000"),
            ),
            (
                "logcat_2026-03-10 09_00_00 2.txt",
                "03-10 09:00:00.000  1 2 I Run: b\n",
            ),
            (
                "logcat_2026-03-10 08_00_00 1.txt",
                &boot("01-01 09:00:05.000", "03-10 08:00:20.000"),
            ),
        ],
    );
    let (paths, by) = arrange(given);
    assert_eq!(by, Some(By::Time));
    assert_eq!(
        names(&paths),
        [
            "logcat_2026-03-10 08_00_00 1.txt",
            "logcat_2026-03-10 09_00_00 2.txt",
            "logcat_2026-03-11 07_30_00 1.txt",
            "logcat_2026-03-11 08_30_00 2.txt",
        ]
    );
}

#[test]
fn only_ends_are_read_and_unreadable_files_do_not_fail_ordering() {
    let dir = tempfile::tempdir().unwrap();
    // The later file's only row lies between its first and last `HEAD`
    // bytes, so names decide.
    let filler = "x\n".repeat(HEAD / 2);
    let late = format!("{filler}06-16 10:00:00.000  1 2 I Late: row\n{filler}");
    let paths = write(
        dir.path(),
        &[
            ("b.log", "06-17 10:00:00.000  1 2 I B: row\n"),
            ("a.log", &late),
        ],
    );
    let (ordered, by) = arrange(paths.clone());
    assert_eq!(
        (names(&ordered), by),
        (vec!["a.log".into(), "b.log".into()], Some(By::Name))
    );

    // A missing file counts as one without rows; opening reports it.
    let mut given = paths.clone();
    given.insert(0, dir.path().join("missing.log"));
    let (ordered, by) = arrange(given);
    assert_eq!(names(&ordered), ["a.log", "b.log", "missing.log"]);
    assert_eq!(by, Some(By::Name));
    let e = Arc::new(Engine::default());
    let id = e.begin_open();
    assert_eq!(
        e.open_files(id, &ordered).unwrap_err().to_string(),
        "Selected file was not found; select it again"
    );
}

// A FIFO would block a plain read; its head is never read.
#[cfg(unix)]
#[test]
fn a_fifo_is_not_read() {
    let dir = tempfile::tempdir().unwrap();
    let fifo = dir.path().join("pipe.log");
    assert!(std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .unwrap()
        .success());
    let paths = write(
        dir.path(),
        &[("z.log", "06-17 10:00:00.000  1 2 I Z: row\n")],
    );
    // Without rows to place it, one timed file leaves the order to names.
    let (ordered, by) = arrange(vec![paths[0].clone(), fifo.clone()]);
    assert_eq!(
        (ordered, by),
        (vec![fifo, paths[0].clone()], Some(By::Name))
    );
}
