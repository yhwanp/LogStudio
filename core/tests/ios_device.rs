//! Capture checks against a real iPhone or iPad connected by USB or Wi-Fi and
//! trusted, run by hand:
//!
//! cargo test -p logstudio-core --test ios_device -- --ignored --test-threads 1 --nocapture
//!
//! `LOGSTUDIO_IOS_UDID` picks a device when several are connected.
use logstudio_core::{
    capture::{
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
