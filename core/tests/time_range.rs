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

const YEAR_END: &str = "12-30 23:45:00.000  1 2 I A: before
12-31 23:30:00.000  1 2 I A: a
01-01 00:10:00.000  1 2 I A: b
raw continuation
01-02 09:00:00.000  1 2 I A: after
";

#[test]
fn a_dated_range_without_years_runs_past_the_year_end() {
    counts(
        YEAR_END,
        &[
            ("12-31 23:00", "01-01 01:00", 3),
            ("12-31", "01-01", 3),
            ("12-31 23:30", "01-01 00:09", 1),
            ("12-30", "01-02 08:59", 4),
            // Two ends in order stay within a year.
            ("01-01", "12-31", 5),
            ("01-01", "01-02", 3),
            // Half a year or less apart the ends are the wrong way round.
            ("01-02", "01-01", 0),
            ("12-31 23:30", "12-30", 0),
        ],
    );
    // More than 183 days of a 366-day year, to the nanosecond.
    const JULY: &str = "07-02 12:00:00.000  1 2 I A: a
01-01 11:00:00.000  1 2 I A: b
";
    counts(
        JULY,
        &[
            ("07-02 12:00", "01-01 11:59", 2),
            ("07-02 12:00", "01-01 12:00", 0),
            ("07-02 11:59:59.999999999", "01-01 11:59:59", 0),
            ("07-02 12:00", "01-01 11:59:59.999999999", 2),
            ("07-03", "01-01", 1),
            ("07-02", "01-01", 0),
        ],
    );
}

/// The ranges the page's row menu makes of one end (`narrowedTime`).
#[test]
fn ranges_completed_at_a_year_end_keep_their_rows() {
    // Dates keep rows a moment out of order at the log's ends.
    counts(
        "12-30 08:00:00.000  1 2 I A: first
12-30 07:59:59.998  1 2 I A: before
12-31 23:59:59.000  1 2 I A: x
01-01 00:00:01.000  1 2 I A: y
01-02 09:00:00.004  1 2 I A: after
01-02 09:00:00.000  1 2 I A: last
",
        &[
            ("12-30", "12-31 23:59:59.000", 3),
            ("12-31 23:59:59.000", "01-02", 4),
        ],
    );
    // Ends 183 days apart cross the year end only with their times.
    counts(
        "07-02 10:00:00.000  1 2 I A: a
07-02 11:00:00.000  1 2 I A: b
01-01 08:30:00.000  1 2 I A: c
01-01 09:00:00.000  1 2 I A: d
",
        &[
            ("07-02", "01-01 08:30:00.000", 0),
            ("07-02 10:00:00.000", "01-01 08:30:00.000", 3),
            ("07-02 11:00:00.000", "01-01", 0),
            ("07-02 11:00:00.000", "01-01 09:00:00.000", 3),
            ("07-02 10:00:00.000", "01-01 09:00:00.000", 4),
        ],
    );
}

#[test]
fn rows_with_a_year_take_a_yearless_range_every_year() {
    const YEARS: &str = "2024-12-31 23:59:59.000  1 2 I A: a
2025-01-01 00:00:01.000  1 2 I A: b
2025-06-18 00:00:00.000  1 2 I A: c
2025-12-31 23:59:59.000  1 2 I A: d
2026-01-01 00:00:01.000  1 2 I A: e
";
    counts(
        YEARS,
        &[
            ("12-31", "01-01", 4),
            // A year on one end gives the other its own.
            ("2025-12-31", "01-01", 2),
            ("12-31", "2025-01-01", 2),
            // Years on both are compared as given.
            ("2025-12-31", "2026-01-01", 2),
            ("2024-12-31", "2026-01-01", 5),
            ("2026-01-01", "2025-12-31", 0),
        ],
    );
    counts(
        IOS,
        &[
            ("12-31 23:59:59", "01-01 00:00:01", 2),
            ("2025-12-31 23:00", "01-01 01:00", 2),
        ],
    );
}

#[test]
fn rows_without_a_year_take_a_range_of_two_years() {
    counts(
        YEAR_END,
        &[
            ("2025-12-31 23:00", "2026-01-01 01:00", 3),
            ("2025-12-31 23:00", "01-01 01:00", 3),
            ("12-31 23:00", "2026-01-01 01:00", 3),
            ("2025-12-30", "2025-12-31", 2),
            // Any span of years in order, however far apart the dates.
            ("2025-01-02", "2026-01-01", 5),
            ("2026-01-01", "2025-12-31", 0),
        ],
    );
}

