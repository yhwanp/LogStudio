//! COVESA/AUTOSAR DLT files (protocol version 1, each message after a
//! `DLT\x01` storage header) as text rows: one LF-terminated row per unit.
//!
//! A unit is a message, a run of bytes that is not one, or a version 2
//! message, decided by `units` from bytes alone. A run of whole units splits
//! the same way on its own, so a block of them renders without the rest of
//! the file, and the text of a row never depends on how the file was read.
//!
//! Message row: `DATE TIME UPTIME COUNT ECU APID CTID SESSION KIND [PAYLOAD]`
//! with `-` for absent fields. KIND is the log level letter (`F E W I D V`)
//! or `type:subtype`. The payload follows dlt-viewer: arguments joined by a
//! space, whitespace runs collapsed, NUL removed. Other units render as
//! `!! ` rows that say what the bytes are and where they start.
use memchr::memmem;
use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
};

pub const MAGIC_V1: [u8; 4] = *b"DLT\x01";
pub const MAGIC_V2: [u8; 4] = *b"DLT\x02";
/// Storage header: pattern, seconds and microseconds (little-endian), ECU.
pub const STORAGE: usize = 16;
/// Bytes of one unit that is not a message; a longer run is several units.
pub const GARBAGE_CAP: usize = 64 * 1024;
/// Largest message: the storage header and a 16-bit standard header length.
pub const MAX_MESSAGE: usize = STORAGE + u16::MAX as usize;
/// Prefix of rows for units that are not messages; no message row has it.
pub const ISSUE: &str = "!! ";
const CORRUPTED: &[u8] = b"!!CORRUPTED MESSAGE!!";
/// Non-verbose data bytes shown after the message ID, as dlt-viewer does.
const NON_VERBOSE_SHOWN: usize = 256;
/// Bytes of a non-message unit shown in its row.
const ISSUE_SHOWN: usize = 32;

const UEH: u8 = 0x01;
const MSBF: u8 = 0x02;
const WEID: u8 = 0x04;
const WSID: u8 = 0x08;
const WTMS: u8 = 0x10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Message,
    /// Bytes that are no message, also a message cut short by the file end.
    Garbage,
    /// A protocol version 2 message (`DLT\x02`) and what follows it up to
    /// the next message.
    Unsupported,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unit {
    pub start: usize,
    pub len: usize,
    pub kind: Kind,
}

/// Whether a file starts like a DLT file of either version.
pub fn is_dlt(head: &[u8]) -> bool {
    head.starts_with(&MAGIC_V1) || head.starts_with(&MAGIC_V2)
}

/// Bytes the standard header needs for the flags in `htyp`.
fn min_len(htyp: u8) -> usize {
    4 + 4
        * ((htyp & WEID != 0) as usize + (htyp & WSID != 0) as usize + (htyp & WTMS != 0) as usize)
        + if htyp & UEH != 0 { 10 } else { 0 }
}

/// Bytes of a file's start that `headerless` reads: three of the largest
/// messages and a header.
pub const HEADERLESS_HEAD: usize = 3 * u16::MAX as usize + 8;
/// Whether `head`, the start of a file of `size` bytes that has no storage
/// headers, reads as DLT messages: standard headers of protocol version 1 or
/// 2 whose lengths lead from one to the next, through three messages or to
/// the file's end. Such data carries no recording time and does not open;
/// this only names the reason.
pub fn headerless(head: &[u8], size: u64) -> bool {
    fn chain(size: u64, len_at: impl Fn(usize) -> Option<usize>) -> bool {
        let mut p = 0;
        for _ in 0..3 {
            match len_at(p) {
                Some(len) if (p + len) as u64 <= size => p += len,
                _ => return false,
            }
            if p as u64 == size {
                return true;
            }
        }
        true
    }
    let v1 = |p: usize| {
        let b = head.get(p..p + 4)?;
        let len = u16::from_be_bytes([b[2], b[3]]) as usize;
        (b[0] >> 5 == 1 && len >= min_len(b[0])).then_some(len)
    };
    // Version 2 writers disagree on the length's byte order: dlt-viewer's
    // sample is big-endian, dlt-daemon's little-endian.
    let v2 = |big: bool| {
        move |p: usize| {
            let b = head.get(p..p + 7)?;
            let len = if big {
                u16::from_be_bytes([b[5], b[6]])
            } else {
                u16::from_le_bytes([b[5], b[6]])
            } as usize;
            (b[0] >> 5 == 2 && len >= 7).then_some(len)
        }
    };
    chain(size, v1) || chain(size, v2(true)) || chain(size, v2(false))
}
enum Check {
    Yes(usize),
    No,
    /// More bytes than `buf` holds decide it.
    More,
}
/// A version 1 message at `p` that fits before `limit`: its length.
/// `strict` also wants plausible microseconds and message type, for
/// accepting a start found inside other bytes.
fn message_at(buf: &[u8], p: usize, limit: usize, strict: bool) -> Check {
    if limit < p + STORAGE + 4 {
        return Check::No;
    }
    let b = match buf.get(p..p + STORAGE + 4) {
        Some(b) => b,
        None => {
            // A mismatch in the bytes already here decides it.
            let have = &buf[p.min(buf.len())..];
            let n = have.len().min(4);
            return if have[..n] != MAGIC_V1[..n] {
                Check::No
            } else {
                Check::More
            };
        }
    };
    if b[..4] != MAGIC_V1 {
        return Check::No;
    }
    let htyp = b[16];
    let len = u16::from_be_bytes([b[18], b[19]]) as usize;
    if htyp >> 5 > 1 || len < min_len(htyp) || p + STORAGE + len > limit {
        return Check::No;
    }
    if strict {
        if u32::from_le_bytes([b[8], b[9], b[10], b[11]]) >= 1_000_000 {
            return Check::No;
        }
        if htyp & UEH != 0 {
            let msin = p + STORAGE + min_len(htyp) - 10;
            match buf.get(msin) {
                Some(m) if (m >> 1) & 7 > 3 => return Check::No,
                Some(_) => {}
                None => return Check::More,
            }
        }
    }
    if p + STORAGE + len > buf.len() {
        return Check::More;
    }
    Check::Yes(STORAGE + len)
}
static PATTERN: LazyLock<memmem::Finder<'static>> = LazyLock::new(|| memmem::Finder::new(b"DLT"));
/// End of the non-message unit at `p`: the next strict message start or
/// version 2 pattern, the cap, or `limit`. None when bytes beyond `buf`
/// decide it.
fn skip_end(buf: &[u8], p: usize, limit: usize) -> Option<usize> {
    let end = (p + GARBAGE_CAP).min(limit);
    // A pattern that starts before `end` may end up to 3 bytes after it.
    let want = (end + 3).min(limit);
    for i in PATTERN.find_iter(&buf[p + 1..want.min(buf.len())]) {
        let q = p + 1 + i;
        if q >= end {
            break;
        }
        match buf.get(q + 3) {
            Some(2) => return Some(q),
            Some(1) => match message_at(buf, q, limit, true) {
                Check::Yes(_) => return Some(q),
                Check::No => {}
                Check::More => return None,
            },
            Some(_) => {}
            None if q + 3 >= limit => {}
            None => return None,
        }
    }
    (want <= buf.len()).then_some(end)
}
/// Units of `buf`, whose file part continues to `limit` bytes from its
/// start (`limit >= buf.len()`). Stops before the first unit that bytes
/// beyond `buf` decide; with `limit == buf.len()` every byte is in a unit.
pub fn units(buf: &[u8], limit: usize) -> Units<'_> {
    debug_assert!(limit >= buf.len());
    Units { buf, limit, at: 0 }
}
pub struct Units<'a> {
    buf: &'a [u8],
    limit: usize,
    at: usize,
}
impl Units<'_> {
    /// Bytes in the units returned so far.
    pub fn position(&self) -> usize {
        self.at
    }
}
impl Iterator for Units<'_> {
    type Item = Unit;
    fn next(&mut self) -> Option<Unit> {
        let (buf, p) = (self.buf, self.at);
        if p >= buf.len() {
            return None;
        }
        let (len, kind) = match message_at(buf, p, self.limit, false) {
            Check::Yes(n) => (n, Kind::Message),
            Check::More => return None,
            Check::No => {
                let kind = if buf[p..].starts_with(&MAGIC_V2) {
                    Kind::Unsupported
                } else if buf.len() - p < 4
                    && self.limit > buf.len()
                    && MAGIC_V2.starts_with(&buf[p..])
                {
                    // A version 2 pattern may still be arriving.
                    return None;
                } else {
                    Kind::Garbage
                };
                (skip_end(buf, p, self.limit)? - p, kind)
            }
        };
        self.at = p + len;
        Some(Unit {
            start: p,
            len,
            kind,
        })
    }
}

