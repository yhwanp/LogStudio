//! The adb command line as a capture's connection. Each request runs one
//! fixed adb command; the serial is passed as an argument of its own.
use super::{
    adb::{command, run, Output},
    gate::Stamp,
    link::{self, parse_devices, Closer, Device, Link, Stream},
};
use crate::source::error;
use std::{
    io::{self, Read},
    path::{Path, PathBuf},
    process::{Child, ChildStdout, Stdio},
    sync::{Arc, Mutex},
    time::Duration,
};

/// Starting the adb server takes seconds; a device query should not.
const SERVER_TIMEOUT: Duration = Duration::from_secs(20);
const QUERY_TIMEOUT: Duration = Duration::from_secs(10);
const OUTPUT_LIMIT: u64 = 16 * 1024 * 1024;
/// What adb prints when it replaces a server of another version.
const MISMATCH: &str = "doesn't match this client";
pub struct AdbCli {
    adb: PathBuf,
}
/// Why adb failed for a device, by what it printed.
fn device_error(stderr: &str) -> io::Error {
    let s = stderr.to_ascii_lowercase();
    if s.contains(MISMATCH) {
        error(link::CONFLICT)
    } else if s.contains("unauthorized") {
        error(link::UNAUTHORIZED)
    } else if s.contains("offline") {
        error(link::OFFLINE)
    } else if s.contains("not found") || s.contains("no devices") {
        error(link::GONE)
    } else {
        io::Error::other(format!("adb failed: {}", stderr.trim()))
    }
}
impl AdbCli {
    pub fn new(adb: &Path) -> Self {
        Self { adb: adb.into() }
    }
    fn run(&self, args: &[&str], timeout: Duration) -> io::Result<Output> {
        let mut c = command(&self.adb);
        c.args(args);
        run(c, timeout, OUTPUT_LIMIT)
    }
    /// Starts the adb server, when it is not running, before a stream does:
    /// a server that a stream's command started would keep the stream's
    /// pipe open. A server of another adb version shows up here as a
    /// conflict.
    fn serve(&self) -> io::Result<()> {
        let out = self.run(&["start-server"], SERVER_TIMEOUT)?;
        if out.stderr.contains(MISMATCH) {
            return Err(error(link::CONFLICT));
        }
        if !out.success {
            return Err(io::Error::other("adb server did not start"));
        }
        Ok(())
    }
    /// Runs adb for one device and returns what it printed.
    fn device(&self, serial: &str, args: &[&str], timeout: Duration) -> io::Result<Vec<u8>> {
        let mut all = vec!["-s", serial];
        all.extend_from_slice(args);
        let out = self.run(&all, timeout)?;
        if !out.success || out.stderr.contains(MISMATCH) {
            return Err(device_error(&out.stderr));
        }
        Ok(out.stdout)
    }
    fn text(&self, serial: &str, args: &[&str]) -> io::Result<String> {
        let out = self.device(serial, args, QUERY_TIMEOUT)?;
        Ok(String::from_utf8_lossy(&out).trim().to_string())
    }
}
struct CliStream {
    stdout: ChildStdout,
    child: Arc<Mutex<Child>>,
}
impl Stream for CliStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.stdout.read(buf)
    }
    fn closer(&self) -> Closer {
        let child = self.child.clone();
        Arc::new(move || {
            let _ = child.lock().unwrap().kill();
        })
    }
}
impl Drop for CliStream {
    fn drop(&mut self) {
        let mut child = self.child.lock().unwrap();
        let _ = child.kill();
        let _ = child.wait();
    }
}
impl Link for AdbCli {
    fn devices(&self) -> io::Result<Vec<Device>> {
        self.serve()?;
        let out = self.run(&["devices", "-l"], QUERY_TIMEOUT)?;
        if !out.success || out.stderr.contains(MISMATCH) {
            return Err(device_error(&out.stderr));
        }
        Ok(parse_devices(&String::from_utf8_lossy(&out.stdout)))
    }
    fn boot_id(&self, serial: &str) -> io::Result<String> {
        self.text(serial, &["shell", "cat /proc/sys/kernel/random/boot_id"])
    }
    fn identity(&self, serial: &str) -> io::Result<String> {
        self.text(serial, &["shell", "getprop ro.serialno"])
    }
    fn recent(&self, serial: &str, count: usize) -> io::Result<Vec<u8>> {
        let count = count.to_string();
        let args = ["logcat", "-d", "-v", "threadtime", "-t", &count];
        self.device(serial, &args, QUERY_TIMEOUT)
    }
    fn stream(&self, serial: &str, since: Option<Stamp>) -> io::Result<Box<dyn Stream>> {
        let mut c = command(&self.adb);
        c.args(["-s", serial, "logcat", "-v", "threadtime"]);
        if let Some(since) = since {
            c.args(["-T", &since.arg()]);
        }
        let mut child = c.stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("adb has no output"))?;
        Ok(Box::new(CliStream {
            stdout,
            child: Arc::new(Mutex::new(child)),
        }))
    }
}
