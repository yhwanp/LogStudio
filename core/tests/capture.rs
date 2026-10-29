use logstudio_core::{
    capture::{
        fake::FakeLink,
        gate::{Gate, Stamp},
        link::{self, parse_devices, Platform},
        store::Store,
        Break, Capture, Cut, Launch, Options, Phase,
    },
    search::Filters,
    source::public_message,
    Engine,
};
use serde_json::json;
use std::{
    cmp::Ordering,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

fn wait(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !done() {
        assert!(Instant::now() < deadline, "timed out: {what}");
        thread::sleep(Duration::from_millis(1));
    }
}
/// A synthetic `threadtime` row at `10-23 14:00:<sec>.<ms>`.
fn row(sec: u32, ms: u32, message: &str) -> String {
    format!("10-23 14:00:{sec:02}.{ms:03}  1234  5678 I Tag: {message}\n")
}
fn options() -> Options {
    Options {
        poll: Duration::from_millis(2),
        recent: 256,
    }
}
struct Setup {
    _dir: tempfile::TempDir,
    store: Store,
    link: Arc<FakeLink>,
}
fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().join("captures"));
    let link = FakeLink::new();
    link.add("SER1", "Pixel_8", "SER1");
    Setup {
        _dir: dir,
        store,
        link,
    }
}
impl Setup {
    fn start(&self, serial: &str) -> Arc<Capture> {
        let slot = self.store.create(Platform::Android, "Pixel 8").unwrap();
        Capture::start(self.link.clone(), serial, slot, options()).unwrap()
    }
}
fn content(c: &Capture) -> Vec<u8> {
    std::fs::read(c.path()).unwrap()
}
fn written(c: &Capture, expected: &str) {
    wait(&format!("capture holds {expected:?}"), || {
        content(c) == expected.as_bytes()
    });
}

#[test]
fn stamps_read_threadtime_times_and_order_across_a_year_end() {
    let t = Stamp::of(b"10-23 14:02:11.123  1 2 I T: m").unwrap();
    assert_eq!(t.arg(), "10-23 14:02:11.123");
    assert_eq!(Stamp::of(t.arg().as_bytes()), Some(t));
    for other in [
        &b"--------- beginning of main"[..],
        b"2026-10-23 14:02:11.123  1 2 I T: m",
        b"10-23 14:02:11.123456  1 2 I T: m",
        b"13-23 14:02:11.123  1 2 I T: m",
        b"10-23 14:02",
    ] {
        assert_eq!(Stamp::of(other), None, "{}", String::from_utf8_lossy(other));
    }
    let late = Stamp::of(b"12-31 23:59:59.999").unwrap();
    let early = Stamp::of(b"01-01 00:00:00.000").unwrap();
    assert_eq!(late.order(&early), Ordering::Less);
    assert_eq!(early.order(&late), Ordering::Greater);
    let june = Stamp::of(b"06-01 00:00:00.000").unwrap();
    assert_eq!(early.order(&june), Ordering::Less);
}

#[test]
fn a_gate_drops_rows_from_before_its_boundary_and_rows_it_has() {
    let rows = [
        row(1, 0, "a"),
        row(2, 0, "b"),
        row(2, 0, "b"),
        row(2, 0, "c"),
    ]
    .concat();
    let mut gate = Gate::after(rows.as_bytes());
    assert_eq!(gate.since().unwrap().arg(), "10-23 14:00:02.000");
    assert!(!gate.keep(row(1, 0, "a").as_bytes()));
    assert!(!gate.keep(row(2, 0, "b").as_bytes()));
    assert!(!gate.keep(row(2, 0, "c").trim_end().as_bytes()));
    // A row of the boundary's time the capture lacks is new; rows it has
    // may still follow.
    assert!(gate.keep(row(2, 0, "d").as_bytes()));
    assert!(!gate.keep(row(2, 0, "b").as_bytes()));
    assert!(gate.keep(row(2, 0, "b").as_bytes()));
    assert!(gate.keep(b"--------- beginning of main\n"));
    assert!(!gate.is_open());
    assert!(gate.keep(row(3, 0, "e").as_bytes()));
    assert!(gate.is_open());
    assert!(gate.keep(row(1, 0, "a").as_bytes()));
    // Rows compare without their line endings.
    let mut crlf = Gate::after(row(2, 0, "b").replace('\n', "\r\n").as_bytes());
    assert!(!crlf.keep(row(2, 0, "b").as_bytes()));
    assert!(Gate::after(b"no time\n").is_open());
    assert!(Gate::after(b"").is_open());
}

#[test]
fn a_gate_lets_rows_through_after_dropping_a_mebibyte() {
    let mut gate = Gate::after(row(30, 0, "boundary").as_bytes());
    let old = row(1, 0, &"x".repeat(1000));
    let dropped = (0..2000).take_while(|_| !gate.keep(old.as_bytes())).count();
    assert!((1000..1100).contains(&dropped), "{dropped}");
    assert!(gate.is_open());
}

