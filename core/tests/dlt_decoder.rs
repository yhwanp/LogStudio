//! The DLT decoder on its own: unit splitting, rows, row fields and the
//! independent `dlt_parse` reading of the same messages.
use logstudio_core::{
    dlt::{
        self,
        synth::{self, id, msin, with_id, Arg, Message},
        Clock, Decoder, Kind, Memo, Unit,
    },
    search::{Condition, Filter, Filters},
    source::Format,
};

fn render(raw: &[u8]) -> String {
    render_at(raw, 0)
}
fn render_at(raw: &[u8], base: u64) -> String {
    let mut out = vec![];
    Decoder::new(Clock::Fixed(0)).render(raw, base, &mut out);
    String::from_utf8(out).unwrap()
}
/// Row of one message, without its LF.
fn row(m: &Message) -> String {
    let text = render(&m.bytes());
    assert_eq!(text.matches('\n').count(), 1, "{text:?}");
    text.trim_end_matches('\n').to_string()
}
/// Payload of one message's row.
fn payload(m: &Message) -> String {
    let r = row(m);
    dlt::parse_row(&r).unwrap().payload.to_string()
}
fn args(a: &[Arg]) -> String {
    let le = payload(&Message::log(4, "APP", "CTX", a));
    let be = payload(&Message::log(4, "APP", "CTX", &[]).big(a));
    assert_eq!(le, be, "byte order must not change the text");
    le
}

