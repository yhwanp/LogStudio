#[cfg(unix)]
use std::os::unix::fs::{FileExt, MetadataExt};
#[cfg(windows)]
use std::os::windows::fs::{FileExt, OpenOptionsExt};
use std::{
    fs::{File, Metadata},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};
pub const MAX_FILE: u64 = 8 * 1024 * 1024 * 1024;
pub const MAX_LINE: usize = 1024 * 1024;
pub const BLOCK: usize = 128 * 1024;
/// A user-presentable policy message, distinguishable from incidental
/// serde/OS/internal errors that must not reach the webview verbatim.
#[derive(Debug)]
struct Policy(String);
impl std::fmt::Display for Policy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Policy {}
pub fn error(s: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, Policy(s.into()))
}
/// Policy messages pass through; any other failure becomes `fallback`.
pub fn public_message(e: &io::Error, fallback: &str) -> String {
    match e.get_ref().and_then(|r| r.downcast_ref::<Policy>()) {
        Some(policy) => policy.0.clone(),
        None => fallback.into(),
    }
}
/// Keep policy errors intact and expose OS failures without private paths.
fn open_error(e: io::Error) -> io::Error {
    let message = match e.kind() {
        io::ErrorKind::NotFound => "Selected file was not found; select it again",
        io::ErrorKind::PermissionDenied => "Read permission denied for selected file",
        io::ErrorKind::WouldBlock => "Selected file is busy; retry",
        _ => "Could not open selected file; check availability and retry",
    };
    io::Error::new(e.kind(), Policy(message.into()))
}
/// How a source's bytes become rows and which parser reads them. Rows of
/// one source share one format; a parser never guesses it from row text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Format {
    /// Physical CR/LF rows of the file's own bytes: Android and iOS logs.
    #[default]
    Text,
    /// COVESA DLT messages (`DLT\x01` storage headers), decoded into one
    /// text row each.
    Dlt,
}
impl Format {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Dlt => "dlt",
        }
    }
}
/// One granted file of a source. Files occupy consecutive ranges of the
/// source's byte space in the order they were selected; no row spans two.
pub struct Part {
    path: Option<PathBuf>,
    stamp: Metadata,
    pub offset: u64,
    pub size: u64,
    pub name: String,
    /// The file's format by its first bytes; None for an empty file, which
    /// joins any format.
    pub format: Option<Format>,
    /// A text file whose first `IOS_HEAD` bytes read as an iOS log
    /// (`parser::ios_log`): its rows get Mac OS Roman mojibake repaired.
    pub ios: bool,
}
/// Bytes from a text file's start that decide whether it is an iOS log.
const IOS_HEAD: usize = 64 * 1024;
pub struct Source {
    files: Mutex<Option<Vec<File>>>,
    parts: Vec<Part>,
    /// Size when granted; a followed file can grow (`current_size`).
    pub size: u64,
    pub name: String,
    /// Only a single granted text file can be followed.
    follow: Option<Mutex<Accepted>>,
    format: Format,
}
/// Bytes of a followed file's start and accepted end that growth must keep.
const WINDOW: u64 = 4096;
/// What a followable file has been accepted as: the length reads may reach,
/// its metadata then, and its first and last bytes.
struct Accepted {
    len: u64,
    stamp: Metadata,
    head: Vec<u8>,
    tail: Vec<u8>,
}
impl Accepted {
    fn read(f: &File, stamp: &Metadata) -> io::Result<Self> {
        let len = stamp.len();
        let w = WINDOW.min(len);
        let (mut head, mut tail) = (vec![0; w as usize], vec![0; w as usize]);
        read_exact_at(f, &mut head, 0)?;
        read_exact_at(f, &mut tail, len - w)?;
        Ok(Self {
            len,
            stamp: stamp.clone(),
            head,
            tail,
        })
    }
}
fn read_exact_at(f: &File, buf: &mut [u8], mut offset: u64) -> io::Result<()> {
    let mut at = 0;
    while at < buf.len() {
        #[cfg(unix)]
        let n = f.read_at(&mut buf[at..], offset)?;
        #[cfg(windows)]
        let n = f.seek_read(&mut buf[at..], offset)?;
        if n == 0 {
            return Err(error("Source truncated"));
        }
        at += n;
        offset += n as u64;
    }
    Ok(())
}
/// Files one source may combine; each keeps an open handle.
pub const MAX_FILES: usize = 64;
/// A granted file as it was when opened: native code may reopen it or name
/// saved work by it; the path never reaches the page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grant {
    pub path: PathBuf,
    pub size: u64,
    pub modified: Option<std::time::SystemTime>,
}
/// Whether two metadata describe one file, whatever its length and times.
/// Windows has no stable file ID in std; the creation time stands in, and
/// the path must also show at least the accepted length (see `unchanged`).
fn same_file(a: &Metadata, b: &Metadata) -> bool {
    #[cfg(unix)]
    {
        a.dev() == b.dev() && a.ino() == b.ino()
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        a.creation_time() == b.creation_time()
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (a, b);
        true
    }
}
fn same(a: &Metadata, b: &Metadata) -> bool {
    let basic = a.len() == b.len() && a.modified().ok() == b.modified().ok();
    #[cfg(unix)]
    {
        basic && a.dev() == b.dev() && a.ino() == b.ino()
    }
    #[cfg(not(unix))]
    {
        basic
    }
}
impl Source {
    /// True when `path` names this source's file (an export must never
    /// overwrite the log it reads).
    pub fn is_same_file(&self, path: &Path) -> bool {
        self.parts.iter().any(|part| part.is_same_file(path))
    }
    pub fn parts(&self) -> &[Part] {
        &self.parts
    }
    /// The format all of the source's rows are read in.
    pub fn format(&self) -> Format {
        self.format
    }
    /// The granted files in order; None for pasted text, which has no file.
    pub fn grants(&self) -> Option<Vec<Grant>> {
        self.parts
            .iter()
            .map(|p| {
                Some(Grant {
                    path: p.path.clone()?,
                    size: p.size,
                    modified: p.stamp.modified().ok(),
                })
            })
            .collect()
    }
    pub fn granted(path: &Path) -> io::Result<Self> {
        Self::granted_files(std::slice::from_ref(&path.to_path_buf()))
    }
    /// Combines granted files, in the given order, into one source.
    pub fn granted_files(paths: &[PathBuf]) -> io::Result<Self> {
        if paths.is_empty() {
            return Err(error("Select at least one log file"));
        }
        if paths.len() > MAX_FILES {
            return Err(error(format!("Select 1 to {MAX_FILES} log files")));
        }
        let (mut files, mut parts, mut size) = (vec![], vec![], 0u64);
        for path in paths {
            let (file, part) = Part::open(path, size)?;
            if parts.iter().any(|p: &Part| p.is_same_file(path)) {
                return Err(error("The same file was selected more than once"));
            }
            // Each file is bounded; the combined size is not.
            size += part.size;
            files.push(file);
            parts.push(part);
        }
        let name = match parts.len() {
            1 => parts[0].name.clone(),
            n => format!("{} 외 {}개", parts[0].name, n - 1),
        };
        let has = |f: Format| parts.iter().any(|p| p.format == Some(f));
        if has(Format::Dlt) && has(Format::Text) {
            return Err(error("DLT files and text logs cannot be opened together"));
        }
        let format = if has(Format::Dlt) {
            Format::Dlt
        } else {
            Format::Text
        };
        // A DLT file's rows are decoded messages; following one would need
        // messages to be decided as they arrive.
        let follow = match &files[..] {
            [file] if format == Format::Text => {
                Some(Mutex::new(Accepted::read(file, &parts[0].stamp)?))
            }
            _ => None,
        };
        Ok(Self {
            size,
            name,
            parts,
            files: Mutex::new(Some(files)),
            follow,
            format,
        })
    }
    pub fn text(text: &str, name: &str) -> io::Result<Self> {
        if text.len() > 25 * 1024 * 1024 {
            return Err(error("Pasted text exceeds 25 MiB"));
        }
        let mut file = tempfile::tempfile()?;
        file.write_all(text.as_bytes())?;
        let stamp = file.metadata().map_err(open_error)?;
        let name: String = name.chars().take(300).collect();
        Ok(Self {
            size: stamp.len(),
            parts: vec![Part {
                path: None,
                offset: 0,
                size: stamp.len(),
                name: name.clone(),
                stamp,
                format: Some(Format::Text),
                ios: crate::parser::ios_log(&text.as_bytes()[..text.len().min(IOS_HEAD)]),
            }],
            name,
            files: Mutex::new(Some(vec![file])),
            follow: None,
            format: Format::Text,
        })
    }
    pub fn followable(&self) -> bool {
        self.follow.is_some()
    }
    /// Bytes reads may reach: the granted size, or a followed file's
    /// accepted length.
    pub fn current_size(&self) -> u64 {
        self.follow
            .as_ref()
            .map_or(self.size, |a| a.lock().unwrap().len)
    }
    /// Size of file `k`; a followable source has one file.
    pub fn part_size(&self, k: usize) -> u64 {
        match &self.follow {
            Some(a) => a.lock().unwrap().len,
            None => self.parts[k].size,
        }
    }
    /// Whether a followable file is longer than its accepted length, so
    /// following would read more rows.
    pub fn grown(&self) -> bool {
        let Some(follow) = &self.follow else {
            return false;
        };
        let guard = self.files.lock().unwrap();
        let Some(files) = guard.as_ref() else {
            return false;
        };
        files[0]
            .metadata()
            .is_ok_and(|m| m.len() > follow.lock().unwrap().len)
    }
    /// Accepts bytes appended to a followed file and returns its new length,
    /// or None when it has not grown (see `appended`).
    pub fn grow(&self) -> io::Result<Option<u64>> {
        let follow = self
            .follow
            .as_ref()
            .ok_or_else(|| error("Only one opened log file can be followed"))?;
        let guard = self.files.lock().unwrap();
        let f = &guard.as_ref().ok_or_else(|| error("Source closed"))?[0];
        let m = f.metadata()?;
        let mut a = follow.lock().unwrap();
        self.appended(&mut a, f, &m, true)?;
        if m.len() == a.len {
            return Ok(None);
        }
        // The new end is what later growth must keep.
        let w = WINDOW.min(m.len());
        a.tail = vec![0; w as usize];
        read_exact_at(f, &mut a.tail, m.len() - w)?;
        if a.head.len() < w as usize {
            a.head = vec![0; w as usize];
            read_exact_at(f, &mut a.head, 0)?;
        }
        a.len = m.len();
        Ok(Some(a.len))
    }
    /// A single granted file may only grow by appended bytes, as a log being
    /// written does: the same file, at least as long, with its first and last
    /// accepted bytes unchanged. Reads still end at the accepted length until
    /// following accepts more. Anything else needs reopening; rewriting
    /// other bytes without changing those is not detected.
    fn appended(&self, a: &mut Accepted, f: &File, m: &Metadata, path: bool) -> io::Result<()> {
        if same_file(&a.stamp, m) && m.len() < a.len {
            return Err(error("Source truncated"));
        }
        let at_path = !path
            || self.parts[0].path.as_ref().is_none_or(|p| {
                std::fs::symlink_metadata(p)
                    .is_ok_and(|n| n.is_file() && same_file(&a.stamp, &n) && n.len() >= a.len)
            });
        if !same_file(&a.stamp, m) || !at_path {
            return Err(error("Source changed; reopen the file"));
        }
        if same(&a.stamp, m) {
            return Ok(());
        }
        let (mut head, mut tail) = (vec![0; a.head.len()], vec![0; a.tail.len()]);
        read_exact_at(f, &mut head, 0)?;
        read_exact_at(f, &mut tail, a.len - a.tail.len() as u64)?;
        if head != a.head || tail != a.tail {
            return Err(error("Source changed; reopen the file"));
        }
        a.stamp = m.clone();
        Ok(())
    }
    /// The file holding byte `at`; the end of the source maps to the last.
    fn part_at(&self, at: u64) -> usize {
        self.parts
            .partition_point(|p| p.offset + p.size <= at)
            .min(self.parts.len() - 1)
    }
    pub fn read(&self, start: u64, len: usize) -> io::Result<Vec<u8>> {
        let size = self.current_size();
        if len > MAX_LINE + 2 * BLOCK || start > size || len as u64 > size - start {
            return Err(error("Read budget exceeded"));
        }
        let guard = self.files.lock().unwrap();
        let files = guard.as_ref().ok_or_else(|| error("Source closed"))?;
        let first = self.part_at(start);
        self.check(first, &files[first], true)?;
        let mut out = vec![0; len];
        let (mut at, mut k) = (0, first);
        // Blocks never cross files, but a read may; each file is checked.
        while at < len {
            let (part, f) = (&self.parts[k], &files[k]);
            if k != first {
                self.check(k, f, true)?;
            }
            let local = start + at as u64 - part.offset;
            let end = at + (self.part_size(k) - local).min((len - at) as u64) as usize;
            read_exact_at(f, &mut out[at..end], local)?;
            at = end;
            self.check(k, f, false)?;
            k += 1;
        }
        Ok(out)
    }
    /// File `k`'s handle, and with `path` its path, must still show what was
    /// granted; a single file may also have grown (`appended`).
    fn check(&self, k: usize, f: &File, path: bool) -> io::Result<()> {
        let m = f.metadata()?;
        let stamp = &self.parts[k].stamp;
        let at_path = || {
            !path
                || self.parts[k].path.as_ref().is_none_or(|p| {
                    std::fs::symlink_metadata(p).is_ok_and(|n| n.is_file() && same(stamp, &n))
                })
        };
        match &self.follow {
            _ if same(stamp, &m) && at_path() => Ok(()),
            Some(follow) => self.appended(&mut follow.lock().unwrap(), f, &m, path),
            None => Err(error("Source changed; reopen the file")),
        }
    }
    /// Checks that no file changed since it was granted.
    pub fn verify(&self) -> io::Result<()> {
        let guard = self.files.lock().unwrap();
        let files = guard.as_ref().ok_or_else(|| error("Source closed"))?;
        files
            .iter()
            .enumerate()
            .try_for_each(|(k, f)| self.check(k, f, true))
    }
    pub fn close(&self) {
        self.files.lock().unwrap().take();
    }
    /// A reader that warms the OS file cache ahead of a pass; None for
    /// pasted text, which has no path and stays cached.
    pub fn read_ahead(&self) -> Option<ReadAhead<'_>> {
        self.parts
            .iter()
            .any(|p| p.path.is_some())
            .then(|| ReadAhead {
                source: self,
                file: None,
                buf: vec![0; BLOCK],
            })
    }
}
/// Reads bytes a pass is about to read, in order, on handles of its own: a
/// pass's workers take blocks in turn, which the OS does not read ahead as
/// it does one sequential stream (see docs/LARGE_LOG_VALIDATION.md). The
/// bytes are dropped; only the cache keeps them. A failed read is ignored,
/// as the pass's own reads check the files.
pub struct ReadAhead<'a> {
    source: &'a Source,
    /// The file being read and its handle; None when it cannot be reopened
    /// as the granted file.
    file: Option<(usize, Option<File>)>,
    buf: Vec<u8>,
}
impl ReadAhead<'_> {
    /// Reads source bytes `start..start + len` of file `k` and returns how
    /// many it read: fewer at the file's end, none when it cannot be read.
    pub fn read(&mut self, k: usize, start: u64, len: usize) -> u64 {
        if self.file.as_ref().is_none_or(|(open, _)| *open != k) {
            self.file = Some((k, self.source.parts[k].reopen()));
        }
        let Some((_, Some(f))) = &self.file else {
            return 0;
        };
        let begin = start - self.source.parts[k].offset;
        let (mut at, end) = (begin, begin + len as u64);
        while at < end {
            let n = (end - at).min(self.buf.len() as u64) as usize;
            #[cfg(unix)]
            let read = f.read_at(&mut self.buf[..n], at);
            #[cfg(windows)]
            let read = f.seek_read(&mut self.buf[..n], at);
            match read {
                Ok(0) | Err(_) => break,
                Ok(n) => at += n as u64,
            }
        }
        at - begin
    }
}
impl Part {
    fn open(path: &Path, offset: u64) -> io::Result<(File, Self)> {
        let before = std::fs::symlink_metadata(path).map_err(open_error)?;
        if !before.is_file() || before.file_type().is_symlink() {
            return Err(error(
                "Select a regular file; symbolic links are not supported",
            ));
        }
        #[cfg(not(windows))]
        let file = File::open(path).map_err(open_error)?;
        // Read, write and delete sharing lets a live log stay open in its
        // writer and be rotated; the app only reads, and checks changes.
        #[cfg(windows)]
        let file = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(1 | 2 | 4)
            .open(path)
            .map_err(open_error)?;
        let stamp = file.metadata().map_err(open_error)?;
        // A log being written may grow between the two looks.
        let opened = same_file(&before, &stamp) && stamp.len() >= before.len();
        if !stamp.is_file() || !opened || stamp.len() > MAX_FILE {
            return Err(error("File changed or exceeds 8 GiB"));
        }
        let mut head = vec![0; IOS_HEAD.min(stamp.len() as usize)];
        read_exact_at(&file, &mut head, 0).map_err(open_error)?;
        let format = match head.len() {
            0 => None,
            _ if crate::dlt::is_dlt(&head) => Some(Format::Dlt),
            _ => Some(Format::Text),
        };
        let ios = format == Some(Format::Text) && crate::parser::ios_log(&head);
        let part = Self {
            size: stamp.len(),
            offset,
            name: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .chars()
                .map(|c| if c.is_control() { '�' } else { c })
                .take(300)
                .collect(),
            stamp,
            path: Some(path.to_owned()),
            format,
            ios,
        };
        Ok((file, part))
    }
    /// Another handle on the granted file, for `ReadAhead`; None when its
    /// path no longer names that regular file.
    fn reopen(&self) -> Option<File> {
        let path = self.path.as_ref()?;
        // Checked first so that a path replaced by a FIFO is not opened.
        let at_path = std::fs::symlink_metadata(path).ok()?;
        if !at_path.is_file() || !same_file(&self.stamp, &at_path) {
            return None;
        }
        #[cfg(not(windows))]
        let file = File::open(path).ok()?;
        // Shared as `open` shares it; the flag lets the cache read further
        // ahead of this handle's sequential reads.
        #[cfg(windows)]
        let file = {
            const FILE_FLAG_SEQUENTIAL_SCAN: u32 = 0x0800_0000;
            std::fs::OpenOptions::new()
                .read(true)
                .share_mode(1 | 2 | 4)
                .custom_flags(FILE_FLAG_SEQUENTIAL_SCAN)
                .open(path)
                .ok()?
        };
        same_file(&self.stamp, &file.metadata().ok()?).then_some(file)
    }
    fn is_same_file(&self, path: &Path) -> bool {
        let Some(own) = &self.path else { return false };
        if let (Ok(a), Ok(b)) = (std::fs::canonicalize(own), std::fs::canonicalize(path)) {
            if a == b {
                return true;
            }
        }
        std::fs::metadata(path).is_ok_and(|m| {
            #[cfg(unix)]
            {
                m.dev() == self.stamp.dev() && m.ino() == self.stamp.ino()
            }
            #[cfg(not(unix))]
            {
                let _ = m;
                false
            }
        })
    }
}
/// Physical rows; CRLF is one delimiter. A trailing delimiter adds no row.
pub struct ByteLines<'a> {
    data: &'a [u8],
    at: usize,
}
impl<'a> ByteLines<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, at: 0 }
    }
}
impl<'a> Iterator for ByteLines<'a> {
    type Item = (usize, &'a [u8]);
    fn next(&mut self) -> Option<Self::Item> {
        if self.at == self.data.len() {
            return None;
        }
        let start = self.at;
        let end = memchr::memchr2(b'\r', b'\n', &self.data[start..])
            .map(|i| start + i)
            .unwrap_or(self.data.len());
        self.at = if end == self.data.len() {
            end
        } else if self.data[end] == b'\r' && self.data.get(end + 1) == Some(&b'\n') {
            end + 2
        } else {
            end + 1
        };
        Some((start, &self.data[start..end]))
    }
}
