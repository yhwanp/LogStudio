use crate::{
    dlt, encoding,
    fields_index::{self, FieldIndex},
    parser,
    search::{Cond, Condition, Filter, Mask},
    source::{error, public_message, ByteLines, Format, Part, Source, BLOCK, MAX_FILE, MAX_LINE},
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc, Condvar, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
/// Text files with NUL bytes do not open.
const NUL: &str = "NUL is not supported; select a UTF-8 text log";
/// UTF-16 and UTF-32 text fails on NUL too; this names the reason.
const UTF16: &str = "UTF-16 text is not supported; save the log as UTF-8";
/// So do compressed archives, such as a zipped bugreport or a rotated `.gz`.
const ARCHIVE: &str = "Compressed archives are not supported; extract the log first";
/// Upper bound on threads per block pass (one pane scan or the analysis).
/// Text passes gain nothing past 4; DLT passes render every block and keep
/// gaining up to 8 (see docs/LARGE_LOG_VALIDATION.md).
const MAX_BLOCK_WORKERS: usize = 4;
const MAX_DLT_WORKERS: usize = 8;
fn block_workers(format: Format) -> usize {
    let max = match format {
        Format::Text => MAX_BLOCK_WORKERS,
        Format::Dlt => MAX_DLT_WORKERS,
    };
    thread::available_parallelism()
        .map_or(1, |n| n.get() / 2)
        .clamp(1, max)
}
/// One block's rows for one parse variant. A row without fields continues
/// the log entry before it in the same file and counts in that entry's level
/// slot, also across blocks. A malformed row that `parser::starts_entry_in`
/// opens an orphan entry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BlockRows {
    /// Rows per level slot: `parser::LEVELS` order, then orphan rows (raw and
    /// blank rows before the file's first parsed row).
    pub slots: [u32; 9],
    /// Level slot of the block's last entry row, if any; 8 for an orphan.
    pub tail: Option<u8>,
    /// The parsed row whose entry the block's leading rows without fields
    /// continue; None at a file's start or after an orphan entry.
    pub owner: Option<u64>,
}
/// Index 0 ignores numbered prefixes, 1 recognizes them.
pub type BlockLevels = [BlockRows; 2];
#[derive(Clone, Copy, Debug)]
pub struct Block {
    pub start: u64,
    pub end: u64,
    pub first: u64,
    pub rows: usize,
    /// The source file holding the block; blocks never cross files.
    pub file: usize,
}
/// A block's rows as text, which `ByteLines` splits into its `rows` rows:
/// a text file's own bytes, or, for DLT, its messages decoded one row each.
pub struct Rows {
    pub text: Vec<u8>,
    /// Start of each row's source bytes from the block's start when rows
    /// are decoded; None when `text` is the block's own bytes.
    pub units: Option<Vec<u32>>,
}
#[derive(Default)]
pub struct Index {
    pub blocks: Vec<Block>,
    pub bytes: u64,
    pub total: u64,
    pub done: bool,
    pub error: String,
    /// The file of several whose bytes caused `error`; empty otherwise.
    pub error_file: String,
    pub summary: Option<Value>,
    pub prefix_summary: Option<Value>,
    /// Published with the summaries; lets level-only filters count without
    /// parsing and skip blocks that cannot contain a selected level.
    pub block_levels: Option<Arc<Vec<BlockLevels>>>,
    /// Published with `block_levels`; lets PID, TID and tag filters skip
    /// blocks and count rows without parsing.
    pub block_fields: Option<Arc<FieldIndex>>,
    /// First physical row of each source file indexed so far.
    pub file_rows: Vec<u64>,
    /// Set while the file is followed: queries and the analysis that reach
    /// the last block wait there for more instead of completing.
    pub live: bool,
    /// Why following stopped by itself while the indexed rows stay usable.
    pub follow_error: String,
    /// Counts replacements of the last row, which bytes appended after it
    /// completed, so rows already shown can be read again.
    pub replaced: u64,
    /// Blocks the published summaries and level table cover.
    pub analyzed: usize,
    /// Units of DLT files indexed so far that are no message.
    pub dlt_issues: DltIssues,
}
/// What `Dataset::export_as` writes for each result row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportKind {
    /// The row's source bytes: a text row, or a DLT row's message.
    Original,
    /// A DLT row's text with LF.
    Text,
}
/// Rows and bytes written, and DLT result rows of other bytes not written.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Exported {
    pub rows: u64,
    pub bytes: u64,
    pub skipped: u64,
}
/// DLT units read as `!!` rows instead of messages.
#[derive(Clone, Copy, Debug, Default)]
pub struct DltIssues {
    /// Runs of bytes that are no message, and their bytes.
    pub garbage: u64,
    pub garbage_bytes: u64,
    /// Messages cut short by the end of their file.
    pub truncated: u64,
    /// Protocol version 2 data.
    pub unsupported: u64,
}
/// How often a followed file's metadata is checked, in milliseconds.
pub const FOLLOW_POLL_MS: u64 = 500;
/// Earlier blocks a page reads to find the entry a block's leading rows
/// continue before the analysis table knows it (128 KiB blocks: ~1 MiB).
const OWNER_LOOKBACK: usize = 8;
/// Largest log entry `record` returns as one text: like a physical row, at
/// most 1 MiB, and at most as many rows as one copy.
pub const RECORD_BYTES: usize = MAX_LINE;
pub const RECORD_ROWS: usize = 10_000;
/// Physical rows one `lines` request may name (a bookmark list is 1,000).
pub const LINES_PER_REQUEST: usize = 200;
/// The log entry each page row without fields continues: its parsed row's
/// line and fields, so a view can show which log printed the row.
struct Owners {
    block: usize,
    first: u64,
    /// Rows `..checked` are resolved; `found` is the last entry row there and
    /// whether it is parsed.
    checked: usize,
    found: Option<(usize, bool)>,
    /// Owner JSON of the parsed row last used, and of the block's entry.
    cache: Option<(usize, Value)>,
    entry: Option<Option<Value>>,
}
impl Owners {
    fn new(block: usize, b: &Block) -> Self {
        Self {
            block,
            first: b.first,
            checked: 0,
            found: None,
            cache: None,
            entry: None,
        }
    }
    fn owner(
        &mut self,
        d: &Dataset,
        lines: &[(usize, &[u8])],
        row: usize,
        recognize: bool,
    ) -> io::Result<Option<Value>> {
        // Page rows ascend, so each block row is parsed at most once here.
        if let Some(found) = (self.checked..row)
            .rev()
            .find_map(|j| opens_entry(d.format, lines[j].1, recognize).map(|p| (j, p)))
        {
            self.found = Some(found);
        }
        if opens_entry(d.format, lines[row].1, recognize).is_some() {
            // A malformed row that starts an orphan entry.
            self.found = Some((row, false));
        }
        self.checked = self.checked.max(row + 1);
        let Some((j, parsed)) = self.found else {
            if self.entry.is_none() {
                let owner = d.entry_owner(self.block, recognize)?;
                self.entry = Some(match owner {
                    Some(line) => Some(owner_json(d.format, &d.row_bytes(line)?, line, recognize)),
                    None => None,
                });
            }
            return Ok(self.entry.clone().unwrap());
        };
        if !parsed {
            return Ok(None);
        }
        if self.cache.as_ref().is_none_or(|(k, _)| *k != j) {
            self.cache = Some((
                j,
                owner_json(d.format, lines[j].1, self.first + j as u64, recognize),
            ));
        }
        Ok(self.cache.as_ref().map(|(_, value)| value.clone()))
    }
}
/// Some(parsed) when a row opens a log entry: a parsed row, or a malformed
/// row that `parser::starts_entry_in`. None when it continues the entry
/// before.
fn opens_entry(format: Format, bytes: &[u8], recognize: bool) -> Option<bool> {
    let raw = encoding::decode(bytes);
    if parser::parse_in(format, &raw, 0, recognize)
        .parsed
        .is_some()
    {
        Some(true)
    } else {
        parser::starts_entry_in(format, &raw, recognize).then_some(false)
    }
}
fn owner_json(format: Format, bytes: &[u8], line: u64, recognize: bool) -> Value {
    let raw = encoding::decode(bytes);
    let record = parser::parse_in(format, &raw, line, recognize);
    let Some(p) = &record.parsed else {
        return json!({"lineNumber":line});
    };
    let field = |s| parser::preview(s, 256);
    json!({"lineNumber":line,"date":field(p.date),"time":field(p.time),"level":p.level,"tag":field(p.tag),"pid":field(p.pid),"tid":field(p.tid)})
}
/// Receives a pane and its status snapshot.
type ProgressObserver = Arc<dyn Fn(usize, Value) + Send + Sync>;
pub struct Dataset {
    pub id: u64,
    pub source: Source,
    /// The source's format, which decides how blocks become rows
    /// (`read_rows`) and which parser reads them.
    pub format: Format,
    /// Renders the blocks of a DLT source.
    dlt: Option<dlt::Decoder>,
    pub index: Mutex<Index>,
    pub changed: Condvar,
    pub stop: AtomicBool,
    pub pause_analysis: AtomicBool,
    workers: Mutex<Vec<JoinHandle<()>>>,
    progress: Mutex<Option<ProgressObserver>>,
    revision: AtomicU64,
    pub panes: [Mutex<Option<Arc<Query>>>; 2],
    pub gates: [Mutex<()>; 2],
    /// Analysis state between passes; None while a pass runs.
    analysis: Mutex<Option<Analysis>>,
    /// Pauses every block pass while the last block is replaced.
    hold: AtomicBool,
    follower: Mutex<Option<JoinHandle<()>>>,
    unfollow: AtomicBool,
    follow_gate: Mutex<()>,
    /// `FOLLOW_POLL_MS` unless a test or measurement changes it.
    pub follow_poll_ms: AtomicU64,
    /// What checks learned of each condition's rows, by line number
    /// variant and normal form, for every query of the dataset.
    seen: Mutex<HashMap<(bool, Condition), Seen>>,
}
/// What a dataset knows of the rows of one condition (see `run_check`).
#[derive(Clone, Copy, Debug)]
enum Seen {
    /// A row of this block matches it.
    Some(usize),
    /// No row of the blocks before this one matches it.
    None(usize),
}
/// Lifecycle of one pane query. Only `Running` can still change.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum QueryStatus {
    #[default]
    Running,
    Complete,
    Cancelled,
    Error,
}
impl QueryStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Complete => "complete",
            Self::Cancelled => "cancelled",
            Self::Error => "error",
        }
    }
}
#[derive(Default)]
pub struct Results {
    /// Cumulative matches through each scanned block, in block order.
    pub ends: Vec<u64>,
    /// Field decision for each scanned block's rows before its first entry
    /// row: inherited from the entry they continue, or the orphan rule.
    pub entries: Vec<bool>,
    pub count: u64,
    pub bytes: u64,
    pub status: QueryStatus,
    pub error: String,
    /// Decision the next block's leading rows inherit, and its file.
    carry: bool,
    carry_file: Option<usize>,
    /// A running query whose scan stopped at the last indexed block of a
    /// followed file (or paused); a new block starts it again.
    idle: bool,
}
impl Results {
    /// Forgets blocks `k..` so that a scan continues at block `k`.
    fn truncate(&mut self, k: usize, blocks: &[Block]) {
        if self.ends.len() <= k {
            return;
        }
        self.carry = self.entries[k];
        self.carry_file = k.checked_sub(1).map(|p| blocks[p].file);
        self.ends.truncate(k);
        self.entries.truncate(k);
        self.count = self.ends.last().copied().unwrap_or(0);
        self.bytes = k.checked_sub(1).map_or(0, |p| blocks[p].end);
    }
}
/// One scanned block. `lead` matches were counted as if the block's leading
/// rows passed; the scan applies their inherited decision in block order.
struct Scanned {
    end: u64,
    file: usize,
    count: u64,
    lead: u64,
    tail: Option<bool>,
}
pub struct Query {
    pub id: u64,
    pub filter: Filter,
    pub stop: AtomicBool,
    pub result: Mutex<Results>,
    worker: Mutex<Option<JoinHandle<()>>>,
    check: Mutex<Check>,
    checker: Mutex<Option<JoinHandle<()>>>,
    /// Wakes a waiting check when the query stops or a pane's scan ends.
    /// Its own lock, so that waiting never holds a lock `status` takes.
    wake: (Mutex<()>, Condvar),
}
/// Whether rows of the log match each condition of a logs pane query.
#[derive(Default)]
struct Check {
    /// Per item of `Filter::conditions()`: Some(true) when a row matches
    /// it, Some(false) when none does, None until known. Empty when the
    /// query is not checked.
    states: Vec<Option<bool>>,
    /// Whether a check runs, and whether it should run another round.
    running: bool,
    again: bool,
}
impl Query {
    pub fn cancel(&self) {
        self.stop.store(true, Ordering::Release);
        self.wake();
        if let Some(h) = self.worker.lock().unwrap().take() {
            let _ = h.join();
        }
        if let Some(h) = self.checker.lock().unwrap().take() {
            let _ = h.join();
        }
        let mut r = self.result.lock().unwrap();
        if r.status == QueryStatus::Running {
            r.status = QueryStatus::Cancelled;
        }
    }
    fn wake(&self) {
        let _lock = self.wake.0.lock().unwrap();
        self.wake.1.notify_all();
    }
    /// Waits for `wake` up to `timeout`, unless `stop` is set.
    fn sleep(&self, timeout: Duration) {
        let lock = self.wake.0.lock().unwrap();
        if !self.stop.load(Ordering::Acquire) {
            drop(self.wake.1.wait_timeout(lock, timeout));
        }
    }
    /// The query's state. A checked query (the logs pane's, with
    /// conditions) adds `conditions`: for each item of the request's
    /// `include` and `exclude`, "some" when a row of the log matches it,
    /// "none" when none does (none so far while a file is followed),
    /// "pending" until known, and null for a condition without criteria.
    pub fn json(&self) -> Value {
        let mut v = {
            let r = self.result.lock().unwrap();
            json!({"id":self.id,"status":r.status.as_str(),"count":r.count,"scannedBytes":r.bytes,"complete":r.status==QueryStatus::Complete,"caughtUp":r.status==QueryStatus::Running&&r.idle,"error":r.error})
        };
        let c = self.check.lock().unwrap();
        if !c.states.is_empty() {
            let state = |at: &Option<usize>| match at.map(|k| c.states[k]) {
                None => Value::Null,
                Some(None) => json!("pending"),
                Some(Some(true)) => json!("some"),
                Some(Some(false)) => json!("none"),
            };
            let [include, exclude] = self.filter.listed();
            v["conditions"] = json!({
                "include": include.iter().map(state).collect::<Vec<_>>(),
                "exclude": exclude.iter().map(state).collect::<Vec<_>>(),
            });
        }
        v
    }
}
impl Dataset {
    pub fn new(id: u64, source: Source) -> Arc<Self> {
        Self::with_dlt_clock(id, source, dlt::Clock::local())
    }
    /// A dataset whose DLT rows show storage times with `clock`, so tests
    /// and measurements do not depend on the system time zone.
    #[doc(hidden)]
    pub fn with_dlt_clock(id: u64, source: Source, clock: dlt::Clock) -> Arc<Self> {
        let format = source.format();
        Arc::new(Self {
            id,
            format,
            dlt: (format == Format::Dlt).then(|| dlt::Decoder::new(clock)),
            source,
            index: Mutex::new(Index::default()),
            changed: Condvar::new(),
            stop: AtomicBool::new(false),
            pause_analysis: AtomicBool::new(false),
            workers: Mutex::new(vec![]),
            progress: Mutex::new(None),
            revision: AtomicU64::new(0),
            panes: [Mutex::new(None), Mutex::new(None)],
            gates: [Mutex::new(()), Mutex::new(())],
            analysis: Mutex::new(Some(Analysis::default())),
            hold: AtomicBool::new(false),
            follower: Mutex::new(None),
            unfollow: AtomicBool::new(false),
            follow_gate: Mutex::new(()),
            follow_poll_ms: AtomicU64::new(FOLLOW_POLL_MS),
            seen: Mutex::new(HashMap::new()),
        })
    }
    pub fn observe_progress(&self, f: impl Fn(usize, Value) + Send + Sync + 'static) {
        *self.progress.lock().unwrap() = Some(Arc::new(f));
    }
    fn publish_progress(&self, pane: usize) {
        let callback = self.progress.lock().unwrap().clone();
        if let Some(callback) = callback {
            // No dataset/result lock is held while calling the observer.
            callback(pane, self.status(pane));
        }
    }
    pub fn start(self: &Arc<Self>, finish: impl FnOnce(bool) + Send + 'static) {
        let d = self.clone();
        let h = thread::spawn(move || {
            let result = d.build_index();
            if let Err(e) = &result {
                d.index.lock().unwrap().error =
                    public_message(e, "Could not read the log; reopen the file");
            }
            d.changed.notify_all();
            finish(result.is_ok() && !d.stop.load(Ordering::Acquire));
            if result.is_ok() && !d.stop.load(Ordering::Acquire) {
                let _ = d.analyze();
            }
        });
        self.workers.lock().unwrap().push(h);
    }
    fn build_index(&self) -> io::Result<()> {
        let mut row = 1u64;
        let parts = self.source.parts();
        for (file, part) in parts.iter().enumerate() {
            self.index.lock().unwrap().file_rows.push(row);
            let end = part.offset + part.size;
            let next = match self.format {
                Format::Text => self
                    .index_file(file, part.offset, end, row, true)
                    .map_err(|e| self.explain_nul(part, e)),
                Format::Dlt => self.index_dlt_file(file, part.offset, end, row),
            };
            row = match next {
                Ok(row) => row,
                Err(e) => {
                    // Name which of several files failed; a cancel names none.
                    if parts.len() > 1 && !self.stop.load(Ordering::Acquire) {
                        self.index.lock().unwrap().error_file = part.name.clone();
                    }
                    return Err(e);
                }
            };
        }
        self.source.verify()?;
        let mut index = self.index.lock().unwrap();
        index.done = true;
        index.bytes = self.source.size;
        drop(index);
        self.changed.notify_all();
        Ok(())
    }
    /// A text file that fails on NUL may hold DLT messages without storage
    /// headers, UTF-16 text such as a log that Windows PowerShell's `>`
    /// saved, or a compressed archive; that reason replaces the NUL error.
    fn explain_nul(&self, part: &Part, e: io::Error) -> io::Error {
        if public_message(&e, "") != NUL {
            return e;
        }
        let n = part.size.min(dlt::HEADERLESS_HEAD as u64) as usize;
        // UTF-16 comes first: its ASCII rows, such as `0\09\0` of a
        // threadtime stamp, can chain as DLT lengths over a large file.
        match self.source.read(part.offset, n) {
            Ok(head) if encoding::wide_text(&head) => error(UTF16),
            Ok(head) if dlt::headerless(&head, part.size) => {
                error("DLT data without storage headers is not supported")
            }
            Ok(head) if encoding::archive(&head) => error(ARCHIVE),
            _ => e,
        }
    }
    /// Publishes the blocks of one file's bytes `start..end`, whose first row
    /// is `row`, and returns the next row. With `close` the end closes the
    /// last row, as a file's end does; a row so closed without a line ending
    /// gets a block of its own, which a followed file's appended bytes can
    /// replace. Without `close` an unterminated last row, and a CR that an
    /// LF may follow, wait for more bytes.
    fn index_file(
        &self,
        file: usize,
        start: u64,
        end: u64,
        mut row: u64,
        close: bool,
    ) -> io::Result<u64> {
        let (mut at, mut base, mut tail) = (start, start, Vec::new());
        while at < end || !tail.is_empty() {
            if self.stop.load(Ordering::Acquire) {
                return Err(error("Opening cancelled"));
            }
            let n = (end - at).min(BLOCK as u64) as usize;
            if n > 0 {
                let bytes = self.source.read(at, n)?;
                if self.stop.load(Ordering::Acquire) {
                    return Err(error("Opening cancelled"));
                }
                tail.extend(bytes);
                at += n as u64;
            }
            let eof = at == end && close;
            let mut frontier = 0;
            let mut rows = 0;
            let mut i = 0;
            let mut line_start = 0;
            while let Some(found) = memchr::memchr3(0, b'\r', b'\n', &tail[i..]) {
                i += found;
                if tail[i] == 0 {
                    return Err(error(NUL));
                }
                if tail[i] == b'\r' && i + 1 == tail.len() && !eof {
                    break;
                }
                if i - line_start > MAX_LINE {
                    return Err(error("A physical line exceeds 1 MiB"));
                }
                i += if tail[i] == b'\r' && tail.get(i + 1) == Some(&b'\n') {
                    2
                } else {
                    1
                };
                frontier = i;
                line_start = i;
                rows += 1;
            }
            if tail.len() - line_start > MAX_LINE + 1 {
                return Err(error("A physical line exceeds 1 MiB"));
            }
            let mut open = 0;
            if eof && line_start < tail.len() {
                if tail.len() - line_start > MAX_LINE {
                    return Err(error("A physical line exceeds 1 MiB"));
                }
                open = tail.len() - line_start;
            }
            for (len, rows) in [(frontier, rows), (open, 1)] {
                if len == 0 {
                    continue;
                }
                let mut index = self.index.lock().unwrap();
                index.blocks.push(Block {
                    start: base,
                    end: base + len as u64,
                    first: row,
                    rows,
                    file,
                });
                index.bytes = base + len as u64;
                index.total += rows as u64;
                drop(index);
                base += len as u64;
                row += rows as u64;
                tail.drain(..len);
                self.changed.notify_all();
            }
            if at == end {
                break;
            }
        }
        Ok(row)
    }
    /// Publishes the blocks of one DLT file's bytes `start..end`, whose first
    /// row is `row`, and returns the next row. Each block holds whole units
    /// (`dlt::units`), one row each, so it renders on its own; bytes whose
    /// unit later bytes decide wait for the next read. A file with no
    /// message fails, as a text file with NUL does.
    fn index_dlt_file(&self, file: usize, start: u64, end: u64, mut row: u64) -> io::Result<u64> {
        let (mut at, mut base, mut tail) = (start, start, Vec::new());
        let (mut messages, mut unsupported) = (0u64, 0u64);
        while at < end || !tail.is_empty() {
            if self.stop.load(Ordering::Acquire) {
                return Err(error("Opening cancelled"));
            }
            let n = (end - at).min(BLOCK as u64) as usize;
            if n > 0 {
                tail.extend(self.source.read(at, n)?);
                at += n as u64;
            }
            let mut issues = DltIssues::default();
            let mut it = dlt::units(&tail, (end - base) as usize);
            let mut rows = 0;
            for u in &mut it {
                rows += 1;
                match u.kind {
                    dlt::Kind::Message => messages += 1,
                    dlt::Kind::Unsupported => issues.unsupported += 1,
                    dlt::Kind::Garbage => match dlt::truncated(&tail[u.start..u.start + u.len]) {
                        Some(_) => issues.truncated += 1,
                        None => {
                            issues.garbage += 1;
                            issues.garbage_bytes += u.len as u64;
                        }
                    },
                }
            }
            let used = it.position();
            unsupported += issues.unsupported;
            if used > 0 {
                let mut index = self.index.lock().unwrap();
                index.blocks.push(Block {
                    start: base,
                    end: base + used as u64,
                    first: row,
                    rows,
                    file,
                });
                index.bytes = base + used as u64;
                index.total += rows as u64;
                let total = &mut index.dlt_issues;
                total.garbage += issues.garbage;
                total.garbage_bytes += issues.garbage_bytes;
                total.truncated += issues.truncated;
                total.unsupported += issues.unsupported;
                drop(index);
                base += used as u64;
                row += rows as u64;
                tail.drain(..used);
                self.changed.notify_all();
            } else if at == end {
                // Every byte is decided at the end of the file.
                return Err(error("Could not read the DLT file"));
            }
        }
        if messages == 0 && end > start {
            return Err(error(if unsupported > 0 {
                "DLT version 2 files are not supported"
            } else {
                "A selected DLT file contains no DLT messages"
            }));
        }
        Ok(row)
    }
    /// Run `work` on indexed blocks from block `from` with up to `workers`
    /// threads and hand the results to `deliver` strictly in block order.
    /// Workers wait for blocks that indexing has not published yet. Once
    /// `stop`, the source or `hold` stops, nothing more is delivered, so a
    /// block cut short is never reported.
    fn blocks_in_order<T: Send>(
        &self,
        from: usize,
        workers: usize,
        stop: &AtomicBool,
        work: impl Fn(usize, Block) -> io::Result<T> + Sync,
        mut deliver: impl FnMut(usize, T),
    ) -> io::Result<()> {
        struct Shared<T> {
            ready: BTreeMap<usize, T>,
            error: Option<io::Error>,
            running: usize,
        }
        let shared = Mutex::new(Shared {
            ready: BTreeMap::new(),
            error: None,
            running: workers,
        });
        let (failed, ready) = (AtomicBool::new(false), Condvar::new());
        let next = AtomicUsize::new(from);
        let halted = || {
            stop.load(Ordering::Acquire)
                || self.stop.load(Ordering::Acquire)
                || self.hold.load(Ordering::Acquire)
                || failed.load(Ordering::Acquire)
        };
        thread::scope(|scope| {
            for _ in 0..workers {
                scope.spawn(|| {
                    let outcome = (|| loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(block) = self.wait_block(i, &halted)? else {
                            return Ok(());
                        };
                        let value = work(i, block)?;
                        // A result finished after a stop may be partial.
                        if halted() {
                            return Ok(());
                        }
                        shared.lock().unwrap().ready.insert(i, value);
                        ready.notify_all();
                    })();
                    let mut state = shared.lock().unwrap();
                    if let Err(e) = outcome {
                        failed.store(true, Ordering::Release);
                        state.error.get_or_insert(e);
                    }
                    state.running -= 1;
                    drop(state);
                    ready.notify_all();
                });
            }
            let mut want = from;
            let mut state = shared.lock().unwrap();
            loop {
                if halted() {
                    break;
                }
                if let Some(value) = state.ready.remove(&want) {
                    drop(state);
                    deliver(want, value);
                    want += 1;
                    state = shared.lock().unwrap();
                } else if state.running == 0 {
                    break;
                } else {
                    state = ready.wait(state).unwrap();
                }
            }
            match state.error.take() {
                Some(e) => Err(e),
                None => Ok(()),
            }
        })
    }
    fn wait_block(&self, i: usize, halted: &dyn Fn() -> bool) -> io::Result<Option<Block>> {
        let mut index = self.index.lock().unwrap();
        loop {
            if halted() {
                return Ok(None);
            }
            if !index.error.is_empty() {
                return Err(error(&index.error));
            }
            if let Some(block) = index.blocks.get(i) {
                return Ok(Some(*block));
            }
            if index.done {
                return Ok(None);
            }
            index = self
                .changed
                .wait_timeout(index, std::time::Duration::from_millis(20))
                .unwrap()
                .0;
        }
    }
    fn analyze(&self) -> io::Result<()> {
        // A followed file's blocks indexed meanwhile get another pass.
        while self.analyze_blocks()? {}
        Ok(())
    }
    /// One analysis pass from the first block not analyzed yet. Returns true
    /// when more blocks are waiting, false when another pass is running.
    fn analyze_blocks(&self) -> io::Result<bool> {
        let Some(Analysis {
            mut stats,
            mut prefix,
            mut levels,
            mut carry,
            mut carry_file,
            mut before,
        }) = self.analysis.lock().unwrap().take()
        else {
            return Ok(false);
        };
        // Field metadata comes from the first pass; a followed file's later
        // blocks are read by field filters.
        let mut fields = levels.is_empty().then(fields_index::Builder::default);
        let collect = fields.is_some();
        let never = AtomicBool::new(false);
        let result = self.blocks_in_order(
            levels.len(),
            block_workers(self.format),
            &never,
            |_, b| {
                // Optional metadata yields while either pane scans.
                while self.analysis_yields() {
                    if self.stop.load(Ordering::Acquire) {
                        return Err(error("Source closed"));
                    }
                    thread::park_timeout(std::time::Duration::from_millis(10));
                }
                let rows = self.read_rows(&b)?;
                let (mut stats, mut prefix) = (Summary::default(), Summary::default());
                let mut entries = [Entries::default(), Entries::default()];
                // Decoded once, as in `Filter::mask_from`; field values borrow it.
                let text = encoding::decode(&rows.text);
                let mut values = collect.then(fields_index::Collector::default);
                for (i, (offset, bytes)) in ByteLines::new(text.as_bytes()).enumerate() {
                    let raw = &text[offset..offset + bytes.len()];
                    let line = b.first + i as u64;
                    let r = parser::parse_in(self.format, raw, line, false);
                    stats.add(&r);
                    let opens = entries[0].add(self.format, &r, false);
                    let numbered = if r.parsed.is_none() {
                        parser::parse_numbered_in(self.format, raw, line)
                    } else {
                        None
                    };
                    let r1 = numbered.as_ref().unwrap_or(&r);
                    prefix.add(r1);
                    let opens_numbered = entries[1].add(self.format, r1, true);
                    if let Some(values) = values.as_mut() {
                        values.add(&r, opens, numbered.as_ref(), opens_numbered);
                    }
                }
                Ok((b.file, stats, prefix, entries, values.map(|v| v.finish())))
            },
            |_, (file, block_stats, block_prefix, entries, block_fields)| {
                before = Some((stats.clone(), prefix.clone(), carry, carry_file));
                // Leading rows continue the last entry of an earlier block.
                if carry_file != Some(file) {
                    carry_file = Some(file);
                    carry = [None, None];
                }
                Arc::make_mut(&mut levels).push([0, 1].map(|v| {
                    let e = &entries[v];
                    let mut slots = e.slots;
                    slots[carry[v].map_or(8, |(level, _)| level as usize)] += e.lead;
                    let rows = BlockRows {
                        slots,
                        tail: e.tail.map(|(level, _)| level),
                        owner: carry[v]
                            .filter(|(level, _)| *level < 8)
                            .map(|(_, line)| line),
                    };
                    carry[v] = e.tail.or(carry[v]);
                    rows
                }));
                if let (Some(fields), Some(block)) = (fields.as_mut(), block_fields) {
                    fields.push(file, block);
                }
                stats.merge(&block_stats);
                prefix.merge(&block_prefix);
            },
        );
        let (held, stopped) = (
            self.hold.load(Ordering::Acquire),
            self.stop.load(Ordering::Acquire),
        );
        let state = Analysis {
            stats: stats.clone(),
            prefix: prefix.clone(),
            levels: levels.clone(),
            carry,
            carry_file,
            before,
        };
        if result.is_err() || held || stopped {
            *self.analysis.lock().unwrap() = Some(state);
            return result.map(|_| false);
        }
        // Rows without fields that continue an entry, not orphans.
        let continued = |stats: &Summary, v: usize| {
            let orphans: u64 = levels.iter().map(|l| u64::from(l[v].slots[8])).sum();
            let mut json = stats.json();
            json["continuation"] = json!(stats.total - stats.parsed - orphans);
            json
        };
        let (summary, prefix_summary) = (continued(&stats, 0), continued(&prefix, 1));
        let mut index = self.index.lock().unwrap();
        let more = levels.len() < index.blocks.len();
        index.summary = Some(summary);
        index.prefix_summary = Some(prefix_summary);
        index.analyzed = levels.len();
        index.block_levels = Some(levels);
        if let Some(fields) = fields {
            index.block_fields = Some(Arc::new(fields.finish()));
        }
        // Put back under the index lock: a block published after this is
        // seen by `resume_passes`, one published before by `more`.
        *self.analysis.lock().unwrap() = Some(state);
        Ok(more)
    }
    /// Optional metadata yields while either pane scans, but not to a
    /// followed file's query waiting for rows, nor during `hold`.
    fn analysis_yields(&self) -> bool {
        !self.hold.load(Ordering::Acquire)
            && (self.pause_analysis.load(Ordering::Acquire) || self.panes_scan())
    }
    /// Whether a pane's query scans, not waiting for a followed file's rows.
    fn panes_scan(&self) -> bool {
        self.panes.iter().any(|p| {
            p.lock().unwrap().as_ref().is_some_and(|q| {
                let r = q.result.lock().unwrap();
                r.status == QueryStatus::Running && !r.idle
            })
        })
    }
    pub fn query(self: &Arc<Self>, pane: usize, id: u64, filter: Filter) -> io::Result<Arc<Query>> {
        let _gate = self.gates[pane].lock().unwrap();
        let previous = self.panes[pane].lock().unwrap().clone();
        if previous.as_ref().is_some_and(|q| id <= q.id) {
            return Err(error("Stale query ID"));
        }
        if let Some(p) = previous {
            p.cancel();
        }
        if self.stop.load(Ordering::Acquire) {
            return Err(error("Source closed"));
        }
        let q = Arc::new(Query {
            id,
            filter: filter.with_format(self.format),
            stop: AtomicBool::new(false),
            result: Mutex::new(Results::default()),
            worker: Mutex::new(None),
            check: Mutex::new(Check::default()),
            checker: Mutex::new(None),
            wake: (Mutex::new(()), Condvar::new()),
        });
        // The logs pane's listed conditions are checked (not the shorthand
        // fields, which the state does not report); what is known shows at once.
        if pane == 0 && q.filter.listed().iter().flatten().any(Option::is_some) {
            *q.check.lock().unwrap() = Check {
                states: self.known(&q.filter),
                running: true,
                again: false,
            };
        }
        *self.panes[pane].lock().unwrap() = Some(q.clone());
        let d = self.clone();
        let scan = q.clone();
        let h = thread::spawn(move || d.run_scan(pane, &scan));
        *q.worker.lock().unwrap() = Some(h);
        if q.check.lock().unwrap().running {
            let (d, c) = (self.clone(), q.clone());
            *q.checker.lock().unwrap() = Some(thread::spawn(move || d.run_check(&c)));
        }
        Ok(q)
    }
    /// What earlier checks found of a filter's conditions, as `Check` states.
    fn known(&self, f: &Filter) -> Vec<Option<bool>> {
        let (n, live) = {
            let index = self.index.lock().unwrap();
            (index.blocks.len(), index.live)
        };
        let variant = f.fields.recognizes_line_numbers();
        let seen = self.seen.lock().unwrap();
        f.conditions()
            .map(|c| match seen.get(&(variant, c.condition().clone())) {
                Some(Seen::Some(_)) => Some(true),
                Some(Seen::None(m)) if *m >= n || live => Some(false),
                _ => None,
            })
            .collect()
    }
    fn check_halted(&self, q: &Query) -> bool {
        q.stop.load(Ordering::Acquire)
            || self.stop.load(Ordering::Acquire)
            || self.hold.load(Ordering::Acquire)
    }
    /// Learns whether rows of the log match each listed condition of the
    /// logs pane's query `q`, for its states and for the dataset's later queries
    /// (`seen`). A round covers the blocks indexed when it starts; a
    /// followed file's growth asks for another (`resume_passes`).
    fn run_check(&self, q: &Query) {
        loop {
            self.check_round(q);
            let mut c = q.check.lock().unwrap();
            if c.again && !self.check_halted(q) {
                c.again = false;
                continue;
            }
            c.running = false;
            return;
        }
    }
    /// One round: after the analysis, the level and field tables decide
    /// what they can; one scan of the blocks they leave open reads the
    /// rest, with the pane's worker count, stopping for each condition at
    /// its first matching row. Both yield to the panes' scans, as the
    /// analysis does, and stop with the query, the dataset or `hold`.
    fn check_round(&self, q: &Query) {
        let conds: Vec<&Cond> = q.filter.conditions().collect();
        let variant = q.filter.fields.recognizes_line_numbers();
        let (blocks, levels, fields) = loop {
            if self.check_halted(q) {
                return;
            }
            let index = self.index.lock().unwrap();
            if !index.error.is_empty() {
                return;
            }
            if let (Some(l), Some(f)) = (&index.block_levels, &index.block_fields) {
                break (index.blocks.clone(), l.clone(), f.clone());
            }
            drop(index);
            q.sleep(Duration::from_millis(20));
        };
        let n = blocks.len();
        // Blocks from which each listed condition still needs checking.
        let mut from: Vec<Option<usize>> = vec![None; conds.len()];
        {
            let seen = self.seen.lock().unwrap();
            for &k in q.filter.listed().iter().flatten().flatten() {
                from[k] = match seen.get(&(variant, conds[k].condition().clone())) {
                    Some(Seen::Some(_)) => None,
                    Some(Seen::None(m)) => (*m < n).then_some(*m),
                    None => Some(0),
                };
            }
        }
        // States of conditions known before this round, e.g. after a reload.
        self.learn(q, &conds, variant, &[], n);
        let need: Vec<(usize, usize)> = from
            .iter()
            .enumerate()
            .filter_map(|(k, m)| m.map(|m| (k, m)))
            .collect();
        if need.is_empty() {
            return;
        }
        let workers = block_workers(self.format).min(need.len());
        let wait = || {
            while self.check_yields() {
                if self.check_halted(q) {
                    return false;
                }
                q.sleep(Duration::from_millis(10));
            }
            !self.check_halted(q)
        };
        // Tables: a matching block, or the blocks they cannot decide.
        let next = AtomicUsize::new(0);
        let decided = Mutex::new(vec![]);
        thread::scope(|scope| {
            for _ in 0..workers {
                scope.spawn(|| loop {
                    let j = next.fetch_add(1, Ordering::Relaxed);
                    if j >= need.len() || !wait() {
                        return;
                    }
                    let (k, m) = need[j];
                    let probe = fields.probe(conds[k]);
                    let mut open = vec![];
                    let mut found = None;
                    for i in m..n {
                        // A large tag table takes milliseconds per condition.
                        if i % 256 == 0 && self.check_halted(q) {
                            return;
                        }
                        let answer = levels.get(i).and_then(|l| {
                            probe.block(i, usize::from(variant), &l[usize::from(variant)])
                        });
                        match answer {
                            Some(true) => {
                                found = Some(i);
                                break;
                            }
                            Some(false) => {}
                            None => open.push(i as u32),
                        }
                    }
                    decided.lock().unwrap().push((k, found, open));
                });
            }
        });
        if self.check_halted(q) {
            return;
        }
        let decided = decided.into_inner().unwrap();
        let facts: Vec<(usize, Seen)> = decided
            .iter()
            .filter_map(|(k, found, open)| match found {
                Some(i) => Some((*k, Seen::Some(*i))),
                None => open.is_empty().then_some((*k, Seen::None(n))),
            })
            .collect();
        self.learn(q, &conds, variant, &facts, n);
        let open: Vec<(usize, Vec<u32>)> = decided
            .into_iter()
            .filter(|(_, found, open)| found.is_none() && !open.is_empty())
            .map(|(k, _, open)| (k, open))
            .collect();
        if open.is_empty() {
            return;
        }
        // One scan of the blocks left open, for all of their conditions.
        let mut union: Vec<u32> = open.iter().flat_map(|(_, b)| b.iter().copied()).collect();
        union.sort_unstable();
        union.dedup();
        let all = open.iter().fold(0u64, |bits, (k, _)| bits | 1 << k);
        let (next, found, failed) = (
            AtomicUsize::new(0),
            AtomicU64::new(0),
            AtomicBool::new(false),
        );
        thread::scope(|scope| {
            for _ in 0..block_workers(self.format) {
                scope.spawn(|| loop {
                    if !wait() || failed.load(Ordering::Acquire) {
                        return;
                    }
                    let left = all & !found.load(Ordering::Acquire);
                    let j = next.fetch_add(1, Ordering::Relaxed);
                    if left == 0 || j >= union.len() {
                        return;
                    }
                    let i = union[j];
                    let wanted = open
                        .iter()
                        .filter(|(k, b)| left & 1 << k != 0 && b.binary_search(&i).is_ok())
                        .fold(0u64, |bits, (k, _)| bits | 1 << k);
                    if wanted == 0 {
                        continue;
                    }
                    let b = blocks[i as usize];
                    let Ok(raw) = self.source.read(b.start, (b.end - b.start) as usize) else {
                        failed.store(true, Ordering::Release);
                        return;
                    };
                    let hits = match &self.dlt {
                        Some(decoder) => q.filter.dlt_row_hits(decoder, &raw, wanted, &q.stop),
                        None => q.filter.row_hits(&raw, wanted, &q.stop),
                    };
                    let new = hits & !found.fetch_or(hits, Ordering::AcqRel);
                    if new != 0 {
                        let facts: Vec<_> = (0..conds.len())
                            .filter(|k| new & 1 << k != 0)
                            .map(|k| (k, Seen::Some(i as usize)))
                            .collect();
                        self.learn(q, &conds, variant, &facts, n);
                    }
                });
            }
        });
        if self.check_halted(q) || failed.load(Ordering::Acquire) {
            return;
        }
        let found = found.into_inner();
        let facts: Vec<_> = open
            .iter()
            .filter(|(k, _)| found & 1 << k == 0)
            .map(|(k, _)| (*k, Seen::None(n)))
            .collect();
        self.learn(q, &conds, variant, &facts, n);
    }
    /// The check yields while a pane's query scans, but not during `hold`.
    fn check_yields(&self) -> bool {
        !self.hold.load(Ordering::Acquire) && self.panes_scan()
    }
    /// Keeps facts about conditions `k` of `q` for the dataset, a match
    /// never replaced by a later "none", and updates `q`'s states from all
    /// it knows of them over `n` blocks; the logs pane hears of a change.
    fn learn(&self, q: &Query, conds: &[&Cond], variant: bool, facts: &[(usize, Seen)], n: usize) {
        // The index lock comes before `seen`, as in `drop_last_block`.
        let live = self.index.lock().unwrap().live;
        let changed = {
            let mut seen = self.seen.lock().unwrap();
            for &(k, fact) in facts {
                let key = (variant, conds[k].condition().clone());
                if !matches!(seen.get(&key), Some(Seen::Some(_))) {
                    seen.insert(key, fact);
                }
            }
            let mut c = q.check.lock().unwrap();
            let mut changed = false;
            for (k, cond) in conds.iter().enumerate() {
                let state = match seen.get(&(variant, cond.condition().clone())) {
                    Some(Seen::Some(_)) => Some(true),
                    Some(Seen::None(m)) if *m >= n => Some(false),
                    // A followed file's earlier "none" holds until a match.
                    Some(Seen::None(_)) if live => c.states[k],
                    _ => None,
                };
                changed |= c.states[k] != state;
                c.states[k] = state;
            }
            changed
        };
        if changed && !q.stop.load(Ordering::Acquire) {
            self.publish_progress(0);
        }
    }
    /// Scans until the query has every block. While the file is followed it
    /// then waits (idle) instead of completing; `hold` pauses it silently.
    fn run_scan(&self, pane: usize, q: &Query) {
        loop {
            let res = self.scan(pane, q);
            let index = self.index.lock().unwrap();
            let mut r = q.result.lock().unwrap();
            if let Err(e) = res {
                r.status = QueryStatus::Error;
                r.error = public_message(&e, "Search failed; retry");
            } else if q.stop.load(Ordering::Acquire) || self.stop.load(Ordering::Acquire) {
                r.status = QueryStatus::Cancelled;
            } else if self.hold.load(Ordering::Acquire) {
                r.idle = true;
                return;
            } else if r.ends.len() < index.blocks.len() {
                // A followed file grew after the scan's last block.
                continue;
            } else if index.live {
                r.idle = true;
            } else {
                r.status = QueryStatus::Complete;
            }
            drop((r, index));
            // A check waiting for the panes' scans goes on.
            if let Some(logs) = self.panes[0].lock().unwrap().clone() {
                logs.wake();
            }
            self.publish_progress(pane);
            return;
        }
    }
    fn scan(&self, pane: usize, q: &Query) -> io::Result<()> {
        let identity = q.filter.identity();
        // The level table is published after analysis. It covers every block
        // or, while a file is followed, the blocks analyzed so far; a scan
        // parses the blocks after them. The field table covers the blocks of
        // the first pass only; `plan` reads the blocks after it.
        let (levels, fields) = {
            let index = self.index.lock().unwrap();
            (index.block_levels.clone(), index.block_fields.clone())
        };
        let plan = fields
            .filter(|_| !q.filter.only_levels())
            .and_then(|f| f.plan(&q.filter));
        let slots = q.filter.level_slots();
        let variant = usize::from(q.filter.fields.recognizes_line_numbers());
        // Blocks are scanned in parallel, but a block's leading rows continue
        // the last entry before it, so their decision is applied in order.
        let orphan = q.filter.admits_unparsed();
        // A followed file's query continues after its last scanned block.
        let (from, mut carry, mut carry_file, mut announced) = {
            let r = q.result.lock().unwrap();
            (r.ends.len(), r.carry, r.carry_file, r.count > 0)
        };
        self.blocks_in_order(
            from,
            if identity {
                1
            } else {
                block_workers(self.format)
            },
            &q.stop,
            |i, b| {
                let scanned = |count, lead, tail| Scanned {
                    end: b.end,
                    file: b.file,
                    count,
                    lead,
                    tail,
                };
                if identity {
                    return Ok(scanned(b.rows as u64, 0, None));
                }
                if let Some(table) = levels.as_ref().filter(|t| i < t.len()) {
                    // The table counts leading rows in their entry's slot.
                    let rows = &table[i][variant];
                    let possible: u64 = (0..9)
                        .filter(|slot| slots & (1 << slot) != 0)
                        .map(|slot| u64::from(rows.slots[slot]))
                        .sum();
                    // No row of a selected level, or levels decide alone.
                    if possible == 0 || q.filter.only_levels() {
                        let tail = rows.tail.map(|level| slots & (1 << level) != 0);
                        return Ok(scanned(possible, 0, tail));
                    }
                    // Field metadata also counts leading rows with their entry.
                    if let Some((count, tail)) =
                        plan.as_ref().and_then(|p| p.decide(i, variant, rows))
                    {
                        return Ok(scanned(count, 0, tail));
                    }
                }
                // Without a search query a DLT block is decided from its
                // messages' headers; only the query needs rendered rows.
                if let (Some(decoder), None) = (&self.dlt, &q.filter.matcher) {
                    let raw = self.source.read(b.start, (b.end - b.start) as usize)?;
                    let Mask { bits, tail, .. } =
                        q.filter.dlt_mask(decoder, &raw, b.rows, &q.stop)?;
                    let count = (0..b.rows)
                        .filter(|i| bits[i / 8] & (1 << (i % 8)) != 0)
                        .count();
                    return Ok(scanned(count as u64, 0, tail));
                }
                let rows = self.read_rows(&b)?;
                let Mask { bits, lead, tail } =
                    q.filter.mask_from(&rows.text, b.rows, &q.stop, true)?;
                let count = (0..b.rows)
                    .filter(|i| bits[i / 8] & (1 << (i % 8)) != 0)
                    .count()
                    - lead;
                Ok(scanned(count as u64, lead as u64, tail))
            },
            |_, s| {
                if carry_file != Some(s.file) {
                    (carry, carry_file) = (orphan, Some(s.file));
                }
                let entry = carry;
                carry = s.tail.unwrap_or(carry);
                let mut r = q.result.lock().unwrap();
                r.count += s.count + if entry { s.lead } else { 0 };
                r.entries.push(entry);
                (r.carry, r.carry_file) = (carry, carry_file);
                let total = r.count;
                r.ends.push(total);
                r.bytes = s.end;
                let first_available = !announced && r.count > 0;
                drop(r);
                if first_available {
                    announced = true;
                    self.publish_progress(pane);
                }
            },
        )
    }
    pub fn status(&self, pane: usize) -> Value {
        // Before the index lock: it waits for a block read in progress.
        let grown = self.source.grown();
        let i = self.index.lock().unwrap();
        // Snapshots are serialized by the index lock, so this revision orders
        // polled replies and pushed notifications that can arrive out of order.
        let revision = self.revision.fetch_add(1, Ordering::Relaxed) + 1;
        let pane_state = self.panes[pane].lock().unwrap();
        let recognize = pane_state
            .as_ref()
            .is_none_or(|q| q.filter.fields.recognizes_line_numbers());
        let q = pane_state.as_ref().map(|q| q.json());
        let summary = if recognize {
            &i.prefix_summary
        } else {
            &i.summary
        };
        let files: Vec<Value> = self
            .source
            .parts()
            .iter()
            .enumerate()
            .map(|(k, p)| json!({"name":p.name,"size":self.source.part_size(k),"firstLine":i.file_rows.get(k)}))
            .collect();
        // While followed, the summary covers the rows so far, not the file.
        let summarized = i.summary.is_some() && !i.live && i.analyzed >= i.blocks.len();
        let mut status = json!({"datasetId":self.id,"revision":revision,"name":self.source.name,"files":files,"size":self.source.current_size(),"status":if self.stop.load(Ordering::Acquire){"closed"}else if !i.error.is_empty(){"error"}else if i.done{"ready"}else{"indexing"},"indexedBytes":i.bytes,"total":i.total,"textComplete":i.done,"summaryComplete":summarized,"complete":i.done&&summarized,"summary":summary,"analysis":{"status":if summarized{"complete"}else{"running"}},"followable":self.source.followable(),"following":i.live,"grown":grown&&!i.live,"followError":i.follow_error,"tailRevision":i.replaced,"error":i.error,"query":q,"format":self.format.as_str()});
        if self.format == Format::Dlt {
            let d = i.dlt_issues;
            status["dltIssues"] = json!({"garbage":d.garbage,"garbageBytes":d.garbage_bytes,"truncated":d.truncated,"unsupported":d.unsupported});
        }
        status
    }
    pub fn selected(&self, pane: usize, id: u64) -> io::Result<Arc<Query>> {
        self.panes[pane]
            .lock()
            .unwrap()
            .as_ref()
            .filter(|q| q.id == id)
            .cloned()
            .ok_or_else(|| error("Stale query"))
    }
    // Resolve physical IDs into the current filtered result order using sparse
    // per-block counts and one bounded block; never materialize prior rows.
    pub fn locate(&self, q: &Query, line: u64) -> io::Result<Value> {
        let (bi, block) = {
            let index = self.index.lock().unwrap();
            let bi = index
                .blocks
                .partition_point(|b| b.first + b.rows as u64 <= line);
            let block = index
                .blocks
                .get(bi)
                .copied()
                .filter(|b| line >= b.first)
                .ok_or_else(|| error("Physical row is not indexed yet; retry"))?;
            (bi, block)
        };
        let (prefix, entry) = {
            let result = q.result.lock().unwrap();
            if result.ends.get(bi).is_none() {
                // A running query may still reach this block. Once it stopped
                // (cancelled/error) the block was never counted, so absence is
                // unknown, not a final "not in the results".
                let pending = result.status == QueryStatus::Running;
                return Ok(
                    json!({"datasetId":self.id,"queryId":q.id,"lineNumber":line,"found":false,"pending":pending,"incomplete":!pending}),
                );
            }
            (
                bi.checked_sub(1).map_or(0, |p| result.ends[p]),
                result.entries[bi],
            )
        };
        if self.stop.load(Ordering::Acquire) {
            return Err(error("Source closed"));
        }
        let rows = self.read_rows(&block)?;
        let mask = q
            .filter
            .mask_from(&rows.text, block.rows, &self.stop, entry)?
            .bits;
        // `mask` returns an empty mask when stopped; that is not "not found".
        if self.stop.load(Ordering::Acquire) {
            return Err(error("Source closed"));
        }
        let local = (line - block.first) as usize;
        let found = mask[local / 8] & (1 << (local % 8)) != 0;
        let preceding = (0..local)
            .filter(|i| mask[i / 8] & (1 << (i % 8)) != 0)
            .count() as u64;
        Ok(
            json!({"datasetId":self.id,"queryId":q.id,"lineNumber":line,"found":found,"pending":false,"index":prefix+preceding}),
        )
    }
    pub fn page(&self, q: &Query, start: u64, count: usize) -> io::Result<Value> {
        self.page_marked(q, start, count, None)
    }
    /// A page whose rows also carry `search`'s highlights and a `searchHit`
    /// mark. `search` applies only with the same field conditions, so a
    /// marked row of `q` is exactly a result of `search`; the page then names
    /// it as `searchQueryId`. Only rows that reach the page are matched.
    pub fn page_marked(
        &self,
        q: &Query,
        start: u64,
        count: usize,
        search: Option<&Query>,
    ) -> io::Result<Value> {
        let search = search.filter(|s| {
            q.filter.matcher.is_none()
                && s.filter.matcher.is_some()
                && s.filter.same_fields(&q.filter)
        });
        // Snapshot the counted prefix; rows stay within the reported count.
        let (matched, counted, mut bi, mut skip) = {
            let r = q.result.lock().unwrap();
            let bi = r.ends.partition_point(|&e| e <= start);
            let before = bi.checked_sub(1).map_or(0, |p| r.ends[p]);
            (r.count, r.ends.len(), bi, start - before.min(start))
        };
        if count == 0 {
            return Ok(self.page_json(q, search, start, matched, vec![]));
        }
        let (mut rows, mut wire, mut reads) = (vec![], 0usize, 0usize);
        let stop = AtomicBool::new(false);
        let recognize = q.filter.fields.recognizes_line_numbers();
        loop {
            // Copy bounded windows of block metadata instead of the index.
            let blocks = {
                let index = self.index.lock().unwrap();
                let end = counted.min(index.blocks.len());
                index.blocks[bi.min(end)..end.min(bi + 256)].to_vec()
            };
            if blocks.is_empty() {
                break;
            }
            let (counts, entries): (Vec<u64>, Vec<bool>) = {
                let r = q.result.lock().unwrap();
                (bi..bi + blocks.len())
                    .map(|i| {
                        (
                            r.ends[i] - i.checked_sub(1).map_or(0, |p| r.ends[p]),
                            r.entries[i],
                        )
                    })
                    .unzip()
            };
            for (k, b) in blocks.iter().enumerate() {
                if self.stop.load(Ordering::Acquire) {
                    return Err(error("Source closed"));
                }
                if skip >= counts[k] {
                    skip -= counts[k];
                    continue;
                }
                if reads + (b.end - b.start) as usize > 8 * 1024 * 1024 {
                    return Ok(self.page_json(q, search, start, matched, rows));
                }
                let block_rows = self.read_rows(b)?;
                let data = &block_rows.text;
                reads += data.len();
                let mask = q.filter.mask_from(data, b.rows, &stop, entries[k])?.bits;
                let lines: Vec<(usize, &[u8])> = ByteLines::new(data).collect();
                let mut owners = Owners::new(bi + k, b);
                for (j, &(offset, bytes)) in lines.iter().enumerate() {
                    let line = b.first + j as u64;
                    if mask[j / 8] & (1 << (j % 8)) == 0 {
                        continue;
                    }
                    if skip > 0 {
                        skip -= 1;
                        continue;
                    }
                    // Only rows that reach the page are decoded.
                    let raw = encoding::decode(bytes);
                    let record = parser::parse_in(self.format, &raw, line, recognize);
                    let mut value = record.json(4096);
                    if record.parsed.is_none() {
                        if let Some(owner) = owners.owner(self, &lines, j, recognize)? {
                            value["owner"] = owner;
                        }
                    }
                    if let Some(s) = search {
                        let ranges = s.filter.highlights(&raw);
                        if !ranges.is_empty() || s.filter.row_matches(&raw) {
                            value["searchHit"] = json!(true);
                        }
                        value["highlights"] = json!(ranges);
                    } else {
                        value["highlights"] = json!(q.filter.highlights(&raw));
                    }
                    value["rawBytes"] = json!(bytes.len());
                    value["sourceStartByte"] = json!(Self::row_source(b, &block_rows, j, offset));
                    self.mark_file(&mut value, b.file, line);
                    let damaged = matches!(raw, std::borrow::Cow::Owned(_));
                    if damaged {
                        value["encodingDamaged"] = json!(true);
                        value["encodingDamage"] = encoding::damage(
                            bytes,
                            0,
                            value["raw"].as_str().unwrap().len(),
                            b.start + offset as u64,
                        );
                    }
                    if let Some(p) = &record.parsed {
                        // Fields appear in row order, so UTF-16 offsets advance from
                        // the previous field instead of recounting the whole prefix.
                        let (mut from, mut units) = (0usize, 0usize);
                        let replaced = if damaged {
                            encoding::replacement_offsets(bytes)
                        } else {
                            vec![]
                        };
                        for (key, text) in p.spans() {
                            // Absent iOS IDs and Android's empty tag are static
                            // empty strings, not slices of the decoded row.
                            if text.is_empty() {
                                continue;
                            }
                            let at = text.as_ptr() as usize - raw.as_ptr() as usize;
                            if at < from {
                                (from, units) = (0, 0);
                            }
                            units += raw[from..at].encode_utf16().count();
                            from = at;
                            value["fieldOffsets"][key] = json!(units);
                            if damaged {
                                let end = at + value[key].as_str().map_or(text.len(), str::len);
                                // Clean fields carry no damage object; only fields
                                // that contain a replacement repeat the row metadata.
                                let i = replaced.partition_point(|&d| d < at);
                                if replaced.get(i).is_some_and(|&d| d < end) {
                                    value["fieldDamage"][key] =
                                        encoding::damage(bytes, at, end, b.start + offset as u64);
                                }
                            }
                        }
                    }
                    let bytes = serde_json::to_vec(&value)?.len();
                    if wire + bytes > 512 * 1024 {
                        return Ok(self.page_json(q, search, start, matched, rows));
                    }
                    wire += bytes;
                    rows.push(value);
                    if rows.len() >= count.min(200) {
                        return Ok(self.page_json(q, search, start, matched, rows));
                    }
                }
            }
            bi += blocks.len();
        }
        Ok(self.page_json(q, search, start, matched, rows))
    }
    /// Writes every result row of a completed query as its original bytes and
    /// line ending, in physical order. Only blocks with matches are read.
    /// `progress` receives (rows, bytes) after each written block.
    pub fn export(
        &self,
        q: &Query,
        out: &mut dyn io::Write,
        cancel: &AtomicBool,
        progress: impl FnMut(u64, u64),
    ) -> io::Result<(u64, u64)> {
        let e = self.export_as(q, ExportKind::Original, out, cancel, progress)?;
        Ok((e.rows, e.bytes))
    }
    /// Writes a completed query's result rows: `Original` writes each row's
    /// source bytes (a text row with its own line ending, a DLT row its
    /// message, skipping rows of other bytes); `Text` writes a DLT row's
    /// text with LF.
    pub fn export_as(
        &self,
        q: &Query,
        kind: ExportKind,
        out: &mut dyn io::Write,
        cancel: &AtomicBool,
        mut progress: impl FnMut(u64, u64),
    ) -> io::Result<Exported> {
        let messages = match (self.format, kind) {
            (Format::Text, ExportKind::Text) => return Err(error("Invalid export format")),
            (Format::Dlt, ExportKind::Original) => true,
            _ => false,
        };
        let (ends, entries) = {
            let r = q.result.lock().unwrap();
            if r.status != QueryStatus::Complete {
                return Err(error("Wait until the results are complete"));
            }
            (r.ends.clone(), r.entries.clone())
        };
        let blocks = self.index.lock().unwrap().blocks.clone();
        let stop = AtomicBool::new(false);
        let (mut rows, mut bytes, mut skipped) = (0u64, 0u64, 0u64);
        // Only a file's last row can lack a line ending. When a row of a later
        // file follows it, an LF keeps the two rows apart.
        let mut unterminated = false;
        for (i, b) in blocks.iter().enumerate().take(ends.len()) {
            if self.stop.load(Ordering::Acquire) {
                return Err(error("Source closed"));
            }
            if cancel.load(Ordering::Acquire) {
                return Err(error("Export cancelled"));
            }
            if ends[i] == i.checked_sub(1).map_or(0, |p| ends[p]) {
                continue;
            }
            let raw = self.source.read(b.start, (b.end - b.start) as usize)?;
            let decoded = self.dlt.is_some().then(|| self.render(b, &raw));
            let (data, units) = match &decoded {
                Some(r) => (&r.text, r.units.as_deref()),
                None => (&raw, None),
            };
            let mask = q.filter.mask_from(data, b.rows, &stop, entries[i])?.bits;
            for (j, (offset, line)) in ByteLines::new(data).enumerate() {
                if mask[j / 8] & (1 << (j % 8)) == 0 {
                    continue;
                }
                if let (true, Some(units)) = (messages, units) {
                    // A DLT row is its message; rows of other bytes have none.
                    if line.starts_with(dlt::ISSUE.as_bytes()) {
                        skipped += 1;
                        continue;
                    }
                    let message = &raw[units[j] as usize..units[j + 1] as usize];
                    out.write_all(message)?;
                    rows += 1;
                    bytes += message.len() as u64;
                    continue;
                }
                // Keep the row's own terminator: CRLF, CR, LF or none at EOF.
                let after = offset + line.len();
                let ending = match (data.get(after), data.get(after + 1)) {
                    (Some(b'\r'), Some(b'\n')) => 2,
                    (Some(b'\r' | b'\n'), _) => 1,
                    _ => 0,
                };
                if unterminated {
                    out.write_all(b"\n")?;
                    bytes += 1;
                }
                unterminated = ending == 0;
                out.write_all(&data[offset..after + ending])?;
                rows += 1;
                bytes += (line.len() + ending) as u64;
            }
            progress(rows, bytes);
        }
        out.flush()?;
        Ok(Exported {
            rows,
            bytes,
            skipped,
        })
    }
    /// The last parsed row before block `bi` in the same file, which the
    /// block's leading rows without fields continue. The analysis table
    /// knows it; before the table exists a bounded look-back reads earlier
    /// blocks, and an entry beyond it stays unknown (None), like an orphan.
    fn entry_owner(&self, bi: usize, recognize: bool) -> io::Result<Option<u64>> {
        let (table, blocks) = {
            let index = self.index.lock().unwrap();
            let from = bi.saturating_sub(OWNER_LOOKBACK);
            (index.block_levels.clone(), index.blocks[from..=bi].to_vec())
        };
        if let Some(levels) = table.as_ref().and_then(|t| t.get(bi)) {
            return Ok(levels[usize::from(recognize)].owner);
        }
        let file = blocks.last().unwrap().file;
        for b in blocks[..blocks.len() - 1].iter().rev() {
            if b.file != file {
                break;
            }
            let data = self.read_rows(b)?.text;
            let lines: Vec<_> = ByteLines::new(&data).collect();
            if let Some((j, parsed)) = (0..lines.len())
                .rev()
                .find_map(|j| opens_entry(self.format, lines[j].1, recognize).map(|p| (j, p)))
            {
                return Ok(parsed.then_some(b.first + j as u64));
            }
        }
        Ok(None)
    }
    /// Reads physical row `line` of an indexed block.
    fn row_bytes(&self, line: u64) -> io::Result<Vec<u8>> {
        let block = {
            let index = self.index.lock().unwrap();
            let i = index
                .blocks
                .partition_point(|b| b.first + b.rows as u64 <= line);
            index
                .blocks
                .get(i)
                .copied()
                .filter(|b| line >= b.first)
                .ok_or_else(|| error("Physical row is not indexed"))?
        };
        let data = self.read_rows(&block)?.text;
        let (_, bytes) = ByteLines::new(&data)
            .nth((line - block.first) as usize)
            .ok_or_else(|| error("Physical row is not indexed"))?;
        Ok(bytes.to_vec())
    }
    /// Rows of a combined source name their file and row within it.
    fn mark_file(&self, value: &mut Value, file: usize, line: u64) {
        if self.source.parts().len() > 1 {
            let first = self.index.lock().unwrap().file_rows[file];
            value["file"] = json!(file);
            value["fileLine"] = json!(line - first + 1);
        }
    }
    fn page_json(
        &self,
        q: &Query,
        search: Option<&Query>,
        start: u64,
        count: u64,
        rows: Vec<Value>,
    ) -> Value {
        let mut page =
            json!({"datasetId":self.id,"queryId":q.id,"start":start,"count":count,"rows":rows});
        if let Some(s) = search {
            page["searchQueryId"] = json!(s.id);
        }
        page
    }
    pub fn range(&self, q: &Query, line: u64, start: usize) -> io::Result<Value> {
        let block = {
            let index = self.index.lock().unwrap();
            let i = index
                .blocks
                .partition_point(|b| b.first + b.rows as u64 <= line);
            index
                .blocks
                .get(i)
                .copied()
                .filter(|b| line >= b.first)
                .ok_or_else(|| error("Physical row is not indexed"))?
        };
        let rows = self.read_rows(&block)?;
        let local = (line - block.first) as usize;
        let (offset, bytes) = ByteLines::new(&rows.text).nth(local).unwrap();
        if start > bytes.len() {
            return Err(error("Invalid detail offset"));
        }
        let (from, end) = encoding::window(bytes, start, 65536);
        // A DLT row's text is no source bytes: name its message's start.
        let source = match rows.units {
            Some(_) => Self::row_source(&block, &rows, local, offset),
            None => block.start + (offset + from) as u64,
        };
        let text = encoding::decode(&bytes[from..end]);
        let mut value = json!({"datasetId":self.id,"queryId":q.id,"lineNumber":line,"startByte":from,"nextByte":end,"totalBytes":bytes.len(),"sourceStartByte":source,"complete":end==bytes.len(),"text":text,"highlights":q.filter.highlights(&text)});
        self.mark_file(&mut value, block.file, line);
        if matches!(text, std::borrow::Cow::Owned(_)) {
            value["encodingDamage"] = encoding::damage(
                &bytes[from..end],
                0,
                text.len(),
                block.start + offset as u64 + from as u64,
            );
        }
        Ok(value)
    }
    /// Rows at ascending physical line numbers whatever `q` filters, parsed
    /// like page rows (`q` only sets line-number recognition) without
    /// highlights. Restored bookmarks read their rows again through this
    /// instead of storing log text. Stops at a row not yet indexed or at the
    /// page wire budget; `next` counts the requested lines answered.
    pub fn lines(&self, q: &Query, wanted: &[u64]) -> io::Result<Value> {
        let recognize = q.filter.fields.recognizes_line_numbers();
        let (mut rows, mut wire, mut at) = (vec![], 0usize, 0usize);
        while let Some(&line) = wanted.get(at) {
            let found = {
                let index = self.index.lock().unwrap();
                let i = index
                    .blocks
                    .partition_point(|b| b.first + b.rows as u64 <= line);
                index
                    .blocks
                    .get(i)
                    .copied()
                    .filter(|b| line >= b.first)
                    .map(|b| (i, b))
            };
            let Some((bi, b)) = found else { break };
            if self.stop.load(Ordering::Acquire) {
                return Err(error("Source closed"));
            }
            let block_rows = self.read_rows(&b)?;
            let lines: Vec<(usize, &[u8])> = ByteLines::new(&block_rows.text).collect();
            let mut owners = Owners::new(bi, &b);
            while let Some(&line) = wanted.get(at).filter(|&&l| l < b.first + b.rows as u64) {
                let j = (line - b.first) as usize;
                let (offset, bytes) = lines[j];
                let raw = encoding::decode(bytes);
                let record = parser::parse_in(self.format, &raw, line, recognize);
                let mut value = record.json(4096);
                if record.parsed.is_none() {
                    if let Some(owner) = owners.owner(self, &lines, j, recognize)? {
                        value["owner"] = owner;
                    }
                }
                value["rawBytes"] = json!(bytes.len());
                value["sourceStartByte"] = json!(Self::row_source(&b, &block_rows, j, offset));
                if matches!(raw, std::borrow::Cow::Owned(_)) {
                    value["encodingDamaged"] = json!(true);
                }
                self.mark_file(&mut value, b.file, line);
                let size = serde_json::to_vec(&value)?.len();
                if !rows.is_empty() && wire + size > 512 * 1024 {
                    return Ok(json!({"datasetId":self.id,"queryId":q.id,"rows":rows,"next":at}));
                }
                wire += size;
                rows.push(value);
                at += 1;
            }
        }
        Ok(json!({"datasetId":self.id,"queryId":q.id,"rows":rows,"next":at}))
    }
    /// One display window of the log entry that physical row `line` belongs
    /// to: its parsed entry row and the rows without fields that continue it
    /// in the same file, joined by LF like a multi-row copy. `startByte`,
    /// `nextByte` and `totalBytes` count bytes of that joined text. A row
    /// outside a parsed entry (an orphan, or an entry beyond the look-back
    /// before analysis) is its own record, like `range`. An entry longer than
    /// `RECORD_BYTES` or `RECORD_ROWS` keeps its first rows and reports
    /// `recordComplete: false`, as does one that may continue past the rows
    /// indexed so far.
    pub fn record(&self, q: &Query, line: u64, start: usize) -> io::Result<Value> {
        let recognize = q.filter.fields.recognizes_line_numbers();
        let block_at = |i: usize| self.index.lock().unwrap().blocks.get(i).copied();
        let bi = {
            let index = self.index.lock().unwrap();
            index
                .blocks
                .partition_point(|b| b.first + b.rows as u64 <= line)
        };
        let block = block_at(bi)
            .filter(|b| line >= b.first)
            .ok_or_else(|| error("Physical row is not indexed"))?;
        let local = (line - block.first) as usize;
        let first = {
            let data = self.read_rows(&block)?.text;
            let lines: Vec<_> = ByteLines::new(&data).collect();
            match (0..=local)
                .rev()
                .find_map(|j| opens_entry(self.format, lines[j].1, recognize).map(|p| (j, p)))
            {
                Some((j, parsed)) => parsed.then_some(block.first + j as u64),
                None => self.entry_owner(bi, recognize)?,
            }
        };
        // Rows of the record: (source offset, bytes), from its entry row on.
        let mut rows: Vec<(u64, Vec<u8>)> = vec![];
        let (mut bytes, mut complete) = (0usize, false);
        if let Some(first) = first {
            let mut i = bi;
            while block_at(i).is_some_and(|b| b.first > first) {
                i -= 1;
            }
            'blocks: loop {
                let Some(b) = block_at(i).filter(|b| b.file == block.file) else {
                    // A later file starts, or the indexed rows end here.
                    complete = block_at(i).is_some() || self.index.lock().unwrap().done;
                    break;
                };
                if self.stop.load(Ordering::Acquire) {
                    return Err(error("Source closed"));
                }
                let block_rows = self.read_rows(&b)?;
                for (j, (offset, row)) in ByteLines::new(&block_rows.text).enumerate() {
                    let at = b.first + j as u64;
                    if at < first {
                        continue;
                    }
                    // Rows up to `line` are known to continue the entry.
                    if at > line && opens_entry(self.format, row, recognize).is_some() {
                        complete = true;
                        break 'blocks;
                    }
                    let join = usize::from(!rows.is_empty());
                    if !rows.is_empty()
                        && (bytes + join + row.len() > RECORD_BYTES || rows.len() >= RECORD_ROWS)
                    {
                        break 'blocks;
                    }
                    bytes += join + row.len();
                    rows.push((Self::row_source(&b, &block_rows, j, offset), row.to_vec()));
                }
                i += 1;
            }
        }
        let first_line = match first {
            Some(first) if first + rows.len() as u64 > line => first,
            // The entry is too long to reach the selected row: show the row.
            _ => {
                let block_rows = self.read_rows(&block)?;
                let (offset, row) = ByteLines::new(&block_rows.text).nth(local).unwrap();
                rows = vec![(
                    Self::row_source(&block, &block_rows, local, offset),
                    row.to_vec(),
                )];
                complete = true;
                line
            }
        };
        let mut joined = Vec::with_capacity(bytes);
        let mut starts = Vec::with_capacity(rows.len());
        for (k, (source, row)) in rows.iter().enumerate() {
            if k > 0 {
                joined.push(b'\n');
            }
            starts.push((joined.len(), *source));
            joined.extend_from_slice(row);
        }
        if start > joined.len() {
            return Err(error("Invalid detail offset"));
        }
        // A joined offset maps to the source byte of its row; a DLT row's
        // text maps to its message's start.
        let source_at = |at: usize| {
            let (row, source) = starts[starts.partition_point(|s| s.0 <= at) - 1];
            match self.format {
                Format::Text => source + (at - row) as u64,
                Format::Dlt => source,
            }
        };
        let (from, end) = encoding::window(&joined, start, 65536);
        let text = encoding::decode(&joined[from..end]);
        // Each physical row is matched on its own, as in pages.
        let (mut highlights, mut units) = (Vec::<[usize; 2]>::new(), 0);
        for part in text.split('\n') {
            for [a, b] in q.filter.highlights(part) {
                highlights.push([units + a, units + b]);
            }
            units += part.encode_utf16().count() + 1;
        }
        highlights.truncate(32);
        let mut value = json!({"datasetId":self.id,"queryId":q.id,"lineNumber":line,"first":first_line,"last":first_line+rows.len() as u64-1,"recordComplete":complete,"startByte":from,"nextByte":end,"totalBytes":joined.len(),"sourceStartByte":source_at(from),"complete":end==joined.len(),"text":text,"highlights":highlights});
        self.mark_file(&mut value, block.file, first_line);
        if matches!(text, std::borrow::Cow::Owned(_)) {
            let mut damage = encoding::damage(&joined[from..end], 0, text.len(), 0);
            for sample in damage["samples"].as_array_mut().unwrap() {
                let at = from + sample["startByte"].as_u64().unwrap() as usize;
                sample["sourceStartByte"] = json!(source_at(at));
            }
            value["encodingDamage"] = damage;
        }
        Ok(value)
    }
    /// A block's rows. Every pass over indexed rows reads them here; only
    /// indexing and a followed file's edges read source bytes directly. A
    /// DLT block's units render as rows; issue rows name their offset in
    /// their own file.
    fn read_rows(&self, b: &Block) -> io::Result<Rows> {
        let raw = self.source.read(b.start, (b.end - b.start) as usize)?;
        Ok(match self.dlt {
            Some(_) => self.render(b, &raw),
            None => Rows {
                text: raw,
                units: None,
            },
        })
    }
    /// The rows of DLT block `b` from its source bytes `raw`.
    fn render(&self, b: &Block, raw: &[u8]) -> Rows {
        let decoder = self.dlt.as_ref().expect("a DLT dataset");
        let mut text = Vec::with_capacity(raw.len() + raw.len() / 2);
        let base = b.start - self.source.parts()[b.file].offset;
        let units = decoder.render(raw, base, &mut text);
        Rows {
            text,
            units: Some(units),
        }
    }
    /// Source byte of row `j` of block `b`, whose text starts at `offset`
    /// of its rows: for DLT, the start of its unit.
    fn row_source(b: &Block, rows: &Rows, j: usize, offset: usize) -> u64 {
        b.start
            + match &rows.units {
                Some(units) => units[j] as u64,
                None => offset as u64,
            }
    }
    /// Starts or stops following a single file's growth. While followed,
    /// appended rows are indexed every `follow_poll_ms` and queries and the
    /// analysis continue over them instead of completing. Turning it off
    /// ends the rows at the file's length then, as opening it then would,
    /// and completes them; later appended bytes are ignored.
    pub fn follow(self: &Arc<Self>, on: bool) -> io::Result<()> {
        let _gate = self.follow_gate.lock().unwrap();
        if !on {
            self.unfollow();
            return Ok(());
        }
        if self.format == Format::Dlt {
            return Err(error("DLT files cannot be followed"));
        }
        if !self.source.followable() {
            return Err(error("Only one opened log file can be followed"));
        }
        {
            let index = self.index.lock().unwrap();
            if self.stop.load(Ordering::Acquire) {
                return Err(error("Source closed"));
            }
            if !index.error.is_empty() {
                return Err(error(&index.error));
            }
            if !index.done {
                return Err(error("Wait until the file is read"));
            }
            if index.live {
                return Ok(());
            }
        }
        // Accept the bytes appended so far; the follower indexes them.
        self.source.grow()?;
        {
            let mut index = self.index.lock().unwrap();
            index.live = true;
            index.follow_error.clear();
        }
        // Completed results continue with the rows that follow.
        for pane in &self.panes {
            if let Some(q) = pane.lock().unwrap().clone() {
                let mut r = q.result.lock().unwrap();
                if r.status == QueryStatus::Complete {
                    (r.status, r.idle) = (QueryStatus::Running, true);
                }
            }
        }
        let previous = self.follower.lock().unwrap().take();
        if let Some(h) = previous {
            let _ = h.join();
        }
        let d = self.clone();
        *self.follower.lock().unwrap() = Some(thread::spawn(move || d.follow_loop()));
        Ok(())
    }
    fn follow_loop(self: &Arc<Self>) {
        // `follow` accepted the bytes appended before it started.
        let mut grown = true;
        loop {
            if grown {
                if let Err(e) = self.index_growth(false) {
                    return self.stop_following(e, false);
                }
                self.resume_passes();
            }
            let poll = self.follow_poll_ms.load(Ordering::Relaxed);
            let deadline = Instant::now() + Duration::from_millis(poll);
            let mut index = self.index.lock().unwrap();
            loop {
                if !index.live
                    || self.stop.load(Ordering::Acquire)
                    || self.unfollow.load(Ordering::Acquire)
                {
                    return;
                }
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    break;
                }
                index = self.changed.wait_timeout(index, left).unwrap().0;
            }
            drop(index);
            grown = match self.source.grow() {
                Ok(grown) => grown.is_some(),
                Err(e) => return self.stop_following(e, true),
            };
        }
    }
    fn unfollow(self: &Arc<Self>) {
        if !self.index.lock().unwrap().live {
            return;
        }
        self.unfollow.store(true, Ordering::Release);
        self.changed.notify_all();
        let follower = self.follower.lock().unwrap().take();
        if let Some(h) = follower {
            let _ = h.join();
        }
        self.unfollow.store(false, Ordering::Release);
        // Following may have stopped by itself meanwhile.
        if !self.index.lock().unwrap().live {
            return;
        }
        if let Err(e) = self.source.grow() {
            return self.stop_following(e, true);
        }
        if let Err(e) = self.index_growth(true) {
            return self.stop_following(e, false);
        }
        self.index.lock().unwrap().live = false;
        self.changed.notify_all();
        self.resume_passes();
    }
    /// Following stops by itself. A changed file fails everything, as reads
    /// of it would; invalid appended rows end following at the last valid row.
    fn stop_following(self: &Arc<Self>, e: io::Error, changed: bool) {
        if self.stop.load(Ordering::Acquire) {
            return;
        }
        let message = public_message(&e, "Could not read the log; reopen the file");
        {
            let mut index = self.index.lock().unwrap();
            index.live = false;
            if changed {
                index.error = message;
            } else {
                index.follow_error = message;
            }
        }
        self.changed.notify_all();
        self.resume_passes();
    }
    /// Indexes the bytes a followed file accepted after the indexed rows.
    /// They may first end the last row: an LF after a CR joins its line
    /// ending, and a last row without one is replaced once a line ending
    /// (or `close`, as at a file's end) completes it.
    fn index_growth(&self, close: bool) -> io::Result<()> {
        let len = self.source.current_size();
        if len > MAX_FILE {
            return Err(error("Following stopped at 8 GiB"));
        }
        let (mut start, last) = {
            let index = self.index.lock().unwrap();
            (index.bytes, index.blocks.last().copied())
        };
        if len <= start {
            return Ok(());
        }
        if let Some(b) = last {
            let edge = self.source.read(start - 1, 2)?;
            match edge[0] {
                b'\n' => {}
                b'\r' => {
                    if edge[1] == b'\n' {
                        let mut index = self.index.lock().unwrap();
                        index.blocks.last_mut().unwrap().end += 1;
                        index.bytes += 1;
                        start += 1;
                    }
                }
                _ => {
                    if !close && !self.completes_row(b.start, start, len)? {
                        return Ok(());
                    }
                    self.drop_last_block();
                    start = b.start;
                }
            }
        }
        let row = self.index.lock().unwrap().total + 1;
        self.index_file(0, start, len, row, close)?;
        Ok(())
    }
    /// Whether bytes `from..to` end the row that starts at `row`: an LF, or
    /// a CR that more bytes follow.
    fn completes_row(&self, row: u64, from: u64, to: u64) -> io::Result<bool> {
        let mut at = from;
        while at < to {
            let n = (to - at).min(BLOCK as u64) as usize;
            let data = self.source.read(at, n)?;
            let found = memchr::memchr2(b'\r', b'\n', &data);
            let end = found.map_or(at + n as u64, |i| at + i as u64);
            // Checked before the row is dropped, as indexing it would fail.
            if end - row > MAX_LINE as u64 {
                return Err(error("A physical line exceeds 1 MiB"));
            }
            if let Some(i) = found {
                return Ok(data[i] == b'\n' || end + 1 < to);
            }
            at = end;
        }
        Ok(false)
    }
    /// Removes the last block, a row without a line ending that appended
    /// bytes complete, from the index and from every pass that went
    /// through it. The passes pause meanwhile and continue from there.
    fn drop_last_block(&self) {
        self.hold.store(true, Ordering::Release);
        self.changed.notify_all();
        if let Some(logs) = self.panes[0].lock().unwrap().clone() {
            logs.wake();
        }
        for pane in &self.panes {
            let q = pane.lock().unwrap().clone();
            for slot in q.iter().flat_map(|q| [&q.worker, &q.checker]) {
                let worker = slot.lock().unwrap().take();
                if let Some(h) = worker {
                    let _ = h.join();
                }
            }
        }
        // Take the analysis state as a pass would; one running stops after
        // its current block.
        let mut analysis = loop {
            if let Some(a) = self.analysis.lock().unwrap().take() {
                break Some(a);
            }
            if self.stop.load(Ordering::Acquire) {
                break None;
            }
            thread::sleep(Duration::from_millis(1));
        };
        let mut index = self.index.lock().unwrap();
        let b = index.blocks.pop().unwrap();
        let k = index.blocks.len();
        index.total -= b.rows as u64;
        index.bytes = b.start;
        index.replaced += 1;
        index.analyzed = index.analyzed.min(k);
        if let Some(table) = index.block_levels.as_mut().filter(|t| t.len() > k) {
            Arc::make_mut(table).truncate(k);
        }
        if let Some(table) = index.block_fields.as_mut().filter(|t| t.blocks() > k) {
            Arc::make_mut(table).truncate(k);
        }
        for pane in &self.panes {
            if let Some(q) = pane.lock().unwrap().as_ref() {
                q.result.lock().unwrap().truncate(k, &index.blocks);
            }
        }
        // Facts from the dropped block go; its conditions are checked again.
        {
            let mut seen = self.seen.lock().unwrap();
            seen.retain(|_, s| !matches!(s, Seen::Some(b) if *b >= k));
            for s in seen.values_mut() {
                if let Seen::None(m) = s {
                    *m = (*m).min(k);
                }
            }
            if let Some(q) = self.panes[0].lock().unwrap().as_ref() {
                let variant = q.filter.fields.recognizes_line_numbers();
                let mut c = q.check.lock().unwrap();
                for (state, cond) in c.states.iter_mut().zip(q.filter.conditions()) {
                    if !seen.contains_key(&(variant, cond.condition().clone())) {
                        *state = None;
                    }
                }
            }
        }
        if let Some(a) = analysis.as_mut().filter(|a| a.levels.len() > k) {
            a.drop_last();
        }
        *self.analysis.lock().unwrap() = analysis;
        drop(index);
        self.hold.store(false, Ordering::Release);
    }
    /// Starts the passes waiting at the end of a followed file's rows again:
    /// queries continue, or complete once following stopped, and the
    /// analysis takes the new blocks.
    fn resume_passes(self: &Arc<Self>) {
        for (pane, gate) in self.gates.iter().enumerate() {
            // The gate orders this with a replacing query and with close.
            let _gate = gate.lock().unwrap();
            if self.stop.load(Ordering::Acquire) {
                return;
            }
            let Some(q) = self.panes[pane].lock().unwrap().clone() else {
                continue;
            };
            let mut worker = q.worker.lock().unwrap();
            {
                let mut r = q.result.lock().unwrap();
                if r.status != QueryStatus::Running || !r.idle || q.stop.load(Ordering::Acquire) {
                    continue;
                }
                r.idle = false;
            }
            if let Some(h) = worker.take() {
                let _ = h.join();
            }
            let (d, scan) = (self.clone(), q.clone());
            *worker = Some(thread::spawn(move || d.run_scan(pane, &scan)));
        }
        // The logs pane's check covers the new blocks too.
        if let Some(q) = self.panes[0].lock().unwrap().clone() {
            let _gate = self.gates[0].lock().unwrap();
            let start = {
                let mut c = q.check.lock().unwrap();
                if c.states.is_empty() || q.stop.load(Ordering::Acquire) {
                    false
                } else if c.running {
                    c.again = true;
                    false
                } else {
                    c.running = true;
                    true
                }
            };
            if start {
                let mut checker = q.checker.lock().unwrap();
                if let Some(h) = checker.take() {
                    let _ = h.join();
                }
                let (d, c) = (self.clone(), q.clone());
                *checker = Some(thread::spawn(move || d.run_check(&c)));
            }
        }
        let more = {
            let index = self.index.lock().unwrap();
            let analysis = self.analysis.lock().unwrap();
            analysis
                .as_ref()
                .is_some_and(|a| a.levels.len() < index.blocks.len())
        };
        if more {
            let d = self.clone();
            let mut workers = self.workers.lock().unwrap();
            workers.retain(|h| !h.is_finished());
            workers.push(thread::spawn(move || {
                let _ = d.analyze();
            }));
        }
    }

    pub fn suspend(self: &Arc<Self>) {
        let _ = self.follow(false);
        self.pause_analysis.store(true, Ordering::Release);
        for pane in &self.panes {
            let q = pane.lock().unwrap().clone();
            if let Some(q) = q {
                q.cancel();
            }
        }
    }
    pub fn resume(&self) {
        self.pause_analysis.store(false, Ordering::Release);
    }
    pub fn close(&self) {
        self.stop.store(true, Ordering::Release);
        self.changed.notify_all();
        // The gate keeps a followed file's resumed scans from starting now.
        for (pane, gate) in self.panes.iter().zip(&self.gates) {
            let _gate = gate.lock().unwrap();
            let q = pane.lock().unwrap().take();
            if let Some(q) = q {
                q.cancel();
            }
        }
        self.source.close();
        let follower = self.follower.lock().unwrap().take();
        if let Some(h) = follower.filter(|h| h.thread().id() != thread::current().id()) {
            let _ = h.join();
        }
        for h in self.workers.lock().unwrap().drain(..) {
            if h.thread().id() != thread::current().id() {
                let _ = h.join();
            }
        }
    }
}

