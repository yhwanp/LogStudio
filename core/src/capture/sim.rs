//! iOS simulators on this Mac, through the simulator tool that Xcode
//! installs with CoreSimulator. The tool is run directly: Xcode's own
//! `simctl` and `xcrun` may first install components, which this app never
//! does. Without the tool, no simulator is listed and nothing is installed.
//!
//! A capture runs `log stream` inside the simulator and reads its JSON
//! lines, one per entry. Each log entry becomes the row an iPhone's entry
//! becomes (see `ios`), so both read the same:
//!
//! `2026-10-24 14:02:12.017455 SampleApp[96:2210] [Error] [com.example.sample:Core] message`
//!
//! The stream sends only entries from the moment it starts. `log` has
//! already made each message valid UTF-8, so a message's bytes are not
//! always the bytes the app logged, and a field ends at its first NUL, as
//! an iPhone's does. A line that is not a JSON log entry gives no row:
//! `log`'s own notices, activity entries, a line JSON cannot read (such as
//! one with half a surrogate pair) and a line longer than `MAX_LINE`.
// Only the stream reads entries, and it needs a Unix socket.
#![cfg_attr(not(unix), allow(dead_code))]
use super::{
    ios::{write_row, Entry},
    link::{self, Device, Link, Platform, Stream},
    process::{command, run},
};
use crate::source::error;
use chrono::{DateTime, TimeZone};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    fmt::Display,
    io,
    path::{Path, PathBuf},
    time::Duration,
};

pub const LIST_FAILED: &str = "Could not list iOS simulators; check Xcode and retry";
pub const FAILED: &str = "Could not read the simulator's log; check the simulator and retry";
/// CoreSimulator's own tool, which Xcode's `simctl` runs after its checks.
const SIMCTL: &str =
    "/Library/Developer/PrivateFrameworks/CoreSimulator.framework/Versions/A/Resources/bin/simctl";
/// Listing took 0.07 s on a Mac with 21 simulators.
const LIST_TIMEOUT: Duration = Duration::from_secs(5);
const LIST_LIMIT: u64 = 16 << 20;
/// The longest JSON line converted. Its row is never longer: every field of
/// the row comes from the line, an escape is never shorter than the text it
/// stands for, and the line's keys outweigh the row's fixed parts. A longer
/// line would make a row the viewer cannot open, so it is dropped.
const MAX_LINE: usize = crate::source::MAX_LINE;
/// The entries a capture asks for: every level, log entries only, as JSON
/// lines, whose line breaks inside messages are escaped.
const STREAM: [&str; 8] = [
    "log", "stream", "--style", "ndjson", "--level", "debug", "--type", "log",
];

/// The simulators of CoreSimulator's tool. A simulator is named by its UDID.
pub struct SimLink {
    simctl: PathBuf,
}
impl SimLink {
    /// The tool, when this Mac has it.
    pub fn locate() -> Option<Self> {
        let simctl = Path::new(SIMCTL);
        (cfg!(target_os = "macos") && simctl.is_file()).then(|| Self {
            simctl: simctl.into(),
        })
    }
}

