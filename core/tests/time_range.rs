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

const ANDROID: &str = "06-18 23:59:59.900  1 2 I A: a
06-19 00:00:00.000  1 2 I A: b
06-19 00:10:00.500  1 2 I A: c
06-19 12:30:15.123456  1 2 W A: d
raw continuation
06-20 08:00:00.000  1 2 E A: e
";
const IOS: &str = "2025-12-31 23:59:59.000 Runner[1:2] [A] x
2026-01-01 00:00:01.000 Runner[1:2] [A] y
";

fn wait(mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !done() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
}
fn counts(text: &str, cases: &[(&str, &str, u64)]) {
    let d: Arc<Dataset> = Dataset::new(1, Source::text(text, "time").unwrap());
    d.start(|_| {});
    wait(|| d.status(0)["complete"] == true);
    for (id, (from, to, expected)) in cases.iter().enumerate() {
        let filter = Filter::new(Filters {
            time_from: (*from).into(),
            time_to: (*to).into(),
            ..Filters::default()
        })
        .unwrap();
        assert!(!filter.identity());
        let q = d.query(0, id as u64 + 1, filter).unwrap();
        wait(|| q.json()["status"] == "complete");
        assert_eq!(q.json()["count"], *expected, "from {from:?} to {to:?}");
    }
    d.close();
}

#[test]
fn android_bounds_follow_the_given_precision_inclusively() {
    counts(
        ANDROID,
        &[
            // The raw row continues d and takes its time.
            ("06-19", "", 5),
            ("", "06-19", 5),
            ("06-19", "06-19", 4),
            (" 06-19 12:30:15.123 ", "06-19 12:30:15.123", 2),
            ("12:30:15.1235", "", 1),
            ("06-19 00", "06-19 00", 2),
            ("06-19 00:10", "06-19 00:10:00", 1),
            ("", "00:10", 2),
            // Clock-only bounds that cross midnight wrap.
            ("23:00", "00:05", 2),
            // Android rows have no year, so a year in the bound is not compared.
            ("2026-06-19", "", 5),
            ("06-21", "", 0),
        ],
    );
}

#[test]
fn a_clock_only_bound_takes_the_other_bound_date_and_runs_forward() {
    counts(
        ANDROID,
        &[
            // The end takes the start's date: rows on 06-20 before 08:30 stay out.
            ("06-19 00:00", "08:30", 2),
            // An end earlier than the start falls on the next day.
            ("06-19 12:00", "08:00", 3),
            // A start later than the end falls on the previous day.
            ("23:00", "06-19 00:05", 2),
            ("06-19 00:10", "", 4),
        ],
    );
    const YEAR_END: &str = "12-30 23:45:00.000  1 2 I A: before
12-31 23:30:00.000  1 2 I A: a
01-01 00:10:00.000  1 2 I A: b
01-01 09:00:00.000  1 2 I A: after
";
    counts(
        YEAR_END,
        &[
            // Without a year the next day after 12-31 is 01-01.
            ("12-31 23:00", "00:30", 2),
            ("23:00", "01-01 00:30", 2),
            // A year on the bound is not compared with yearless rows.
            ("2026-12-31 23:00", "00:30", 2),
        ],
    );
    counts(
        IOS,
        &[
            ("2025-12-31 23:00", "00:30", 2),
            ("12-31 23:59:59.5", "00:00:00.5", 0),
            ("23:00", "2026-01-01 00:00:00", 1),
        ],
    );
}

#[test]
fn ios_years_are_compared_only_when_both_sides_have_one() {
    counts(
        IOS,
        &[
            ("2026-01-01", "", 1),
            ("", "2025-12-31", 1),
            ("2025-12-31 23:59:59.000", "2026-01-01 00:00:01", 2),
            ("01-01", "", 2),
        ],
    );
}

#[test]
fn malformed_bounds_are_rejected() {
    for bad in [
        "6-19",
        "25:00",
        "06-19 12:3",
        "12:30:15.",
        "06-19 12 x",
        "13-01",
        "06-32",
        "12:60",
        "1:00",
        "12:30.5",
        "12:30:15.1234567890",
    ] {
        let error = Filter::new(Filters {
            time_from: bad.into(),
            ..Filters::default()
        })
        .err();
        assert_eq!(
            error.map(|e| e.to_string()).as_deref(),
            Some("Invalid time filter"),
            "{bad:?}"
        );
    }
    let filter = Filter::new(Filters {
        time_from: "  ".into(),
        time_to: "".into(),
        ..Filters::default()
    })
    .unwrap();
    assert!(filter.identity(), "blank bounds are no filter");
}

#[test]
fn dated_android_rows_compare_their_year() {
    const DATED: &str = "2025-12-31 23:59:59.000  1 2 I A: old
2026-01-01 00:00:01.000123456  1 2 I A: new
";
    counts(
        DATED,
        &[
            ("2026-01-01", "", 1),
            ("", "2025-12-31", 1),
            // A bound without a year still compares month and day only.
            ("12-31", "", 1),
            ("00:00:01.000123456", "00:00:01.000123456", 1),
        ],
    );
}

/// The page warns about a dated range only when it can match nothing
/// (`isEmptyTimeRange`); these are the cases it relies on.
#[test]
fn inverted_dated_ranges_match_nothing_unless_years_are_mixed() {
    const YEAR_END: &str = "12-31 23:30:00.000  1 2 I A: a
01-01 00:10:00.000  1 2 I A: b
";
    counts(YEAR_END, &[("12-31 23:00", "01-01 01:00", 0)]);
    counts(
        IOS,
        &[
            ("2026-01-01", "2025-12-31", 0),
            // One year: dated rows compare it on that side only.
            ("2025-12-31 23:00", "01-01 01:00", 1),
        ],
    );
}
