//! iPhones and iPads through Apple's device service (usbmuxd), which macOS
//! runs and the Apple Devices app or iTunes installs on Windows. A capture reads the device's
//! `com.apple.os_trace_relay` service: each log entry arrives as a binary
//! record from the moment the service starts, never from before. Entries
//! are written as rows the iOS parser reads:
//!
//! `2026-10-24 14:02:12.017455 SampleApp[96:2210] [Error] [com.example.sample:Core] message`
//!
//! Each entry's time is written in this computer's time zone, to the
//! microsecond. Default entries, which
//! NSLog and print write, carry no level field, so a level the app wrote
//! into its message, as in `[INFO] [Net] done`, is the row's level; Info,
//! Debug, Error and Fault entries carry theirs. A subsystem and category
//! form one tag. Message bytes are kept as they came, including the rows
//! after a message's line breaks.
use super::link::{self, Closer, Device, Link, Platform, Stream};
use crate::source::{error, public_message};
use chrono::{DateTime, TimeZone};
use idevice::{
    lockdown::LockdownClient,
    pairing_file::PairingFile,
    provider::{IdeviceProvider, UsbmuxdProvider},
    usbmuxd::{Connection, UsbmuxdAddr, UsbmuxdDevice},
    Idevice, IdeviceError, IdeviceService,
};
use std::{
    fmt::Display,
    future::Future,
    io::{self, Write},
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{runtime::Runtime, sync::Notify};

/// No device service answers: on Windows, neither the Apple Devices app nor
/// iTunes is installed.
pub const NO_SERVICE: &str = "Apple device service not found; install Apple Devices or iTunes";
pub(super) const FAILED: &str = "Could not reach the device; check its connection and retry";
const SERVICE: &str = "com.apple.os_trace_relay";
/// How long one device request may take.
const TIMEOUT: Duration = Duration::from_secs(5);
/// How long describing one listed device may take within `TIMEOUT`, so a
/// Wi-Fi device that stopped answering cannot hide the others.
const DESCRIBE_TIMEOUT: Duration = Duration::from_secs(3);
/// The name the device's logs give this app's requests.
pub(super) const LABEL: &str = "LogStudio";

/// One log entry of an `os_trace_relay` record. Strings are the record's
/// bytes up to their NUL; `process` is the process's executable path.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Entry<'a> {
    pub pid: u32,
    pub tid: u64,
    pub seconds: u32,
    pub micros: u32,
    pub level: u8,
    pub process: &'a [u8],
    /// The library or executable that logged the entry.
    pub sender: &'a [u8],
    pub message: &'a [u8],
    pub subsystem: &'a [u8],
    pub category: &'a [u8],
}
/// A record's fixed header, as pymobiledevice3 and idevice read it; its
/// other bytes are unknown. The process path follows, ended by a NUL, then
/// the sender, message, subsystem and category, each as long as the header
/// says, NUL included.
const HEADER: usize = 129;
const PID: usize = 9;
const SECONDS: usize = 55;
const MICROS: usize = 63;
const LEVEL: usize = 68;
const TID: usize = 83;
const SENDER_LEN: usize = 107;
const MESSAGE_LEN: usize = 109;
const SUBSYSTEM_LEN: usize = 117;
const CATEGORY_LEN: usize = 121;