/// Header fields of one whole message unit.
#[derive(Clone, Copy, Debug)]
pub struct Header<'a> {
    pub seconds: u32,
    pub micros: u32,
    pub storage_ecu: [u8; 4],
    pub htyp: u8,
    pub count: u8,
    pub len: u16,
    pub ecu: Option<[u8; 4]>,
    pub session: Option<u32>,
    /// Time since the ECU started, in 0.1 ms.
    pub timestamp: Option<u32>,
    pub ext: Option<Ext>,
    pub payload: &'a [u8],
}
#[derive(Clone, Copy, Debug)]
pub struct Ext {
    pub msin: u8,
    pub noar: u8,
    pub apid: [u8; 4],
    pub ctid: [u8; 4],
}
impl Ext {
    pub fn verbose(&self) -> bool {
        self.msin & 1 != 0
    }
    /// Message type: 0 log, 1 app_trace, 2 nw_trace, 3 control.
    pub fn mstp(&self) -> u8 {
        (self.msin >> 1) & 7
    }
    /// Message type info, such as the log level of a log message.
    pub fn mtin(&self) -> u8 {
        self.msin >> 4
    }
}
impl Header<'_> {
    /// Payload values (arguments, message IDs) are big-endian with MSBF.
    pub fn big_endian(&self) -> bool {
        self.htyp & MSBF != 0
    }
}
fn id(b: &[u8]) -> [u8; 4] {
    [b[0], b[1], b[2], b[3]]
}
fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}
/// The header of a message unit as `units` returns it.
pub fn header(unit: &[u8]) -> Option<Header<'_>> {
    let b = unit.get(..STORAGE + 4)?;
    if b[..4] != MAGIC_V1 {
        return None;
    }
    let htyp = b[16];
    let len = u16::from_be_bytes([b[18], b[19]]);
    let end = STORAGE + len as usize;
    if htyp >> 5 > 1 || (len as usize) < min_len(htyp) || unit.len() < end {
        return None;
    }
    let mut at = STORAGE + 4;
    let mut field = |flag: u8| {
        (htyp & flag != 0).then(|| {
            at += 4;
            &unit[at - 4..at]
        })
    };
    let ecu = field(WEID).map(id);
    let session = field(WSID).map(be32);
    let timestamp = field(WTMS).map(be32);
    let ext = (htyp & UEH != 0).then(|| {
        let e = &unit[at..at + 10];
        at += 10;
        Ext {
            msin: e[0],
            noar: e[1],
            apid: id(&e[2..6]),
            ctid: id(&e[6..10]),
        }
    });
    Some(Header {
        seconds: u32::from_le_bytes([b[4], b[5], b[6], b[7]]),
        micros: u32::from_le_bytes([b[8], b[9], b[10], b[11]]),
        storage_ecu: id(&b[12..16]),
        htyp,
        count: b[17],
        len,
        ecu,
        session,
        timestamp,
        ext,
        payload: &unit[at..end],
    })
}
/// Bytes a message cut short by its unit would need: the unit starts like a
/// message whose header claims more bytes than the unit has.
pub fn truncated(unit: &[u8]) -> Option<usize> {
    let b = unit.get(..STORAGE + 4)?;
    let htyp = b[16];
    let len = u16::from_be_bytes([b[18], b[19]]) as usize;
    (b[..4] == MAGIC_V1 && htyp >> 5 <= 1 && len >= min_len(htyp) && unit.len() < STORAGE + len)
        .then_some(STORAGE + len)
}
/// Storage time of the first message in a file head, as UTC seconds and
/// microseconds (microseconds of a second or more carry into the seconds).
pub fn first_time(head: &[u8]) -> Option<(u64, u32)> {
    if !head.starts_with(&MAGIC_V1) && !head.starts_with(&MAGIC_V2) {
        return None;
    }
    units(head, head.len())
        .filter(|u| u.kind == Kind::Message)
        .find_map(|u| header(&head[u.start..u.start + u.len]))
        .map(|h| {
            (
                h.seconds as u64 + (h.micros / 1_000_000) as u64,
                h.micros % 1_000_000,
            )
        })
}

