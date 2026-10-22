use logstudio_core::{
    dataset::{Dataset, Query},
    search::{Filter, Filters},
    source::Source,
    Engine,
};
use serde_json::{json, Value};
use std::{
    io::Write,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

fn wait(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !done() {
        assert!(Instant::now() < deadline, "timed out: {what}");
        thread::sleep(Duration::from_millis(1));
    }
}
fn append(path: &Path, bytes: &[u8]) {
    let mut f = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    f.write_all(bytes).unwrap();
}
// Datasets poll fast so the tests need not wait for the product interval.
fn open(path: &Path) -> Arc<Dataset> {
    let d = Dataset::new(1, Source::granted(path).unwrap());
    d.follow_poll_ms.store(2, Ordering::Relaxed);
    d.start(|_| {});
    wait("index", || d.index.lock().unwrap().done);
    d
}
// Filters from their wire form, as the page sends them.
fn wire(v: Value) -> Filters {
    serde_json::from_value(v).unwrap()
}
fn filters(query: &str) -> Filters {
    wire(json!({ "query": query }))
}
fn query(d: &Arc<Dataset>, pane: usize, id: u64, f: Filters) -> Arc<Query> {
    d.query(pane, id, Filter::new(f).unwrap()).unwrap()
}
fn total(d: &Dataset) -> u64 {
    d.index.lock().unwrap().total
}
/// Waits until the query has scanned every indexed block and waits for more.
fn caught_up(d: &Dataset, q: &Query, rows: u64) {
    wait("caught up", || {
        total(d) == rows && q.json()["caughtUp"] == true && {
            let blocks = d.index.lock().unwrap().blocks.len();
            q.result.lock().unwrap().ends.len() == blocks
        }
    });
}
fn complete(d: &Dataset, q: &Query) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while q.json()["status"] != "complete" || d.status(0)["summaryComplete"] != true {
        let index = d.index.lock().unwrap();
        let state = (index.blocks.len(), index.analyzed, index.live);
        drop(index);
        assert!(
            Instant::now() < deadline,
            "not complete: {} {state:?} {}",
            q.json(),
            d.status(0)
        );
        thread::sleep(Duration::from_millis(1));
    }
}
fn raws(d: &Dataset, q: &Query) -> Vec<String> {
    pages(d, q)
        .iter()
        .map(|r| r["raw"].as_str().unwrap().to_string())
        .collect()
}
fn pages(d: &Dataset, q: &Query) -> Vec<Value> {
    let mut rows = vec![];
    loop {
        let page = d.page(q, rows.len() as u64, 200).unwrap();
        let got = page["rows"].as_array().unwrap().clone();
        if got.is_empty() {
            return rows;
        }
        rows.extend(got);
    }
}
fn line(n: usize, level: &str, tag: &str, message: &str) -> String {
    format!(
        "06-18 19:{:02}:{:02}.{:03}  1{} 2{} {level} {tag}: {message}",
        n / 3600 % 60,
        n / 60 % 60,
        n % 1000,
        n % 7,
        n % 5
    )
}

#[test]
fn appended_complete_rows_extend_rows_queries_and_levels() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("live.log");
    std::fs::write(&path, b"").unwrap();
    let d = open(&path);
    let all = query(&d, 0, 1, Filters::default());
    let errors = query(&d, 1, 1, wire(json!({"levels":["E"]})));
    complete(&d, &all);
    d.follow(true).unwrap();
    assert_eq!(d.status(0)["following"], true);
    assert_eq!(all.json()["status"], "running");
    append(&path, format!("{}\n", line(1, "I", "Net", "up")).as_bytes());
    caught_up(&d, &all, 1);
    append(
        &path,
        format!(
            "{}\n{}\n",
            line(2, "E", "Net", "down"),
            line(3, "W", "Ui", "slow")
        )
        .as_bytes(),
    );
    caught_up(&d, &all, 3);
    caught_up(&d, &errors, 3);
    assert_eq!(all.json()["count"], 3);
    assert_eq!(errors.json()["count"], 1);
    assert!(raws(&d, &errors)[0].ends_with("Net: down"));
    // While followed, results and level counts are partial, not final.
    let status = d.status(0);
    assert_eq!(status["summaryComplete"], false);
    assert_eq!(status["textComplete"], true);
    assert_eq!(status["size"], std::fs::metadata(&path).unwrap().len());
    wait("levels", || d.status(0)["summary"]["counts"]["W"] == 1);
    d.follow(false).unwrap();
    complete(&d, &all);
    complete(&d, &errors);
    let status = d.status(0);
    assert_eq!(status["following"], false);
    assert_eq!(status["summary"]["counts"]["E"], 1);
    assert_eq!(status["summary"]["total"], 3);
    d.close();
}