/// The bytes before the first NUL.
fn c_string(bytes: &[u8]) -> &[u8] {
    bytes.split(|&b| b == 0).next().unwrap_or_default()
}
/// Reads a record; None when it is shorter than its header says.
pub fn entry(record: &[u8]) -> Option<Entry<'_>> {
    let head = record.get(..HEADER)?;
    let le = |at: usize, n: usize| {
        head[at..at + n]
            .iter()
            .rev()
            .fold(0u64, |v, &b| v << 8 | u64::from(b))
    };
    let mut rest = &record[HEADER..];
    let path = memchr::memchr(0, rest)?;
    let process = &rest[..path];
    rest = &rest[path + 1..];
    let mut take = |len: u64| {
        let part = rest.get(..usize::try_from(len).ok()?)?;
        rest = &rest[part.len()..];
        Some(c_string(part))
    };
    let sender = take(le(SENDER_LEN, 2))?;
    let message = take(le(MESSAGE_LEN, 2))?;
    // A label too long for the record is left out; the entry is not.
    let (subsystem, category) = take(le(SUBSYSTEM_LEN, 4))
        .zip(take(le(CATEGORY_LEN, 4)))
        .unwrap_or_default();
    Some(Entry {
        pid: le(PID, 4) as u32,
        tid: le(TID, 8),
        seconds: le(SECONDS, 4) as u32,
        micros: le(MICROS, 4) as u32,
        level: head[LEVEL],
        process,
        sender,
        message,
        subsystem,
        category,
    })
}
/// The level field of a level byte. Default entries (0) and levels this app
/// does not know have none.
fn level(byte: u8) -> &'static [u8] {
    match byte {
        0x01 => b"[Info] ",
        0x02 => b"[Debug] ",
        0x10 => b"[Error] ",
        0x11 => b"[Fault] ",
        _ => b"",
    }
}
/// Appends `e` as one row, timed in `zone`, with its line ending. Line
/// endings that end the message would only add empty rows.
pub fn write_row<Tz: TimeZone>(e: &Entry, zone: &Tz, out: &mut Vec<u8>)
where
    Tz::Offset: Display,
{
    let time = DateTime::from_timestamp(i64::from(e.seconds), e.micros.min(999_999) * 1000)
        .unwrap_or_default()
        .with_timezone(zone);
    let _ = write!(out, "{} ", time.format("%Y-%m-%d %H:%M:%S%.6f"));
    let path = if e.process.is_empty() {
        e.sender
    } else {
        e.process
    };
    out.extend_from_slice(path.rsplit(|&b| b == b'/').next().unwrap_or(path));
    let _ = write!(out, "[{}:{}] ", e.pid, e.tid);
    out.extend_from_slice(level(e.level));
    if !e.subsystem.is_empty() {
        out.push(b'[');
        out.extend_from_slice(e.subsystem);
        if !e.category.is_empty() {
            out.push(b':');
            out.extend_from_slice(e.category);
        }
        out.extend_from_slice(b"] ");
    }
    let end = e
        .message
        .iter()
        .rposition(|&b| b != b'\n' && b != b'\r')
        .map_or(0, |i| i + 1);
    out.extend_from_slice(&e.message[..end]);
    out.push(b'\n');
}

/// Frames of the service, after the reply to its start: a type byte, a
/// length and that many bytes. Type 2 frames hold a record; a type 1 frame
/// holds a property list, whose length alone is big-endian.
#[derive(Default)]
struct Frames {
    bytes: Vec<u8>,
    at: usize,
}
/// Larger than any record: the stream's framing is corrupt.
const MAX_FRAME: usize = 16 << 20;
impl Frames {
    /// The next whole frame's type and bytes; Ok(None) until it has come.
    fn next(&mut self) -> io::Result<Option<(u8, &[u8])>> {
        let rest = &self.bytes[self.at..];
        let Some(head) = rest.get(..5) else {
            return Ok(None);
        };
        let len = [head[1], head[2], head[3], head[4]];
        let len = if head[0] == 1 {
            u32::from_be_bytes(len)
        } else {
            u32::from_le_bytes(len)
        } as usize;
        if len > MAX_FRAME {
            return Err(error(FAILED));
        }
        if rest.len() < 5 + len {
            return Ok(None);
        }
        let start = self.at + 5;
        self.at = start + len;
        Ok(Some((head[0], &self.bytes[start..start + len])))
    }
    fn push(&mut self, bytes: &[u8]) {
        if self.at > 0 {
            self.bytes.drain(..self.at);
            self.at = 0;
        }
        self.bytes.extend_from_slice(bytes);
    }
}