/// `YYYY-MM-DD` and `HH:MM:SS.uuuuuu` of a UTC time, as rows show dates.
pub fn utc_text(seconds: u64, micros: u32) -> (String, String) {
    let t = seconds as i64;
    let (y, m, d) = civil(t.div_euclid(86_400));
    let s = t.rem_euclid(86_400);
    (
        format!("{y:04}-{m:02}-{d:02}"),
        format!(
            "{:02}:{:02}:{:02}.{micros:06}",
            s / 3600,
            s / 60 % 60,
            s % 60
        ),
    )
}
/// UTC offsets that convert storage times to local times.
pub enum Clock {
    /// The system time zone, each offset kept once seen so that rendering
    /// the same bytes again gives the same text even if the zone changes.
    Local(Memo),
    /// A fixed offset in seconds east of UTC, for tests and measurements.
    Fixed(i32),
}
impl Clock {
    pub fn local() -> Self {
        Clock::Local(Memo::default())
    }
    /// Offset at UTC second `t`.
    pub fn offset(&self, t: i64) -> i32 {
        match self {
            Clock::Fixed(o) => *o,
            Clock::Local(m) => m.offset(t, system_offset),
        }
    }
}
/// Offsets by UTC hour; an hour in which the offset changes keeps them by
/// second. Entries past `MEMO_LIMIT` are computed without being kept.
#[derive(Default)]
pub struct Memo {
    hours: Mutex<HashMap<i64, Option<i32>>>,
    seconds: Mutex<HashMap<i64, i32>>,
}
const MEMO_LIMIT: usize = 1 << 20;
impl Memo {
    pub fn offset(&self, t: i64, system: impl Fn(i64) -> i32) -> i32 {
        let hour = t.div_euclid(3600);
        let mut hours = self.hours.lock().unwrap();
        let known = match hours.get(&hour) {
            Some(known) => *known,
            None => {
                let (a, b) = (system(hour * 3600), system(hour * 3600 + 3599));
                let known = (a == b).then_some(a);
                if hours.len() < MEMO_LIMIT {
                    hours.insert(hour, known);
                }
                known
            }
        };
        drop(hours);
        if let Some(o) = known {
            return o;
        }
        let mut seconds = self.seconds.lock().unwrap();
        if let Some(o) = seconds.get(&t) {
            return *o;
        }
        let o = system(t);
        if seconds.len() < MEMO_LIMIT {
            seconds.insert(t, o);
        }
        o
    }
    /// Whether the offset of UTC second `t` is uniform within its hour and
    /// already known, and which it is.
    fn hour(&self, t: i64) -> Option<i32> {
        *self.hours.lock().unwrap().get(&t.div_euclid(3600))?
    }
}
fn system_offset(t: i64) -> i32 {
    use chrono::{Local, Offset, TimeZone};
    chrono::DateTime::from_timestamp(t, 0).map_or(0, |utc| {
        Local
            .offset_from_utc_datetime(&utc.naive_utc())
            .fix()
            .local_minus_utc()
    })
}

/// Proleptic Gregorian date of a day count from 1970-01-01.
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}
fn push_u64(out: &mut Vec<u8>, mut n: u64) {
    let mut buf = [0u8; 20];
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    out.extend_from_slice(&buf[i..]);
}
fn push_i64(out: &mut Vec<u8>, n: i64) {
    if n < 0 {
        out.push(b'-');
    }
    push_u64(out, n.unsigned_abs());
}
fn push_padded(out: &mut Vec<u8>, mut n: u64, width: usize) {
    let mut buf = [b'0'; 20];
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    out.extend_from_slice(&buf[i.min(buf.len() - width)..]);
}
const HEX: &[u8; 16] = b"0123456789abcdef";
/// Lowercase hex bytes separated by spaces.
fn push_hex(out: &mut Vec<u8>, data: &[u8]) {
    for (i, b) in data.iter().enumerate() {
        if i > 0 {
            out.push(b' ');
        }
        out.extend_from_slice(&[HEX[(b >> 4) as usize], HEX[(b & 15) as usize]]);
    }
}
/// An ID up to its first NUL without trailing spaces; other bytes outside
/// printable ASCII become `_`, an empty ID `-`.
/// An ID's text in its row: the bytes up to the first NUL without trailing
/// ASCII whitespace, other non-graphic bytes as `_`, and its length; empty
/// when there is none, which the row shows as `-`.
fn id_text(id: &[u8; 4]) -> ([u8; 4], usize) {
    let n = id.iter().position(|&b| b == 0).unwrap_or(4);
    let id = id[..n].trim_ascii_end();
    let mut text = [0; 4];
    for (t, &b) in text.iter_mut().zip(id) {
        *t = if b.is_ascii_graphic() { b } else { b'_' };
    }
    (text, id.len())
}
fn push_id(out: &mut Vec<u8>, id: &[u8; 4]) {
    let (text, n) = id_text(id);
    if n == 0 {
        out.push(b'-');
    }
    out.extend_from_slice(&text[..n]);
}
/// An ID as the parsed row's field holds it (`parser::DltFields`): its row
/// text, or "" where the row shows `-`.
pub fn id_field<'a>(id: &[u8; 4], buf: &'a mut [u8; 4]) -> &'a str {
    let (text, n) = id_text(id);
    *buf = text;
    // ASCII graphic bytes only.
    let s = std::str::from_utf8(&buf[..n]).unwrap_or("");
    if absent(s) {
        ""
    } else {
        s
    }
}
fn push_opt(out: &mut Vec<u8>, n: Option<u32>) {
    match n {
        Some(n) => push_u64(out, n.into()),
        None => out.push(b'-'),
    }
}

/// Bytes that may start whitespace (`char::is_whitespace`) or NUL in
/// UTF-8: ASCII controls and space, and the lead bytes of U+0085, U+00A0,
/// U+1680, U+2000-U+200A, U+2028, U+2029, U+202F, U+205F and U+3000.
static SPECIAL: [bool; 256] = {
    let mut t = [false; 256];
    let mut i = 0;
    while i <= 0x20 {
        t[i] = true;
        i += 1;
    }
    t[0xc2] = true;
    t[0xe1] = true;
    t[0xe2] = true;
    t[0xe3] = true;
    t
};
/// Writes payload text with dlt-viewer's `simplified()`: whitespace runs
/// become one space, leading and trailing whitespace go, NUL is removed.
struct Simple<'a> {
    out: &'a mut Vec<u8>,
    start: usize,
    space: bool,
}
impl Simple<'_> {
    fn space(&mut self) {
        self.space = self.out.len() > self.start;
    }
    fn text(&mut self, s: &str) {
        let b = s.as_bytes();
        let mut i = 0;
        while i < b.len() {
            // Bytes that cannot start whitespace or NUL are copied in runs.
            let run = b[i..]
                .iter()
                .position(|&c| SPECIAL[c as usize])
                .map_or(b.len(), |n| i + n);
            if run > i {
                if self.space {
                    self.out.push(b' ');
                    self.space = false;
                }
                self.out.extend_from_slice(&b[i..run]);
                i = run;
                continue;
            }
            let ch = s[i..].chars().next().unwrap();
            if ch == '\0' {
            } else if ch.is_whitespace() {
                self.space();
            } else {
                if self.space {
                    self.out.push(b' ');
                    self.space = false;
                }
                self.out.extend_from_slice(&b[i..i + ch.len_utf8()]);
            }
            i += ch.len_utf8();
        }
    }
    /// Nonempty text known to hold no whitespace or NUL, such as a number.
    fn word(&mut self, f: impl FnOnce(&mut Vec<u8>)) {
        if self.space {
            self.out.push(b' ');
            self.space = false;
        }
        f(self.out);
    }
}

