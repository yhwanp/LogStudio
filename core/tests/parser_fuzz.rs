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
    match rng.below(6) {
        0 => s.push_str("09-21 12:07:30.386 "),
        1 => s.push_str("2024-09-21 12:07:30.386 "),
        2 => s.push_str("12 09-21 12:07:30.386 "),
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
