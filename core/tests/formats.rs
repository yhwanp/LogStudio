//! Formats tried after `threadtime` and iOS: `-v time` (with the year,
//! fraction and `uid` options), `threadtime` with `uid`, Android Studio's
//! Logcat window, `brief`, the zone, epoch and monotonic stamps, `-v long`
//! headers and iPhone syslog rows, and the color codes of `-v color` around
//! any of them. Rows the earlier formats parse keep their fields; see also
//! `parser.rs` and the golden fixtures.
use logstudio_core::{
    dataset::{Dataset, Query},
    encoding,
    parser::{self, parse_line},
    search::{Filter, Filters},
    source::{ByteLines, Source},
};
use serde_json::{json, Value};
use std::{
    sync::{atomic::Ordering, Arc},
    thread,
    time::{Duration, Instant},
};

/// The fields of a parsed row, as `[date, time, pid, tid, level, tag,
/// message, process]`; None for a row without fields.
fn fields(raw: &str, recognize: bool) -> Option<[String; 8]> {
    let v = parse_line(raw, 1, recognize).json(usize::MAX);
    (v["kind"] == "parsed").then(|| {
        [
            "date", "time", "pid", "tid", "level", "tag", "message", "process",
        ]
        .map(|k| v[k].as_str().unwrap_or("-").to_owned())
    })
}
fn expect(raw: &str, want: [&str; 8]) {
    for recognize in [false, true] {
        assert_eq!(
            fields(raw, recognize),
            Some(want.map(String::from)),
            "{raw:?}"
        );
    }
}

#[test]
fn time_format_rows() {
    let d = "Sending update";
    for (raw, want) in [
        (
            "06-18 19:24:01.123 D/AlertsService( 2983): Sending update",
            [
                "06-18",
                "19:24:01.123",
                "2983",
                "",
                "D",
                "AlertsService",
                d,
                "-",
            ],
        ),
        // `%-8s` pads short tags; `-v year`, `usec` and `nsec` change the stamp.
        (
            "2026-06-18 19:24:01.123456 W/Tag     (  417): x: y",
            [
                "2026-06-18",
                "19:24:01.123456",
                "417",
                "",
                "W",
                "Tag",
                "x: y",
                "-",
            ],
        ),
        (
            "06-18 19:24:01.123456789 A/DEBUG   (12345): ",
            [
                "06-18",
                "19:24:01.123456789",
                "12345",
                "",
                "A",
                "DEBUG",
                "",
                "-",
            ],
        ),
        // `-v uid` puts a UID or a short user name before the PID.
        (
            "06-18 19:24:01.123 E/Foo(bar)(10123: 2983): boom",
            [
                "06-18",
                "19:24:01.123",
                "2983",
                "",
                "E",
                "Foo(bar)",
                "boom",
                "-",
            ],
        ),
        (
            "06-18 19:24:01.123 I/init    ( root:    1): start",
            ["06-18", "19:24:01.123", "1", "", "I", "init", "start", "-"],
        ),
        (
            "06-18 19:24:01.123 V/chromium(u0_a57  4567): page",
            [
                "06-18",
                "19:24:01.123",
                "4567",
                "",
                "V",
                "chromium",
                "page",
                "-",
            ],
        ),
        // Unicode tags and messages, whitespace of the `WS` set, one space
        // before the message removed and the rest kept.
        (
            "\u{3000} 06-18\u{a0}19:24:01.123\tF/알림 🙂 ( 7):   들여쓴 메시지 🙂",
            [
                "06-18",
                "19:24:01.123",
                "7",
                "",
                "F",
                "알림 🙂",
                "  들여쓴 메시지 🙂",
                "-",
            ],
        ),
    ] {
        expect(raw, want);
    }
}

#[test]
fn threadtime_uid_rows() {
    for (raw, want) in [
        (
            "06-18 19:24:01.123  1000: 2983  4241 D Tag     : msg",
            [
                "06-18",
                "19:24:01.123",
                "2983",
                "4241",
                "D",
                "Tag",
                "msg",
                "-",
            ],
        ),
        (
            "06-18 19:24:01.123 10123:12345 12360 W chromium: a: b",
            [
                "06-18",
                "19:24:01.123",
                "12345",
                "12360",
                "W",
                "chromium",
                "a: b",
                "-",
            ],
        ),
        (
            "2026-06-18 19:24:01.123456  root     1     1 I init    : boot",
            [
                "2026-06-18",
                "19:24:01.123456",
                "1",
                "1",
                "I",
                "init",
                "boot",
                "-",
            ],
        ),
        (
            "06-18 19:24:01.123 u0_a57  4567  4580 E 앱: 오류 🙂",
            [
                "06-18",
                "19:24:01.123",
                "4567",
                "4580",
                "E",
                "앱",
                "오류 🙂",
                "-",
            ],
        ),
    ] {
        expect(raw, want);
    }
}

#[test]
fn android_studio_rows() {
    for (raw, want) in [
        (
            "2026-06-18 19:24:01.123  2983-4241  AlertsService           com.example.app                      D  Sending update",
            ["2026-06-18", "19:24:01.123", "2983", "4241", "D", "AlertsService", "Sending update", "com.example.app"],
        ),
        // Unknown apps show the PID or `?`; `Show process ID` without the
        // thread ID leaves TID empty.
        (
            "2026-06-18 19:24:01.123   639-639   ActivityManager         pid-639                              W  ",
            ["2026-06-18", "19:24:01.123", "639", "639", "W", "ActivityManager", "", "pid-639"],
        ),
        (
            "2026-06-18 19:24:01.123  1534  Zygote                  ?                                    I  Late-enabling",
            ["2026-06-18", "19:24:01.123", "1534", "", "I", "Zygote", "Late-enabling", "?"],
        ),
        // Two spaces after the level are removed; more stay in the message.
        (
            "2026-06-18 19:24:01.123 1234567-1234568 Tag /system/bin/surfaceflinger A     x  y 🙂",
            ["2026-06-18", "19:24:01.123", "1234567", "1234568", "A", "Tag", "   x  y 🙂", "/system/bin/surfaceflinger"],
        ),
        // Columns collapsed to single spaces or other `WS` still parse.
        (
            "2026-06-18 19:24:01.123 2983-4241 알림🙂 com.example.app E 한글 메시지",
            ["2026-06-18", "19:24:01.123", "2983", "4241", "E", "알림🙂", "한글 메시지", "com.example.app"],
        ),
        (
            "2026-06-18\u{a0}19:24:01.123\u{3000}2983-4241\tTag\u{a0}pkg\u{a0}V\u{a0}\u{a0}m",
            ["2026-06-18", "19:24:01.123", "2983", "4241", "V", "Tag", "m", "pkg"],
        ),
    ] {
        expect(raw, want);
    }
}

