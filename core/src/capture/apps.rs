//! Developer builds on an iPhone or iPad, started so that their log keeps
//! the values it would otherwise send as `<private>`. An app started with
//! `OS_ACTIVITY_DT_MODE` set, as Xcode starts the app it debugs, sends its
//! own entries unredacted; other processes stay redacted, and rows already
//! recorded stay as they were. Apple calls the variable an implementation
//! detail, so an iOS release may end this.
//!
//! Apps are listed through Apple's device service (`installation_proxy`),
//! over USB or Wi-Fi alike. Only apps signed with `get-task-allow`, which
//! development signing grants, are listed: `devicectl`'s `builtByDeveloper`
//! also marks App Store apps. They are started through Xcode's `devicectl`,
//! as a fixed command with the device and app as arguments of their own:
//! since iOS 17, starting an app takes a CoreDevice tunnel, the pairing
//! Xcode keeps and the developer disk image Xcode mounts, none of which this
//! app makes.
use super::{
    adb::run,
    ios::{self, IosLink},
    link,
};
use crate::source::error;
use idevice::{lockdown::LockdownClient, provider::IdeviceProvider, Idevice, IdeviceService};
use serde_json::{json, Value};
use std::{
    ffi::OsString,
    io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

/// Starting apps needs Xcode, which only macOS has.
pub const NO_XCODE: &str = "Starting an app needs Xcode; install it from the App Store";
/// Xcode's first-launch setup has not run.
pub const XCODE_SETUP: &str = "Open Xcode once to finish installing its components, then retry";
/// The device service reaches the device, but Xcode does not: on Wi-Fi it
/// was never connected in Xcode, for one.
pub const NOT_IN_XCODE: &str =
    "Xcode cannot reach the device; connect it in Xcode's Devices and Simulators window, then retry";
/// Leads `devicectl`'s own reason why the app did not start.
pub const LAUNCH_FAILED: &str = "Could not start the app";
pub const NOT_LISTED: &str = "App not found; open the app list again";
/// `devicectl` did not answer within `LAUNCH_TIMEOUT`.
pub const LAUNCH_TIMED_OUT: &str =
    "Xcode did not start the app within 90 s; check the device and retry";
/// `devicectl` itself could not be run.
pub const DEVICECTL_FAILED: &str = "Could not run Xcode's devicectl; check the Xcode installation";
const ENVIRONMENT: &str = r#"{"OS_ACTIVITY_DT_MODE":"1"}"#;
const INSTALLATION_PROXY: &str = "com.apple.mobile.installation_proxy";
const ATTRIBUTES: [&str; 6] = [
    "CFBundleIdentifier",
    "CFBundleDisplayName",
    "CFBundleName",
    "CFBundleShortVersionString",
    "CFBundleExecutable",
    "Entitlements",
];
/// Larger than any reply of the app list: the stream is corrupt.
const MAX_REPLY: usize = 16 << 20;
/// A start took 0.5 s on an iPhone over Wi-Fi. The first start after the
/// device boots also mounts Xcode's disk image, which was not measured.
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(90);
const _: () = assert!(
    LAUNCH_TIMEOUT.as_secs() == 90,
    "LAUNCH_TIMED_OUT names 90 s"
);
const QUERY_TIMEOUT: Duration = Duration::from_secs(10);

/// A developer build on a device.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct App {
    pub bundle: String,
    /// The name its home screen shows.
    pub name: String,
    pub version: String,
    /// The executable, which names the app's process in its rows.
    pub executable: String,
}
impl App {
    pub fn json(&self) -> Value {
        json!({"bundle":self.bundle,"name":self.name,"version":self.version,"executable":self.executable})
    }
}
/// A device's developer builds, and starting one. Errors with the public
/// messages above, or of `link`, say why an app cannot be listed or started.
pub trait Apps: Send + Sync {
    /// The developer builds on the device, by name.
    fn apps(&self, serial: &str) -> io::Result<Vec<App>>;
    /// Starts `bundle` with `OS_ACTIVITY_DT_MODE` set, ending a running copy
    /// first.
    fn launch(&self, serial: &str, bundle: &str) -> io::Result<()>;
}