#[test]
fn an_unterminated_last_row_is_replaced_when_appended_bytes_complete_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("live.log");
    let head = format!(
        "{}\n{}",
        line(1, "I", "App", "start"),
        line(2, "I", "App", "b")
    );
    std::fs::write(&path, &head).unwrap();
    let d = open(&path);
    // A row the file's end closes gets its own block.
    assert_eq!(d.index.lock().unwrap().blocks.last().unwrap().rows, 1);
    let all = query(&d, 0, 1, Filters::default());
    let search = query(&d, 1, 1, filters("bcd"));
    complete(&d, &all);
    complete(&d, &search);
    assert_eq!(search.json()["count"], 0);
    d.follow(true).unwrap();
    append(&path, b"c");
    // An incomplete row keeps showing what it was.
    thread::sleep(Duration::from_millis(50));
    assert_eq!(total(&d), 2);
    assert!(raws(&d, &all)[1].ends_with("App: b"));
    assert_eq!(d.status(0)["tailRevision"], 0);
    append(
        &path,
        format!("d\n{}\n", line(3, "E", "App", "e")).as_bytes(),
    );
    caught_up(&d, &all, 3);
    caught_up(&d, &search, 3);
    let rows = raws(&d, &all);
    assert_eq!(rows.len(), 3);
    assert!(rows[1].ends_with("App: bcd"), "{rows:?}");
    assert_eq!(search.json()["count"], 1);
    assert_eq!(d.status(0)["tailRevision"], 1);
    let located = d.locate(&search, 2).unwrap();
    assert_eq!(located["found"], true);
    d.follow(false).unwrap();
    complete(&d, &all);
    assert_same_as_fresh(&d, &path);
    d.close();
}

#[test]
fn crlf_split_across_appends_keeps_one_row_per_line_ending() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("crlf.log");
    std::fs::write(&path, b"A\r").unwrap();
    let d = open(&path);
    let all = query(&d, 0, 1, Filters::default());
    complete(&d, &all);
    d.follow(true).unwrap();
    append(&path, b"\nB\r");
    // The CR may still get its LF, so B waits.
    thread::sleep(Duration::from_millis(50));
    assert_eq!(total(&d), 1);
    append(&path, b"\nC\rD\r\n\nE");
    caught_up(&d, &all, 5);
    assert_eq!(raws(&d, &all), ["A", "B", "C", "D", ""]);
    // The LF joined row A's line ending; the row itself never changed.
    assert_eq!(d.status(0)["tailRevision"], 0);
    d.follow(false).unwrap();
    complete(&d, &all);
    assert_eq!(raws(&d, &all), ["A", "B", "C", "D", "", "E"]);
    let mut out = vec![];
    d.export(&all, &mut out, &AtomicBool::new(false), |_, _| {})
        .unwrap();
    assert_eq!(out, std::fs::read(&path).unwrap());
    assert_same_as_fresh(&d, &path);
    d.close();
}

