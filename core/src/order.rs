//! The order of files picked or dropped together: by the time of each file's
//! last log row, else by rotation-aware natural name. Only the first and last
//! `HEAD` bytes of a file are read, so ordering costs the same for any file
//! size.
use crate::{
    dlt, encoding, parser,
    source::{ByteLines, MAX_FILES},
};
use std::{
    cmp::Ordering,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

/// Bytes read from each end of a file to place it.
pub const HEAD: usize = 64 * 1024;
const DAY: u64 = 86_400_000_000_000;
/// Year-less times further apart may have crossed a New Year.
const HALF_YEAR: u64 = 183 * DAY;
/// Days before each month, counting February 29.
const BEFORE: [u64; 13] = [0, 31, 60, 91, 121, 152, 182, 213, 244, 274, 305, 335, 366];

/// What decided an order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum By {
    Time,
    Name,
}
impl By {
    pub fn as_str(self) -> &'static str {
        match self {
            By::Time => "time",
            By::Name => "name",
        }
    }
}

/// A row time: its year, if the row has one, and nanoseconds into a 366-day
/// year.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stamp {
    year: Option<u16>,
    at: u64,
}
fn stamp(date: &str, time: &str) -> Option<Stamp> {
    // The parser gives `MM-DD` or `YYYY-MM-DD` and `HH:MM:SS.` with 3 to 9
    // fraction digits; any other shape is no time rather than a wrong one.
    let n = |s: Option<&str>| s?.parse::<u64>().ok();
    let (year, md) = match date.len() {
        10 => (Some(date.get(..4)?.parse().ok()?), date.get(5..)?),
        _ => (None, date),
    };
    let (month, day) = (n(md.get(..2))?, n(md.get(3..5))?);
    let (h, m, s) = (n(time.get(..2))?, n(time.get(3..5))?, n(time.get(6..8))?);
    let fraction = time.get(9..).unwrap_or("");
    if !(1..=12).contains(&month)
        || day == 0
        || day > BEFORE[month as usize] - BEFORE[month as usize - 1]
        || h > 23
        || m > 59
        || s > 59
        || fraction.len() > 9
    {
        return None;
    }
    let nanos = match fraction {
        "" => 0,
        f => n(Some(f))? * 10u64.pow(9 - f.len() as u32),
    };
    let seconds = (h * 60 + m) * 60 + s;
    Some(Stamp {
        year,
        at: (BEFORE[month as usize - 1] + day - 1) * DAY + seconds * 1_000_000_000 + nanos,
    })
}

/// What one file's ends say about its place.
struct Place {
    /// The time that places the file among the others.
    stamp: Option<Stamp>,
    /// The whole file was read and holds only blank rows (or nothing).
    blank: bool,
}
fn is_break(b: &u8) -> bool {
    *b == b'\n' || *b == b'\r'
}
/// The time of a log row, if `bytes` is one.
fn row_time(bytes: &[u8]) -> Option<Stamp> {
    let raw = encoding::decode(bytes);
    // Numbered prefixes are recognized, as in the default reading mode.
    let record = parser::parse_line(&raw, 0, true);
    record.parsed.and_then(|p| stamp(p.date, p.time))
}
/// `head` is a file's first `HEAD` bytes, or the whole file when shorter;
/// `tail` is its last `HEAD` bytes when `head` is full.
fn inspect(head: &[u8], tail: Option<&[u8]>) -> Place {
    // A DLT file's time is its first message's storage time. Only DLT files
    // open together, so UTC orders them without a time zone.
    if dlt::is_dlt(head) {
        let stamp = dlt::first_time(head).and_then(|(seconds, micros)| {
            let (date, time) = dlt::utc_text(seconds, micros);
            stamp(&date, &time)
        });
        return Place {
            stamp,
            blank: false,
        };
    }
    let Some(tail) = tail else {
        // The whole file: its last log row places it.
        let lines: Vec<&[u8]> = ByteLines::new(head).map(|(_, b)| b).collect();
        let blank = lines
            .iter()
            .all(|b| parser::trim(&encoding::decode(b)).is_empty());
        let stamp = lines.iter().rev().find_map(|b| row_time(b));
        return Place { stamp, blank };
    };
    // Each end may be cut inside a row; that row is not judged.
    let head = &head[..head.iter().rposition(is_break).map_or(0, |i| i + 1)];
    let tail = &tail[tail.iter().position(is_break).map_or(tail.len(), |i| i + 1)..];
    // Rows logged before a device sets its clock come first and may carry a
    // wrong time, so the last log row places a file; without one in the
    // tail, the first in the head does.
    let lines: Vec<&[u8]> = ByteLines::new(tail).map(|(_, b)| b).collect();
    let stamp = lines
        .iter()
        .rev()
        .find_map(|b| row_time(b))
        .or_else(|| ByteLines::new(head).find_map(|(_, b)| row_time(b)));
    Place {
        stamp,
        blank: false,
    }
}