#[test]
fn adb_device_listings_are_parsed() {
    let text = "List of devices attached\n\
        * daemon started successfully\n\
        emulator-5554          device product:sdk_gphone64_arm64 model:sdk_gphone64_arm64 device:emu64a transport_id:1\n\
        QA0SERIAL02            unauthorized usb:1-1 transport_id:2\n\
        192.168.0.10:5555      offline transport_id:3\n\
        0123456789ABCDEF       no permissions (missing udev rules? user is in the plugdev group); see [http://developer.android.com/tools/device.html] usb:1-1 transport_id:4\n\
        adb-QA0SERIAL03-x._adb-tls-connect._tcp device product:tablet_x1 model:Tablet_X1 device:tablet_x1 transport_id:5\n\
        \n";
    let devices = parse_devices(text);
    let summary: Vec<(&str, &str, &str, bool)> = devices
        .iter()
        .map(|d| {
            (
                d.serial.as_str(),
                d.state.as_str(),
                d.model.as_str(),
                d.ready(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            ("emulator-5554", "device", "sdk_gphone64_arm64", true),
            ("QA0SERIAL02", "unauthorized", "", false),
            ("192.168.0.10:5555", "offline", "", false),
            ("0123456789ABCDEF", "no permissions", "", false),
            (
                "adb-QA0SERIAL03-x._adb-tls-connect._tcp",
                "device",
                "Tablet_X1",
                true
            ),
        ]
    );
    assert_eq!(devices[1].problem(), Some(link::UNAUTHORIZED));
    assert_eq!(devices[2].problem(), Some(link::OFFLINE));
    assert_eq!(devices[3].problem(), Some(link::NOT_READY));
    assert_eq!(devices[0].problem(), None);
    assert!(parse_devices("List of devices attached\n\n").is_empty());
}

#[test]
fn a_capture_starts_after_the_rows_the_device_already_holds() {
    let s = setup();
    for r in [row(1, 0, "old"), row(2, 0, "old"), row(2, 0, "old too")] {
        s.link.log("SER1", &r);
    }
    let c = s.start("SER1");
    assert_eq!(
        s.link.opened(),
        [("SER1".to_string(), Some("10-23 14:00:02.000".to_string()))]
    );
    let new = [row(2, 0, "new at the same time"), row(3, 0, "new")].concat();
    s.link.log("SER1", &new);
    written(&c, &new);
    assert_eq!(c.phase(), Phase::Recording);
    // The status follows the file.
    wait("rows", || c.json()["rows"] == 2);
    assert_eq!(c.json()["device"], "Pixel 8");
    c.stop();
    assert_eq!(c.phase(), Phase::Stopped);
    assert_eq!(content(&c), new.as_bytes());
}

#[test]
fn a_device_with_an_empty_log_is_captured_from_its_first_row() {
    let s = setup();
    let c = s.start("SER1");
    assert_eq!(s.link.opened()[0].1, None);
    s.link.log("SER1", &row(1, 0, "first"));
    written(&c, &row(1, 0, "first"));
    c.stop();
}

#[test]
fn rows_are_written_whole_and_unchanged() {
    let s = setup();
    let c = s.start("SER1");
    s.link
        .log("SER1", "10-23 14:00:01.000  1234  5678 I Tag: part");
    thread::sleep(Duration::from_millis(30));
    assert_eq!(content(&c), b"");
    let mut bytes = b" one\r\n".to_vec();
    bytes.extend_from_slice(b"10-23 14:00:02.000  1234  5678 W Tag: \xff\xfe broken\n");
    bytes.extend_from_slice(b"no time\rlone CR\n");
    s.link.log_bytes("SER1", &bytes);
    let mut expected = b"10-23 14:00:01.000  1234  5678 I Tag: part".to_vec();
    expected.extend_from_slice(&bytes);
    wait("raw rows", || content(&c) == expected);
    // CRLF ends one row; a lone CR ends one too, as rows are read.
    wait("rows", || c.json()["rows"] == 4);
    c.stop();
}

#[test]
fn a_device_that_returns_continues_after_the_rows_the_capture_has() {
    let s = setup();
    let c = s.start("SER1");
    let first = [row(1, 0, "one"), row(2, 0, "two")].concat();
    s.link.log("SER1", &first);
    written(&c, &first);
    // The stream ends within a row; the device logs on meanwhile.
    let three = row(2, 0, "three");
    s.link.log("SER1", &three[..20]);
    s.link.unplug("SER1");
    wait("waiting", || c.phase() == Phase::Waiting);
    assert!(c.json()["lost"].is_i64());
    s.link.log("SER1", &three[20..]);
    s.link.log("SER1", &row(3, 0, "four"));
    s.link.plug("SER1");
    let all = [first.clone(), three, row(3, 0, "four")].concat();
    written(&c, &all);
    assert_eq!(c.phase(), Phase::Recording);
    let breaks = c.breaks();
    assert_eq!(breaks.len(), 1);
    assert_eq!(
        (breaks[0].row, breaks[0].reboot, breaks[0].gap),
        (3, false, false)
    );
    assert!(breaks[0].lost <= breaks[0].resumed);
    assert_eq!(s.link.opened()[1].1.as_deref(), Some("10-23 14:00:02.000"));
    s.link.log("SER1", &row(4, 0, "five"));
    written(&c, &[all, row(4, 0, "five")].concat());
    c.stop();
}

#[test]
fn a_device_that_booted_meanwhile_continues_from_its_first_row() {
    let s = setup();
    let c = s.start("SER1");
    s.link.log("SER1", &row(50, 0, "before"));
    written(&c, &row(50, 0, "before"));
    s.link.unplug("SER1");
    wait("waiting", || c.phase() == Phase::Waiting);
    s.link.reboot("SER1");
    // A booting device's clock may be behind the rows before.
    let boot = [row(1, 0, "boot"), row(2, 0, "boot")].concat();
    s.link.log("SER1", &boot);
    s.link.plug("SER1");
    written(&c, &[row(50, 0, "before"), boot].concat());
    assert_eq!(
        c.breaks()
            .iter()
            .map(|b| (b.row, b.reboot))
            .collect::<Vec<_>>(),
        [(2, true)]
    );
    assert_eq!(s.link.opened().last().unwrap().1, None);
    c.stop();
}

#[test]
fn a_device_listed_under_a_new_serial_is_found_by_its_identity() {
    let s = setup();
    let link = &s.link;
    link.add("adb-X._adb-tls-connect._tcp", "Pixel_9", "WIRELESS1");
    let c = s.start("adb-X._adb-tls-connect._tcp");
    link.log("adb-X._adb-tls-connect._tcp", &row(1, 0, "one"));
    written(&c, &row(1, 0, "one"));
    link.unplug("adb-X._adb-tls-connect._tcp");
    wait("waiting", || c.phase() == Phase::Waiting);
    // Another device appearing is not the one captured.
    link.add("192.168.0.6:5555", "Other", "OTHER");
    thread::sleep(Duration::from_millis(30));
    assert_eq!(c.phase(), Phase::Waiting);
    link.rename("adb-X._adb-tls-connect._tcp", "192.168.0.5:5555");
    link.log("192.168.0.5:5555", &row(2, 0, "two"));
    link.plug("192.168.0.5:5555");
    written(&c, &[row(1, 0, "one"), row(2, 0, "two")].concat());
    assert_eq!(c.json()["serial"], "192.168.0.5:5555");
    c.stop();
}

#[test]
fn an_adb_server_of_another_version_ends_the_capture() {
    let s = setup();
    let c = s.start("SER1");
    s.link.log("SER1", &row(1, 0, "one"));
    written(&c, &row(1, 0, "one"));
    s.link.conflict(true);
    s.link.unplug("SER1");
    wait("failed", || c.phase() == Phase::Failed);
    assert_eq!(c.json()["error"], link::CONFLICT);
    assert!(!c.live());
    assert_eq!(content(&c), row(1, 0, "one").as_bytes());
}

#[test]
fn stopping_keeps_every_row_received() {
    let s = setup();
    let c = s.start("SER1");
    s.link.log("SER1", &row(1, 0, "one"));
    s.link
        .log("SER1", "10-23 14:00:02.000  1234  5678 I Tag: cut");
    wait("received", || c.json()["rows"] == 1);
    thread::sleep(Duration::from_millis(20));
    c.stop();
    assert_eq!(c.phase(), Phase::Stopped);
    let expected = [
        row(1, 0, "one"),
        "10-23 14:00:02.000  1234  5678 I Tag: cut".into(),
    ]
    .concat();
    assert_eq!(content(&c), expected.as_bytes());
    assert_eq!(c.bytes(), expected.len() as u64);
    // Stopping again does nothing.
    c.stop();
    assert_eq!(c.phase(), Phase::Stopped);
}

#[test]
fn stopping_while_waiting_ends_at_once() {
    let s = setup();
    let slot = s.store.create(Platform::Android, "Pixel 8").unwrap();
    let c = Capture::start(
        s.link.clone(),
        "SER1",
        slot,
        Options {
            poll: Duration::from_secs(60),
            recent: 256,
        },
    )
    .unwrap();
    s.link.unplug("SER1");
    wait("waiting", || c.phase() == Phase::Waiting);
    let at = Instant::now();
    c.stop();
    assert!(at.elapsed() < Duration::from_secs(5));
    assert_eq!(c.phase(), Phase::Stopped);
    assert_eq!(c.breaks(), Vec::<Break>::new());
}

#[test]
fn a_stopped_capture_records_on_into_its_file_from_its_resume() {
    let s = setup();
    let c = s.start("SER1");
    s.link.log("SER1", &row(1, 0, "one"));
    written(&c, &row(1, 0, "one"));
    // Only a stopped capture resumes.
    assert_eq!(
        public_message(&c.resume().err().unwrap(), ""),
        logstudio_core::capture::NOT_STOPPED
    );
    c.stop();
    let path = c.path();
    // Rows logged while stopped are not recorded.
    s.link.log("SER1", &row(2, 0, "while stopped"));
    c.resume().unwrap();
    assert_eq!(c.phase(), Phase::Recording);
    assert_eq!(c.path(), path);
    s.link.log("SER1", &row(3, 0, "three"));
    written(&c, &[row(1, 0, "one"), row(3, 0, "three")].concat());
    assert!(c.unsaved());
    let breaks = c.breaks();
    assert_eq!(breaks.len(), 1);
    assert_eq!(
        (
            breaks[0].row,
            breaks[0].reboot,
            breaks[0].gap,
            breaks[0].stopped
        ),
        (2, false, true, true)
    );
    assert!(breaks[0].lost <= breaks[0].resumed);
    assert_eq!(c.json()["breaks"][0]["stopped"], true);
    // It starts after the rows the device held, as a new capture does.
    assert_eq!(
        s.link.opened().last().unwrap().1.as_deref(),
        Some("10-23 14:00:02.000")
    );
    // Away later, it continues after its rows as before.
    s.link.unplug("SER1");
    wait("waiting", || c.phase() == Phase::Waiting);
    s.link.log("SER1", &row(4, 0, "four"));
    s.link.plug("SER1");
    written(
        &c,
        &[row(1, 0, "one"), row(3, 0, "three"), row(4, 0, "four")].concat(),
    );
    assert!(!c.breaks()[1].gap && !c.breaks()[1].stopped);
    c.stop();
    assert_eq!(c.phase(), Phase::Stopped);
    // Closed, it never records again.
    c.close();
    assert!(c.resume().is_err());
}

#[test]
fn a_capture_resumed_without_its_device_waits_for_it() {
    let s = setup();
    let c = s.start("SER1");
    s.link.log("SER1", &row(1, 0, "one"));
    written(&c, &row(1, 0, "one"));
    c.stop();
    s.link.unplug("SER1");
    s.link.reboot("SER1");
    thread::sleep(Duration::from_millis(5));
    let resumed = chrono::Utc::now().timestamp_millis();
    c.resume().unwrap();
    assert_eq!(c.phase(), Phase::Waiting);
    // It waits from its resume, and says it is recording on from a stop.
    let stopped = c.json()["stopped"].as_i64().unwrap();
    assert!(stopped < resumed);
    assert!(c.json()["lost"].as_i64().unwrap() >= resumed);
    // Stopped again while waiting, it still breaks where it first stopped.
    c.stop();
    assert_eq!(c.json()["stopped"], stopped);
    assert_eq!(c.phase(), Phase::Stopped);
    c.resume().unwrap();
    s.link.log("SER1", &row(5, 0, "boot"));
    s.link.plug("SER1");
    wait("recording", || c.phase() == Phase::Recording);
    assert!(c.json()["stopped"].is_null());
    s.link.log("SER1", &row(6, 0, "back"));
    written(&c, &[row(1, 0, "one"), row(6, 0, "back")].concat());
    let breaks = c.breaks();
    assert_eq!(breaks.len(), 1);
    assert_eq!(
        (breaks[0].row, breaks[0].reboot, breaks[0].stopped),
        (2, true, true)
    );
    assert!(breaks[0].lost < resumed);
    c.stop();
}

#[test]
fn a_row_cut_short_by_stopping_ends_before_the_resumed_rows() {
    let s = setup();
    let c = s.start("SER1");
    s.link.log("SER1", &row(1, 0, "one"));
    s.link
        .log("SER1", "10-23 14:00:02.000  1234  5678 I Tag: cut");
    wait("received", || c.json()["rows"] == 1);
    thread::sleep(Duration::from_millis(20));
    c.stop();
    let cut = [
        row(1, 0, "one"),
        "10-23 14:00:02.000  1234  5678 I Tag: cut".into(),
    ]
    .concat();
    assert_eq!(content(&c), cut.as_bytes());
    // The device ends the row the closed stream cut.
    s.link.log("SER1", "\n");
    c.resume().unwrap();
    s.link.log("SER1", &row(3, 0, "three"));
    written(&c, &format!("{cut}\n{}", row(3, 0, "three")));
    assert_eq!(c.breaks()[0].row, 3);
    assert_eq!(c.rows(), 3);
    c.stop();
}

#[test]
fn an_iphone_capture_resumed_records_only_rows_logged_since() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().join("captures"));
    let link = FakeLink::live();
    link.add_ios("QA0UDID0000000000000001", "Test iPhone", "27.0");
    let slot = store.create(Platform::Ios, "Test iPhone").unwrap();
    let c = Capture::start(link.clone(), "QA0UDID0000000000000001", slot, options()).unwrap();
    let first = "2026-10-24 14:00:01.000001 SampleApp[812:4410] [INFO] [Sync] first\n";
    link.log("QA0UDID0000000000000001", first);
    written(&c, first);
    c.stop();
    link.log(
        "QA0UDID0000000000000001",
        "2026-10-24 14:00:02.000001 SampleApp[812:4410] [INFO] [Sync] stopped\n",
    );
    c.resume().unwrap();
    assert_eq!(c.phase(), Phase::Recording);
    let back = "2026-10-24 14:00:03.000001 SampleApp[812:4410] [INFO] [Sync] back\n";
    link.log("QA0UDID0000000000000001", back);
    written(&c, &format!("{first}{back}"));
    let b = c.breaks()[0];
    assert_eq!((b.row, b.gap, b.stopped), (2, true, true));
    c.stop();
}