#[test]
fn header_fields_and_absent_values() {
    let mut m = Message::log(2, "NAV", "RTE", &[Arg::Ascii(b"x")]);
    assert_eq!(
        row(&m),
        "2026-03-14 09:26:53.589793 1234.5678 0 ECU1 NAV RTE 4242 E x"
    );
    m.count = 255;
    m.timestamp = Some(0);
    m.session = None;
    m.ecu = None;
    m.storage_ecu = id("RECV");
    assert_eq!(
        row(&m),
        "2026-03-14 09:26:53.589793 0.0000 255 RECV NAV RTE - E x"
    );
    // The ECU of the standard header wins over the storage header's.
    m.ecu = Some(id("HU"));
    m.timestamp = None;
    assert_eq!(row(&m), "2026-03-14 09:26:53.589793 - 255 HU NAV RTE - E x");
    // Microseconds of a second or more carry into the seconds.
    m.micros = 1_500_000;
    assert!(row(&m).starts_with("2026-03-14 09:26:54.500000 "));
    // Local time is the storage time plus the clock's offset.
    let mut out = vec![];
    Decoder::new(Clock::Fixed(9 * 3600)).render(
        &Message::log(4, "A", "C", &[]).bytes(),
        0,
        &mut out,
    );
    assert!(String::from_utf8(out)
        .unwrap()
        .starts_with("2026-03-14 18:26:53.589793 "));
}
#[test]
fn ids_are_cleaned_to_one_word() {
    for (raw, shown) in [
        (*b"AB\0\0", "AB"),
        (*b"AB  ", "AB"),
        (*b"A B\0", "A_B"),
        (*b"\0\0\0\0", "-"),
        (*b"A\0BC", "A"),
        ([0xc3, 0xa9, b'x', 0], "__x"),
        (*b"ABCD", "ABCD"),
    ] {
        let mut m = Message::log(4, "", "", &[]);
        m.ext = Some((msin(true, 0, 4), 0, raw, *b"CTX\0"));
        let r = row(&m);
        assert_eq!(dlt::parse_row(&r).unwrap().apid, shown, "{r}");
    }
}
#[test]
fn kinds_and_levels() {
    for (mstp, mtin, kind, level) in [
        (0, 1, "F", "F"),
        (0, 2, "E", "E"),
        (0, 3, "W", "W"),
        (0, 4, "I", "I"),
        (0, 5, "D", "D"),
        (0, 6, "V", "V"),
        (0, 0, "log:0", "U"),
        (0, 9, "log:9", "U"),
        (1, 1, "app_trace:variable", "U"),
        (2, 2, "nw_trace:can", "U"),
        (3, 1, "control:request", "U"),
        (3, 2, "control:response", "U"),
        (5, 2, "5:2", "U"),
    ] {
        let mut m = Message::log(4, "A", "C", &[]);
        m.ext = Some((msin(mstp != 3, mstp, mtin), 0, id("A"), id("C")));
        if mstp == 3 {
            m.payload = with_id(false, 3, &[]);
        }
        let r = row(&m);
        let f = dlt::parse_row(&r).unwrap();
        assert_eq!((f.kind, f.level), (kind, level), "{r}");
    }
    // Without an extended header: no IDs, no type, non-verbose payload.
    let m = Message {
        payload: with_id(false, 305_419_896, b"AB\0"),
        ..Message::default()
    };
    assert_eq!(
        row(&m),
        "2026-03-14 09:26:53.589793 1234.5678 0 ECU1 - - 4242 - [305419896] AB-|41 42 00"
    );
    let r = row(&m);
    let f = dlt::parse_row(&r).unwrap();
    assert_eq!((f.apid, f.ctid, f.kind, f.level), ("-", "-", "-", "U"));
}
#[test]
fn argument_types_render_as_dlt_viewer_shows_them() {
    assert_eq!(args(&[Arg::Bool(true), Arg::Bool(false)]), "true false");
    assert_eq!(
        args(&[Arg::Sint(-5, 1), Arg::Sint(-300, 2), Arg::Sint(-70_000, 4)]),
        "-5 -300 -70000"
    );
    assert_eq!(
        args(&[Arg::Sint(i64::MIN, 8), Arg::Uint(u64::MAX, 8)]),
        "-9223372036854775808 18446744073709551615"
    );
    assert_eq!(
        args(&[Arg::Uint(255, 1), Arg::Uint(65535, 2), Arg::Uint(7, 4)]),
        "255 65535 7"
    );
    assert_eq!(
        args(&[
            Arg::Hex(0xab, 1),
            Arg::Hex(0x1234, 2),
            Arg::Hex(0xdead_beef, 4)
        ]),
        "0xab 0x1234 0xdeadbeef"
    );
    assert_eq!(args(&[Arg::Hex(1, 8)]), "0x0000000000000001");
    assert_eq!(args(&[Arg::Bin(0x5a, 1)]), "0b0101 1010");
    assert_eq!(
        args(&[Arg::F32(1.5), Arg::F64(-0.1), Arg::F64(1e21)]),
        "1.50000000 -0.10000000 1000000000000000000000.00000000"
    );
    assert_eq!(
        args(&[
            Arg::F64(f64::NAN),
            Arg::F64(f64::INFINITY),
            Arg::F64(f64::NEG_INFINITY)
        ]),
        "nan inf -inf"
    );
    assert_eq!(
        args(&[Arg::Utf8("한글 🙂"), Arg::Ascii(b"plain")]),
        "한글 🙂 plain"
    );
    assert_eq!(args(&[Arg::Raw(&[0xde, 0xad, 0x00, 0x0f])]), "de ad 00 0f");
    assert_eq!(args(&[Arg::Raw(&[])]), "");
    // VARI names and units are read past and not shown.
    assert_eq!(
        args(&[
            Arg::Named("speed", "km/h", &Arg::Uint(3, 4)),
            Arg::Named("label", "", &Arg::Ascii(b"x")),
            Arg::Named("on", "", &Arg::Bool(true)),
            Arg::Named("r", "", &Arg::Raw(&[1]))
        ]),
        "3 x true 01"
    );
    // A non-zero boolean byte is true.
    assert_eq!(args(&[Arg::Other(0x11, &[7])]), "true");
    // With several type bits, dlt-viewer's precedence: string first.
    let both = Message::log(
        4,
        "A",
        "C",
        &[Arg::Other(0x200 | 0x10 | 1, &[2, 0, b'x', 0])],
    );
    assert_eq!(payload(&both), "x");
}
#[test]
fn unsupported_values_and_corrupted_arguments() {
    // f16, f128, 128-bit integers and trace info print `?`.
    assert_eq!(args(&[Arg::Other(0x80 | 2, &[0, 0])]), "?");
    assert_eq!(args(&[Arg::Other(0x80 | 5, &[0; 16])]), "?");
    assert_eq!(
        args(&[
            Arg::Other(0x40 | 5, &[1; 16]),
            Arg::Other(0x20 | 5, &[1; 16])
        ]),
        "? ?"
    );
    let le = payload(&Message::log(
        4,
        "A",
        "C",
        &[Arg::Other(0x2000, &[2, 0, b'x', 0])],
    ));
    assert_eq!(le, "?");
    // Arrays, structures, fixed point, a missing type or a short value
    // corrupt the whole payload.
    for a in [
        Arg::Other(0x100 | 0x40 | 3, &[1, 0, 1, 0, 1, 0, 0, 0]),
        Arg::Other(0x4000, &[0, 0]),
        Arg::Other(0x1000 | 0x40 | 3, &[0; 12]),
        Arg::Other(0, &[]),
        Arg::Other(0x40, &[1]),
        Arg::Other(0x40 | 3, &[1, 2]),
        Arg::Other(0x200, &[9, 0, b'a']),
    ] {
        let m = Message::log(4, "A", "C", &[Arg::Ascii(b"before"), a]);
        assert_eq!(payload(&m), "!!CORRUPTED MESSAGE!!");
    }
    // NOAR larger than the arguments present.
    let mut m = Message::log(4, "A", "C", &[Arg::Ascii(b"one")]);
    m.ext = m.ext.map(|(i, _, a, c)| (i, 2, a, c));
    assert_eq!(payload(&m), "!!CORRUPTED MESSAGE!!");
    // Bytes after the last argument are ignored.
    let mut m = Message::log(4, "A", "C", &[Arg::Ascii(b"one")]);
    m.payload.extend_from_slice(b"trailing");
    assert_eq!(payload(&m), "one");
}
#[test]
fn payload_whitespace_and_text() {
    assert_eq!(args(&[Arg::Ascii(b"  a\tb\r\nc  d  ")]), "a b c d");
    assert_eq!(
        args(&[Arg::Ascii(b"a"), Arg::Ascii(b"   "), Arg::Ascii(b"b")]),
        "a b"
    );
    assert_eq!(args(&[Arg::Ascii(b""), Arg::Ascii(b"x")]), "x");
    assert_eq!(args(&[Arg::Utf8("a\u{2028}b\u{a0}c\u{3000}d")]), "a b c d");
    // NUL inside a string goes; invalid UTF-8 becomes U+FFFD, as fromUtf8.
    assert_eq!(
        args(&[Arg::Ascii(b"a\0b"), Arg::Ascii(b"caf\xe9")]),
        "ab caf\u{fffd}"
    );
    // A string of only whitespace leaves no payload and no trailing space.
    let r = row(&Message::log(4, "A", "C", &[Arg::Ascii(b" \n ")]));
    assert!(r.ends_with(" 4242 I"), "{r:?}");
}
#[test]
fn non_verbose_and_control_payloads() {
    let nv = |big: bool, data: Vec<u8>| {
        let mut m = Message::log(4, "A", "C", &[]);
        m.ext = Some((msin(false, 0, 4), 0, id("A"), id("C")));
        m.big_endian = big;
        m.payload = data;
        payload(&m)
    };
    assert_eq!(nv(false, with_id(false, 7, b"x y")), "[7] x y|78 20 79");
    assert_eq!(nv(true, with_id(true, 7, b"\x01")), "[7] -|01");
    assert_eq!(nv(false, with_id(false, 7, b"")), "[7] |");
    assert_eq!(nv(false, vec![1, 2]), "[?] --|01 02");
    // Only the first 256 data bytes are shown.
    let long = nv(false, with_id(false, 1, &[b'a'; 300]));
    assert_eq!(long.matches("61").count(), 256);
    let control = |mtin: u8, data: Vec<u8>| {
        let mut m = Message::log(4, "DA1", "DC1", &[]);
        m.ext = Some((msin(false, 3, mtin), 0, id("DA1"), id("DC1")));
        m.payload = data;
        payload(&m)
    };
    assert_eq!(
        control(1, with_id(false, 3, &[0x41, 0x42])),
        "[get_log_info] 41 42"
    );
    assert_eq!(
        control(2, with_id(false, 3, &[0, 7, 1])),
        "[get_log_info ok] 07 01"
    );
    assert_eq!(control(2, with_id(false, 3, &[3])), "[get_log_info 3]");
    assert_eq!(
        control(2, with_id(false, 0xf02, &[8])),
        "[connection_info no_matching_context_id]"
    );
    assert_eq!(
        control(2, with_id(false, 0x99, &[1])),
        "[153 not_supported]"
    );
    assert_eq!(control(2, with_id(false, 0xf04, &[0])), "MARKER");
    let mut version = vec![0];
    version.extend_from_slice(&4u32.to_le_bytes());
    version.extend_from_slice(b"v1.2 \xe9");
    assert_eq!(
        control(2, with_id(false, 0x13, &version)),
        "[get_software_version ok] v1.2 é"
    );
    assert_eq!(control(1, vec![1]), "[?] 01");
}