#[test]
fn brief_rows() {
    for (raw, want) in [
        (
            "D/AlertsService( 2983): Sending update",
            [
                "",
                "",
                "2983",
                "",
                "D",
                "AlertsService",
                "Sending update",
                "-",
            ],
        ),
        (
            "  E/AndroidRuntime(10123: 2983): FATAL EXCEPTION: main",
            [
                "",
                "",
                "2983",
                "",
                "E",
                "AndroidRuntime",
                "FATAL EXCEPTION: main",
                "-",
            ],
        ),
        (
            "I/O error( 42): a brief row",
            ["", "", "42", "", "I", "O error", "a brief row", "-"],
        ),
    ] {
        expect(raw, want);
    }
}

#[test]
fn near_misses_stay_raw() {
    for raw in [
        // `-v time` and `brief` need `L/`, a known level, digits in the PID
        // group and the colon after it.
        "06-18 19:24:01.123 D/Tag( 2983) no colon",
        "06-18 19:24:01.123 D/Tag(2983 ): space before the parenthesis",
        "06-18 19:24:01.123 D/Tag( 29a3): letters",
        "06-18 19:24:01.123 D/Tag( １２): fullwidth digits",
        "06-18 19:24:01.123 D/Tag(): empty",
        "06-18 19:24:01.123 X/Tag( 1): unknown level",
        "06-18 19:24:01.123 d/Tag( 1): lowercase level",
        "06-18 19:24:01.123 D /Tag( 1): space before the slash",
        "06-18 19:24:01.123 DD/Tag( 1): two letters",
        "06-18 19:24:01.12 D/Tag( 1): short fraction",
        "06-18 19:24:01.1234567 D/Tag( 1): seven fraction digits",
        "06-18 19:24:01.12345678 D/Tag( 1): eight fraction digits",
        "06-18 19:24:01 D/Tag( 1): no fraction",
        "6-18 19:24:01.123 D/Tag( 1): short date",
        "D/Tag(1)",
        "Done/Tag( 1): word",
        "I/O: no PID",
        "W/Tag: tag format",
        // `threadtime` with `uid` needs three IDs and a tag colon.
        "06-18 19:24:01.123  1000: 2983 D Tag: one ID after the UID",
        "06-18 19:24:01.123  1000  2983  4241 D Tag without colon",
        "06-18 19:24:01.123  1000-2983  4241 D Tag: dash",
        "06-18 19:24:01.123  ro.ot:  1  1 I init: dot in the user name",
        // Android Studio needs a year, `PID[-TID]`, exactly two words and a
        // one-letter level.
        "06-18 19:24:01.123  2983-4241  AlertsService  com.example.app  D  no year",
        "2026-06-18 19:24:01.123  2983-4241  AlertsService  D  tag or package hidden",
        "2026-06-18 19:24:01.123  2983-4241  My Tag  com.example.app  D  tag with a space",
        "2026-06-18 19:24:01.123  2983-4241  Tag  com.example.app  proc  D  process column",
        "2026-06-18 19:24:01.123  AlertsService  com.example.app  D  IDs hidden",
        "2026-06-18 19:24:01.123  2983-  Tag  pkg  D  missing TID",
        "2026-06-18 19:24:01.123  -4241  Tag  pkg  D  missing PID",
        "2026-06-18 19:24:01.123  2983-4241-1  Tag  pkg  D  three IDs",
        "2026-06-18 19:24:01.123  2983-４２  Tag  pkg  D  fullwidth TID",
        "2026-06-18 19:24:01.123  2983-4241  Tag  pkg  d  lowercase level",
        "2026-06-18 19:24:01.123  2983-4241  Tag  pkg  DEBUG  level word",
        "2026-06-18 19:24:01.123  2983-4241  Tag  pkg  U  unknown level",
        "2026-06-18 19:24:01.123  2983-4241  Tag  pkg",
        "19:24:01.123  2983-4241  Tag  com.example.app  D  time only",
        "1718709841.123  2983-4241  Tag  com.example.app  D  epoch",
        "2026-06-18 19:24:01.123 +0900  2983-4241  Tag  pkg  D  zone",
        // Zones are `+HHMM` or `-HHMM` with whitespace after them; epoch and
        // monotonic seconds take a 3, 6 or 9 digit fraction and no date.
        "06-18 19:24:01.123 +09:00  1  2 I Tag: colon in the zone",
        "06-18 19:24:01.123 +090  1  2 I Tag: short zone",
        "06-18 19:24:01.123 +0900:  1  2 I Tag: no space after the zone",
        "1718709841.1234567  1  2 I Tag: seven fraction digits",
        "1718709841  1  2 I Tag: no fraction",
        "1718709841.123 x 2 I Tag: letters",
        "1718709841.123  1  2 I Tag without colon",
        "06-18 1718709841.123  1  2 I Tag: date and seconds",
        "-1.123  1  2 I Tag: sign",
        // After zone, epoch and monotonic stamps a UID needs logcat's `:`.
        "  1.500 s  2 3 E x: y",
        "1718709841.123 10123  1234  5678 I Tag: UID without colon",
        "06-18 19:24:01.123 +0900  1000  2983  4241 I Tag: UID without colon",
        // `-v long` headers need `[ `, the IDs as `PID:TID`, `L/` and ` ]`.
        "[ 06-18 19:24:01.123  2983  4241 D/Tag ]",
        "[ 06-18 19:24:01.123  2983:4241:1:2 D/Tag ]",
        "[ 06-18 19:24:01.123  29a3: 4241 D/Tag ]",
        "[ 06-18 19:24:01.123  2983: 4241 X/Tag ]",
        "[ 06-18 19:24:01.123  2983: 4241 D Tag ]",
        "[ 06-18 19:24:01.123  2983: 4241 D/Tag",
        "[ 06-18 19:24:01.123  2983: 4241 D/Tag ] trailing",
        "[06-18 19:24:01.123  2983: 4241 D/Tag ]",
        "[ 06-18 19:24:01.123  2983: 4241D/Tag ]",
        "[  root:    1:    1 I/init     ]",
        // Syslog rows need `Mmm dd HH:MM:SS`, a device, `process[PID]`, a
        // known `<Level>` and the colon.
        "Oct 24 09:00:00 iPhone App[123] <Notice> no colon",
        "Oct 24 09:00:00 iPhone App[123] <Verbose>: unknown level",
        "Oct 24 09:00:00 iPhone App[12a] <Notice>: letters",
        "Oct 24 09:00:00 iPhone App[] <Notice>: empty PID",
        "Oct 24 09:00:00 App[123] <Notice>: no device",
        "Oct 24 09:00:00 iPhone [123] <Notice>: no process",
        "Oct 24 09:00:00 iPhone (Lib)[123] <Notice>: library only",
        "Oct 24 09:00:00 iPhone App (Lib)[123] <Notice>: space before the library",
        "Oct 24 09:00:00 iPhone App(Lib)) [123] <Notice>: space before the PID",
        "Oct 24 09:00:00 iPhone App[1:2] <Notice>: thread",
        "Oct 24 9:00:00 iPhone App[123] <Notice>: short hour",
        "Oct 24 09:00:00.123 iPhone App[123] <Notice>: fraction",
        "oct 24 09:00:00 iPhone App[123] <Notice>: lowercase month",
        "Okt 24 09:00:00 iPhone App[123] <Notice>: other language",
        // Other logs that start with a stamp.
        "2026-06-18 19:24:01.123  INFO 1234 --- [main] o.s.b.App  : Started",
        "2026-06-18 19:24:01.123 [main] INFO c.e.App - message",
        "2026-06-18 19:24:01,123 1234 Tag pkg D comma fraction",
    ] {
        for recognize in [false, true] {
            let r = parse_line(raw, 3, recognize);
            assert_eq!(r.kind, "raw", "{raw:?}");
            assert!(r.parsed.is_none() && r.source_number.is_none(), "{raw:?}");
        }
    }
}