/// Summaries and carried entries before a block.
type Before = (Summary, Summary, [Option<(u8, u64)>; 2], Option<usize>);
/// The level analysis so far, kept between passes so that a followed
/// file's new blocks extend it. Per-block data lives in `levels`.
#[derive(Default)]
struct Analysis {
    stats: Summary,
    prefix: Summary,
    levels: Arc<Vec<BlockLevels>>,
    carry: [Option<(u8, u64)>; 2],
    carry_file: Option<usize>,
    /// The state before the last block, which a replaced last row drops.
    before: Option<Before>,
}
impl Analysis {
    fn drop_last(&mut self) {
        match self.before.take() {
            Some((stats, prefix, carry, carry_file)) => {
                let n = self.levels.len() - 1;
                Arc::make_mut(&mut self.levels).truncate(n);
                (self.stats, self.prefix, self.carry, self.carry_file) =
                    (stats, prefix, carry, carry_file);
            }
            // Not expected: analyze again from the first block.
            None => *self = Self::default(),
        }
    }
}
#[derive(Clone, Default)]
struct Summary {
    total: u64,
    parsed: u64,
    blank: u64,
    counts: [u64; 8],
}
impl Summary {
    fn add(&mut self, r: &parser::Record) {
        self.total += 1;
        if let Some(p) = &r.parsed {
            self.parsed += 1;
            if let Some(i) = parser::LEVELS.iter().position(|l| *l == p.level) {
                self.counts[i] += 1;
            }
        } else if r.kind == "blank" {
            self.blank += 1;
        }
    }
    fn merge(&mut self, other: &Self) {
        self.total += other.total;
        self.parsed += other.parsed;
        self.blank += other.blank;
        for (a, b) in self.counts.iter_mut().zip(other.counts) {
            *a += b;
        }
    }
    fn json(&self) -> Value {
        let counts: serde_json::Map<String, Value> = parser::LEVELS
            .iter()
            .zip(self.counts)
            .map(|(l, n)| (l.to_string(), json!(n)))
            .collect();
        json!({"total":self.total,"parsed":self.parsed,"blank":self.blank,"raw":self.total-self.parsed-self.blank,"counts":counts})
    }
}
/// One block's rows by log entry level before leading rows are resolved;
/// one block's rows fit in u32. A malformed row that starts an entry opens
/// an orphan entry (slot 8).
#[derive(Default)]
struct Entries {
    /// Entry rows and the rows that continue them within the block.
    slots: [u32; 9],
    /// Rows without fields before the block's first entry row.
    lead: u32,
    /// Level slot and line of the last entry row.
    tail: Option<(u8, u64)>,
}
impl Entries {
    /// Adds a row; returns the slot of the entry it opens, if it opens one.
    fn add(&mut self, format: Format, r: &parser::Record, recognize: bool) -> Option<usize> {
        let level = match &r.parsed {
            Some(p) => parser::LEVELS.iter().position(|l| *l == p.level),
            None => parser::starts_entry_in(format, r.raw, recognize).then_some(8),
        };
        match (level, self.tail) {
            (Some(level), _) => {
                self.slots[level] += 1;
                self.tail = Some((level as u8, r.line));
            }
            (None, Some((level, _))) => self.slots[level as usize] += 1,
            (None, None) => self.lead += 1,
        }
        level
    }
}
