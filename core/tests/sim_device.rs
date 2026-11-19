//! Capture checks against a running iOS simulator on this Mac, run by hand:
//!
//! cargo test -p logstudio-core --test sim_device -- --ignored --test-threads 1 --nocapture
//!
//! `LOGSTUDIO_SIM_UDID` picks a simulator when several run. With
//! `LOGSTUDIO_SIM_RESTART=1`, the simulator is shut down and started again;
//! without it, that test says it was skipped.
//!
//! The first test also runs a second `log stream` into a file, as a pipe
//! would take it, and compares the entries both got over the same time.
use logstudio_core::{
    capture::{
        link::{self, Link, Platform},
        sim::SimLink,
        store::Store,
        Capture, Options, Phase,
    },
    parser::parse_line,
};
use std::{
    collections::BTreeMap,
    fs::File,
    process::{Command, Stdio},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

const SIMCTL: &str =
    "/Library/Developer/PrivateFrameworks/CoreSimulator.framework/Versions/A/Resources/bin/simctl";

fn udid(link: &SimLink) -> String {
    let devices = link.devices().unwrap();
    let wanted = std::env::var("LOGSTUDIO_SIM_UDID").ok();
    let d = devices
        .iter()
        .find(|d| d.ready() && wanted.as_ref().is_none_or(|w| *w == d.serial))
        .unwrap_or_else(|| panic!("no running simulator among {devices:?}"));
    assert!(d.simulator && d.platform == Platform::Ios);
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
fn simctl(args: &[&str]) {
    let status = Command::new(SIMCTL).args(args).status().unwrap();
    assert!(status.success(), "simctl {args:?}");
}
/// Processes running `log stream` for a capture, in any simulator.
fn log_streams() -> usize {
    let out = Command::new("pgrep")
        .args(["-f", "log stream --style ndjson --level debug --type log"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).lines().count()
}
/// This process's CPU time so far.
fn cpu() -> Duration {
    let out = Command::new("ps")
        .args(["-o", "cputime=", "-p", &std::process::id().to_string()])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    let mut seconds = 0.0;
    for part in text.trim().split(':') {
        seconds = seconds * 60.0 + part.parse::<f64>().unwrap_or(0.0);
    }
    Duration::from_secs_f64(seconds)
}
/// The times of the entries in a `log stream --style ndjson` file, as rows
/// write them.
fn entry_times(json: &str) -> Vec<String> {
    json.lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|e| e["eventType"] == "logEvent")
        .filter_map(|e| {
            let t = e["timestamp"].as_str()?;
            let t = chrono::DateTime::parse_from_str(t, "%Y-%m-%d %H:%M:%S%.f%z").ok()?;
            Some(
                t.with_timezone(&chrono::Local)
                    .format("%Y-%m-%d %H:%M:%S%.6f")
                    .to_string(),
            )
        })
        .collect()
}
/// How many of `times` fall in each second from `from` up to `to`.
fn per_second<'a>(times: &'a [String], from: &str, to: &str) -> BTreeMap<&'a str, usize> {
    let mut counts = BTreeMap::new();
    for t in times
        .iter()
        .filter(|t| t.as_str() >= from && t.as_str() < to)
    {
        *counts.entry(&t[..19]).or_default() += 1;
    }
    counts
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
#[ignore = "needs a running iOS simulator"]
fn simulator_rows_from_the_start_on_are_recorded_as_ios_rows() {
    let link = Arc::new(SimLink::locate().expect("CoreSimulator's simctl"));
    let serial = udid(&link);
    let none = log_streams();
    let dir = tempfile::tempdir().unwrap();
    // The same entries through a file; it ends on its own.
    let reference = dir.path().join("reference.ndjson");
    let mut tool = Command::new(SIMCTL)
        .args(["spawn", &serial, "log", "stream", "--style", "ndjson"])
        .args(["--level", "debug", "--type", "log", "--timeout", "20"])
        .stdout(File::create(&reference).unwrap())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    thread::sleep(Duration::from_secs(1));
    let before = log_streams();
    let slot = Store::new(dir.path())
        .create(Platform::Ios, "iPhone")
        .unwrap();
    // Rows of entries logged before the start are not recorded; a second
    // allows for the clocks of the simulator and this computer.
    let started = (chrono::Local::now() - chrono::Duration::seconds(1))
        .format("%Y-%m-%d %H:%M:%S%.6f")
        .to_string();
    let (at, cpu_at) = (Instant::now(), cpu());
    let capture = Capture::start(link, &serial, slot, Options::default()).unwrap();
    println!("started in {:.0} ms", at.elapsed().as_secs_f64() * 1000.0);
    assert!(capture.simulator);
    let first = Instant::now();
    wait("a first row", 10, || capture.bytes() > 0);
    println!(
        "first row after {:.0} ms",
        first.elapsed().as_secs_f64() * 1000.0
    );
    // A burst, as an app starting logs one.
    simctl(&[
        "launch",
        "--terminate-running-process",
        &serial,
        "com.apple.Preferences",
    ]);
    thread::sleep(Duration::from_secs(5));
    let stopping = Instant::now();
    capture.stop();
    println!(
        "stopped in {:.0} ms",
        stopping.elapsed().as_secs_f64() * 1000.0
    );
    let (seconds, used) = (at.elapsed().as_secs_f64(), cpu() - cpu_at);
    assert_eq!(capture.phase(), Phase::Stopped);
    // `log` ends once its output is shut, however quiet the simulator is;
    // the reference stream runs on.
    wait("log stream to end", 5, || log_streams() <= before);
    let text = String::from_utf8_lossy(&std::fs::read(capture.path()).unwrap()).into_owned();
    let times = check_rows(&text);
    assert!(times.len() > 10, "{} rows", times.len());
    let earliest = times.iter().min().unwrap();
    assert!(
        earliest.as_str() > started.as_str(),
        "{earliest} before {started}"
    );
    // Whole seconds that both streams spent running.
    let (first, last) = (times.iter().min().unwrap(), times.iter().max().unwrap());
    let from = format!("{}.999999", &first[..19]);
    let to = last[..19].to_string();
    let recorded = per_second(&times, &from, &to);
    wait("the reference stream to end", 30, || log_streams() <= none);
    tool.wait().unwrap();
    let reference = entry_times(&std::fs::read_to_string(&reference).unwrap());
    let expected = per_second(&reference, &from, &to);
    let (got, want): (usize, usize) = (recorded.values().sum(), expected.values().sum());
    println!("from {from} to {to}: {got} rows recorded, {want} entries through a file");
    for (second, n) in &expected {
        let r = recorded.get(second).copied().unwrap_or_default();
        if r != *n {
            println!("  {second}: {r} recorded, {n} through a file");
        }
    }
    assert!(got >= want, "{} entries missing", want - got);
    println!(
        "{} rows, {} bytes in {seconds:.1} s: {:.0} rows/s, {:.0} KB/s; this process used {:.2} s CPU ({:.1}% of one core)",
        times.len(),
        text.len(),
        times.len() as f64 / seconds,
        text.len() as f64 / seconds / 1024.0,
        used.as_secs_f64(),
        used.as_secs_f64() / seconds * 100.0
    );
}

#[test]
#[ignore = "needs a running iOS simulator; set LOGSTUDIO_SIM_RESTART=1"]
fn a_simulator_shut_down_while_recording_is_recorded_again_once_it_runs() {
    if std::env::var("LOGSTUDIO_SIM_RESTART").as_deref() != Ok("1") {
        println!("skipped: set LOGSTUDIO_SIM_RESTART=1 to shut the simulator down and start it");
        return;
    }
    let link = Arc::new(SimLink::locate().expect("CoreSimulator's simctl"));
    let serial = udid(&link);
    let dir = tempfile::tempdir().unwrap();
    let slot = Store::new(dir.path())
        .create(Platform::Ios, "iPhone")
        .unwrap();
    let capture = Capture::start(link, &serial, slot, Options::default()).unwrap();
    wait("a first row", 10, || capture.bytes() > 0);
    simctl(&["shutdown", &serial]);
    wait("waiting for the simulator", 30, || {
        capture.phase() == Phase::Waiting && capture.json()["reason"] == link::SHUTDOWN
    });
    simctl(&["boot", &serial]);
    wait("recording again", 120, || {
        capture.phase() == Phase::Recording
    });
    let bytes = capture.bytes();
    wait("rows after the restart", 30, || capture.bytes() > bytes);
    let breaks = capture.breaks();
    assert_eq!(breaks.len(), 1);
    assert!(breaks[0].gap);
    capture.stop();
}