#[test]
fn line_terminators_keep_rows_raw() {
    for row in [
        "06-18 19:24:01.123 D/Tag( 1): m",
        "06-18 19:24:01.123  1000: 2983  4241 D Tag: m",
        "2026-06-18 19:24:01.123  2983-4241  Tag  pkg  D  m",
        "D/Tag( 1): m",
    ] {
        assert!(fields(row, false).is_some(), "{row:?}");
        for end in ["\r", "\n", "\r\n", "\u{2028}", "\u{2029}"] {
            let before_m = format!("{}{end}m", &row[..row.len() - 1]);
            for raw in [format!("{row}{end}"), before_m] {
                assert!(fields(&raw, false).is_none(), "{raw:?}");
                assert!(fields(&raw, true).is_none(), "{raw:?}");
            }
        }
    }
}

#[test]
fn earlier_formats_keep_their_fields() {
    // A dated row that is also a valid iOS row stays iOS, as dated
    // `threadtime` rows do.
    for raw in [
        "2026-06-18 19:24:01.123  2983-4241  Tag  pkg  D  sent to Runner[1:2] [INFO] [T] x",
        "2026-06-18 19:24:01.123 D/Tag( 1): Runner[1:2] [INFO] [T] x",
    ] {
        let v = parse_line(raw, 1, false).json(usize::MAX);
        assert_eq!(
            (v["kind"].as_str(), v["year"].as_str()),
            (Some("parsed"), Some("2026")),
            "{raw}"
        );
        assert_eq!(v["pid"], "1");
    }
    // `threadtime` keeps a tag that looks like the start of another format.
    let p = parse_line("06-18 19:24:01.123 1 2 I 3 4 D x: y", 1, false)
        .parsed
        .unwrap();
    assert_eq!(
        (p.pid, p.tid, p.level, p.tag, p.message),
        ("1", "2", "I", "3 4 D x", "y")
    );
    let p = parse_line("06-18 19:24:01.123 1 2 W D/Tag( 3): m", 1, false)
        .parsed
        .unwrap();
    assert_eq!((p.pid, p.tag, p.message), ("1", "D/Tag( 3)", "m"));
}

#[test]
fn numbered_prefixes() {
    for row in [
        "06-18 19:24:01.123 D/Tag( 1): m",
        "06-18 19:24:01.123  1000: 2983  4241 D Tag: m",
        "2026-06-18 19:24:01.123  2983-4241  Tag  pkg  D  m",
        "D/Tag( 1): m",
    ] {
        let want = fields(row, false).unwrap();
        for prefix in ["0042 ", "  17\t", "999999999999 "] {
            let raw = format!("{prefix}{row}");
            let r = parse_line(&raw, 5, true);
            assert_eq!(r.source_number, Some(parser::trim(prefix)), "{raw:?}");
            assert_eq!(r.raw, raw);
            assert_eq!(fields(&raw, true).unwrap(), want, "{raw:?}");
            assert!(parser::has_source_prefix(&raw));
            assert!(fields(&raw, false).is_none(), "{raw:?}");
        }
        assert!(!parser::has_source_prefix(row));
        assert_eq!(parse_line(row, 5, true).source_number, None);
    }
    // The prefix needs a whole supported row after it.
    for raw in [
        "12 06-18 19:24:01.123 D/Tag( 1) m",
        "12 D/Tag: m",
        "12 2026-06-18 19:24:01.123 1 Tag D m",
    ] {
        assert!(!parser::has_source_prefix(raw), "{raw:?}");
    }
}

#[test]
fn malformed_rows_start_entries_as_before() {
    // Parsed rows always open entries. Without fields, a stamped row still
    // opens an orphan entry and an unstamped `brief` row continues one.
    for (raw, starts) in [
        ("06-18 19:24:01.123 X/Tag( 1): m", true),
        ("2026-06-18 19:24:01.123  2983-4241  Tag  D  m", true),
        ("X/Tag( 1): m", false),
        ("D/Tag(1)", false),
        // A malformed `-v long` header or syslog row opens one too; rows
        // that start with seconds only continue, as numbers often do.
        ("[ 06-18 19:24:01.123  2983: 4241 X/Tag ]", true),
        ("[\t2026-06-18 19:24:01 broken", true),
        ("Oct 24 09:00:00 iPhone App[123] <Verbose>: m", true),
        ("Oct  4 09:00:00 iPhone App", true),
        ("[06-18 19:24:01.123 no space", false),
        ("[ ok ]", false),
        ("Oct 24 9:00 later", false),
        ("October 24 09:00:00 iPhone App[1] <Notice>: m", false),
        ("1718709841.123  1  2 X Tag: m", false),
        ("  12.500 ms elapsed", false),
    ] {
        assert!(parse_line(raw, 1, true).parsed.is_none());
        assert_eq!(parser::starts_entry(raw, true), starts, "{raw:?}");
    }
}