#[test]
fn truncation_and_replacement_stop_following_with_the_changed_file_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("live.log");
    std::fs::write(&path, b"one\ntwo\n").unwrap();
    let d = open(&path);
    let all = query(&d, 0, 1, Filters::default());
    d.follow(true).unwrap();
    caught_up(&d, &all, 2);
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(3)
        .unwrap();
    wait("truncated", || d.status(0)["status"] == "error");
    let status = d.status(0);
    assert_eq!(status["error"], "Source truncated");
    assert_eq!(status["following"], false);
    wait("query error", || all.json()["status"] == "error");
    d.close();

    // Rewriting in place to a larger length changes the first bytes.
    std::fs::write(&path, b"one\ntwo\n").unwrap();
    let d = open(&path);
    d.follow(true).unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .write_all(b"ONE\ntwo\nthree\n")
        .unwrap();
    wait("rewritten", || d.status(0)["status"] == "error");
    assert_eq!(d.status(0)["error"], "Source changed; reopen the file");
    d.close();

    // Replacing the file at its path (rotation) is another file.
    #[cfg(unix)]
    {
        std::fs::write(&path, b"one\ntwo\n").unwrap();
        let d = open(&path);
        d.follow(true).unwrap();
        let next = dir.path().join("next.log");
        std::fs::write(&next, b"one\ntwo\nthree\n").unwrap();
        std::fs::rename(&next, &path).unwrap();
        wait("replaced", || d.status(0)["status"] == "error");
        assert_eq!(d.status(0)["error"], "Source changed; reopen the file");
        d.close();
    }
}

#[test]
fn a_search_running_while_rows_arrive_ends_with_exact_counts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("big.log");
    let mut text = String::new();
    for n in 0..60_000 {
        let tag = if n % 997 == 0 { "Needle" } else { "Hay" };
        text += &line(n, ["I", "D", "E"][n % 3], tag, &format!("row {n} 한글 🙂"));
        text.push('\n');
    }
    std::fs::write(&path, &text).unwrap();
    let d = open(&path);
    d.follow(true).unwrap();
    let search = query(&d, 1, 1, filters("needle"));
    let tagged = query(&d, 0, 1, wire(json!({"tag":"needle"})));
    // Appended while both scans run.
    let mut more = String::new();
    for n in 60_000..61_000 {
        let tag = if n % 10 == 0 { "Needle" } else { "Hay" };
        more += &line(n, "W", tag, "late");
        more.push('\n');
    }
    append(&path, more.as_bytes());
    caught_up(&d, &search, 61_000);
    caught_up(&d, &tagged, 61_000);
    let expected = (0..60_000).filter(|n| n % 997 == 0).count() + 100;
    assert_eq!(search.json()["count"], expected as u64);
    assert_eq!(tagged.json()["count"], expected as u64);
    d.follow(false).unwrap();
    complete(&d, &search);
    assert_eq!(search.json()["count"], expected as u64);
    d.close();
}

// Appends of random sizes split rows, line endings and UTF-8 characters,
// across blocks, and following goes off and on. The result must equal
// opening the final file: rows, owners, search and filter counts, level
// counts and export.
#[test]
fn random_appends_match_a_fresh_open_of_the_final_file() {
    let mut text = String::new();
    let endings = ["\n", "\r\n", "\r"];
    for n in 0..12_000 {
        let ending = endings[n % 7 % 3];
        let row = match n % 11 {
            0 => "  continuation of the entry 한글".to_string(),
            1 => format!(
                "2026-09-30 21:55:53.{n:06} Runner[{}:{}] [ERROR] [SDK] [Car] 🙂 {n}",
                n % 9,
                n % 4
            ),
            2 => String::new(),
            3 => format!("raw text {n}"),
            _ => line(
                n,
                ["V", "D", "I", "W", "E", "F"][n % 6],
                ["Net", "Ui", "Db"][n % 3],
                &format!("message {n} ünïcode"),
            ),
        };
        text += &row;
        text += ending;
    }
    // The file ends without a line ending.
    text += "tail without ending";
    for seed in [0x2545_f491_4f6c_dd1d, 0x9e37_79b9_7f4a_7c15, 7] {
        random_case(text.as_bytes(), seed);
    }
}
fn random_case(bytes: &[u8], mut seed: u64) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("random.log");
    let mut next = |n: u64| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed % n
    };
    let first = 1000 + next(200_000) as usize;
    std::fs::write(&path, &bytes[..first]).unwrap();
    let d = open(&path);
    let all = query(&d, 0, 1, Filters::default());
    let search = query(&d, 1, 1, filters("4"));
    d.follow(true).unwrap();
    let (mut at, mut on) = (first, true);
    while at < bytes.len() {
        let most = if next(10) == 0 { 40_000 } else { 400 };
        let n = (1 + next(most) as usize).min(bytes.len() - at);
        append(&path, &bytes[at..at + n]);
        at += n;
        match next(20) {
            0 => thread::sleep(Duration::from_millis(5)),
            1 => {
                on = !on;
                d.follow(on).unwrap();
            }
            _ => {}
        }
    }
    d.follow(true).unwrap();
    thread::sleep(Duration::from_millis(30));
    d.follow(false).unwrap();
    complete(&d, &all);
    complete(&d, &search);
    assert!(d.status(0)["tailRevision"].as_u64().unwrap() > 0);
    assert!(d.index.lock().unwrap().blocks.len() > 4);
    assert_same_as_fresh(&d, &path);
    d.close();
}

