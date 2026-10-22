use logstudio_core::{
    dataset::{Dataset, QueryStatus},
    search::{Filter, Filters},
    source::Source,
    Engine,
};
use serde_json::json;
use std::{
    thread,
    time::{Duration, Instant},
};
fn wait(mut f: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(5);
    while !f() {
        assert!(Instant::now() < until);
        thread::sleep(Duration::from_millis(1));
    }
}
fn bytes() -> Vec<u8> {
    let mut data = Vec::new();
    for i in 0..8000 {
        data.extend_from_slice(
            format!(
                "10-08 10:06:02.158 {} 009 I Tag: {} original 한글 🙂",
                if i % 3 == 0 { "007" } else { "009" },
                if i % 2 == 0 { "needle" } else { "other" }
            )
            .as_bytes(),
        );
        if i % 5 == 0 {
            data.extend_from_slice(b"\xff\xe2\x82");
        }
        data.extend_from_slice(if i % 3 == 0 {
            b"\r\n"
        } else if i % 3 == 1 {
            b"\r"
        } else {
            b"\n"
        });
    }
    data
}

#[test]
fn filtered_physical_locator_crosses_blocks_without_losing_damaged_rows() {
    // Close the writable handle before Source opens with Windows read-only sharing.
    let f = tempfile::NamedTempFile::new().unwrap().into_temp_path();
    std::fs::write(&f, bytes()).unwrap();
    let source = Source::granted(&f).unwrap();
    let d = Dataset::new(1, source);
    d.start(|_| {});
    wait(|| d.index.lock().unwrap().done);
    let q = d
        .query(
            0,
            1,
            Filter::new(Filters {
                pid: "007".into(),
                query: "needle".into(),
                ..Filters::default()
            })
            .unwrap(),
        )
        .unwrap();
    wait(|| q.json()["status"] == "complete");
    assert_eq!(q.json()["count"], 1334);
    for i in [0, 198, 3996, 7998] {
        let location = d.locate(&q, i + 1).unwrap();
        assert_eq!(location["found"], true);
        assert_eq!(location["index"], i / 6);
        let page = d.page(&q, i / 6, 200).unwrap();
        assert_eq!(page["rows"][0]["lineNumber"], i + 1);
        assert!(page["rows"][0]["raw"]
            .as_str()
            .unwrap()
            .contains("original 한글 🙂"));
        if i == 0 {
            assert_eq!(page["rows"][0]["encodingDamaged"], true);
        }
    }
    assert_eq!(d.locate(&q, 2).unwrap()["found"], false);
    d.close();
}

#[test]
fn locator_reports_partial_and_cancelled_ranges_without_waiting_for_unscanned_blocks() {
    // Close the writable handle before Source opens with Windows read-only sharing.
    let f = tempfile::NamedTempFile::new().unwrap().into_temp_path();
    std::fs::write(&f, bytes()).unwrap();
    let d = Dataset::new(1, Source::granted(&f).unwrap());
    d.start(|_| {});
    wait(|| d.index.lock().unwrap().done);
    let q = d
        .query(0, 1, Filter::new(Filters::default()).unwrap())
        .unwrap();
    wait(|| q.json()["status"] == "complete");
    let saved = {
        let mut r = q.result.lock().unwrap();
        let saved = r.ends.clone();
        r.ends.truncate(1);
        r.status = QueryStatus::Running;
        saved
    };
    let running = d.locate(&q, 8000).unwrap();
    assert_eq!(running["pending"], true);
    assert_eq!(running["incomplete"], false);
    q.result.lock().unwrap().status = QueryStatus::Cancelled;
    let partial = d.locate(&q, 8000).unwrap();
    assert_eq!(partial["pending"], false);
    assert_eq!(partial["found"], false);
    // Never counted is unknown, not a final absence from the results.
    assert_eq!(partial["incomplete"], true);
    {
        let mut r = q.result.lock().unwrap();
        r.ends = saved;
        r.status = QueryStatus::Complete;
    }
    assert_eq!(d.locate(&q, 8000).unwrap()["index"], 7999);
    d.close();
}

#[test]
fn locator_ipc_rejects_invalid_and_stale_queries() {
    let e = std::sync::Arc::new(Engine::default());
    let id = e.begin_open();
    e.open_text(
        id,
        "10-08 10:06:02.158 007 009 I Tag: one\nraw\n",
        "synthetic",
    )
    .unwrap();
    wait(|| e.request(json!({"action":"status"})).unwrap()["textComplete"] == true);
    e.request(json!({"action":"query","datasetId":id,"queryId":1,"filters":{}}))
        .unwrap();
    wait(|| e.request(json!({"action":"status"})).unwrap()["query"]["status"] == "complete");
    let request = json!({"action":"locate","datasetId":id,"queryId":1,"lineNumber":2});
    assert_eq!(e.request(request.clone()).unwrap()["index"], 1);
    let mut invalid = request.clone();
    invalid["lineNumber"] = json!(0);
    assert!(e.request(invalid).is_err());
    e.request(json!({"action":"query","datasetId":id,"queryId":2,"filters":{}}))
        .unwrap();
    assert!(e.request(request.clone()).is_err());
    e.close();
    assert!(e.request(request).is_err());
}
