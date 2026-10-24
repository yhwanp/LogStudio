//! A scripted adb connection for tests, measurements and QA builds:
//! devices with logs that can be written, unplugged, plugged again and
//! rebooted, as `logcat` would report them.
use super::{
    gate::Stamp,
    link::{self, Closer, Device, Link, Stream},
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
}
#[derive(Default)]
struct World {
    devices: Vec<FakeDevice>,
    conflict: bool,
    /// `since` of each stream opened, in order.
    opened: Vec<(String, Option<String>)>,
}
struct FakeDevice {
    info: Device,
    plugged: bool,
    identity: String,
    boot: u64,
    /// Rows the device holds, each with its line ending.
    log: Vec<Vec<u8>>,
    streams: Vec<Arc<Pipe>>,
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
    /// Plugs in a new ready device.
    pub fn add(&self, serial: &str, model: &str, identity: &str) {
        self.world.lock().unwrap().devices.push(FakeDevice {
            info: Device {
                serial: serial.into(),
                state: "device".into(),
                model: model.into(),
                product: model.into(),
            },
            plugged: true,
            identity: identity.into(),
            boot: 1,
            log: vec![],
            streams: vec![],
        });
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
            while !done.load(Ordering::Acquire) {
                n += 1;
                link.log(&serial, &synthetic_row(n));
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
impl Link for FakeLink {
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
        for row in &d.log {
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