#[test]
fn a_capture_left_by_an_earlier_run_does_not_resume() {
    let s = setup();
    let c = s.start("SER1");
    s.link.log("SER1", &row(1, 0, "one"));
    written(&c, &row(1, 0, "one"));
    c.stop();
    let path = c.path();
    drop(c);
    let again = Capture::adopt(s.store.adopt(&path).unwrap()).unwrap();
    assert!(again.resume().is_err());
    assert_eq!(again.phase(), Phase::Stopped);
}

#[test]
fn a_device_that_cannot_be_used_fails_at_once() {
    let s = setup();
    let fails = |serial: &str| {
        let slot = s.store.create(Platform::Android, "x").unwrap();
        let e = Capture::start(s.link.clone(), serial, slot, options())
            .err()
            .unwrap();
        public_message(&e, "")
    };
    assert_eq!(fails("NONE"), link::GONE);
    s.link.set_state("SER1", "unauthorized");
    assert_eq!(fails("SER1"), link::UNAUTHORIZED);
    s.link.set_state("SER1", "offline");
    assert_eq!(fails("SER1"), link::OFFLINE);
    s.link.set_state("SER1", "recovery");
    assert_eq!(fails("SER1"), link::NOT_READY);
    // No file is left of a capture that did not start.
    assert!(s.store.leftovers().is_empty());
}