/// Compares every row, several filters and the level summary with a fresh
/// open of the file as it is now.
fn assert_same_as_fresh(d: &Arc<Dataset>, path: &Path) {
    let fresh = open(path);
    assert_eq!(total(d), total(&fresh));
    let cases = [
        Filters::default(),
        filters("4"),
        wire(json!({"query":"e\\d+$","searchMode":"regex"})),
        wire(json!({"levels":["E","W"]})),
        wire(json!({"tag":"net"})),
        wire(json!({"includeRaw":false,"excludeTags":"ui"})),
    ];
    for (k, f) in cases.into_iter().enumerate() {
        let id = 100 + k as u64;
        let (a, b) = (query(d, 1, id, f.clone()), query(&fresh, 1, id, f));
        complete(d, &a);
        complete(&fresh, &b);
        assert_eq!(a.json()["count"], b.json()["count"], "filter {k}");
        assert_eq!(pages(d, &a), pages(&fresh, &b), "filter {k}");
        let (mut x, mut y) = (vec![], vec![]);
        d.export(&a, &mut x, &AtomicBool::new(false), |_, _| {})
            .unwrap();
        fresh
            .export(&b, &mut y, &AtomicBool::new(false), |_, _| {})
            .unwrap();
        assert_eq!(x, y, "filter {k}");
        assert_eq!(d.status(1)["summary"], fresh.status(1)["summary"]);
    }
    fresh.close();
}

#[test]
fn invalid_appended_rows_stop_following_at_the_last_valid_row() {
    let dir = tempfile::tempdir().unwrap();
    for (bad, message) in [
        (
            b"bad\0row\n".to_vec(),
            "NUL is not supported; select a UTF-8 text log",
        ),
        (vec![b'x'; 1024 * 1024 + 8], "A physical line exceeds 1 MiB"),
    ] {
        let path = dir.path().join("live.log");
        std::fs::write(&path, b"one\n").unwrap();
        let d = open(&path);
        let all = query(&d, 0, 1, Filters::default());
        d.follow(true).unwrap();
        append(&path, b"two\n");
        caught_up(&d, &all, 2);
        append(&path, &bad);
        wait("stopped", || d.status(0)["following"] == false);
        let status = d.status(0);
        assert_eq!(status["followError"], message);
        assert_eq!(status["status"], "ready");
        complete(&d, &all);
        assert_eq!(raws(&d, &all), ["one", "two"]);
        d.close();
    }
}

#[test]
fn cancel_and_close_while_following() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("live.log");
    std::fs::write(&path, b"a one\n").unwrap();
    let d = open(&path);
    let all = query(&d, 0, 1, Filters::default());
    let search = query(&d, 1, 1, filters("one"));
    d.follow(true).unwrap();
    caught_up(&d, &search, 1);
    search.cancel();
    assert_eq!(search.json()["status"], "cancelled");
    append(&path, b"b one\nc one\n");
    caught_up(&d, &all, 3);
    assert_eq!(search.json()["status"], "cancelled");
    assert_eq!(search.json()["count"], 1);
    // A replacing query sees every row and keeps following.
    let replaced = query(&d, 1, 2, filters("one"));
    caught_up(&d, &replaced, 3);
    assert_eq!(replaced.json()["count"], 3);
    let started = Instant::now();
    d.close();
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(d.status(0)["status"], "closed");
    assert!(d.source.read(0, 1).is_err());
    append(&path, b"d one\n");
    thread::sleep(Duration::from_millis(20));
    assert_eq!(total(&d), 3);
}