/// Reads payload values in the message's byte order.
struct Reader<'a> {
    b: &'a [u8],
    at: usize,
    be: bool,
}
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.b.get(self.at..self.at.checked_add(n)?)?;
        self.at += n;
        Some(s)
    }
    fn uint(&mut self, n: usize) -> Option<u128> {
        let s = self.take(n)?;
        let mut v = 0u128;
        if self.be {
            for &b in s {
                v = v << 8 | b as u128;
            }
        } else {
            for &b in s.iter().rev() {
                v = v << 8 | b as u128;
            }
        }
        Some(v)
    }
    fn u16(&mut self) -> Option<usize> {
        self.uint(2).map(|v| v as usize)
    }
    fn u32(&mut self) -> Option<u32> {
        self.uint(4).map(|v| v as u32)
    }
}

const BOOL: u32 = 0x10;
const SINT: u32 = 0x20;
const UINT: u32 = 0x40;
const FLOA: u32 = 0x80;
const ARAY: u32 = 0x100;
const STRG: u32 = 0x200;
const RAWD: u32 = 0x400;
const VARI: u32 = 0x800;
const FIXP: u32 = 0x1000;
const TRAI: u32 = 0x2000;
const STRU: u32 = 0x4000;
const SCOD: u32 = 0x38000;
const SCOD_HEX: u32 = 0x10000;
const SCOD_BIN: u32 = 0x18000;