#[test]
fn zone_epoch_and_monotonic_rows() {
    for (raw, want) in [
        // `-v zone` puts the offset after the time; it stays in the raw row.
        (
            "06-18 19:24:01.123 +0900  2983  4241 D AlertsService: zone",
            [
                "06-18",
                "19:24:01.123",
                "2983",
                "4241",
                "D",
                "AlertsService",
                "zone",
                "-",
            ],
        ),
        (
            "2026-06-18 19:24:01.123456 -0130 W/Tag     (  417): time zone",
            [
                "2026-06-18",
                "19:24:01.123456",
                "417",
                "",
                "W",
                "Tag",
                "time zone",
                "-",
            ],
        ),
        (
            "06-18 19:24:01.123 +0000  1000: 2983  4241 I Tag: uid zone",
            [
                "06-18",
                "19:24:01.123",
                "2983",
                "4241",
                "I",
                "Tag",
                "uid zone",
                "-",
            ],
        ),
        // `-v epoch` pads the seconds to 19 columns, `-v monotonic` to 6;
        // they fill the time field and leave the date empty.
        (
            "         1718709841.123  2983  4241 E Tag: epoch",
            [
                "",
                "1718709841.123",
                "2983",
                "4241",
                "E",
                "Tag",
                "epoch",
                "-",
            ],
        ),
        (
            "  1234.567890  2983  4241 V Tag: monotonic usec",
            [
                "",
                "1234.567890",
                "2983",
                "4241",
                "V",
                "Tag",
                "monotonic usec",
                "-",
            ],
        ),
        (
            "     0.123456789 u0_a57:  4567  4568 F chromium: uid nsec",
            [
                "",
                "0.123456789",
                "4567",
                "4568",
                "F",
                "chromium",
                "uid nsec",
                "-",
            ],
        ),
        (
            "1718709841.123 E/Foo(bar)(10123: 2983): time epoch",
            [
                "",
                "1718709841.123",
                "2983",
                "",
                "E",
                "Foo(bar)",
                "time epoch",
                "-",
            ],
        ),
        (
            "\x1b[38;5;196m1718709841.123  2983  4241 E Tag: color epoch\x1b[0m",
            [
                "",
                "1718709841.123",
                "2983",
                "4241",
                "E",
                "Tag",
                "color epoch",
                "-",
            ],
        ),
    ] {
        expect(raw, want);
    }
    let numbered = "0042 1718709841.123  2983  4241 E Tag: numbered";
    assert_eq!(parse_line(numbered, 1, true).source_number, Some("0042"));
    assert!(fields(numbered, false).is_none());
}

#[test]
fn long_headers() {
    for (raw, want) in [
        (
            "[ 06-18 19:24:01.123  2983: 4241 D/AlertsService ]",
            [
                "06-18",
                "19:24:01.123",
                "2983",
                "4241",
                "D",
                "AlertsService",
                "",
                "-",
            ],
        ),
        // `%-8s` pads the tag; tags may hold spaces and `]`.
        (
            "[ 2026-06-18 19:24:01.123456789   417:  418 W/My [Tag]  ]",
            [
                "2026-06-18",
                "19:24:01.123456789",
                "417",
                "418",
                "W",
                "My [Tag]",
                "",
                "-",
            ],
        ),
        (
            "[ 06-18 19:24:01.123 +0900  1000: 2983: 4241 I/uid      ]",
            ["06-18", "19:24:01.123", "2983", "4241", "I", "uid", "", "-"],
        ),
        (
            "[     1718709841.123  2983: 4241 E/epoch ]",
            ["", "1718709841.123", "2983", "4241", "E", "epoch", "", "-"],
        ),
        (
            "\x1b[38;5;40m[ 06-18 19:24:01.123  2983: 4241 I/color ]",
            [
                "06-18",
                "19:24:01.123",
                "2983",
                "4241",
                "I",
                "color",
                "",
                "-",
            ],
        ),
    ] {
        expect(raw, want);
    }
}

#[test]
fn iphone_syslog_rows() {
    for (raw, want, level_text) in [
        (
            "Oct 24 09:00:00 iPhone MyApp(Foundation)[123] <Notice>: Sending update",
            [
                "Oct 24",
                "09:00:00",
                "123",
                "",
                "U",
                "Foundation",
                "Sending update",
                "MyApp",
            ],
            "Notice",
        ),
        // Device names may hold spaces; the day is padded with a space.
        (
            "Jan  4 23:59:59 Kim's iPad SpringBoard[60] <Error>: 한글 🙂 [x] <y>: z",
            [
                "Jan  4",
                "23:59:59",
                "60",
                "",
                "E",
                "",
                "한글 🙂 [x] <y>: z",
                "SpringBoard",
            ],
            "Error",
        ),
        (
            "Dec 31 00:00:01 iPhone kernel(AppleMobileFileIntegrity)[0] <Fault>:",
            [
                "Dec 31",
                "00:00:01",
                "0",
                "",
                "F",
                "AppleMobileFileIntegrity",
                "",
                "kernel",
            ],
            "Fault",
        ),
        (
            "Mar 13 10:15:44 iPhone-de-Juan com.apple.WebKit.WebContent(WebKit)[4567] <Debug>: d",
            [
                "Mar 13",
                "10:15:44",
                "4567",
                "",
                "D",
                "WebKit",
                "d",
                "com.apple.WebKit.WebContent",
            ],
            "Debug",
        ),
        // The library may hold spaces.
        (
            "Jun 30 12:00:00 iPhone App(Core Data)[1] <Notice>: m",
            ["Jun 30", "12:00:00", "1", "", "U", "Core Data", "m", "App"],
            "Notice",
        ),
        (
            "Sep  9 08:07:06 iPhone App[1] <Warning>: w",
            ["Sep  9", "08:07:06", "1", "", "W", "", "w", "App"],
            "Warning",
        ),
        (
            "Apr  1 08:07:06 iPhone App[1] <Critical>: c",
            ["Apr  1", "08:07:06", "1", "", "F", "", "c", "App"],
            "Critical",
        ),
        (
            "May  1 08:07:06 iPhone App[1] <Info>: i",
            ["May  1", "08:07:06", "1", "", "I", "", "i", "App"],
            "Info",
        ),
    ] {
        expect(raw, want);
        let v = parse_line(raw, 1, false).json(usize::MAX);
        assert_eq!(v["levelText"], level_text, "{raw:?}");
        // No year, but the key marks an iOS row for the page's layout.
        assert!(v["year"].is_null() && v.get("year").is_some(), "{raw:?}");
        let tags: Vec<&str> = if want[5].is_empty() {
            vec![]
        } else {
            vec![want[5]]
        };
        assert_eq!(v["tags"], json!(tags), "{raw:?}");
        assert!(parse_line(raw, 1, false).parsed.unwrap().ios);
    }
    // A syslog file is an iOS log: Apple level names and repair apply.
    assert!(parser::ios_log(
        b"Oct 24 09:00:00 iPhone App[1] <Notice>: a\nOct 24 09:00:01 iPhone App[1] <Error>: b\n"
    ));
}

