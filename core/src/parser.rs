//! Lexical Android/iOS fields; raw physical rows are never rewritten.
use crate::{dlt, source::Format};
use regex_automata::{
    meta::{Cache, Regex},
    util::captures::Captures,
    Input,
};
use serde_json::{json, Value};
use std::{cell::RefCell, collections::HashSet, sync::LazyLock, thread::LocalKey};

pub const LEVELS: [&str; 8] = ["V", "D", "I", "W", "E", "F", "A", "U"];
const WS: &str = "\\t\\n\\x0b\\x0c\\r \\u{a0}\\u{1680}\\u{2000}-\\u{200a}\\u{2028}\\u{2029}\\u{202f}\\u{205f}\\u{3000}\\u{feff}";
fn is_ws(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}
pub fn trim(s: &str) -> &str {
    s.trim_matches(is_ws)
}
fn terminator(c: char) -> bool {
    matches!(c, '\r' | '\n' | '\u{2028}' | '\u{2029}')
}
// Byte search for `terminator`: CR, LF and the E2 80 A8/A9 encodings of
// U+2028/U+2029. In valid UTF-8, E2 only starts a scalar, so offsets match
// `char_indices`.
fn is_terminator_at(b: &[u8], i: usize) -> bool {
    b[i] != 0xE2 || (b.get(i + 1) == Some(&0x80) && matches!(b.get(i + 2), Some(0xA8 | 0xA9)))
}
fn has_terminator(s: &str) -> bool {
    let b = s.as_bytes();
    memchr::memchr3_iter(b'\r', b'\n', 0xE2, b).any(|i| is_terminator_at(b, i))
}
fn last_terminator(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    memchr::memrchr3_iter(b'\r', b'\n', 0xE2, b).find(|&i| is_terminator_at(b, i))
}
// `threadtime` rows, optionally with the year of `-v year` and the nanosecond
// fraction of `-v nsec`. The date reads `MM-DD` or `YYYY-MM-DD` as two digits,
// an optional `YY-MM`, then `-DD`, and the fraction takes 3-9 digits; each
// step is decided by the next character, so captures keep the regex's
// one-pass engine. `threadtime` rejects 7 and 8 fraction digits.
static ANDROID: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"^[{WS}]*([0-9]{{2}}(?:[0-9]{{2}}-[0-9]{{2}})?-[0-9]{{2}})[{WS}]+([0-9]{{2}}:[0-9]{{2}}:[0-9]{{2}}\.[0-9]{{3,9}})[{WS}]+([0-9]+)[{WS}]+([0-9]+)[{WS}]+([VDIWEFA])([{WS}]+)")).unwrap()
});
// The date and time of `ANDROID` and the whitespace after them, for the
// formats tried after it (`later`).
static STAMP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"^[{WS}]*([0-9]{{2}}(?:[0-9]{{2}}-[0-9]{{2}})?-[0-9]{{2}})[{WS}]+([0-9]{{2}}:[0-9]{{2}}:[0-9]{{2}}\.[0-9]{{3,9}})[{WS}]+")).unwrap()
});
// The seconds and fraction of `-v epoch` (since 1970) and `-v monotonic`
// (since boot), which replace the date and time, and the whitespace after.
static SECONDS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"^[{WS}]*([0-9]+\.[0-9]{{3,9}})[{WS}]+")).unwrap());
// What follows a stamp in `threadtime` rows, as in `ANDROID`: PID, TID,
// level and the whitespace after the level.
static IDS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"^([0-9]+)[{WS}]+([0-9]+)[{WS}]+([VDIWEFA])([{WS}]+)"
    ))
    .unwrap()
});
// `IDS` after the numeric UID or short user name of `-v uid`, followed by
// `:` or whitespace. The UID word ends there, so captures stay one-pass.
static UID_IDS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"^[0-9A-Za-z_]+(?::[{WS}]*|[{WS}]+)([0-9]+)[{WS}]+([0-9]+)[{WS}]+([VDIWEFA])([{WS}]+)"
    ))
    .unwrap()
});
static IOS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"^[{WS}]*([0-9]{{4}}-[0-9]{{2}}-[0-9]{{2}})[\t ]+([0-9]{{2}}:[0-9]{{2}}:[0-9]{{2}}\.[0-9]{{3,6}})[\t ]+")).unwrap()
});
static PROCESS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([^\[\]\r\n]+)\[([0-9]+):([0-9]+)\][\t ]+").unwrap());
/// One leading `[field]`: an iOS level, primary tag or component tag.
static BRACKET: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\[([^\[\]\r\n]+)\][\t ]*").unwrap());
thread_local! {
    static ANDROID_SLOTS: Slots = slots(&ANDROID);
    static STAMP_SLOTS: Slots = slots(&STAMP);
    static SECONDS_SLOTS: Slots = slots(&SECONDS);
    static IDS_SLOTS: Slots = slots(&IDS);
    static UID_IDS_SLOTS: Slots = slots(&UID_IDS);
    static IOS_SLOTS: Slots = slots(&IOS);
    static PROCESS_SLOTS: Slots = slots(&PROCESS);
    static BRACKET_SLOTS: Slots = slots(&BRACKET);
}
/// A thread's search cache and capture slots for one regex.
type Slots = RefCell<(Cache, Captures)>;
fn slots(re: &Regex) -> Slots {
    RefCell::new((re.create_cache(), re.create_captures()))
}
/// The end and group spans of a match whose groups all take part in it.
#[derive(Clone, Copy)]
struct Groups<const N: usize> {
    end: usize,
    spans: [(usize, usize); N],
}
impl<const N: usize> Groups<N> {
    /// Group `i` (from 1) of `s`, the text the match was found in.
    fn text<'a>(&self, s: &'a str, i: usize) -> &'a str {
        let (start, end) = self.spans[i - 1];
        &s[start..end]
    }
}
/// Groups 1 to `N` of `re` in `s`, found with this thread's `slots` for `re`.
/// `regex::Regex::captures` allocates captures on every call and takes its
/// cache from a pool the scan workers share; with every row parsed, both
/// cost scans about as much as the matching itself.
fn read<const N: usize>(re: &Regex, slots: &'static LocalKey<Slots>, s: &str) -> Option<Groups<N>> {
    slots.with_borrow_mut(|(cache, caps)| {
        re.search_captures_with(cache, &Input::new(s), caps);
        let end = caps.get_match()?.end();
        let mut spans = [(0, 0); N];
        for (i, span) in spans.iter_mut().enumerate() {
            let g = caps.get_group(i + 1)?;
            *span = (g.start, g.end);
        }
        Some(Groups { end, spans })
    })
}
fn bracket(s: &str) -> Option<Groups<1>> {
    read(&BRACKET, &BRACKET_SLOTS, s)
}
/// A numbered prefix: optional whitespace, ASCII digits, then tabs or
/// spaces. The number with its leading whitespace, and the prefix's end.
fn prefix(raw: &str) -> Option<(&str, usize)> {
    let b = raw.as_bytes();
    let lead = raw.len() - raw.trim_start_matches(is_ws).len();
    let digits = lead + b[lead..].iter().take_while(|c| c.is_ascii_digit()).count();
    let end = digits
        + b[digits..]
            .iter()
            .take_while(|c| matches!(c, b'\t' | b' '))
            .count();
    (digits > lead && end > digits).then(|| (&raw[..digits], end))
}
/// `s` inside the SGR codes that `logcat -v color` writes around each row:
/// one or more `ESC [ … m` at the start and a reset at the end. A row that
/// does not start with a code is returned whole, so messages keep theirs.
fn uncolored(s: &str) -> &str {
    let mut start = 0;
    while let Some(n) = sgr(&s.as_bytes()[start..]) {
        start += n;
    }
    if start == 0 {
        return s;
    }
    let rest = &s[start..];
    rest.strip_suffix("\x1b[0m")
        .or_else(|| rest.strip_suffix("\x1b[m"))
        .unwrap_or(rest)
}
/// The length of the SGR code (`ESC [`, digits and `;`, `m`) that starts `b`.
fn sgr(b: &[u8]) -> Option<usize> {
    let params = b.strip_prefix(b"\x1b[")?;
    let n = params
        .iter()
        .take_while(|c| c.is_ascii_digit() || **c == b';')
        .count();
    (params.get(n) == Some(&b'm')).then_some(n + 3)
}
fn ios_level(s: &str) -> Option<&'static str> {
    let s = trim(s);
    if s.is_ascii() {
        // ASCII uppercasing equals Unicode uppercasing; skip the allocation.
        const NAMES: [(&str, &str); 16] = [
            ("V", "V"),
            ("VERBOSE", "V"),
            ("D", "D"),
            ("DEBUG", "D"),
            ("I", "I"),
            ("INFO", "I"),
            ("W", "W"),
            ("WARN", "W"),
            ("WARNING", "W"),
            ("E", "E"),
            ("ERROR", "E"),
            ("F", "F"),
            ("FATAL", "F"),
            // Apple's most severe level.
            ("FAULT", "F"),
            ("A", "A"),
            ("ASSERT", "A"),
        ];
        return NAMES
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(s))
            .map(|(_, level)| *level);
    }
    // Non-ASCII text such as dotless i or long s can uppercase to a level name.
    match s.to_uppercase().as_str() {
        "V" | "VERBOSE" => Some("V"),
        "D" | "DEBUG" => Some("D"),
        "I" | "INFO" => Some("I"),
        "W" | "WARN" | "WARNING" => Some("W"),
        "E" | "ERROR" => Some("E"),
        "F" | "FATAL" | "FAULT" => Some("F"),
        "A" | "ASSERT" => Some("A"),
        _ => None,
    }
}
#[derive(Debug)]
pub struct Parsed<'a> {
    pub date: &'a str,
    pub time: &'a str,
    pub pid: &'a str,
    pub tid: &'a str,
    pub level: &'a str,
    pub tag: &'a str,
    pub message: &'a str,
    /// iOS primary and component tags. Android rows use `tag_list`, which
    /// avoids allocating a one-element list for every parsed row.
    pub tags: Vec<&'a str>,
    pub process: Option<&'a str>,
    pub level_text: Option<&'a str>,
    pub ios: bool,
    /// Fields of a decoded DLT message row; `tag` is its APID and `tags` its
    /// ECU, APID and CTID.
    pub dlt: Option<DltFields<'a>>,
}
/// DLT row fields without a `Parsed` counterpart; absent values are "".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DltFields<'a> {
    pub uptime: &'a str,
    pub count: &'a str,
    pub ecu: &'a str,
    pub apid: &'a str,
    pub ctid: &'a str,
    pub session: &'a str,
    /// Log level letter, or `type:subtype` of other messages.
    pub kind: &'a str,
}
impl<'a> Parsed<'a> {
    /// Tags available to tag filtering.
    pub fn tag_list(&self) -> &[&'a str] {
        if self.ios || self.dlt.is_some() {
            &self.tags
        } else {
            std::slice::from_ref(&self.tag)
        }
    }
    /// Fields a page shows, keyed as its `fieldOffsets`, in row order except
    /// Android Studio's process, which follows the tag. Absent fields are
    /// static empty strings, not slices of the row.
    pub fn spans(&self) -> impl Iterator<Item = (&'static str, &'a str)> {
        match &self.dlt {
            // The cells of a DLT row; the APID cell shows `tag`.
            Some(d) => [
                ("date", self.date),
                ("time", self.time),
                ("uptime", d.uptime),
                ("ecu", d.ecu),
                ("apid", d.apid),
                ("ctid", d.ctid),
                ("message", self.message),
            ],
            None => [
                ("date", self.date),
                ("time", self.time),
                ("process", self.process.unwrap_or("")),
                ("pid", self.pid),
                ("tid", self.tid),
                ("tag", self.tag),
                ("message", self.message),
            ],
        }
        .into_iter()
    }
}
#[derive(Debug)]
pub struct Record<'a> {
    pub raw: &'a str,
    pub line: u64,
    pub kind: &'static str,
    pub parsed: Option<Parsed<'a>>,
    pub source_number: Option<&'a str>,
}