#[test]
fn units_of_damaged_files() {
    let a = Message::log(4, "A", "C", &[Arg::Ascii(b"first")]).bytes();
    let b = Message::log(3, "B", "C", &[Arg::Ascii(b"second")]).bytes();
    let kinds = |raw: &[u8]| -> Vec<(usize, usize, Kind)> {
        dlt::units(raw, raw.len())
            .map(|u| (u.start, u.len, u.kind))
            .collect()
    };
    let cat = |parts: &[&[u8]]| parts.concat();
    // Garbage before, between and after messages.
    let raw = cat(&[b"xyz", &a, b"\0\0", &b, b"tail"]);
    assert_eq!(
        kinds(&raw),
        [
            (0, 3, Kind::Garbage),
            (3, a.len(), Kind::Message),
            (3 + a.len(), 2, Kind::Garbage),
            (5 + a.len(), b.len(), Kind::Message),
            (5 + a.len() + b.len(), 4, Kind::Garbage)
        ]
    );
    // A message cut short by the end of the file.
    let raw = cat(&[&a, &b[..b.len() - 3]]);
    assert_eq!(kinds(&raw)[1], (a.len(), b.len() - 3, Kind::Garbage));
    assert_eq!(dlt::truncated(&raw[a.len()..]), Some(b.len()));
    let text = render(&raw);
    assert!(
        text.lines().nth(1).unwrap().starts_with(&format!(
            "!! 잘린 DLT 메시지: {}바이트 중 {}바이트 · 파일 위치 {}: 44 4c 54 01",
            b.len(),
            b.len() - 3,
            a.len()
        )),
        "{text}"
    );
    // A pattern inside a payload is not a message start.
    let inner = Message::log(4, "A", "C", &[Arg::Raw(&b)]).bytes();
    assert_eq!(kinds(&inner), [(0, inner.len(), Kind::Message)]);
    // A header length too small for its flags, and an unknown version.
    let mut bad = a.clone();
    bad[18..20].copy_from_slice(&5u16.to_be_bytes());
    assert_eq!(kinds(&cat(&[&bad, &b]))[0].2, Kind::Garbage);
    let mut v7 = a.clone();
    v7[16] |= 0xe0;
    assert_eq!(
        kinds(&cat(&[&v7, &b])),
        [
            (0, a.len(), Kind::Garbage),
            (a.len(), b.len(), Kind::Message)
        ]
    );
    // Version 0 is read as version 1.
    let mut v0 = a.clone();
    v0[16] &= 0x1f;
    assert_eq!(kinds(&v0), [(0, a.len(), Kind::Message)]);
    // Version 2 data up to the next message.
    let raw = cat(&[b"DLT\x02abcdef", &b"DLT\x02gh"[..], &a]);
    assert_eq!(
        kinds(&raw),
        [
            (0, 10, Kind::Unsupported),
            (10, 6, Kind::Unsupported),
            (16, a.len(), Kind::Message)
        ]
    );
    assert!(render(&raw)
        .starts_with("!! 지원하지 않는 DLT 버전 2 데이터 10바이트 · 파일 위치 0: 44 4c 54 02 61"));
    // A long run of other bytes is cut into units of at most the cap.
    let raw = cat(&[&vec![0x55; dlt::GARBAGE_CAP * 2 + 10], &a]);
    assert_eq!(
        kinds(&raw),
        [
            (0, dlt::GARBAGE_CAP, Kind::Garbage),
            (dlt::GARBAGE_CAP, dlt::GARBAGE_CAP, Kind::Garbage),
            (2 * dlt::GARBAGE_CAP, 10, Kind::Garbage),
            (2 * dlt::GARBAGE_CAP + 10, a.len(), Kind::Message)
        ]
    );
    // While resynchronizing, a start needs plausible microseconds; at a unit
    // start a valid header is enough.
    let mut odd = b.clone();
    odd[8..12].copy_from_slice(&2_000_000u32.to_le_bytes());
    assert_eq!(
        kinds(&cat(&[b"zz", &odd, &a]))[0],
        (0, 2 + odd.len(), Kind::Garbage)
    );
    assert_eq!(kinds(&cat(&[&odd, &a]))[0], (0, odd.len(), Kind::Message));
    // A file of other bytes; an empty one.
    assert!(render(b"hello")
        .starts_with("!! DLT 메시지가 아닌 데이터 5바이트 · 파일 위치 0: 68 65 6c 6c 6f\n"));
    assert_eq!(
        render_at(b"x", 1000),
        "!! DLT 메시지가 아닌 데이터 1바이트 · 파일 위치 1000: 78\n"
    );
    assert_eq!(render(b""), "");
}
#[test]
fn first_message_time() {
    let mut m = Message::log(4, "A", "C", &[]);
    let raw = [b"DLT\x01x".as_slice(), &m.bytes()].concat();
    assert_eq!(dlt::first_time(&raw), Some((1_773_480_413, 589_793)));
    m.micros = 2_000_001;
    assert_eq!(dlt::first_time(&m.bytes()), Some((1_773_480_415, 1)));
    assert_eq!(dlt::first_time(b"09-21 12:07:30.386 1 2 I T: x"), None);
    assert_eq!(dlt::first_time(&m.bytes()[..30]), None);
    assert!(dlt::is_dlt(b"DLT\x02") && dlt::is_dlt(b"DLT\x01") && !dlt::is_dlt(b"DLT"));
}