#[test]
fn without_following_rows_stay_as_opened_and_other_changes_need_reopening() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("live.log");
    std::fs::write(&path, b"one\ntwo\n").unwrap();
    let d = open(&path);
    let all = query(&d, 0, 1, Filters::default());
    complete(&d, &all);
    assert_eq!(d.status(0)["grown"], false);
    // A log being written keeps working as it was opened.
    append(&path, b"three\n");
    thread::sleep(Duration::from_millis(20));
    assert_eq!(raws(&d, &all), ["one", "two"]);
    let status = d.status(0);
    assert_eq!(
        (status["total"].as_u64(), status["size"].as_u64()),
        (Some(2), Some(8))
    );
    assert_eq!(status["grown"], true);
    assert_eq!(all.json()["status"], "complete");
    // Following accepts the appended row.
    d.follow(true).unwrap();
    caught_up(&d, &all, 3);
    assert_eq!(raws(&d, &all), ["one", "two", "three"]);
    assert_eq!(d.status(0)["grown"], false);
    d.close();

    // Any other change still needs reopening, as before.
    for (change, message) in [
        (&b"ONE\ntwo\nthree\n"[..], "Source changed; reopen the file"),
        (&b"one\n"[..], "Source truncated"),
    ] {
        std::fs::write(&path, b"one\ntwo\n").unwrap();
        let d = open(&path);
        let all = query(&d, 0, 1, Filters::default());
        complete(&d, &all);
        let mut f = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        f.write_all(change).unwrap();
        f.set_len(change.len() as u64).unwrap();
        let e = d.page(&all, 0, 10).unwrap_err();
        assert_eq!(e.to_string(), message);
        assert!(d.follow(true).is_err());
        d.close();
    }
}

#[test]
fn after_following_appended_bytes_are_ignored_until_followed_again() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("live.log");
    std::fs::write(&path, b"one\n").unwrap();
    let d = open(&path);
    let all = query(&d, 0, 1, Filters::default());
    d.follow(true).unwrap();
    append(&path, b"two\nthr");
    caught_up(&d, &all, 2);
    d.follow(false).unwrap();
    complete(&d, &all);
    // Turning it off ends the rows like opening the file then would.
    assert_eq!(raws(&d, &all), ["one", "two", "thr"]);
    append(&path, b"ee\nfour\n");
    thread::sleep(Duration::from_millis(20));
    assert_eq!(raws(&d, &all), ["one", "two", "thr"]);
    assert_eq!(all.json()["status"], "complete");
    d.follow(true).unwrap();
    caught_up(&d, &all, 4);
    assert_eq!(raws(&d, &all), ["one", "two", "three", "four"]);
    d.close();
}

#[test]
fn only_a_single_file_source_can_be_followed() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (dir.path().join("a.log"), dir.path().join("b.log"));
    std::fs::write(&a, b"one\n").unwrap();
    std::fs::write(&b, b"two\n").unwrap();
    let sources = [
        Source::text("one\n", "pasted").unwrap(),
        Source::granted_files(&[a.clone(), b]).unwrap(),
    ];
    for source in sources {
        assert!(!source.followable());
        let d = Dataset::new(1, source);
        d.start(|_| {});
        wait("index", || d.index.lock().unwrap().done);
        assert_eq!(d.status(0)["followable"], false);
        let e = d.follow(true).unwrap_err();
        assert_eq!(e.to_string(), "Only one opened log file can be followed");
        d.follow(false).unwrap();
        d.close();
    }
    let d = open(&a);
    assert_eq!(d.status(0)["followable"], true);
    d.close();
}

