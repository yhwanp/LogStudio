//! Filters with several conditions: a row shows when it matches one showing
//! condition (or there is none) and no hiding condition. Each condition must
//! decide a row as the same condition does alone in a filter's own fields.
use logstudio_core::{
    parser,
    search::{Condition, Filter, Filters, MAX_CONDITIONS},
};
use serde_json::json;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
}

/// Android, Android Studio and iOS rows (with several and no tags), rows
/// without fields and malformed stamped rows.
fn rows() -> Vec<String> {
    let tags = [
        "ActivityManager",
        "WifiService",
        "wifiscanner",
        "네트워크",
        "AlertsService",
    ];
    let mut out = vec![];
    for i in 0..400 {
        let level = ["V", "D", "I", "W", "E", "F", "A"][i % 7];
        let (pid, tid) = (["101", "102", "4242"][i % 3], ["7", "8", "9000"][i % 5 % 3]);
        let tag = tags[i * 7 % tags.len()];
        let clock = format!("12:07:{:02}.{:03}", i % 60, i % 1000);
        out.push(match i % 6 {
            0 | 1 => format!("06-18 {clock}  {pid}  {tid} {level} {tag}: message {i}"),
            2 => {
                format!("2026-06-18 {clock}  {pid}-{tid}  {tag}  com.example  {level}  studio {i}")
            }
            3 => format!("2026-06-18 {clock} Runner[{pid}:{tid}] [{tag}] [Comp] ios {i}"),
            4 => format!("2026-06-18 {clock} Runner[{pid}:{tid}] untagged {i}"),
            _ if i % 4 == 0 => format!("06-18 {clock} not a log row {i}"),
            _ => format!("  at continuation {i}"),
        });
    }
    out
}

const NEEDLES: [&str; 8] = [
    "activity",
    "wifi",
    "네트워크",
    "Alerts",
    "comp",
    "zzz",
    "a",
    "",
];
fn condition(r: &mut Rng) -> Condition {
    let mut c = Condition::default();
    if r.chance(50) {
        c.levels.retain(|_| r.chance(55));
    }
    if r.chance(60) {
        c.tag = (0..1 + r.below(2))
            .map(|_| NEEDLES[r.below(NEEDLES.len())])
            .collect::<Vec<_>>()
            .join(", ");
    }
    if r.chance(25) {
        c.pid = ["101", "4242", "102 101"][r.below(3)].into();
    }
    if r.chance(20) {
        c.tid = ["7", "9000"][r.below(2)].into();
    }
    if r.chance(20) {
        c.time_from = ["12:07:10", "06-18 12:07:30", "12:07"][r.below(3)].into();
    }
    if r.chance(20) {
        c.time_to = ["12:07:40", "2026-06-18 12:07:50.5"][r.below(2)].into();
    }
    c
}
/// The condition as a filter's own fields.
fn alone(c: &Condition) -> Filter {
    Filter::new(Filters {
        levels: c.levels.clone(),
        tag: c.tag.clone(),
        pid: c.pid.clone(),
        tid: c.tid.clone(),
        time_from: c.time_from.clone(),
        time_to: c.time_to.clone(),
        ..Filters::default()
    })
    .unwrap()
}

#[test]
fn conditions_combine_as_the_same_conditions_alone() {
    let rows = rows();
    let mut r = Rng(0x5eed_c0de);
    let (mut shown, mut hidden) = (0, 0);
    for _ in 0..600 {
        let filters = Filters {
            include: (0..r.below(4)).map(|_| condition(&mut r)).collect(),
            exclude: (0..r.below(3)).map(|_| condition(&mut r)).collect(),
            include_raw: r.chance(70),
            ..Filters::default()
        };
        let filter = Filter::new(filters.clone()).unwrap();
        // Conditions without criteria are skipped, also as single filters.
        let shows: Vec<Filter> = filters
            .include
            .iter()
            .map(alone)
            .filter(|f| !f.identity())
            .collect();
        let hides: Vec<Filter> = filters
            .exclude
            .iter()
            .map(alone)
            .filter(|f| !f.identity())
            .collect();
        assert_eq!(
            (filter.shows().len(), filter.hides().len()),
            (shows.len(), hides.len())
        );
        for (line, raw) in rows.iter().enumerate() {
            let record = parser::parse_line(raw, line as u64 + 1, true);
            let expected = if record.parsed.is_some() {
                (shows.is_empty() || shows.iter().any(|f| f.fields_match(&record)))
                    && !hides.iter().any(|f| f.fields_match(&record))
            } else {
                // Rows without fields match no condition.
                filters.include_raw && shows.is_empty()
            };
            assert_eq!(
                filter.fields_match(&record),
                expected,
                "{raw:?} with {filters:?}"
            );
            if expected {
                shown += 1;
            } else {
                hidden += 1;
            }
        }
        assert_eq!(
            filter.admits_unparsed(),
            filters.include_raw && shows.is_empty()
        );
        // Level slots bound the rows that can show.
        let slots = filter.level_slots();
        for raw in &rows {
            let record = parser::parse_line(raw, 1, true);
            if let (Some(p), true) = (&record.parsed, filter.fields_match(&record)) {
                let slot = parser::LEVELS.iter().position(|l| *l == p.level).unwrap();
                assert!(slots & (1 << slot) != 0, "{raw:?} with {filters:?}");
            }
        }
    }
    assert!(shown > 20_000 && hidden > 20_000, "{shown} {hidden}");
}

