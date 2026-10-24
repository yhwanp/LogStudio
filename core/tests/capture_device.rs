//! Capture checks against a real device or emulator, run by hand:
//!
//! LOGSTUDIO_ADB=~/Library/Android/sdk/platform-tools/adb \
//! LOGSTUDIO_SERIAL=emulator-5554 \
//! cargo test -p logstudio-core --test capture_device -- --ignored --test-threads 1 --nocapture
//!
//! Captures go through the product's adb command line link; `Tool` only
//! drives the device for the checks.
use logstudio_core::{
    capture::{cli::AdbCli, gate::Stamp, store::Store, Capture, Options, Phase},
    source::error,
};
use std::{
    collections::HashMap,
    io,
    process::{Command, Stdio},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

fn adb_path() -> String {
    std::env::var("LOGSTUDIO_ADB").unwrap_or_else(|_| "adb".into())
}
struct Tool;
impl Tool {
    fn run(&self, args: &[&str]) -> io::Result<Vec<u8>> {
        let out = Command::new(adb_path())
            .args(args)
            .stdin(Stdio::null())
            .output()?;
        if !out.status.success() {
            return Err(error(
                String::from_utf8_lossy(&out.stderr).trim().to_string(),
            ));
        }
        Ok(out.stdout)
    }
    fn shell(&self, serial: &str, command: &str) -> io::Result<String> {
        let out = self.run(&["-s", serial, "shell", command])?;
        Ok(String::from_utf8_lossy(&out).trim().to_string())
    }
}

fn serial() -> String {
    std::env::var("LOGSTUDIO_SERIAL").unwrap_or_else(|_| "emulator-5554".into())
}
fn wait(what: &str, secs: u64, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while !done() {
        assert!(Instant::now() < deadline, "timed out: {what}");
        thread::sleep(Duration::from_millis(50));
    }
}
struct Run {
    _dir: tempfile::TempDir,
    adb: Tool,
    capture: Arc<Capture>,
    /// The device's rows when the capture started.
    before: Vec<u8>,
}
fn start() -> Run {
    let dir = tempfile::tempdir().unwrap();
    let adb = Tool;
    let before = adb
        .run(&["-s", &serial(), "logcat", "-d", "-v", "threadtime"])
        .unwrap();
    let slot = Store::new(dir.path()).create("device").unwrap();
    let options = Options {
        poll: Duration::from_millis(200),
        recent: 256,
    };
    let link = Arc::new(AdbCli::new(std::path::Path::new(&adb_path())));
    let capture = Capture::start(link, &serial(), slot, options).unwrap();
    Run {
        _dir: dir,
        adb,
        capture,
        before,
    }
}
impl Run {
    fn text(&self) -> String {
        String::from_utf8_lossy(&std::fs::read(self.capture.path()).unwrap()).into_owned()
    }
    /// Logs `tag` rows `tick 1..=n` from a device process that outlives
    /// the adb connection, `ms` apart.
    fn ticks(&self, tag: &str, n: u32, ms: u32) {
        let script = format!(
            "i=1; while [ $i -le {n} ]; do log -t {tag} tick$i; i=$((i+1)); sleep 0.0{ms:02}; done"
        );
        let command = format!("nohup sh -c '{script}' >/dev/null 2>&1 &");
        self.adb.shell(&serial(), &command).unwrap();
    }
    /// How often each `tick<k>` of `tag` was captured.
    fn counts(&self, tag: &str) -> HashMap<u32, usize> {
        let mut counts = HashMap::new();
        for line in self.text().lines() {
            if let Some((_, rest)) = line.split_once(&format!(" {tag}: tick")) {
                *counts.entry(rest.trim().parse().unwrap()).or_default() += 1;
            }
        }
        counts
    }
    fn assert_each_once(&self, tag: &str, n: u32) {
        let counts = self.counts(tag);
        let missing: Vec<u32> = (1..=n).filter(|k| !counts.contains_key(k)).collect();
        let repeated: Vec<(u32, usize)> = counts
            .iter()
            .filter(|(_, c)| **c > 1)
            .map(|(k, c)| (*k, *c))
            .collect();
        assert!(
            missing.is_empty() && repeated.is_empty(),
            "missing {missing:?} repeated {repeated:?}"
        );
    }
    /// Timed rows of the capture that the device held before it started.
    fn rows_from_before(&self) -> usize {
        let before = String::from_utf8_lossy(&self.before);
        let old: std::collections::HashSet<&str> = before
            .lines()
            .filter(|l| Stamp::of(l.as_bytes()).is_some())
            .collect();
        self.text().lines().filter(|l| old.contains(l)).count()
    }
}

#[test]
#[ignore = "needs a device or emulator"]
fn device_rows_logged_after_start_are_captured_and_none_before() {
    let run = start();
    run.ticks("LogStudioA", 100, 10);
    wait("ticks", 60, || run.counts("LogStudioA").len() == 100);
    run.assert_each_once("LogStudioA", 100);
    assert_eq!(run.rows_from_before(), 0);
    run.capture.stop();
    let text = run.text();
    println!(
        "rows {} bytes {} dividers {}",
        text.lines().count(),
        text.len(),
        text.lines().filter(|l| l.starts_with("---------")).count()
    );
}

#[test]
#[ignore = "needs a device or emulator"]
fn a_restarted_adb_server_leaves_no_gap_or_repeat() {
    let run = start();
    run.ticks("LogStudioB", 400, 10);
    wait("first ticks", 60, || run.counts("LogStudioB").len() >= 50);
    run.adb.run(&["kill-server"]).unwrap();
    wait("waiting", 30, || run.capture.phase() == Phase::Waiting);
    // `adb devices` starts the server again; the capture continues.
    wait("recording", 60, || run.capture.phase() == Phase::Recording);
    wait("all ticks", 120, || run.counts("LogStudioB").len() == 400);
    run.assert_each_once("LogStudioB", 400);
    let breaks = run.capture.breaks();
    println!("breaks {breaks:?}");
    assert_eq!(breaks.len(), 1);
    assert!(!breaks[0].reboot);
    run.capture.stop();
    // No row around the break is there more often than the device holds
    // it: devices log identical rows at one time too.
    let text = run.text();
    let rows: Vec<&str> = text.lines().collect();
    let at = breaks[0].row as usize - 1;
    let near = &rows[at.saturating_sub(300)..(at + 300).min(rows.len())];
    let device = run
        .adb
        .run(&["-s", &serial(), "logcat", "-d", "-v", "threadtime"])
        .unwrap();
    let device = String::from_utf8_lossy(&device);
    let mut held: HashMap<&str, usize> = HashMap::new();
    for l in device.lines() {
        *held.entry(l).or_default() += 1;
    }
    let mut captured: HashMap<&str, usize> = HashMap::new();
    for l in near.iter().filter(|l| Stamp::of(l.as_bytes()).is_some()) {
        *captured.entry(l).or_default() += 1;
    }
    let extra: Vec<(&str, usize, usize)> = captured
        .iter()
        .filter(|(l, n)| held.get(*l).is_some_and(|h| **n > *h))
        .map(|(l, n)| (*l, *n, held[l]))
        .collect();
    let repeats = captured.values().filter(|n| **n > 1).count();
    println!(
        "rows near the break: {}, repeated by the device: {repeats}",
        near.len()
    );
    assert!(extra.is_empty(), "captured more often than held: {extra:?}");
}

#[test]
#[ignore = "needs a device or emulator"]
fn a_rebooted_device_is_captured_from_its_first_row() {
    let run = start();
    run.ticks("LogStudioC", 20, 10);
    wait("ticks", 60, || run.counts("LogStudioC").len() == 20);
    run.adb.run(&["-s", &serial(), "reboot"]).unwrap();
    wait("rebooted", 300, || {
        run.capture.breaks().iter().any(|b| b.reboot) && run.capture.phase() == Phase::Recording
    });
    let b = run.capture.breaks().into_iter().find(|b| b.reboot).unwrap();
    // The first row after the break is the new boot's first row.
    wait("boot rows", 120, || {
        run.text().lines().count() as u64 > b.row + 100
    });
    let text = run.text();
    let first: Vec<&str> = text.lines().skip(b.row as usize - 1).take(5).collect();
    println!(
        "breaks {:?}\nfirst rows after the reboot:\n{}",
        run.capture.breaks(),
        first.join("\n")
    );
    run.assert_each_once("LogStudioC", 20);
    run.capture.stop();
}

#[test]
#[ignore = "needs an installed adb"]
fn the_installed_adb_is_found() {
    use logstudio_core::capture::adb::{candidates, locate, CHECK_TIMEOUT};
    let list = candidates(None, |k| std::env::var_os(k));
    let found = locate(&list, CHECK_TIMEOUT).expect("no adb found");
    println!(
        "{} {:?} {}",
        found.path.display(),
        found.origin,
        found.json()
    );
}