/// xorshift64*: seeded and dependency-free.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }
    fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.next() as u8).collect()
    }
    /// Fewer than `n` random bytes.
    fn some(&mut self, n: usize) -> Vec<u8> {
        let k = self.below(n);
        self.bytes(k)
    }
}
const WORDS: [&str; 8] = [
    "route",
    "한글",
    "🙂",
    "a b",
    "tab\there",
    "line\nbreak",
    "  ",
    "x",
];
/// A random message; `strict` keeps to what the oracle reads the same way
/// (little-endian, valid UTF-8 strings, no hex/bin display, no corrupt
/// arguments).
fn random_message(r: &mut Rng, strict: bool) -> Message {
    let texts: Vec<String> = (0..4)
        .map(|_| {
            (0..r.below(4))
                .map(|_| WORDS[r.below(WORDS.len())])
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect();
    let raw = r.some(40);
    let plain: Vec<Arg> = (0..r.below(5))
        .map(|k| match r.below(if strict { 9 } else { 12 }) {
            0 => Arg::Bool(r.chance(50)),
            1 => Arg::Sint(r.next() as i64 >> r.below(64), [1, 2, 4, 8][r.below(4)]),
            2 => Arg::Uint(r.next() >> r.below(64), [1, 2, 4, 8][r.below(4)]),
            3 => Arg::F32(r.next() as u32 as f32 / 7.0),
            4 => Arg::F64(r.next() as f64 / 3.0 - 1e18),
            5 => Arg::Ascii(texts[k % 4].as_bytes()),
            6 | 8 => Arg::Utf8(&texts[(k + 1) % 4]),
            7 => Arg::Raw(&raw),
            9 => Arg::Hex(r.next(), [1, 2, 4, 8][r.below(4)]),
            10 => Arg::Bin(r.next(), 1),
            _ => Arg::Other(0x100 | 0x40 | 3, &[]),
        })
        .collect();
    let args: Vec<Arg> = plain
        .iter()
        .map(|a| match a {
            Arg::Other(..) => *a,
            _ if r.chance(25) => Arg::Named("name", "unit", a),
            _ => *a,
        })
        .collect();
    let mut m = Message::log(
        1 + r.below(6) as u8,
        ["NAV", "AUD", "", "A B"][r.below(4)],
        ["RTE", "MAIN", "CTX1"][r.below(3)],
        &args,
    );
    // dlt_parse reads type info words as little-endian whatever MSBF says;
    // dlt-daemon and dlt-viewer read them in the message's byte order.
    if !strict && r.chance(30) {
        m = m.big(&args);
    }
    m.seconds = 1_700_000_000 + r.below(100_000_000) as u32;
    m.micros = r.below(1_000_000) as u32;
    m.count = r.next() as u8;
    m.ecu = r.chance(70).then(|| id(["ECU1", "HU", "TCU"][r.below(3)]));
    m.session = r.chance(70).then(|| r.next() as u32);
    m.timestamp = r.chance(70).then(|| r.next() as u32);
    if !strict && r.chance(10) {
        // Non-verbose or control, or no extended header.
        let (_, _, a, c) = m.ext.unwrap();
        let mstp = [0, 3][r.below(2)];
        m.ext = r
            .chance(80)
            .then_some((msin(false, mstp, 1 + r.below(2) as u8), 0, a, c));
        m.payload = with_id(m.big_endian, r.next() as u32 % 30, &r.some(20));
    }
    m
}
/// A stream of messages and damage, as a damaged or still-written file.
fn random_stream(r: &mut Rng, n: usize, long_runs: bool) -> Vec<u8> {
    let mut out = vec![];
    for _ in 0..n {
        match r.below(100) {
            0..=79 => random_message(r, false).write(&mut out),
            80..=86 => out.extend({
                let mut v = r.some(300);
                v.push(0x33);
                v
            }),
            87..=89 => {
                // A message cut short, then more.
                let m = random_message(r, false).bytes();
                out.extend_from_slice(&m[..r.below(m.len())]);
            }
            90..=92 => {
                // A bare pattern or a pattern with a plausible-looking header.
                out.extend_from_slice(&dlt::MAGIC_V1);
                out.extend(r.some(30));
            }
            93..=94 => {
                out.extend_from_slice(&dlt::MAGIC_V2);
                out.extend(r.some(50));
            }
            95 if long_runs => {
                out.extend(std::iter::repeat_n(0u8, dlt::GARBAGE_CAP + r.below(1000)))
            }
            _ => {
                // One flipped byte inside a message.
                let mut m = random_message(r, false).bytes();
                let i = r.below(m.len());
                m[i] ^= 1 << r.below(8);
                out.extend(m);
            }
        }
    }
    if r.chance(50) {
        let m = random_message(r, false).bytes();
        out.extend_from_slice(&m[..r.below(m.len())]);
    }
    out
}
/// Units as an indexer finds them: reading `chunk`-sized pieces and keeping
/// the bytes of undecided units for the next read.
fn units_by_chunks(all: &[u8], r: &mut Rng, max_chunk: usize) -> Vec<Unit> {
    let (mut got, mut tail, mut base, mut at) = (vec![], Vec::new(), 0, 0);
    loop {
        let n = (1 + r.below(max_chunk)).min(all.len() - at);
        tail.extend_from_slice(&all[at..at + n]);
        at += n;
        let mut it = dlt::units(&tail, all.len() - base);
        for u in &mut it {
            got.push(Unit {
                start: base + u.start,
                ..u
            });
        }
        let used = it.position();
        tail.drain(..used);
        base += used;
        if at == all.len() {
            assert!(tail.is_empty(), "every byte is decided at the end");
            return got;
        }
    }
}
#[test]
fn units_do_not_depend_on_reads_or_blocks() {
    let mut r = Rng(0x9e37_79b9_7f4a_7c15);
    for round in 0..300 {
        // Reads of a few bytes rescan undecided bytes often; keep them to
        // files without long runs.
        let long_runs = round % 3 != 0;
        let pieces = 1 + r.below(60);
        let all = random_stream(&mut r, pieces, long_runs);
        let one: Vec<Unit> = dlt::units(&all, all.len()).collect();
        // Units cover the file in order.
        let mut end = 0;
        for u in &one {
            assert_eq!(u.start, end);
            assert!(u.len > 0 && u.len <= dlt::MAX_MESSAGE.max(dlt::GARBAGE_CAP));
            end += u.len;
        }
        assert_eq!(end, all.len(), "round {round}");
        let chunks: &[usize] = if long_runs {
            &[4096, 1 << 17, 1 << 20]
        } else {
            &[1, 7, 4096]
        };
        for &max_chunk in chunks {
            assert_eq!(
                units_by_chunks(&all, &mut r, max_chunk),
                one,
                "round {round} chunk {max_chunk}"
            );
        }
        // Any run of whole units splits again into the same units, and
        // renders to the same rows as the whole file.
        let whole = render(&all);
        let mut joined = String::new();
        let mut k = 0;
        while k < one.len() {
            let take = 1 + r.below(8);
            let block = &one[k..(k + take).min(one.len())];
            let (s, e) = (
                block[0].start,
                block.last().map(|u| u.start + u.len).unwrap(),
            );
            let again: Vec<Unit> = dlt::units(&all[s..e], e - s)
                .map(|u| Unit {
                    start: s + u.start,
                    ..u
                })
                .collect();
            assert_eq!(again, block, "round {round} block at {s}");
            joined.push_str(&render_at(&all[s..e], s as u64));
            k += take;
        }
        assert_eq!(joined, whole, "round {round}");
    }
}
#[test]
fn rows_are_one_line_each_and_parse_back() {
    let mut r = Rng(42);
    for _ in 0..50 {
        let all = random_stream(&mut r, 200, true);
        let mut out = vec![];
        let starts = Decoder::new(Clock::Fixed(0)).render(&all, 0, &mut out);
        let text = String::from_utf8(out).expect("rows are UTF-8");
        assert!(!text.contains('\0') && !text.contains('\r'));
        let units: Vec<Unit> = dlt::units(&all, all.len()).collect();
        assert_eq!(starts.len(), units.len() + 1);
        assert_eq!(*starts.last().unwrap() as usize, all.len());
        assert_eq!(text.matches('\n').count(), units.len());
        for (k, (u, line)) in units.iter().zip(text.split_terminator('\n')).enumerate() {
            assert!(line.len() <= 1 << 20);
            assert_eq!(starts[k] as usize, u.start);
            let parsed = dlt::parse_row(line);
            if u.kind == Kind::Message {
                let p = parsed.unwrap_or_else(|| panic!("{line}"));
                assert!(
                    !p.payload.starts_with(' ')
                        && !p.payload.ends_with(' ')
                        && !p.payload.contains("  "),
                    "{line:?}"
                );
            } else {
                assert!(line.starts_with(dlt::ISSUE) && parsed.is_none(), "{line}");
            }
        }
    }
    // Arbitrary text never panics.
    for _ in 0..20_000 {
        let n = r.below(80);
        let s: String = (0..n)
            .map(|_| {
                [
                    "2026-03-14 09:26:53.589793 ",
                    "-",
                    " ",
                    "1",
                    ".",
                    "A",
                    "W",
                    ":",
                    "한",
                ][r.below(9)]
            })
            .collect();
        let _ = dlt::parse_row(&s);
    }
}
#[test]
fn rendering_is_the_same_on_every_thread() {
    let mut r = Rng(7);
    let all = random_stream(&mut r, 2000, true);
    let d = std::sync::Arc::new(Decoder::new(Clock::local()));
    let mut first = vec![];
    d.render(&all, 0, &mut first);
    let threads: Vec<_> = (0..4)
        .map(|_| {
            let (d, all) = (d.clone(), all.clone());
            std::thread::spawn(move || {
                let mut out = vec![];
                d.render(&all, 0, &mut out);
                out
            })
        })
        .collect();
    for t in threads {
        assert_eq!(t.join().unwrap(), first);
    }
}
#[test]
fn clock_memo_keeps_offsets_and_splits_changing_hours() {
    let calls = std::cell::Cell::new(0);
    // A zone that moves from +1 h to +2 h at 10:30 UTC on day 0.
    let change = 10 * 3600 + 1800;
    let zone = |t: i64| {
        calls.set(calls.get() + 1);
        if t < change {
            3600
        } else {
            7200
        }
    };
    let m = Memo::default();
    assert_eq!(m.offset(5 * 3600, zone), 3600);
    let after_first = calls.get();
    assert_eq!(m.offset(5 * 3600 + 10, zone), 3600);
    assert_eq!(calls.get(), after_first, "a uniform hour is kept");
    assert_eq!(m.offset(change - 1, zone), 3600);
    assert_eq!(m.offset(change, zone), 7200);
    let n = calls.get();
    assert_eq!(m.offset(change, zone), 7200);
    assert_eq!(calls.get(), n, "seconds of a changing hour are kept");
    // A later zone change does not alter offsets already given.
    assert_eq!(m.offset(5 * 3600 + 20, |_| 0), 3600);
    assert_eq!(Clock::Fixed(-3600).offset(0), -3600);
    let _ = Clock::local().offset(1_773_480_413);
}

#[test]
fn dlt_parse_reads_the_same_messages() {
    use dlt_parse::{storage::DltStorageReader, verbose::VerboseValue as V};
    let mut r = Rng(0x5eed);
    let messages: Vec<Message> = (0..3000).map(|_| random_message(&mut r, true)).collect();
    let mut all = vec![];
    for m in &messages {
        m.write(&mut all);
    }
    let ours: Vec<Unit> = dlt::units(&all, all.len()).collect();
    assert!(ours.iter().all(|u| u.kind == Kind::Message));
    let mut reader = DltStorageReader::new(&all[..]);
    let mut n = 0;
    while let Some(slice) = reader.next_packet() {
        let s = slice.expect("valid message");
        let u = ours[n];
        let h = dlt::header(&all[u.start..u.start + u.len]).unwrap();
        let theirs = s.packet.header();
        assert_eq!(
            (
                s.storage_header.timestamp_seconds,
                s.storage_header.timestamp_microseconds,
                s.storage_header.ecu_id
            ),
            (h.seconds, h.micros, h.storage_ecu)
        );
        assert_eq!(theirs.message_counter, h.count);
        assert_eq!(theirs.length, h.len);
        assert_eq!(theirs.is_big_endian, h.big_endian());
        assert_eq!(
            (theirs.ecu_id, theirs.session_id, theirs.timestamp),
            (h.ecu, h.session, h.timestamp)
        );
        let ext = theirs.extended_header.as_ref().unwrap();
        let e = h.ext.unwrap();
        assert_eq!(
            (
                ext.message_info.0,
                ext.number_of_arguments,
                ext.application_id,
                ext.context_id
            ),
            (e.msin, e.noar, e.apid, e.ctid)
        );
        assert_eq!(s.packet.payload(), h.payload);
        // Their values, shown with the same rules, give our payload.
        let mut words = vec![];
        for v in s.packet.verbose_value_iter().unwrap() {
            words.push(match v.expect("valid argument") {
                V::Bool(b) => b.value.to_string(),
                V::Str(s) => s.value.to_string(),
                V::I8(v) => v.value.to_string(),
                V::I16(v) => v.value.to_string(),
                V::I32(v) => v.value.to_string(),
                V::I64(v) => v.value.to_string(),
                V::U8(v) => v.value.to_string(),
                V::U16(v) => v.value.to_string(),
                V::U32(v) => v.value.to_string(),
                V::U64(v) => v.value.to_string(),
                V::F32(v) => format!("{:.8}", v.value as f64),
                V::F64(v) => format!("{:.8}", v.value),
                V::Raw(v) => v
                    .data
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<Vec<_>>()
                    .join(" "),
                other => panic!("unexpected {other:?}"),
            });
        }
        let expected = words
            .join(" ")
            .replace('\0', "")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(
            dlt::parse_row(&row(&messages[n])).unwrap().payload,
            expected,
            "message {n}"
        );
        n += 1;
    }
    assert_eq!(n, ours.len());
    let _ = synth::verbose(false, &[]);
}

#[test]
fn headerless_messages_are_recognised() {
    let messages: Vec<Vec<u8>> = (0..5)
        .map(|i| {
            let text = format!("message {i}");
            Message::log(4, "APP", "CTX", &[Arg::Ascii(text.as_bytes())]).bytes()[dlt::STORAGE..]
                .to_vec()
        })
        .collect();
    let bare = messages.concat();
    let size = bare.len() as u64;
    assert!(dlt::headerless(&bare, size));
    // One or two messages that are the whole file.
    assert!(dlt::headerless(&messages[0], messages[0].len() as u64));
    let two = messages[..2].concat();
    assert!(dlt::headerless(&two, two.len() as u64));
    // Three messages are enough even when the head stops there.
    let three = messages[..3].concat();
    assert!(dlt::headerless(&three, size));
    // A length past the file's end, or a chain into other bytes, is not.
    assert!(!dlt::headerless(&messages[0], messages[0].len() as u64 - 1));
    let broken = [&messages[0][..], b"\0\0\0\0garbage bytes"].concat();
    assert!(!dlt::headerless(&broken, broken.len() as u64));
    // Text with NUL bytes and random bytes are not.
    for text in [
        &b"hello\0world\n"[..],
        "h\0e\0l\0l\0o\0".as_bytes(),
        b"\0\0\0\0",
    ] {
        assert!(!dlt::headerless(text, text.len() as u64), "{text:?}");
    }
    let mut r = Rng(0xbad5eed);
    for _ in 0..500 {
        let n = 1 + r.below(8192);
        let bytes = r.bytes(n);
        assert!(!dlt::headerless(&bytes, bytes.len() as u64), "{bytes:?}");
    }
    // A raw version 2 message of either length byte order: dlt-viewer's
    // sample is one big-endian message.
    let viewer = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../tests/fixtures/covesa/dlt-viewer-testfile.dlt"
    ))
    .unwrap();
    assert!(dlt::headerless(&viewer, viewer.len() as u64));
    let mut little = viewer.clone();
    little.swap(5, 6);
    assert!(dlt::headerless(&little, little.len() as u64));
    little[0] = 0x6c; // version 3
    assert!(!dlt::headerless(&little, little.len() as u64));
}