#[test]
fn long_and_syslog_logs_group_their_rows() {
    let text = "\
[ 06-18 19:24:01.123  2983: 4241 D/AlertsService ]\n\
Sending update\n\
  {\"body\": \"안내문구3\"}\n\
\n\
[ 06-18 19:24:02.500  2983: 4242 E/AndroidRuntime ]\n\
FATAL EXCEPTION: main\n\
\tat com.example.Main.run(Main.java:10)\n\
\n\
Oct 24 09:00:00 iPhone MyApp(Foundation)[123] <Notice>: first\n\
second line of first\n\
Oct 24 09:00:01 iPhone MyApp[123] <Error>: failed\n\
         1718709841.123  2983  4241 W Tag: epoch\n\
  1234.567 not an entry of its own\n";
    let d = Dataset::new(1, Source::text(text, "long").unwrap());
    d.start(|_| {});
    wait(|| d.status(0)["complete"] == true);
    assert_eq!(
        d.status(0)["summary"]["counts"],
        json!({"V":0,"D":1,"I":0,"W":1,"E":2,"F":0,"A":0,"U":1})
    );
    let base = Filters::default;
    let mut id = 0;
    for (filters, expected) in [
        (
            Filters {
                levels: vec!["E".into()],
                ..base()
            },
            vec![5, 6, 7, 8, 11],
        ),
        (
            Filters {
                tag: "alerts".into(),
                ..base()
            },
            vec![1, 2, 3, 4],
        ),
        (
            Filters {
                tag: "foundation".into(),
                ..base()
            },
            vec![9, 10],
        ),
        (
            Filters {
                pid: "123".into(),
                ..base()
            },
            vec![9, 10, 11],
        ),
        (
            Filters {
                levels: vec!["W".into()],
                ..base()
            },
            vec![12, 13],
        ),
        // Rows without a month-day date are left out by time filters.
        (
            Filters {
                time_from: "00:00".into(),
                time_to: "23:59".into(),
                ..base()
            },
            vec![1, 2, 3, 4, 5, 6, 7, 8],
        ),
        (
            Filters {
                levels: vec!["E".into()],
                query: "main".into(),
                ..base()
            },
            vec![6, 7],
        ),
    ] {
        id += 1;
        let q = query(&d, id, &filters);
        assert_eq!(lines(&all_rows(&d, &q)), expected, "{filters:?}");
    }
    let rows = all_rows(&d, &query(&d, 100, &Filters::default()));
    assert_eq!(rows[1]["owner"]["lineNumber"], 1);
    assert_eq!(rows[9]["owner"]["tag"], "Foundation");
    assert_eq!(rows[8]["process"], "MyApp");
    for row in rows.iter().filter(|r| r["kind"] == "parsed") {
        let raw: Vec<u16> = row["raw"].as_str().unwrap().encode_utf16().collect();
        for (key, at) in row["fieldOffsets"].as_object().unwrap() {
            let text: Vec<u16> = row[key].as_str().unwrap().encode_utf16().collect();
            let at = at.as_u64().unwrap() as usize;
            assert_eq!(&raw[at..at + text.len()], &text[..], "{key} of {row}");
        }
    }
    d.close();
}

#[test]
fn color_codes_stay_outside_the_fields() {
    // `logcat -v color` wraps each row in an SGR color and a reset; the
    // fields are read inside them and the raw row keeps both.
    for (raw, want) in [
        (
            "\x1b[38;5;196m09-21 12:07:30.386  1234  5678 E Tag: boom\x1b[0m",
            [
                "09-21",
                "12:07:30.386",
                "1234",
                "5678",
                "E",
                "Tag",
                "boom",
                "-",
            ],
        ),
        (
            "\x1b[38;5;75m2026-09-21 12:07:30.386123 D/Tag     ( 417): x\x1b[0m",
            [
                "2026-09-21",
                "12:07:30.386123",
                "417",
                "",
                "D",
                "Tag",
                "x",
                "-",
            ],
        ),
        (
            "\x1b[38;5;40mI/Tag( 7): brief\x1b[0m",
            ["", "", "7", "", "I", "Tag", "brief", "-"],
        ),
        (
            "\x1b[38;5;166m09-21 12:07:30.386 10123  1234  5678 W Tag: uid\x1b[0m",
            [
                "09-21",
                "12:07:30.386",
                "1234",
                "5678",
                "W",
                "Tag",
                "uid",
                "-",
            ],
        ),
        // Several leading codes, `ESC [ m` as the reset, a missing reset.
        (
            "\x1b[1m\x1b[38;5;231m09-21 12:07:30.386 1 2 V Tag: v\x1b[m",
            ["09-21", "12:07:30.386", "1", "2", "V", "Tag", "v", "-"],
        ),
        (
            "\x1b[38;5;40m09-21 12:07:30.386 1 2 I Tag: cut",
            ["09-21", "12:07:30.386", "1", "2", "I", "Tag", "cut", "-"],
        ),
        // Only the row's own reset is removed; the message keeps its codes.
        (
            "\x1b[38;5;40m09-21 12:07:30.386 1 2 I Tag: \x1b[1mbold\x1b[0m\x1b[0m",
            [
                "09-21",
                "12:07:30.386",
                "1",
                "2",
                "I",
                "Tag",
                "\x1b[1mbold\x1b[0m",
                "-",
            ],
        ),
        // Without a leading code nothing is removed, as before.
        (
            "09-21 12:07:30.386 1 2 I Tag: \x1b[31mred\x1b[0m",
            [
                "09-21",
                "12:07:30.386",
                "1",
                "2",
                "I",
                "Tag",
                "\x1b[31mred\x1b[0m",
                "-",
            ],
        ),
    ] {
        expect(raw, want);
        assert_eq!(parse_line(raw, 1, false).raw, raw);
    }
    let numbered = "0042 \x1b[38;5;40m09-21 12:07:30.386 1 2 I Tag: m\x1b[0m";
    assert_eq!(parse_line(numbered, 1, true).source_number, Some("0042"));
    assert_eq!(fields(numbered, true).unwrap()[6], "m");
    // Codes that are no SGR color leave the row raw.
    for raw in [
        "\x1b[2K09-21 12:07:30.386 1 2 I Tag: erase line",
        "\x1b[38;5;40 09-21 12:07:30.386 1 2 I Tag: no m",
        "\x1b09-21 12:07:30.386 1 2 I Tag: bare escape",
    ] {
        assert!(fields(raw, true).is_none(), "{raw:?}");
    }
    // A colored row without fields opens an entry when it is stamped and
    // continues one otherwise.
    for (raw, starts) in [
        (
            "\x1b[38;5;196m09-21 12:07:30.386 x y E Tag: bad ids\x1b[0m",
            true,
        ),
        ("0007 \x1b[38;5;196m2026-09-21 12:07:30 body\x1b[0m", true),
        (
            "\x1b[38;5;196m\tat com.example.Main.run(Main.java:10)\x1b[0m",
            false,
        ),
        ("\x1b[31m", false),
    ] {
        assert!(parse_line(raw, 1, true).parsed.is_none(), "{raw:?}");
        assert_eq!(parser::starts_entry(raw, true), starts, "{raw:?}");
    }
}

