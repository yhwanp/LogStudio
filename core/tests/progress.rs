use logstudio_core::{
    dataset::Dataset,
    search::{Filter, Filters},
    source::Source,
    Engine,
};
use serde_json::{json, Value};
use std::{
    sync::{mpsc, Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

fn wait(mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(4);
    while !done() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn notifications_are_bounded_to_first_available_and_terminal_without_metadata() {
    let d = Dataset::new(
        1,
        Source::text(&"raw COMMON 한글 🙂\r\n".repeat(40000), "progress").unwrap(),
    );
    d.pause_analysis
        .store(true, std::sync::atomic::Ordering::Release);
    d.start(|_| {});
    wait(|| d.index.lock().unwrap().done);
    let (tx, rx) = mpsc::channel();
    d.observe_progress(move |pane, state| {
        tx.send((pane, state)).unwrap();
    });
    let q = d
        .query(
            1,
            1,
            Filter::new(Filters {
                query: "common".into(),
                ..Filters::default()
            })
            .unwrap(),
        )
        .unwrap();
    let (pane, first) = rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(pane, 1);
    assert_eq!(first["query"]["status"], "running");
    assert!(first["query"]["count"].as_u64().unwrap() > 0);
    assert!(first["query"]["count"].as_u64().unwrap() < 40000);
    assert_eq!(first["summaryComplete"], false);
    let (_, terminal) = rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(terminal["query"]["status"], "complete");
    assert_eq!(terminal["query"]["count"], 40000);
    // Polls and notifications share one ordered revision sequence.
    let revision = |v: &Value| v["revision"].as_u64().unwrap();
    assert!(revision(&terminal) > revision(&first));
    assert!(revision(&d.status(1)) > revision(&terminal));
    q.cancel();
    assert!(rx.try_recv().is_err(), "no per-block/progress flood");
    let q = d
        .query(
            1,
            2,
            Filter::new(Filters {
                query: "NO_MATCH".into(),
                ..Filters::default()
            })
            .unwrap(),
        )
        .unwrap();
    let (_, terminal) = rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(terminal["query"]["id"], 2);
    assert_eq!(terminal["query"]["count"], 0);
    assert_eq!(terminal["query"]["status"], "complete");
    q.cancel();
    assert!(rx.try_recv().is_err());
    d.close();
}

#[test]
fn engine_feedback_is_identified_and_cancelled_queries_keep_partial_results() {
    let e = Arc::new(Engine::default());
    let (events, rx) = mpsc::channel();
    e.observe(move |event| {
        events.send(event).unwrap();
    });
    let id = e.begin_open();
    e.open_text(id, &"raw COMMON\n".repeat(20000), "engine-progress")
        .unwrap();
    wait(|| e.request(json!({"action":"status"})).unwrap()["textComplete"] == true);
    e.request(json!({"action":"query","pane":"search","datasetId":id,"queryId":1,"filters":{"query":"common"}})).unwrap();
    wait(|| {
        e.request(json!({"action":"status","pane":"search"}))
            .unwrap()["query"]["status"]
            == "complete"
    });
    // Joining guarantees the terminal observer has finished before inspecting.
    e.request(json!({"action":"cancel","pane":"search","datasetId":id,"queryId":1}))
        .unwrap();
    let feedback: Vec<Value> = rx
        .try_iter()
        .filter(|v| v["status"] == "query-progress")
        .collect();
    assert_eq!(feedback.len(), 2);
    for event in feedback {
        assert_eq!(event["pane"], "search");
        assert_eq!(event["state"]["datasetId"], id);
        assert_eq!(event["state"]["query"]["id"], 1);
    }
    e.close();

    let d = Dataset::new(
        9,
        Source::text(&"raw COMMON\n".repeat(20000), "cancel-progress").unwrap(),
    );
    d.start(|_| {});
    wait(|| d.index.lock().unwrap().done);
    let (entered, first) = mpsc::channel();
    let (release, gate) = mpsc::channel();
    let gate = Mutex::new(gate);
    d.observe_progress(move |_, state| {
        if state["query"]["status"] == "running" {
            entered.send(()).unwrap();
            gate.lock().unwrap().recv().unwrap();
        }
    });
    let q = d
        .query(
            1,
            1,
            Filter::new(Filters {
                query: "common".into(),
                ..Filters::default()
            })
            .unwrap(),
        )
        .unwrap();
    first.recv_timeout(Duration::from_secs(2)).unwrap();
    let cancelling = q.clone();
    let join = thread::spawn(move || cancelling.cancel());
    wait(|| q.stop.load(std::sync::atomic::Ordering::Acquire));
    release.send(()).unwrap();
    join.join().unwrap();
    assert_eq!(q.json()["status"], "cancelled");
    assert!(!d.page(&q, 0, 200).unwrap()["rows"]
        .as_array()
        .unwrap()
        .is_empty());
    d.close();
}

#[test]
fn replacing_source_joins_a_notifying_query_without_holding_engine_state() {
    let e = Arc::new(Engine::default());
    let id = e.begin_open();
    e.open_text(id, &"raw COMMON\n".repeat(40000), "old-progress")
        .unwrap();
    wait(|| e.request(json!({"action":"status"})).unwrap()["textComplete"] == true);
    let (entered, first) = mpsc::channel();
    let (release, gate) = mpsc::channel();
    let gate = Mutex::new(gate);
    let (events, rx) = mpsc::channel();
    e.observe(move |event| {
        if event["status"] == "query-progress"
            && event["state"]["datasetId"] == id
            && event["state"]["query"]["status"] == "running"
        {
            entered.send(()).unwrap();
            gate.lock().unwrap().recv().unwrap();
        }
        events.send(event).unwrap();
    });
    e.request(json!({"action":"query","pane":"search","datasetId":id,"queryId":1,"filters":{"query":"common"}})).unwrap();
    first.recv_timeout(Duration::from_secs(2)).unwrap();
    let next = e.begin_open();
    let replacing = e.clone();
    let (done, result) = mpsc::channel();
    let worker = thread::spawn(move || {
        done.send(replacing.open_text(next, "new raw\n", "new-progress"))
            .unwrap();
    });
    // Keep the old worker in its observer while installation starts joining it.
    thread::sleep(Duration::from_millis(20));
    release.send(()).unwrap();
    assert_eq!(
        result
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
            .unwrap()["datasetId"],
        next
    );
    worker.join().unwrap();
    wait(|| e.request(json!({"action":"status"})).unwrap()["textComplete"] == true);
    assert!(rx
        .try_iter()
        .all(|event| !(event["status"] == "query-progress"
            && event["state"]["datasetId"] == id
            && event["state"]["query"]["status"] == "cancelled")));
    e.close();
}