/// IDs as files may hold them: ordinary and lowercase, padded with NUL or
/// spaces, a lone `-`, empty, with a tab or space inside, and non-ASCII.
const ODD_IDS: [[u8; 4]; 12] = [
    *b"NAV\0",
    *b"nav\0",
    *b"ECU1",
    *b"HU  ",
    *b"-\0\0\0",
    [0; 4],
    *b"A B\0",
    *b"N\tV\0",
    [0xc3, 0xa9, b'K', 0],
    *b"SK\0\0",
    *b"CTX1",
    *b"   \0",
];
/// Tag filter items: IDs, substrings, other cases, `_` for replaced bytes,
/// `-`, and characters whose Unicode case folding meets ASCII (ſ is s,
/// K is k).
const NEEDLES: [&str; 14] = [
    "NAV", "nav", "av", "ECU", "1", "_", "a_b", "-", "U", "ctx", "ſ", "\u{212a}", "zz", "A",
];

fn random_filters(r: &mut Rng, rows: &[String]) -> Filters {
    let list = |r: &mut Rng| {
        (0..1 + r.below(3))
            .map(|_| NEEDLES[r.below(NEEDLES.len())])
            .collect::<Vec<_>>()
            .join(", ")
    };
    // A bound near a row's own time, at some precision, maybe without its
    // year or date.
    let bound = |r: &mut Rng| {
        let row = rows
            .iter()
            .filter(|row| dlt::parse_row(row).is_some())
            .nth(r.below(rows.len().max(1)));
        let Some(row) = row else {
            return String::new();
        };
        let text = &row[..[13, 16, 19, 23, 26][r.below(5)]];
        match r.below(4) {
            0 => text[5..].to_string(),
            1 => text[11..].to_string(),
            _ => text.to_string(),
        }
    };
    let mut f = Filters {
        include_raw: r.chance(50),
        recognize_line_numbers: [None, Some(true), Some(false)][r.below(3)],
        ..Filters::default()
    };
    if r.chance(40) {
        f.levels.retain(|_| r.chance(60));
    }
    if r.chance(50) {
        f.tag = list(r);
    }
    if r.chance(40) {
        f.exclude_tags = list(r);
    }
    if r.chance(40) {
        f.time_from = bound(r);
    }
    if r.chance(40) {
        f.time_to = bound(r);
    }
    if r.chance(10) {
        f.pid = "42".into();
    }
    if r.chance(10) {
        f.tid = "7".into();
    }
    // More showing and hiding conditions, each with its own levels, tags,
    // time and IDs.
    let condition = |r: &mut Rng| {
        let mut c = Condition::default();
        if r.chance(40) {
            c.levels.retain(|_| r.chance(60));
        }
        if r.chance(60) {
            c.tag = list(r);
        }
        if r.chance(25) {
            c.time_from = bound(r);
        }
        if r.chance(25) {
            c.time_to = bound(r);
        }
        if r.chance(5) {
            c.pid = "42".into();
        }
        c
    };
    if r.chance(40) {
        f.include = (0..1 + r.below(3)).map(|_| condition(r)).collect();
    }
    if r.chance(40) {
        f.exclude = (0..1 + r.below(2)).map(|_| condition(r)).collect();
    }
    f
}

