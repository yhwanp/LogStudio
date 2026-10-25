//! Random rows must never panic the parsers: a panic in a scan, analysis or
//! page worker would leave its query running forever. The rows mix the
//! characters the formats care about (digits, separators, level letters,
//! brackets, Unicode whitespace, line terminators, multibyte text) so that
//! slicing lands on and around every kind of boundary.
use logstudio_core::{
    dlt, encoding,
    parser::{parse_in, parse_line, starts_entry},
    source::Format,
};

const TOKENS: &[&str] = &[
    "0",
    "1",
    "9",
    "09-21",
    "2024-09-21",
    "12:07:30.386",
    "12:07:30.386123",
    "12:07:30.3861234",
    "-",
    ":",
    ".",
    "/",
    "(",
    ")",
    "[",
    "]",
    " ",
    "  ",
    "\t",
    "\r",
    "\n",
    "\r\n",
    "\u{a0}",
    "\u{2028}",
    "\u{2029}",
    "\u{3000}",
    "\u{feff}",
    "V",
    "D",
    "I",
    "W",
    "E",
    "F",
    "A",
    "U",
    "I/",
    "E/",
    "Tag",
    "Tag:",
    "한글",
    "🙂",
    "ı",
    "ſ",
    "İ",
    "[Error]",
    "[info]",
    "proc[1:2]",
    "123",
    "4567",
    "0:",
    "u0_a12:",
    "root ",
    "( 2983):",
    "(10123: 2983):",
    "1234-5678",
    "!! ",
    "ECU1",
    "APP",
    "CTX",
    "log info",
    "--",
    "[",
    "]",
    "e",
    "\u{85}",
    "\u{200b}",
    "\x1b[38;5;196m",
    "\x1b[0m",
    "\x1b[",
    "\x1b",
    "m",
    "+0900",
    "1718709841.123",
    "] <",
    "<Notice>:",
    "<Error>",
    ">:",
    "App(Lib)[12]",
    "Oct",
    "Oct 24 09:00:00",
    "2983: 4241",
    " ]",
];

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn row(rng: &mut Rng) -> String {
    let mut s = String::new();
    // Mostly plausible rows: start with a stamp or a number half the time.
    match rng.below(11) {
        0 => s.push_str("09-21 12:07:30.386 "),
        1 => s.push_str("2024-09-21 12:07:30.386 "),
        2 => s.push_str("12 09-21 12:07:30.386 "),
        3 => s.push_str("\x1b[38;5;40m09-21 12:07:30.386 "),
        4 => s.push_str("1718709841.123 "),
        5 => s.push_str("[ 06-18 19:24:01.123 "),
        6 => s.push_str("Oct 24 09:00:00 iPhone "),
        _ => {}
    }
    for _ in 0..rng.below(14) {
        s.push_str(TOKENS[rng.below(TOKENS.len())]);
    }
    s
}

fn exercise(raw: &str, line: u64) {
    for recognize in [false, true] {
        let r = parse_line(raw, line, recognize);
        let _ = r.json(usize::MAX);
        let _ = r.json(8);
        if let Some(p) = &r.parsed {
            let _ = p.tag_list().len();
            let _ = p.spans().count();
        }
        let _ = starts_entry(raw, recognize);
        let _ = parse_in(Format::Dlt, raw, line, recognize).json(usize::MAX);
    }
    let _ = dlt::parse_row(raw);
}

#[test]
fn random_rows_never_panic() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for i in 0..200_000u64 {
        let raw = row(&mut rng);
        exercise(&raw, i);
    }
}

/// Each thread searches with its own regex caches and capture slots, so
/// rows parsed on several threads at once, in any order, keep the fields
/// they have on one thread.
#[test]
fn threads_parse_rows_as_one_thread_does() {
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    let mut rows: Vec<String> = (0..20_000).map(|_| row(&mut rng)).collect();
    rows.extend(
        [
            "09-21 12:07:30.386  1234  5678 I Tag: message",
            "2026-09-30 21:55:53.776261 Runner[7:9] [INFO] [SDK_UI] [Gallery] body",
            "2026-09-30 21:55:53.776 R[1:2] [demo][Render][INFO] [View] v",
            "0001 09-21 12:07:30.386 007 009 I Tag: numbered",
            "06-18 19:24:01.123 D/Tag(10123: 2983): uid time",
            "09-21 12:07:30.386 u0_a12: 1234 5678 I Tag: uid",
            "2026-06-18 19:24:01.123  2983-4241  Tag  com.example.app  D  msg",
            "\x1b[38;5;196m09-21 12:07:30.386 +0900  1  2 E Tag: color zone\x1b[0m",
            "         1718709841.123  2983  4241 E Tag: epoch",
            "[ 06-18 19:24:01.123  1000: 2983: 4241 D/Tag      ]",
            "Oct 24 09:00:00 iPhone MyApp(Foundation)[123] <Notice>: syslog",
        ]
        .map(String::from),
    );
    let fields = |raw: &str| [false, true].map(|r| parse_line(raw, 1, r).json(usize::MAX));
    let want: Vec<_> = rows.iter().map(|raw| fields(raw)).collect();
    std::thread::scope(|s| {
        for k in 0..4 {
            let (rows, want) = (&rows, &want);
            s.spawn(move || {
                for i in (0..rows.len()).map(|i| (i * 7 + k * 1009) % rows.len()) {
                    assert_eq!(fields(&rows[i]), want[i], "{:?}", rows[i]);
                }
            });
        }
    });
}

#[test]
fn random_bytes_decode_and_parse() {
    let mut rng = Rng(0xD1B5_4A32_D192_ED03);
    for i in 0..50_000u64 {
        let n = rng.below(48);
        let bytes: Vec<u8> = (0..n)
            .map(|_| match rng.below(4) {
                0 => b"09-21 12:07:30.386 1 2 I T: [x]\r\n\t"[rng.below(31)],
                1 => (0xC0 + rng.below(0x40)) as u8,
                2 => (0x80 + rng.below(0x40)) as u8,
                _ => rng.below(256) as u8,
            })
            .collect();
        let text = encoding::decode(&bytes);
        let _ = encoding::replacement_offsets(&bytes, false);
        let _ = encoding::window(&bytes, 0, 16);
        exercise(&text, i);
        // iOS rows display repaired; marks follow the repaired text.
        let fixed = encoding::decoded(&bytes, true).text;
        let marks = encoding::replacement_offsets(&bytes, true);
        assert!(marks.iter().all(|&at| fixed[at..].starts_with('\u{fffd}')));
        let _ = encoding::damage(&bytes, true, 0, fixed.len(), 0);
        exercise(&fixed, i);
    }
}