/// One verbose argument; None when it cannot be read, which marks the
/// whole message corrupted as dlt-viewer does.
fn argument(r: &mut Reader, s: &mut Simple) -> Option<()> {
    let info = r.u32()?;
    let bytes = match info & 0x0f {
        1 => 1,
        2 => 2,
        3 => 4,
        4 => 8,
        5 => 16,
        _ => 0,
    };
    let vari = info & VARI != 0;
    if info & (ARAY | STRU | FIXP) != 0 {
        return None;
    }
    // dlt-viewer's precedence when several type bits are set.
    let kind = *[STRG, BOOL, SINT, UINT, FLOA, RAWD, TRAI]
        .iter()
        .find(|&&k| info & k != 0)?;
    match kind {
        BOOL => {
            if vari {
                let name = r.u16()?;
                r.take(name)?;
            }
            let v = r.take(bytes.max(1))?;
            s.word(|o| {
                o.extend_from_slice(if v.iter().any(|&b| b != 0) {
                    b"true"
                } else {
                    b"false"
                })
            });
        }
        SINT | UINT | FLOA => {
            if bytes == 0 {
                return None;
            }
            if vari {
                let (name, unit) = (r.u16()?, r.u16()?);
                r.take(name)?;
                r.take(unit)?;
            }
            let v = r.uint(bytes)?;
            if kind == FLOA {
                let f = match bytes {
                    4 => Some(f32::from_bits(v as u32) as f64),
                    8 => Some(f64::from_bits(v as u64)),
                    _ => None,
                };
                s.word(|o| match f {
                    Some(f) if f.is_nan() => o.extend_from_slice(b"nan"),
                    Some(f) if f.is_infinite() => {
                        o.extend_from_slice(if f < 0.0 { b"-inf" } else { b"inf" })
                    }
                    Some(f) => {
                        use std::io::Write;
                        let _ = write!(o, "{f:.8}");
                    }
                    None => o.push(b'?'),
                });
            } else if bytes == 16 {
                s.word(|o| o.push(b'?'));
            } else if kind == SINT {
                let shift = 128 - 8 * bytes;
                let n = ((v << shift) as i128 >> shift) as i64;
                s.word(|o| push_i64(o, n));
            } else {
                match info & SCOD {
                    SCOD_HEX => s.word(|o| {
                        o.extend_from_slice(b"0x");
                        for i in (0..bytes * 2).rev() {
                            o.push(HEX[((v >> (4 * i)) & 15) as usize]);
                        }
                    }),
                    SCOD_BIN => {
                        // Nibbles separated by spaces: `0b0101 0101`.
                        for i in (0..bytes * 2).rev() {
                            s.word(|o| {
                                if i == bytes * 2 - 1 {
                                    o.extend_from_slice(b"0b");
                                }
                                for bit in (0..4).rev() {
                                    o.push(b'0' + ((v >> (4 * i + bit)) & 1) as u8);
                                }
                            });
                            if i > 0 {
                                s.space();
                            }
                        }
                    }
                    _ => s.word(|o| push_u64(o, v as u64)),
                }
            }
        }
        STRG | RAWD => {
            let len = r.u16()?;
            if vari {
                let name = r.u16()?;
                r.take(name)?;
            }
            let data = r.take(len)?;
            if kind == STRG {
                s.text(&String::from_utf8_lossy(data));
            } else if !data.is_empty() {
                s.word(|o| push_hex(o, data));
            }
        }
        _ => {
            let len = r.u16()?;
            r.take(len)?;
            s.word(|o| o.push(b'?'));
        }
    }
    Some(())
}
const SERVICES: [&str; 20] = [
    "set_log_level",
    "set_trace_status",
    "get_log_info",
    "get_default_log_level",
    "store_config",
    "reset_to_factory_default",
    "set_com_interface_status",
    "set_com_interface_max_bandwidth",
    "set_verbose_mode",
    "set_message_filtering",
    "set_timing_packets",
    "get_local_time",
    "use_ecu_id",
    "use_session_id",
    "use_timestamp",
    "use_extended_header",
    "set_default_log_level",
    "set_default_trace_status",
    "get_software_version",
    "message_buffer_overflow",
];
const SERVICES_F: [&str; 9] = [
    "unregister_context",
    "connection_info",
    "timezone",
    "marker",
    "offline_logstorage",
    "passive_node_connect",
    "passive_node_connection_status",
    "set_all_log_level",
    "set_all_trace_status",
];
fn service_name(id: u32, out: &mut Vec<u8>) {
    let name = match id {
        1..=20 => Some(SERVICES[id as usize - 1]),
        0xf01..=0xf09 => Some(SERVICES_F[id as usize - 0xf01]),
        _ => None,
    };
    match name {
        Some(n) => out.extend_from_slice(n.as_bytes()),
        None => push_u64(out, id.into()),
    }
}
fn control(h: &Header, mtin: u8, s: &mut Simple) {
    let p = h.payload;
    let mut r = Reader {
        b: p,
        at: 0,
        be: h.big_endian(),
    };
    let Some(service) = r.u32() else {
        s.word(|o| o.extend_from_slice(b"[?]"));
        s.space();
        s.word(|o| push_hex(o, p));
        return;
    };
    if service == 0xf04 {
        s.word(|o| o.extend_from_slice(b"MARKER"));
        return;
    }
    let response = mtin == 2;
    s.word(|o| {
        o.push(b'[');
        service_name(service, o);
    });
    let mut rest = &p[4..];
    if response {
        if let Some((&status, after)) = rest.split_first() {
            s.space();
            s.word(|o| match status {
                0 => o.extend_from_slice(b"ok"),
                1 => o.extend_from_slice(b"not_supported"),
                2 => o.extend_from_slice(b"error"),
                8 => o.extend_from_slice(b"no_matching_context_id"),
                n => push_u64(o, n.into()),
            });
            rest = after;
        }
    }
    s.word(|o| o.push(b']'));
    if response && service == 0x13 && p.len() > 9 {
        // The version text after a 4-byte length, as Latin-1.
        s.space();
        let text: String = p[9..].iter().map(|&b| b as char).collect();
        s.text(&text);
    } else if !rest.is_empty() {
        s.space();
        s.word(|o| push_hex(o, rest));
    }
}
fn non_verbose(h: &Header, s: &mut Simple) {
    let p = h.payload;
    let mut r = Reader {
        b: p,
        at: 0,
        be: h.big_endian(),
    };
    let data = match r.u32() {
        Some(id) => {
            s.word(|o| {
                o.push(b'[');
                push_u64(o, id.into());
                o.push(b']');
            });
            &p[4..]
        }
        None => {
            s.word(|o| o.extend_from_slice(b"[?]"));
            p
        }
    };
    let data = &data[..data.len().min(NON_VERBOSE_SHOWN)];
    s.space();
    let ascii: String = data
        .iter()
        .map(|&b| {
            if (0x20..0x7f).contains(&b) {
                b as char
            } else {
                '-'
            }
        })
        .collect();
    s.text(&ascii);
    s.word(|o| {
        o.push(b'|');
        push_hex(o, data);
    });
}
fn payload(h: &Header, out: &mut Vec<u8>) {
    let start = out.len();
    let mut s = Simple {
        out,
        start,
        space: false,
    };
    match &h.ext {
        Some(e) if e.mstp() == 3 => control(h, e.mtin(), &mut s),
        Some(e) if e.verbose() => {
            let mut r = Reader {
                b: h.payload,
                at: 0,
                be: h.big_endian(),
            };
            for _ in 0..e.noar {
                s.space();
                if argument(&mut r, &mut s).is_none() {
                    s.out.truncate(start);
                    s.out.extend_from_slice(CORRUPTED);
                    return;
                }
            }
        }
        _ => non_verbose(h, &mut s),
    }
}
const LOG_LEVELS: [u8; 6] = *b"FEWIDV";
/// The `parser::LEVELS` letter `parse_row` reads from the kind `push_kind`
/// writes: the log level, or `U` for other messages and those without an
/// extended header.
fn level_of(ext: Option<&Ext>) -> &'static str {
    match ext {
        Some(e) if e.mstp() == 0 && (1..=6).contains(&e.mtin()) => {
            ["F", "E", "W", "I", "D", "V"][e.mtin() as usize - 1]
        }
        _ => "U",
    }
}
const TYPES: [&str; 4] = ["log", "app_trace", "nw_trace", "control"];
const SUBTYPES: [&[&str]; 4] = [
    &["fatal", "error", "warn", "info", "debug", "verbose"],
    &["variable", "func_in", "func_out", "state", "vfb"],
    &["ipc", "can", "flexray", "most", "vfb"],
    &["request", "response", "time"],
];
fn push_kind(out: &mut Vec<u8>, e: &Ext) {
    let (mstp, mtin) = (e.mstp() as usize, e.mtin() as usize);
    if mstp == 0 && (1..=6).contains(&mtin) {
        out.push(LOG_LEVELS[mtin - 1]);
        return;
    }
    match TYPES.get(mstp) {
        Some(t) => out.extend_from_slice(t.as_bytes()),
        None => push_u64(out, mstp as u64),
    }
    out.push(b':');
    match SUBTYPES.get(mstp).and_then(|s| s.get(mtin.wrapping_sub(1))) {
        Some(t) => out.extend_from_slice(t.as_bytes()),
        None => push_u64(out, mtin as u64),
    }
}

