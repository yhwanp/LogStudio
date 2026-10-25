//! Reading ahead only warms the OS file cache for a pass, on handles of its
//! own, so it never changes a result. These tests check when it runs, that
//! it ends with its pass, and what it reads.
use logstudio_core::{
    dataset::{Dataset, Query},
    search::{Filter, Filters},
    source::Source,
};
use serde_json::{json, Value};
use std::{
    path::Path,
    sync::{atomic::Ordering, Arc},
    thread,
    time::{Duration, Instant},
};

fn wait(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !done() {
        assert!(Instant::now() < deadline, "timed out: {what}");
        thread::sleep(Duration::from_millis(1));
    }
}
/// Android rows of a few levels, tags and PIDs, about 90 bytes each.
fn log(rows: usize) -> Vec<u8> {
    let (levels, tags) = (["V", "D", "I", "W", "E"], ["WifiService", "SDK_UI", "Gatt"]);
    (0..rows)
        .flat_map(|i| {
            format!(
                "09-21 12:07:{:02}.{:03} {} {} {} {}: message {i} value={}\n",
                i / 1000 % 60,
                i % 1000,
                101 + i % 4,
                2000 + i % 30,
                levels[i % 5],
                tags[i % 3],
                i * 7
            )
            .into_bytes()
        })
        .collect()
}
fn open(path: &Path) -> Arc<Dataset> {
    let d = Dataset::new(1, Source::granted(path).unwrap());
    d.start(|_| {});
    wait("analysis", || d.status(0)["summaryComplete"] == true);
    d
}
fn query(d: &Arc<Dataset>, pane: usize, id: u64, filters: Value) -> Arc<Query> {
    let filters: Filters = serde_json::from_value(filters).unwrap();
    d.query(pane, id, Filter::new(filters).unwrap()).unwrap()
}
fn complete(q: &Query) {
    wait("query", || q.json()["status"] != "running");
    assert_eq!(q.json()["status"], "complete");
}

#[test]
fn only_passes_that_read_blocks_read_ahead() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.log");
    std::fs::write(&path, log(40_000)).unwrap();
    let d = open(&path);
    assert!(d.index.lock().unwrap().blocks.len() > 8);
    // The analysis pass read every block.
    let analyzed = d.read_aheads.load(Ordering::Relaxed);
    assert!(analyzed >= 1);
    // The tables decide these without reading a block.
    for (id, filters) in [
        json!({"levels":["E"]}),
        json!({"tag":"wifi"}),
        json!({"pid":"102"}),
        json!({"include":[{"tag":"gatt"},{"levels":["W"]}]}),
    ]
    .into_iter()
    .enumerate()
    {
        complete(&query(&d, 0, 1 + id as u64, filters));
    }
    assert_eq!(d.read_aheads.load(Ordering::Relaxed), analyzed);
    // A search reads every block, and reads ahead once.
    complete(&query(&d, 1, 1, json!({"query":"value=7"})));
    assert_eq!(d.read_aheads.load(Ordering::Relaxed), analyzed + 1);
    d.close();
}

#[test]
fn reading_ahead_ends_with_a_cancelled_search() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.log");
    std::fs::write(&path, log(400_000)).unwrap();
    let d = open(&path);
    let before = d.read_aheads.load(Ordering::Relaxed);
    let q = query(
        &d,
        1,
        1,
        json!({"query":"(\\w+ ){3}\\d+9 ","searchMode":"regex"}),
    );
    wait("reading ahead", || {
        d.read_aheads.load(Ordering::Relaxed) > before || q.json()["status"] != "running"
    });
    assert_eq!(q.json()["status"], "running", "the search ended first");
    // The read-ahead thread belongs to the scan's pass: the cancel joins it.
    let t = Instant::now();
    q.cancel();
    assert!(t.elapsed() < Duration::from_secs(1), "{:?}", t.elapsed());
    assert_eq!(q.json()["status"], "cancelled");
    d.close();
}

#[test]
fn reads_each_file_on_a_handle_of_its_own() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (log(3_000), log(2_000));
    let paths = [dir.path().join("a.log"), dir.path().join("b.log")];
    std::fs::write(&paths[0], &a).unwrap();
    std::fs::write(&paths[1], &b).unwrap();
    let source = Source::granted_files(&paths).unwrap();
    let second = source.parts()[1].offset;
    assert_eq!(second, a.len() as u64);
    let mut reader = source.read_ahead().unwrap();
    assert_eq!(reader.read(0, 0, a.len()), a.len() as u64);
    assert_eq!(reader.read(1, second, b.len()), b.len() as u64);
    // The end of a file ends a read.
    assert_eq!(reader.read(1, second + 100, b.len()), b.len() as u64 - 100);
    // A path that now names another file is not read; the others are.
    // Windows tells files apart by creation time, which NTFS gives a file
    // renamed onto a name it just replaced (tunneling), so there the new
    // file may be read: harmless, as only the cache keeps the bytes.
    let other = dir.path().join("other.log");
    std::fs::write(&other, log(2_000)).unwrap();
    std::fs::rename(&other, &paths[1]).unwrap();
    let mut reader = source.read_ahead().unwrap();
    #[cfg(unix)]
    assert_eq!(reader.read(1, second, 10), 0);
    assert_eq!(reader.read(0, 0, 10), 10);
    // Pasted text has no path and stays cached.
    let pasted = String::from_utf8(log(10)).unwrap();
    assert!(Source::text(&pasted, "pasted")
        .unwrap()
        .read_ahead()
        .is_none());
}