#[test]
fn capture_files_held_by_a_running_app_are_no_leftovers() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().join("captures"));
    assert!(store.leftovers().is_empty());
    let a = store.create(Platform::Android, "Pixel 8").unwrap();
    let b = store.create(Platform::Android, "Pixel 8").unwrap();
    assert_ne!(a.path, b.path);
    let name = a.path.file_name().unwrap().to_str().unwrap().to_string();
    assert!(
        name.starts_with("logcat-Pixel_8-") && name.ends_with(".log"),
        "{name}"
    );
    assert!(store.leftovers().is_empty());
    std::fs::write(&a.path, b"rows\n").unwrap();
    let path = a.path.clone();
    drop(a);
    let left = store.leftovers();
    assert_eq!(left.len(), 1);
    assert_eq!((left[0].path.clone(), left[0].size), (path.clone(), 5));
    // A leftover shown again is held again.
    let shown = store.adopt(&path).unwrap();
    assert!(store.leftovers().is_empty());
    assert!(store.adopt(&path).is_err());
    drop(shown);
    store.remove(&path).unwrap();
    assert!(!path.exists());
    assert!(store.leftovers().is_empty());
    // Only capture files of the store can be removed.
    let outside = dir.path().join("logcat-other.log");
    std::fs::write(&outside, b"x").unwrap();
    assert!(store.remove(&outside).is_err());
    assert!(store.adopt(&outside).is_err());
    assert!(outside.exists());
    drop(b);
    assert_eq!(store.leftovers().len(), 1);
}