/// `main.log.2` is (`main.log`, 2): a rotated copy, older as the number
/// grows. One to three digits after a dot that follows a non-digit, so
/// `app.2026.10.17` stays a dated name.
fn rotation(name: &str) -> (&str, Option<u32>) {
    if let Some(dot) = name.rfind('.') {
        let (stem, n) = (&name[..dot], &name[dot + 1..]);
        if (1..=3).contains(&n.len())
            && n.bytes().all(|b| b.is_ascii_digit())
            && stem
                .bytes()
                .last()
                .is_some_and(|b| !b.is_ascii_digit() && b != b'.')
        {
            return (stem, n.parse().ok());
        }
    }
    (name, None)
}
/// Digit runs compare by value (`2` before `10`), other text ignoring case.
fn natural(a: &str, b: &str) -> Ordering {
    let digits = |s: &str| s.bytes().take_while(u8::is_ascii_digit).count();
    let (mut x, mut y) = (a, b);
    loop {
        let (Some(p), Some(q)) = (x.chars().next(), y.chars().next()) else {
            return x.len().cmp(&y.len());
        };
        if p.is_ascii_digit() && q.is_ascii_digit() {
            let (i, j) = (digits(x), digits(y));
            let (u, v) = (
                x[..i].trim_start_matches('0'),
                y[..j].trim_start_matches('0'),
            );
            match u.len().cmp(&v.len()).then_with(|| u.cmp(v)) {
                Ordering::Equal => (x, y) = (&x[i..], &y[j..]),
                o => return o,
            }
        } else {
            match p.to_lowercase().cmp(q.to_lowercase()) {
                Ordering::Equal => (x, y) = (&x[p.len_utf8()..], &y[q.len_utf8()..]),
                o => return o,
            }
        }
    }
}
/// Name order: natural, rotated copies oldest first and before the current
/// file (`main.log.2`, `main.log.1`, `main.log`), then exact text.
fn name_order(a: &str, b: &str) -> Ordering {
    let ((x, i), (y, j)) = (rotation(a), rotation(b));
    natural(x, y).then(j.cmp(&i)).then_with(|| a.cmp(b))
}

/// Orders files by their names and contents, of which only the first and
/// last `HEAD` bytes are read, and returns input indices with the basis.
///
/// A text file's time is that of its last log row in the last `HEAD` bytes,
/// else of its first in the first `HEAD` bytes; a DLT file's is its first
/// message's. By time when every file with rows has a time and at least two
/// times differ: equal times keep name order, and files without rows go last.
/// Years are compared only when every time has one. Year-less times are
/// month-day order, but when they spread over more than 183 days a New Year
/// may lie between them (`12-31` and `01-01`), so name order decides. Any
/// file without a time also leaves the whole order to names. Equal names keep
/// the given order.
pub fn order(files: &[(&str, &[u8])]) -> (Vec<usize>, By) {
    let names: Vec<&str> = files.iter().map(|f| f.0).collect();
    let places: Vec<Place> = files
        .iter()
        .map(|&(_, bytes)| {
            let head = &bytes[..bytes.len().min(HEAD)];
            let tail = (bytes.len() >= HEAD).then(|| &bytes[bytes.len() - HEAD..]);
            inspect(head, tail)
        })
        .collect();
    sort(&names, &places)
}
fn sort(names: &[&str], places: &[Place]) -> (Vec<usize>, By) {
    let named = |mut v: Vec<usize>| {
        v.sort_by(|&a, &b| name_order(names[a], names[b]));
        v
    };
    let (rows, blank): (Vec<usize>, Vec<usize>) = (0..names.len()).partition(|&k| !places[k].blank);
    let stamps: Vec<Stamp> = rows.iter().filter_map(|&k| places[k].stamp).collect();
    let years = stamps.iter().all(|s| s.year.is_some());
    let key = |s: Stamp| (s.year.filter(|_| years), s.at);
    let low = stamps.iter().map(|&s| key(s)).min();
    let high = stamps.iter().map(|&s| key(s)).max();
    let timed = stamps.len() == rows.len()
        && low != high
        && (years || high.zip(low).is_some_and(|(h, l)| h.1 - l.1 <= HALF_YEAR));
    if !timed {
        return (named((0..names.len()).collect()), By::Name);
    }
    // Both sorts are stable: equal times keep name order.
    let mut rows = named(rows);
    rows.sort_by_key(|&k| places[k].stamp.map(key));
    rows.extend(named(blank));
    (rows, By::Time)
}

