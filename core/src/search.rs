use crate::{
    dlt, encoding,
    parser::{self, Record, LEVELS},
    source::{error, ByteLines, Format},
};
use grep_matcher::Matcher;
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use grep_searcher::{Searcher, SearcherBuilder, Sink, SinkMatch};
use serde::{Deserialize, Serialize};
use std::{
    borrow::Cow,
    cell::OnceCell,
    collections::HashMap,
    hash::{BuildHasherDefault, Hasher},
    io,
    sync::atomic::{AtomicBool, Ordering},
};
/// One condition of a filter: a row matches when it has one of the levels
/// and every listed field: a tag containing one of the tag items, one of the
/// PIDs, one of the TIDs, and a timestamp in the range. A row without the
/// field a condition tests never matches it.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq, Hash)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct Condition {
    pub levels: Vec<String>,
    pub tag: String,
    pub pid: String,
    pub tid: String,
    pub time_from: String,
    pub time_to: String,
}
impl Default for Condition {
    fn default() -> Self {
        Self {
            levels: LEVELS.iter().map(|s| s.to_string()).collect(),
            tag: String::new(),
            pid: String::new(),
            tid: String::new(),
            time_from: String::new(),
            time_to: String::new(),
        }
    }
}
/// Conditions a filter may list, besides its own fields.
pub const MAX_CONDITIONS: usize = 32;
/// A filter shows the rows that match one of its showing conditions (every
/// row when it has none) and none of its hiding conditions. The fields from
/// `levels` to `tid` are one showing condition and `exclude_tags` one hiding
/// condition of tags, the shorthand most callers need; `include` and
/// `exclude` list more. A condition without criteria is skipped.
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct Filters {
    pub levels: Vec<String>,
    /// Comma-separated tag substrings (at most 32); a parsed row matches when
    /// any of its tags contains any of them. Rows without tags never match.
    pub tag: String,
    /// Comma-separated tag substrings; a parsed row with any matching tag is
    /// excluded. Rows without tags are never excluded by this field.
    pub exclude_tags: String,
    /// Inclusive timestamp bounds in the log's own format, given as a prefix:
    /// `[YYYY-MM-DD | MM-DD] [HH[:MM[:SS[.fraction]]]]`.
    pub time_from: String,
    pub time_to: String,
    /// Exact ASCII-digit IDs (at most 32 each) separated by commas or
    /// whitespace; a parsed row matches when its ID equals any of them.
    pub pid: String,
    pub tid: String,
    /// More showing conditions, at most `MAX_CONDITIONS` with `exclude`.
    pub include: Vec<Condition>,
    /// Hiding conditions: a row that matches one is not shown. Rows without
    /// fields (orphans) match none.
    pub exclude: Vec<Condition>,
    pub query: String,
    pub case_sensitive: bool,
    pub include_raw: bool,
    /// None (JSON null or omitted) automatically recognizes numbered log rows.
    /// Explicit booleans preserve the manual on/off API.
    pub recognize_line_numbers: Option<bool>,
    pub search_mode: String,
}
impl Default for Filters {
    fn default() -> Self {
        Self {
            levels: LEVELS.iter().map(|s| s.to_string()).collect(),
            tag: String::new(),
            exclude_tags: String::new(),
            time_from: String::new(),
            time_to: String::new(),
            pid: String::new(),
            tid: String::new(),
            include: vec![],
            exclude: vec![],
            query: String::new(),
            case_sensitive: false,
            include_raw: true,
            recognize_line_numbers: None,
            search_mode: "literal".into(),
        }
    }
}
impl Filters {
    pub fn recognizes_line_numbers(&self) -> bool {
        self.recognize_line_numbers.unwrap_or(true)
    }
}
#[derive(Clone)]
pub struct Filter {
    pub fields: Filters,
    pub matcher: Option<RegexMatcher>,
    /// Conditions with criteria: the shorthand one first, then `include`.
    shows: Vec<Cond>,
    /// The shorthand tag condition first, then `exclude`.
    hides: Vec<Cond>,
    /// Where each condition of `include` and of `exclude` is among
    /// `conditions()`; None for one without criteria.
    listed: [Vec<Option<usize>>; 2],
    /// The format rows are parsed in; `Dataset::query` sets its own.
    format: Format,
}
const ALL_LEVELS: u16 = (1 << LEVELS.len()) - 1;
fn level_bit(level: &str) -> u16 {
    LEVELS
        .iter()
        .position(|l| *l == level)
        .map_or(0, |i| 1 << i)
}
/// A condition ready to test rows, with its normal form.
#[derive(Clone)]
pub struct Cond {
    /// Bits over `LEVELS`.
    levels: u16,
    tag: Option<RegexMatcher>,
    pids: Vec<String>,
    tids: Vec<String>,
    time: Option<TimeRange>,
    source: Condition,
}
impl Cond {
    /// Checks a condition's budget before any of it is parsed.
    fn budget(c: &Condition) -> io::Result<()> {
        if c.tag.len() > 4000
            || c.time_from.len() > 64
            || c.time_to.len() > 64
            || c.pid.len() > 1000
            || c.tid.len() > 1000
            || c.levels.len() > 8
        {
            return Err(error("Filter budget exceeded"));
        }
        Ok(())
    }
    /// The ID lists; `finish` reads the rest once every condition has them,
    /// so errors keep the order a single condition always reported.
    fn ids(mut c: Condition) -> io::Result<(Condition, Vec<String>, Vec<String>)> {
        let (pid, pids) = id_list(&c.pid)?;
        let (tid, tids) = id_list(&c.tid)?;
        (c.pid, c.tid) = (pid, tid);
        Ok((c, pids, tids))
    }
    fn valid_levels(c: &Condition) -> bool {
        c.levels.iter().all(|l| LEVELS.contains(&l.as_str()))
    }
    /// The levels, tags and time. `keep_tag` drops tag items that can change
    /// nothing.
    fn finish(
        (mut c, pids, tids): (Condition, Vec<String>, Vec<String>),
        keep_tag: impl Fn(&str) -> bool,
    ) -> io::Result<Self> {
        c.levels.sort();
        c.levels.dedup();
        let (tag, matcher) = tag_list(&c.tag, keep_tag)?;
        c.tag = tag;
        let (from, to) = (
            parse_bound(&c.time_from, false)?,
            parse_bound(&c.time_to, true)?,
        );
        c.time_from = parser::trim(&c.time_from).to_owned();
        c.time_to = parser::trim(&c.time_to).to_owned();
        Ok(Self {
            levels: c.levels.iter().fold(0, |bits, l| bits | level_bit(l)),
            tag: matcher,
            pids,
            tids,
            time: TimeRange::new(from, to),
            source: c,
        })
    }
    /// Whether the condition has no criterion and would match every row.
    fn is_empty(&self) -> bool {
        self.levels == ALL_LEVELS && self.levels_only()
    }
    /// Whether only the level decides the condition.
    pub fn levels_only(&self) -> bool {
        self.tag.is_none() && self.pids.is_empty() && self.tids.is_empty() && self.time.is_none()
    }
    /// Bits over `LEVELS` of the levels the condition matches.
    pub fn level_bits(&self) -> u16 {
        self.levels
    }
    /// Whether the PID, TID and tag lists can reject a value, in that order.
    pub fn tests(&self) -> [bool; 3] {
        [
            !self.pids.is_empty(),
            !self.tids.is_empty(),
            self.tag.is_some(),
        ]
    }
    pub(crate) fn has_time(&self) -> bool {
        self.time.is_some()
    }
    /// The condition's normal form: conditions of the same normal form
    /// match the same rows.
    pub fn condition(&self) -> &Condition {
        &self.source
    }
    /// Whether one tag contains a listed item; true without a tag list. A
    /// row matches with one such tag, so a row without tags matches only
    /// without a list.
    pub fn accepts_tag(&self, tag: &str) -> bool {
        self.tag
            .as_ref()
            .is_none_or(|t| t.is_match(tag.as_bytes()).unwrap_or(false))
    }
    /// The tag part of a parsed row's decision over all of its tags.
    pub fn tags_match(&self, tags: &[&str]) -> bool {
        self.tag.is_none() || tags.iter().any(|t| self.accepts_tag(t))
    }
    /// Whether a PID equals a listed one; true without a PID list.
    pub fn accepts_pid(&self, pid: &str) -> bool {
        listed(&self.pids, pid)
    }
    /// Whether a TID equals a listed one; true without a TID list.
    pub fn accepts_tid(&self, tid: &str) -> bool {
        listed(&self.tids, tid)
    }
    /// Whether a timestamp is in the range; true without one.
    fn accepts_time(&self, stamp: impl FnOnce() -> Option<Stamp>) -> bool {
        self.time
            .as_ref()
            .is_none_or(|range| stamp().is_some_and(|stamp| range.contains(&stamp)))
    }
    /// Whether a parsed row matches the condition.
    pub fn matches(&self, p: &parser::Parsed) -> bool {
        self.matches_at(p, &OnceCell::new())
    }
    /// `matches` with the row's timestamp kept in `stamp`, so that the
    /// conditions of one filter read it at most once per row.
    fn matches_at(&self, p: &parser::Parsed, stamp: &OnceCell<Option<Stamp>>) -> bool {
        self.levels & level_bit(p.level) != 0
            && self.accepts_pid(p.pid)
            && self.accepts_tid(p.tid)
            && self.tags_match(p.tag_list())
            && self.accepts_time(|| *stamp.get_or_init(|| row_stamp(p.date, p.time)))
    }
}
/// A row or bound timestamp. Year and date are optional on bounds; the year
/// is compared only when both sides have one (Android rows have none).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Stamp {
    year: Option<u16>,
    month_day: Option<u16>,
    nanos: u64,
}
#[derive(Clone, Copy, Debug)]
struct TimeRange {
    from: Option<Stamp>,
    to: Option<Stamp>,
    /// Whether the span wraps for rows without and with a year: it runs past
    /// a midnight or year end the compared fields cannot show.
    wraps: [bool; 2],
}
const DAY_NANOS: u64 = 86_400_000_000_000;
fn digits(s: &str, len: usize) -> Option<u64> {
    (s.len() == len && s.bytes().all(|b| b.is_ascii_digit())).then(|| s.parse().ok())?
}
fn parse_date(s: &str) -> Option<(Option<u16>, u16)> {
    // `[YYYY-]MM-DD`, split without allocating: rows parse it too.
    let mut parts = s.split('-');
    let (a, b, c) = (parts.next()?, parts.next()?, parts.next());
    if parts.next().is_some() {
        return None;
    }
    let (year, month, day) = match c {
        Some(d) => (Some(digits(a, 4)? as u16), digits(b, 2)?, digits(d, 2)?),
        None => (None, digits(a, 2)?, digits(b, 2)?),
    };
    ((1..=12).contains(&month) && (1..=31).contains(&day))
        .then_some((year, (month * 100 + day) as u16))
}
/// Parses `HH[:MM[:SS[.fraction]]]`. A lower bound fills unspecified digits
/// with zeros; an upper bound fills them to the end of the given precision.
fn parse_clock(s: &str, upper: bool) -> Option<u64> {
    let (clock, fraction) = match s.split_once('.') {
        Some((c, f)) => (c, Some(f)),
        None => (s, None),
    };
    let limits = [23, 59, 59];
    let units = [3_600_000_000_000u64, 60_000_000_000, 1_000_000_000];
    // One to three parts, three with a fraction; counted without
    // allocating, as rows parse their time here too.
    let (mut nanos, mut parts) = (0, 0);
    for part in clock.split(':') {
        if parts == 3 {
            return None;
        }
        let value = digits(part, 2)?;
        if value > limits[parts] {
            return None;
        }
        nanos += value * units[parts];
        parts += 1;
    }
    if fraction.is_some() && parts != 3 {
        return None;
    }
    let span = match fraction {
        Some(f) => {
            if f.is_empty() || f.len() > 9 || !f.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            nanos += f.parse::<u64>().ok()? * 10u64.pow(9 - f.len() as u32);
            10u64.pow(9 - f.len() as u32)
        }
        None => units[parts - 1],
    };
    Some(if upper { nanos + span - 1 } else { nanos })
}
fn parse_bound(s: &str, upper: bool) -> io::Result<Option<Stamp>> {
    let s = parser::trim(s);
    if s.is_empty() {
        return Ok(None);
    }
    let invalid = || error("Invalid time filter");
    let mut words = s.split_whitespace();
    let (first, second) = (words.next().ok_or_else(invalid)?, words.next());
    if words.next().is_some() {
        return Err(invalid());
    }
    let (date, clock) = match second {
        Some(clock) => (Some(first), Some(clock)),
        None if first.contains('-') => (Some(first), None),
        None => (None, Some(first)),
    };
    let (year, month_day) = match date {
        Some(d) => {
            let (y, md) = parse_date(d).ok_or_else(invalid)?;
            (y, Some(md))
        }
        None => (None, None),
    };
    let nanos = match clock {
        Some(c) => parse_clock(c, upper).ok_or_else(invalid)?,
        None if upper => DAY_NANOS - 1,
        None => 0,
    };
    Ok(Some(Stamp {
        year,
        month_day,
        nanos,
    }))
}
/// Reads a parsed row's own timestamp; malformed values never match.
fn row_stamp(date: &str, time: &str) -> Option<Stamp> {
    let (year, month_day) = parse_date(date)?;
    Some(Stamp {
        year,
        month_day: Some(month_day),
        nanos: parse_clock(time, false)?,
    })
}
fn compare(row: &Stamp, bound: &Stamp) -> std::cmp::Ordering {
    use std::cmp::Ordering::Equal;
    if let Some(md) = bound.month_day {
        if let (Some(a), Some(b)) = (row.year, bound.year) {
            if a != b {
                return a.cmp(&b);
            }
        }
        let order = row.month_day.unwrap_or(0).cmp(&md);
        if order != Equal {
            return order;
        }
    }
    row.nanos.cmp(&bound.nanos)
}
/// Without a year February has 28 days, so a span crossing it may include
/// a leap day rather than miss the first of March.
fn days_in_month(year: Option<u16>, month: u16) -> u16 {
    match month {
        2 if year.is_some_and(|y| y % 4 == 0 && (y % 100 != 0 || y % 400 == 0)) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}
fn next_day(year: Option<u16>, month_day: u16) -> (Option<u16>, u16) {
    let (month, day) = (month_day / 100, month_day % 100);
    if day < days_in_month(year, month) {
        (year, month_day + 1)
    } else if month < 12 {
        (year, (month + 1) * 100 + 1)
    } else {
        (year.map(|y| y + 1), 101)
    }
}
fn previous_day(year: Option<u16>, month_day: u16) -> (Option<u16>, u16) {
    let (month, day) = (month_day / 100, month_day % 100);
    if day > 1 {
        (year, month_day - 1)
    } else if month > 1 {
        (year, (month - 1) * 100 + days_in_month(year, month - 1))
    } else {
        (year.map(|y| y.saturating_sub(1)), 1231)
    }
}
impl TimeRange {
    fn new(mut from: Option<Stamp>, mut to: Option<Stamp>) -> Option<Self> {
        let mut contiguous = false;
        if let (Some(f), Some(t)) = (&mut from, &mut to) {
            match (f.month_day, t.month_day) {
                // Two clock-only bounds apply to every day and may run past
                // midnight.
                (None, None) => contiguous = true,
                // A clock-only bound takes the other bound's date, a day later
                // or earlier when needed so that the span runs forward.
                (Some(md), None) => {
                    (t.year, t.month_day) = (f.year, Some(md));
                    if t.nanos < f.nanos {
                        let (year, md) = next_day(f.year, md);
                        (t.year, t.month_day) = (year, Some(md));
                    }
                    contiguous = true;
                }
                (None, Some(md)) => {
                    (f.year, f.month_day) = (t.year, Some(md));
                    if f.nanos > t.nanos {
                        let (year, md) = previous_day(t.year, md);
                        (f.year, f.month_day) = (year, Some(md));
                    }
                    contiguous = true;
                }
                // Two explicit dates are compared as given.
                (Some(_), Some(_)) => {}
            }
        }
        // Rows without a year see a span across a year end inverted, as all
        // rows see a clock-only span across midnight.
        let inverted = |with_year: bool| match (from, to) {
            (Some(f), Some(t)) if contiguous => {
                let seen = Stamp {
                    year: f.year.filter(|_| with_year),
                    ..f
                };
                compare(&seen, &t).is_gt()
            }
            _ => false,
        };
        let wraps = [inverted(false), inverted(true)];
        (from.is_some() || to.is_some()).then_some(Self { from, to, wraps })
    }
    fn contains(&self, row: &Stamp) -> bool {
        let after = self.from.is_none_or(|f| compare(row, &f).is_ge());
        let before = self.to.is_none_or(|t| compare(row, &t).is_le());
        if self.wraps[row.year.is_some() as usize] {
            after || before
        } else {
            after && before
        }
    }
}
/// Tags a parsed row is filtered by. A row without tags has the empty tag,
/// which no tag filter accepts and none excludes.
pub fn row_tags<'a, 'b>(tags: &'b [&'a str]) -> &'b [&'a str] {
    if tags.is_empty() {
        &[""]
    } else {
        tags
    }
}
const MAX_LIST_ITEMS: usize = 32;
/// Whitespace as `parser::trim` (and JavaScript `\s`) defines it.
fn is_space(c: char) -> bool {
    parser::trim(c.encode_utf8(&mut [0; 4])).is_empty()
}
/// Trimmed, non-empty, distinct items in first-occurrence order, at most 32.
/// `keep` drops items that can change nothing.
fn items(
    list: &str,
    split: impl Fn(char) -> bool,
    keep: impl Fn(&str) -> bool,
) -> io::Result<Vec<&str>> {
    let mut items: Vec<&str> = Vec::new();
    for item in list.split(split).map(parser::trim) {
        if !item.is_empty() && keep(item) && !items.contains(&item) {
            items.push(item);
        }
    }
    if items.len() > MAX_LIST_ITEMS {
        return Err(error("Filter budget exceeded"));
    }
    Ok(items)
}
/// Case-insensitive literal alternation for a tag list, and its normal form.
fn tag_list(list: &str, keep: impl Fn(&str) -> bool) -> io::Result<(String, Option<RegexMatcher>)> {
    let items = items(list, |c| c == ',', keep)?;
    if items.is_empty() {
        return Ok((String::new(), None));
    }
    let matcher = RegexMatcherBuilder::new()
        .case_insensitive(true)
        .unicode(true)
        .fixed_strings(true)
        .size_limit(16 * 1024 * 1024)
        .dfa_size_limit(8 * 1024 * 1024)
        .build_literals(&items)
        .map_err(|_| error("Invalid filter"))?;
    Ok((items.join(","), Some(matcher)))
}
/// Exact PID/TID values separated by commas or whitespace: their normal form
/// and the values sorted for `listed`.
fn id_list(list: &str) -> io::Result<(String, Vec<String>)> {
    let items = items(list, |c| c == ',' || is_space(c), |_| true)?;
    if items.iter().any(|s| !s.bytes().all(|b| b.is_ascii_digit())) {
        return Err(error("PID and TID must contain ASCII digits"));
    }
    let mut sorted: Vec<String> = items.iter().map(|s| s.to_string()).collect();
    sorted.sort_unstable();
    Ok((items.join(","), sorted))
}
/// One ID stays a plain comparison. A row's ID usually matches none of a
/// list, and a linear scan would compare it with all 32 values.
fn listed(ids: &[String], id: &str) -> bool {
    match ids {
        [] => true,
        [one] => one == id,
        many => many.binary_search_by(|p| p.as_str().cmp(id)).is_ok(),
    }
}
fn matcher(q: &str, mode: &str, sensitive: bool) -> io::Result<RegexMatcher> {
    let impossible = mode == "literal" && (q.contains('\r') || q.contains('\n'));
    RegexMatcherBuilder::new()
        .case_insensitive(!sensitive)
        .multi_line(true)
        .unicode(true)
        .line_terminator(Some(b'\n'))
        .fixed_strings(mode == "literal" && !impossible)
        .size_limit(16 * 1024 * 1024)
        .dfa_size_limit(8 * 1024 * 1024)
        .nest_limit(128)
        .build(if impossible { r"\b\B" } else { q })
        .map_err(|e| {
            let s = e.to_string();
            error(format!(
                "Invalid or unsupported search pattern: {}",
                s.lines()
                    .rev()
                    .find_map(|l| l.strip_prefix("error: "))
                    .unwrap_or(&s)
                    .chars()
                    .take(320)
                    .collect::<String>()
            ))
        })
}
impl Filter {
    pub fn new(mut fields: Filters) -> io::Result<Self> {
        let own = Condition {
            levels: std::mem::take(&mut fields.levels),
            tag: std::mem::take(&mut fields.tag),
            pid: std::mem::take(&mut fields.pid),
            tid: std::mem::take(&mut fields.tid),
            time_from: std::mem::take(&mut fields.time_from),
            time_to: std::mem::take(&mut fields.time_to),
        };
        let own_hide = Condition {
            tag: std::mem::take(&mut fields.exclude_tags),
            ..Condition::default()
        };
        let (include, exclude) = (
            std::mem::take(&mut fields.include),
            std::mem::take(&mut fields.exclude),
        );
        let all = || {
            [&own, &own_hide]
                .into_iter()
                .chain(&include)
                .chain(&exclude)
        };
        if fields.query.len() > 4000 || include.len() + exclude.len() > MAX_CONDITIONS {
            return Err(error("Filter budget exceeded"));
        }
        for c in all() {
            Cond::budget(c)?;
        }
        let ids = all()
            .cloned()
            .map(Cond::ids)
            .collect::<io::Result<Vec<_>>>()?;
        if !all().all(Cond::valid_levels)
            || !matches!(fields.search_mode.as_str(), "literal" | "regex")
        {
            return Err(error("Invalid filter"));
        }
        let query = if fields.query.is_empty() {
            None
        } else {
            Some(matcher(
                &fields.query,
                &fields.search_mode,
                fields.case_sensitive,
            )?)
        };
        let mut ids = ids.into_iter();
        // An included item with a line break matches no tag but still
        // filters. The shorthand excluded tags drop such items, which can
        // remove nothing; a listed hiding condition keeps them, as it may
        // test other fields too.
        let own = Cond::finish(ids.next().unwrap(), |_| true)?;
        let own_hide = Cond::finish(ids.next().unwrap(), |t| !t.contains(['\r', '\n']))?;
        let include = (0..include.len())
            .map(|_| Cond::finish(ids.next().unwrap(), |_| true))
            .collect::<io::Result<Vec<_>>>()?;
        let exclude = (0..exclude.len())
            .map(|_| Cond::finish(ids.next().unwrap(), |_| true))
            .collect::<io::Result<Vec<_>>>()?;
        // Keep the normal form of every condition.
        let o = own.source.clone();
        (fields.levels, fields.tag, fields.pid, fields.tid) = (o.levels, o.tag, o.pid, o.tid);
        (fields.time_from, fields.time_to) = (o.time_from, o.time_to);
        fields.exclude_tags = own_hide.source.tag.clone();
        fields.include = include.iter().map(|c| c.source.clone()).collect();
        fields.exclude = exclude.iter().map(|c| c.source.clone()).collect();
        let keep = |c: &Cond| !c.is_empty();
        // Places among the kept conditions from `first` on.
        let place = |list: &[Cond], first: usize| -> Vec<Option<usize>> {
            let mut at = first;
            list.iter()
                .map(|c| {
                    keep(c).then(|| {
                        at += 1;
                        at - 1
                    })
                })
                .collect()
        };
        let include_at = place(&include, usize::from(keep(&own)));
        let shown = include_at.iter().flatten().count() + usize::from(keep(&own));
        let listed = [
            include_at,
            place(&exclude, shown + usize::from(keep(&own_hide))),
        ];
        let shows = std::iter::once(own).chain(include).filter(keep).collect();
        let hides = std::iter::once(own_hide)
            .chain(exclude)
            .filter(keep)
            .collect();
        Ok(Self {
            fields,
            matcher: query,
            shows,
            hides,
            listed,
            format: Format::Text,
        })
    }
    /// The filter reading rows of a dataset in `format`.
    pub fn with_format(mut self, format: Format) -> Self {
        self.format = format;
        self
    }
    /// Showing conditions with criteria; a row needs one unless there are none.
    pub fn shows(&self) -> &[Cond] {
        &self.shows
    }
    /// Hiding conditions with criteria.
    pub fn hides(&self) -> &[Cond] {
        &self.hides
    }
    /// Showing conditions with criteria, then hiding ones.
    pub fn conditions(&self) -> impl Iterator<Item = &Cond> {
        self.shows.iter().chain(&self.hides)
    }
    /// Where each condition of `fields.include` (0) and `fields.exclude`
    /// (1) is among `conditions()`; None for one without criteria, which a
    /// filter skips.
    pub fn listed(&self) -> &[Vec<Option<usize>>; 2] {
        &self.listed
    }
    pub fn identity(&self) -> bool {
        self.matcher.is_none()
            && self.shows.is_empty()
            && self.hides.is_empty()
            && self.fields.include_raw
    }
    /// Level slots a row can occupy and still match: bits follow `LEVELS`,
    /// and bit 8 admits orphan rows, as in `fields_match`. A continuation row
    /// occupies its log entry's level slot. Slots that only levels hide are
    /// left out.
    pub fn level_slots(&self) -> u16 {
        let shown = if self.shows.is_empty() {
            ALL_LEVELS
        } else {
            self.shows.iter().fold(0, |bits, c| bits | c.levels)
        };
        let hidden = self
            .hides
            .iter()
            .filter(|c| c.levels_only())
            .fold(0, |bits, c| bits | c.levels);
        let mut slots = shown & !hidden;
        if self.admits_unparsed() {
            slots |= 1 << 8;
        }
        slots
    }
    /// Whether orphan rows can match: rows without fields (raw and blank)
    /// before a file's first parsed row. They have no level, IDs, tag or
    /// timestamp, so they match no condition: a showing condition excludes
    /// them, a hiding condition never removes them, and "include raw"
    /// decides the rest. The block level table and `fields_match` must
    /// agree, so both use this one rule.
    ///
    /// A row that `parser::starts_entry_in` is decided by this rule too. Any
    /// other row without fields continues the log entry before it in the same
    /// file (a message printed over several lines) and takes that entry's
    /// field decision; see `mask_from`.
    pub fn admits_unparsed(&self) -> bool {
        self.fields.include_raw && self.shows.is_empty()
    }
    /// True when levels, IDs and tags decide every row: no time range and no
    /// search query, so per-block field metadata can count rows unread.
    pub fn only_fields(&self) -> bool {
        self.matcher.is_none() && !self.conditions().any(Cond::has_time)
    }
    /// Whether a condition tests the PID, TID or tags, in that order.
    pub fn tested_fields(&self) -> [bool; 3] {
        self.conditions().fold([false; 3], |acc, c| {
            let t = c.tests();
            [acc[0] || t[0], acc[1] || t[1], acc[2] || t[2]]
        })
    }
    /// True when `level_slots` alone decides every row.
    pub fn only_levels(&self) -> bool {
        self.matcher.is_none() && self.conditions().all(Cond::levels_only)
    }
    /// Whether field filters decide rows, so continuation rows need their
    /// entry's decision. Otherwise every row passes the field part.
    pub fn needs_fields(&self) -> bool {
        !self.shows.is_empty() || !self.hides.is_empty() || !self.fields.include_raw
    }
    /// Field decision for a parsed row; a row without fields is decided as an
    /// orphan here. Continuation rows are decided by `mask_from`.
    pub fn fields_match(&self, r: &Record) -> bool {
        if let Some(p) = &r.parsed {
            // Read when the first condition tests the time, then shared.
            let stamp = OnceCell::new();
            (self.shows.is_empty() || self.shows.iter().any(|c| c.matches_at(p, &stamp)))
                && !self.hides.iter().any(|c| c.matches_at(p, &stamp))
        } else {
            self.admits_unparsed()
        }
    }
    pub fn highlights(&self, s: &str) -> Vec<[usize; 2]> {
        let mut out: Vec<[usize; 2]> = vec![];
        if let Some(m) = &self.matcher {
            let mut matches = 0;
            let _ = m.find_iter(s.as_bytes(), |hit| {
                matches += 1;
                if hit.start() < hit.end() {
                    // Byte-mode regexes may match inside a UTF-8 scalar. Keep
                    // the matcher semantics, but highlight the whole character.
                    let (mut start, mut end) = (hit.start(), hit.end());
                    while !s.is_char_boundary(start) {
                        start -= 1;
                    }
                    while !s.is_char_boundary(end) {
                        end += 1;
                    }
                    // Merge scalar-expanded byte ranges before conversion, so
                    // the disjoint UTF-16 ranges can advance through text once.
                    if let Some(last) = out.last_mut().filter(|last| start < last[1]) {
                        last[1] = last[1].max(end);
                    } else {
                        out.push([start, end]);
                    }
                }
                matches < 32
            });
        }
        let (mut from, mut units) = (0, 0);
        for range in &mut out {
            let [start, end] = *range;
            units += s[from..start].encode_utf16().count();
            range[0] = units;
            units += s[start..end].encode_utf16().count();
            range[1] = units;
            from = end;
        }
        out
    }
    /// Whether the search query matches one decoded physical row. The query
    /// is line-oriented, so this is the row's decision in a block search.
    pub fn row_matches(&self, s: &str) -> bool {
        self.matcher
            .as_ref()
            .is_some_and(|m| m.is_match(s.as_bytes()).unwrap_or(false))
    }
    /// Whether both filters apply the same field conditions, so a row that
    /// passes one passes the other's field part.
    pub fn same_fields(&self, other: &Filter) -> bool {
        let sources = |list: &[Cond]| list.iter().map(|c| c.source.clone()).collect::<Vec<_>>();
        let (a, b) = (&self.fields, &other.fields);
        self.format == other.format
            && sources(&self.shows) == sources(&other.shows)
            && sources(&self.hides) == sources(&other.hides)
            && a.include_raw == b.include_raw
            && a.recognizes_line_numbers() == b.recognizes_line_numbers()
    }
    /// Matches of a block that starts a file: rows without fields before the
    /// first parsed row are orphans.
    pub fn mask(&self, data: &[u8], rows: usize, stop: &AtomicBool) -> io::Result<Vec<u8>> {
        Ok(self
            .mask_from(data, rows, stop, self.admits_unparsed())?
            .bits)
    }
    /// Matches of one block. `entry` is the field decision for the rows
    /// before the block's first parsed row: the decision of the log entry
    /// they continue from an earlier block, or the orphan rule at a file's
    /// start. The search query still matches each physical row on its own.
    pub fn mask_from(
        &self,
        data: &[u8],
        rows: usize,
        stop: &AtomicBool,
        entry: bool,
    ) -> io::Result<Mask> {
        let mut mask = vec![0; rows.div_ceil(8)];
        let (mut lead, mut tail) = (0, None);
        if stop.load(Ordering::Acquire) {
            return Ok(Mask {
                bits: mask,
                lead,
                tail,
            });
        }
        let fields = self.needs_fields();
        // Decode the block once. Invalid sequences become U+FFFD, which never
        // contains CR/LF, so physical rows keep their indexes and boundaries
        // and the field pass can slice rows out of the same decoded text.
        let display = (self.matcher.is_some() || fields).then(|| encoding::decode(data));
        if let (Some(matcher), Some(display)) = (&self.matcher, &display) {
            let data = display.as_bytes();
            let normalized: Cow<[u8]> = if memchr::memchr(b'\r', data).is_some() {
                // CR and CRLF become one LF; copy the runs between CRs.
                let mut d = Vec::with_capacity(data.len());
                let mut at = 0;
                for i in memchr::memchr_iter(b'\r', data) {
                    d.extend_from_slice(&data[at..i]);
                    d.push(b'\n');
                    at = i + 1 + usize::from(data.get(i + 1) == Some(&b'\n'));
                }
                d.extend_from_slice(&data[at..]);
                Cow::Owned(d)
            } else {
                Cow::Borrowed(data)
            };
            let mut sink = Bits {
                mask: &mut mask,
                stop,
            };
            SearcherBuilder::new()
                .line_number(true)
                .multi_line(false)
                .build()
                .search_slice(matcher, &normalized, &mut sink)?;
        } else {
            mask.fill(255);
        }
        if let (true, Some(display)) = (fields, &display) {
            let lines: Vec<(usize, &[u8])> = ByteLines::new(display.as_bytes()).collect();
            let parse = |i: usize| {
                let (offset, bytes) = lines[i];
                parser::parse_in(
                    self.format,
                    &display[offset..offset + bytes.len()],
                    i as u64 + 1,
                    self.fields.recognizes_line_numbers(),
                )
            };
            // `owner` is the decision of the last parsed row in rows
            // `..checked`, None while none is parsed. Only matched rows and
            // the rows back to the parsed row before them are parsed, each at
            // most once, as the earlier pass parsed only matched rows.
            let (mut owner, mut checked) = (None::<bool>, 0);
            // A parsed row decides its entry; a malformed row that starts an
            // entry is decided as an orphan, and so are the rows after it.
            let opens = |r: &Record| {
                r.parsed.is_some()
                    || parser::starts_entry_in(
                        self.format,
                        r.raw,
                        self.fields.recognizes_line_numbers(),
                    )
            };
            let latest = |from: usize, to: usize, owner: Option<bool>| {
                (to..from)
                    .rev()
                    .find_map(|k| {
                        let r = parse(k);
                        opens(&r).then(|| self.fields_match(&r))
                    })
                    .or(owner)
            };
            for i in 0..rows.min(lines.len()) {
                if mask[i / 8] & (1 << (i % 8)) == 0 {
                    continue;
                }
                if stop.load(Ordering::Acquire) {
                    break;
                }
                let r = parse(i);
                let pass = if opens(&r) {
                    owner = Some(self.fields_match(&r));
                    owner.unwrap()
                } else {
                    owner = latest(i, checked, owner);
                    owner.unwrap_or_else(|| {
                        lead += 1;
                        entry
                    })
                };
                checked = i + 1;
                if !pass {
                    mask[i / 8] &= !(1 << (i % 8));
                }
            }
            tail = latest(lines.len(), checked, owner);
        }
        Ok(Mask {
            bits: mask,
            lead,
            tail,
        })
    }
}
/// Hashes a 4-byte ID read as one integer.
#[derive(Default)]
struct IdHasher(u64);
impl Hasher for IdHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.write_u32(b.into());
        }
    }
    fn write_u32(&mut self, n: u32) {
        let x = (self.0 ^ u64::from(n)).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        self.0 = x ^ (x >> 29);
    }
}
/// Each of `wanted` that `matches(i)` passes, as bits.
fn hits(wanted: u64, mut matches: impl FnMut(usize) -> bool) -> u64 {
    let (mut left, mut found) = (wanted, 0);
    while left != 0 {
        let i = left.trailing_zeros() as usize;
        left &= left - 1;
        if matches(i) {
            found |= 1 << i;
        }
    }
    found
}
impl Filter {
    /// Matches of a block of DLT units decided from their messages'
    /// headers (`dlt::Decoder::heads`), without rendering the rows: what
    /// `mask_from` gives for the rendered block when there is no search
    /// query. Every DLT row is its own entry, so `lead` is 0 and `tail` is
    /// the last row's decision.
    pub fn dlt_mask(
        &self,
        decoder: &dlt::Decoder,
        raw: &[u8],
        rows: usize,
        stop: &AtomicBool,
    ) -> io::Result<Mask> {
        debug_assert!(self.matcher.is_none(), "a search query needs the rows");
        let mut bits = vec![0; rows.div_ceil(8)];
        let (mut row, mut tail) = (0, None);
        if !self.needs_fields() {
            bits.fill(255);
            return Ok(Mask {
                bits,
                lead: 0,
                tail,
            });
        }
        let orphan = self.admits_unparsed();
        // Showing conditions, then hiding ones; bit `i` of a mask is the
        // condition at `i`. PID and TID are constant over DLT rows, which
        // have none.
        let conds: Vec<&Cond> = self.conditions().collect();
        let (shows, all) = (self.shows.len(), conds.len());
        let ids_ok: u64 = conds
            .iter()
            .enumerate()
            .filter(|(_, c)| c.accepts_pid("") && c.accepts_tid(""))
            .fold(0, |bits, (i, _)| bits | 1 << i);
        let tagged: u64 = conds
            .iter()
            .enumerate()
            .filter(|(_, c)| c.tag.is_some())
            .fold(0, |bits, (i, _)| bits | 1 << i);
        // The conditions whose tag list accepts an ID, by the ID's bytes; a
        // block repeats few IDs. An absent ID is no tag of the row.
        let mut seen: HashMap<u32, u64, BuildHasherDefault<IdHasher>> = HashMap::default();
        let mut accepted = |id: [u8; 4]| {
            *seen.entry(u32::from_ne_bytes(id)).or_insert_with(|| {
                let mut buf = [0; 4];
                match dlt::id_field(&id, &mut buf) {
                    "" => 0,
                    text => conds
                        .iter()
                        .enumerate()
                        .filter(|(_, c)| c.tag.is_some() && c.accepts_tag(text))
                        .fold(0, |bits, (i, _)| bits | 1 << i),
                }
            })
        };
        decoder.heads(raw, |head| {
            if row >= rows || (row % 4096 == 0 && stop.load(Ordering::Acquire)) {
                row += 1;
                return;
            }
            let pass = match head {
                None => orphan,
                Some(h) => {
                    let level = level_bit(h.level);
                    let tags = if tagged == 0 {
                        0
                    } else {
                        h.ids
                            .into_iter()
                            .flatten()
                            .fold(0, |bits, id| bits | accepted(id))
                    };
                    let stamp = Stamp {
                        year: Some(h.year),
                        month_day: Some(h.month_day),
                        nanos: h.nanos,
                    };
                    let matches = |i: usize| {
                        let c = conds[i];
                        c.levels & level != 0
                            && ids_ok & 1 << i != 0
                            && (tagged & 1 << i == 0 || tags & 1 << i != 0)
                            && c.accepts_time(|| Some(stamp))
                    };
                    (shows == 0 || (0..shows).any(matches)) && !(shows..all).any(matches)
                }
            };
            if pass {
                bits[row / 8] |= 1 << (row % 8);
            }
            tail = Some(pass);
            row += 1;
        });
        if stop.load(Ordering::Acquire) {
            bits.fill(0);
        }
        Ok(Mask {
            bits,
            lead: 0,
            tail,
        })
    }
    /// Which of the conditions in `wanted` (bit `i` for `conditions()`
    /// item `i`) a row of a text block matches, as bits. Only rows with
    /// fields can match a condition, and a row continuing another entry
    /// has none, so each row is decided alone. Stops once each is found,
    /// and between rows once `stop` is set; bits found until then stay true.
    pub fn row_hits(&self, data: &[u8], wanted: u64, stop: &AtomicBool) -> u64 {
        let conds: Vec<&Cond> = self.conditions().collect();
        let text = encoding::decode(data);
        let recognize = self.fields.recognizes_line_numbers();
        let mut found = 0;
        for (i, (offset, bytes)) in ByteLines::new(text.as_bytes()).enumerate() {
            if found == wanted || (i % 256 == 0 && stop.load(Ordering::Acquire)) {
                break;
            }
            let r = parser::parse_in(
                self.format,
                &text[offset..offset + bytes.len()],
                i as u64 + 1,
                recognize,
            );
            if let Some(p) = &r.parsed {
                let stamp = OnceCell::new();
                found |= hits(wanted & !found, |k| conds[k].matches_at(p, &stamp));
            }
        }
        found
    }
    /// `row_hits` for a block of DLT units, decided from their messages'
    /// headers with the same per-condition tests as `dlt_mask`.
    pub fn dlt_row_hits(
        &self,
        decoder: &dlt::Decoder,
        raw: &[u8],
        wanted: u64,
        stop: &AtomicBool,
    ) -> u64 {
        let conds: Vec<&Cond> = self.conditions().collect();
        let ids_ok: u64 = conds
            .iter()
            .enumerate()
            .filter(|(_, c)| c.accepts_pid("") && c.accepts_tid(""))
            .fold(0, |bits, (i, _)| bits | 1 << i);
        let tagged: u64 = conds
            .iter()
            .enumerate()
            .filter(|(_, c)| c.tag.is_some())
            .fold(0, |bits, (i, _)| bits | 1 << i);
        let mut seen: HashMap<u32, u64, BuildHasherDefault<IdHasher>> = HashMap::default();
        let mut accepted = |id: [u8; 4]| {
            *seen.entry(u32::from_ne_bytes(id)).or_insert_with(|| {
                let mut buf = [0; 4];
                match dlt::id_field(&id, &mut buf) {
                    "" => 0,
                    text => conds
                        .iter()
                        .enumerate()
                        .filter(|(_, c)| c.tag.is_some() && c.accepts_tag(text))
                        .fold(0, |bits, (i, _)| bits | 1 << i),
                }
            })
        };
        let (mut found, mut row) = (0, 0usize);
        decoder.heads(raw, |head| {
            row += 1;
            if found == wanted || (row % 256 == 0 && stop.load(Ordering::Acquire)) {
                return;
            }
            let Some(h) = head else {
                return;
            };
            let level = level_bit(h.level);
            let tags = if tagged == 0 {
                0
            } else {
                h.ids
                    .into_iter()
                    .flatten()
                    .fold(0, |bits, id| bits | accepted(id))
            };
            let stamp = Stamp {
                year: Some(h.year),
                month_day: Some(h.month_day),
                nanos: h.nanos,
            };
            found |= hits(wanted & !found, |i| {
                let c = conds[i];
                c.levels & level != 0
                    && ids_ok & 1 << i != 0
                    && (tagged & 1 << i == 0 || tags & 1 << i != 0)
                    && c.accepts_time(|| Some(stamp))
            });
        });
        found
    }
}

/// One block's matches and what the blocks around it need to know.
#[derive(Debug)]
pub struct Mask {
    /// One bit per row in row order; bits past the last row are padding.
    pub bits: Vec<u8>,
    /// Matched rows before the block's first entry row (a parsed row or one
    /// that `parser::starts_entry_in`); `entry` decided them. Zero when no field
    /// filter applies.
    pub lead: usize,
    /// Field decision of the block's last entry row, which the next block of
    /// the same file inherits; None when the block has none or no field
    /// filter applies.
    pub tail: Option<bool>,
}
struct Bits<'a> {
    mask: &'a mut [u8],
    stop: &'a AtomicBool,
}
impl Sink for &mut Bits<'_> {
    type Error = io::Error;
    fn matched(&mut self, _: &Searcher, m: &SinkMatch<'_>) -> io::Result<bool> {
        if self.stop.load(Ordering::Acquire) {
            return Ok(false);
        }
        let n = m
            .line_number()
            .ok_or_else(|| error("Missing physical row"))? as usize
            - 1;
        if let Some(b) = self.mask.get_mut(n / 8) {
            *b |= 1 << (n % 8);
        }
        Ok(true)
    }
}
