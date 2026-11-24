//! Exports and saves write a temporary file of their own beside the
//! destination: no other file there is read, written or deleted.
use logstudio_core::{
    capture::{fake::FakeLink, link::Platform, store::Store, Capture, Options},
    search::Filters,
    Engine,
};
use serde_json::json;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, mpsc, Arc},
    thread,
    time::{Duration, Instant},
};

fn wait(mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !done() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
}
fn log(tag: &str, rows: usize) -> Vec<u8> {
    (0..rows)
        .flat_map(|i| format!("06-18 19:24:00.000  1 2 I {tag}: row {i}\n").into_bytes())
        .collect()
}
/// A log open with the completed query 1.
fn open(path: &Path) -> (Arc<Engine>, u64) {
    let e = Arc::new(Engine::default());
    let id = e.begin_open();
    e.open_file(id, path).unwrap();
    wait(|| e.request(json!({"action":"status"})).unwrap()["textComplete"] == true);
    e.request(json!({"datasetId":id,"queryId":1,"action":"query","pane":"logs","filters":Filters::default()}))
        .unwrap();
    wait(|| {
        e.request(json!({"action":"status","pane":"logs"})).unwrap()["query"]["status"]
            == "complete"
    });
    (e, id)
}
/// The name every temporary file had before each got its own.
fn fixed(path: &Path) -> PathBuf {
    let name = path.file_name().unwrap().to_string_lossy();
    path.with_file_name(format!(".{name}.logstudio-partial"))
}
fn files(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .filter(|f| f.path().is_file())
        .map(|f| {
            let name = f.file_name().to_string_lossy().into_owned();
            (name, std::fs::read(f.path()).unwrap())
        })
        .collect()
}
fn flag(on: bool) -> AtomicBool {
    AtomicBool::new(on)
}

#[test]
fn an_export_touches_no_file_but_its_own_and_its_destination() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.log");
    std::fs::write(&source, log("A", 5000)).unwrap();
    let (e, id) = open(&source);
    let out = dir.path().join("out.log");
    std::fs::write(fixed(&out), b"not ours").unwrap();
    let before = files(dir.path());
    let cancelled = e.export("logs", id, 1, &out, &flag(true), |_, _| {});
    assert_eq!(cancelled.unwrap_err().to_string(), "Export cancelled");
    assert_eq!(files(dir.path()), before);
    e.export("logs", id, 1, &out, &flag(false), |_, _| {})
        .unwrap();
    let mut after = before;
    after.insert("out.log".into(), log("A", 5000));
    assert_eq!(files(dir.path()), after);

    // A destination that cannot be replaced leaves nothing behind either.
    let folder = dir.path().join("folder");
    std::fs::create_dir(&folder).unwrap();
    assert!(e
        .export("logs", id, 1, &folder, &flag(false), |_, _| {})
        .is_err());
    assert_eq!(files(dir.path()), after);
    e.close();
}

#[test]
fn an_export_cancelled_after_its_last_block_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.log");
    // One block, so that nothing is read after the cancel; its rows are
    // still in the writer's buffer then.
    std::fs::write(&source, log("A", 100)).unwrap();
    let (e, id) = open(&source);
    let out = dir.path().join("out.log");
    std::fs::write(&out, b"before").unwrap();
    let before = files(dir.path());
    let cancel = flag(false);
    let result = e.export("logs", id, 1, &out, &cancel, |rows, _| {
        assert_eq!(rows, 100);
        cancel.store(true, std::sync::atomic::Ordering::Release);
    });
    assert_eq!(result.unwrap_err().to_string(), "Export cancelled");
    assert_eq!(files(dir.path()), before);
    e.close();
}

#[test]
fn a_destination_may_have_the_longest_name_a_file_can() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.log");
    std::fs::write(&source, log("A", 10)).unwrap();
    let (e, id) = open(&source);
    let out = dir.path().join("x".repeat(255));
    // Some systems allow only a shorter path.
    if std::fs::write(&out, b"").is_err() {
        return;
    }
    e.export("logs", id, 1, &out, &flag(false), |_, _| {})
        .unwrap();
    assert_eq!(std::fs::read(&out).unwrap(), log("A", 10));
    e.close();
}

#[test]
fn two_exports_to_one_destination_never_mix() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (dir.path().join("a.log"), dir.path().join("b.log"));
    std::fs::write(&a, log("AAAA", 30_000)).unwrap();
    std::fs::write(&b, log("BBBB", 10_000)).unwrap();
    let ((first, i), (second, j)) = (open(&a), open(&b));
    let out = dir.path().join("out.log");
    let (reached, at) = mpsc::channel();
    let (go, gone) = mpsc::channel::<()>();
    let to = out.clone();
    // The first export is in the middle while the second is made.
    let slow = thread::spawn(move || {
        let mut blocks = 0;
        first.export("logs", i, 1, &to, &flag(false), |_, _| {
            blocks += 1;
            if blocks == 3 {
                reached.send(()).unwrap();
                gone.recv().unwrap();
            }
        })
    });
    at.recv().unwrap();
    second
        .export("logs", j, 1, &out, &flag(false), |_, _| {})
        .unwrap();
    assert_eq!(std::fs::read(&out).unwrap(), log("BBBB", 10_000));
    go.send(()).unwrap();
    slow.join().unwrap().unwrap();
    assert_eq!(std::fs::read(&out).unwrap(), log("AAAA", 30_000));
    assert_eq!(files(dir.path()).len(), 3);
}

#[test]
fn saving_a_capture_touches_no_file_but_its_own_and_its_destination() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().join("captures"));
    let link = FakeLink::new();
    link.add("SER1", "Pixel_8", "SER1");
    let slot = store.create(Platform::Android, "Pixel 8").unwrap();
    let options = Options {
        poll: Duration::from_millis(2),
        recent: 256,
    };
    let c = Capture::start(link.clone(), "SER1", slot, options).unwrap();
    let rows = String::from_utf8(log("A", 100)).unwrap();
    link.log("SER1", &rows);
    wait(|| c.bytes() == rows.len() as u64);
    let out = dir.path().join("saved.log");
    std::fs::write(fixed(&out), b"not ours").unwrap();
    let before = files(dir.path());
    assert!(c.save(&out, &flag(true)).is_err());
    assert_eq!(files(dir.path()), before);
    c.save(&out, &flag(false)).unwrap();
    let mut after = before;
    after.insert("saved.log".into(), rows.into_bytes());
    assert_eq!(files(dir.path()), after);
    c.close();
}