/// Orders granted paths as `order` does, reading only each file's ends. A file
/// that cannot be read counts as one without rows; opening it reports the
/// error. Returns the basis only when the order changed.
pub fn arrange(paths: Vec<PathBuf>) -> (Vec<PathBuf>, Option<By>) {
    // More files than a source may combine fail to open anyway.
    if paths.len() < 2 || paths.len() > MAX_FILES {
        return (paths, None);
    }
    let names: Vec<String> = paths
        .iter()
        .map(|p| p.file_name().unwrap_or_default().to_string_lossy().into())
        .collect();
    let places: Vec<Place> = paths
        .iter()
        .map(|p| {
            let (head, tail) = ends(p);
            inspect(&head, tail.as_deref())
        })
        .collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let (order, by) = sort(&names, &places);
    if order.iter().enumerate().all(|(i, &k)| i == k) {
        return (paths, None);
    }
    let mut paths: Vec<Option<PathBuf>> = paths.into_iter().map(Some).collect();
    let paths = order.iter().filter_map(|&k| paths[k].take()).collect();
    (paths, Some(by))
}
/// A file's first `HEAD` bytes and, when they are full, its last `HEAD`
/// bytes (empty if they cannot be read).
fn ends(path: &Path) -> (Vec<u8>, Option<Vec<u8>>) {
    let (mut head, mut tail) = (Vec::new(), None);
    // Only a regular file: a FIFO or device could block or never end.
    if std::fs::symlink_metadata(path).is_ok_and(|m| m.is_file()) {
        if let Ok(mut file) = File::open(path) {
            let _ = (&mut file).take(HEAD as u64).read_to_end(&mut head);
            if head.len() == HEAD {
                let mut last = Vec::new();
                if file.seek(SeekFrom::End(-(HEAD as i64))).is_ok() {
                    let _ = file.take(HEAD as u64).read_to_end(&mut last);
                }
                tail = Some(last);
            }
        }
    }
    (head, tail)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn names<'a>(files: &[(&'a str, &str)]) -> (Vec<&'a str>, By) {
        let input: Vec<(&str, &[u8])> = files.iter().map(|f| (f.0, f.1.as_bytes())).collect();
        let (order, by) = order(&input);
        (order.iter().map(|&k| files[k].0).collect(), by)
    }
    fn android(time: &str) -> String {
        format!("{time}  1234  5678 I Tag: body\n")
    }
    #[test]
    fn rotated_names_put_higher_numbers_first() {
        let raw = "no log rows here\n";
        let (order, by) = names(&[
            ("main.log", raw),
            ("main.log.1", raw),
            ("main.log.10", raw),
            ("main.log.2", raw),
            ("events.log", raw),
        ]);
        assert_eq!(
            order,
            [
                "events.log",
                "main.log.10",
                "main.log.2",
                "main.log.1",
                "main.log"
            ]
        );
        assert_eq!(by, By::Name);
    }
    #[test]
    fn natural_names_compare_numbers_by_value() {
        let mut v = vec![
            "logcat_10.txt",
            "logcat_2.txt",
            "Logcat_1.txt",
            "logcat_02.txt",
            "app.2026.10.17",
            "app.2026.10.9",
            "2026-10-09.log",
            "2026-10-10.log",
            "a.1",
            "a.0",
            "a",
        ];
        v.sort_by(|a, b| name_order(a, b));
        assert_eq!(
            v,
            [
                "2026-10-09.log",
                "2026-10-10.log",
                "a.1",
                "a.0",
                "a",
                "app.2026.10.9",
                "app.2026.10.17",
                "Logcat_1.txt",
                "logcat_02.txt",
                "logcat_2.txt",
                "logcat_10.txt",
            ]
        );
        assert_eq!(rotation("trace.2026"), ("trace.2026", None));
        assert_eq!(rotation("syslog.7"), ("syslog", Some(7)));
        assert_eq!(rotation(".1"), (".1", None));
    }
    #[test]
    fn android_times_order_files() {
        let (order, by) = names(&[
            ("a.log", &android("06-18 19:24:05.000")),
            ("b.log", &android("06-18 19:24:01.000")),
            ("c.log", &android("06-17 23:59:59.999")),
        ]);
        assert_eq!((order, by), (vec!["c.log", "b.log", "a.log"], By::Time));
        // Fractions of 3, 6 and 9 digits compare by value.
        let (order, _) = names(&[
            ("ms", &android("06-18 19:24:00.123")),
            ("us", &android("06-18 19:24:00.122999")),
            ("ns", &android("06-18 19:24:00.122999001")),
        ]);
        assert_eq!(order, ["us", "ns", "ms"]);
    }
    #[test]
    fn dated_android_and_ios_compare_years() {
        let ios = "2026-01-01 00:00:00.000001 Runner[1:2] [INFO] [Tag] body\n";
        let (order, by) = names(&[
            ("ios.log", ios),
            ("year.log", &android("2025-12-31 23:59:59.000")),
            ("old.log", &android("2025-06-01 00:00:00.000")),
        ]);
        assert_eq!(
            (order, by),
            (vec!["old.log", "year.log", "ios.log"], By::Time)
        );
    }
    #[test]
    fn year_less_times_across_a_new_year_fall_back_to_names() {
        let (order, by) = names(&[
            ("main.log", &android("01-01 00:00:01.000")),
            ("main.log.1", &android("12-31 23:59:58.000")),
        ]);
        assert_eq!((order, by), (vec!["main.log.1", "main.log"], By::Name));
        // Within half a year, month-day order decides.
        let (order, by) = names(&[
            ("a", &android("07-01 00:00:00.000")),
            ("b", &android("01-02 00:00:00.000")),
        ]);
        assert_eq!((order, by), (vec!["b", "a"], By::Time));
        // A mix of dated and year-less rows compares without years.
        let (order, by) = names(&[
            (
                "a-ios",
                "2027-01-05 10:00:00.000 Runner[1:2] [INFO] [Tag] body\n",
            ),
            ("b-android", &android("01-04 10:00:00.000")),
        ]);
        assert_eq!((order, by), (vec!["b-android", "a-ios"], By::Time));
    }
    #[test]
    fn a_file_without_times_leaves_the_order_to_names() {
        let (order, by) = names(&[
            ("b.log", &android("06-18 19:24:00.000")),
            ("c.log", "brief format I/Tag( 1): body\n"),
            ("a.log", &android("06-18 19:25:00.000")),
        ]);
        assert_eq!((order, by), (vec!["a.log", "b.log", "c.log"], By::Name));
        // Equal times everywhere give no order either.
        let row = android("06-18 19:24:00.000");
        assert_eq!(names(&[("b", &row), ("a", &row)]).1, By::Name);
    }
    #[test]
    fn equal_times_keep_name_order_and_blank_files_go_last() {
        let row = android("06-18 19:24:00.000");
        let (order, by) = names(&[
            ("empty.log", ""),
            ("b.log", &row),
            ("blank.log", " \r\n\t\n"),
            ("a.log", &row),
            ("c.log", &android("06-18 19:23:00.000")),
        ]);
        assert_eq!(order, ["c.log", "a.log", "b.log", "blank.log", "empty.log"]);
        assert_eq!(by, By::Time);
        // Equal names keep the given order.
        let early = android("06-18 19:00:00.000");
        let input: Vec<(&str, &[u8])> = vec![
            ("x.log", row.as_bytes()),
            ("x.log", early.as_bytes()),
            ("x.log", row.as_bytes()),
        ];
        assert_eq!(super::order(&input).0, [1, 0, 2]);
    }
    #[test]
    fn the_last_parsed_row_counts_with_numbered_prefixes() {
        let numbered = format!(
            "{}000017 {}",
            android("06-18 19:24:00.000"),
            android("06-18 19:24:09.000")
        );
        let invalid = format!(
            "{}13-45 00:00:00.000  1 2 I Bad: date\n",
            android("06-18 19:24:10.000")
        );
        let damaged = [
            android("06-18 19:24:08.000").as_bytes(),
            b"\xff\xfe junk\n".as_slice(),
        ]
        .concat();
        let input: Vec<(&str, &[u8])> = vec![
            ("numbered", numbered.as_bytes()),
            ("invalid", invalid.as_bytes()),
            ("damaged", &damaged),
        ];
        assert_eq!(order(&input), (vec![2, 0, 1], By::Time));
    }
    #[test]
    fn rows_from_before_a_reboot_set_the_clock_do_not_place_a_file() {
        // Each boot starts with rows from before the clock was set.
        let boot = |early: &str, set: &str| android(early) + &android(set);
        let (order, by) = names(&[
            ("a.log", &android("03-11 08:30:00.000")),
            ("b.log", &boot("01-01 09:00:02.000", "03-11 07:30:20.000")),
            ("c.log", &android("03-10 09:00:00.000")),
            ("d.log", &boot("01-01 09:00:05.000", "03-10 08:00:20.000")),
        ]);
        assert_eq!(
            (order, by),
            (vec!["d.log", "c.log", "b.log", "a.log"], By::Time)
        );
        // Dated rows compare years, so a clock left at 1970 is no New Year.
        let ios = |time: &str| format!("{time} Runner[1:2] [INFO] [Tag] body\n");
        let (order, by) = names(&[
            (
                "a.log",
                &(ios("1970-01-01 09:00:02.000") + &ios("2026-09-21 07:30:20.000")),
            ),
            ("b.log", &ios("2026-09-20 09:00:00.000")),
        ]);
        assert_eq!((order, by), (vec!["b.log", "a.log"], By::Time));
    }
    #[test]
    fn a_long_file_is_placed_by_its_tail_else_by_its_head() {
        let middle = ("x".repeat(99) + "\n").repeat(2 * HEAD / 100);
        let (order, by) = names(&[
            // Spans 10:00 to 12:00, so it follows the 11:00 file.
            (
                "a.log",
                &format!(
                    "{}{middle}{}",
                    android("06-18 10:00:00.000"),
                    android("06-18 12:00:00.000")
                ),
            ),
            ("b.log", &android("06-18 11:00:00.000")),
            // No log row in the tail: the first in the head places it.
            (
                "c.log",
                &format!("{}{middle}", android("06-18 09:00:00.000")),
            ),
        ]);
        assert_eq!((order, by), (vec!["c.log", "b.log", "a.log"], By::Time));
        // Rows between the ends are not read.
        let (order, by) = names(&[
            ("b.log", &android("06-18 09:00:00.000")),
            (
                "a.log",
                &format!("{middle}{}{middle}", android("06-18 10:00:00.000")),
            ),
        ]);
        assert_eq!((order, by), (vec!["a.log", "b.log"], By::Name));
    }
    #[test]
    fn cut_rows_at_either_end_are_not_judged() {
        let filler = "x".repeat(99) + "\n";
        let row = android("06-18 19:24:00.000");
        // The head ends inside a row; the tail starts inside one (say after a
        // line number) that would parse on its own.
        let mut head = filler.repeat(HEAD / 100);
        head.push_str(&row);
        head.truncate(HEAD);
        let tail = row.clone() + &filler.repeat(10);
        let p = inspect(head.as_bytes(), Some(tail.as_bytes()));
        assert!(p.stamp.is_none() && !p.blank);
        // A tail that starts at a line end judges every row after it.
        for start in ["\n", "\r\n", "\r"] {
            let tail = format!("{start}{row}");
            let p = inspect(head.as_bytes(), Some(tail.as_bytes()));
            assert!(p.stamp.is_some(), "{start:?}");
        }
        // A whole file's last row counts without a line end.
        let mut whole = filler.repeat(10);
        whole.push_str(row.trim_end());
        assert!(inspect(whole.as_bytes(), None).stamp.is_some());
    }
    #[test]
    fn invalid_dates_and_times_are_no_times() {
        for (date, time) in [
            ("00-10", "10:00:00.000"),
            ("02-30", "10:00:00.000"),
            ("06-18", "24:00:00.000"),
            ("06-18", "10:60:00.000"),
            ("06-18", "10:00:00.0000000001"),
            ("6-18", "10:00:00.000"),
            ("06-18", "10:00"),
        ] {
            assert!(stamp(date, time).is_none(), "{date} {time}");
        }
        assert!(stamp("02-29", "23:59:59.999999999").is_some());
        // A time without a fraction is the start of that second.
        assert_eq!(stamp("06-18", "10:00:00"), stamp("06-18", "10:00:00.000"));
        assert!(stamp("12-31", "23:59:59.999").unwrap().at < 366 * DAY);
    }
}