#[test]
fn an_engine_follows_a_capture_until_it_stops() {
    let s = setup();
    let e = Arc::new(Engine::default());
    let id = e.begin_open();
    let c = s.start("SER1");
    let status = e.open_capture(id, c.clone()).unwrap();
    assert_eq!(status["capture"]["state"], "recording");
    let state = || e.request(json!({"action":"status"})).unwrap();
    wait("following", || state()["following"] == true);
    e.request(json!({"datasetId":id,"queryId":1,"action":"query","filters":Filters::default()}))
        .unwrap();
    s.link
        .log("SER1", &[row(1, 0, "one"), row(2, 0, "two")].concat());
    // The product polls every 500 ms.
    wait("rows", || {
        let s = state();
        s["total"] == 2 && s["query"]["count"] == 2
    });
    assert_eq!(state()["capture"]["rows"], 2);
    s.link.log("SER1", &row(3, 0, "three"));
    let stopped = e.stop_capture(id).unwrap();
    assert_eq!(stopped["state"], "stopped");
    // The rows written last complete the log.
    wait("complete", || {
        let s = state();
        s["following"] == false && s["query"]["status"] == "complete" && s["total"] == 3
    });
    assert!(e.stop_capture(id + 1).is_err());
    e.close();
    assert!(e.capture(id).is_none());
}

#[test]
fn a_resumed_capture_log_follows_its_new_rows() {
    let s = setup();
    let e = Arc::new(Engine::default());
    let id = e.begin_open();
    let c = s.start("SER1");
    e.open_capture(id, c.clone()).unwrap();
    let state = || e.request(json!({"action":"status"})).unwrap();
    wait("following", || state()["following"] == true);
    e.request(json!({"datasetId":id,"queryId":1,"action":"query","filters":Filters::default()}))
        .unwrap();
    s.link
        .log("SER1", &[row(1, 0, "one"), row(2, 0, "two")].concat());
    wait("rows", || c.json()["rows"] == 2);
    e.stop_capture(id).unwrap();
    wait("complete", || {
        let s = state();
        s["following"] == false && s["query"]["status"] == "complete" && s["total"] == 2
    });
    assert!(e.resume_capture(id + 1).is_err());
    let status = e.resume_capture(id).unwrap();
    assert_eq!(status["state"], "recording");
    assert_eq!(state()["datasetId"], id);
    wait("following again", || state()["following"] == true);
    s.link.log("SER1", &row(3, 0, "three"));
    wait("new row", || {
        let s = state();
        s["total"] == 3 && s["query"]["count"] == 3 && s["capture"]["breaks"][0]["row"] == 3
    });
    // Recording, it cannot resume again.
    assert!(e.resume_capture(id).is_err());
    e.close();
    assert_eq!(c.phase(), Phase::Stopped);
    assert!(c.resume().is_err());
}

#[test]
fn closing_or_replacing_a_capture_log_stops_the_capture() {
    let s = setup();
    let e = Arc::new(Engine::default());
    let id = e.begin_open();
    let c = s.start("SER1");
    e.open_capture(id, c.clone()).unwrap();
    wait("following", || {
        e.request(json!({"action":"status"})).unwrap()["following"] == true
    });
    // Another log replaces it once that opens.
    let next = e.begin_open();
    e.open_text(next, "other\n", "other").unwrap();
    wait("replaced", || c.phase() == Phase::Stopped);
    assert!(e.capture(id).is_none());
    e.close();
}

#[test]
fn a_capture_log_kept_for_rollback_is_followed_again() {
    let s = setup();
    let e = Arc::new(Engine::default());
    let id = e.begin_open();
    let c = s.start("SER1");
    e.open_capture(id, c.clone()).unwrap();
    let state = || e.request(json!({"action":"status"})).unwrap();
    wait("following", || state()["following"] == true);
    // A file that fails to open restores the capture log.
    let dir = tempfile::tempdir().unwrap();
    let bad = dir.path().join("bad.log");
    std::fs::write(&bad, b"a\0b\n").unwrap();
    let next = e.begin_open();
    e.open_file(next, &bad).unwrap();
    wait("restored", || state()["datasetId"] == id);
    wait("following again", || state()["following"] == true);
    assert!(c.live());
    s.link.log("SER1", &row(1, 0, "one"));
    wait("row", || state()["total"] == 1);
    e.close();
    assert_eq!(c.phase(), Phase::Stopped);
}

#[test]
fn a_paused_capture_log_stays_paused_when_an_open_is_cancelled() {
    let s = setup();
    let e = Arc::new(Engine::default());
    let id = e.begin_open();
    let c = s.start("SER1");
    e.open_capture(id, c.clone()).unwrap();
    let state = || e.request(json!({"action":"status"})).unwrap();
    wait("following", || state()["following"] == true);
    e.request(json!({"datasetId":id,"action":"follow","follow":false}))
        .unwrap();
    // A file dialog opened and cancelled.
    let next = e.begin_open();
    e.cancel_open(next);
    thread::sleep(Duration::from_millis(30));
    assert_eq!(state()["following"], false);
    assert!(c.live());
    e.close();
}

#[test]
fn saving_copies_the_rows_so_far_while_recording_goes_on() {
    let s = setup();
    let c = s.start("SER1");
    let first = [row(1, 0, "one"), row(2, 0, "two")].concat();
    s.link.log("SER1", &first);
    written(&c, &first);
    wait("bytes", || c.bytes() == first.len() as u64);
    assert!(c.unsaved());
    let out = s._dir.path().join("saved.log");
    let never = std::sync::atomic::AtomicBool::new(false);
    assert_eq!(c.save(&out, &never).unwrap(), first.len() as u64);
    assert_eq!(std::fs::read(&out).unwrap(), first.as_bytes());
    assert!(!c.unsaved());
    assert_eq!(c.json()["unsaved"], false);
    s.link.log("SER1", &row(3, 0, "three"));
    wait("unsaved", || c.unsaved());
    // The capture file itself is no place to save to.
    let e = c.save(&c.path(), &never).unwrap_err();
    assert_eq!(
        public_message(&e, ""),
        "Choose a file other than the capture"
    );
    // A cancelled save leaves nothing behind.
    let cancelled = std::sync::atomic::AtomicBool::new(true);
    let other = s._dir.path().join("other.log");
    assert!(c.save(&other, &cancelled).is_err());
    assert!(!other.exists());
    let leftovers: Vec<_> = std::fs::read_dir(s._dir.path())
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().contains("partial"))
        .collect();
    assert!(leftovers.is_empty());
    c.stop();
}