#[test]
fn color_log_counts_filters_and_offsets() {
    let color = |c: u8, row: &str| format!("\x1b[38;5;{c}m{row}\x1b[0m\n");
    let text = [
        color(40, "09-21 12:07:30.386  1234  5678 I Activity: start"),
        color(196, "09-21 12:07:30.400  1234  5678 E Activity: crash"),
        color(
            196,
            "09-21 12:07:30.400  1234  5678 E Activity: \tat Main.run",
        ),
        color(166, "09-21 12:07:31.000  4321  4321 W 알림🙂: 한글"),
        "  plain continuation\n".to_string(),
    ]
    .concat();
    let d = Dataset::new(1, Source::text(&text, "color").unwrap());
    d.start(|_| {});
    wait(|| d.status(0)["complete"] == true);
    assert_eq!(
        d.status(0)["summary"],
        json!({"total":5,"parsed":4,"blank":0,"raw":1,"continuation":1,
            "counts":{"V":0,"D":0,"I":1,"W":1,"E":2,"F":0,"A":0,"U":0}})
    );
    let errors = Filters {
        levels: vec!["E".into()],
        ..Filters::default()
    };
    assert_eq!(lines(&all_rows(&d, &query(&d, 1, &errors))), vec![2, 3]);
    let tag = Filters {
        tag: "알림".into(),
        ..Filters::default()
    };
    assert_eq!(lines(&all_rows(&d, &query(&d, 2, &tag))), vec![4, 5]);
    let rows = all_rows(&d, &query(&d, 3, &Filters::default()));
    for row in rows.iter().filter(|r| r["kind"] == "parsed") {
        let raw = row["raw"].as_str().unwrap();
        assert!(
            raw.starts_with("\x1b[38;5;") && raw.ends_with("\x1b[0m"),
            "{row}"
        );
        let raw: Vec<u16> = raw.encode_utf16().collect();
        for (key, at) in row["fieldOffsets"].as_object().unwrap() {
            let text: Vec<u16> = row[key].as_str().unwrap().encode_utf16().collect();
            let at = at.as_u64().unwrap() as usize;
            assert_eq!(&raw[at..at + text.len()], &text[..], "{key} of {row}");
        }
    }
    assert_eq!(rows[0]["fieldOffsets"]["date"], 10);
    assert_eq!(rows[3]["message"], "한글");
    d.close();
}

fn wait(mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !done() {
        assert!(Instant::now() < deadline, "timed out");
        thread::sleep(Duration::from_millis(1));
    }
}
fn query(d: &Arc<Dataset>, id: u64, filters: &Filters) -> Arc<Query> {
    let q = d
        .query(0, id, Filter::new(filters.clone()).unwrap())
        .unwrap();
    wait(|| q.json()["status"] == "complete");
    q
}
fn all_rows(d: &Dataset, q: &Query) -> Vec<Value> {
    let count = q.json()["count"].as_u64().unwrap();
    let mut rows = vec![];
    while (rows.len() as u64) < count {
        let page = d.page(q, rows.len() as u64, 200).unwrap();
        let more = page["rows"].as_array().unwrap();
        assert!(!more.is_empty());
        rows.extend(more.iter().cloned());
    }
    rows
}
fn lines(rows: &[Value]) -> Vec<u64> {
    rows.iter()
        .map(|r| r["lineNumber"].as_u64().unwrap())
        .collect()
}