/// Whether `s` is a bundle ID: Apple's letters, digits, `-` and `.`, plus
/// the `_` older apps have. It never starts with `-` or `.`, so `devicectl`
/// cannot read it as an option.
pub fn valid_bundle_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 255
        && !s.starts_with(['-', '.'])
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_'))
}
/// The developer builds among `installation_proxy`'s entries, by name.
pub fn developer_apps(list: &[plist::Value]) -> Vec<App> {
    let mut apps: Vec<App> = list
        .iter()
        .filter_map(|entry| {
            let d = entry.as_dictionary()?;
            let text = |key: &str| d.get(key).and_then(plist::Value::as_string).unwrap_or("");
            let debuggable = d
                .get("Entitlements")
                .and_then(plist::Value::as_dictionary)
                .and_then(|e| e.get("get-task-allow"))
                .and_then(plist::Value::as_boolean);
            let bundle = text("CFBundleIdentifier");
            if debuggable != Some(true) || !valid_bundle_id(bundle) {
                return None;
            }
            let name = [text("CFBundleDisplayName"), text("CFBundleName"), bundle]
                .into_iter()
                .find(|n| !n.trim().is_empty())?;
            Some(App {
                bundle: bundle.into(),
                name: name.trim().into(),
                version: text("CFBundleShortVersionString").into(),
                executable: text("CFBundleExecutable").into(),
            })
        })
        .collect();
    apps.sort_by_cached_key(|a| (a.name.to_lowercase(), a.bundle.clone()));
    apps
}

/// Sends one property list as lockdown services read it: its length, then
/// the list.
async fn send(device: &mut Idevice, message: plist::Value) -> io::Result<()> {
    let mut body = vec![];
    message
        .to_writer_xml(&mut body)
        .map_err(|_| error(ios::FAILED))?;
    let mut framed = (body.len() as u32).to_be_bytes().to_vec();
    framed.extend_from_slice(&body);
    device.send_raw(&framed).await.map_err(ios::io_error)
}
async fn receive(device: &mut Idevice) -> io::Result<plist::Dictionary> {
    let len = device.read_raw(4).await.map_err(ios::io_error)?;
    let len = u32::from_be_bytes([len[0], len[1], len[2], len[3]]) as usize;
    if len > MAX_REPLY {
        return Err(error(ios::FAILED));
    }
    let body = device.read_raw(len).await.map_err(ios::io_error)?;
    plist::from_bytes(&body).map_err(|_| error(ios::FAILED))
}
/// The user apps' entries, with `ATTRIBUTES`, in the parts the service
/// sends them.
async fn browse(link: &IosLink, serial: &str) -> io::Result<Vec<plist::Value>> {
    let d = link
        .listed()
        .await?
        .into_iter()
        .find(|d| d.udid == serial)
        .ok_or_else(|| error(link::GONE))?;
    let provider = d.to_provider(link.addr.clone(), ios::LABEL);
    let mut lockdown = LockdownClient::connect(&provider)
        .await
        .map_err(ios::io_error)?;
    let pair = ios::session(&provider, &mut lockdown).await?;
    let (port, tls) = lockdown
        .start_service(INSTALLATION_PROXY)
        .await
        .map_err(ios::io_error)?;
    let mut device = provider.connect(port).await.map_err(ios::io_error)?;
    if tls {
        device
            .start_session(&pair, false)
            .await
            .map_err(ios::io_error)?;
    }
    let mut options = plist::Dictionary::new();
    options.insert("ApplicationType".into(), "User".into());
    let attributes = ATTRIBUTES.iter().map(|&a| a.into()).collect();
    options.insert("ReturnAttributes".into(), plist::Value::Array(attributes));
    let mut ask = plist::Dictionary::new();
    ask.insert("Command".into(), "Browse".into());
    ask.insert("ClientOptions".into(), options.into());
    send(&mut device, ask.into()).await?;
    let mut entries = vec![];
    loop {
        let reply = receive(&mut device).await?;
        let list = reply.get("CurrentList").and_then(plist::Value::as_array);
        if let Some(list) = list {
            entries.extend(list.iter().cloned());
        }
        match reply.get("Status").and_then(plist::Value::as_string) {
            Some("Complete") => return Ok(entries),
            // An error, or a reply that would never end the list.
            _ if list.is_none() => return Err(error(ios::FAILED)),
            _ => {}
        }
    }
}