/// Ends a stream's blocked read: the flag for reads to come, the notice
/// for the one waiting.
#[derive(Default)]
struct Stop {
    set: AtomicBool,
    notice: Notify,
}
/// A device's log as rows. The runtime runs only inside `read`.
struct IosStream {
    runtime: Runtime,
    device: Idevice,
    stop: Arc<Stop>,
    frames: Frames,
    rows: Vec<u8>,
    sent: usize,
}
impl IosStream {
    /// Turns the whole frames received into rows; false when none came.
    fn rows(&mut self) -> io::Result<bool> {
        self.rows.clear();
        self.sent = 0;
        let zone = chrono::Local;
        while let Some((kind, frame)) = self.frames.next()? {
            if kind == 2 {
                // A record shorter than its header says is skipped: its
                // bytes are not a row of text.
                if let Some(e) = entry(frame) {
                    write_row(&e, &zone, &mut self.rows);
                }
            }
        }
        Ok(!self.rows.is_empty())
    }
}
impl Stream for IosStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            if self.sent < self.rows.len() {
                let n = buf.len().min(self.rows.len() - self.sent);
                buf[..n].copy_from_slice(&self.rows[self.sent..self.sent + n]);
                self.sent += n;
                return Ok(n);
            }
            if self.rows()? {
                continue;
            }
            if self.stop.set.load(Ordering::Acquire) {
                return Ok(0);
            }
            let (device, stop) = (&mut self.device, self.stop.clone());
            let received = guard(|| {
                self.runtime.block_on(async {
                    tokio::select! {
                        r = device.read_any(64 * 1024) => Some(r),
                        () = stop.notice.notified() => None,
                    }
                })
            })?;
            match received {
                Some(Ok(bytes)) if !bytes.is_empty() => self.frames.push(&bytes),
                Some(Err(e)) => return Err(io_error(e)),
                _ => return Ok(0),
            }
        }
    }
    fn closer(&self) -> Closer {
        let stop = self.stop.clone();
        Arc::new(move || {
            stop.set.store(true, Ordering::Release);
            stop.notice.notify_one();
        })
    }
}