const SAMPLE: &str = "--------- beginning of main\r
06-18 19:24:01.123 D/AlertsService( 2983): Sending update\r
  {\"body\": \"안내문구3\"}\r
06-18 19:24:02.500 E/AndroidRuntime( 2983): FATAL EXCEPTION: main\r
\tat com.example.Main.run(Main.java:10)\r
2026-06-18 19:24:03.000  2983-4241  AlertsService           com.example.app                      W  Studio 한글 🙂\r
                                                                                                      second line\r
2026-06-18 19:24:04.250  1000-1000  ActivityManager         system_server                        I  Start proc\r
06-18 19:24:05.000  1000: 2983  4241 I AlertsService: uid row\r
I/ActivityManager( 1000): brief row\r
  brief continuation\r
06-18 19:24:06.000 X/Broken( 1): malformed\r
  tail of malformed\r
09-21 12:07:30.386  2983  4241 E Legacy: threadtime\r
2026-06-18 19:24:07.000 Runner[1:2] [INFO] [Swift] ios row\r
2026-06-18 19:24:08.000  4321-4321  알림🙂                     com.example.app                      V  유니코드 태그\r
";

#[test]
fn filters_and_summary_on_new_formats() {
    let d = Dataset::new(1, Source::text(SAMPLE, "formats").unwrap());
    d.start(|_| {});
    wait(|| d.status(0)["complete"] == true);
    let summary = d.status(0)["summary"].clone();
    assert_eq!(
        summary,
        json!({"total":16,"parsed":9,"blank":0,"raw":7,"continuation":4,
            "counts":{"V":1,"D":1,"I":4,"W":1,"E":2,"F":0,"A":0,"U":0}})
    );
    let base = Filters::default;
    let mut id = 0;
    for (filters, expected) in [
        (
            Filters {
                levels: vec!["E".into()],
                ..base()
            },
            vec![4, 5, 14],
        ),
        (
            Filters {
                levels: vec!["W".into()],
                ..base()
            },
            vec![6, 7],
        ),
        (
            Filters {
                tag: "alertsservice".into(),
                ..base()
            },
            vec![2, 3, 6, 7, 9],
        ),
        (
            Filters {
                tag: "알림".into(),
                ..base()
            },
            vec![16],
        ),
        (
            Filters {
                pid: "2983".into(),
                ..base()
            },
            vec![2, 3, 4, 5, 6, 7, 9, 14],
        ),
        (
            Filters {
                pid: "1000".into(),
                ..base()
            },
            vec![8, 10, 11],
        ),
        (
            Filters {
                tid: "4241".into(),
                ..base()
            },
            vec![6, 7, 9, 14],
        ),
        (
            Filters {
                time_from: "06-18 19:24:02".into(),
                time_to: "06-18 19:24:04.5".into(),
                ..base()
            },
            vec![4, 5, 6, 7, 8],
        ),
        (
            Filters {
                time_from: "2026-06-18 19:24:03".into(),
                time_to: "2026-06-18 19:24:04.25".into(),
                ..base()
            },
            vec![6, 7, 8],
        ),
        // `brief` rows have no timestamp, so a time filter leaves them out.
        (
            Filters {
                time_from: "00:00".into(),
                time_to: "23:59".into(),
                ..base()
            },
            vec![2, 3, 4, 5, 6, 7, 8, 9, 14, 15, 16],
        ),
        (
            Filters {
                include_raw: false,
                ..base()
            },
            vec![2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 14, 15, 16],
        ),
        (
            Filters {
                exclude_tags: "alerts".into(),
                ..base()
            },
            vec![1, 4, 5, 8, 10, 11, 12, 13, 14, 15, 16],
        ),
        (
            Filters {
                tag: "AlertsService".into(),
                query: "안내문구3".into(),
                ..base()
            },
            vec![3],
        ),
        (
            Filters {
                levels: vec!["I".into()],
                query: "brief".into(),
                ..base()
            },
            vec![10, 11],
        ),
        (
            Filters {
                recognize_line_numbers: Some(false),
                ..base()
            },
            (1..=16).collect(),
        ),
    ] {
        id += 1;
        let q = query(&d, id, &filters);
        assert_eq!(lines(&all_rows(&d, &q)), expected, "{filters:?}");
    }
    // Page rows: Android Studio's package is the `process` field, every
    // field offset (UTF-16) points at its text in the raw row, and
    // continuation rows name their entry.
    let q = query(&d, 100, &Filters::default());
    let rows = all_rows(&d, &q);
    assert_eq!(rows[5]["process"], "com.example.app");
    assert_eq!(rows[15]["process"], "com.example.app");
    assert_eq!(rows[15]["tag"], "알림🙂");
    assert!(rows[1].get("process").is_none() && rows[9].get("process").is_none());
    assert_eq!(
        (rows[9]["date"].as_str(), rows[9]["time"].as_str()),
        (Some(""), Some(""))
    );
    for row in rows.iter().filter(|r| r["kind"] == "parsed") {
        let raw: Vec<u16> = row["raw"].as_str().unwrap().encode_utf16().collect();
        assert!(!row["raw"].as_str().unwrap().ends_with('\r'));
        for (key, at) in row["fieldOffsets"].as_object().unwrap() {
            let text: Vec<u16> = row[key].as_str().unwrap().encode_utf16().collect();
            let at = at.as_u64().unwrap() as usize;
            assert_eq!(&raw[at..at + text.len()], &text[..], "{key} of {row}");
        }
    }
    for (line, owner) in [(3, 2), (5, 4), (7, 6), (11, 10)] {
        assert_eq!(rows[line - 1]["owner"]["lineNumber"], owner as u64);
    }
    assert_eq!(rows[6]["owner"]["tag"], "AlertsService");
    assert_eq!(rows[10]["owner"]["pid"], "1000");
    assert!(rows[0].get("owner").is_none() && rows[12].get("owner").is_none());
    d.close();
}

/// Native page rows rendered by the mounted UI test in
/// `tests/view-lifecycle.test.mjs`. `LOGSTUDIO_WRITE_FIXTURE=1` rewrites
/// the fixture from the current core.
#[test]
fn page_rows_for_the_ui_fixture() {
    let d = Dataset::new(1, Source::text(SAMPLE, "formats").unwrap());
    d.start(|_| {});
    wait(|| d.status(0)["complete"] == true);
    let filters = Filters {
        query: r"com\.example|brief|second|Sending|uid row".into(),
        search_mode: "regex".into(),
        ..Filters::default()
    };
    let page = d.page(&query(&d, 1, &filters), 0, 200).unwrap();
    if std::env::var_os("LOGSTUDIO_WRITE_FIXTURE").is_some() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../tests/android-formats-page.json"
        );
        let text = serde_json::to_string_pretty(&json!({"rows": page["rows"]})).unwrap();
        std::fs::write(path, text + "\n").unwrap();
    }
    let golden: Value =
        serde_json::from_str(include_str!("../../tests/android-formats-page.json")).unwrap();
    assert_eq!(
        page["rows"], golden["rows"],
        "native fields consumed by the UI fixture"
    );
    let rows = page["rows"].as_array().unwrap();
    assert_eq!(lines(rows), vec![2, 5, 6, 7, 9, 10, 11, 16]);
    d.close();
}