#[test]
fn the_follow_request_follows_the_open_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("live.log");
    std::fs::write(&path, b"one\n").unwrap();
    let e = Arc::new(Engine::default());
    let id = e.begin_open();
    e.open_file(id, &path).unwrap();
    wait("index", || {
        e.request(json!({"action":"status"})).unwrap()["textComplete"] == true
    });
    let request = |v: Value| e.request(v);
    let status = request(json!({"datasetId":id,"action":"follow","follow":true})).unwrap();
    assert_eq!(status["following"], true);
    assert!(request(json!({"datasetId":id + 1,"action":"follow","follow":true})).is_err());
    assert!(request(json!({"datasetId":id,"action":"follow"})).is_err());
    request(json!({"datasetId":id,"queryId":1,"action":"query","filters":Filters::default()}))
        .unwrap();
    append(&path, b"two\n");
    // The product polls every 500 ms.
    wait("grown", || {
        let s = e.request(json!({"action":"status"})).unwrap();
        s["total"] == 2 && s["query"]["count"] == 2 && s["query"]["caughtUp"] == true
    });
    let status = request(json!({"datasetId":id,"action":"follow","follow":false})).unwrap();
    assert_eq!(status["following"], false);
    wait("complete", || {
        e.request(json!({"action":"status"})).unwrap()["query"]["status"] == "complete"
    });
    e.close();
}

#[test]
fn a_replaced_last_block_leaves_no_stale_field_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fields.log");
    let mut text = String::new();
    for i in 0..6000 {
        let (pid, tag) = if i % 7 == 0 {
            (4242, "net")
        } else {
            (100 + i % 3, "ui")
        };
        text.push_str(&format!(
            "06-18 10:{:02}:{:02}.000  {pid}  {pid} {} {tag}: row {i}\n",
            i / 60 % 60,
            i % 60,
            ["I", "W", "E"][i % 3]
        ));
    }
    // The opened file ends without a line ending; the append completes it.
    text.push_str("06-18 11:00:00.000  4242  4242 E net: partial");
    std::fs::write(&path, &text).unwrap();
    let d = open(&path);
    let until = Instant::now() + Duration::from_secs(20);
    while d.index.lock().unwrap().block_fields.is_none() {
        assert!(Instant::now() < until, "analysis did not finish");
        thread::sleep(Duration::from_millis(5));
    }
    let blocks = d.index.lock().unwrap().blocks.len();
    assert!(blocks > 2);
    let net = query(&d, 0, 1, wire(json!({"tag":"net"})));
    complete(&d, &net);
    d.follow(true).unwrap();
    append(
        &path,
        b" done\n06-18 11:00:01.000   101   101 W ui: after\n",
    );
    let until = Instant::now() + Duration::from_secs(20);
    while d.status(0)["tailRevision"].as_u64().unwrap() == 0 || total(&d) < 6002 {
        assert!(Instant::now() < until, "append was not followed");
        thread::sleep(Duration::from_millis(5));
    }
    d.follow(false).unwrap();
    // The level table covers the replaced block again once analysis catches up.
    let until = Instant::now() + Duration::from_secs(20);
    while d.status(0)["summaryComplete"] != true {
        assert!(Instant::now() < until, "analysis did not catch up");
        thread::sleep(Duration::from_millis(5));
    }
    for (k, f) in [
        json!({"tag":"net"}),
        json!({"pid":"4242","levels":["E","W"]}),
        json!({"excludeTags":"ui"}),
        json!({"pid":"101, 4242"}),
    ]
    .into_iter()
    .enumerate()
    {
        let fresh = open(&path);
        let id = 10 + k as u64;
        let (a, b) = (
            query(&d, 1, id, wire(f.clone())),
            query(&fresh, 1, id, wire(f)),
        );
        complete(&d, &a);
        complete(&fresh, &b);
        assert_eq!(a.json()["count"], b.json()["count"], "filter {k}");
        assert_eq!(pages(&d, &a), pages(&fresh, &b), "filter {k}");
        fresh.close();
    }
    // The metadata no longer describes the replaced last block.
    let kept = d
        .index
        .lock()
        .unwrap()
        .block_fields
        .as_ref()
        .map(|t| t.blocks());
    assert!(kept.is_some_and(|k| k < blocks), "{kept:?} of {blocks}");
    d.close();
}