/// The fields of a message row that field filters read, taken from the
/// message's header without rendering the row (`Decoder::heads`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Head {
    /// `parser::LEVELS` letter, as `parse_row` reads it.
    pub level: &'static str,
    /// ECU (the header's, else the storage header's), APID and CTID bytes;
    /// None where the row shows none. `id_field` gives their text.
    pub ids: [Option<[u8; 4]>; 3],
    /// The row's local date and time: year, month * 100 + day, and
    /// nanoseconds since midnight.
    pub year: u16,
    pub month_day: u16,
    pub nanos: u64,
}
/// Renders blocks of whole units as rows.
pub struct Decoder {
    clock: Clock,
}
/// Formatting reused across the messages of one `render` call.
#[derive(Default)]
struct Recent {
    hour: Option<(i64, i32)>,
    second: Option<(i64, [u8; 19])>,
}
impl Decoder {
    pub fn new(clock: Clock) -> Self {
        Self { clock }
    }
    fn offset(&self, t: i64, recent: &mut Recent) -> i32 {
        let hour = t.div_euclid(3600);
        if let Some((h, o)) = recent.hour {
            if h == hour {
                return o;
            }
        }
        let o = self.clock.offset(t);
        let uniform = match &self.clock {
            Clock::Fixed(_) => true,
            Clock::Local(m) => m.hour(t).is_some(),
        };
        if uniform {
            recent.hour = Some((hour, o));
        }
        o
    }
    /// The local second a message's row shows; its microseconds past
    /// 999,999 carry into the second.
    fn local(&self, h: &Header, recent: &mut Recent) -> i64 {
        let utc = h.seconds as i64 + (h.micros / 1_000_000) as i64;
        utc + self.offset(utc, recent) as i64
    }
    fn stamp(&self, h: &Header, out: &mut Vec<u8>, recent: &mut Recent) {
        let local = self.local(h, recent);
        let text = match recent.second {
            Some((s, text)) if s == local => text,
            _ => {
                let (y, m, d) = civil(local.div_euclid(86_400));
                let t = local.rem_euclid(86_400);
                let mut text = [0u8; 19];
                let mut v = Vec::with_capacity(19);
                push_padded(&mut v, y.max(0) as u64, 4);
                v.push(b'-');
                push_padded(&mut v, m.into(), 2);
                v.push(b'-');
                push_padded(&mut v, d.into(), 2);
                v.push(b' ');
                push_padded(&mut v, (t / 3600) as u64, 2);
                v.push(b':');
                push_padded(&mut v, (t / 60 % 60) as u64, 2);
                v.push(b':');
                push_padded(&mut v, (t % 60) as u64, 2);
                text.copy_from_slice(&v[..19]);
                recent.second = Some((local, text));
                text
            }
        };
        out.extend_from_slice(&text);
        out.push(b'.');
        push_padded(out, (h.micros % 1_000_000).into(), 6);
    }
    fn message(&self, h: &Header, out: &mut Vec<u8>, recent: &mut Recent) {
        self.stamp(h, out, recent);
        out.push(b' ');
        match h.timestamp {
            Some(t) => {
                push_u64(out, (t / 10_000).into());
                out.push(b'.');
                push_padded(out, (t % 10_000).into(), 4);
            }
            None => out.push(b'-'),
        }
        out.push(b' ');
        push_u64(out, h.count.into());
        out.push(b' ');
        push_id(out, h.ecu.as_ref().unwrap_or(&h.storage_ecu));
        out.push(b' ');
        match &h.ext {
            Some(e) => {
                push_id(out, &e.apid);
                out.push(b' ');
                push_id(out, &e.ctid);
            }
            None => out.extend_from_slice(b"- -"),
        }
        out.push(b' ');
        push_opt(out, h.session);
        out.push(b' ');
        match &h.ext {
            Some(e) => push_kind(out, e),
            None => out.push(b'-'),
        }
        let at = out.len();
        out.push(b' ');
        payload(h, out);
        if out.len() == at + 1 {
            out.pop();
        }
    }
    fn issue(&self, unit: &[u8], kind: Kind, at: u64, out: &mut Vec<u8>) {
        use std::io::Write;
        let n = unit.len();
        let _ = match (kind, truncated(unit)) {
            (Kind::Unsupported, _) => {
                write!(
                    out,
                    "{ISSUE}지원하지 않는 DLT 버전 2 데이터 {n}바이트 · 파일 위치 {at}"
                )
            }
            (_, Some(need)) => write!(
                out,
                "{ISSUE}잘린 DLT 메시지: {need}바이트 중 {n}바이트 · 파일 위치 {at}"
            ),
            _ => write!(
                out,
                "{ISSUE}DLT 메시지가 아닌 데이터 {n}바이트 · 파일 위치 {at}"
            ),
        };
        out.extend_from_slice(b": ");
        push_hex(out, &unit[..n.min(ISSUE_SHOWN)]);
        if n > ISSUE_SHOWN {
            out.extend_from_slice(" …".as_bytes());
        }
    }
    /// Calls `f` for each unit of `raw` (whole units, as `render` takes them)
    /// in row order with the fields a filter reads of its row: the `Head` of
    /// a message, None for the rows of other units.
    pub fn heads(&self, raw: &[u8], mut f: impl FnMut(Option<&Head>)) {
        let mut recent = Recent::default();
        for u in units(raw, raw.len()) {
            match (u.kind, header(&raw[u.start..u.start + u.len])) {
                (Kind::Message, Some(h)) => {
                    let local = self.local(&h, &mut recent);
                    let (y, m, d) = civil(local.div_euclid(86_400));
                    let ext = h.ext.as_ref();
                    f(Some(&Head {
                        level: level_of(ext),
                        ids: [
                            Some(h.ecu.unwrap_or(h.storage_ecu)),
                            ext.map(|e| e.apid),
                            ext.map(|e| e.ctid),
                        ],
                        year: y.max(0) as u16,
                        month_day: (m * 100 + d) as u16,
                        nanos: local.rem_euclid(86_400) as u64 * 1_000_000_000
                            + u64::from(h.micros % 1_000_000) * 1_000,
                    }))
                }
                _ => f(None),
            }
        }
    }
    /// Appends one LF-terminated row per unit of `raw`, which holds whole
    /// units from the start of one (a block). `base` is the file offset of
    /// `raw[0]`, which rows of other units show. Returns the units' starts
    /// within `raw` followed by `raw.len()`.
    pub fn render(&self, raw: &[u8], base: u64, out: &mut Vec<u8>) -> Vec<u32> {
        let mut starts = Vec::with_capacity(raw.len() / 96 + 2);
        let mut recent = Recent::default();
        for u in units(raw, raw.len()) {
            starts.push(u.start as u32);
            let bytes = &raw[u.start..u.start + u.len];
            match (u.kind, header(bytes)) {
                (Kind::Message, Some(h)) => self.message(&h, out, &mut recent),
                (kind, _) => self.issue(bytes, kind, base + u.start as u64, out),
            }
            out.push(b'\n');
        }
        starts.push(raw.len() as u32);
        starts
    }
}

