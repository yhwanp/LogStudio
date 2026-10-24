use logstudio_core::capture::adb::{candidates, parse_version, Candidate, Origin};
use std::{
    collections::HashMap,
    ffi::OsString,
    path::{Path, PathBuf},
};
// Running programs is checked with shell scripts, so on Unix only.
#[cfg(unix)]
use logstudio_core::capture::adb::{check, locate};
#[cfg(unix)]
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

const VERSION: &str = "Android Debug Bridge version 1.0.41\n\
    Version 37.0.1-15733141\n\
    Installed as /opt/sdk/platform-tools/adb\n\
    Running on Darwin 27.0.0 (arm64)\n";

#[test]
fn adb_versions_are_read_from_its_output() {
    assert_eq!(parse_version(VERSION).as_deref(), Some("37.0.1"));
    assert_eq!(
        parse_version("Android Debug Bridge version 1.0.32\n").as_deref(),
        Some("1.0.32")
    );
    assert_eq!(
        parse_version("Android Debug Bridge version 1.0.41\r\nVersion 35.0.2\r\n").as_deref(),
        Some("35.0.2")
    );
    assert_eq!(parse_version("adb: command not found\n"), None);
    assert_eq!(parse_version(""), None);
}

fn env(pairs: &[(&str, OsString)]) -> impl Fn(&str) -> Option<OsString> {
    let map: HashMap<String, OsString> = pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect();
    move |k| map.get(k).cloned()
}
fn exe(dir: &Path) -> PathBuf {
    dir.join(if cfg!(windows) { "adb.exe" } else { "adb" })
}

#[test]
fn candidates_follow_the_choice_the_sdk_android_studio_path_and_homebrew() {
    let root = std::env::temp_dir().join("logstudio-adb-candidates");
    let (chosen, sdk, user) = (root.join("chosen/adb"), root.join("sdk"), root.join("user"));
    let (first, second) = (root.join("bin1"), root.join("bin2"));
    let path = std::env::join_paths([
        first.clone(),
        PathBuf::from("relative"),
        second.clone(),
        first.clone(),
    ])
    .unwrap();
    let var = env(&[
        ("ANDROID_HOME", sdk.clone().into()),
        ("ANDROID_SDK_ROOT", sdk.clone().into()),
        ("HOME", user.clone().into()),
        ("LOCALAPPDATA", user.clone().into()),
        ("PATH", path),
    ]);
    let found = candidates(Some(&chosen), var);
    let tools = |sdk: PathBuf| exe(&sdk.join("platform-tools"));
    let studio = if cfg!(target_os = "macos") {
        user.join("Library/Android/sdk")
    } else if cfg!(windows) {
        user.join("Android").join("Sdk")
    } else {
        user.join("Android/Sdk")
    };
    // Homebrew's places are fixed and only macOS has them.
    let homebrew: Vec<(PathBuf, Origin)> = if cfg!(target_os = "macos") {
        ["/opt/homebrew/bin/adb", "/usr/local/bin/adb"]
            .map(|p| (PathBuf::from(p), Origin::Homebrew))
            .into()
    } else {
        vec![]
    };
    let mut expected = vec![
        (chosen, Origin::Chosen),
        (tools(sdk), Origin::Sdk),
        (tools(studio), Origin::Studio),
        (exe(&first), Origin::Path),
        (exe(&second), Origin::Path),
    ];
    expected.extend(homebrew.clone());
    let pairs = |list: Vec<Candidate>| -> Vec<(PathBuf, Origin)> {
        list.into_iter().map(|c| (c.path, c.origin)).collect()
    };
    assert_eq!(pairs(found), expected);
    // Nothing set leaves only the fixed places.
    assert_eq!(pairs(candidates(None, |_| None)), homebrew);
}

#[cfg(unix)]
fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[cfg(unix)]
#[test]
fn an_adb_is_the_first_candidate_that_runs_as_one() {
    use logstudio_core::capture::adb::Candidate;
    let dir = tempfile::tempdir().unwrap();
    let other = script(dir.path(), "other", "echo something else");
    let failing = script(
        dir.path(),
        "failing",
        &format!("printf '{VERSION}'; exit 1"),
    );
    let adb = script(dir.path(), "adb", &format!("printf '{VERSION}'"));
    let c = |path: &Path, origin| Candidate {
        path: path.to_path_buf(),
        origin,
    };
    let list = [
        c(&dir.path().join("missing"), Origin::Chosen),
        c(dir.path(), Origin::Sdk),
        c(&other, Origin::Studio),
        c(&failing, Origin::Path),
        c(&adb, Origin::Path),
    ];
    let found = locate(&list, Duration::from_secs(5)).unwrap();
    assert_eq!(
        (found.path, found.origin, found.version.as_str()),
        (adb, Origin::Path, "37.0.1")
    );
}

#[cfg(unix)]
#[test]
fn a_candidate_that_does_not_answer_in_time_is_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let slow = script(dir.path(), "slow", "sleep 30");
    let at = Instant::now();
    assert_eq!(check(&slow, Duration::from_millis(300)), None);
    assert!(at.elapsed() < Duration::from_secs(5));
    // A process it leaves behind holding the output does not keep it waiting.
    let lingering = script(
        dir.path(),
        "lingering",
        &format!("printf '{VERSION}'; (sleep 30 &)"),
    );
    let at = Instant::now();
    assert_eq!(check(&lingering, Duration::from_millis(500)), None);
    assert!(at.elapsed() < Duration::from_secs(5));
}