/// Apple's device service, which lists and reaches iPhones and iPads
/// connected by USB or Wi-Fi. A device is named by its UDID however it
/// connects.
#[derive(Default)]
pub struct IosLink {
    pub(super) addr: UsbmuxdAddr,
}
fn runtime() -> io::Result<Runtime> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
}
/// Runs `f`, turning a panic of the device library into an error.
fn guard<T>(f: impl FnOnce() -> T) -> io::Result<T> {
    catch_unwind(AssertUnwindSafe(f)).map_err(|_| error(FAILED))
}
/// Runs `f` on `runtime` within `TIMEOUT`.
fn block_on<T>(runtime: &Runtime, f: impl Future<Output = io::Result<T>>) -> io::Result<T> {
    guard(|| {
        runtime.block_on(async {
            tokio::time::timeout(TIMEOUT, f)
                .await
                .unwrap_or_else(|_| Err(error(FAILED)))
        })
    })?
}
/// Runs one request on a runtime of its own.
pub(super) fn request<T>(f: impl Future<Output = io::Result<T>>) -> io::Result<T> {
    block_on(&runtime()?, f)
}
pub(super) fn io_error(e: IdeviceError) -> io::Error {
    match e {
        IdeviceError::InvalidHostID => error(link::UNTRUSTED),
        IdeviceError::DeviceLocked => error(link::LOCKED),
        // With the library's pairing feature off, these come as unknown errors.
        IdeviceError::UnknownErrorType(e) if e.starts_with("UserDeniedPairing") => {
            error(link::UNTRUSTED)
        }
        IdeviceError::UnknownErrorType(e) if e.starts_with("PasswordProtected") => {
            error(link::LOCKED)
        }
        IdeviceError::DeviceNotFound => error(link::GONE),
        _ => error(FAILED),
    }
}
/// The devices connected by USB or Wi-Fi, each once. A device on both is
/// reached through USB.
fn reachable(list: Vec<UsbmuxdDevice>) -> Vec<UsbmuxdDevice> {
    let mut found: Vec<UsbmuxdDevice> = vec![];
    for d in list {
        let usb = match d.connection_type {
            Connection::Usb => true,
            Connection::Network(_) => false,
            Connection::Unknown(_) => continue,
        };
        match found.iter_mut().find(|f| f.udid == d.udid) {
            Some(f) if usb => *f = d,
            Some(_) => {}
            None => found.push(d),
        }
    }
    found
}
/// Starts the session services need on a lockdown connection, with the
/// pairing record that makes this computer trusted.
pub(super) async fn session(
    provider: &UsbmuxdProvider,
    lockdown: &mut LockdownClient,
) -> io::Result<PairingFile> {
    // Without a pairing record this computer was never trusted.
    let pair = provider
        .get_pairing_file()
        .await
        .map_err(|_| error(link::UNTRUSTED))?;
    lockdown.start_session(&pair).await.map_err(io_error)?;
    Ok(pair)
}
/// A text value the device gives; empty when it gives none.
async fn value(lockdown: &mut LockdownClient, key: &str) -> String {
    match lockdown.get_value(Some(key), None).await {
        Ok(v) => v.as_string().unwrap_or_default().to_string(),
        Err(_) => String::new(),
    }
}
/// A listed device that cannot be described.
fn offline(udid: String) -> Device {
    Device {
        serial: udid,
        state: "offline".into(),
        platform: Platform::Ios,
        ..Device::default()
    }
}
/// The listed device's names and whether a capture can start on it.
async fn describe(addr: UsbmuxdAddr, d: UsbmuxdDevice) -> Device {
    let provider = d.to_provider(addr, LABEL);
    let mut device = offline(d.udid);
    let Ok(mut lockdown) = LockdownClient::connect(&provider).await else {
        return device;
    };
    // Lockdown gives these values before a session, even to untrusted
    // computers.
    device.product = value(&mut lockdown, "ProductType").await;
    device.version = value(&mut lockdown, "ProductVersion").await;
    device.model = value(&mut lockdown, "DeviceName").await;
    if device.model.is_empty() {
        device.model = value(&mut lockdown, "DeviceClass").await;
    }
    device.state = match session(&provider, &mut lockdown).await {
        Ok(_) => "device",
        Err(e) => match public_message(&e, "").as_str() {
            link::UNTRUSTED => "untrusted",
            link::LOCKED => "locked",
            _ => "offline",
        },
    }
    .into();
    device
}
/// Starts the log service on a device in a session and reads its reply.
async fn start(provider: &UsbmuxdProvider) -> io::Result<Idevice> {
    let mut lockdown = LockdownClient::connect(provider).await.map_err(io_error)?;
    let pair = session(provider, &mut lockdown).await?;
    let (port, tls) = lockdown.start_service(SERVICE).await.map_err(io_error)?;
    let mut device = provider.connect(port).await.map_err(io_error)?;
    if tls {
        device.start_session(&pair, false).await.map_err(io_error)?;
    }
    // Every entry from now on, as idevicesyslog and pymobiledevice3 ask.
    let mut ask = plist::Dictionary::new();
    ask.insert("Request".into(), "StartActivity".into());
    ask.insert("Pid".into(), (-1i64).into());
    ask.insert("MessageFilter".into(), 65_535i64.into());
    ask.insert("StreamFlags".into(), 60i64.into());
    let mut body = vec![];
    plist::Value::Dictionary(ask)
        .to_writer_binary(&mut body)
        .map_err(|_| error(FAILED))?;
    let mut message = (body.len() as u32).to_be_bytes().to_vec();
    message.extend_from_slice(&body);
    device.send_raw(&message).await.map_err(io_error)?;
    let kind = device.read_raw(1).await.map_err(io_error)?;
    let len = device.read_raw(4).await.map_err(io_error)?;
    let len = u32::from_be_bytes([len[0], len[1], len[2], len[3]]) as usize;
    if kind[0] != 1 || len > MAX_FRAME {
        return Err(error(FAILED));
    }
    let reply: plist::Value = plist::from_bytes(&device.read_raw(len).await.map_err(io_error)?)
        .map_err(|_| error(FAILED))?;
    let status = reply
        .as_dictionary()
        .and_then(|d| d.get("Status"))
        .and_then(plist::Value::as_string);
    if status != Some("RequestSuccessful") {
        return Err(error(FAILED));
    }
    Ok(device)
}
impl IosLink {
    pub(super) async fn listed(&self) -> io::Result<Vec<UsbmuxdDevice>> {
        let mut mux = self.addr.connect(0).await.map_err(|_| error(NO_SERVICE))?;
        Ok(reachable(mux.get_devices().await.map_err(io_error)?))
    }
}
impl Link for IosLink {
    fn replays(&self) -> bool {
        false
    }
    fn devices(&self) -> io::Result<Vec<Device>> {
        request(async {
            // Side by side, each within its own time: a device that does not
            // answer is offline instead of failing the whole list.
            let tasks: Vec<_> = self
                .listed()
                .await?
                .into_iter()
                .map(|d| {
                    let udid = d.udid.clone();
                    let described = describe(self.addr.clone(), d);
                    let task = tokio::spawn(tokio::time::timeout(DESCRIBE_TIMEOUT, described));
                    (udid, task)
                })
                .collect();
            let mut devices = vec![];
            for (udid, task) in tasks {
                devices.push(match task.await {
                    Ok(Ok(device)) => device,
                    _ => offline(udid),
                });
            }
            Ok(devices)
        })
    }
    /// Streams send only new rows, so no boot needs telling apart.
    fn boot_id(&self, _: &str) -> io::Result<String> {
        Ok(String::new())
    }
    /// The UDID names the device however it is connected.
    fn identity(&self, serial: &str) -> io::Result<String> {
        Ok(serial.into())
    }
    fn recent(&self, _: &str, _: usize) -> io::Result<Vec<u8>> {
        Ok(vec![])
    }
    fn stream(&self, serial: &str, _: Option<super::gate::Stamp>) -> io::Result<Box<dyn Stream>> {
        let runtime = runtime()?;
        let device = block_on(&runtime, async {
            let d = self
                .listed()
                .await?
                .into_iter()
                .find(|d| d.udid == serial)
                .ok_or_else(|| error(link::GONE))?;
            start(&d.to_provider(self.addr.clone(), LABEL)).await
        })?;
        Ok(Box::new(IosStream {
            runtime,
            device,
            stop: Arc::default(),
            frames: Frames::default(),
            rows: vec![],
            sent: 0,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    /// A record as the service sends it, with synthetic values.
    pub fn record(e: &Entry) -> Vec<u8> {
        let mut r = vec![0u8; HEADER];
        let put = |r: &mut Vec<u8>, at: usize, bytes: &[u8]| {
            r[at..at + bytes.len()].copy_from_slice(bytes)
        };
        put(&mut r, PID, &e.pid.to_le_bytes());
        put(&mut r, SECONDS, &e.seconds.to_le_bytes());
        put(&mut r, MICROS, &e.micros.to_le_bytes());
        r[LEVEL] = e.level;
        put(&mut r, TID, &e.tid.to_le_bytes());
        let sized = |s: &[u8]| if s.is_empty() { 0 } else { s.len() + 1 };
        put(&mut r, SENDER_LEN, &(sized(e.sender) as u16).to_le_bytes());
        put(
            &mut r,
            MESSAGE_LEN,
            &(sized(e.message) as u16).to_le_bytes(),
        );
        put(
            &mut r,
            SUBSYSTEM_LEN,
            &(sized(e.subsystem) as u32).to_le_bytes(),
        );
        put(
            &mut r,
            CATEGORY_LEN,
            &(sized(e.category) as u32).to_le_bytes(),
        );
        r.extend_from_slice(e.process);
        r.push(0);
        for s in [e.sender, e.message, e.subsystem, e.category] {
            if !s.is_empty() {
                r.extend_from_slice(s);
                r.push(0);
            }
        }
        r
    }
    fn sample() -> Entry<'static> {
        Entry {
            pid: 812,
            tid: 4410,
            // 2026-10-24 05:02:11.482913 UTC
            seconds: 1_792_818_131,
            micros: 482_913,
            level: 0,
            process: b"/var/apps/SampleApp.app/SampleApp",
            sender: b"/Library/Frameworks/SampleKit.framework/SampleKit",
            message: "[INFO] [Net] 요청 완료".as_bytes(),
            subsystem: b"",
            category: b"",
        }
    }
    fn row(e: &Entry) -> String {
        let mut out = vec![];
        write_row(e, &FixedOffset::east_opt(9 * 3600).unwrap(), &mut out);
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn records_read_back_their_fields() {
        let e = Entry {
            level: 0x10,
            subsystem: b"com.example.sample",
            category: b"sync",
            ..sample()
        };
        assert_eq!(entry(&record(&e)), Some(e));
    }

    #[test]
    fn short_or_overlong_records_are_not_read() {
        let r = record(&sample());
        assert_eq!(entry(&r[..HEADER - 1]), None);
        assert_eq!(entry(&r[..HEADER + 10]), None, "the path has no NUL");
        let mut long = r.clone();
        long[MESSAGE_LEN] = 0xff;
        long[MESSAGE_LEN + 1] = 0xff;
        assert_eq!(entry(&long), None, "the message is longer than the record");
        // A label longer than the record is left out, and the entry kept.
        let labeled = Entry {
            subsystem: b"com.example.sample",
            category: b"sync",
            ..sample()
        };
        let mut r = record(&labeled);
        r[CATEGORY_LEN] = 0xff;
        let read = entry(&r).unwrap();
        assert_eq!((read.subsystem, read.category), (&b""[..], &b""[..]));
        assert_eq!(read.message, labeled.message);
    }

    #[test]
    fn rows_carry_the_time_process_ids_level_and_label() {
        assert_eq!(
            row(&sample()),
            "2026-10-24 14:02:11.482913 SampleApp[812:4410] [INFO] [Net] 요청 완료\n"
        );
        let e = Entry {
            level: 0x10,
            process: b"/usr/libexec/sampled",
            message: b"request failed",
            subsystem: b"com.example.sample",
            category: b"Core",
            ..sample()
        };
        assert_eq!(
            row(&e),
            "2026-10-24 14:02:11.482913 sampled[812:4410] [Error] [com.example.sample:Core] request failed\n"
        );
        let levels = [(1, "[Info] "), (2, "[Debug] "), (0x11, "[Fault] "), (3, "")];
        for (byte, field) in levels {
            let e = Entry {
                level: byte,
                message: b"m",
                subsystem: b"com.example",
                ..sample()
            };
            assert_eq!(
                row(&e),
                format!("2026-10-24 14:02:11.482913 SampleApp[812:4410] {field}[com.example] m\n")
            );
        }
    }

    #[test]
    fn rows_keep_message_bytes_and_inner_line_breaks() {
        let e = Entry {
            process: b"",
            sender: b"/usr/lib/libexample.dylib",
            message: b"first\nsecond \xff\r\n\n",
            ..sample()
        };
        let mut out = vec![];
        write_row(&e, &FixedOffset::east_opt(9 * 3600).unwrap(), &mut out);
        assert_eq!(
            out,
            b"2026-10-24 14:02:11.482913 libexample.dylib[812:4410] first\nsecond \xff\n".to_vec()
        );
    }

    #[test]
    fn rows_are_ios_rows_to_the_parser() {
        let mut out = vec![];
        let zone = FixedOffset::east_opt(0).unwrap();
        let entries = [
            sample(),
            Entry {
                level: 0x11,
                subsystem: b"com.example.sample",
                category: b"sync",
                message: b"store corrupted",
                ..sample()
            },
            Entry {
                message: b"plain message",
                ..sample()
            },
        ];
        for e in &entries {
            write_row(e, &zone, &mut out);
        }
        let text = String::from_utf8(out).unwrap();
        let rows: Vec<_> = text
            .lines()
            .map(|l| crate::parser::parse_line(l, 1, false).parsed.unwrap())
            .collect();
        assert!(rows
            .iter()
            .all(|p| p.ios && p.pid == "812" && p.tid == "4410"));
        assert_eq!(rows[0].process, Some("SampleApp"));
        assert_eq!((rows[0].level, rows[0].tag), ("I", "Net"));
        assert_eq!(
            (rows[1].level, rows[1].tag),
            ("F", "com.example.sample:sync")
        );
        assert_eq!(
            (rows[2].level, rows[2].tag, rows[2].message),
            ("U", "", "plain message")
        );
        assert_eq!(rows[0].time, "05:02:11.482913");
    }

    #[test]
    fn wifi_devices_are_listed_once_and_usb_wins() {
        let at = |udid: &str, device_id, connection_type| UsbmuxdDevice {
            connection_type,
            udid: udid.into(),
            device_id,
        };
        let wifi = || Connection::Network("192.0.2.7".parse().unwrap());
        let list = reachable(vec![
            at("QA0UDID0000000000000001", 1, wifi()),
            at("QA0UDID0000000000000002", 2, wifi()),
            at("QA0UDID0000000000000001", 3, Connection::Usb),
            at("QA0UDID0000000000000001", 4, wifi()),
            at(
                "QA0UDID0000000000000003",
                5,
                Connection::Unknown("Bluetooth".into()),
            ),
        ]);
        let got: Vec<_> = list
            .iter()
            .map(|d| (d.udid.as_str(), d.device_id))
            .collect();
        assert_eq!(
            got,
            [
                ("QA0UDID0000000000000001", 3),
                ("QA0UDID0000000000000002", 2)
            ]
        );
    }

    /// A device service that lists `udids` as Wi-Fi devices and never
    /// answers a connection to them, as for a device that went out of range.
    #[cfg(unix)]
    fn silent_service(dir: &std::path::Path, udids: &[&str]) -> UsbmuxdAddr {
        use std::{io::Read, os::unix::net::UnixListener};
        let path = dir.join("usbmuxd");
        let listener = UnixListener::bind(&path).unwrap();
        let list: Vec<plist::Value> = udids
            .iter()
            .enumerate()
            .map(|(i, udid)| {
                let mut properties = plist::Dictionary::new();
                properties.insert("ConnectionType".into(), "Network".into());
                // A sockaddr_in as macOS gives it: length, AF_INET, port,
                // then 192.0.2.7.
                let address = vec![0x10, 2, 0, 0, 192, 0, 2, 7, 0, 0, 0, 0, 0, 0, 0, 0];
                properties.insert("NetworkAddress".into(), plist::Value::Data(address));
                properties.insert("SerialNumber".into(), (*udid).into());
                let mut device = plist::Dictionary::new();
                device.insert("DeviceID".into(), (i as u64 + 1).into());
                device.insert("MessageType".into(), "Attached".into());
                device.insert("Properties".into(), properties.into());
                device.into()
            })
            .collect();
        let mut reply = plist::Dictionary::new();
        reply.insert("DeviceList".into(), list.into());
        let mut body = vec![];
        plist::Value::Dictionary(reply)
            .to_writer_xml(&mut body)
            .unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let (Ok(mut stream), body) = (stream, body.clone()) else {
                    return;
                };
                std::thread::spawn(move || {
                    let mut header = [0; 16];
                    if stream.read_exact(&mut header).is_err() {
                        return;
                    }
                    let len = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
                    let mut request = vec![0; len as usize - 16];
                    if stream.read_exact(&mut request).is_err() {
                        return;
                    }
                    let request: plist::Dictionary = plist::from_bytes(&request).unwrap();
                    if request.get("MessageType").and_then(plist::Value::as_string)
                        == Some("ListDevices")
                    {
                        let mut out = ((16 + body.len()) as u32).to_le_bytes().to_vec();
                        out.extend(1u32.to_le_bytes());
                        out.extend(8u32.to_le_bytes());
                        out.extend_from_slice(&header[12..]);
                        out.extend(&body);
                        let _ = stream.write_all(&out);
                    }
                    // Connections stay unanswered until the client leaves.
                    let _ = io::copy(&mut stream, &mut io::sink());
                });
            }
        });
        UsbmuxdAddr::UnixSocket(path.to_string_lossy().into_owned())
    }

    #[cfg(unix)]
    #[test]
    fn wifi_devices_that_do_not_answer_are_offline_without_hiding_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let udids = ["QA0UDID0000000000000001", "QA0UDID0000000000000002"];
        let link = IosLink {
            addr: silent_service(dir.path(), &udids),
        };
        let at = std::time::Instant::now();
        let devices = link.devices().unwrap();
        // One after another, the two would take longer than the request may.
        assert!(at.elapsed() < TIMEOUT, "{:?}", at.elapsed());
        let got: Vec<_> = devices
            .iter()
            .map(|d| (d.serial.as_str(), d.state.as_str()))
            .collect();
        assert_eq!(got, [(udids[0], "offline"), (udids[1], "offline")]);
    }