#[test]
fn a_cut_capture_records_into_its_new_file_from_then_on() {
    let s = setup();
    let c = s.start("SER1");
    let first = [row(1, 0, "one"), row(2, 0, "two")].concat();
    s.link.log("SER1", &first);
    written(&c, &first);
    wait("rows", || c.rows() == 2);
    let never = std::sync::atomic::AtomicBool::new(false);
    c.save(&s._dir.path().join("kept.log"), &never).unwrap();
    // A row being received when the cut comes goes to the new file whole.
    let three = row(3, 0, "three");
    s.link.log("SER1", &three[..20]);
    thread::sleep(Duration::from_millis(30));
    let before = c.path();
    assert!(c.json()["cleared"].is_null());
    let (_, cuts) = c.next_row();
    let slot = s.store.create(Platform::Android, "Pixel 8").unwrap();
    let old = c.cut(Cut::open(slot).unwrap());
    assert_eq!(old.path, before);
    assert_ne!(c.path(), before);
    assert_eq!(std::fs::read(&before).unwrap(), first.as_bytes());
    // The new file starts empty and owes no save.
    assert_eq!((c.bytes(), c.rows()), (0, 0));
    assert!(!c.unsaved());
    assert_eq!(c.json()["saved"], 0);
    // The page marks the new file's start with when it was cleared.
    assert!(c.json()["cleared"].is_i64());
    assert_eq!(c.phase(), Phase::Recording);
    s.link.log("SER1", &three[20..]);
    s.link.log("SER1", &row(4, 0, "four"));
    written(&c, &[three, row(4, 0, "four")].concat());
    wait("rows", || c.rows() == 2);
    assert!(c.unsaved());
    assert_eq!(std::fs::read(&before).unwrap(), first.as_bytes());
    // An app start that began before the cut marks the new file's start.
    c.mark_launch(
        Launch {
            row: 3,
            at: 0,
            app: "SampleApp".into(),
            process: "SampleApp".into(),
        },
        cuts,
    );
    assert_eq!(c.launches()[0].row, 1);
    old.remove().unwrap();
    let path = c.path();
    c.close();
    drop(c);
    // Its rows were never saved, so the new file stays for the next run.
    assert!(path.exists());
    assert_eq!(s.store.leftovers().len(), 1);
}

#[test]
fn a_capture_cut_while_its_device_is_away_records_from_its_return() {
    let s = setup();
    let c = s.start("SER1");
    s.link.log("SER1", &row(1, 0, "one"));
    written(&c, &row(1, 0, "one"));
    s.link.unplug("SER1");
    wait("waiting", || c.phase() == Phase::Waiting);
    // The device logs while away, then the capture is cut.
    s.link.log("SER1", &row(2, 0, "away"));
    let slot = s.store.create(Platform::Android, "Pixel 8").unwrap();
    let old = c.cut(Cut::open(slot).unwrap());
    assert_eq!(c.phase(), Phase::Waiting);
    s.link.plug("SER1");
    wait("recording again", || c.phase() == Phase::Recording);
    s.link.log("SER1", &row(3, 0, "back"));
    written(&c, &row(3, 0, "back"));
    let breaks = c.breaks();
    assert_eq!(breaks.len(), 1);
    assert_eq!((breaks[0].row, breaks[0].gap), (1, true));
    // It starts after the rows the device held, as a new capture does.
    assert_eq!(s.link.opened()[1].1.as_deref(), Some("10-23 14:00:02.000"));
    // Away again, it continues after its rows as before.
    s.link.unplug("SER1");
    wait("waiting", || c.phase() == Phase::Waiting);
    s.link.log("SER1", &row(4, 0, "four"));
    s.link.plug("SER1");
    written(&c, &[row(3, 0, "back"), row(4, 0, "four")].concat());
    assert!(!c.breaks()[1].gap);
    c.stop();
    old.remove().unwrap();
}

#[test]
fn clearing_a_capture_log_shows_its_capture_anew() {
    let s = setup();
    let e = Arc::new(Engine::default());
    let id = e.begin_open();
    let c = s.start("SER1");
    e.open_capture(id, c.clone()).unwrap();
    let state = || e.request(json!({"action":"status"})).unwrap();
    s.link
        .log("SER1", &[row(1, 0, "one"), row(2, 0, "two")].concat());
    wait("rows", || state()["total"] == 2);
    let before = c.path();
    let cut = |s: &Setup| Cut::open(s.store.create(Platform::Android, "Pixel 8").unwrap()).unwrap();
    // Only the shown log's recording capture is cleared.
    let next = e.begin_open();
    assert!(e.clear_capture(id + 7, next, cut(&s)).is_err());
    let next = e.begin_open();
    let status = e.clear_capture(id, next, cut(&s)).unwrap();
    assert_eq!(status["datasetId"], next);
    wait("cleared", || {
        let s = state();
        s["datasetId"] == next && s["following"] == true && s["capture"]["rows"] == 0
    });
    assert!(e.cleared_from(next).is_none());
    assert!(e.capture(id).is_none());
    assert!(Arc::ptr_eq(&e.capture(next).unwrap(), &c));
    assert!(c.live());
    // The rows before go with the log that showed them.
    assert!(!before.exists());
    s.link.log("SER1", &row(3, 0, "three"));
    wait("new row", || {
        let s = state();
        s["total"] == 1 && s["capture"]["rows"] == 1
    });
    // A clear that does not open leaves the capture in its file.
    let path = c.path();
    let stale = e.begin_open();
    e.begin_open();
    assert!(e.clear_capture(next, stale, cut(&s)).is_err());
    assert_eq!(c.path(), path);
    assert!(c.live());
    assert_eq!(s.store.leftovers().len(), 0);
    e.close();
    assert_eq!(c.phase(), Phase::Stopped);
    drop(c);
    // Its row was never saved, so its file stays for the next run.
    assert_eq!(s.store.leftovers().len(), 1);
}