#[test]
fn conditions_are_skipped_normalized_bounded_and_compared() {
    let f = |json: serde_json::Value| -> Filters { serde_json::from_value(json).unwrap() };
    // Conditions without criteria change nothing.
    let blank = Filter::new(f(
        json!({"include":[{}, {"tag":" , "}],"exclude":[{}, {"pid":" "}]}),
    ))
    .unwrap();
    assert!(blank.identity() && blank.only_levels() && blank.admits_unparsed());
    // A showing condition keeps out rows without fields; a hiding one does not.
    let shows = Filter::new(f(json!({"include":[{"levels":["E"]}]}))).unwrap();
    let hides = Filter::new(f(json!({"exclude":[{"levels":["V"]}]}))).unwrap();
    assert!(!shows.admits_unparsed() && hides.admits_unparsed());
    assert!(shows.only_levels() && hides.only_levels());
    assert_eq!(shows.level_slots(), 1 << 4);
    assert_eq!(
        hides.level_slots(),
        0b1_1111_1110,
        "every level but V, and orphans"
    );
    // Hiding conditions that test more than levels leave their slots in.
    let mixed = Filter::new(f(json!({"exclude":[{"levels":["V"],"tag":"x"}]}))).unwrap();
    assert_eq!(mixed.level_slots(), 0b1_1111_1111);
    assert!(!mixed.only_levels() && mixed.tested_fields() == [false, false, true]);
    // Showing levels add up.
    let either = Filter::new(f(
        json!({"levels":["W"],"include":[{"levels":["E"],"tag":"a"}]}),
    ))
    .unwrap();
    assert_eq!(either.level_slots(), 1 << 3 | 1 << 4);
    // Conditions take their normal form.
    let normal = Filter::new(f(json!({"include":[{"tag":" b ,a,b","pid":"2 1,2","levels":["W","E","W"],"timeFrom":" 12:00 "}]})))
        .unwrap();
    assert_eq!(
        normal.fields.include[0],
        Condition {
            levels: vec!["E".into(), "W".into()],
            tag: "b,a".into(),
            pid: "2,1".into(),
            time_from: "12:00".into(),
            ..Condition::default()
        }
    );
    // The shorthand fields and the same listed condition are the same fields.
    let own = Filter::new(f(json!({"tag":"a","excludeTags":"b"}))).unwrap();
    let listed = Filter::new(f(json!({"include":[{"tag":"a"}],"exclude":[{"tag":"b"}]}))).unwrap();
    let other = Filter::new(f(json!({"include":[{"tag":"a"}],"exclude":[{"tag":"c"}]}))).unwrap();
    assert!(own.same_fields(&listed) && !own.same_fields(&other));
    // Bounds and validation apply to every condition.
    let many = |n: usize| json!({"include": vec![json!({"tag":"a"}); n]});
    assert!(Filter::new(f(many(MAX_CONDITIONS))).is_ok());
    assert!(Filter::new(f(many(MAX_CONDITIONS + 1))).is_err());
    for bad in [
        json!({"include":[{"pid":"12a"}]}),
        json!({"exclude":[{"tid":"x"}]}),
        json!({"include":[{"levels":["Q"]}]}),
        json!({"exclude":[{"timeFrom":"25:00"}]}),
        json!({"include":[{"tag":(0..33).map(|i| format!("t{i}")).collect::<Vec<_>>().join(",")}]}),
        json!({"include":[{"tag":"x".repeat(4001)}]}),
    ] {
        assert!(Filter::new(f(bad.clone())).is_err(), "{bad}");
    }
    assert!(serde_json::from_value::<Filters>(json!({"include":[{"payload":"x"}]})).is_err());
    // A hiding condition keeps a tag item with a line break: it matches no
    // tag, so the condition hides nothing rather than every level.
    let broken = Filter::new(Filters {
        exclude: vec![Condition {
            tag: "a\nb".into(),
            ..Condition::default()
        }],
        ..Filters::default()
    })
    .unwrap();
    assert_eq!(broken.hides().len(), 1);
    assert!(broken.fields_match(&parser::parse_line(
        "06-18 12:07:00.000  1  2 I ab: x",
        1,
        true
    )));
}