    #[cfg(unix)]
    #[test]
    fn a_missing_device_service_is_named() {
        let link = IosLink {
            addr: UsbmuxdAddr::UnixSocket("/nonexistent/usbmuxd".into()),
        };
        assert_eq!(public_message(&link.devices().unwrap_err(), ""), NO_SERVICE);
        let Err(e) = link.stream("QA0UDID0000000000000001", None) else {
            panic!("a stream without the service");
        };
        assert_eq!(public_message(&e, ""), NO_SERVICE);
    }

    #[test]
    fn frames_reassemble_across_chunk_sizes() {
        let mut bytes = vec![];
        for (kind, body) in [(2u8, &b"one"[..]), (1, b"plist"), (2, b"")] {
            bytes.push(kind);
            let len = body.len() as u32;
            bytes.extend(if kind == 1 {
                len.to_be_bytes()
            } else {
                len.to_le_bytes()
            });
            bytes.extend_from_slice(body);
        }
        for size in [1, 2, 7, bytes.len()] {
            let mut frames = Frames::default();
            let mut got = vec![];
            for chunk in bytes.chunks(size) {
                frames.push(chunk);
                while let Some((kind, body)) = frames.next().unwrap() {
                    got.push((kind, body.to_vec()));
                }
            }
            assert_eq!(
                got,
                vec![(2, b"one".to_vec()), (1, b"plist".to_vec()), (2, vec![])],
                "{size}"
            );
        }
        let mut frames = Frames::default();
        frames.push(&[2, 0xff, 0xff, 0xff, 0x7f]);
        assert!(frames.next().is_err(), "a length past any record");
    }
}
