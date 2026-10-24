//! A scripted device connection for tests, measurements and QA builds:
//! devices with logs that can be written, unplugged, plugged again and
//! rebooted, as `logcat` would report them, or, for a live link, iPhones
//! that send only the rows logged while a stream is open, with developer
//! builds that can be started with their private values shown.
use super::{
    apps::{self, App, Apps},
    gate::Stamp,
    ios,
    link::{self, Closer, Device, Link, Platform, Stream},
};
use crate::source::error;
use std::{
    collections::VecDeque,
    io,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex,
    },
    thread,
    time::Duration,
};

#[derive(Default)]
pub struct FakeLink {
    world: Mutex<World>,
    /// Streams send only rows logged after they open, as iPhones do.
    live: bool,
}
#[derive(Default)]
struct World {
    devices: Vec<FakeDevice>,
    conflict: bool,
    /// `since` of each stream opened, in order.
    opened: Vec<(String, Option<String>)>,
    launched: Vec<(String, String)>,
    /// Why apps can be neither listed nor started, as without Xcode.
    apps_error: Option<&'static str>,
    /// How long starting an app takes, as on a device Xcode waits for.
    launch_delay: Duration,
}
struct FakeDevice {
    info: Device,
    plugged: bool,
    identity: String,
    boot: u64,
    /// Rows the device holds, each with its line ending.
    log: Vec<Vec<u8>>,
    streams: Vec<Arc<Pipe>>,
    apps: Vec<App>,
    /// The process ID SampleApp has since it was started with its private
    /// values shown.
    unredacted_pid: Option<u32>,
}
#[derive(Default)]
struct Pipe {
    state: Mutex<PipeState>,
    ready: Condvar,
}
#[derive(Default)]
struct PipeState {
    bytes: VecDeque<u8>,
    closed: bool,
}
impl Pipe {
    fn send(&self, bytes: &[u8]) {
        let mut s = self.state.lock().unwrap();
        if !s.closed {
            s.bytes.extend(bytes);
            self.ready.notify_all();
        }
    }
    fn close(&self) {
        self.state.lock().unwrap().closed = true;
        self.ready.notify_all();
    }
}
struct FakeStream(Arc<Pipe>);
impl Stream for FakeStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut s = self.0.state.lock().unwrap();
        loop {
            if !s.bytes.is_empty() {
                let n = buf.len().min(s.bytes.len());
                for (b, v) in buf.iter_mut().zip(s.bytes.drain(..n)) {
                    *b = v;
                }
                return Ok(n);
            }
            if s.closed {
                return Ok(0);
            }
            s = self.0.ready.wait(s).unwrap();
        }
    }
    fn closer(&self) -> Closer {
        let pipe = self.0.clone();
        Arc::new(move || pipe.close())
    }
}
impl World {
    fn device(&mut self, serial: &str) -> io::Result<&mut FakeDevice> {
        self.devices
            .iter_mut()
            .find(|d| d.plugged && d.info.serial == serial)
            .ok_or_else(|| error(link::GONE))
    }
    fn any(&mut self, serial: &str) -> &mut FakeDevice {
        self.devices
            .iter_mut()
            .find(|d| d.info.serial == serial)
            .expect("no such fake device")
    }
}
impl FakeLink {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
    /// A link whose streams send only new rows, like Apple's device service.
    pub fn live() -> Arc<Self> {
        Arc::new(Self {
            live: true,
            ..Self::default()
        })
    }
    /// Plugs in a new ready device.
    pub fn add(&self, serial: &str, model: &str, identity: &str) {
        self.world.lock().unwrap().devices.push(FakeDevice {
            info: Device {
                serial: serial.into(),
                state: "device".into(),
                model: model.into(),
                product: model.into(),
                ..Device::default()
            },
            plugged: true,
            identity: identity.into(),
            boot: 1,
            log: vec![],
            streams: vec![],
            apps: vec![],
            unredacted_pid: None,
        });
    }
    /// Plugs in a new ready iPhone, named by its owner and listed by its
    /// UDID, which is also its identity.
    pub fn add_ios(&self, udid: &str, name: &str, version: &str) {
        self.add(udid, name, udid);
        let mut w = self.world.lock().unwrap();
        let info = &mut w.any(udid).info;
        info.platform = Platform::Ios;
        info.product = if name.contains("iPad") {
            "iPad0,0"
        } else {
            "iPhone0,0"
        }
        .into();
        info.version = version.into();
    }
    /// Installs a developer build on the device.
    pub fn add_app(&self, serial: &str, bundle: &str, name: &str, version: &str) {
        let executable = name.split_whitespace().collect::<String>();
        self.world.lock().unwrap().any(serial).apps.push(App {
            bundle: bundle.into(),
            name: name.into(),
            version: version.into(),
            executable,
        });
    }
    /// Makes listing and starting apps fail with `e`, or work again.
    pub fn apps_error(&self, e: Option<&'static str>) {
        self.world.lock().unwrap().apps_error = e;
    }
    pub fn launch_delay(&self, delay: Duration) {
        self.world.lock().unwrap().launch_delay = delay;
    }
    /// The serial and bundle of each app started, in order.
    pub fn launched(&self) -> Vec<(String, String)> {
        self.world.lock().unwrap().launched.clone()
    }
    pub fn set_state(&self, serial: &str, state: &str) {
        self.world.lock().unwrap().any(serial).info.state = state.into();
    }
    /// Logs bytes: whole rows are kept for later streams, and every open
    /// stream gets the bytes as they are, so a row may arrive in parts.
    pub fn log(&self, serial: &str, bytes: &str) {
        self.log_bytes(serial, bytes.as_bytes());
    }
    pub fn log_bytes(&self, serial: &str, bytes: &[u8]) {
        let mut w = self.world.lock().unwrap();
        let d = w.any(serial);
        // A row in parts joins the previous part until its line ending.
        let mut rest = bytes;
        while !rest.is_empty() {
            let end = rest
                .iter()
                .position(|&b| b == b'\n')
                .map_or(rest.len(), |i| i + 1);
            match d.log.last_mut().filter(|r| !r.ends_with(b"\n")) {
                Some(open) => open.extend_from_slice(&rest[..end]),
                None => d.log.push(rest[..end].to_vec()),
            }
            rest = &rest[end..];
        }
        if d.plugged {
            for s in &d.streams {
                s.send(bytes);
            }
        }
    }
    /// The device leaves: its streams end, but it keeps logging rows and
    /// holds them for the next stream.
    pub fn unplug(&self, serial: &str) {
        let mut w = self.world.lock().unwrap();
        let d = w.any(serial);
        d.plugged = false;
        for s in d.streams.drain(..) {
            s.close();
        }
    }
    pub fn plug(&self, serial: &str) {
        self.world.lock().unwrap().any(serial).plugged = true;
    }
    /// The device starts again with an empty log.
    pub fn reboot(&self, serial: &str) {
        let mut w = self.world.lock().unwrap();
        let d = w.any(serial);
        d.boot += 1;
        d.log.clear();
        for s in d.streams.drain(..) {
            s.close();
        }
    }
    /// The device is now listed under another serial, as wireless
    /// debugging may do.
    pub fn rename(&self, serial: &str, to: &str) {
        self.world.lock().unwrap().any(serial).info.serial = to.into();
    }
    /// Every request fails as when another adb version took the server.
    pub fn conflict(&self, on: bool) {
        self.world.lock().unwrap().conflict = on;
    }
    /// Logs a synthetic row on `serial` once per `interval`, timed as it
    /// is logged, until the returned flag is set.
    pub fn feed(self: &Arc<Self>, serial: &str, interval: Duration) -> Arc<AtomicBool> {
        let stop = Arc::new(AtomicBool::new(false));
        let (link, serial, done) = (self.clone(), serial.to_string(), stop.clone());
        thread::spawn(move || {
            let mut n = 0;
            let ios = link.world.lock().unwrap().any(&serial).info.platform == Platform::Ios;
            while !done.load(Ordering::Acquire) {
                n += 1;
                let row = if ios {
                    let pid = link.world.lock().unwrap().any(&serial).unredacted_pid;
                    ios_row(n, pid)
                } else {
                    synthetic_row(n)
                };
                link.log(&serial, &row);
                thread::sleep(interval);
            }
        });
        stop
    }
    /// The serial and `since` of each stream opened, in order.
    pub fn opened(&self) -> Vec<(String, Option<String>)> {
        self.world.lock().unwrap().opened.clone()
    }
    fn check(&self) -> io::Result<std::sync::MutexGuard<'_, World>> {
        let w = self.world.lock().unwrap();
        if w.conflict {
            return Err(error(link::CONFLICT));
        }
        Ok(w)
    }
}
/// Synthetic `threadtime` row `n` at the current local time.
pub fn synthetic_row(n: u64) -> String {
    const ROWS: [(&str, &str, &str); 8] = [
        (
            "I",
            "ActivityManager",
            "Start proc 4321:com.example.notes/u0a123 for activity",
        ),
        (
            "D",
            "NotesSync",
            "fetched # items from https://example.com/api in 42 ms",
        ),
        ("V", "Choreographer", "frame # rendered in 7 ms"),
        (
            "W",
            "InputDispatcher",
            "channel 'com.example.notes' is slow to respond",
        ),
        ("I", "NotesUi", "opened note #"),
        ("E", "NotesSync", "sync failed: timeout after 5000 ms"),
        ("D", "ConnectivityService", "network 101 validated"),
        ("I", "PowerManager", "screen brightness set to 128"),
    ];
    let (level, tag, message) = ROWS[(n % ROWS.len() as u64) as usize];
    let time = chrono::Local::now().format("%m-%d %H:%M:%S%.3f");
    let tid = 4321 + n % 3;
    format!(
        "{time}  4321  {tid} {level} {tag}: {}\n",
        message.replace('#', &n.to_string())
    )
}
/// Synthetic iPhone row `n` at the current time, as a capture writes it.
pub fn synthetic_ios_row(n: u64) -> String {
    ios_row(n, None)
}
/// Row `n`; SampleApp's rows show their values once it runs as
/// `unredacted_pid`.
fn ios_row(n: u64, unredacted_pid: Option<u32>) -> String {
    const ENTRIES: [(u8, &str, &str, &str); 6] = [
        (0, "SampleApp", "", "[INFO] [Sync] fetched # items in 42 ms"),
        (
            0x02,
            "SampleApp",
            "com.example.sample:ui",
            "layout pass # took 7 ms",
        ),
        (
            0x01,
            "sampled",
            "com.example.location:core",
            "fix accuracy # m",
        ),
        (
            0,
            "SampleApp",
            "",
            "[ERROR] [Sync] request # to <private> failed: timeout after 5000 ms",
        ),
        (
            0x10,
            "samplenetd",
            "com.example.network:task",
            "task # failed: -1001",
        ),
        (
            0x11,
            "SampleApp",
            "com.example.sample:store",
            "store # could not be opened",
        ),
    ];
    let (level, process, label, message) = ENTRIES[(n % ENTRIES.len() as u64) as usize];
    let (subsystem, category) = label.split_once(':').unwrap_or_default();
    let now = chrono::Utc::now();
    let mut message = message.replace('#', &n.to_string());
    let mut pid = 812;
    if let Some(started) = unredacted_pid.filter(|_| process == "SampleApp") {
        pid = started;
        message = message.replace("<private>", "api.example.com");
    }
    let e = ios::Entry {
        pid,
        tid: 4410 + n % 3,
        seconds: now.timestamp() as u32,
        micros: now.timestamp_subsec_micros(),
        level,
        process: process.as_bytes(),
        message: message.as_bytes(),
        subsystem: subsystem.as_bytes(),
        category: category.as_bytes(),
        ..ios::Entry::default()
    };
    let mut out = vec![];
    ios::write_row(&e, &chrono::Local, &mut out);
    String::from_utf8(out).unwrap_or_default()
}
impl Link for FakeLink {
    fn replays(&self) -> bool {
        !self.live
    }
    fn devices(&self) -> io::Result<Vec<Device>> {
        Ok(self
            .check()?
            .devices
            .iter()
            .filter(|d| d.plugged)
            .map(|d| d.info.clone())
            .collect())
    }
    fn boot_id(&self, serial: &str) -> io::Result<String> {
        Ok(format!("boot-{}", self.check()?.device(serial)?.boot))
    }
    fn identity(&self, serial: &str) -> io::Result<String> {
        Ok(self.check()?.device(serial)?.identity.clone())
    }
    fn recent(&self, serial: &str, count: usize) -> io::Result<Vec<u8>> {
        let mut w = self.check()?;
        let log = &w.device(serial)?.log;
        Ok(log[log.len().saturating_sub(count)..].concat())
    }
    fn stream(&self, serial: &str, since: Option<Stamp>) -> io::Result<Box<dyn Stream>> {
        let mut w = self.check()?;
        w.opened.push((serial.into(), since.map(|s| s.arg())));
        let d = w.device(serial)?;
        let pipe = Arc::new(Pipe::default());
        // As `logcat -T`, rows from that time on, then the live rows.
        for row in d.log.iter().filter(|_| !self.live) {
            let from = match (since, Stamp::of(row)) {
                (Some(since), Some(time)) => time.order(&since).is_ge(),
                _ => true,
            };
            if from {
                pipe.send(row);
            }
        }
        d.streams.push(pipe.clone());
        Ok(Box::new(FakeStream(pipe)))
    }
}
impl Apps for FakeLink {
    fn apps(&self, serial: &str) -> io::Result<Vec<App>> {
        let mut w = self.check()?;
        if let Some(e) = w.apps_error {
            return Err(error(e));
        }
        Ok(w.device(serial)?.apps.clone())
    }
    /// Starting SampleApp shows its private values in the rows it logs
    /// from then on, as a new process.
    fn launch(&self, serial: &str, bundle: &str) -> io::Result<()> {
        let delay = self.world.lock().unwrap().launch_delay;
        thread::sleep(delay);
        let mut w = self.check()?;
        if let Some(e) = w.apps_error {
            return Err(error(e));
        }
        let started = w.launched.len() as u32;
        let d = w.device(serial)?;
        let app = d
            .apps
            .iter()
            .find(|a| a.bundle == bundle)
            .ok_or_else(|| error(apps::NOT_LISTED))?;
        if app.executable == "SampleApp" {
            d.unredacted_pid = Some(913 + started);
        }
        w.launched.push((serial.into(), bundle.into()));
        Ok(())
    }
}