fn android(s: &str) -> Option<Parsed<'_>> {
    threadtime(s, &read(&ANDROID, &ANDROID_SLOTS, s)?)
}
/// The number of fraction digits of a stamp's time. logcat prints 3 (ms),
/// 6 (us) or 9 (ns); the regexes take 3 to 9, and rows with 7 or 8 are raw.
fn fraction(time: &str) -> usize {
    time.rfind('.').map_or(0, |i| time.len() - i - 1)
}
/// A `threadtime` row whose header matched `p`, a match of `ANDROID` or
/// one composed by `threadtime_at`: date, time, PID, TID, level and the
/// whitespace after the level.
fn threadtime<'a>(s: &'a str, p: &Groups<6>) -> Option<Parsed<'a>> {
    if matches!(fraction(p.text(s, 2)), 7 | 8) {
        return None;
    }
    let start = p.end;
    let last = last_terminator(&s[start..]).map(|i| start + i);
    let min = last
        .and_then(|i| s[..i].char_indices().last().map(|(j, _)| j))
        .unwrap_or(0)
        .max(start + s[start..].chars().next().map(char::len_utf8).unwrap_or(0));
    let colon = s.get(min..).and_then(|v| v.find(':')).map(|i| min + i);
    let candidate = colon.map(|i| s[start..i].trim_end_matches(is_ws));
    let (colon, tag) = if let (Some(i), Some(tag)) = (colon, candidate) {
        if !tag.is_empty() && !tag.chars().any(terminator) {
            (i, tag)
        } else {
            fallback(s, p, start, last)?
        }
    } else {
        fallback(s, p, start, last)?
    };
    let mut message = colon + 1;
    if let Some(c) = s[message..].chars().next() {
        if is_ws(c) {
            message += c.len_utf8();
        }
    }
    Some(Parsed {
        date: p.text(s, 1),
        time: p.text(s, 2),
        pid: p.text(s, 3),
        tid: p.text(s, 4),
        level: p.text(s, 5),
        tag,
        message: &s[message..],
        tags: Vec::new(),
        process: None,
        level_text: None,
        ios: false,
        dlt: None,
    })
}
fn fallback<'a>(
    s: &'a str,
    p: &Groups<6>,
    start: usize,
    last: Option<usize>,
) -> Option<(usize, &'a str)> {
    if s.as_bytes().get(start) != Some(&b':') || last.is_some_and(|i| i > start + 1) {
        return None;
    }
    let first = p.text(s, 6).chars().next()?.len_utf8();
    if !s[p.spans[5].0 + first..start]
        .chars()
        .any(|c| !terminator(c))
    {
        return None;
    }
    Some((start, ""))
}
fn ios(s: &str) -> Option<Parsed<'_>> {
    let stamp = read::<2>(&IOS, &IOS_SLOTS, s)?;
    let mut body = &s[stamp.end..];
    // `PROCESS` and `BRACKET` both need a `[`; other dated rows, such as
    // Android ones, skip their scans.
    if body.is_empty() || has_terminator(body) || memchr::memchr(b'[', body.as_bytes()).is_none() {
        return None;
    }
    let (mut process, mut pid, mut tid) = ("", "", "");
    let mut context = false;
    if let Some(p) = read::<3>(&PROCESS, &PROCESS_SLOTS, body) {
        process = trim(p.text(body, 1));
        if process.is_empty() {
            return None;
        }
        pid = p.text(body, 2);
        tid = p.text(body, 3);
        // Terminators were rejected above, so the remainder is the message.
        body = &body[p.end..];
        context = true;
    }
    let first = bracket(body);
    let mut level_text = first.map(|p| p.text(body, 1));
    let mut supplied = level_text.and_then(ios_level);
    let primary = if supplied.is_some() {
        body = &body[first?.end..];
        bracket(body)
    } else {
        level_text = None;
        first
    };
    if !context && supplied.is_none() {
        return None;
    }
    let (tag, message) = if let Some(primary) = primary {
        let tag = trim(primary.text(body, 1));
        if tag.is_empty() {
            return None;
        }
        (tag, &body[primary.end..])
    } else if context && !trim(body).is_empty() && !trim(body).starts_with('[') {
        // A plain message requires the existing complete process/ID header.
        // Do not reinterpret malformed bracket fields as an absent tag.
        ("", body)
    } else {
        return None;
    };
    let mut tags = Vec::new();
    if !tag.is_empty() {
        tags.push(tag);
    }
    // Rows usually carry a few tags; a set is built only for long lists.
    let mut seen: Option<HashSet<&str>> = None;
    let mut rest = message;
    while let Some(p) = bracket(rest) {
        let text = trim(p.text(rest, 1));
        // Some loggers put the level after their own tags, as in
        // `[plugin][Render][INFO]`. Only a full level name counts
        // there: one-letter fields such as `[A]` are usually components.
        if supplied.is_none() && text.chars().nth(1).is_some() {
            if let Some(level) = ios_level(text) {
                supplied = Some(level);
                level_text = Some(p.text(rest, 1));
                rest = &rest[p.end..];
                continue;
            }
        }
        let new = !text.is_empty()
            && match &mut seen {
                Some(seen) => seen.insert(text),
                None if tags.len() < 16 => !tags.contains(&text),
                None => seen.insert(tags.iter().copied().collect()).insert(text),
            };
        if new {
            tags.push(text);
        }
        rest = &rest[p.end..];
    }
    Some(Parsed {
        date: stamp.text(s, 1),
        time: stamp.text(s, 2),
        pid,
        tid,
        level: supplied.unwrap_or("U"),
        tag,
        message,
        tags,
        process: Some(process),
        level_text,
        ios: true,
        dlt: None,
    })
}
/// Formats tried only after `threadtime` and iOS fail, so a row those parse
/// keeps its fields: rows that start with a stamp (`stamped`), `-v long`
/// headers, `brief`, and iPhone syslog rows. `lead` is the row after leading
/// whitespace; its first bytes rule out other rows before any regex runs.
/// Rows with line terminators stay raw.
#[inline(never)]
fn later<'a>(s: &'a str, lead: &[u8]) -> Option<Parsed<'a>> {
    match lead.first()? {
        b'0'..=b'9' => stamped(s),
        b'[' => long(s, s.len() - lead.len()),
        _ if lead.get(1) == Some(&b'/') => {
            if has_terminator(s) {
                return None;
            }
            slashed(s, s.len() - lead.len(), "", "")
        }
        _ => syslog(s, s.len() - lead.len()),
    }
}
/// A stamp's date and time spans; `-v epoch` and `-v monotonic` have an
/// empty date. `end` is after the stamp and its whitespace; `plain` stamps
/// are those of `ANDROID`, without the zone of `-v zone`.
struct Stamp {
    date: (usize, usize),
    time: (usize, usize),
    end: usize,
    plain: bool,
}
fn stamp_at(s: &str, at: usize) -> Option<Stamp> {
    let shift = |(a, b): (usize, usize)| (at + a, at + b);
    let stamp = if let Some(p) = read::<2>(&STAMP, &STAMP_SLOTS, &s[at..]) {
        let end = at + p.end;
        let zoned = zone(s, end);
        Stamp {
            date: shift(p.spans[0]),
            time: shift(p.spans[1]),
            end: zoned.unwrap_or(end),
            plain: zoned.is_none(),
        }
    } else {
        let p = read::<1>(&SECONDS, &SECONDS_SLOTS, &s[at..])?;
        let time = shift(p.spans[0]);
        Stamp {
            date: (time.0, time.0),
            time,
            end: at + p.end,
            plain: false,
        }
    };
    (!matches!(fraction(&s[stamp.time.0..stamp.time.1]), 7 | 8)).then_some(stamp)
}
/// The offset after the `+0900` that `-v zone` puts after the time at `at`,
/// and after the whitespace that follows it.
fn zone(s: &str, at: usize) -> Option<usize> {
    let b = s.as_bytes();
    let digits = b.get(at + 1..at + 5)?;
    if !matches!(b[at], b'+' | b'-') || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let rest = s[at + 5..].trim_start_matches(is_ws);
    (rest.len() < s.len() - at - 5).then(|| s.len() - rest.len())
}
/// Rows that start with a stamp but are no `ANDROID` row: `-v time` (also
/// with the year, fraction, zone, epoch, monotonic and `uid` options),
/// `threadtime` with `uid` or with a zone, epoch or monotonic stamp, and
/// Android Studio's Logcat window.
fn stamped(s: &str) -> Option<Parsed<'_>> {
    let stamp = stamp_at(s, 0)?;
    if has_terminator(s) {
        return None;
    }
    let text = |(a, b): (usize, usize)| &s[a..b];
    let (at, date, time) = (stamp.end, text(stamp.date), text(stamp.time));
    // `L/` starts `-v time`. Otherwise the UID form has three IDs before
    // the level and Android Studio's IDs are followed by two words.
    if let Some(p) = slashed(s, at, date, time) {
        return Some(p);
    }
    // IDs need `:` or whitespace after their first word; Android Studio's
    // `PID-TID` skips those regexes.
    let b = s.as_bytes();
    let w = at
        + b[at..]
            .iter()
            .take_while(|c| c.is_ascii_alphanumeric() || **c == b'_')
            .count();
    let colon = b[w..].starts_with(b":");
    if w > at && (colon || s[w..].starts_with(is_ws)) {
        if let Some(p) = threadtime_at(s, &stamp, colon) {
            return Some(p);
        }
    }
    // Android Studio prints neither zones nor seconds.
    stamp.plain.then(|| studio(s, at, date, time)).flatten()
}
/// `threadtime` after `stamp`: IDs, with or without the UID of `-v uid`,
/// level and tag. `ANDROID` already read plain stamps with their IDs.
/// logcat ends the UID with `:` (`colon`). Plain stamps also take
/// whitespace there, as before; zone, epoch and monotonic stamps need the
/// `:`, so that continuation rows such as `1.500 s  2 3 E x: y` stay raw.
fn threadtime_at<'a>(s: &'a str, stamp: &Stamp, colon: bool) -> Option<Parsed<'a>> {
    let rest = &s[stamp.end..];
    let uid = || read::<4>(&UID_IDS, &UID_IDS_SLOTS, rest);
    let ids = if stamp.plain {
        uid()
    } else {
        read::<4>(&IDS, &IDS_SLOTS, rest).or_else(|| colon.then(uid).flatten())
    }?;
    let [pid, tid, level, gap] = ids.spans.map(|(a, b)| (stamp.end + a, stamp.end + b));
    let p = Groups {
        end: stamp.end + ids.end,
        spans: [stamp.date, stamp.time, pid, tid, level, gap],
    };
    threadtime(s, &p)
}
/// The header row of `-v long`, `[ stamp PID:TID L/tag ]` from `at`, with
/// any stamp of `stamped` and the UID of `-v uid` before the PID. The
/// message is on the rows after it, which continue its entry, so the
/// header's message is empty.
fn long(s: &str, at: usize) -> Option<Parsed<'_>> {
    let b = s.as_bytes();
    if !s[at + 1..].starts_with(is_ws) {
        return None;
    }
    let stamp = stamp_at(s, at + 1)?;
    if has_terminator(s) {
        return None;
    }
    // `UID:PID:TID` or `PID:TID`: words that end at `:`, then the TID.
    let mut ids = [(0, 0); 3];
    let (mut n, mut at) = (0, stamp.end);
    loop {
        let w = at
            + b[at..]
                .iter()
                .take_while(|c| c.is_ascii_alphanumeric() || **c == b'_')
                .count();
        if w == at || n == ids.len() {
            return None;
        }
        ids[n] = (at, w);
        n += 1;
        if b.get(w) != Some(&b':') {
            at = w;
            break;
        }
        at = s.len() - s[w + 1..].trim_start_matches(is_ws).len();
    }
    let number = |(a, b): (usize, usize)| s[a..b].bytes().all(|c| c.is_ascii_digit());
    if n < 2 || !number(ids[n - 2]) || !number(ids[n - 1]) {
        return None;
    }
    let level = s.len() - s[at..].trim_start_matches(is_ws).len();
    if level == at
        || !matches!(
            b.get(level)?,
            b'V' | b'D' | b'I' | b'W' | b'E' | b'F' | b'A'
        )
        || b.get(level + 1) != Some(&b'/')
    {
        return None;
    }
    let tag = s[level + 2..].trim_end_matches(is_ws).strip_suffix(']')?;
    let text = |(a, b): (usize, usize)| &s[a..b];
    Some(Parsed {
        date: text(stamp.date),
        time: text(stamp.time),
        pid: text(ids[n - 2]),
        tid: text(ids[n - 1]),
        level: &s[level..level + 1],
        tag: trim(tag),
        message: &s[s.len()..],
        tags: Vec::new(),
        process: None,
        level_text: None,
        ios: false,
        dlt: None,
    })
}
const MONTHS: [&[u8]; 12] = [
    b"Jan", b"Feb", b"Mar", b"Apr", b"May", b"Jun", b"Jul", b"Aug", b"Sep", b"Oct", b"Nov", b"Dec",
];
/// Whether `b` starts with the `Mmm dd HH:MM:SS ` stamp of BSD syslog; the
/// day is padded with a space.
fn syslog_stamp(b: &[u8]) -> bool {
    let digits = |r: std::ops::Range<usize>| b[r].iter().all(u8::is_ascii_digit);
    b.len() > 16
        && MONTHS.contains(&&b[..3])
        && b[3] == b' '
        && (b[4] == b' ' || b[4].is_ascii_digit())
        && b[5].is_ascii_digit()
        && b[6] == b' '
        && digits(7..9)
        && b[9] == b':'
        && digits(10..12)
        && b[12] == b':'
        && digits(13..15)
        && b[15] == b' '
}
/// Apple's level for a syslog priority name, as `idevicesyslog` prints it.
fn syslog_level(name: &str) -> Option<&'static str> {
    const NAMES: [(&str, &str); 10] = [
        ("Debug", "D"),
        ("Info", "I"),
        // os_log's default level.
        ("Notice", "U"),
        ("Default", "U"),
        ("Warning", "W"),
        ("Error", "E"),
        ("Fault", "F"),
        ("Critical", "F"),
        ("Alert", "F"),
        ("Emergency", "F"),
    ];
    NAMES
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, level)| *level)
}
/// An iPhone or iPad syslog row from `at`, as `idevicesyslog` prints it:
/// `Mmm dd HH:MM:SS device process(library)[PID] <Level>: message`. The
/// device name stays in the raw row; the library, when present, is the tag.
/// The header ends at the first `] <Level>:`. The library is the
/// parenthesized text just before its `[PID`, spaces allowed, and the
/// process is the word before the library or the `[`.
fn syslog(s: &str, at: usize) -> Option<Parsed<'_>> {
    let b = &s.as_bytes()[at..];
    if !syslog_stamp(b) || has_terminator(s) {
        return None;
    }
    let (date, time, head) = (&s[at..at + 6], &s[at + 7..at + 15], at + 16);
    for close in memchr::memmem::find_iter(&b[16..], b"] <").map(|i| head + i) {
        let word = &s[close + 3..];
        let n = word.bytes().take_while(u8::is_ascii_alphabetic).count();
        let Some(level) = syslog_level(&word[..n]) else {
            continue;
        };
        if !word[n..].starts_with(">:") {
            continue;
        }
        let open = s[head..close].rfind('[').map(|i| head + i)?;
        let pid = &s[open + 1..close];
        let named = &s[head..open];
        let (named, library) = match named.strip_suffix(')').and_then(|n| n.rsplit_once('(')) {
            Some((named, library)) => (named, trim(library)),
            None => (named, ""),
        };
        let (device, process) = named.rsplit_once(' ')?;
        if pid.is_empty() || !pid.bytes().all(|c| c.is_ascii_digit()) || trim(device).is_empty() {
            return None;
        }
        if process.is_empty() || process.contains(['(', ')', '[', ']']) {
            return None;
        }
        let mut message = close + 3 + n + 2;
        if s[message..].starts_with(' ') {
            message += 1;
        }
        return Some(Parsed {
            date,
            time,
            pid,
            tid: "",
            level,
            tag: library,
            message: &s[message..],
            tags: if library.is_empty() {
                Vec::new()
            } else {
                vec![library]
            },
            process: Some(process),
            level_text: Some(&word[..n]),
            ios: true,
            dlt: None,
        });
    }
    None
}
/// `L/TAG(PID): message` from `at`: a `brief` row, or a `-v time` row after
/// its stamp. The tag runs to the first `(` that opens a PID group, without
/// its padding; logcat puts one space before the message.
fn slashed<'a>(s: &'a str, at: usize, date: &'a str, time: &'a str) -> Option<Parsed<'a>> {
    let b = s.as_bytes();
    if !matches!(b.get(at)?, b'V' | b'D' | b'I' | b'W' | b'E' | b'F' | b'A')
        || b.get(at + 1) != Some(&b'/')
    {
        return None;
    }
    let start = at + 2;
    memchr::memchr_iter(b'(', &b[start..]).find_map(|i| {
        let (pid, end) = pid_group(s, start + i + 1)?;
        let mut message = end;
        if let Some(c) = s[end..].chars().next().filter(|&c| is_ws(c)) {
            message += c.len_utf8();
        }
        Some(Parsed {
            date,
            time,
            pid,
            tid: "",
            level: &s[at..at + 1],
            tag: trim(&s[start..start + i]),
            message: &s[message..],
            tags: Vec::new(),
            process: None,
            level_text: None,
            ios: false,
            dlt: None,
        })
    })
}
/// `( 2983):` from just after `(`, or `(10123: 2983):` with `-v uid`, whose
/// UID may also be a short user name or end with whitespace: the PID and
/// the offset after the colon. `%5d` pads the IDs with spaces.
fn pid_group(s: &str, at: usize) -> Option<(&str, usize)> {
    let b = s.as_bytes();
    let skip = |i: usize| s.len() - s[i..].trim_start_matches(is_ws).len();
    let run = |i: usize, f: fn(&u8) -> bool| i + b[i..].iter().take_while(|c| f(c)).count();
    let mut i = skip(at);
    let word = run(i, |c| c.is_ascii_alphanumeric() || *c == b'_');
    if word > i && b.get(word) == Some(&b':') {
        i = skip(word + 1);
    } else if word > i && skip(word) > word {
        i = skip(word);
    }
    let end = run(i, u8::is_ascii_digit);
    (end > i && b.get(end..end + 2) == Some(b"):")).then(|| (&s[i..end], end + 2))
}
/// The range of the next whitespace-separated word of `s` from `at`.
/// Spaces and printable ASCII are skipped by byte before the `is_ws` scans.
fn word(s: &str, at: usize) -> Option<(usize, usize)> {
    let b = s.as_bytes();
    let at = at + b[at..].iter().take_while(|&&c| c == b' ').count();
    let rest = s[at..].trim_start_matches(is_ws);
    let start = s.len() - rest.len();
    let ascii = rest.bytes().take_while(u8::is_ascii_graphic).count();
    let end = rest[ascii..]
        .find(is_ws)
        .map_or(s.len(), |i| start + ascii + i);
    (end > start).then_some((start, end))
}
/// Android Studio's Logcat window after a dated stamp: `PID-TID` or `PID`,
/// tag, package or process name, level, message, in padded columns. Exactly
/// two words sit between the IDs and the level, so rows with a hidden or
/// blanked column, a tag with spaces or an extra column stay raw. The
/// package goes to `process`, as iOS process names do.
fn studio<'a>(s: &'a str, at: usize, date: &'a str, time: &'a str) -> Option<Parsed<'a>> {
    if date.len() != 10 {
        return None;
    }
    let mut words = [(0, 0); 4];
    let mut end = at;
    for w in &mut words {
        *w = word(s, end)?;
        end = w.1;
    }
    let [ids, tag, process, level] = words.map(|(a, b)| &s[a..b]);
    let number = |v: &str| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit());
    let (pid, tid) = match ids.split_once('-') {
        Some((pid, tid)) if number(tid) => (pid, tid),
        Some(_) => return None,
        None => (ids, ""),
    };
    if !number(pid)
        || !matches!(
            level.as_bytes(),
            [b'V' | b'D' | b'I' | b'W' | b'E' | b'F' | b'A']
        )
    {
        return None;
    }
    // Android Studio prints two spaces after the level.
    let mut message = end;
    for c in s[end..].chars().take(2).take_while(|&c| is_ws(c)) {
        message += c.len_utf8();
    }
    Some(Parsed {
        date,
        time,
        pid,
        tid,
        level,
        tag,
        message: &s[message..],
        tags: Vec::new(),
        process: Some(process),
        level_text: None,
        ios: false,
        dlt: None,
    })
}
/// Android or iOS fields. Only a row that starts with a year can be iOS or
/// dated Android; iOS keeps precedence there, as before dated Android rows
/// were recognized. Other rows need only the Android attempt. Formats added
/// later are tried last (`later`). The color codes of `logcat -v color` stay
/// in the raw row, outside the fields.
fn fields(s: &str) -> Option<Parsed<'_>> {
    let s = uncolored(s);
    let b = s.trim_start_matches(is_ws).as_bytes();
    let known = if b.len() > 4 && b[4] == b'-' && b[..4].iter().all(u8::is_ascii_digit) {
        ios(s).or_else(|| android(s))
    } else {
        android(s)
    };
    known.or_else(|| later(s, b))
}
// A number is a prefix only when the entire remainder is a supported log row.
// Return the validated record so optional summaries do not parse it twice.
pub(crate) fn parse_numbered_line(raw: &str, line: u64) -> Option<Record<'_>> {
    let (number, end) = prefix(raw)?;
    let parsed = fields(&raw[end..])?;
    Some(Record {
        raw,
        line,
        kind: "parsed",
        parsed: Some(parsed),
        source_number: Some(trim(number)),
    })
}
/// Whether a row without fields still opens its own log entry: it starts
/// like a log row, with a date and time (`MM-DD HH:MM` or `YYYY-MM-DD HH:MM`,
/// also after the `[ ` of a `-v long` header, or syslog's `Mmm dd HH:MM:SS`;
/// also after a numbered prefix when prefixes are recognized, and inside
/// color codes), but the rest is malformed. Any other row without fields
/// continues the entry before it. Epoch and monotonic seconds alone do not
/// open entries: continuation rows often start with a number.
pub fn starts_entry(raw: &str, recognize: bool) -> bool {
    fn stamp(s: &str) -> bool {
        let s = uncolored(s).trim_start_matches(is_ws);
        let s = s
            .strip_prefix('[')
            .filter(|r| r.starts_with(is_ws))
            .unwrap_or(s);
        let b = s.trim_start_matches(is_ws).as_bytes();
        if syslog_stamp(b) {
            return true;
        }
        let digits = |at: usize, n: usize| {
            b.get(at..at + n)
                .is_some_and(|d| d.iter().all(u8::is_ascii_digit))
        };
        let at = if digits(0, 4) && b.get(4) == Some(&b'-') {
            5
        } else {
            0
        };
        if !(digits(at, 2) && b.get(at + 2) == Some(&b'-') && digits(at + 3, 2)) {
            return false;
        }
        let mut at = at + 5;
        let gap = at;
        while matches!(b.get(at), Some(b' ' | b'\t')) || (at == gap && b.get(at) == Some(&b'T')) {
            at += 1;
        }
        at > gap && digits(at, 2) && b.get(at + 2) == Some(&b':') && digits(at + 3, 2)
    }
    // Stamps start with a digit, `[`, a month or a color code; most
    // continuation rows do not.
    let lead = raw.trim_start_matches(is_ws).as_bytes().first();
    matches!(lead, Some(b'0'..=b'9' | b'[' | b'A'..=b'S' | 0x1b))
        && (stamp(raw) || (recognize && prefix(raw).is_some_and(|(_, end)| stamp(&raw[end..]))))
}
pub fn has_source_prefix(raw: &str) -> bool {
    parse_numbered_line(raw, 0).is_some()
}
pub fn parse_line(raw: &str, line: u64, recognize: bool) -> Record<'_> {
    if recognize {
        if let Some(record) = parse_numbered_line(raw, line) {
            return record;
        }
    }
    let parsed = fields(raw);
    let kind = if parsed.is_some() {
        "parsed"
    } else if trim(raw).is_empty() {
        "blank"
    } else {
        "raw"
    };
    Record {
        raw,
        line,
        kind,
        parsed,
        source_number: None,
    }
}
/// A row read in its dataset's format. Text rows are Android or iOS rows
/// (`parse_line`); a DLT row is never read as one.
/// Whether text starting with `head` is an iOS log: its rows that parse,
/// numbered or not, are iOS rows at least as often as others. Such logs get
/// Mac OS Roman mojibake repaired (`encoding::repair`).
pub fn ios_log(head: &[u8]) -> bool {
    let (mut ios, mut other) = (0usize, 0usize);
    for (_, bytes) in crate::source::ByteLines::new(head) {
        let raw = crate::encoding::decode(bytes);
        match parse_line(&raw, 0, true).parsed {
            Some(p) if p.ios => ios += 1,
            Some(_) => other += 1,
            None => {}
        }
    }
    ios > 0 && ios >= other
}
pub fn parse_in(format: Format, raw: &str, line: u64, recognize: bool) -> Record<'_> {
    match format {
        Format::Text => parse_line(raw, line, recognize),
        // Rows of units that are no message (`!! …`) have no fields. DLT
        // rows have no numbered prefixes.
        Format::Dlt => {
            let parsed = dlt::parse_row(raw).map(dlt_fields);
            Record {
                raw,
                line,
                kind: if parsed.is_some() {
                    "parsed"
                } else if trim(raw).is_empty() {
                    "blank"
                } else {
                    "raw"
                },
                parsed,
                source_number: None,
            }
        }
    }
}
fn dlt_fields(r: dlt::Row<'_>) -> Parsed<'_> {
    let field = |s: &'static str, v| if dlt::absent(v) { s } else { v };
    let (ecu, apid, ctid) = (field("", r.ecu), field("", r.apid), field("", r.ctid));
    let mut tags = Vec::with_capacity(3);
    for t in [ecu, apid, ctid] {
        if !t.is_empty() && !tags.contains(&t) {
            tags.push(t);
        }
    }
    Parsed {
        date: r.date,
        time: r.time,
        pid: "",
        tid: "",
        level: r.level,
        tag: apid,
        message: r.payload,
        tags,
        process: None,
        level_text: None,
        ios: false,
        dlt: Some(DltFields {
            uptime: field("", r.uptime),
            count: r.count,
            ecu,
            apid,
            ctid,
            session: field("", r.session),
            kind: field("", r.kind),
        }),
    }
}
/// `parse_numbered_line` in a dataset's format; DLT rows have no prefixes.
pub(crate) fn parse_numbered_in(format: Format, raw: &str, line: u64) -> Option<Record<'_>> {
    match format {
        Format::Text => parse_numbered_line(raw, line),
        Format::Dlt => None,
    }
}
/// Whether a row without fields opens its own log entry in its dataset's
/// format (`starts_entry` for text). Every DLT row is one message, its own
/// entry.
pub fn starts_entry_in(format: Format, raw: &str, recognize: bool) -> bool {
    match format {
        Format::Text => starts_entry(raw, recognize),
        Format::Dlt => true,
    }
}
pub fn preview(s: &str, max: usize) -> &str {
    s.char_indices().nth(max).map(|(i, _)| &s[..i]).unwrap_or(s)
}
impl Record<'_> {
    pub fn json(&self, max: usize) -> Value {
        let mut v = json!({"lineNumber":self.line,"raw":preview(self.raw,max),"kind":self.kind});
        if self.raw.chars().count() > max {
            v["truncated"] = json!(true);
            v["rawBytes"] = json!(self.raw.len());
        }
        if let Some(p) = &self.parsed {
            for (k, text) in [
                ("date", p.date),
                ("time", p.time),
                ("pid", p.pid),
                ("tid", p.tid),
                ("level", p.level),
                ("tag", p.tag),
                ("message", p.message),
            ] {
                v[k] = json!(preview(text, max));
            }
            v["sourceLineNumber"] = json!(self.source_number);
            if let Some(d) = &p.dlt {
                v["dlt"] = json!({"uptime":d.uptime,"count":d.count,"ecu":d.ecu,"apid":d.apid,"ctid":d.ctid,"session":d.session,"kind":d.kind});
                v["tags"] = json!(p.tags);
            } else if p.ios {
                // Syslog rows have no year; `null` still marks an iOS row,
                // which the page lays out by `year`.
                v["year"] = json!(p
                    .date
                    .get(..4)
                    .filter(|y| y.bytes().all(|c| c.is_ascii_digit())));
                v["process"] = json!(p.process.unwrap_or(""));
                v["levelText"] = json!(p.level_text);
                v["tags"] = json!(p
                    .tags
                    .iter()
                    .take(64)
                    .map(|s| preview(s, max.min(256)))
                    .collect::<Vec<_>>());
            } else if let Some(process) = p.process {
                // Android Studio's package or process column.
                v["process"] = json!(preview(process, max));
            }
        }
        v
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lexical_android() {
        let r = parse_line("09-21 12:07:30.386 007 009 I Full.Tag: 한글 🙂", 1, false);
        let p = r.parsed.unwrap();
        assert_eq!(p.pid, "007");
        assert_eq!(p.tid, "009");
        assert_eq!(p.tag, "Full.Tag");
        assert_eq!(p.message, "한글 🙂");
    }
    #[test]
    fn ios_multiple_tags() {
        let r = parse_line(
            "2026-09-30 21:55:53.776261 Runner[007:009] [INFO] [SDK_UI] [GalleryManager] body",
            2,
            false,
        );
        let p = r.parsed.unwrap();
        assert_eq!(p.tags, vec!["SDK_UI", "GalleryManager"]);
        assert_eq!(p.level, "I");
        assert_eq!(p.message, "[GalleryManager] body");
    }
    #[test]
    fn ios_level_after_leading_tags() {
        let raw = "2026-09-30 21:55:53.776 Runner[7:9] [demo_plugin][Render][INFO] [ViewRegistry] view attached: MainView";
        let p = parse_line(raw, 1, false).parsed.unwrap();
        assert_eq!(p.level, "I");
        assert_eq!(p.level_text, Some("INFO"));
        assert_eq!(p.tag, "demo_plugin");
        assert_eq!(p.tags, vec!["demo_plugin", "Render", "ViewRegistry"]);
        assert_eq!(
            p.message,
            "[Render][INFO] [ViewRegistry] view attached: MainView"
        );
        for (raw, level, tags) in [
            // A leading level still wins; a later name stays a tag.
            (
                "2026-09-30 21:55:53.776 R[1:2] [ERROR] [Tag] [INFO] m",
                "E",
                vec!["Tag", "INFO"],
            ),
            // The first later level counts; later ones are tags.
            (
                "2026-09-30 21:55:53.776 R[1:2] [Tag] [ warning ][Error] m",
                "W",
                vec!["Tag", "Error"],
            ),
            // One letter is a component, and text ends the tag fields.
            (
                "2026-09-30 21:55:53.776 R[1:2] [Root][A][I] m",
                "U",
                vec!["Root", "A", "I"],
            ),
            (
                "2026-09-30 21:55:53.776 R[1:2] [Root] m [INFO]",
                "U",
                vec!["Root"],
            ),
        ] {
            let p = parse_line(raw, 1, false).parsed.unwrap();
            assert_eq!((p.level, p.tags), (level, tags), "{raw}");
        }
        // Without a process header the level must come first, as before.
        assert!(
            parse_line("2026-09-30 21:55:53.776 [Tag] [INFO] m", 1, false)
                .parsed
                .is_none()
        );
    }
    #[test]
    fn ios_logs_are_told_by_their_parsed_rows() {
        let ios = "2026-09-30 21:55:53.776 Runner[1:2] [INFO] [Tag] m\n";
        let android = "09-30 21:55:53.776  1  2 I Tag: m\n";
        assert!(ios_log(ios.as_bytes()));
        assert!(ios_log(
            format!("--- header\n{ios}  continued\r\n{ios}").as_bytes()
        ));
        assert!(ios_log(format!("{ios}{android}").as_bytes()), "a tie");
        assert!(ios_log(format!("0001 {ios}").as_bytes()), "numbered rows");
        assert!(!ios_log(format!("{ios}{android}{android}").as_bytes()));
        assert!(!ios_log(android.as_bytes()));
        assert!(!ios_log(b"plain text\nwithout rows\n"));
        assert!(!ios_log(b""));
    }
    #[test]
    fn process_unknown() {
        assert_eq!(
            parse_line(
                "2026-09-30 21:55:51.494 Runner[31457:5203417] [Memory] line",
                3,
                false
            )
            .parsed
            .unwrap()
            .level,
            "U"
        );
    }
    #[test]
    fn ios_levels_keep_unicode_uppercasing() {
        for (text, level) in [
            ("Info", Some("I")),
            ("wArNiNg", Some("W")),
            ("ınfo", Some("I")),
            ("aſſert", Some("A")),
            ("verboſe", Some("V")),
            ("Fault", Some("F")),
            ("notice", None),
            ("ＩＮＦＯ", None),
        ] {
            assert_eq!(ios_level(text), level, "{text}");
        }
    }
    #[test]
    fn many_component_tags_keep_first_occurrence_order() {
        let body: String = (0..40).map(|i| format!("[c{}]", i % 25)).collect();
        let raw = format!("2026-09-30 21:55:53.776 R[1:2] [ERROR] [c3] {body}");
        let r = parse_line(&raw, 1, false);
        let tags = r.parsed.unwrap().tags;
        let mut expected = vec!["c3".to_string()];
        expected.extend((0..25).filter(|i| *i != 3).map(|i| format!("c{i}")));
        assert_eq!(tags, expected);
    }
    #[test]
    fn android_tag_list_needs_no_allocation() {
        let r = parse_line("09-21 12:07:30.386 007 009 I Tag: body", 1, false);
        let p = r.parsed.unwrap();
        assert!(p.tags.is_empty());
        assert_eq!(p.tag_list(), ["Tag"]);
    }
    #[test]
    fn terminator_search_matches_char_scan() {
        for s in [
            "",
            "a\u{2028}b",
            "…\u{2029}",
            "\u{2026}\u{2027}",
            "x\ry",
            "한\n글\u{2028}",
        ] {
            let chars = s
                .char_indices()
                .filter(|(_, c)| terminator(*c))
                .map(|(i, _)| i);
            assert_eq!(last_terminator(s), chars.clone().next_back(), "{s:?}");
            assert_eq!(has_terminator(s), chars.count() > 0, "{s:?}");
        }
    }
    #[test]
    fn android_year_and_nanosecond_formats() {
        for (raw, date, time) in [
            (
                "2026-09-21 12:07:30.386  1234  5678 I Tag: year",
                "2026-09-21",
                "12:07:30.386",
            ),
            (
                "09-21 12:07:30.386123456  1234  5678 I Tag: nsec",
                "09-21",
                "12:07:30.386123456",
            ),
            (
                "2026-09-21 12:07:30.386123456 1234 5678 W Tag: both",
                "2026-09-21",
                "12:07:30.386123456",
            ),
        ] {
            let p = parse_line(raw, 1, true).parsed.unwrap();
            assert_eq!(
                (p.date, p.time, p.pid, p.tid, p.tag),
                (date, time, "1234", "5678", "Tag"),
                "{raw}"
            );
            assert!(!p.ios);
        }
        // Seven or eight fraction digits are no logcat format.
        for raw in [
            "09-21 12:07:30.3861234 1 2 I Tag: x",
            "09-21 12:07:30.38612345 1 2 I Tag: x",
        ] {
            assert!(parse_line(raw, 1, true).parsed.is_none(), "{raw}");
        }
        // A dated row that is also a valid iOS row keeps its iOS fields.
        let ios = "2026-09-30 21:55:53.776 12 34 W Runner[7:9] [INFO] [Tag] body";
        assert!(parse_line(ios, 1, false).parsed.unwrap().ios);
    }
    #[test]
    fn malformed_stamped_rows_start_entries() {
        for (raw, recognize, starts) in [
            (
                "2026-09-15 10:47:35.184 Runner[3870:bad] malformed",
                false,
                true,
            ),
            ("09-21 12:07:30.386 x y I Tag: bad ids", false, true),
            ("  2026-09-30T21:55:54Z plain", false, true),
            ("0006 2026-09-30 21:55:54 body", true, true),
            ("0006 2026-09-30 21:55:54 body", false, false),
            ("    \"body\": \"안내문구3\",", false, false),
            ("  ]", false, false),
            ("\tat com.example.Main.run(Main.java:10)", false, false),
            ("42 ordinary message", true, false),
            ("2026-09-30", false, false),
            ("12-31 nope", false, false),
            ("", false, false),
        ] {
            assert_eq!(starts_entry(raw, recognize), starts, "{raw:?}");
        }
    }
    #[test]
    fn numbered_prefix_matches_its_former_regex() {
        let re = Regex::new(&format!(r"^([{WS}]*[0-9]+)[\t ]+")).unwrap();
        let mut caps = re.create_captures();
        let parts = [
            "", " ", "\t", "\u{3000}", "\u{a0}", "\u{feff}", "0", "42", "１", "٣", "x", "-", "\n",
            "\r", "09-21",
        ];
        for a in parts {
            for b in parts {
                for c in parts {
                    for d in parts {
                        let s = format!("{a}{b}{c}{d}");
                        re.captures(s.as_str(), &mut caps);
                        let want = caps
                            .get_match()
                            .map(|m| (&s[caps.get_group(1).unwrap().range()], m.end()));
                        assert_eq!(prefix(&s), want, "{s:?}");
                    }
                }
            }
        }
    }
    #[test]
    fn dlt_rows_are_never_read_as_text_rows() {
        for raw in [
            "09-21 12:07:30.386 007 009 I Tag: body",
            "  0003\t09-21 12:07:30.386 007 009 I Tag: body",
            "\tat com.example.Main.run(Main.java:10)",
        ] {
            let r = parse_in(Format::Dlt, raw, 1, true);
            assert!(r.parsed.is_none() && r.source_number.is_none(), "{raw:?}");
            assert!(parse_numbered_in(Format::Dlt, raw, 1).is_none(), "{raw:?}");
            // Each DLT row is a message of its own, never a continuation.
            assert!(starts_entry_in(Format::Dlt, raw, false), "{raw:?}");
        }
    }
    #[test]
    fn prefix_keeps_raw() {
        let r = parse_line("  0003\t09-21 12:07:30.386 007 009 I Tag: body", 4, true);
        assert_eq!(r.source_number, Some("0003"));
        assert!(r.raw.starts_with("  0003"));
        assert!(r.parsed.is_some());
    }
}
