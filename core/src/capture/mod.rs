//! Records a device's log into a capture file that a followed dataset
//! reads: an Android device's logcat, or an iPhone's, iPad's or iOS
//! simulator's log. Rows from before the capture started are left out. A device that leaves is
//! waited for; an Android device is continued after the last row recorded,
//! without repeating it, or from its first row when it booted meanwhile,
//! and rows it dropped from its buffer while away cannot come back. An
//! iPhone sends only new rows, so rows it logged while away are lost. A
//! capture can be cut: it records into a new file from then on, and the rows
//! before stay with the old one. A stopped capture can record on into the
//! same file from the moment it resumes; rows logged while stopped are not
//! recorded.
pub mod adb;
pub mod apps;
pub mod cli;
pub mod fake;
pub mod gate;
pub mod ios;
pub mod link;
mod process;
pub mod sim;
pub mod store;

use crate::{
    partial::Partial,
    source::{error, MAX_FILE, MAX_LINE},
};
use gate::Gate;
use link::{Cancel, Closer, Device, Link, Platform, Stream};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Condvar, Mutex,
    },
    thread::{self, JoinHandle},
    time::Duration,
};
use store::Slot;

pub const LIMIT: &str = "Recording stopped at 8 GiB";
pub const DISK_FULL: &str = "Recording stopped: not enough disk space";
pub const WRITE_FAILED: &str = "Recording stopped: the capture file could not be written";
pub const NOT_STOPPED: &str = "Only a stopped device capture can record on";
pub const NOTHING_TO_RESTORE: &str = "The cleared device log can no longer be restored";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Recording,
    /// The device left; the capture waits for it.
    Waiting,
    Stopped,
    Failed,
}
impl Phase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Recording => "recording",
            Self::Waiting => "waiting",
            Self::Stopped => "stopped",
            Self::Failed => "failed",
        }
    }
}
/// Where the capture lost and found the device again, or where the user
/// stopped it and it recorded on: `row` is the first physical row after it.
/// Times are Unix milliseconds of this computer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Break {
    pub row: u64,
    pub lost: i64,
    pub resumed: i64,
    pub reboot: bool,
    /// Rows the device logged while away were not recorded: its stream
    /// sends only new rows, as an iPhone's does, or the capture was stopped.
    pub gap: bool,
    /// The user stopped the capture at `lost` and resumed it.
    pub stopped: bool,
    /// The device, a simulator, was seen shut down or starting while away.
    pub shutdown: bool,
}
/// An app the capture's device started with its private values shown:
/// `row` is the first physical row recorded after the start began, `at` its
/// Unix milliseconds on this computer, `app` the name its home screen shows
/// and `process` the executable name its rows carry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Launch {
    pub row: u64,
    pub at: i64,
    pub app: String,
    pub process: String,
}
#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// How often a waiting capture looks for its device.
    pub poll: Duration,
    /// Device rows read to find where the capture starts.
    pub recent: usize,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            poll: Duration::from_secs(1),
            recent: 256,
        }
    }
}
struct Status {
    phase: Phase,
    /// The file the capture writes now.
    slot: Slot,
    /// Times the capture was cut into a new file.
    cuts: u64,
    /// When the capture was last cut, in Unix milliseconds of this
    /// computer: the page marks the new file's first row with it.
    cleared: Option<i64>,
    /// The capture was cut while its device was away, or it resumes after
    /// a stop: rows the device logged in the meantime are skipped, and
    /// recording starts from the device's latest row, as a new capture does.
    fresh: bool,
    /// When the capture last stopped, until it records again after a
    /// resume: its break starts there.
    stopped: Option<i64>,
    /// The device's boot when its stream last opened.
    boot: String,
    /// Another device with the same identity was seen.
    shared: bool,
    serial: String,
    bytes: u64,
    rows: u64,
    error: String,
    breaks: Vec<Break>,
    launches: Vec<Launch>,
    lost: Option<i64>,
    /// While waiting: why the device, listed again, cannot be used yet,
    /// such as USB debugging awaiting the user's approval.
    reason: String,
    /// The device was seen shut down or starting since it was last back.
    shutdown: bool,
    /// The rows a cut of the stopped capture set aside, to be restored
    /// until the capture records again, closes or lets them go.
    kept: Option<Kept>,
    /// The cut that is being undone: the rows it set aside are back, and
    /// its file waits until they opened.
    undone: Option<Undone>,
}
/// The cut a restore undoes, to be made again if the rows do not open.
struct Undone {
    slot: Slot,
    file: File,
    cleared: Option<i64>,
}
impl Undone {
    fn remove(self) {
        drop(self.file);
        let _ = self.slot.remove();
    }
}
/// What a capture had before a cut made while stopped.
struct Kept {
    slot: Slot,
    file: File,
    tail: Vec<u8>,
    bytes: u64,
    rows: u64,
    breaks: Vec<Break>,
    launches: Vec<Launch>,
    cleared: Option<i64>,
    saved: u64,
    discard: bool,
}
impl Kept {
    fn remove(self) {
        drop(self.file);
        let _ = self.slot.remove();
    }
}
type Observer = Arc<dyn Fn(&Capture) + Send + Sync>;
pub struct Capture {
    link: Arc<dyn Link>,
    /// The device as people know it: its model, else its serial.
    pub model: String,
    pub platform: Platform,
    /// An iOS simulator's capture.
    pub simulator: bool,
    identity: String,
    /// An emulator: only its own serial, the port it runs on, is it.
    pinned: bool,
    /// The device as it was listed when the capture started.
    device: Device,
    options: Options,
    status: Mutex<Status>,
    wake: Condvar,
    /// Set as the capture stops; also ends the link's running request.
    stop: Arc<Cancel>,
    closer: Mutex<Option<Closer>>,
    /// The thread that records. Held while a capture stops or resumes, so
    /// that the two do not cross.
    worker: Mutex<Option<JoinHandle<()>>>,
    /// What the worker writes with, which a cut changes. Locked before
    /// `status`. A capture left by an earlier run has none.
    writer: Option<Mutex<Writer>>,
    observer: Mutex<Option<Observer>>,
    /// Bytes the user saved elsewhere.
    saved: AtomicU64,
    /// Whether closing deletes the file: the user chose not to keep it.
    discard: AtomicBool,
    /// A file left by an earlier run, shown again.
    recovered: bool,
    /// The capture closed and never resumes.
    closed: AtomicBool,
}
fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
/// How long listing the devices may take after one left, before the
/// capture shows that it waits without saying why.
const LISTING: Duration = Duration::from_millis(500);
/// Bytes of written rows kept to continue after them.
const TAIL: usize = 64 * 1024;
/// Appends whole rows; the bytes after the last LF wait for the rest.
struct Writer {
    file: File,
    pending: Vec<u8>,
    /// `pending` belongs to a lost stream; the next one sends the row again.
    stale: bool,
    tail: Vec<u8>,
    bytes: u64,
    rows: u64,
}
/// Physical rows as `ByteLines` counts them in bytes ending with LF.
fn rows(bytes: &[u8]) -> u64 {
    let lf = memchr::memchr_iter(b'\n', bytes).count();
    let cr = memchr::memchr_iter(b'\r', bytes)
        .filter(|&i| bytes.get(i + 1) != Some(&b'\n'))
        .count();
    (lf + cr) as u64
}
impl Writer {
    fn push(&mut self, data: &[u8], gate: &mut Gate) -> io::Result<()> {
        if self.stale {
            self.pending.clear();
            self.stale = false;
        }
        self.pending.extend_from_slice(data);
        let end = match memchr::memrchr(b'\n', &self.pending) {
            Some(i) => i + 1,
            // A row longer than any row a log may have is written without
            // waiting for its end; reading the log then reports it.
            None if self.pending.len() > MAX_LINE + 1 => self.pending.len(),
            None => return Ok(()),
        };
        if gate.is_open() {
            let chunk = std::mem::take(&mut self.pending);
            self.write(&chunk[..end])?;
            self.pending = chunk[end..].to_vec();
            return Ok(());
        }
        let mut kept = Vec::with_capacity(end);
        let mut at = 0;
        while at < end {
            let next = memchr::memchr(b'\n', &self.pending[at..end]).map_or(end, |i| at + i + 1);
            if gate.keep(&self.pending[at..next]) {
                kept.extend_from_slice(&self.pending[at..next]);
            }
            at = next;
        }
        self.pending.drain(..end);
        self.write(&kept)
    }
    fn write(&mut self, out: &[u8]) -> io::Result<()> {
        if out.is_empty() {
            return Ok(());
        }
        if self.bytes + out.len() as u64 > MAX_FILE {
            return Err(error(LIMIT));
        }
        self.file.write_all(out).map_err(|e| {
            error(match e.kind() {
                io::ErrorKind::StorageFull => DISK_FULL,
                _ => WRITE_FAILED,
            })
        })?;
        self.bytes += out.len() as u64;
        self.rows += rows(out);
        self.tail.extend_from_slice(out);
        if self.tail.len() > TAIL {
            self.tail.drain(..self.tail.len() - TAIL);
        }
        Ok(())
    }
    /// The capture ends: a row still waiting for its end is kept as it is.
    fn finish(&mut self) -> io::Result<()> {
        let rest = std::mem::take(&mut self.pending);
        self.write(&rest)
    }
}
impl Capture {
    /// Starts recording `serial` into the slot's file from now on. Fails at
    /// once when the device cannot be used, and deletes the file then.
    pub fn start(
        link: Arc<dyn Link>,
        serial: &str,
        slot: Slot,
        options: Options,
    ) -> io::Result<Arc<Self>> {
        let stop = Arc::new(Cancel::default());
        let link = link.cancelled_by(stop.clone()).unwrap_or(link);
        let c = match connect(&*link, serial, &slot.path, &options) {
            Ok(c) => c,
            Err(e) => {
                let _ = slot.remove();
                return Err(e);
            }
        };
        let writer = Writer {
            file: c.file,
            pending: vec![],
            stale: false,
            tail: vec![],
            bytes: 0,
            rows: 0,
        };
        let capture = Arc::new(Self {
            link,
            model: c.device.label(),
            platform: c.device.platform,
            simulator: c.device.simulator,
            pinned: c.pinned,
            device: c.device,
            identity: c.identity,
            options,
            status: Mutex::new(Status {
                phase: Phase::Recording,
                slot,
                cuts: 0,
                cleared: None,
                fresh: false,
                stopped: None,
                boot: c.boot,
                shared: c.shared,
                serial: serial.into(),
                bytes: 0,
                rows: 0,
                error: String::new(),
                breaks: vec![],
                launches: vec![],
                lost: None,
                reason: String::new(),
                shutdown: false,
                kept: None,
                undone: None,
            }),
            wake: Condvar::new(),
            stop,
            closer: Mutex::new(None),
            worker: Mutex::new(None),
            writer: Some(Mutex::new(writer)),
            observer: Mutex::new(None),
            saved: AtomicU64::new(0),
            discard: AtomicBool::new(false),
            recovered: false,
            closed: AtomicBool::new(false),
        });
        let (stream, gate) = (c.stream, c.gate);
        let worker = capture.clone();
        *capture.worker.lock().unwrap() =
            Some(thread::spawn(move || worker.run(Some((stream, gate)))));
        Ok(capture)
    }
    /// A capture file left by an earlier run, held again to be shown,
    /// saved or deleted as a stopped capture.
    pub fn adopt(slot: Slot) -> io::Result<Arc<Self>> {
        let bytes = std::fs::metadata(&slot.path)?.len();
        let name = slot
            .path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let platform = if name.starts_with("ios-") {
            Platform::Ios
        } else {
            Platform::Android
        };
        Ok(Arc::new(Self {
            link: Arc::new(Unlinked),
            model: name,
            platform,
            simulator: false,
            identity: String::new(),
            pinned: false,
            device: Device::default(),
            options: Options::default(),
            status: Mutex::new(Status {
                phase: Phase::Stopped,
                slot,
                cuts: 0,
                cleared: None,
                fresh: false,
                stopped: None,
                boot: String::new(),
                shared: false,
                serial: String::new(),
                bytes,
                rows: 0,
                error: String::new(),
                breaks: vec![],
                launches: vec![],
                lost: None,
                reason: String::new(),
                shutdown: false,
                kept: None,
                undone: None,
            }),
            wake: Condvar::new(),
            stop: Arc::default(),
            closer: Mutex::new(None),
            worker: Mutex::new(None),
            writer: None,
            observer: Mutex::new(None),
            saved: AtomicU64::new(0),
            discard: AtomicBool::new(false),
            recovered: true,
            closed: AtomicBool::new(false),
        }))
    }
    /// The file the capture writes now.
    pub fn path(&self) -> PathBuf {
        self.status.lock().unwrap().slot.path.clone()
    }
    /// Whether rows were recorded that no save holds.
    pub fn unsaved(&self) -> bool {
        self.bytes() > self.saved.load(Ordering::Acquire)
    }
    /// Copies the rows recorded so far to `to` through a temporary file in
    /// its folder, as exports do; recording goes on. Returns the bytes.
    pub fn save(&self, to: &Path, cancel: &AtomicBool) -> io::Result<u64> {
        let (path, len, cuts) = {
            let s = self.status.lock().unwrap();
            (s.slot.path.clone(), s.bytes, s.cuts)
        };
        if to == path || std::fs::canonicalize(to).ok() == std::fs::canonicalize(&path).ok() {
            return Err(error("Choose a file other than the capture"));
        }
        if to.file_name().is_none() {
            return Err(error("Save failed"));
        }
        if let Err(e) = copy_start(&path, len, to, cancel) {
            return Err(match crate::source::public_message(&e, "") {
                m if !m.is_empty() => e,
                _ => error(match e.kind() {
                    io::ErrorKind::PermissionDenied => "Write permission denied for the file",
                    io::ErrorKind::StorageFull => "Not enough disk space to save",
                    _ => "Save failed",
                }),
            });
        }
        // A save of the previous file must not mark the new file's rows as
        // saved.
        let s = self.status.lock().unwrap();
        if s.cuts == cuts {
            self.saved.fetch_max(len, Ordering::AcqRel);
        }
        Ok(len)
    }
    /// Records into `cut`'s file from now on, as a capture started now
    /// would: the rows so far stay in the file it wrote, whose slot it
    /// returns to be deleted once nothing reads it, and the breaks and app
    /// starts are cleared. A row being received goes to the new file. A
    /// stopped capture stays stopped, and records into the new file once it
    /// resumes; its rows so far are set aside for `restore` instead, and no
    /// file is returned. A capture left by an earlier run writes nothing and
    /// returns `cut`'s slot unused.
    pub fn cut(&self, cut: Cut) -> Option<Slot> {
        let Some(writer) = &self.writer else {
            return Some(cut.slot);
        };
        let old = {
            let mut w = writer.lock().unwrap();
            let mut s = self.status.lock().unwrap();
            let file = std::mem::replace(&mut w.file, cut.file);
            let slot = std::mem::replace(&mut s.slot, cut.slot);
            let undone = s.undone.take();
            // Keep the first set of cleared rows: clearing again while still
            // stopped and empty discards only the empty file.
            let old = if s.phase == Phase::Stopped && s.kept.is_none() {
                s.kept = Some(Kept {
                    slot,
                    file,
                    tail: std::mem::take(&mut w.tail),
                    bytes: w.bytes,
                    rows: w.rows,
                    breaks: std::mem::take(&mut s.breaks),
                    launches: std::mem::take(&mut s.launches),
                    cleared: s.cleared,
                    saved: self.saved.load(Ordering::Acquire),
                    discard: self.discard.load(Ordering::Acquire),
                });
                // Cleared again while a restore was pending: that restore's
                // empty file is deleted.
                undone.map(|u| u.slot)
            } else {
                Some(slot)
            };
            (w.bytes, w.rows) = (0, 0);
            (s.bytes, s.rows) = (0, 0);
            s.breaks.clear();
            s.launches.clear();
            s.cuts += 1;
            s.cleared = Some(now());
            s.fresh |= s.phase == Phase::Waiting;
            self.saved.store(0, Ordering::Release);
            self.discard.store(false, Ordering::Release);
            old
        };
        self.notify();
        old
    }
    /// The file of the rows a cut of the stopped capture set aside.
    pub fn kept_path(&self) -> Option<PathBuf> {
        let s = self.status.lock().unwrap();
        s.kept.as_ref().map(|k| k.slot.path.clone())
    }
    /// Takes back the rows a cut of the stopped capture set aside: the
    /// capture writes their file again, with their breaks, app starts and
    /// saves. Returns that file to be opened; call `confirm_restore` once it
    /// opens, or `revert_restore` if it does not. Until then `forget_kept`
    /// cannot delete the rows, and closing the capture keeps unsaved ones.
    /// Fails while a restore is pending, and once the capture recorded again
    /// or let the rows go.
    pub fn restore(&self) -> io::Result<PathBuf> {
        let nothing = || error(NOTHING_TO_RESTORE);
        let mut w = self.writer.as_ref().ok_or_else(nothing)?.lock().unwrap();
        let mut s = self.status.lock().unwrap();
        if s.phase != Phase::Stopped || s.bytes > 0 || s.undone.is_some() {
            return Err(nothing());
        }
        let k = s.kept.take().ok_or_else(nothing)?;
        let file = std::mem::replace(&mut w.file, k.file);
        let slot = std::mem::replace(&mut s.slot, k.slot);
        let cleared = std::mem::replace(&mut s.cleared, k.cleared);
        s.undone = Some(Undone {
            slot,
            file,
            cleared,
        });
        (w.tail, w.bytes, w.rows) = (k.tail, k.bytes, k.rows);
        (s.bytes, s.rows) = (k.bytes, k.rows);
        (s.breaks, s.launches) = (k.breaks, k.launches);
        // A save of the cut file must not count for these rows.
        s.cuts += 1;
        self.saved.store(k.saved, Ordering::Release);
        self.discard.store(k.discard, Ordering::Release);
        Ok(s.slot.path.clone())
    }
    /// The rows a restore took back opened. Returns the file the undone cut
    /// made, to be deleted once nothing reads it.
    pub fn confirm_restore(&self) -> Option<Slot> {
        let undone = self.status.lock().unwrap().undone.take();
        undone.map(|u| u.slot)
    }
    /// The rows a restore took back did not open: they are set aside again,
    /// as after the cut.
    pub fn revert_restore(&self) {
        let Some(writer) = &self.writer else { return };
        {
            let mut w = writer.lock().unwrap();
            let mut s = self.status.lock().unwrap();
            let Some(u) = s.undone.take() else { return };
            s.kept = Some(Kept {
                file: std::mem::replace(&mut w.file, u.file),
                slot: std::mem::replace(&mut s.slot, u.slot),
                cleared: std::mem::replace(&mut s.cleared, u.cleared),
                tail: std::mem::take(&mut w.tail),
                bytes: w.bytes,
                rows: w.rows,
                breaks: std::mem::take(&mut s.breaks),
                launches: std::mem::take(&mut s.launches),
                saved: self.saved.swap(0, Ordering::AcqRel),
                discard: false,
            });
            self.discard.store(false, Ordering::Release);
            (w.bytes, w.rows) = (0, 0);
            (s.bytes, s.rows) = (0, 0);
            s.cuts += 1;
        }
        self.notify();
    }
    /// Deletes the rows a cut of the stopped capture set aside.
    pub fn forget_kept(&self) {
        let kept = self.status.lock().unwrap().kept.take();
        if let Some(k) = kept {
            k.remove();
        }
    }
    /// Marks the file to be deleted when the capture closes, or kept.
    pub fn discard(&self, on: bool) {
        self.discard.store(on, Ordering::Release);
    }
    /// Stops the capture for good and deletes its file when the user chose
    /// not to keep it, or when no row would be lost: all were saved, or none
    /// came.
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
        self.stop();
        self.forget_kept();
        let undone = self.status.lock().unwrap().undone.take();
        if let Some(u) = undone {
            u.remove();
        }
        if self.discard.load(Ordering::Acquire) || !self.unsaved() {
            let _ = self.status.lock().unwrap().slot.remove();
        }
    }
    /// Called when the phase, a waiting capture's reason or the launches
    /// change, never with a lock held.
    pub fn observe(&self, f: impl Fn(&Capture) + Send + Sync + 'static) {
        *self.observer.lock().unwrap() = Some(Arc::new(f));
    }
    pub fn phase(&self) -> Phase {
        self.status.lock().unwrap().phase
    }
    /// Whether rows may still arrive.
    pub fn live(&self) -> bool {
        matches!(self.phase(), Phase::Recording | Phase::Waiting)
    }
    /// Whether a clear may cut it: it records, or the user stopped it and
    /// it can record on. One that failed or that an earlier run left cannot.
    pub fn clearable(&self) -> bool {
        self.writer.is_some()
            && !self.closed.load(Ordering::Acquire)
            && self.phase() != Phase::Failed
    }
    pub fn bytes(&self) -> u64 {
        self.status.lock().unwrap().bytes
    }
    /// The serial the device is recorded under now.
    pub fn serial(&self) -> String {
        self.status.lock().unwrap().serial.clone()
    }
    /// The physical rows written so far.
    pub fn rows(&self) -> u64 {
        self.status.lock().unwrap().rows
    }
    /// The row written next, with the cuts so far, for `mark_launch`.
    pub fn next_row(&self) -> (u64, u64) {
        let s = self.status.lock().unwrap();
        (s.rows + 1, s.cuts)
    }
    /// Remembers an app started on the device, for the page to mark where
    /// its rows begin; the capture file stays as the device sent it.
    /// `cuts` is from `next_row` with `launch.row`: a start that began
    /// before a cut marks the first row of the new file.
    pub fn mark_launch(&self, mut launch: Launch, cuts: u64) {
        {
            let mut s = self.status.lock().unwrap();
            if s.cuts != cuts {
                launch.row = 1;
            }
            s.launches.push(launch);
        }
        self.notify();
    }
    pub fn launches(&self) -> Vec<Launch> {
        self.status.lock().unwrap().launches.clone()
    }
    pub fn breaks(&self) -> Vec<Break> {
        self.status.lock().unwrap().breaks.clone()
    }
    pub fn json(&self) -> Value {
        let s = self.status.lock().unwrap();
        let breaks: Vec<Value> = s
            .breaks
            .iter()
            .map(|b| {
                json!({"row":b.row,"lost":b.lost,"resumed":b.resumed,"reboot":b.reboot,"gap":b.gap,"stopped":b.stopped,"shutdown":b.shutdown})
            })
            .collect();
        let launches: Vec<Value> = s
            .launches
            .iter()
            .map(|l| json!({"row":l.row,"at":l.at,"app":l.app,"process":l.process}))
            .collect();
        let saved = self.saved.load(Ordering::Acquire);
        json!({
            "state": s.phase.as_str(),
            "device": self.model,
            "platform": self.platform.as_str(),
            "simulator": self.simulator,
            "serial": s.serial,
            "bytes": s.bytes,
            "rows": s.rows,
            "error": s.error,
            "breaks": breaks,
            "launches": launches,
            "lost": s.lost,
            "stopped": s.stopped,
            "cleared": s.cleared,
            "reason": s.reason,
            "saved": saved,
            "unsaved": s.bytes > saved,
            "recovered": self.recovered,
        })
    }
    /// Ends the capture and waits until the file has every row received;
    /// it keeps them.
    pub fn stop(&self) {
        let worker = {
            // A resume holds the lock while it looks for the device, which
            // this ends.
            let _waiting = self.stop.wait();
            let mut worker = self.worker.lock().unwrap();
            self.stop.set_stopped(true);
            let closer = self.closer.lock().unwrap().take();
            if let Some(close) = closer {
                close();
            }
            {
                let _s = self.status.lock().unwrap();
                self.wake.notify_all();
            }
            worker.take()
        };
        if let Some(h) = worker.filter(|h| h.thread().id() != thread::current().id()) {
            let _ = h.join();
        }
    }
    /// Records again into the file of a capture the user stopped, as a
    /// capture started now would: after the rows the device holds now, with
    /// a break where it stopped. A device not ready yet is waited for. A row
    /// that stopping cut short ends where it was cut. A capture left by an
    /// earlier run, one closing, or one not stopped, fails, as does a resume
    /// stopped before it found the device.
    pub fn resume(self: &Arc<Self>) -> io::Result<()> {
        {
            let mut worker = self.worker.lock().unwrap();
            if self.writer.is_none()
                || self.closed.load(Ordering::Acquire)
                || self.phase() != Phase::Stopped
            {
                return Err(error(NOT_STOPPED));
            }
            // A thread still held has ended, as the phase says.
            if let Some(h) = worker.take() {
                let _ = h.join();
            }
            {
                let mut w = self.writer();
                if w.bytes > 0 && !matches!(w.tail.last(), Some(b'\n' | b'\r')) {
                    w.write(b"\n")?;
                    let mut s = self.status.lock().unwrap();
                    (s.bytes, s.rows) = (w.bytes, w.rows);
                }
            }
            // Recording on keeps the clear: from here its rows cannot be
            // taken back. One being undone is not recorded over.
            let kept = {
                let mut s = self.status.lock().unwrap();
                if s.undone.is_some() {
                    return Err(error(NOT_STOPPED));
                }
                s.fresh = true;
                s.kept.take()
            };
            self.stop.set_stopped(false);
            // A device found ready records at once; the page then never
            // shows the capture waiting for it.
            let found = self.attempt(&mut HashMap::new());
            let back = match found {
                Ok(back) if !self.stopping() => back,
                found => {
                    self.stop.set_stopped(true);
                    self.status.lock().unwrap().kept = kept;
                    return Err(found.err().unwrap_or_else(|| error(NOT_STOPPED)));
                }
            };
            if let Some(k) = kept {
                k.remove();
            }
            let stream = match back {
                Some(back) => Some(self.returned(back)),
                None => {
                    self.away(now());
                    None
                }
            };
            let capture = self.clone();
            *worker = Some(thread::spawn(move || capture.run(stream)));
        }
        self.notify();
        Ok(())
    }
    fn stopping(&self) -> bool {
        self.stop.is_set()
    }
    fn notify(&self) {
        let observer = self.observer.lock().unwrap().clone();
        if let Some(f) = observer {
            f(self);
        }
    }
    /// The writer of a started capture, the only kind that runs or is cut.
    fn writer(&self) -> std::sync::MutexGuard<'_, Writer> {
        self.writer
            .as_ref()
            .expect("a started capture has a writer")
            .lock()
            .unwrap()
    }
    /// Records the device's streams until stopped, starting with `stream`,
    /// or with waiting for the device when there is none.
    fn run(&self, mut stream: Option<(Box<dyn Stream>, Gate)>) {
        let result = loop {
            if let Some((s, gate)) = &mut stream {
                if let Err(e) = self.pump(&mut **s, gate) {
                    break Err(e);
                }
                if self.stopping() {
                    break Ok(());
                }
                self.leave();
            }
            match self.reconnect() {
                Ok(Some(back)) => stream = Some(self.returned(back)),
                Ok(None) => break Ok(()),
                Err(e) => break Err(e),
            }
            self.notify();
        };
        {
            let mut w = self.writer();
            let result = result.and_then(|()| w.finish());
            let mut s = self.status.lock().unwrap();
            (s.bytes, s.rows, s.lost) = (w.bytes, w.rows, None);
            match result {
                Ok(()) => {
                    s.phase = Phase::Stopped;
                    s.stopped.get_or_insert_with(now);
                }
                Err(e) => {
                    s.phase = Phase::Failed;
                    s.error = crate::source::public_message(&e, WRITE_FAILED);
                }
            }
        }
        self.notify();
    }
    /// The stream ended: the capture waits. The reason the device left, such
    /// as a simulator shut down, shows at once, not a poll later; when the
    /// listing hangs, the capture is shown waiting without it.
    fn leave(&self) {
        let lost = now();
        thread::scope(|scope| {
            let (answer, listing) = mpsc::channel();
            scope.spawn(move || {
                let _ = answer.send(self.link.devices().unwrap_or_default());
            });
            let listed = listing.recv_timeout(LISTING);
            self.away(lost);
            let devices = listed.unwrap_or_else(|_| {
                self.notify();
                listing.recv().unwrap_or_default()
            });
            self.explain(&devices);
        });
        self.notify();
    }
    /// The device left, or a resume did not find it: the capture waits.
    fn away(&self, lost: i64) {
        self.writer().stale = true;
        let mut s = self.status.lock().unwrap();
        s.phase = Phase::Waiting;
        s.lost = Some(lost);
    }
    /// Records the device that came back: its break, and its stream.
    fn returned(&self, back: Back) -> (Box<dyn Stream>, Gate) {
        let w = self.writer();
        let mut s = self.status.lock().unwrap();
        let stopped = s.stopped.take();
        let gone = Break {
            row: w.rows + 1,
            lost: stopped.or(s.lost).unwrap_or_else(now),
            resumed: now(),
            reboot: back.boot != s.boot,
            gap: back.gap,
            stopped: stopped.is_some(),
            shutdown: s.shutdown,
        };
        s.breaks.push(gone);
        s.boot = back.boot;
        s.phase = Phase::Recording;
        s.lost = None;
        s.reason.clear();
        s.shutdown = false;
        if back.fresh {
            s.fresh = false;
        }
        (back.stream, back.gate)
    }
    /// Writes a stream's rows until it ends; fails only when writing does.
    fn pump(&self, stream: &mut dyn Stream, gate: &mut Gate) -> io::Result<()> {
        let close = stream.closer();
        *self.closer.lock().unwrap() = Some(close.clone());
        // A stop before the closer was published still ends the stream.
        if self.stopping() {
            close();
        }
        let mut buf = vec![0; 64 * 1024];
        loop {
            let n = match stream.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            let mut w = self.writer();
            w.push(&buf[..n], gate)?;
            let mut s = self.status.lock().unwrap();
            (s.bytes, s.rows) = (w.bytes, w.rows);
        }
        self.closer.lock().unwrap().take();
        Ok(())
    }
    /// Waits until the device is back and continues: after the rows the
    /// capture has, or from its first row after a boot. None once stopped.
    fn reconnect(&self) -> io::Result<Option<Back>> {
        let mut others = HashMap::new();
        loop {
            {
                let s = self.status.lock().unwrap();
                if !self.stopping() {
                    let _ = self.wake.wait_timeout(s, self.options.poll).unwrap();
                }
            }
            if self.stopping() {
                return Ok(None);
            }
            if let Some(back) = self.attempt(&mut others)? {
                return Ok(Some(back));
            }
        }
    }
    /// Looks once for the device to continue with; None while it is not
    /// ready. Fails only when another adb server holds the devices.
    fn attempt(&self, others: &mut HashMap<String, String>) -> io::Result<Option<Back>> {
        let conflict = |e: &io::Error| crate::source::public_message(e, "") == link::CONFLICT;
        let devices = match self.link.devices() {
            Ok(d) => d,
            Err(e) if conflict(&e) => return Err(e),
            Err(_) => return Ok(None),
        };
        self.explain(&devices);
        let Some((serial, boot)) = self.find(&devices, others) else {
            return Ok(None);
        };
        match self.reopen(&serial, boot) {
            Ok(back) => {
                self.status.lock().unwrap().serial = serial;
                Ok(Some(back))
            }
            Err(e) if conflict(&e) => Err(e),
            Err(_) => Ok(None),
        }
    }
    /// Opens the device's stream again: after the rows written when it
    /// still has the same boot, else from the first row of its new boot.
    /// A capture cut while away, or resumed after a stop, starts after the
    /// rows the device holds now. `found` is the boot the device was chosen
    /// for, if it was asked.
    fn reopen(&self, serial: &str, found: Option<String>) -> io::Result<Back> {
        let now_boot = match found {
            Some(boot) => boot,
            None => self.link.boot_id(serial)?,
        };
        let (fresh, boot) = {
            let s = self.status.lock().unwrap();
            (s.fresh, s.boot.clone())
        };
        let gate = if !self.link.replays() {
            Gate::open()
        } else if fresh {
            Gate::after(&self.link.recent(serial, self.options.recent)?)
        } else if now_boot == boot {
            Gate::after(&self.writer().tail)
        } else {
            Gate::open()
        };
        let stream = self.link.stream(serial, gate.since())?;
        // Another device may have taken the serial since it was found. One
        // of the same identity and another boot shares it: no device boots
        // within an attempt.
        if !self.identity.is_empty() && self.link.identity(serial)? != self.identity {
            return Err(error(link::GONE));
        }
        if self.link.boot_id(serial)? != now_boot {
            self.status.lock().unwrap().shared = true;
            return Err(error(link::GONE));
        }
        Ok(Back {
            stream,
            gate,
            boot: now_boot,
            gap: !self.link.replays() || fresh,
            fresh,
        })
    }
    /// Says why the device, listed under its serial, cannot be used yet.
    fn explain(&self, devices: &[Device]) {
        let changed = {
            let mut s = self.status.lock().unwrap();
            let reason = devices
                .iter()
                .find(|d| d.serial == s.serial)
                .and_then(Device::problem)
                .unwrap_or("");
            let changed = s.reason != reason;
            s.shutdown |= matches!(reason, link::SHUTDOWN | link::BOOTING);
            s.reason = reason.into();
            changed
        };
        if changed {
            self.notify();
        }
    }
    /// Returns the serial the device is ready under, with the boot it was
    /// chosen for when that was asked; None to wait. Identities are asked
    /// again at every attempt, except of the serials in `others`, which
    /// named another device: those are asked again only before a match is
    /// taken. An emulator keeps its serial. When several serials have the
    /// identity, the one with the recorded boot is the device. With none, the
    /// device rebooted: it is the serial it had, or else the only match,
    /// unless devices were seen to share the identity. While a match does
    /// not answer, none is taken.
    fn find(
        &self,
        devices: &[Device],
        others: &mut HashMap<String, String>,
    ) -> Option<(String, Option<String>)> {
        let (current, boot, shared) = {
            let s = self.status.lock().unwrap();
            (s.serial.clone(), s.boot.clone(), s.shared)
        };
        let ready: Vec<&str> = devices
            .iter()
            .filter(|d| d.ready())
            .filter(|d| d.serial == current || (!self.pinned && self.device.alike(d)))
            .map(|d| d.serial.as_str())
            .collect();
        if self.identity.is_empty() {
            let here = ready.contains(&current.as_str());
            return here.then_some((current, None));
        }
        others.retain(|s, _| *s != current && ready.contains(&s.as_str()));
        let (skipped, asked): (Vec<&str>, Vec<&str>) =
            ready.into_iter().partition(|s| others.contains_key(*s));
        let (mut same, mut silent) = (vec![], false);
        for serials in [asked, skipped] {
            for serial in serials {
                match self.link.identity(serial) {
                    Ok(id) if id == self.identity => {
                        others.remove(serial);
                        same.push(serial);
                    }
                    Ok(id) => {
                        others.insert(serial.into(), id);
                    }
                    Err(_) => silent = true,
                }
            }
            if same.is_empty() {
                return None;
            }
        }
        same.sort();
        let here = same.contains(&current.as_str());
        if here && same.len() == 1 && !shared {
            return Some((current, None));
        }
        let boots: Vec<(&str, String)> = same
            .iter()
            .filter_map(|s| Some((*s, self.link.boot_id(s).ok()?)))
            .collect();
        let shared = {
            let mut s = self.status.lock().unwrap();
            s.shared |= boots.windows(2).any(|b| b[0].1 != b[1].1);
            s.shared
        };
        let recorded = || boots.iter().filter(|(_, b)| *b == boot);
        let mut chosen = recorded().find(|(s, _)| *s == current);
        chosen = chosen.or_else(|| recorded().next());
        if chosen.is_none() && (boots.len() < same.len() || (silent && shared)) {
            return None;
        }
        if chosen.is_none() && here {
            chosen = boots.iter().find(|(s, _)| *s == current);
        }
        chosen
            .or_else(|| boots.first().filter(|_| !shared))
            .map(|(s, b)| (s.to_string(), Some(b.clone())))
    }
}
/// The link of an adopted capture, which never connects.
struct Unlinked;
impl Link for Unlinked {
    fn devices(&self) -> io::Result<Vec<Device>> {
        Ok(vec![])
    }
    fn boot_id(&self, _: &str) -> io::Result<String> {
        Err(error(link::GONE))
    }
    fn identity(&self, _: &str) -> io::Result<String> {
        Err(error(link::GONE))
    }
    fn recent(&self, _: &str, _: usize) -> io::Result<Vec<u8>> {
        Err(error(link::GONE))
    }
    fn stream(&self, _: &str, _: Option<gate::Stamp>) -> io::Result<Box<dyn Stream>> {
        Err(error(link::GONE))
    }
}
/// A device found ready to record, with what recording it needs.
struct Connected {
    device: Device,
    identity: String,
    boot: String,
    pinned: bool,
    /// Another device connected now has the same identity.
    shared: bool,
    gate: Gate,
    file: File,
    stream: Box<dyn Stream>,
}
/// Checks the device, finds where its rows start from now on, and opens
/// its stream and the capture file.
fn connect(link: &dyn Link, serial: &str, path: &Path, options: &Options) -> io::Result<Connected> {
    let devices = link.devices()?;
    let device = devices
        .iter()
        .find(|d| d.serial == serial)
        .ok_or_else(|| error(link::GONE))?
        .clone();
    if let Some(problem) = device.problem() {
        return Err(error(problem));
    }
    let identity = link.identity(serial)?;
    let boot = link.boot_id(serial)?;
    let pinned = serial.starts_with("emulator-") || identity.starts_with(link::EMULATOR);
    // Only devices alike are asked, as each takes a moment.
    let shared = !pinned
        && !identity.is_empty()
        && devices
            .iter()
            .filter(|d| d.serial != serial && d.ready() && device.alike(d))
            .filter(|d| link.identity(&d.serial).is_ok_and(|id| id == identity))
            .any(|d| link.boot_id(&d.serial).is_ok_and(|b| b != boot));
    let gate = if link.replays() {
        Gate::after(&link.recent(serial, options.recent)?)
    } else {
        Gate::open()
    };
    let file = OpenOptions::new().append(true).open(path)?;
    let stream = link.stream(serial, gate.since())?;
    Ok(Connected {
        device,
        identity,
        boot,
        pinned,
        shared,
        gate,
        file,
        stream,
    })
}
/// A device back after it left: its new stream, where rows start in it,
/// and the boot it has now.
struct Back {
    stream: Box<dyn Stream>,
    gate: Gate,
    boot: String,
    /// Rows the device logged while away were left out.
    gap: bool,
    /// It started anew, the capture having been cut while away.
    fresh: bool,
}
/// A new file for a capture to record into from a cut on (`Capture::cut`),
/// opened beforehand so that the cut cannot fail.
pub struct Cut {
    slot: Slot,
    file: File,
}
impl Cut {
    /// Deletes `slot`'s file when it cannot be opened.
    pub fn open(slot: Slot) -> io::Result<Self> {
        match OpenOptions::new().append(true).open(&slot.path) {
            Ok(file) => Ok(Self { slot, file }),
            Err(e) => {
                let _ = slot.remove();
                Err(e)
            }
        }
    }
    pub fn path(&self) -> &Path {
        &self.slot.path
    }
    /// Deletes the file of a cut not made.
    pub fn remove(self) {
        let _ = self.slot.remove();
    }
}
/// Copies the first `len` bytes of `from` to `to`, which changes only once
/// the copy is complete.
fn copy_start(from: &Path, len: u64, to: &Path, cancel: &AtomicBool) -> io::Result<()> {
    let mut from = std::io::Read::take(File::open(from)?, len);
    let partial = Partial::beside(to)?;
    let mut buf = vec![0; 1 << 20];
    loop {
        if cancel.load(Ordering::Acquire) {
            return Err(error("Save cancelled"));
        }
        let n = std::io::Read::read(&mut from, &mut buf)?;
        if n == 0 {
            break;
        }
        partial.file().write_all(&buf[..n])?;
    }
    partial.replace(to, cancel, "Save cancelled")
}

