//! What a capture needs of a device connection: adb for Android devices,
//! Apple's device service for iPhones and iPads, and Xcode's simulator tool
//! for iOS simulators. Each request is a fixed device query, so no
//! connection runs commands the page chose.
use super::gate::Stamp;
use serde_json::{json, Value};
use std::{
    io,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
};

/// The device left the connection, or the connection names none such.
pub const GONE: &str = "Device not found; connect it and retry";
pub const UNAUTHORIZED: &str = "Allow USB debugging on the device, then retry";
pub const OFFLINE: &str = "Device is offline; reconnect it and retry";
pub const NOT_READY: &str = "Device is not ready for logs; check its connection";
/// adbs of two versions replace each other's server, as adb does when
/// versions differ; reconnecting would go on replacing it.
pub const CONFLICT: &str =
    "Two adb versions are replacing each other's server; use one adb for all tools";
/// An iPhone or iPad that has not trusted this computer, or no longer does.
pub const UNTRUSTED: &str = "Trust this computer on the device, then retry";
/// An iPhone or iPad that has stayed locked since it started.
pub const LOCKED: &str = "Unlock the device, then retry";
/// An iOS simulator that is not running.
pub const SHUTDOWN: &str = "Start the simulator, then retry";
/// An iOS simulator that is starting or changing state.
pub const BOOTING: &str = "The simulator is starting; wait for it, then retry";
/// What an emulator's `ro.serialno` starts with, and its `identity`.
pub const EMULATOR: &str = "EMULATOR";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Platform {
    #[default]
    Android,
    Ios,
}
impl Platform {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Android => "android",
            Self::Ios => "ios",
        }
    }
}