/// Fields of a rendered message row, each a slice of the row; `-` marks an
/// absent value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row<'a> {
    pub date: &'a str,
    pub time: &'a str,
    pub uptime: &'a str,
    pub count: &'a str,
    pub ecu: &'a str,
    pub apid: &'a str,
    pub ctid: &'a str,
    pub session: &'a str,
    pub kind: &'a str,
    /// `parser::LEVELS` letter: the log level, or `U` for other messages.
    pub level: &'static str,
    pub payload: &'a str,
}
/// Whether a row field holds no value.
pub fn absent(field: &str) -> bool {
    field == "-"
}
/// The fields of a message row as `Decoder::render` writes it; None for
/// the rows of other units and for any other text.
pub fn parse_row(raw: &str) -> Option<Row<'_>> {
    let b = raw.as_bytes();
    // `YYYY-MM-DD HH:MM:SS.uuuuuu ` is 27 bytes.
    let digits =
        |r: std::ops::Range<usize>| b.get(r).is_some_and(|d| d.iter().all(u8::is_ascii_digit));
    if b.len() < 27
        || !(digits(0..4)
            && digits(5..7)
            && digits(8..10)
            && digits(11..13)
            && digits(14..16)
            && digits(17..19)
            && digits(20..26))
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b' '
        || b[13] != b':'
        || b[16] != b':'
        || b[19] != b'.'
        || b[26] != b' '
    {
        return None;
    }
    let mut at = 27;
    let mut word = || {
        let rest = raw.get(at..)?;
        let n = rest.find(' ').unwrap_or(rest.len());
        let w = &rest[..n];
        at += n + 1;
        (!w.is_empty()).then_some(w)
    };
    let (uptime, count, ecu, apid, ctid, session, kind) = (
        word()?,
        word()?,
        word()?,
        word()?,
        word()?,
        word()?,
        word()?,
    );
    let number = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    let uptime_ok = absent(uptime)
        || uptime
            .split_once('.')
            .is_some_and(|(s, f)| !s.is_empty() && number(s) && f.len() == 4 && number(f));
    if !uptime_ok
        || !number(count)
        || !(absent(session) || number(session))
        || [ecu, apid, ctid].iter().any(|i| i.len() > 4)
    {
        return None;
    }
    let level = match kind {
        "F" => "F",
        "E" => "E",
        "W" => "W",
        "I" => "I",
        "D" => "D",
        "V" => "V",
        _ if kind == "-" || kind.contains(':') => "U",
        _ => return None,
    };
    let payload = raw.get(at.min(raw.len())..).unwrap_or("");
    Some(Row {
        date: &raw[..10],
        time: &raw[11..26],
        uptime,
        count,
        ecu,
        apid,
        ctid,
        session,
        kind,
        level,
        payload,
    })
}