#[test]
fn a_discarded_capture_deletes_its_file_once_its_log_closes() {
    let s = setup();
    let e = Arc::new(Engine::default());
    let id = e.begin_open();
    let c = s.start("SER1");
    let path = c.path().to_path_buf();
    e.open_capture(id, c.clone()).unwrap();
    wait("following", || {
        e.request(json!({"action":"status"})).unwrap()["following"] == true
    });
    c.discard(true);
    // An open that fails restores the log, and its file stays.
    let dir = tempfile::tempdir().unwrap();
    let bad = dir.path().join("bad.log");
    std::fs::write(&bad, b"a\0b\n").unwrap();
    let next = e.begin_open();
    e.open_file(next, &bad).unwrap();
    wait("restored", || {
        e.request(json!({"action":"status"})).unwrap()["datasetId"] == id
    });
    assert!(path.exists());
    e.close();
    assert!(!path.exists());
    assert!(s.store.leftovers().is_empty());
}

#[test]
fn a_kept_capture_file_is_left_for_the_next_run() {
    let s = setup();
    let e = Arc::new(Engine::default());
    let id = e.begin_open();
    let c = s.start("SER1");
    s.link.log("SER1", &row(1, 0, "one"));
    written(&c, &row(1, 0, "one"));
    e.open_capture(id, c.clone()).unwrap();
    e.close();
    drop(c);
    let left = s.store.leftovers();
    assert_eq!(left.len(), 1);
    // Shown again, it is a stopped capture of all its rows.
    let again = Capture::adopt(s.store.adopt(&left[0].path).unwrap()).unwrap();
    assert!(again.unsaved() && !again.live());
    assert_eq!(again.json()["recovered"], true);
    let id = e.begin_open();
    let status = e.open_capture(id, again.clone()).unwrap();
    assert_eq!(status["capture"]["state"], "stopped");
    wait("rows", || {
        let s = e.request(json!({"action":"status"})).unwrap();
        s["total"] == 1 && s["following"] == false && s["textComplete"] == true
    });
    assert!(s.store.leftovers().is_empty());
    again.discard(true);
    e.close();
    assert!(!left[0].path.exists());
}

#[test]
fn closing_deletes_a_capture_file_that_holds_nothing_unsaved() {
    let s = setup();
    let never = std::sync::atomic::AtomicBool::new(false);
    // Every row saved.
    let c = s.start("SER1");
    s.link.log("SER1", &row(1, 0, "one"));
    written(&c, &row(1, 0, "one"));
    wait("bytes", || c.bytes() > 0);
    c.save(&s._dir.path().join("kept.log"), &never).unwrap();
    let path = c.path().to_path_buf();
    c.close();
    assert!(!path.exists());
    // No row came.
    let c = s.start("SER1");
    let path = c.path().to_path_buf();
    c.close();
    assert!(!path.exists());
    assert!(s.store.leftovers().is_empty());
}

#[test]
fn a_paused_capture_log_reads_its_last_rows_when_the_capture_stops() {
    let s = setup();
    let e = Arc::new(Engine::default());
    let id = e.begin_open();
    let c = s.start("SER1");
    e.open_capture(id, c.clone()).unwrap();
    let state = || e.request(json!({"action":"status"})).unwrap();
    wait("following", || state()["following"] == true);
    e.request(json!({"datasetId":id,"action":"follow","follow":false}))
        .unwrap();
    s.link
        .log("SER1", &[row(1, 0, "one"), row(2, 0, "two")].concat());
    wait("written", || c.json()["rows"] == 2);
    e.stop_capture(id).unwrap();
    wait("complete", || {
        let s = state();
        s["total"] == 2 && s["following"] == false && s["grown"] == false
    });
    e.close();
}

#[test]
fn a_waiting_capture_says_why_a_listed_device_cannot_be_used_yet() {
    let s = setup();
    let c = s.start("SER1");
    s.link.log("SER1", &row(1, 0, "one"));
    written(&c, &row(1, 0, "one"));
    s.link.unplug("SER1");
    wait("waiting", || c.phase() == Phase::Waiting);
    assert_eq!(c.json()["reason"], "");
    // Back after a reboot, it waits for USB debugging to be allowed again.
    s.link.reboot("SER1");
    s.link.set_state("SER1", "unauthorized");
    s.link.plug("SER1");
    wait("reason", || c.json()["reason"] == link::UNAUTHORIZED);
    assert_eq!(c.phase(), Phase::Waiting);
    s.link.log("SER1", &row(2, 0, "after"));
    s.link.set_state("SER1", "device");
    written(&c, &[row(1, 0, "one"), row(2, 0, "after")].concat());
    assert_eq!(c.json()["reason"], "");
    c.stop();
}

#[test]
fn an_iphone_capture_records_only_rows_logged_while_connected() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().join("captures"));
    let link = FakeLink::live();
    link.add_ios("QA0UDID0000000000000001", "Test iPhone", "27.0");
    let held = "2026-10-24 14:00:00.000001 SampleApp[812:4410] [INFO] [Sync] before\n";
    link.log("QA0UDID0000000000000001", held);
    let slot = store.create(Platform::Ios, "Test iPhone").unwrap();
    let name = slot
        .path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert!(name.starts_with("ios-Test_iPhone-"), "{name}");
    let c = Capture::start(link.clone(), "QA0UDID0000000000000001", slot, options()).unwrap();
    assert_eq!(c.json()["device"], "Test iPhone");
    let first = "2026-10-24 14:00:01.000001 SampleApp[812:4410] [INFO] [Sync] first\n";
    link.log("QA0UDID0000000000000001", first);
    written(&c, first);
    link.unplug("QA0UDID0000000000000001");
    wait("waiting", || c.phase() == Phase::Waiting);
    let away = "2026-10-24 14:00:02.000001 SampleApp[812:4410] [INFO] [Sync] away\n";
    link.log("QA0UDID0000000000000001", away);
    link.plug("QA0UDID0000000000000001");
    wait("recording again", || c.phase() == Phase::Recording);
    let back = "2026-10-24 14:00:03.000001 SampleApp[812:4410] [INFO] [Sync] back\n";
    link.log("QA0UDID0000000000000001", back);
    // The rows logged before the start and while away never come.
    written(&c, &format!("{first}{back}"));
    let breaks = c.breaks();
    assert_eq!(breaks.len(), 1);
    assert_eq!(
        (breaks[0].row, breaks[0].reboot, breaks[0].gap),
        (2, false, true)
    );
    assert_eq!(c.json()["breaks"][0]["gap"], true);
    assert_eq!(link.opened().len(), 2);
    assert!(link.opened().iter().all(|(_, since)| since.is_none()));
    c.stop();
}

