use logstudio_core::{search::Filters, Engine};
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
fn query(e: &Engine, d: u64, pane: &str, id: u64, filters: Filters) {
    request(e, d, id, "query", json!({"pane":pane,"filters":filters}));
    wait(|| request(e, d, id, "status", json!({"pane":pane}))["query"]["status"] == "complete");
}
fn rows(page: &Value) -> Vec<(u64, bool, Value)> {
    page["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            let hit = r.get("searchHit") == Some(&json!(true));
            (
                r["lineNumber"].as_u64().unwrap(),
                hit,
                r["highlights"].clone(),
            )
        })
        .collect()
}

// Upper rows carry the search pane's highlights and hit marks from the same
// native matcher; a replaced, cleared or differently filtered search marks
// nothing, and CR/LF/CRLF rows and UTF-16 offsets stay as in search pages.
#[test]
fn upper_page_marks_exactly_the_current_search_results() {
    let e = Arc::new(Engine::default());
    let id = e.begin_open();
    let text = "09-21 12:07:30.386 101 9 I 한글Tag: 🙂 ΣIGMA common\r\n\
                09-21 12:07:30.387 102 9 I Other: σigma elsewhere\n\
                09-21 12:07:30.388 101 9 W Tag: {\r\
                \x20 \"body\": \"🙂σ\"\r\n\
                09-21 12:07:30.389 101 9 E Tag: plain COMMON\n\
                raw after σ";
    e.open_text(id, text, "unicode").unwrap();
    wait(|| e.request(json!({"action":"status"})).unwrap()["textComplete"] == true);
    let fields = || Filters {
        pid: "101".into(),
        ..Filters::default()
    };
    query(&e, id, "logs", 1, fields());
    query(
        &e,
        id,
        "search",
        1,
        Filters {
            query: "σ".into(),
            ..fields()
        },
    );
    let search = request(&e, id, 1, "page", json!({"pane":"search"}));
    let expected: Vec<(u64, bool, Value)> = rows(&search)
        .into_iter()
        .map(|(l, _, h)| (l, true, h))
        .collect();
    assert_eq!(
        expected.iter().map(|r| r.0).collect::<Vec<_>>(),
        vec![1, 4, 6]
    );
    let plain = request(&e, id, 1, "page", json!({"pane":"logs"}));
    assert!(plain.get("searchQueryId").is_none());
    assert!(rows(&plain)
        .iter()
        .all(|(_, hit, h)| !hit && h == &json!([])));
    let marked = request(&e, id, 1, "page", json!({"pane":"logs","searchQueryId":1}));
    assert_eq!(marked["searchQueryId"], 1);
    let upper = rows(&marked);
    assert_eq!(
        upper.iter().map(|r| r.0).collect::<Vec<_>>(),
        vec![1, 3, 4, 5, 6]
    );
    for row in &upper {
        match expected.iter().find(|r| r.0 == row.0) {
            Some(hit) => assert_eq!(row, hit),
            None => assert_eq!(row, &(row.0, false, json!([]))),
        }
    }
    // "ΣIGMA" folds to the query; offsets are UTF-16 units after the emoji.
    assert_eq!(upper[0].2, json!([[37, 38]]));
    // A replaced search identity marks nothing, and the new one marks its own.
    query(
        &e,
        id,
        "search",
        2,
        Filters {
            query: "COMMON".into(),
            case_sensitive: true,
            ..fields()
        },
    );
    let stale = request(&e, id, 1, "page", json!({"pane":"logs","searchQueryId":1}));
    assert!(stale.get("searchQueryId").is_none());
    assert!(rows(&stale)
        .iter()
        .all(|(_, hit, h)| !hit && h == &json!([])));
    let current = request(&e, id, 1, "page", json!({"pane":"logs","searchQueryId":2}));
    assert_eq!(
        rows(&current)
            .iter()
            .filter(|r| r.1)
            .map(|r| r.0)
            .collect::<Vec<_>>(),
        vec![5]
    );
    // Other field conditions or a cleared query never mark upper rows.
    query(
        &e,
        id,
        "search",
        3,
        Filters {
            query: "σ".into(),
            ..Filters::default()
        },
    );
    let other = request(&e, id, 1, "page", json!({"pane":"logs","searchQueryId":3}));
    assert!(other.get("searchQueryId").is_none());
    query(&e, id, "search", 4, fields());
    let cleared = request(&e, id, 1, "page", json!({"pane":"logs","searchQueryId":4}));
    assert!(cleared.get("searchQueryId").is_none());
    assert!(rows(&cleared).iter().all(|(_, hit, _)| !hit));
    // The search pane's own pages ignore the field.
    let lower = request(
        &e,
        id,
        4,
        "page",
        json!({"pane":"search","searchQueryId":4}),
    );
    assert!(lower.get("searchQueryId").is_none());
    e.close();
}