#[derive(Deserialize)]
struct Listing {
    devices: BTreeMap<String, Vec<Simulator>>,
}
#[derive(Deserialize)]
struct Simulator {
    udid: String,
    name: String,
    state: String,
    #[serde(rename = "isAvailable", default)]
    available: bool,
    #[serde(rename = "deviceTypeIdentifier", default)]
    kind: String,
}
/// A UDID `simctl` cannot read as an option: hex digits and `-`, not
/// starting with `-`.
fn valid_udid(udid: &str) -> bool {
    !udid.is_empty()
        && udid.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
        && !udid.starts_with('-')
}
/// The iOS simulators of `simctl list devices -j`: running ones are ready,
/// shut down ones `shutdown`, and others, such as one starting, `booting`.
/// Simulators of other systems are left out.
pub fn parse_list(json: &[u8]) -> io::Result<Vec<Device>> {
    let listing: Listing = serde_json::from_slice(json).map_err(|_| error(LIST_FAILED))?;
    let mut devices = vec![];
    for (runtime, simulators) in listing.devices {
        // `com.apple.CoreSimulator.SimRuntime.iOS-27-0` is iOS 27.0.
        let Some(version) = runtime
            .rsplit('.')
            .next()
            .and_then(|r| r.strip_prefix("iOS-"))
        else {
            continue;
        };
        for s in simulators {
            if !s.available || !valid_udid(&s.udid) {
                continue;
            }
            devices.push(Device {
                serial: s.udid,
                state: match s.state.as_str() {
                    "Booted" => "device",
                    "Shutdown" | "Shutting Down" => "shutdown",
                    _ => "booting",
                }
                .into(),
                model: s.name,
                // `…SimDeviceType.iPad-Pro-11-inch-M5` names an iPad.
                product: s.kind.rsplit('.').next().unwrap_or_default().into(),
                platform: Platform::Ios,
                version: version.replace('-', "."),
                simulator: true,
            });
        }
    }
    Ok(devices)
}

/// The fields of a `log stream --style ndjson` line that make a row.
#[derive(Deserialize)]
struct Event {
    #[serde(rename = "eventType", default)]
    kind: Option<String>,
    #[serde(rename = "messageType", default)]
    level: Option<String>,
    #[serde(default)]
    timestamp: Option<String>,
    #[serde(rename = "processImagePath", default)]
    process: Option<String>,
    #[serde(rename = "senderImagePath", default)]
    sender: Option<String>,
    #[serde(rename = "eventMessage", default)]
    message: Option<String>,
    #[serde(default)]
    subsystem: Option<String>,
    #[serde(default)]
    category: Option<String>,
    #[serde(rename = "processID", default)]
    pid: Option<u64>,
    #[serde(rename = "threadID", default)]
    tid: Option<u64>,
}
/// The level byte an iPhone's entry carries for `messageType`. Default
/// entries have none, so a level tag in the message becomes the row's level.
fn level(name: &str) -> u8 {
    match name {
        "Info" => 0x01,
        "Debug" => 0x02,
        "Error" => 0x10,
        "Fault" => 0x11,
        _ => 0,
    }
}
/// A field up to its first NUL, which no row may hold.
fn text(field: &Option<String>) -> &[u8] {
    let bytes = field.as_deref().unwrap_or_default().as_bytes();
    bytes.split(|&b| b == 0).next().unwrap_or_default()
}
/// Appends the row of one JSON line, timed in `zone`, when it is a log
/// entry. An unreadable `timestamp` becomes the current time.
fn convert<Tz: TimeZone>(line: &[u8], zone: &Tz, out: &mut Vec<u8>)
where
    Tz::Offset: Display,
{
    let Ok(e) = serde_json::from_slice::<Event>(line) else {
        return;
    };
    if e.kind.as_deref() != Some("logEvent") {
        return;
    }
    let time = e
        .timestamp
        .as_deref()
        .and_then(|t| DateTime::parse_from_str(t, "%Y-%m-%d %H:%M:%S%.f%z").ok())
        .map(|t| t.to_utc())
        .unwrap_or_else(chrono::Utc::now);
    let entry = Entry {
        pid: e.pid.unwrap_or_default() as u32,
        tid: e.tid.unwrap_or_default(),
        seconds: u32::try_from(time.timestamp()).unwrap_or_default(),
        micros: time.timestamp_subsec_micros(),
        level: level(e.level.as_deref().unwrap_or_default()),
        process: text(&e.process),
        sender: text(&e.sender),
        message: text(&e.message),
        subsystem: text(&e.subsystem),
        category: text(&e.category),
    };
    write_row(&entry, zone, out);
}
/// JSON lines as they come, in pieces of any size.
#[derive(Default)]
struct Lines {
    pending: Vec<u8>,
    /// The line being received is longer than `MAX_LINE`; it is dropped.
    skipping: bool,
}
impl Lines {
    /// Appends the rows of the whole lines `bytes` completes.
    fn push<Tz: TimeZone>(&mut self, mut bytes: &[u8], zone: &Tz, out: &mut Vec<u8>)
    where
        Tz::Offset: Display,
    {
        while let Some(end) = memchr::memchr(b'\n', bytes) {
            if self.skipping || self.pending.len() + end > MAX_LINE {
            } else if self.pending.is_empty() {
                convert(&bytes[..end], zone, out);
            } else {
                self.pending.extend_from_slice(&bytes[..end]);
                convert(&self.pending, zone, out);
            }
            self.pending.clear();
            self.skipping = false;
            bytes = &bytes[end + 1..];
        }
        if self.skipping {
        } else if self.pending.len() + bytes.len() > MAX_LINE {
            self.pending = vec![];
            self.skipping = true;
        } else {
            self.pending.extend_from_slice(bytes);
        }
    }
}