#[cfg(test)]
mod tests {
    //! Moments of stopping and recording on that only a held lock can reach.
    use super::*;
    use std::time::Instant;

    /// Devices that listing hangs on, once told to, until it is cancelled.
    #[derive(Clone)]
    struct Hanging {
        world: Arc<fake::FakeLink>,
        hang: Arc<AtomicBool>,
        cancel: Arc<Cancel>,
    }
    impl Link for Hanging {
        fn cancelled_by(&self, cancel: Arc<Cancel>) -> Option<Arc<dyn Link>> {
            Some(Arc::new(Self {
                cancel,
                ..self.clone()
            }))
        }
        fn devices(&self) -> io::Result<Vec<Device>> {
            let until = Instant::now() + Duration::from_secs(10);
            while self.hang.load(Ordering::Acquire) && Instant::now() < until {
                if self.cancel.is_set() {
                    return Err(io::ErrorKind::Interrupted.into());
                }
                thread::sleep(Duration::from_millis(1));
            }
            self.world.devices()
        }
        fn boot_id(&self, serial: &str) -> io::Result<String> {
            self.world.boot_id(serial)
        }
        fn identity(&self, serial: &str) -> io::Result<String> {
            self.world.identity(serial)
        }
        fn recent(&self, serial: &str, count: usize) -> io::Result<Vec<u8>> {
            self.world.recent(serial, count)
        }
        fn stream(&self, serial: &str, since: Option<gate::Stamp>) -> io::Result<Box<dyn Stream>> {
            self.world.stream(serial, since)
        }
    }
    #[test]
    fn a_stop_before_a_resume_looks_for_the_device_is_not_lost() {
        let dir = tempfile::tempdir().unwrap();
        let store = store::Store::new(dir.path());
        let link = Hanging {
            world: fake::FakeLink::new(),
            hang: Arc::default(),
            cancel: Arc::default(),
        };
        link.world.add("SER1", "Pixel_8", "SER1");
        let slot = store.create(Platform::Android, "Pixel 8").unwrap();
        let c = Capture::start(Arc::new(link.clone()), "SER1", slot, Options::default()).unwrap();
        c.stop();
        link.hang.store(true, Ordering::Release);
        // The resume holds the worker's lock and waits for the writer's.
        let writer = c.writer();
        let resume = thread::spawn({
            let c = c.clone();
            move || c.resume()
        });
        while c.worker.try_lock().is_ok() {
            thread::sleep(Duration::from_millis(1));
        }
        // The stop comes before adb is asked anything.
        let stop = thread::spawn({
            let c = c.clone();
            move || c.stop()
        });
        while !c.stop.is_waited() {
            thread::sleep(Duration::from_millis(1));
        }
        drop(writer);
        let at = Instant::now();
        stop.join().unwrap();
        let took = at.elapsed();
        assert!(took < Duration::from_secs(3), "stop took {took:?}");
        assert!(resume.join().unwrap().is_err());
        assert_eq!(c.phase(), Phase::Stopped);
        // A resume after it records on.
        link.hang.store(false, Ordering::Release);
        c.resume().unwrap();
        assert_eq!(c.phase(), Phase::Recording);
        c.stop();
    }
}