#[test]
fn an_untrusted_iphone_is_waited_for_until_it_trusts_the_computer() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().join("captures"));
    let link = FakeLink::live();
    link.add_ios("QA0UDID0000000000000001", "Test iPhone", "27.0");
    link.set_state("QA0UDID0000000000000001", "untrusted");
    let slot = store.create(Platform::Ios, "Test iPhone").unwrap();
    let path = slot.path.clone();
    let Err(err) = Capture::start(link.clone(), "QA0UDID0000000000000001", slot, options()) else {
        panic!("an untrusted iPhone was captured");
    };
    assert_eq!(public_message(&err, ""), link::UNTRUSTED);
    assert!(
        !path.exists(),
        "a capture that did not start leaves no file"
    );
    link.set_state("QA0UDID0000000000000001", "device");
    let slot = store.create(Platform::Ios, "Test iPhone").unwrap();
    let c = Capture::start(link.clone(), "QA0UDID0000000000000001", slot, options()).unwrap();
    link.unplug("QA0UDID0000000000000001");
    wait("waiting", || c.phase() == Phase::Waiting);
    link.set_state("QA0UDID0000000000000001", "locked");
    link.plug("QA0UDID0000000000000001");
    wait("asks to unlock", || c.json()["reason"] == link::LOCKED);
    link.set_state("QA0UDID0000000000000001", "device");
    wait("recording again", || c.phase() == Phase::Recording);
    c.stop();
}

#[test]
fn synthetic_iphone_rows_parse_as_ios_rows() {
    use logstudio_core::{capture::fake::synthetic_ios_row, parser::parse_line};
    let levels: Vec<&str> = (0..6)
        .map(|n| {
            let row = synthetic_ios_row(n);
            let p = parse_line(row.trim_end(), 1, false).parsed.unwrap();
            assert!(p.ios && p.pid == "812", "{row}");
            match p.level {
                "I" => "I",
                "D" => "D",
                "E" => "E",
                "F" => "F",
                other => panic!("{other} in {row}"),
            }
        })
        .collect();
    assert_eq!(levels, ["I", "D", "I", "E", "E", "F"]);
}

#[test]
fn a_scripted_developer_build_started_again_shows_its_private_values() {
    use logstudio_core::capture::apps::{Apps, NOT_LISTED};
    const IPHONE: &str = "QA0UDID0000000000000001";
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().join("captures"));
    let link = FakeLink::live();
    link.add_ios(IPHONE, "Test iPhone", "27.0");
    link.add_app(IPHONE, "com.example.sample", "SampleApp", "1.4.0");
    let feeding = link.feed(IPHONE, Duration::from_millis(1));
    let slot = store.create(Platform::Ios, "Test iPhone").unwrap();
    let c = Capture::start(link.clone(), IPHONE, slot, options()).unwrap();
    let text = || String::from_utf8_lossy(&content(&c)).into_owned();
    // The redacted row itself: the feed's first rows from the app hold no
    // value, so starting it after them can leave none before its values.
    wait("a redacted row", || text().contains("to <private> failed"));
    assert_eq!(c.serial(), IPHONE);
    let apps = link.apps(IPHONE).unwrap();
    assert_eq!(
        (apps[0].name.as_str(), apps[0].executable.as_str()),
        ("SampleApp", "SampleApp")
    );
    let missing = link.launch(IPHONE, "com.example.other").unwrap_err();
    assert_eq!(public_message(&missing, ""), NOT_LISTED);
    // As the desktop does: the row after those written when the start
    // began is marked once the app has started.
    let (row, cuts) = c.next_row();
    link.launch(IPHONE, "com.example.sample").unwrap();
    c.mark_launch(
        Launch {
            row,
            at: 1_792_818_131_000,
            app: "SampleApp".into(),
            process: "SampleApp".into(),
        },
        cuts,
    );
    wait("a row with its value", || {
        text().contains("request ") && text().contains("to api.example.com")
    });
    feeding.store(true, std::sync::atomic::Ordering::Release);
    c.stop();
    let text = text();
    let shown = text.find("to api.example.com").unwrap();
    let before = &text[..shown];
    assert!(before.contains("to <private> failed"), "{before}");
    // The app started again is a new process; others keep their IDs.
    let after = &text[text[..shown].rfind('\n').map_or(0, |i| i + 1)..];
    assert!(
        after
            .lines()
            .filter(|l| l.contains("SampleApp["))
            .all(|l| l.contains("SampleApp[913:")),
        "{after}"
    );
    assert!(!after.contains("<private>"), "{after}");
    // The mark comes before the first row with a value; the file holds
    // only the device's rows.
    let first_shown = text[..shown].matches('\n').count() as u64 + 1;
    assert!(
        row <= first_shown,
        "mark {row}, first shown row {first_shown}"
    );
    assert_eq!(
        c.json()["launches"],
        json!([{"row":row,"at":1_792_818_131_000i64,"app":"SampleApp","process":"SampleApp"}])
    );
    assert_eq!(c.launches().len(), 1);
    assert_eq!(
        link.launched(),
        [(IPHONE.to_string(), "com.example.sample".to_string())]
    );
}