/// One device as its connection lists it: adb as `adb devices -l` does,
/// or Apple's device service, which names an iPhone by its UDID.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Device {
    pub serial: String,
    /// `device` when ready; also `unauthorized`, `offline`, `no permissions`
    /// and others from adb, `untrusted` or `locked` for iPhones, and
    /// `shutdown` or `booting` for simulators.
    pub state: String,
    /// adb's model, such as `Pixel_8`, or the name an iPhone's owner gave it.
    pub model: String,
    /// adb's product, or an iPhone's product type (`iPhoneN,M`), which
    /// names its model.
    pub product: String,
    pub platform: Platform,
    /// The iOS version; empty for Android devices.
    pub version: String,
    /// An iOS simulator on this computer, whose rows are an iPhone's.
    pub simulator: bool,
}
impl Device {
    /// What names the device as adb writes it: its model, else its serial.
    pub fn name(&self) -> &str {
        if self.model.is_empty() {
            &self.serial
        } else {
            &self.model
        }
    }
    /// The name as people read it: adb's model `Pixel_8` reads `Pixel 8`. An
    /// iPhone's name, which its owner chose, stays as it is.
    pub fn label(&self) -> String {
        if self.model.is_empty() {
            self.serial.clone()
        } else if self.platform == Platform::Ios {
            self.model.clone()
        } else {
            self.model.replace('_', " ")
        }
    }
    /// Whether both are of one model, as far as they say it.
    pub fn alike(&self, other: &Device) -> bool {
        let same = |a: &str, b: &str| a == b || a.is_empty() || b.is_empty();
        same(&self.model, &other.model) && same(&self.product, &other.product)
    }
    pub fn ready(&self) -> bool {
        self.state == "device"
    }
    /// Why a capture cannot start on the device; None when it can.
    pub fn problem(&self) -> Option<&'static str> {
        match self.state.as_str() {
            "device" => None,
            "unauthorized" | "authorizing" => Some(UNAUTHORIZED),
            "offline" | "connecting" => Some(OFFLINE),
            "untrusted" => Some(UNTRUSTED),
            "locked" => Some(LOCKED),
            "shutdown" => Some(SHUTDOWN),
            "booting" => Some(BOOTING),
            _ => Some(NOT_READY),
        }
    }
    pub fn json(&self) -> Value {
        json!({
            "serial": self.serial,
            "state": self.state,
            "model": self.model,
            "product": self.product,
            "platform": self.platform.as_str(),
            "version": self.version,
            "simulator": self.simulator,
            "ready": self.ready(),
        })
    }
}
/// Parses `adb devices -l` output, which `host:devices-l` also returns
/// without its heading.
pub fn parse_devices(text: &str) -> Vec<Device> {
    text.lines()
        .filter(|l| !l.starts_with("List of devices") && !l.starts_with("* "))
        .filter_map(|line| {
            let (serial, rest) = line.trim_end().split_once(|c: char| c.is_whitespace())?;
            if serial.is_empty() || serial.chars().any(char::is_control) {
                return None;
            }
            let rest = rest.trim_start();
            // The one state with spaces explains itself at length.
            let state = if rest.starts_with("no permissions") {
                "no permissions"
            } else {
                rest.split_whitespace().next()?
            };
            let field = |key: &str| {
                rest.split_whitespace()
                    .find_map(|t| t.strip_prefix(key)?.strip_prefix(':'))
                    .unwrap_or("")
                    .to_string()
            };
            Some(Device {
                serial: serial.to_string(),
                state: state.to_string(),
                model: field("model"),
                product: field("product"),
                ..Device::default()
            })
        })
        .collect()
}
/// Ends a capture's running request early: while the capture is stopped,
/// and while a stop waits for a resume, which the resume cannot undo.
#[derive(Default)]
pub struct Cancel {
    stopped: AtomicBool,
    waiting: AtomicUsize,
}
impl Cancel {
    pub fn is_set(&self) -> bool {
        self.waiting.load(Ordering::Acquire) > 0 || self.stopped.load(Ordering::Acquire)
    }
    pub(super) fn set_stopped(&self, on: bool) {
        self.stopped.store(on, Ordering::Release);
    }
    #[cfg(test)]
    pub(super) fn is_waited(&self) -> bool {
        self.waiting.load(Ordering::Acquire) > 0
    }
    /// Keeps this set while a stop waits, until the returned guard drops.
    pub(super) fn wait(&self) -> Waiting<'_> {
        self.waiting.fetch_add(1, Ordering::AcqRel);
        Waiting(self)
    }
}
pub(super) struct Waiting<'a>(&'a Cancel);
impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        self.0.waiting.fetch_sub(1, Ordering::AcqRel);
    }
}
/// Ends a stream's blocked read from another thread.
pub type Closer = Arc<dyn Fn() + Send + Sync>;
/// A running log stream's output: `logcat`'s, or an iPhone's log as text.
pub trait Stream: Send {
    /// Bytes as they come; 0 once the stream ended.
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize>;
    fn closer(&self) -> Closer;
}
/// A device connection. Errors with the public messages above say why a
/// device cannot be used; others are failures to retry.
pub trait Link: Send + Sync {
    /// Whether a stream sends rows logged before it opened, as `logcat`
    /// does. Streams that send only new rows need no boundary, and rows
    /// logged while the device was away are lost.
    fn replays(&self) -> bool {
        true
    }
    /// This connection with requests that end early once `cancel` is set;
    /// None when its requests end soon anyway.
    fn cancelled_by(&self, _cancel: Arc<Cancel>) -> Option<Arc<dyn Link>> {
        None
    }
    fn devices(&self) -> io::Result<Vec<Device>>;
    /// `/proc/sys/kernel/random/boot_id`, which changes when it boots.
    fn boot_id(&self, serial: &str) -> io::Result<String>;
    /// `ro.serialno`, which names the device however it is connected, or
    /// `EMULATOR:` and the name of an emulator's virtual device. Development
    /// boards may share one.
    fn identity(&self, serial: &str) -> io::Result<String>;
    /// `logcat -d -v threadtime -t <count>`: the last rows it holds now.
    fn recent(&self, serial: &str, count: usize) -> io::Result<Vec<u8>>;
    /// `logcat -v threadtime [-T <since>]`: the rows from `since`, or all it
    /// holds, then rows as they are logged.
    fn stream(&self, serial: &str, since: Option<Stamp>) -> io::Result<Box<dyn Stream>>;
}