/// Deterministic DLT writer for tests and measurements.
#[doc(hidden)]
pub mod synth {
    use super::*;
    /// An ID padded with NUL to four bytes; longer text is cut.
    pub fn id(s: &str) -> [u8; 4] {
        let mut out = [0; 4];
        for (o, b) in out.iter_mut().zip(s.bytes()) {
            *o = b;
        }
        out
    }
    #[derive(Clone, Copy, Debug)]
    pub enum Arg<'a> {
        Bool(bool),
        /// Value and width in bytes (1, 2, 4 or 8).
        Sint(i64, u8),
        Uint(u64, u8),
        Hex(u64, u8),
        Bin(u64, u8),
        F32(f32),
        F64(f64),
        /// An ASCII-coded string; the writer adds the terminating NUL.
        Ascii(&'a [u8]),
        Utf8(&'a str),
        Raw(&'a [u8]),
        /// The inner argument with a VARI name (and unit for numbers).
        Named(&'a str, &'a str, &'a Arg<'a>),
        /// Type info and the bytes after it, as given.
        Other(u32, &'a [u8]),
    }
    fn tyle(n: u8) -> u32 {
        match n {
            1 => 1,
            2 => 2,
            4 => 3,
            8 => 4,
            _ => 5,
        }
    }
    struct W<'a> {
        out: &'a mut Vec<u8>,
        be: bool,
    }
    impl W<'_> {
        fn n(&mut self, v: u128, bytes: usize) {
            let le = v.to_le_bytes();
            let mut b = le[..bytes].to_vec();
            if self.be {
                b.reverse();
            }
            self.out.extend_from_slice(&b);
        }
        fn arg(&mut self, a: &Arg, vari: Option<(&str, &str)>) {
            let v = if vari.is_some() { VARI } else { 0 };
            let name = |w: &mut W, unit: bool| {
                if let Some((n, u)) = vari {
                    w.n(n.len() as u128 + 1, 2);
                    if unit {
                        w.n(u.len() as u128 + 1, 2);
                    }
                    w.out.extend_from_slice(n.as_bytes());
                    w.out.push(0);
                    if unit {
                        w.out.extend_from_slice(u.as_bytes());
                        w.out.push(0);
                    }
                }
            };
            match a {
                Arg::Bool(b) => {
                    self.n((BOOL | 1 | v).into(), 4);
                    name(self, false);
                    self.out.push(*b as u8);
                }
                Arg::Sint(n, w) => {
                    self.n((SINT | tyle(*w) | v).into(), 4);
                    name(self, true);
                    self.n(*n as u128, *w as usize);
                }
                Arg::Uint(n, w) | Arg::Hex(n, w) | Arg::Bin(n, w) => {
                    let scod = match a {
                        Arg::Hex(..) => SCOD_HEX,
                        Arg::Bin(..) => SCOD_BIN,
                        _ => 0,
                    };
                    self.n((UINT | tyle(*w) | scod | v).into(), 4);
                    name(self, true);
                    self.n(*n as u128, *w as usize);
                }
                Arg::F32(f) => {
                    self.n((FLOA | 3 | v).into(), 4);
                    name(self, true);
                    self.n(f.to_bits().into(), 4);
                }
                Arg::F64(f) => {
                    self.n((FLOA | 4 | v).into(), 4);
                    name(self, true);
                    self.n(f.to_bits().into(), 8);
                }
                Arg::Ascii(s) => self.string(STRG, s, vari),
                Arg::Utf8(s) => self.string(STRG | 0x8000, s.as_bytes(), vari),
                Arg::Raw(d) => {
                    self.n((RAWD | v).into(), 4);
                    self.n(d.len() as u128, 2);
                    name(self, false);
                    self.out.extend_from_slice(d);
                }
                Arg::Named(n, u, inner) => self.arg(inner, Some((n, u))),
                Arg::Other(info, rest) => {
                    self.n((*info).into(), 4);
                    self.out.extend_from_slice(rest);
                }
            }
        }
        fn string(&mut self, kind: u32, s: &[u8], vari: Option<(&str, &str)>) {
            self.n((kind | if vari.is_some() { VARI } else { 0 }).into(), 4);
            self.n(s.len() as u128 + 1, 2);
            if let Some((n, _)) = vari {
                self.n(n.len() as u128 + 1, 2);
                self.out.extend_from_slice(n.as_bytes());
                self.out.push(0);
            }
            self.out.extend_from_slice(s);
            self.out.push(0);
        }
    }
    /// Verbose payload bytes of `args`.
    pub fn verbose(big_endian: bool, args: &[Arg]) -> Vec<u8> {
        let mut out = vec![];
        let mut w = W {
            out: &mut out,
            be: big_endian,
        };
        for a in args {
            w.arg(a, None);
        }
        out
    }
    /// A message ID and data, for non-verbose and control payloads.
    pub fn with_id(big_endian: bool, id: u32, data: &[u8]) -> Vec<u8> {
        let mut out = if big_endian {
            id.to_be_bytes()
        } else {
            id.to_le_bytes()
        }
        .to_vec();
        out.extend_from_slice(data);
        out
    }
    #[derive(Clone, Debug)]
    pub struct Message {
        pub seconds: u32,
        pub micros: u32,
        pub storage_ecu: [u8; 4],
        pub count: u8,
        pub big_endian: bool,
        pub ecu: Option<[u8; 4]>,
        pub session: Option<u32>,
        pub timestamp: Option<u32>,
        /// MSIN, NOAR, APID and CTID.
        pub ext: Option<(u8, u8, [u8; 4], [u8; 4])>,
        pub payload: Vec<u8>,
        /// Standard header version bits (1 for real files).
        pub version: u8,
    }
    impl Default for Message {
        fn default() -> Self {
            Self {
                seconds: 1_773_480_413,
                micros: 589_793,
                storage_ecu: id("ECU1"),
                count: 0,
                big_endian: false,
                ecu: Some(id("ECU1")),
                session: Some(4242),
                timestamp: Some(12_345_678),
                ext: None,
                payload: vec![],
                version: 1,
            }
        }
    }
    /// MSIN of a message: verbose flag, type and type info.
    pub fn msin(verbose: bool, mstp: u8, mtin: u8) -> u8 {
        verbose as u8 | (mstp & 7) << 1 | mtin << 4
    }
    impl Message {
        /// A verbose log message at `level` (1 fatal … 6 verbose).
        pub fn log(level: u8, apid: &str, ctid: &str, args: &[Arg]) -> Self {
            Self {
                ext: Some((msin(true, 0, level), args.len() as u8, id(apid), id(ctid))),
                payload: verbose(false, args),
                ..Self::default()
            }
        }
        /// The same message with `args` as big-endian payload values.
        pub fn big(mut self, args: &[Arg]) -> Self {
            self.big_endian = true;
            self.payload = verbose(true, args);
            self.ext = self
                .ext
                .map(|(msin, _, apid, ctid)| (msin, args.len() as u8, apid, ctid));
            self
        }
        pub fn write(&self, out: &mut Vec<u8>) {
            let mut htyp = self.version << 5;
            let mut extra = vec![];
            if let Some(e) = self.ecu {
                htyp |= WEID;
                extra.extend_from_slice(&e);
            }
            if let Some(s) = self.session {
                htyp |= WSID;
                extra.extend_from_slice(&s.to_be_bytes());
            }
            if let Some(t) = self.timestamp {
                htyp |= WTMS;
                extra.extend_from_slice(&t.to_be_bytes());
            }
            if let Some((msin, noar, apid, ctid)) = self.ext {
                htyp |= UEH;
                extra.push(msin);
                extra.push(noar);
                extra.extend_from_slice(&apid);
                extra.extend_from_slice(&ctid);
            }
            if self.big_endian {
                htyp |= MSBF;
            }
            let len = 4 + extra.len() + self.payload.len();
            out.extend_from_slice(&MAGIC_V1);
            out.extend_from_slice(&self.seconds.to_le_bytes());
            out.extend_from_slice(&self.micros.to_le_bytes());
            out.extend_from_slice(&self.storage_ecu);
            out.push(htyp);
            out.push(self.count);
            out.extend_from_slice(&(len as u16).to_be_bytes());
            out.extend_from_slice(&extra);
            out.extend_from_slice(&self.payload);
        }
        pub fn bytes(&self) -> Vec<u8> {
            let mut out = vec![];
            self.write(&mut out);
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{synth::*, *};
    fn render(raw: &[u8]) -> String {
        let mut out = vec![];
        Decoder::new(Clock::Fixed(0)).render(raw, 0, &mut out);
        String::from_utf8(out).unwrap()
    }
    #[test]
    fn civil_dates() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(-1), (1969, 12, 31));
        assert_eq!(civil(11_016), (2000, 2, 29));
        assert_eq!(civil(20_526), (2026, 3, 14));
    }
    #[test]
    fn a_log_message_row() {
        let m = Message::log(
            3,
            "NAV",
            "RTE",
            &[
                Arg::Ascii(b"Route recalculated:"),
                Arg::Uint(3, 4),
                Arg::Ascii(b"km"),
            ],
        );
        assert_eq!(
            render(&m.bytes()),
            "2026-03-14 09:26:53.589793 1234.5678 0 ECU1 NAV RTE 4242 W Route recalculated: 3 km\n"
        );
    }
    #[test]
    fn row_parses_back() {
        let row =
            "2026-03-14 09:26:53.589793 1234.5678 17 ECU1 NAV RTE 4242 W Route recalculated: 3 km";
        let r = parse_row(row).unwrap();
        assert_eq!(
            (
                r.date, r.time, r.uptime, r.count, r.ecu, r.apid, r.ctid, r.session, r.kind,
                r.level, r.payload
            ),
            (
                "2026-03-14",
                "09:26:53.589793",
                "1234.5678",
                "17",
                "ECU1",
                "NAV",
                "RTE",
                "4242",
                "W",
                "W",
                "Route recalculated: 3 km"
            )
        );
        let r = parse_row("2026-03-14 09:26:53.589793 - 0 ECU1 - - - -").unwrap();
        assert_eq!((r.level, r.payload, r.kind), ("U", "", "-"));
        assert_eq!(
            parse_row("2026-03-14 09:26:53.589793 - 0 ECU1 DA1 DC1 - control:response [x]")
                .unwrap()
                .level,
            "U"
        );
        for bad in [
            "!! DLT 메시지가 아닌 데이터 3바이트 · 파일 위치 0: 00 01 02",
            "2026-03-14 09:26:53.589793",
            "2026-03-14 09:26:53.589793 1234.567 0 E A C 1 I x",
            "2026-03-14 09:26:53.589793 - 0 ECU12 A C 1 I x",
            "2026-03-14 09:26:53.589793 - 0 E A C 1 info x",
            "2026-09-30 21:55:53.776261 Runner[007:009] [INFO] [SDK_UI] body",
            "2026-09-21 12:07:30.386123 1234 5678 I Tag: x",
        ] {
            assert!(parse_row(bad).is_none(), "{bad}");
        }
    }
}