/// A synthetic adb that answers from files in its folder.
#[cfg(unix)]
const FAKE_ADB: &str = r#"
dir="$(dirname "$0")"
case "$1" in
  start-server)
    [ -f "$dir/mismatch" ] && echo "adb server version (40) doesn't match this client (41); killing..." >&2
    exit 0;;
  devices) cat "$dir/devices"; exit 0;;
  -s)
    serial="$2"; shift 2
    case "$serial" in
      GONE) echo "adb: device 'GONE' not found" >&2; exit 1;;
      LOCKED) echo "adb: device unauthorized." >&2; exit 1;;
      AWAY) echo "adb: device offline" >&2; exit 1;;
    esac
    case "$1 $2" in
      "shell cat /proc/sys/kernel/random/boot_id") echo boot-1;;
      "shell getprop ro.serialno") echo SER1;;
      "logcat -d")
        echo "${ANDROID_LOG_TAGS-unset}" > "$dir/log-tags"
        echo "--------- beginning of main"
        echo "10-23 14:00:01.000  1  2 I T: old";;
      "logcat -v")
        echo "$@" > "$dir/stream-args"
        i=1; while :; do echo "10-23 14:00:05.000  1  2 I T: live $i"; i=$((i+1)); sleep 0.01; done;;
    esac;;
esac
"#;

#[cfg(unix)]
fn fake_adb() -> (tempfile::TempDir, logstudio_core::capture::cli::AdbCli) {
    let dir = tempfile::tempdir().unwrap();
    let adb = script(dir.path(), "adb", FAKE_ADB);
    std::fs::write(
        dir.path().join("devices"),
        "List of devices attached\nSER1  device product:p model:Pixel_8 transport_id:1\nLOCKED  unauthorized transport_id:2\n\n",
    )
    .unwrap();
    (dir, logstudio_core::capture::cli::AdbCli::new(&adb))
}

#[cfg(unix)]
#[test]
fn the_adb_command_line_answers_device_queries_and_names_failures() {
    use logstudio_core::{
        capture::link::{self, Link},
        source::public_message,
    };
    let (dir, adb) = fake_adb();
    let devices = adb.devices().unwrap();
    assert_eq!(
        devices
            .iter()
            .map(|d| (d.serial.as_str(), d.state.as_str()))
            .collect::<Vec<_>>(),
        [("SER1", "device"), ("LOCKED", "unauthorized")]
    );
    assert_eq!(adb.boot_id("SER1").unwrap(), "boot-1");
    assert_eq!(adb.identity("SER1").unwrap(), "SER1");
    let failure = |e: std::io::Error| public_message(&e, "other");
    assert_eq!(failure(adb.boot_id("GONE").unwrap_err()), link::GONE);
    assert_eq!(
        failure(adb.identity("LOCKED").unwrap_err()),
        link::UNAUTHORIZED
    );
    assert_eq!(failure(adb.recent("AWAY", 10).unwrap_err()), link::OFFLINE);
    // adb replacing a server of another version is reported.
    std::fs::write(dir.path().join("mismatch"), "").unwrap();
    assert_eq!(failure(adb.devices().unwrap_err()), link::CONFLICT);
}

#[cfg(unix)]
#[test]
fn a_capture_through_the_adb_command_line_records_until_stopped() {
    use logstudio_core::capture::{store::Store, Capture, Options};
    // Set for this test only; the adb commands must not pass it on.
    struct Unset(&'static str);
    impl Drop for Unset {
        fn drop(&mut self) {
            std::env::remove_var(self.0);
        }
    }
    std::env::set_var("ANDROID_LOG_TAGS", "*:S");
    let _unset = Unset("ANDROID_LOG_TAGS");
    let (dir, adb) = fake_adb();
    let slot = Store::new(dir.path().join("captures"))
        .create("Pixel 8")
        .unwrap();
    let c = Capture::start(Arc::new(adb), "SER1", slot, Options::default()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while c.json()["rows"].as_u64().unwrap() < 5 {
        assert!(Instant::now() < deadline, "no rows");
        std::thread::sleep(Duration::from_millis(5));
    }
    let args = std::fs::read_to_string(dir.path().join("stream-args")).unwrap();
    assert_eq!(args.trim(), "logcat -v threadtime -T 10-23 14:00:01.000");
    // The app's own environment does not filter the device's rows.
    let tags = std::fs::read_to_string(dir.path().join("log-tags")).unwrap();
    assert_eq!(tags.trim(), "unset");
    let at = Instant::now();
    c.stop();
    assert!(at.elapsed() < Duration::from_secs(2), "{:?}", at.elapsed());
    let text = std::fs::read_to_string(c.path()).unwrap();
    assert!(
        text.starts_with("10-23 14:00:05.000  1  2 I T: live 1\n"),
        "{text:.80}"
    );
    assert!(!text.contains("old"));
}