/// Every format, numbered rows, stack traces, malformed and blank rows and
/// CR/LF/CRLF over several 128 KiB blocks.
fn workload() -> String {
    let breaks = ["\n", "\r\n", "\r"];
    let tags = ["WifiService", "GalleryManager", "알림", "SDK_UI"];
    let mut out = String::from("orphan before the first entry\n\n");
    for i in 0..6_000usize {
        let br = breaks[i % 3];
        let (tag, level) = (tags[i % 4], ["V", "D", "I", "W", "E", "F", "A"][i / 3 % 7]);
        let (pid, tid, sec) = (120 + i % 5, 2100 + i % 3, i / 100 % 60);
        let row = match i % 13 {
            0 => format!("09-21 12:07:{sec:02}.{i:03} {level}/{tag:<8}({pid:>5}): time {i}"),
            1 => format!("2026-09-21 12:07:{sec:02}.{i:06} {level}/{tag}(10123:{pid:>5}): year uid {i}"),
            2 => format!("09-21 12:07:{sec:02}.{i:03} {:>5}:{pid:>5} {tid:>5} {level} {tag}: uid {i}", 10000 + i % 7),
            3 => format!(
                "2026-09-21 12:07:{sec:02}.{:03} {pid:>6}-{tid:<5} {tag:<23} {:<35}  {level}  studio 🙂 {i}",
                i % 1000,
                ["com.example.app", "pid-77", "?"][i % 3]
            ),
            4 => format!("{level}/{tag:<8}({pid:>5}): brief rare {i}"),
            5 => format!("{i:05} 09-21 12:07:{sec:02}.{i:03} {level}/{tag}({pid:>5}): numbered {i}"),
            6 => format!("09-21 12:07:{sec:02}.386 {pid} {tid} {level} {tag}: threadtime {i}"),
            7 => format!("09-21 12:07:{sec:02}.386 X/{tag}({pid}): malformed {i}"),
            8 => format!(
                "\x1b[38;5;{}m09-21 12:07:{sec:02}.386 {pid} {tid} {level} {tag}: color {i}\x1b[0m",
                40 + i % 200
            ),
            9 => format!("09-21 12:07:{sec:02}.{i:03} +0900 {pid:>5} {tid:>5} {level} {tag}: zone {i}"),
            10 => format!(
                "{:>19}.{:03} {pid:>5} {tid:>5} {level} {tag}: epoch {i}",
                1_761_350_400 + i,
                i % 1000
            ),
            // A `-v long` entry: its message rows continue the header.
            11 => format!("[ 09-21 12:07:{sec:02}.{i:03} {pid:>5}:{tid:>5} {level}/{tag:<8} ]{br}long body {i}"),
            _ => format!(
                "Sep 21 12:07:{sec:02} Test iPad App({tag})[{pid}] <{}>: syslog {i}",
                ["Debug", "Info", "Notice", "Warning", "Error", "Fault"][i / 13 % 6]
            ),
        };
        // Every kind but the malformed one is a supported row.
        let head = row.split(br).next().unwrap();
        assert_eq!(
            parse_line(head, 1, true).parsed.is_some(),
            i % 13 != 7,
            "{head:?}"
        );
        out += &row;
        out += br;
        for k in 0..i % 3 {
            out += &format!("\tat com.example.C{k}.run(C.java:{i}){br}");
        }
        if i % 50 == 0 {
            out += br;
        }
    }
    out
}
/// Expected rows by the documented per-row rule, and the summary counts.
fn oracle(data: &[u8], filters: &Filters) -> (Vec<u64>, Value) {
    let filter = Filter::new(filters.clone()).unwrap();
    let recognize = filters.recognizes_line_numbers();
    let (mut out, mut line, mut entry) = (vec![], 0u64, None::<(bool, bool)>);
    let (mut parsed, mut blank, mut orphans, mut counts) = (0, 0, 0, [0u64; 8]);
    for (_, bytes) in ByteLines::new(data) {
        line += 1;
        let raw = encoding::decode(bytes);
        let r = parse_line(&raw, line, recognize);
        if let Some(p) = &r.parsed {
            parsed += 1;
            counts[parser::LEVELS.iter().position(|l| *l == p.level).unwrap()] += 1;
        } else if r.kind == "blank" {
            blank += 1;
        }
        let (pass, orphan) = if r.parsed.is_some() || parser::starts_entry(&raw, recognize) {
            entry = Some((filter.fields_match(&r), r.parsed.is_none()));
            entry.unwrap()
        } else {
            entry.unwrap_or((filter.admits_unparsed(), true))
        };
        orphans += orphan as u64;
        if pass && (filters.query.is_empty() || filter.row_matches(&raw)) {
            out.push(line);
        }
    }
    let counts: serde_json::Map<String, Value> = parser::LEVELS
        .iter()
        .zip(counts)
        .map(|(l, n)| (l.to_string(), json!(n)))
        .collect();
    let summary = json!({"total":line,"parsed":parsed,"blank":blank,"raw":line-parsed-blank,
        "continuation":line-parsed-orphans,"counts":counts});
    (out, summary)
}
fn workload_filters() -> Vec<Filters> {
    let mut out = vec![];
    for recognize in [None, Some(false)] {
        let base = || Filters {
            recognize_line_numbers: recognize,
            ..Filters::default()
        };
        out.extend([
            base(),
            Filters {
                levels: vec!["E".into()],
                ..base()
            },
            Filters {
                levels: vec!["V".into(), "A".into()],
                include_raw: false,
                ..base()
            },
            Filters {
                tag: "wifi".into(),
                ..base()
            },
            Filters {
                tag: "알림".into(),
                levels: vec!["W".into()],
                ..base()
            },
            Filters {
                exclude_tags: "gallery, sdk".into(),
                ..base()
            },
            Filters {
                pid: "123".into(),
                ..base()
            },
            Filters {
                tid: "2101".into(),
                ..base()
            },
            Filters {
                time_from: "12:07:10".into(),
                time_to: "12:07:20.5".into(),
                ..base()
            },
            Filters {
                time_from: "2026-09-21 12:07:30".into(),
                ..base()
            },
            Filters {
                include_raw: false,
                ..base()
            },
            Filters {
                query: "rare".into(),
                levels: vec!["D".into(), "I".into()],
                ..base()
            },
            Filters {
                query: r"C[12]\.run".into(),
                search_mode: "regex".into(),
                pid: "124".into(),
                ..base()
            },
        ]);
    }
    out
}
fn check_workload(d: &Arc<Dataset>, data: &[u8], first_id: u64) -> Vec<Vec<u64>> {
    workload_filters()
        .iter()
        .enumerate()
        .map(|(k, filters)| {
            let q = query(d, first_id + k as u64, filters);
            let (expected, _) = oracle(data, filters);
            assert_eq!(q.json()["count"], expected.len() as u64, "{filters:?}");
            assert_eq!(lines(&all_rows(d, &q)), expected, "{filters:?}");
            expected
        })
        .collect()
}

#[test]
fn mixed_formats_across_blocks_before_and_after_analysis() {
    let text = workload();
    let d = Dataset::new(1, Source::text(&text, "mixed").unwrap());
    d.pause_analysis.store(true, Ordering::Release);
    d.start(|_| {});
    wait(|| d.index.lock().unwrap().done);
    let blocks = d.index.lock().unwrap().blocks.len();
    assert!(blocks > 4, "{blocks} blocks");
    let before = check_workload(&d, text.as_bytes(), 1);
    d.resume();
    wait(|| d.index.lock().unwrap().block_levels.is_some());
    let after = check_workload(&d, text.as_bytes(), 1000);
    assert_eq!(before, after);
    // Level summaries in both line-number modes; most rows are parsed.
    for (id, recognize) in [(2000, None), (2001, Some(false))] {
        let filters = Filters {
            recognize_line_numbers: recognize,
            ..Filters::default()
        };
        query(&d, id, &filters);
        let (_, expected) = oracle(text.as_bytes(), &filters);
        assert_eq!(d.status(0)["summary"], expected, "{recognize:?}");
    }
    let summary = d.status(0)["summary"].clone();
    assert!(summary["parsed"].as_u64().unwrap() * 10 > summary["total"].as_u64().unwrap() * 3);
    d.close();
}
