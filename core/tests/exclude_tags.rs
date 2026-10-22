use logstudio_core::{
    dataset::Dataset,
    search::{Filter, Filters},
    source::Source,
};
use std::{
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

const LOG: &str = "06-18 19:24:00.000  100 200 I Choreographer: Skipped 30 frames
06-18 19:24:00.050  100 200 D chatty  : uid=1000 expire
06-18 19:24:00.100  100 200 W ExampleApp: slow response
raw line without fields
2026-09-15 10:47:35.134 Runner[3870:481516] [Memory] heap used = 58.36 MB
2026-09-15 10:47:35.184 Runner[3870:481490] Shader cache pool loaded
";

fn wait(mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !done() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
}

fn count(d: &Arc<Dataset>, id: u64, filters: Filters) -> u64 {
    let q = d.query(0, id, Filter::new(filters).unwrap()).unwrap();
    wait(|| q.json()["status"] == "complete");
    q.json()["count"].as_u64().unwrap()
}

#[test]
fn excluded_tags_remove_only_tagged_rows_with_literal_case_insensitive_matches() {
    let d = Dataset::new(1, Source::text(LOG, "exclude").unwrap());
    d.start(|_| {});
    // With the level table published, exclusion must still scan each block.
    wait(|| d.status(0)["complete"] == true);
    let mut id = 0;
    for (filters, expected) in [
        (
            Filters {
                exclude_tags: "choreo".into(),
                ..Filters::default()
            },
            5,
        ),
        (
            Filters {
                exclude_tags: " choreo, CHATTY ,,choreo".into(),
                ..Filters::default()
            },
            4,
        ),
        // iOS bracketed components are tags; an untagged iOS row stays.
        (
            Filters {
                exclude_tags: "memory".into(),
                ..Filters::default()
            },
            5,
        ),
        // Entries are literals, never patterns.
        (
            Filters {
                exclude_tags: "Exam.leApp".into(),
                ..Filters::default()
            },
            6,
        ),
        (
            Filters {
                tag: "Example".into(),
                exclude_tags: "app".into(),
                ..Filters::default()
            },
            0,
        ),
        (
            Filters {
                exclude_tags: "choreo".into(),
                include_raw: false,
                ..Filters::default()
            },
            // The raw row continues the ExampleApp entry, not an orphan.
            5,
        ),
        (
            Filters {
                levels: vec!["I".into()],
                exclude_tags: "choreo".into(),
                ..Filters::default()
            },
            0,
        ),
        (
            Filters {
                levels: vec!["I".into(), "W".into()],
                exclude_tags: "choreo".into(),
                ..Filters::default()
            },
            // ExampleApp (W) and the raw row that continues it.
            2,
        ),
        (
            Filters {
                exclude_tags: " , ".into(),
                ..Filters::default()
            },
            6,
        ),
    ] {
        id += 1;
        assert_eq!(count(&d, id, filters.clone()), expected, "{filters:?}");
    }
    d.close();
}

#[test]
fn excluded_tag_list_is_normalized_and_bounded() {
    let filter = Filter::new(Filters {
        exclude_tags: " a, B ,a,,".into(),
        ..Filters::default()
    })
    .unwrap();
    assert_eq!(filter.fields.exclude_tags, "a,B");
    assert!(!filter.identity());
    assert!(!filter.only_levels());
    assert!(Filter::new(Filters {
        exclude_tags: " ,  ".into(),
        ..Filters::default()
    })
    .unwrap()
    .identity());
    let many = (0..33)
        .map(|i| format!("t{i}"))
        .collect::<Vec<_>>()
        .join(",");
    assert!(Filter::new(Filters {
        exclude_tags: many,
        ..Filters::default()
    })
    .is_err());
    let json = serde_json::json!({"excludeTags": "chatty"});
    let parsed: Filters = serde_json::from_value(json).unwrap();
    assert_eq!(parsed.exclude_tags, "chatty");
}