#[test]
fn a_range_of_a_whole_year_takes_every_row_without_one() {
    const ALL: &str = "01-01 00:00:00.000  1 2 I A: a
02-28 23:59:59.999  1 2 I A: b
02-29 12:00:00.000  1 2 I A: c
03-01 00:00:00.000  1 2 I A: d
06-18 11:59:30.000  1 2 I A: e
06-18 12:00:00.000  1 2 I A: f
12-31 23:59:59.999  1 2 I A: g
";
    counts(
        ALL,
        &[
            // The year between two year ends.
            ("2024-12-31", "2026-01-01", 7),
            ("2024-12-31", "2025-12-31", 7),
            // Exactly a year, and a moment less.
            ("2025-01-01", "2025-12-31", 7),
            ("2025-06-18 12:00", "2026-06-18 12:00", 7),
            ("2025-06-18 12:00", "2026-06-18 11:59", 7),
            ("2025-06-18 12:00", "2026-06-18 11:58", 6),
            ("2025-06-18 11:59:30", "2026-06-18 11:59:29.999999999", 7),
            (
                "2025-06-18 11:59:30.000000001",
                "2026-06-18 11:59:29.999999999",
                6,
            ),
            // Years whose end has no February 29, and one that has.
            ("2025-03-01", "2026-02-28", 7),
            ("2025-03-01", "2026-02-27", 5),
            ("2023-03-01", "2024-02-29", 7),
            ("2023-03-01", "2024-02-28", 6),
            ("2024-02-29", "2025-02-28", 7),
            ("2024-02-29", "2025-02-27", 6),
            ("2024-03-01", "2025-03-01", 7),
            // Longer, whichever way round the dates are.
            ("2024-06-18", "2026-03-01", 7),
            ("2024-03-01", "2026-06-18", 7),
            ("2020-02-29", "2024-02-29", 7),
            ("2000-01-01", "9999-12-31", 7),
            // Within a year, and the wrong way round.
            ("2025-03-01", "2025-06-18", 3),
            ("2026-01-01", "2024-12-31", 0),
        ],
    );
}

/// The page warns about a dated range only when it can match nothing
/// (`isEmptyTimeRange`); these are the cases it relies on.
#[test]
fn inverted_dated_ranges_match_nothing_within_half_a_year() {
    counts(YEAR_END, &[("12-31 23:30", "12-31 23:00", 0)]);
    counts(
        IOS,
        &[
            ("2026-01-01", "2025-12-31", 0),
            ("01-01 00:00:02", "01-01 00:00:01", 0),
        ],
    );
}

#[test]
fn the_status_has_the_first_and_last_row_times() {
    let span = |d: &Dataset| d.status(0)["span"].clone();
    let open = |text: &str| {
        let d = Dataset::new(1, Source::text(text, "span").unwrap());
        d.start(|_| {});
        wait(|| d.status(0)["textComplete"] == true);
        d
    };
    let d = open(&format!("--------- beginning of main\n{YEAR_END}tail\n"));
    assert_eq!(
        span(&d),
        serde_json::json!(["12-30 23:45:00.000", "01-02 09:00:00.000"])
    );
    d.close();
    // Ends without a row time are looked through, up to 64 KiB of them.
    let raw = "no time here\n".repeat(4000);
    let d = open(&format!("{raw}{YEAR_END}{raw}"));
    assert_eq!(
        span(&d),
        serde_json::json!(["12-30 23:45:00.000", "01-02 09:00:00.000"])
    );
    d.close();
    for text in ["no time\n", "", &"no time here\n".repeat(20_000)] {
        let d = open(text);
        assert!(span(&d).is_null());
        d.close();
    }
}

#[test]
fn the_row_times_follow_a_growing_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("grow.log");
    std::fs::write(&path, "").unwrap();
    let d = Dataset::new(1, Source::granted(&path).unwrap());
    d.start(|_| {});
    wait(|| d.status(0)["textComplete"] == true);
    assert!(d.status(0)["span"].is_null());
    d.follow_poll_ms
        .store(2, std::sync::atomic::Ordering::Relaxed);
    d.follow(true).unwrap();
    let append = |text: &str| {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        f.write_all(text.as_bytes()).unwrap();
    };
    append("12-31 23:59:59.000  1 2 I A: a\n");
    wait(|| d.status(0)["span"] == serde_json::json!(["12-31 23:59:59.000", "12-31 23:59:59.000"]));
    append("01-01 00:00:01.000  1 2 I A: b\n");
    wait(|| d.status(0)["span"] == serde_json::json!(["12-31 23:59:59.000", "01-01 00:00:01.000"]));
    d.close();
}
