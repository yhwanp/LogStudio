//! A tag, PID, TID or DLT ID holds one value, or with its switch a pattern: a
//! tag is text the field contains and an ID the field's whole value, a
//! comma is ordinary text, and fields still combine with AND.
use logstudio_core::{
    dataset::Dataset,
    search::{Condition, Filter, Filters},
    source::{self, Source},
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
/// The rows of a filter; the count must agree, tables or no tables.
fn lines(d: &Arc<Dataset>, id: u64, filters: Filters) -> Vec<u64> {
    let q = d.query(0, id, Filter::new(filters).unwrap()).unwrap();
    wait(|| q.json()["status"] == "complete");
    let page = d.page(&q, 0, 200).unwrap();
    let rows: Vec<u64> = page["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["lineNumber"].as_u64().unwrap())
        .collect();
    assert_eq!(q.json()["count"], rows.len() as u64);
    rows
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
fn show(c: Condition) -> Filters {
    Filters {
        include: vec![c],
        ..Filters::default()
    }
}
fn hide(c: Condition) -> Filters {
    Filters {
        exclude: vec![c],
        ..Filters::default()
    }
}
fn tag(pattern: &str) -> Condition {
    Condition {
        tag: pattern.into(),
        tag_regex: true,
        ..Condition::default()
    }
}
fn pid(pattern: &str) -> Condition {
    Condition {
        pid: pattern.into(),
        pid_regex: true,
        ..Condition::default()
    }
}

#[test]
fn values_and_patterns_match_rows() {
    let d = Dataset::new(1, Source::text(LOG, "values").unwrap());
    d.start(|_| {});
    wait(|| d.status(0)["complete"] == true);
    let cases: &[(Filters, &[u64])] = &[
        (filters("", "", "", ""), &[1, 2, 3, 4, 5, 6, 7, 8, 9]),
        // A value: whole IDs, case-insensitive tag text.
        (filters("", "", "100", ""), &[2]),
        (filters("", "", "10", ""), &[]),
        (filters("", "", "0100", ""), &[]),
        (filters("", "", "102", ""), &[4, 5]),
        (filters("CHOREO", "", "", ""), &[2]),
        (filters("runner", "", "", ""), &[]),
        // A comma is text: no tag holds this one.
        (filters("choreo, example", "", "", ""), &[]),
        // Any of an iOS row's tags may match.
        (filters("gallery", "", "", ""), &[7]),
        // A value with a line break matches no tag but keeps the filter on.
        (filters("a\nb", "", "", ""), &[]),
        (filters("", "memory", "", ""), &[1, 2, 3, 4, 5, 6, 8, 9]),
        // Patterns find a match anywhere in the field: the continuation row
        // follows its entry; the orphan and the untagged iOS row never match.
        (show(pid("^(?:100|102)$")), &[2, 4, 5]),
        (show(pid("^10")), &[2, 3, 4, 5, 6]),
        (show(pid("^(?:3870|3871)$")), &[7, 8, 9]),
        (
            show(Condition {
                tid: "^(?:200|481516)$".into(),
                tid_regex: true,
                ..Condition::default()
            }),
            &[2, 7, 9],
        ),
        (show(tag("choreo|example")), &[2, 4, 5]),
        (show(tag("nosuch|gallery|network")), &[7, 9]),
        // A row without the field holds no value, whatever the pattern accepts.
        (show(tag(".*")), &[2, 3, 4, 5, 6, 7, 9]),
        (show(tag("^$")), &[]),
        (hide(tag(".*")), &[1, 8]),
        // Patterns ignore case unless they say otherwise.
        (show(tag("(?-i)CHOREO")), &[]),
        (show(tag("(?-i)Choreo")), &[2]),
        // Fields still combine with AND, and hiding wins.
        (
            Filters {
                exclude_tags: "app".into(),
                include: vec![tag("choreo|example|gallery")],
                ..Filters::default()
            },
            &[2, 7],
        ),
        (
            show(Condition {
                pid: "^10".into(),
                pid_regex: true,
                tag: "example".into(),
                ..Condition::default()
            }),
            &[4, 5],
        ),
    ];
    for (id, (f, expected)) in cases.iter().enumerate() {
        assert_eq!(&lines(&d, id as u64 + 1, f.clone()), expected, "{f:?}");
    }
    d.close();
}

#[test]
fn values_are_normalized_bounded_and_validated() {
    let f = Filter::new(filters(" a, B ", " x ", " 2983 ", "\t1\n")).unwrap();
    assert_eq!(
        (f.fields.tag.as_str(), f.fields.exclude_tags.as_str()),
        ("a, B", "x")
    );
    assert_eq!(
        (f.fields.pid.as_str(), f.fields.tid.as_str()),
        ("2983", "1")
    );
    // Blank fields are no filter at all, and a blank field has no mode.
    let blank = Filter::new(filters("  ", "  ", " ", "  ")).unwrap();
    assert!(
        blank.identity() && blank.only_levels() && blank.admits_unparsed() && !blank.needs_fields()
    );
    let off = Filter::new(show(Condition {
        tag_regex: true,
        pid_regex: true,
        apid_regex: true,
        ..Condition::default()
    }))
    .unwrap();
    assert!(off.identity());
    let c = &off.fields.include[0];
    assert!(
        !c.tag_regex && !c.pid_regex && !c.apid_regex,
        "a blank field has no mode"
    );
    for bad in ["12a", "１２", "12, 34", "1 2", "-1", "1;2", "+3"] {
        assert!(Filter::new(filters("", "", bad, "")).is_err(), "{bad}");
        assert!(Filter::new(filters("", "", "", bad)).is_err(), "{bad}");
    }
    assert!(
        Filter::new(show(pid("12a"))).is_ok(),
        "a pattern may be any text"
    );
    assert!(Filter::new(filters("", "", &"1".repeat(1001), "")).is_err());
    assert!(Filter::new(filters("", "", &"1".repeat(1000), "")).is_ok());
    assert!(serde_json::from_value::<Filters>(serde_json::json!({"pid": ["1"]})).is_err());
}

#[test]
fn a_saved_list_becomes_a_pattern_of_the_same_rows() {
    // What the page's `upgradeCondition` writes for the items, each as the
    // old list matched it: text inside the tag, ignoring case.
    let items = [
        ("a.b", r"a\.b"),
        ("c(d", r"c\(d"),
        ("x-y", r"x\-y"),
        ("m#n", r"m\#n"),
        ("p&q", r"p\&q"),
        ("r~s", r"r\~s"),
        ("t u", "t u"),
        ("[z]", r"\[z\]"),
        ("\u{e4}rger", "\u{e4}rger"),
    ];
    for (item, escaped) in items {
        let f = Filter::new(show(tag(escaped))).unwrap();
        let [c] = f.shows() else {
            panic!("one condition")
        };
        assert!(c.accepts_tag(&format!("before{item}after")), "{item}");
        assert!(c.accepts_tag(&item.to_uppercase()), "{item} in capitals");
        assert!(!c.accepts_tag("nothing"), "{item}");
    }
    assert!(Filter::new(show(pid("^(2983|3120)$"))).is_ok());
}

#[test]
fn a_bad_pattern_names_its_field() {
    let error = |c: Condition| Filter::new(show(c)).err().unwrap().to_string();
    let bad = "(";
    for (name, c) in [
        ("tag", tag(bad)),
        ("PID", pid(bad)),
        (
            "TID",
            Condition {
                tid: bad.into(),
                tid_regex: true,
                ..Condition::default()
            },
        ),
        (
            "ECU",
            Condition {
                ecu: bad.into(),
                ecu_regex: true,
                ..Condition::default()
            },
        ),
        (
            "APID",
            Condition {
                apid: bad.into(),
                apid_regex: true,
                ..Condition::default()
            },
        ),
        (
            "CTID",
            Condition {
                ctid: bad.into(),
                ctid_regex: true,
                ..Condition::default()
            },
        ),
        (
            "message",
            Condition {
                message: bad.into(),
                message_regex: true,
                ..Condition::default()
            },
        ),
    ] {
        let text = error(c);
        let want = format!("Invalid or unsupported {name} pattern: ");
        assert!(text.starts_with(&want), "{text}");
    }
    // A value is not a pattern.
    assert!(Filter::new(filters("(", "", "", "")).is_ok());
    assert!(Filter::new(show(Condition {
        apid: "(".into(),
        ..Condition::default()
    }))
    .is_ok());
}

#[test]
fn row_predicates_and_rules_agree() {
    let f = Filter::new(Filters {
        include: vec![Condition {
            tag: "Activity|sdk".into(),
            tag_regex: true,
            pid: "^(?:2983|3120)$".into(),
            pid_regex: true,
            tid: "7".into(),
            ..Condition::default()
        }],
        exclude_tags: "manager".into(),
        ..Filters::default()
    })
    .unwrap();
    let ([show], [hide]) = (f.shows(), f.hides()) else {
        panic!("one condition each");
    };
    assert!(
        show.accepts_tag("ActivityThread")
            && show.accepts_tag("SDK_UI")
            && !show.accepts_tag("chatty")
    );
    assert!(hide.accepts_tag("ActivityManager") && !hide.accepts_tag("ActivityThread"));
    assert!(show.tags_match(&["Memory", "SDK_UI"]) && !show.tags_match(&["Memory"]));
    assert!(!show.tags_match(&[]), "a tag test needs a tag");
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
    assert!(hide.accepts_pid("") && hide.accepts_tid("1"));
    // A value or a pattern is a field filter: orphans fall out.
    for f in [
        filters("a", "", "", ""),
        filters("", "", "1", ""),
        filters("", "", "", "1"),
        self::show(tag("a|b")),
        self::show(pid("1|2")),
    ] {
        let filter = Filter::new(f.clone()).unwrap();
        assert!(
            !filter.identity() && !filter.only_levels() && filter.needs_fields(),
            "{f:?}"
        );
        assert!(!filter.admits_unparsed(), "{f:?}");
    }
    let a = Filter::new(filters("a", "", "1", "3")).unwrap();
    let b = Filter::new(filters(" a ", "", "1", " 3 ")).unwrap();
    let c = Filter::new(filters("a", "", "2", "3")).unwrap();
    assert!(a.same_fields(&b) && !a.same_fields(&c));
    // The same text as a value and as a pattern are different fields.
    let value = Filter::new(self::show(Condition {
        tag: "a".into(),
        ..Condition::default()
    }))
    .unwrap();
    let pattern = Filter::new(self::show(tag("a"))).unwrap();
    assert!(!value.same_fields(&pattern));
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
    assert!(text.len() > 2 * source::BLOCK);
    let d = Dataset::new(1, Source::text(&text, "blocks").unwrap());
    d.start(|_| {});
    wait(|| d.status(0)["complete"] == true);
    let count = |id, f: Filters| {
        let q = d.query(0, id, Filter::new(f).unwrap()).unwrap();
        wait(|| q.json()["status"] == "complete");
        q.json()["count"].as_u64().unwrap()
    };
    assert_eq!(count(1, show(pid("^(?:300|999)$"))), 4001);
    assert_eq!(count(2, filters("", "", "301", "")), 3);
    assert_eq!(count(3, filters("", "big", "", "")), 3);
    d.close();
}