/// The developer folders to look in, in order: `DEVELOPER_DIR`, which
/// `xcrun` honors too, the one `xcode-select` chose, then Xcode in
/// Applications, as when `xcode-select` chose the Command Line Tools.
pub fn developer_dirs(env: Option<OsString>, selected: &str) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = vec![];
    let mut add = |dir: PathBuf| {
        // `xcode-select` and `DEVELOPER_DIR` also take Xcode's own folder.
        let dir = if dir.extension().is_some_and(|e| e == "app") {
            dir.join("Contents/Developer")
        } else {
            dir
        };
        if dir.is_absolute() && !dirs.contains(&dir) {
            dirs.push(dir);
        }
    };
    if let Some(dir) = env.filter(|v| !v.is_empty()) {
        add(dir.into());
    }
    if !selected.trim().is_empty() {
        add(PathBuf::from(selected.trim()));
    }
    add("/Applications/Xcode.app/Contents/Developer".into());
    dirs
}
/// A text of a `devicectl` error's `userInfo`; None when empty.
fn info<'a>(e: &'a Value, key: &str) -> Option<&'a str> {
    e["userInfo"][key]["string"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
}
/// Whether `devicectl` started the app, from its JSON output, and why not.
/// Its error names the step that failed; the cause, such as a locked
/// device, is in the errors under it.
pub fn launch_outcome(output: &[u8]) -> io::Result<()> {
    let Ok(v) = serde_json::from_slice::<Value>(output) else {
        return Err(error(LAUNCH_FAILED));
    };
    if v["info"]["outcome"] == "success" {
        return Ok(());
    }
    let e = &v["error"];
    if e["domain"] == "com.apple.dt.CoreDeviceError" && e["code"] == 1000 {
        return Err(error(NOT_IN_XCODE));
    }
    let chain: Vec<&Value> = std::iter::successors(Some(e), |e| {
        Some(&e["userInfo"]["NSUnderlyingError"]["error"]).filter(|u| u.is_object())
    })
    .take(8)
    .collect();
    // SpringBoard refuses to open apps on a locked device.
    let locked = |e: &&Value| {
        e["domain"] == "FBSOpenApplicationErrorDomain" && e["code"] == 7
            || e["userInfo"]["BSErrorCodeDescription"]["string"] == "Locked"
    };
    if chain.iter().any(locked) {
        return Err(error(link::LOCKED));
    }
    let cause = chain
        .iter()
        .rev()
        .find_map(|e| info(e, "NSLocalizedFailureReason"));
    let reason: Vec<&str> = [info(e, "NSLocalizedDescription"), cause]
        .into_iter()
        .flatten()
        .collect();
    if reason.is_empty() {
        return Err(error(LAUNCH_FAILED));
    }
    // devicectl's texts may hold line breaks; the error bar shows one line.
    let words: Vec<&str> = reason.iter().flat_map(|s| s.split_whitespace()).collect();
    let reason: String = words.join(" ").chars().take(300).collect();
    Err(error(format!("{LAUNCH_FAILED}: {reason}")))
}
/// Xcode's tools, from its developer folder.
#[derive(Clone, Debug)]
pub struct Xcode {
    developer: PathBuf,
}
/// A command without input, so a tool that asks something fails instead of
/// waiting.
fn no_input(program: &Path) -> Command {
    let mut command = Command::new(program);
    command.stdin(Stdio::null());
    command
}
impl Xcode {
    /// The first developer folder with `devicectl`. `xcrun` is not run:
    /// without developer tools, it offers to install them.
    pub fn locate() -> io::Result<Self> {
        if !cfg!(target_os = "macos") {
            return Err(error(NO_XCODE));
        }
        let mut select = no_input(Path::new("/usr/bin/xcode-select"));
        select.arg("--print-path");
        let selected = run(select, QUERY_TIMEOUT, 64 * 1024)
            .ok()
            .filter(|out| out.success)
            .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
            .unwrap_or_default();
        developer_dirs(std::env::var_os("DEVELOPER_DIR"), &selected)
            .into_iter()
            .map(|developer| Self { developer })
            .find(|x| x.devicectl().is_file())
            .ok_or_else(|| error(NO_XCODE))
    }
    fn devicectl(&self) -> PathBuf {
        self.developer.join("usr/bin/devicectl")
    }
    /// Whether Xcode finished its first-launch setup. `devicectl` would run
    /// that setup itself, installing system components.
    fn ready(&self) -> io::Result<()> {
        let mut check = no_input(&self.developer.join("usr/bin/xcodebuild"));
        check.env("DEVELOPER_DIR", &self.developer);
        check.arg("-checkFirstLaunchStatus");
        match run(check, QUERY_TIMEOUT, 64 * 1024) {
            Ok(out) if out.success => Ok(()),
            _ => Err(error(XCODE_SETUP)),
        }
    }
    /// Starts `bundle` on the device `udid`. The result is read from a JSON
    /// file: devicectl prints its reasons for people, the file holds them
    /// with their codes.
    fn launch(&self, udid: &str, bundle: &str) -> io::Result<()> {
        let output = tempfile::Builder::new()
            .prefix("logstudio-devicectl-")
            .suffix(".json")
            .tempfile()?;
        let mut launch = no_input(&self.devicectl());
        launch.args(["device", "process", "launch", "--device", udid]);
        launch.args([
            "--terminate-existing",
            "--environment-variables",
            ENVIRONMENT,
        ]);
        launch.arg("--json-output").arg(output.path());
        // `--` ends devicectl's options: the app comes after it.
        launch.args(["--quiet", "--", bundle]);
        match run(launch, LAUNCH_TIMEOUT, 1 << 20) {
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::TimedOut => return Err(error(LAUNCH_TIMED_OUT)),
            Err(_) => return Err(error(DEVICECTL_FAILED)),
        }
        launch_outcome(&std::fs::read(output.path()).unwrap_or_default())
    }
}

