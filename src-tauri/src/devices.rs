//! Device captures for the window: the adb that runs, the Android devices
//! it lists and the iPhones Apple's device service lists, the developer
//! builds an iPhone being recorded has, and the temporary capture files.
//! The page names a device by a serial of the last listing, an iPhone's
//! UDID, an app by a bundle of the last app listing, and a leftover file by
//! an ID; paths stay here.
use logstudio_core::{
    capture::{
        adb::{self, Adb, Origin},
        apps::{self, App, Apps, IosApps},
        cli::AdbCli,
        ios::IosLink,
        link::{Device, Link, Platform},
        store::Store,
        Capture, Cut, Options,
    },
    source::public_message,
};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::UNIX_EPOCH,
};

#[derive(Default)]
pub struct Devices {
    /// `adb.json`, which names the adb the user picked.
    settings: Mutex<Option<PathBuf>>,
    store: Mutex<Option<Arc<Store>>>,
    /// The adb found last.
    adb: Mutex<Option<Adb>>,
    /// The last listing: captures start only on its devices.
    listed: Mutex<Vec<Device>>,
    leftovers: Mutex<Leftovers>,
    /// What QA builds use instead of adb, and instead of Apple's service.
    replaced: Mutex<Option<Replaced>>,
    replaced_ios: Mutex<Option<Replaced>>,
    replaced_apps: Mutex<Option<Arc<dyn Apps>>>,
    /// The serial and apps of the last app listing: only these start.
    listed_apps: Mutex<Option<(String, Vec<App>)>>,
}
/// Leftover files by the IDs the page has, and the ID the next one gets.
#[derive(Default)]
struct Leftovers {
    files: Vec<(u64, PathBuf)>,
    next: u64,
}
/// A scripted link, or none as when no adb or no Apple device service is
/// installed (QA builds; other builds never replace them).
#[derive(Clone)]
#[cfg_attr(not(feature = "qa"), allow(dead_code))]
pub enum Replaced {
    Missing,
    Link(Arc<dyn Link>),
}
/// Why no device can be listed or captured, for the page.
pub const NO_ADB: &str = "adb was not found; install Android SDK Platform-Tools";
const NOT_LISTED: &str = "Device not found; refresh the device list";
fn message(e: std::io::Error) -> String {
    public_message(&e, "adb failed; check the device connection and retry")
}
impl Devices {
    pub fn load(&self, data: &Path, cache: &Path) {
        *self.settings.lock().unwrap() = Some(data.join("adb.json"));
        *self.store.lock().unwrap() = Some(Arc::new(Store::new(cache.join("captures"))));
    }
    fn store(&self) -> Result<Arc<Store>, String> {
        self.store
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| "Capture files are unavailable".into())
    }
    /// The adb the user picked, if any.
    fn chosen(&self) -> Option<PathBuf> {
        let file = self.settings.lock().unwrap().clone()?;
        let stored: Value = serde_json::from_slice(&std::fs::read(file).ok()?).ok()?;
        Some(PathBuf::from(stored["path"].as_str()?)).filter(|p| p.is_absolute())
    }
    /// Remembers the adb the user picked, replacing the file atomically.
    fn choose(&self, path: &Path) -> Result<(), String> {
        let file = self.settings.lock().unwrap().clone();
        let file = file.ok_or("Could not save the adb location")?;
        write_atomically(&file, &json!({"path":path}))
            .map_err(|_| "Could not save the adb location".to_string())
    }
    /// Looks for adb again: the picked one first, then where installers
    /// put it.
    pub fn locate(&self) -> Option<Adb> {
        let chosen = self.chosen();
        let list = adb::candidates(chosen.as_deref(), |k| std::env::var_os(k));
        let found = adb::locate(&list, adb::CHECK_TIMEOUT);
        *self.adb.lock().unwrap() = found.clone();
        found
    }
    /// The adb in use, for the page; `chosenFailed` when the picked file
    /// no longer runs and another adb is used.
    pub fn status(&self) -> Value {
        match self.replaced.lock().unwrap().clone() {
            Some(Replaced::Missing) => return json!({"found":false,"chosenFailed":false}),
            Some(Replaced::Link(_)) => {
                return json!({"found":true,"origin":"studio","version":"37.0.1","chosenFailed":false})
            }
            None => {}
        }
        let picked = self.chosen().is_some();
        match self.locate() {
            Some(adb) => {
                let mut v = adb.json();
                v["found"] = json!(true);
                v["chosenFailed"] = json!(picked && adb.origin != Origin::Chosen);
                v
            }
            None => json!({"found":false,"chosenFailed":picked}),
        }
    }
    /// Uses `path` as adb when it runs as one.
    pub fn pick(&self, path: &Path) -> Result<Value, String> {
        if adb::check(path, adb::CHECK_TIMEOUT).is_none() {
            return Err("The selected file is not adb".into());
        }
        self.choose(path)?;
        Ok(self.status())
    }
    /// Uses a scripted link, or none, instead of adb (QA builds).
    #[cfg_attr(not(feature = "qa"), allow(dead_code))]
    pub fn replace(&self, with: Option<Replaced>) {
        *self.replaced.lock().unwrap() = with;
    }
    /// Uses a scripted link, or none, instead of Apple's device service.
    #[cfg_attr(not(feature = "qa"), allow(dead_code))]
    pub fn replace_ios(&self, with: Option<Replaced>) {
        *self.replaced_ios.lock().unwrap() = with;
    }
    /// Uses scripted developer builds instead of the device's (QA builds).
    #[cfg_attr(not(feature = "qa"), allow(dead_code))]
    pub fn replace_apps(&self, with: Option<Arc<dyn Apps>>) {
        *self.replaced_apps.lock().unwrap() = with;
    }
    fn launcher(&self) -> Arc<dyn Apps> {
        let replaced = self.replaced_apps.lock().unwrap().clone();
        replaced.unwrap_or_else(|| Arc::new(IosApps::default()))
    }
    /// The developer builds on the iPhone recorded under `serial`.
    pub fn apps(&self, serial: &str) -> Result<Value, String> {
        let found = self.launcher().apps(serial).map_err(|e| {
            public_message(
                &e,
                "Could not list the apps; check the connection and retry",
            )
        })?;
        let page: Vec<Value> = found.iter().map(App::json).collect();
        *self.listed_apps.lock().unwrap() = Some((serial.into(), found));
        Ok(json!(page))
    }
    /// Starts an app of the last listing, with its private values shown.
    pub fn launch(&self, serial: &str, bundle: &str) -> Result<App, String> {
        let app = self
            .listed_apps
            .lock()
            .unwrap()
            .as_ref()
            .filter(|(listed, _)| listed == serial)
            .and_then(|(_, found)| found.iter().find(|a| a.bundle == bundle).cloned())
            .ok_or(apps::NOT_LISTED)?;
        self.launcher()
            .launch(serial, &app.bundle)
            .map_err(|e| public_message(&e, apps::LAUNCH_FAILED))?;
        Ok(app)
    }
    fn ios_link(&self) -> Result<Arc<dyn Link>, String> {
        match self.replaced_ios.lock().unwrap().clone() {
            Some(Replaced::Missing) => Err(logstudio_core::capture::ios::NO_SERVICE.into()),
            Some(Replaced::Link(link)) => Ok(link),
            None => Ok(Arc::new(IosLink::default())),
        }
    }
    fn link(&self) -> Result<Arc<dyn Link>, String> {
        match self.replaced.lock().unwrap().clone() {
            Some(Replaced::Missing) => return Err(NO_ADB.into()),
            Some(Replaced::Link(link)) => return Ok(link),
            None => {}
        }
        let known = self.adb.lock().unwrap().clone();
        let adb = known.or_else(|| self.locate()).ok_or(NO_ADB)?;
        Ok(Arc::new(AdbCli::new(&adb.path)))
    }
    /// The Android devices adb lists now.
    fn android(&self) -> Result<Vec<Device>, String> {
        match self.link()?.devices() {
            Ok(devices) => Ok(devices),
            // A moved or removed adb is looked for again once.
            Err(_) if self.replaced.lock().unwrap().is_none() && self.locate().is_some() => {
                self.link()?.devices().map_err(message)
            }
            Err(e) => Err(message(e)),
        }
    }
    /// The iPhones and iPads Apple's device service lists now.
    fn ios(&self) -> Result<Vec<Device>, String> {
        self.ios_link()?.devices().map_err(|e| {
            public_message(
                &e,
                "Could not list iPhones or iPads; check the connection and retry",
            )
        })
    }
    /// The devices both connections list now; captures may start on them.
    /// Each connection that cannot list says why.
    pub fn list(&self) -> Value {
        let (android, ios) = std::thread::scope(|s| {
            let ios = s.spawn(|| self.ios());
            (self.android(), ios.join().unwrap_or_else(|_| Ok(vec![])))
        });
        let mut devices = vec![];
        let mut why = |found: Result<Vec<Device>, String>| match found {
            Ok(found) => {
                devices.extend(found);
                Value::Null
            }
            Err(e) => json!(e),
        };
        let (android_error, ios_error) = (why(android), why(ios));
        let page: Vec<Value> = devices.iter().map(Device::json).collect();
        *self.listed.lock().unwrap() = devices;
        json!({"devices":page,"androidError":android_error,"iosError":ios_error})
    }
    /// Starts capturing a listed device into a new temporary file.
    pub fn start(&self, serial: &str) -> Result<Arc<Capture>, String> {
        let device = self
            .listed
            .lock()
            .unwrap()
            .iter()
            .find(|d| d.serial == serial)
            .cloned()
            .ok_or(NOT_LISTED)?;
        let link = match device.platform {
            Platform::Android => self.link()?,
            Platform::Ios => self.ios_link()?,
        };
        let slot = self
            .store()?
            .create(device.platform, device.name())
            .map_err(message)?;
        Capture::start(link, serial, slot, Options::default()).map_err(message)
    }
    /// A new file for a capture to record into once cleared.
    pub fn cut(&self, capture: &Capture) -> Result<Cut, String> {
        let slot = self
            .store()?
            .create(capture.platform, &capture.model)
            .map_err(message)?;
        Cut::open(slot).map_err(message)
    }
    /// Capture files left by earlier runs, under IDs for the page.
    pub fn leftovers(&self) -> Value {
        let Ok(store) = self.store() else {
            return json!([]);
        };
        let found = store.leftovers();
        let mut known = self.leftovers.lock().unwrap();
        known.files.clear();
        let list: Vec<Value> = found
            .into_iter()
            .map(|l| {
                known.next += 1;
                let id = known.next;
                known.files.push((id, l.path));
                let modified = l
                    .modified
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as u64);
                json!({"id":id,"name":l.name,"size":l.size,"modified":modified})
            })
            .collect();
        json!(list)
    }
    fn leftover(&self, id: u64) -> Result<PathBuf, String> {
        let known = self.leftovers.lock().unwrap();
        known
            .files
            .iter()
            .find(|(k, _)| *k == id)
            .map(|(_, p)| p.clone())
            .ok_or_else(|| "The capture file is gone".into())
    }
    /// A leftover held again, to be shown as a stopped capture.
    pub fn adopt(&self, id: u64) -> Result<Arc<Capture>, String> {
        let path = self.leftover(id)?;
        let slot = self.store()?.adopt(&path).map_err(message)?;
        Capture::adopt(slot).map_err(message)
    }
    /// Deletes a leftover; returns the leftovers that remain.
    pub fn remove(&self, id: u64) -> Result<Value, String> {
        let path = self.leftover(id)?;
        self.store()?
            .remove(&path)
            .map_err(|_| "Could not delete the capture file".to_string())?;
        Ok(self.leftovers())
    }
}
/// Writes `value` as JSON to `file` through a partial file beside it, so a
/// reader never sees half of it.
fn write_atomically(file: &Path, value: &Value) -> std::io::Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let partial = file.with_extension("json.partial");
    std::fs::write(&partial, serde_json::to_vec(value)?)?;
    std::fs::rename(&partial, file)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn devices(dir: &Path) -> Devices {
        let d = Devices::default();
        d.load(&dir.join("data"), &dir.join("cache"));
        d
    }

    #[test]
    fn a_picked_adb_is_kept_only_when_it_runs_as_one() {
        let dir = tempfile::tempdir().unwrap();
        let d = devices(dir.path());
        assert_eq!(d.chosen(), None);
        let other = dir.path().join("not-adb");
        std::fs::write(&other, b"text").unwrap();
        assert!(d.pick(&other).is_err());
        assert_eq!(d.chosen(), None);
        // A temporary folder is an absolute path on every system.
        let adb = dir.path().join("sdk/platform-tools/adb");
        d.choose(&adb).unwrap();
        assert_eq!(d.chosen(), Some(adb));
        // Another app replaced the file meanwhile, or it is malformed.
        std::fs::write(
            dir.path().join("data/adb.json"),
            b"{\"path\":\"relative/adb\"}",
        )
        .unwrap();
        assert_eq!(d.chosen(), None);
        std::fs::write(dir.path().join("data/adb.json"), b"[").unwrap();
        assert_eq!(d.chosen(), None);
    }

    #[test]
    fn android_devices_and_iphones_are_listed_together() {
        use logstudio_core::capture::{fake::FakeLink, ios::NO_SERVICE};
        let dir = tempfile::tempdir().unwrap();
        let d = devices(dir.path());
        let android = FakeLink::new();
        android.add("emulator-5554", "Pixel_8", "QA-PIXEL-8");
        let ios = FakeLink::live();
        ios.add_ios("QA0UDID0000000000000001", "QA iPhone", "27.0");
        d.replace(Some(Replaced::Link(android)));
        d.replace_ios(Some(Replaced::Link(ios)));
        let list = d.list();
        let platforms: Vec<&str> = list["devices"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["platform"].as_str().unwrap())
            .collect();
        assert_eq!(platforms, ["android", "ios"]);
        assert_eq!(
            (&list["androidError"], &list["iosError"]),
            (&Value::Null, &Value::Null)
        );
        let capture = d.start("QA0UDID0000000000000001").unwrap();
        let name = capture
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert!(name.starts_with("ios-QA_iPhone-"), "{name}");
        capture.close();
        // A connection that cannot list leaves the other's devices.
        d.replace(Some(Replaced::Missing));
        d.replace_ios(Some(Replaced::Missing));
        let list = d.list();
        assert_eq!(list["devices"], json!([]));
        assert_eq!(list["androidError"], NO_ADB);
        assert_eq!(list["iosError"], NO_SERVICE);
        assert_eq!(
            d.start("QA0UDID0000000000000001").err().as_deref(),
            Some(NOT_LISTED)
        );
    }

    #[test]
    fn only_listed_apps_of_the_listed_device_start() {
        use logstudio_core::capture::{apps::NO_XCODE, fake::FakeLink};
        const IPHONE: &str = "QA0UDID0000000000000001";
        let dir = tempfile::tempdir().unwrap();
        let d = devices(dir.path());
        let link = FakeLink::live();
        link.add_ios(IPHONE, "QA iPhone", "27.0");
        link.add_ios("QA0UDID0000000000000002", "QA iPad", "27.0");
        link.add_app(IPHONE, "com.example.sample", "SampleApp", "1.4.0");
        d.replace_apps(Some(link.clone()));
        // Nothing was listed yet.
        let not_listed = Some(apps::NOT_LISTED);
        assert_eq!(
            d.launch(IPHONE, "com.example.sample").err().as_deref(),
            not_listed
        );
        assert_eq!(
            d.apps(IPHONE).unwrap(),
            json!([{"bundle":"com.example.sample","name":"SampleApp","version":"1.4.0","executable":"SampleApp"}])
        );
        assert_eq!(
            d.launch(IPHONE, "com.example.other").err().as_deref(),
            not_listed
        );
        let other = "QA0UDID0000000000000002";
        assert_eq!(
            d.launch(other, "com.example.sample").err().as_deref(),
            not_listed
        );
        let started = d.launch(IPHONE, "com.example.sample").unwrap();
        assert_eq!(
            (started.name.as_str(), started.executable.as_str()),
            ("SampleApp", "SampleApp")
        );
        assert_eq!(
            link.launched(),
            [(IPHONE.to_string(), "com.example.sample".to_string())]
        );
        // Listing another device forgets the first one's apps.
        assert_eq!(d.apps(other).unwrap(), json!([]));
        assert_eq!(
            d.launch(IPHONE, "com.example.sample").err().as_deref(),
            not_listed
        );
        link.apps_error(Some(NO_XCODE));
        assert_eq!(d.apps(IPHONE).err().as_deref(), Some(NO_XCODE));
    }

    #[test]
    fn captures_start_only_on_listed_devices() {
        let dir = tempfile::tempdir().unwrap();
        let d = devices(dir.path());
        assert_eq!(d.start("emulator-5554").err().as_deref(), Some(NOT_LISTED));
    }

    #[test]
    fn leftovers_are_named_by_ids_and_deleted_by_them() {
        let dir = tempfile::tempdir().unwrap();
        let d = devices(dir.path());
        assert_eq!(d.leftovers(), json!([]));
        let store = d.store().unwrap();
        let slot = store.create(Platform::Android, "Pixel 8").unwrap();
        std::fs::write(&slot.path, b"row\n").unwrap();
        let path = slot.path.clone();
        drop(slot);
        let list = d.leftovers();
        assert_eq!(list.as_array().unwrap().len(), 1);
        assert_eq!(list[0]["size"], 4);
        assert!(list[0].get("path").is_none());
        let id = list[0]["id"].as_u64().unwrap();
        // Shown again, it is held and no leftover.
        let again = d.adopt(id).unwrap();
        assert_eq!(d.leftovers(), json!([]));
        assert!(d.remove(id).is_err());
        drop(again);
        let id = d.leftovers()[0]["id"].as_u64().unwrap();
        assert_eq!(d.remove(id).unwrap(), json!([]));
        assert!(!path.exists());
        assert!(d.remove(id).is_err());
    }
}
