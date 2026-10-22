//! Tag, PID and TID filters take lists: any item of a field may match (OR),
//! fields still combine with AND, and one value behaves as before.
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

// Row 1 is an orphan, row 5 continues row 4, row 7 has three iOS tags and
// row 8 none.
const LOG: &str = "--------- beginning of main
06-18 19:24:00.000  100 200 I Choreographer: Skipped 30 frames
06-18 19:24:00.050  101 201 D chatty  : uid=1000 expire
06-18 19:24:00.100  102 202 W ExampleApp: slow response
\tat com.example.Foo.run(Foo.java:1)
06-18 19:24:00.150  1000 2000 E ActivityManager: ANR
2026-09-15 10:47:35.134 Runner[3870:481516] [Memory] [SDK_UI] [GalleryManager] heap used
2026-09-15 10:47:35.184 Runner[3870:481490] Shader cache pool loaded
2026-09-15 10:47:35.200 Runner[3871:481516] [Network] request
";

fn wait(mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
}
fn lines(d: &Arc<Dataset>, id: u64, filters: Filters) -> Vec<u64> {
    let q = d.query(0, id, Filter::new(filters).unwrap()).unwrap();
    wait(|| q.json()["status"] == "complete");
    let page = d.page(&q, 0, 200).unwrap();
    page["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["lineNumber"].as_u64().unwrap())
        .collect()
}
fn filters(tag: &str, exclude: &str, pid: &str, tid: &str) -> Filters {
    Filters {
        tag: tag.into(),
        exclude_tags: exclude.into(),
        pid: pid.into(),
        tid: tid.into(),
        ..Filters::default()
    }
}

#[test]
fn list_items_match_any_within_a_field_and_all_across_fields() {
    let d = Dataset::new(1, Source::text(LOG, "lists").unwrap());
    d.start(|_| {});
    wait(|| d.status(0)["complete"] == true);
    let cases: &[(Filters, &[u64])] = &[
        (filters("", "", "", ""), &[1, 2, 3, 4, 5, 6, 7, 8, 9]),
        // One value: exact IDs, case-insensitive tag substrings.
        (filters("", "", "100", ""), &[2]),
        (filters("", "", "10", ""), &[]),
        (filters("", "", "0100", ""), &[]),
        (filters("", "", "102", ""), &[4, 5]),
        (filters("CHOREO", "", "", ""), &[2]),
        (filters("runner", "", "", ""), &[]),
        // Lists: the continuation row follows its entry, the orphan and the
        // untagged iOS row never match an included tag or ID.
        (filters("", "", "100, 102", ""), &[2, 4, 5]),
        (filters("", "", "100 102", ""), &[2, 4, 5]),
        (filters("", "", "\t100,,100 , ", ""), &[2]),
        (filters("", "", "3870,3871", ""), &[7, 8, 9]),
        (filters("", "", "", "200, 481516"), &[2, 7, 9]),
        (filters("", "", "100, 3870", "481516, 201"), &[7]),
        (filters("choreo, example", "", "", ""), &[2, 4, 5]),
        (filters(" CHOREO , , choreo", "", "", ""), &[2]),
        // Any of an iOS row's tags may match any item.
        (filters("gallery", "", "", ""), &[7]),
        (filters("nosuch, gallery, network", "", "", ""), &[7, 9]),
        // An item with a line break matches no tag but keeps the filter on.
        (filters("a\nb", "", "", ""), &[]),
        (filters("a\nb, choreo", "", "", ""), &[2]),
        // Excluded tags still win over included ones, on any of a row's tags.
        (
            filters("choreo, example, gallery", "sdk, app", "", ""),
            &[2],
        ),
        (filters("choreo, example", "", "102, 1000", ""), &[4, 5]),
        (
            filters("", "choreo, memory", "", ""),
            &[1, 3, 4, 5, 6, 8, 9],
        ),
    ];
    for (id, (f, expected)) in cases.iter().enumerate() {
        assert_eq!(&lines(&d, id as u64 + 1, f.clone()), expected, "{f:?}");
    }
    d.close();
}

#[test]
fn lists_are_normalized_bounded_and_validated() {
    let f = Filter::new(filters(
        " a, B ,a,,",
        " x ,x",
        " 2983 ,3120\u{3000}2983,,",
        "\t1\n",
    ))
    .unwrap();
    assert_eq!(
        (f.fields.tag.as_str(), f.fields.exclude_tags.as_str()),
        ("a,B", "x")
    );
    assert_eq!(
        (f.fields.pid.as_str(), f.fields.tid.as_str()),
        ("2983,3120", "1")
    );
    // One value keeps its trimmed form.
    let one = Filter::new(filters("  Wifi ", "", " 12 ", "")).unwrap();
    assert_eq!(
        (one.fields.tag.as_str(), one.fields.pid.as_str()),
        ("Wifi", "12")
    );
    // Separators alone are no filter at all.
    let blank = Filter::new(filters(" , ", " , ", " , ", "  ")).unwrap();
    assert!(
        blank.identity() && blank.only_levels() && blank.admits_unparsed() && !blank.needs_fields()
    );
    let ids = |n: usize| {
        (0..n)
            .map(|i| (1000 + i).to_string())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let tags = |n: usize| {
        (0..n)
            .map(|i| format!("t{i}"))
            .collect::<Vec<_>>()
            .join(",")
    };
    assert!(Filter::new(filters(&tags(32), &tags(32), &ids(32), &ids(32))).is_ok());
    for f in [
        filters(&tags(33), "", "", ""),
        filters("", &tags(33), "", ""),
        filters("", "", &ids(33), ""),
        filters("", "", "", &ids(33)),
        // Duplicates do not count, but the byte budget still applies.
        filters("", "", &"1,".repeat(501), ""),
    ] {
        assert!(Filter::new(f.clone()).is_err(), "{f:?}");
    }
    assert!(Filter::new(filters("", "", &"1,".repeat(450), "")).is_ok());
    for bad in ["12a", "１２", "12, abc", "-1", "1;2", "+3"] {
        assert!(Filter::new(filters("", "", bad, "")).is_err(), "{bad}");
        assert!(Filter::new(filters("", "", "", bad)).is_err(), "{bad}");
    }
    // The IPC shape is unchanged: lists stay strings.
    let json =
        serde_json::json!({"tag": "ActivityManager, MyApp", "pid": "2983, 3120", "tid": "1 2"});
    let parsed: Filters = serde_json::from_value(json).unwrap();
    assert_eq!(parsed.pid, "2983, 3120");
    assert!(serde_json::from_value::<Filters>(serde_json::json!({"pid": ["1"]})).is_err());
}

#[test]
fn row_predicates_and_rules_agree_with_lists() {
    let f = Filter::new(filters("Activity, sdk", "manager", "2983 3120", "7")).unwrap();
    // The shorthand fields are one showing and one hiding condition.
    let ([show], [hide]) = (f.shows(), f.hides()) else {
        panic!("one condition each");
    };
    assert!(
        show.accepts_tag("ActivityThread")
            && show.accepts_tag("SDK_UI")
            && !show.accepts_tag("chatty")
    );
    assert!(hide.accepts_tag("ActivityManager") && !hide.accepts_tag("ActivityThread"));
    assert!(
        show.tags_match(&["ActivityManager"]) && hide.tags_match(&["ActivityManager"]),
        "included and excluded"
    );
    assert!(show.tags_match(&["Memory", "SDK_UI"]) && !show.tags_match(&["Memory"]));
    assert!(!show.tags_match(&[]), "an include list needs a tag");
    assert!(
        !hide.tags_match(&[]),
        "a row without tags is never excluded"
    );
    assert!(show.accepts_pid("3120") && show.accepts_pid("2983") && !show.accepts_pid("312"));
    assert!(
        !show.accepts_pid("")
            && !show.accepts_pid("29833")
            && show.accepts_tid("7")
            && !show.accepts_tid("07")
    );
    assert!(
        hide.accepts_pid("") && hide.accepts_tid("1"),
        "no list accepts every ID"
    );
    let none = Filter::new(Filters::default()).unwrap();
    assert!(none.shows().is_empty() && none.hides().is_empty());
    let exclude = Filter::new(filters("", "chatty", "", "")).unwrap();
    assert!(exclude.shows().is_empty() && exclude.hides().len() == 1);
    // Many IDs are looked up in sorted order but keep their typed order.
    let many = Filter::new(filters("", "", "30 4 1000 200", "")).unwrap();
    assert_eq!(many.fields.pid, "30,4,1000,200");
    for (pid, listed) in [
        ("4", true),
        ("30", true),
        ("200", true),
        ("1000", true),
        ("3", false),
        ("40", false),
        ("2000", false),
    ] {
        assert_eq!(many.shows()[0].accepts_pid(pid), listed, "{pid}");
    }
    // Lists are field filters: orphans fall out, and the normal form makes
    // equally meant lists the same fields.
    for f in [
        filters("a, b", "", "", ""),
        filters("", "", "1, 2", ""),
        filters("", "", "", "1 2"),
    ] {
        let filter = Filter::new(f.clone()).unwrap();
        assert!(
            !filter.identity() && !filter.only_levels() && filter.needs_fields(),
            "{f:?}"
        );
        assert!(!filter.admits_unparsed(), "{f:?}");
    }
    let a = Filter::new(filters("a, b", "", "1 2", "3")).unwrap();
    let b = Filter::new(filters(" a,b ", "", "1,2", " 3 ")).unwrap();
    let c = Filter::new(filters("a, b", "", "1", "3")).unwrap();
    assert!(a.same_fields(&b) && !a.same_fields(&c));
}

#[test]
fn continuation_rows_take_their_entry_decision_across_blocks() {
    // A stack trace longer than two index blocks under PID 300, then PID 301.
    let mut text = String::from("06-18 19:24:00.000  300 1 E Big: start\n");
    for i in 0..4000 {
        text += &format!(
            "\tat com.example.feature.Frame{i:04}.run(Frame.java:{i}) padding padding padding\n"
        );
    }
    text += "06-18 19:24:01.000  301 1 I Small: next\n\tat one\n\tat two\n";
    assert!(text.len() > 2 * logstudio_core::source::BLOCK);
    let d = Dataset::new(1, Source::text(&text, "blocks").unwrap());
    d.start(|_| {});
    wait(|| d.status(0)["complete"] == true);
    let count = |id, f: Filters| {
        let q = d.query(0, id, Filter::new(f).unwrap()).unwrap();
        wait(|| q.json()["status"] == "complete");
        q.json()["count"].as_u64().unwrap()
    };
    assert_eq!(count(1, filters("", "", "300, 999", "")), 4001);
    assert_eq!(count(2, filters("", "", "999 301", "")), 3);
    assert_eq!(count(3, filters("", "", "301, 300", "")), 4004);
    assert_eq!(count(4, filters("nosuch, big", "", "", "")), 4001);
    assert_eq!(count(5, filters("big, small", "small", "", "")), 4001);
    d.close();
}
