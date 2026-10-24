//! Capture checks against a real iPhone or iPad connected by USB or Wi-Fi and
//! trusted, run by hand:
//!
//! cargo test -p logstudio-core --test ios_device -- --ignored --test-threads 1 --nocapture
//!
//! `LOGSTUDIO_IOS_UDID` picks a device when several are connected. Starting
//! a developer build needs Xcode and the build's bundle identifier in
//! `LOGSTUDIO_IOS_APP`; the app is ended and started again.
use logstudio_core::{
    capture::{
        apps::{Apps, IosApps},
        ios::IosLink,
        link::{Link, Platform},
        store::Store,
        Capture, Options, Phase,
    },
    parser::parse_line,
};
use std::{
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

fn udid(link: &IosLink) -> String {
    let devices = link.devices().unwrap();
    let wanted = std::env::var("LOGSTUDIO_IOS_UDID").ok();
    let d = devices
        .iter()
        .find(|d| d.ready() && wanted.as_ref().is_none_or(|w| *w == d.serial))
        .unwrap_or_else(|| panic!("no ready device among {devices:?}"));
    assert_eq!(d.platform, Platform::Ios);
    assert!(!d.model.is_empty() && !d.product.is_empty() && !d.version.is_empty());
    d.serial.clone()
}
fn wait(what: &str, secs: u64, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while !done() {
        assert!(Instant::now() < deadline, "timed out: {what}");
        thread::sleep(Duration::from_millis(100));
    }
}
/// Every row is an iOS row or continues one; returns the iOS rows' times.
fn check_rows(text: &str) -> Vec<String> {
    let mut times = vec![];
    for (n, line) in text.lines().enumerate() {
        match parse_line(line, n as u64 + 1, false).parsed {
            Some(p) => {
                assert!(p.ios, "row {}: {line}", n + 1);
                times.push(format!("{} {}", p.date, p.time));
            }
            // Rows after a message's line breaks; the first row always parses.
            None => assert!(n > 0, "row 1 does not parse: {line}"),
        }
    }
    times
}

#[test]
#[ignore = "needs a trusted iPhone or iPad"]
fn iphone_rows_from_the_start_on_are_recorded_as_ios_rows() {
    let link = Arc::new(IosLink::default());
    let serial = udid(&link);
    let dir = tempfile::tempdir().unwrap();
    let slot = Store::new(dir.path())
        .create(Platform::Ios, "iPhone")
        .unwrap();
    let started = chrono::Local::now()
        .format("%Y-%m-%d %H:%M:%S%.6f")
        .to_string();
    let at = Instant::now();
    let capture = Capture::start(link, &serial, slot, Options::default()).unwrap();
    thread::sleep(Duration::from_secs(5));
    capture.stop();
    let seconds = at.elapsed().as_secs_f64();
    assert_eq!(capture.phase(), Phase::Stopped);
    let text = String::from_utf8_lossy(&std::fs::read(capture.path()).unwrap()).into_owned();
    let times = check_rows(&text);
    assert!(times.len() > 10, "{} rows", times.len());
    // The service sends nothing from before it started; the first entry
    // may have been logged while it started.
    let earliest = times.iter().min().unwrap();
    assert!(
        earliest.as_str() > &started[..17],
        "{earliest} before {started}"
    );
    println!(
        "{} rows, {} bytes in {seconds:.1} s: {:.0} rows/s, {:.0} KB/s",
        times.len(),
        text.len(),
        times.len() as f64 / seconds,
        text.len() as f64 / seconds / 1024.0
    );
}

#[test]
#[ignore = "needs a trusted iPhone or iPad and a person to unplug it"]
fn an_unplugged_iphone_is_waited_for_and_continued() {
    let link = Arc::new(IosLink::default());
    let serial = udid(&link);
    let dir = tempfile::tempdir().unwrap();
    let slot = Store::new(dir.path())
        .create(Platform::Ios, "iPhone")
        .unwrap();
    let options = Options {
        poll: Duration::from_millis(500),
        ..Options::default()
    };
    let capture = Capture::start(link, &serial, slot, options).unwrap();
    println!("Unplug the device within 5 minutes, then plug it in again.");
    wait("unplugged", 300, || capture.phase() == Phase::Waiting);
    let before = capture.bytes();
    println!("Waiting; plug the device in again within 5 minutes.");
    wait("back", 300, || capture.phase() == Phase::Recording);
    wait("rows after the break", 30, || capture.bytes() > before);
    thread::sleep(Duration::from_secs(2));
    capture.stop();
    let breaks = capture.breaks();
    assert_eq!(breaks.len(), 1);
    assert!(!breaks[0].reboot);
    let text = String::from_utf8_lossy(&std::fs::read(capture.path()).unwrap()).into_owned();
    check_rows(&text);
    println!(
        "break at row {} after {} ms away",
        breaks[0].row,
        breaks[0].resumed - breaks[0].lost
    );
}

#[test]
#[ignore = "needs a trusted iPhone or iPad and Xcode"]
fn developer_builds_are_listed() {
    let link = IosLink::default();
    let serial = udid(&link);
    let at = Instant::now();
    let apps = IosApps::default().apps(&serial).unwrap();
    println!(
        "{} developer builds in {:.2} s",
        apps.len(),
        at.elapsed().as_secs_f64()
    );
    for app in &apps {
        println!("  {} {} ({})", app.name, app.version, app.bundle);
    }
    assert!(apps
        .iter()
        .all(|a| !a.name.is_empty() && !a.bundle.is_empty()));
}

#[test]
#[ignore = "needs a trusted iPhone or iPad, Xcode and LOGSTUDIO_IOS_APP"]
fn a_developer_build_started_again_logs_its_private_values() {
    let bundle = std::env::var("LOGSTUDIO_IOS_APP").expect("LOGSTUDIO_IOS_APP names the app");
    let link = Arc::new(IosLink::default());
    let serial = udid(&link);
    let apps = IosApps::default();
    let app = apps
        .apps(&serial)
        .unwrap()
        .into_iter()
        .find(|a| a.bundle == bundle)
        .unwrap_or_else(|| panic!("{bundle} is not a developer build on the device"));
    let dir = tempfile::tempdir().unwrap();
    let slot = Store::new(dir.path())
        .create(Platform::Ios, "iPhone")
        .unwrap();
    let capture = Capture::start(link, &serial, slot, Options::default()).unwrap();
    thread::sleep(Duration::from_secs(1));
    let at = Instant::now();
    apps.launch(&serial, &bundle).unwrap();
    let launched = at.elapsed().as_secs_f64();
    thread::sleep(Duration::from_secs(10));
    capture.stop();
    let text = String::from_utf8_lossy(&std::fs::read(capture.path()).unwrap()).into_owned();
    check_rows(&text);
    // Rows of the app's process, and how many of them and of the others
    // still hold a redacted value.
    let (mut own, mut own_private, mut other_private) = (0, 0, 0);
    for (n, line) in text.lines().enumerate() {
        let Some(p) = parse_line(line, n as u64 + 1, false).parsed else {
            continue;
        };
        let private = line.contains("<private>");
        if p.process == Some(app.executable.as_str()) {
            own += 1;
            own_private += usize::from(private);
        } else {
            other_private += usize::from(private);
        }
    }
    println!(
        "started in {launched:.1} s; {own} rows of {}, {own_private} with <private>; {other_private} <private> rows of other processes",
        app.executable
    );
    assert!(own > 0, "no rows of {}", app.executable);
    assert_eq!(own_private, 0);
}