/// Developer builds through Apple's device service, started through Xcode.
#[derive(Default)]
pub struct IosApps {
    link: IosLink,
}
impl Apps for IosApps {
    /// Checks Xcode first, so no app is listed that cannot be started.
    fn apps(&self, serial: &str) -> io::Result<Vec<App>> {
        Xcode::locate()?.ready()?;
        Ok(developer_apps(&ios::request(browse(&self.link, serial))?))
    }
    fn launch(&self, serial: &str, bundle: &str) -> io::Result<()> {
        if !valid_bundle_id(bundle) {
            return Err(error(NOT_LISTED));
        }
        let xcode = Xcode::locate()?;
        xcode.ready()?;
        xcode.launch(serial, bundle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::public_message;

    fn entry(bundle: &str, name: &str, debuggable: Option<bool>) -> plist::Value {
        let mut d = plist::Dictionary::new();
        d.insert("CFBundleIdentifier".into(), bundle.into());
        d.insert("CFBundleDisplayName".into(), name.into());
        d.insert("CFBundleShortVersionString".into(), "1.0.2".into());
        d.insert("CFBundleExecutable".into(), "SampleApp".into());
        let mut entitlements = plist::Dictionary::new();
        entitlements.insert(
            "application-identifier".into(),
            format!("QA00TEAM00.{bundle}").into(),
        );
        if let Some(allow) = debuggable {
            entitlements.insert("get-task-allow".into(), allow.into());
        }
        d.insert("Entitlements".into(), entitlements.into());
        d.into()
    }

    #[test]
    fn only_developer_signed_apps_are_listed_by_name() {
        let mut unnamed = entry("com.example.unnamed", "", Some(true));
        let d = unnamed.as_dictionary_mut().unwrap();
        d.insert("CFBundleName".into(), "Unnamed".into());
        let list = [
            entry("com.example.store", "Store App", Some(false)),
            entry("com.example.sample", "sample", Some(true)),
            entry("com.example.enterprise", "Enterprise", None),
            entry("com.example.alpha", "Alpha", Some(true)),
            unnamed,
            entry("-com.example.option", "Option", Some(true)),
            plist::Value::String("not an entry".into()),
        ];
        let apps = developer_apps(&list);
        let got: Vec<(&str, &str)> = apps
            .iter()
            .map(|a| (a.name.as_str(), a.bundle.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                ("Alpha", "com.example.alpha"),
                ("sample", "com.example.sample"),
                ("Unnamed", "com.example.unnamed")
            ]
        );
        assert_eq!(
            (apps[0].version.as_str(), apps[0].executable.as_str()),
            ("1.0.2", "SampleApp")
        );
        assert_eq!(
            apps[0].json(),
            json!({"bundle":"com.example.alpha","name":"Alpha","version":"1.0.2","executable":"SampleApp"})
        );
    }

    #[test]
    fn bundle_ids_never_read_as_options() {
        for ok in ["com.example.sample", "com.example.sample-dev", "QA_1.app"] {
            assert!(valid_bundle_id(ok), "{ok}");
        }
        for bad in [
            "",
            "-x",
            "--device",
            ".hidden",
            "a b",
            "com.example/../x",
            "앱",
        ] {
            assert!(!valid_bundle_id(bad), "{bad}");
        }
        assert!(!valid_bundle_id(&"a".repeat(256)));
    }

    /// Xcode's folders are macOS paths, which Windows does not call
    /// absolute; Windows never looks for Xcode.
    #[cfg(unix)]
    #[test]
    fn developer_folders_follow_xcrun() {
        let xcode = PathBuf::from("/Applications/Xcode.app/Contents/Developer");
        assert_eq!(
            developer_dirs(None, "/Applications/Xcode.app/Contents/Developer\n"),
            vec![xcode.clone()]
        );
        // The Command Line Tools have no devicectl; Xcode is looked for.
        assert_eq!(
            developer_dirs(None, "/Library/Developer/CommandLineTools\n"),
            [
                PathBuf::from("/Library/Developer/CommandLineTools"),
                xcode.clone()
            ]
        );
        assert_eq!(
            developer_dirs(Some("/Applications/Xcode-beta.app".into()), ""),
            [
                PathBuf::from("/Applications/Xcode-beta.app/Contents/Developer"),
                xcode.clone()
            ]
        );
        assert_eq!(
            developer_dirs(Some("relative/Xcode.app".into()), "not absolute"),
            [xcode]
        );
    }

    /// devicectl's JSON output, shaped as `jsonVersion` 5 writes it.
    fn output(error: Value) -> Vec<u8> {
        let outcome = if error.is_null() { "success" } else { "failed" };
        let v = json!({"info":{"commandType":"devicectl.device.process.launch","jsonVersion":5,"outcome":outcome},"error":error});
        serde_json::to_vec(&v).unwrap()
    }

    #[test]
    fn launch_outcomes_say_why_the_app_did_not_start() {
        assert!(launch_outcome(&output(Value::Null)).is_ok());
        let message = |e: Value| public_message(&launch_outcome(&output(e)).unwrap_err(), "");
        let missing = json!({"code":10002,"domain":"com.apple.dt.CoreDeviceError","userInfo":{
            "NSLocalizedDescription":{"string":"The application failed to launch."},
            "NSLocalizedFailureReason":{"string":"The requested application com.example.sample is\nnot installed."},
            "NSLocalizedRecoverySuggestion":{"string":"Provide a valid bundle identifier."}}});
        assert_eq!(
            message(missing),
            "Could not start the app: The application failed to launch. The requested application com.example.sample is not installed."
        );
        // Shaped as devicectl reported an iPhone that was locked.
        let locked = json!({"code":10002,"domain":"com.apple.dt.CoreDeviceError","userInfo":{
            "NSLocalizedDescription":{"string":"The application failed to launch."},
            "NSUnderlyingError":{"error":{"code":1,"domain":"FBSOpenApplicationServiceErrorDomain","userInfo":{
                "BSErrorCodeDescription":{"string":"RequestDenied"},
                "NSUnderlyingError":{"error":{"code":7,"domain":"FBSOpenApplicationErrorDomain","userInfo":{
                    "BSErrorCodeDescription":{"string":"Locked"}}}}}}}}});
        assert_eq!(message(locked), link::LOCKED);
        // A cause under the step that failed is the reason given.
        let nested = json!({"code":10002,"domain":"com.apple.dt.CoreDeviceError","userInfo":{
            "NSLocalizedDescription":{"string":"The application failed to launch."},
            "NSUnderlyingError":{"error":{"code":1,"domain":"FBSOpenApplicationServiceErrorDomain","userInfo":{
                "NSLocalizedFailureReason":{"string":"The request was denied."},
                "NSUnderlyingError":{"error":{"code":3,"domain":"FBSOpenApplicationErrorDomain","userInfo":{
                    "NSLocalizedFailureReason":{"string":"The app is not signed for this device."}}}}}}}}});
        assert_eq!(
            message(nested),
            "Could not start the app: The application failed to launch. The app is not signed for this device."
        );
        let unknown = json!({"code":1000,"domain":"com.apple.dt.CoreDeviceError","userInfo":{
            "NSLocalizedDescription":{"string":"The specified device was not found."}}});
        assert_eq!(message(unknown), NOT_IN_XCODE);
        assert_eq!(message(json!({"code":1})), LAUNCH_FAILED);
        let long = json!({"userInfo":{"NSLocalizedDescription":{"string":"x".repeat(1000)}}});
        assert_eq!(message(long).chars().count(), LAUNCH_FAILED.len() + 2 + 300);
        let e = launch_outcome(b"").unwrap_err();
        assert_eq!(public_message(&e, ""), LAUNCH_FAILED);
    }

    #[test]
    fn a_devicectl_that_does_not_run_is_named() {
        let xcode = Xcode {
            developer: PathBuf::from("/nonexistent/Xcode.app/Contents/Developer"),
        };
        let e = xcode
            .launch("QA0UDID0000000000000001", "com.example.sample")
            .unwrap_err();
        assert_eq!(public_message(&e, ""), DEVICECTL_FAILED);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn other_systems_have_no_xcode() {
        let e = IosApps::default()
            .apps("QA0UDID0000000000000001")
            .unwrap_err();
        assert_eq!(public_message(&e, ""), NO_XCODE);
    }
}