#[cfg(unix)]
mod stream {
    use super::*;
    use crate::capture::link::Closer;
    use std::{
        io::Read,
        net::Shutdown,
        os::{fd::OwnedFd, unix::net::UnixStream},
        process::{Child, Stdio},
        sync::Arc,
        time::Instant,
    };

    /// How long a stream may take to show it runs. `log` first says what it
    /// filters (`--type log` is a filter), however quiet the simulator is:
    /// after 0.30–0.39 s on a Mac with nothing else to do.
    const OPEN_TIMEOUT: Duration = if cfg!(test) {
        Duration::from_millis(500)
    } else {
        Duration::from_secs(5)
    };
    /// `simctl spawn` exit codes: the simulator is not running, or there is
    /// no simulator of that UDID.
    const NOT_BOOTED: i32 = 149;
    const NO_DEVICE: i32 = 148;

    /// A simulator's log as rows. `log` writes to a socket rather than a
    /// pipe: `log` runs inside the simulator, not as the tool's child, so
    /// ending the tool would not end it, but shutting the socket does at
    /// once.
    pub(super) struct SimStream {
        socket: Arc<UnixStream>,
        child: Child,
        lines: Lines,
        buf: Vec<u8>,
        rows: Vec<u8>,
        sent: usize,
    }
    /// Why `log` ended before it ran, by the tool's exit code.
    fn ended(child: &mut Child) -> io::Error {
        let deadline = Instant::now() + Duration::from_secs(2);
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status.code(),
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                _ => break None,
            }
        };
        match status {
            Some(NOT_BOOTED) => error(link::SHUTDOWN),
            Some(NO_DEVICE) => error(link::GONE),
            _ => error(FAILED),
        }
    }
    impl SimLink {
        pub(super) fn open(&self, udid: &str) -> io::Result<SimStream> {
            if !valid_udid(udid) {
                return Err(error(link::GONE));
            }
            let (ours, theirs) = UnixStream::pair().map_err(|_| error(FAILED))?;
            let mut c = command(&self.simctl);
            c.arg("spawn").arg(udid).args(STREAM);
            c.stdout(Stdio::from(OwnedFd::from(theirs)))
                .stderr(Stdio::null());
            let child = c.spawn().map_err(|_| error(FAILED))?;
            // The command holds its end too; the stream ends only once
            // `log` alone does.
            drop(c);
            let mut stream = SimStream {
                socket: Arc::new(ours),
                child,
                lines: Lines::default(),
                buf: vec![0; 64 * 1024],
                rows: vec![],
                sent: 0,
            };
            // A simulator that cannot stream ends `log` at once. `log` that
            // prints nothing within OPEN_TIMEOUT, as while a simulator still
            // boots, did not start.
            stream.socket.set_read_timeout(Some(OPEN_TIMEOUT))?;
            match (&*stream.socket).read(&mut stream.buf) {
                Ok(0) => return Err(ended(&mut stream.child)),
                Ok(n) => {
                    let first = stream.buf[..n].to_vec();
                    stream.lines.push(&first, &chrono::Local, &mut stream.rows);
                }
                Err(_) => return Err(error(FAILED)),
            }
            stream.socket.set_read_timeout(None)?;
            Ok(stream)
        }
    }
    impl Stream for SimStream {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            loop {
                if self.sent < self.rows.len() {
                    let n = buf.len().min(self.rows.len() - self.sent);
                    buf[..n].copy_from_slice(&self.rows[self.sent..self.sent + n]);
                    self.sent += n;
                    return Ok(n);
                }
                self.rows.clear();
                self.sent = 0;
                let n = match (&*self.socket).read(&mut self.buf) {
                    Ok(n) => n,
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(e) => return Err(e),
                };
                if n == 0 {
                    // The tool is done too; it is not left unreaped while
                    // the capture waits.
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                    return Ok(0);
                }
                self.lines
                    .push(&self.buf[..n], &chrono::Local, &mut self.rows);
            }
        }
        fn closer(&self) -> Closer {
            let socket = self.socket.clone();
            Arc::new(move || {
                let _ = socket.shutdown(Shutdown::Both);
            })
        }
    }
    impl Drop for SimStream {
        fn drop(&mut self) {
            let _ = self.socket.shutdown(Shutdown::Both);
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

impl Link for SimLink {
    /// A stream sends only entries from the moment it starts.
    fn replays(&self) -> bool {
        false
    }
    fn devices(&self) -> io::Result<Vec<Device>> {
        let mut c = command(&self.simctl);
        c.args(["list", "devices", "-j"]);
        let out = run(c, LIST_TIMEOUT, LIST_LIMIT).map_err(|_| error(LIST_FAILED))?;
        if !out.success {
            return Err(error(LIST_FAILED));
        }
        parse_list(&out.stdout)
    }
    /// A simulator's entries carry no boot, and streams send only new rows.
    fn boot_id(&self, _: &str) -> io::Result<String> {
        Ok(String::new())
    }
    fn identity(&self, serial: &str) -> io::Result<String> {
        Ok(serial.into())
    }
    fn recent(&self, _: &str, _: usize) -> io::Result<Vec<u8>> {
        Ok(vec![])
    }
    #[cfg(unix)]
    fn stream(&self, serial: &str, _: Option<super::gate::Stamp>) -> io::Result<Box<dyn Stream>> {
        Ok(Box::new(self.open(serial)?))
    }
    #[cfg(not(unix))]
    fn stream(&self, _: &str, _: Option<super::gate::Stamp>) -> io::Result<Box<dyn Stream>> {
        Err(error(link::GONE))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    /// A JSON line as `log stream --style ndjson` writes one, with synthetic
    /// values and some of the fields that make no row.
    fn line(level: &str, message: &str, subsystem: &str, category: &str) -> String {
        let mut e = serde_json::json!({
            "timezoneName": "",
            "messageType": level,
            "eventType": "logEvent",
            "source": null,
            "formatString": "%{public}s",
            "subsystem": subsystem,
            "category": category,
            "threadID": 4410,
            "backtrace": {"frames": [{"imageOffset": 1024, "imageUUID": "00000000-0000-0000-0000-000000000001"}]},
            "bootUUID": "",
            "processImagePath": "/Users/qa/Library/Developer/CoreSimulator/Devices/QA/data/Containers/Bundle/Application/QA/SampleApp.app/SampleApp",
            "senderImagePath": "/Library/Frameworks/SampleKit.framework/SampleKit",
            "timestamp": "2026-10-24 14:02:11.482913+0900",
            "machTimestamp": 1,
            "eventMessage": message,
            "processID": 812,
        });
        if level.is_empty() {
            e.as_object_mut().unwrap().remove("messageType");
        }
        e.to_string()
    }
    fn rows(input: &[u8], size: usize) -> String {
        let zone = FixedOffset::east_opt(9 * 3600).unwrap();
        let mut lines = Lines::default();
        let mut out = vec![];
        for chunk in input.chunks(size) {
            lines.push(chunk, &zone, &mut out);
        }
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn entries_become_iphone_rows() {
        let input = [
            line("Default", "[INFO] [Net] 요청 완료", "", ""),
            line("Error", "request failed", "com.example.sample", "Core"),
            line("Info", "m", "com.example", ""),
            line("Debug", "m", "com.example", ""),
            line("Fault", "m", "com.example", ""),
            line("Notice", "m", "com.example", ""),
        ]
        .join("\n")
            + "\n";
        assert_eq!(
            rows(input.as_bytes(), input.len()),
            "2026-10-24 14:02:11.482913 SampleApp[812:4410] [INFO] [Net] 요청 완료\n\
             2026-10-24 14:02:11.482913 SampleApp[812:4410] [Error] [com.example.sample:Core] request failed\n\
             2026-10-24 14:02:11.482913 SampleApp[812:4410] [Info] [com.example] m\n\
             2026-10-24 14:02:11.482913 SampleApp[812:4410] [Debug] [com.example] m\n\
             2026-10-24 14:02:11.482913 SampleApp[812:4410] [Fault] [com.example] m\n\
             2026-10-24 14:02:11.482913 SampleApp[812:4410] [com.example] m\n"
        );
    }

    #[test]
    fn times_are_written_in_the_zone_of_this_computer() {
        let input = line("Default", "m", "", "").replace("+0900", "-0130") + "\n";
        assert!(rows(input.as_bytes(), 7).starts_with("2026-10-25 00:32:11.482913 "));
    }

    #[test]
    fn messages_keep_inner_line_breaks_and_escaped_characters() {
        let input = line("Default", "first\n\tsecond \"q\" a/b\r\n\n", "", "") + "\n";
        assert!(input.contains("\\n") && !input.trim_end().contains('\n'));
        assert_eq!(
            rows(input.as_bytes(), 3),
            "2026-10-24 14:02:11.482913 SampleApp[812:4410] first\n\tsecond \"q\" a/b\n"
        );
    }

    #[test]
    fn lines_that_are_not_log_entries_give_no_row() {
        let activity = line("", "", "", "").replace("logEvent", "activityCreateEvent");
        let entry = line("Default", "kept", "", "");
        let input = format!(
            "Filtering the log data using \"type == 1024\"\n{activity}\n{{\"count\":2,\"finished\":1}}\n\
             getpwuid_r did not find a match for uid 501\n{{\"eventType\":\"logEvent\",\"processID\":\"x\"}}\n{entry}\n"
        );
        for size in [1, 5, 64, input.len()] {
            let got = rows(input.as_bytes(), size);
            assert_eq!(got.lines().count(), 1, "{size}: {got}");
            assert!(got.ends_with(" kept\n"));
        }
    }

    #[test]
    fn a_line_longer_than_max_line_is_dropped_up_to_its_end() {
        let zone = FixedOffset::east_opt(9 * 3600).unwrap();
        let mut lines = Lines::default();
        let mut out = vec![];
        let long = vec![b'x'; 1 << 20];
        for _ in 0..=MAX_LINE >> 20 {
            lines.push(&long, &zone, &mut out);
        }
        assert!(lines.skipping && lines.pending.is_empty());
        let rest = format!("tail\n{}\n", line("Default", "after", "", ""));
        lines.push(rest.as_bytes(), &zone, &mut out);
        let text = String::from_utf8(out).unwrap();
        assert_eq!(text.lines().count(), 1);
        assert!(text.ends_with(" after\n"));
    }

    #[test]
    fn a_line_within_max_line_gives_a_row_the_viewer_opens() {
        let zone = FixedOffset::east_opt(9 * 3600).unwrap();
        let rest = line("Default", "", "", "").len();
        for (len, kept) in [(MAX_LINE, true), (MAX_LINE + 1, false)] {
            let input = line("Default", &"x".repeat(len - rest), "", "") + "\n";
            assert_eq!(input.len(), len + 1);
            for size in [input.len(), 64 << 10] {
                let mut lines = Lines::default();
                let mut out = vec![];
                for chunk in input.as_bytes().chunks(size) {
                    lines.push(chunk, &zone, &mut out);
                }
                lines.push(line("Default", "next", "", "").as_bytes(), &zone, &mut out);
                lines.push(b"\n", &zone, &mut out);
                let text = String::from_utf8(out).unwrap();
                assert_eq!(
                    text.lines().count(),
                    if kept { 2 } else { 1 },
                    "{len} in {size}"
                );
                assert!(text.lines().all(|l| l.len() < crate::source::MAX_LINE));
                assert!(text.ends_with(" next\n"));
            }
        }
    }

    #[test]
    fn a_field_ends_at_its_first_nul() {
        let input = line("Default", "kept\u{0}dropped", "com.example\u{0}x", "") + "\n";
        assert!(input.contains("\\u0000"));
        assert_eq!(
            rows(input.as_bytes(), input.len()),
            "2026-10-24 14:02:11.482913 SampleApp[812:4410] [com.example] kept\n"
        );
    }

    #[test]
    fn rows_are_ios_rows_to_the_parser() {
        let input = line("Error", "request failed", "com.example.sample", "sync") + "\n";
        let text = rows(input.as_bytes(), input.len());
        let p = crate::parser::parse_line(text.trim_end(), 1, false)
            .parsed
            .unwrap();
        assert!(p.ios && p.pid == "812" && p.tid == "4410");
        assert_eq!(p.process, Some("SampleApp"));
        assert_eq!((p.level, p.tag), ("E", "com.example.sample:sync"));
    }

    #[test]
    fn ios_simulators_are_listed_by_runtime() {
        let json = serde_json::json!({"devices": {
            "com.apple.CoreSimulator.SimRuntime.iOS-27-0": [
                {"udid": "00000000-0000-0000-0000-00000000000A", "name": "iPhone 17", "state": "Booted",
                 "isAvailable": true, "deviceTypeIdentifier": "com.apple.CoreSimulator.SimDeviceType.iPhone-17"},
                {"udid": "00000000-0000-0000-0000-00000000000B", "name": "iPad Air", "state": "Shutdown",
                 "isAvailable": true, "deviceTypeIdentifier": "com.apple.CoreSimulator.SimDeviceType.iPad-Air-13-inch-M4"},
                {"udid": "00000000-0000-0000-0000-00000000000C", "name": "Old", "state": "Booted",
                 "isAvailable": false, "deviceTypeIdentifier": "x"},
                {"udid": "00000000-0000-0000-0000-00000000000E", "name": "iPhone Air", "state": "Booting",
                 "isAvailable": true, "deviceTypeIdentifier": "com.apple.CoreSimulator.SimDeviceType.iPhone-Air"},
                {"udid": "00000000-0000-0000-0000-00000000000F", "name": "iPad mini", "state": "Shutting Down",
                 "isAvailable": true, "deviceTypeIdentifier": "com.apple.CoreSimulator.SimDeviceType.iPad-mini-A17-Pro"},
                {"udid": "-0", "name": "Option", "state": "Booted", "isAvailable": true},
            ],
            "com.apple.CoreSimulator.SimRuntime.watchOS-12-0": [
                {"udid": "00000000-0000-0000-0000-00000000000D", "name": "Watch", "state": "Booted", "isAvailable": true},
            ],
        }});
        let devices = parse_list(json.to_string().as_bytes()).unwrap();
        let got: Vec<_> = devices
            .iter()
            .map(|d| {
                (
                    d.model.as_str(),
                    d.state.as_str(),
                    d.product.as_str(),
                    d.version.as_str(),
                )
            })
            .collect();
        assert_eq!(
            got,
            [
                ("iPhone 17", "device", "iPhone-17", "27.0"),
                ("iPad Air", "shutdown", "iPad-Air-13-inch-M4", "27.0"),
                ("iPhone Air", "booting", "iPhone-Air", "27.0"),
                ("iPad mini", "shutdown", "iPad-mini-A17-Pro", "27.0"),
            ]
        );
        assert!(devices
            .iter()
            .all(|d| d.simulator && d.platform == Platform::Ios));
        assert_eq!(devices[1].problem(), Some(link::SHUTDOWN));
        assert_eq!(devices[2].problem(), Some(link::BOOTING));
        assert_eq!(
            crate::source::public_message(&parse_list(b"not json").unwrap_err(), ""),
            LIST_FAILED
        );
    }

    #[cfg(unix)]
    mod streams {
        use super::*;
        use crate::source::public_message;
        use std::{os::unix::fs::PermissionsExt, time::Instant};

        /// A stand-in for the tool that runs `script` for `spawn`.
        fn tool(dir: &Path, script: &str) -> SimLink {
            let simctl = dir.join("simctl");
            std::fs::write(&simctl, format!("#!/bin/sh\n{script}\n")).unwrap();
            std::fs::set_permissions(&simctl, std::fs::Permissions::from_mode(0o755)).unwrap();
            SimLink { simctl }
        }
        const UDID: &str = "00000000-0000-0000-0000-00000000000A";

        #[test]
        fn a_quiet_stream_ends_at_once_when_closed() {
            let dir = tempfile::tempdir().unwrap();
            // A child of the tool keeps the output open, as `log` does.
            let entry = line("Default", "hello", "", "");
            let link = tool(
                dir.path(),
                &format!("printf '%s\\n' '{entry}'\nsleep 5 &\nwait"),
            );
            let mut stream = link.stream(UDID, None).unwrap();
            let mut buf = [0; 4096];
            let n = stream.read(&mut buf).unwrap();
            assert!(String::from_utf8_lossy(&buf[..n]).ends_with(" hello\n"));
            let close = stream.closer();
            let reader = std::thread::spawn(move || {
                let at = Instant::now();
                let n = stream.read(&mut buf).unwrap();
                (n, at.elapsed())
            });
            std::thread::sleep(Duration::from_millis(100));
            close();
            let (n, waited) = reader.join().unwrap();
            assert_eq!(n, 0);
            assert!(waited < Duration::from_secs(2), "{waited:?}");
        }

        #[test]
        fn a_simulator_that_is_not_running_is_named() {
            let dir = tempfile::tempdir().unwrap();
            let Err(e) = tool(dir.path(), "exit 149").stream(UDID, None) else {
                panic!("a stream of a shut down simulator");
            };
            assert_eq!(public_message(&e, ""), link::SHUTDOWN);
            let Err(e) = tool(dir.path(), "exit 148").stream(UDID, None) else {
                panic!("a stream of no simulator");
            };
            assert_eq!(public_message(&e, ""), link::GONE);
        }

        #[test]
        fn a_stream_that_says_nothing_has_not_started() {
            let dir = tempfile::tempdir().unwrap();
            let at = Instant::now();
            let Err(e) = tool(dir.path(), "sleep 5 &\nwait").stream(UDID, None) else {
                panic!("a silent stream");
            };
            assert_eq!(public_message(&e, ""), FAILED);
            assert!(at.elapsed() < Duration::from_secs(4), "{:?}", at.elapsed());
        }

        #[test]
        fn the_tool_gets_the_udid_as_its_own_argument() {
            let dir = tempfile::tempdir().unwrap();
            let args = dir.path().join("args");
            let link = tool(
                dir.path(),
                &format!("printf '%s\\n' \"$@\" > '{}'\nexit 1", args.display()),
            );
            assert!(link.stream(UDID, None).is_err());
            let got = std::fs::read_to_string(&args).unwrap();
            let want: Vec<&str> = ["spawn", UDID].into_iter().chain(STREAM).collect();
            assert_eq!(got.lines().collect::<Vec<_>>(), want);
            assert!(link.stream("-0", None).is_err(), "an option is no UDID");
        }
    }
}