#[test]
fn header_masks_equal_rendered_row_masks() {
    use std::sync::atomic::AtomicBool;
    let stop = AtomicBool::new(false);
    let mut r = Rng(0x0dd_1d5);
    let (mut decided, mut kept) = (0, 0);
    for round in 0..400 {
        let mut raw = random_stream(&mut r, 40, false);
        // Messages with odd IDs, carried microseconds and every message
        // type and subtype.
        for _ in 0..20 {
            let mut m = random_message(&mut r, false);
            m.storage_ecu = ODD_IDS[r.below(ODD_IDS.len())];
            m.ecu = r.chance(50).then(|| ODD_IDS[r.below(ODD_IDS.len())]);
            if let Some((_, noar, _, _)) = m.ext {
                m.ext = Some((
                    msin(r.chance(50), r.below(8) as u8, r.below(16) as u8),
                    noar,
                    ODD_IDS[r.below(ODD_IDS.len())],
                    ODD_IDS[r.below(ODD_IDS.len())],
                ));
            }
            if r.chance(15) {
                m.micros = 1_000_000 + r.below(4_000_000) as u32;
            }
            m.write(&mut raw);
        }
        let decoder = Decoder::new(match round % 4 {
            0 => Clock::Fixed(0),
            1 => Clock::Fixed(9 * 3600),
            2 => Clock::Fixed(-(5 * 3600 + 1800)),
            _ => Clock::local(),
        });
        let mut text = vec![];
        decoder.render(&raw, 0, &mut text);
        let rows: Vec<String> = String::from_utf8(text.clone())
            .unwrap()
            .split_terminator('\n')
            .map(String::from)
            .collect();
        let n = rows.len();
        assert_eq!(n, dlt::units(&raw, raw.len()).count());
        for _ in 0..8 {
            let filters = random_filters(&mut r, &rows);
            let filter = Filter::new(filters.clone())
                .unwrap()
                .with_format(Format::Dlt);
            let rendered = filter
                .mask_from(&text, n, &stop, filter.admits_unparsed(), false)
                .unwrap();
            let headers = filter.dlt_mask(&decoder, &raw, n, &stop).unwrap();
            let bit = |bits: &[u8], i: usize| bits[i / 8] & (1 << (i % 8)) != 0;
            for (i, row) in rows.iter().enumerate() {
                assert_eq!(
                    bit(&headers.bits, i),
                    bit(&rendered.bits, i),
                    "row {i} {row:?} with {filters:?}"
                );
            }
            assert_eq!(
                (headers.lead, headers.tail),
                (rendered.lead, rendered.tail),
                "{filters:?}"
            );
            decided += 1;
            kept += (0..n).filter(|&i| bit(&rendered.bits, i)).count();
        }
    }
    // The filters keep some rows and drop others.
    assert!(decided == 3200 && kept > 10_000, "{kept}");
}
