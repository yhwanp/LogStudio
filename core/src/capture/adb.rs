//! Finds the adb that captures use. Apps started from Finder or the Start
//! menu do not see a shell's PATH, so the places that installers put adb
//! are looked at as well. A candidate counts only if it runs as an adb.
use super::process::run;
use serde_json::{json, Value};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

/// Where a candidate comes from, as the page names it; paths stay native.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// The file the user picked.
    Chosen,
    /// `ANDROID_HOME` or `ANDROID_SDK_ROOT`.
    Sdk,
    /// Android Studio's default SDK folder.
    Studio,
    Path,
    Homebrew,
}
impl Origin {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Chosen => "chosen",
            Self::Sdk => "sdk",
            Self::Studio => "studio",
            Self::Path => "path",
            Self::Homebrew => "homebrew",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub path: PathBuf,
    pub origin: Origin,
}
#[cfg(windows)]
const EXE: &str = "adb.exe";
#[cfg(not(windows))]
const EXE: &str = "adb";
/// Candidates in the order they are tried, with environment `var`.
/// Relative PATH entries are skipped: they would run whatever the current
/// folder holds.
pub fn candidates(chosen: Option<&Path>, var: impl Fn(&str) -> Option<OsString>) -> Vec<Candidate> {
    let mut found: Vec<Candidate> = vec![];
    let mut add = |path: PathBuf, origin| {
        if path.is_absolute() && !found.iter().any(|c| c.path == path) {
            found.push(Candidate { path, origin });
        }
    };
    if let Some(path) = chosen {
        add(path.to_path_buf(), Origin::Chosen);
    }
    let tools = |sdk: PathBuf| sdk.join("platform-tools").join(EXE);
    for key in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Some(sdk) = var(key).filter(|v| !v.is_empty()) {
            add(tools(sdk.into()), Origin::Sdk);
        }
    }
    #[cfg(target_os = "macos")]
    if let Some(home) = var("HOME") {
        add(
            tools(PathBuf::from(home).join("Library/Android/sdk")),
            Origin::Studio,
        );
    }
    #[cfg(windows)]
    if let Some(local) = var("LOCALAPPDATA") {
        add(
            tools(PathBuf::from(local).join("Android").join("Sdk")),
            Origin::Studio,
        );
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    if let Some(home) = var("HOME") {
        add(
            tools(PathBuf::from(home).join("Android/Sdk")),
            Origin::Studio,
        );
    }
    if let Some(path) = var("PATH") {
        for dir in std::env::split_paths(&path) {
            add(dir.join(EXE), Origin::Path);
        }
    }
    #[cfg(target_os = "macos")]
    for dir in ["/opt/homebrew/bin", "/usr/local/bin"] {
        add(Path::new(dir).join(EXE), Origin::Homebrew);
    }
    found
}
/// The package version `adb version` reports, such as `37.0.1`, or its
/// protocol version from older adbs; None for other programs.
pub fn parse_version(output: &str) -> Option<String> {
    let mut lines = output.lines();
    let protocol = lines
        .next()?
        .trim()
        .strip_prefix("Android Debug Bridge version ")?;
    let package = lines
        .find_map(|l| l.trim().strip_prefix("Version "))
        .and_then(|v| v.split('-').next())
        .filter(|v| !v.is_empty());
    Some(package.unwrap_or(protocol).to_string())
}
/// An adb found and run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Adb {
    pub path: PathBuf,
    pub origin: Origin,
    pub version: String,
}
impl Adb {
    /// What the page shows: no path.
    pub fn json(&self) -> Value {
        json!({"origin":self.origin.as_str(),"version":self.version})
    }
}
/// How long `adb version` may take; a first run can be slow to start.
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(5);
/// Runs `adb version`; the version when the file is an adb that answers
/// within `timeout`.
pub fn check(path: &Path, timeout: Duration) -> Option<String> {
    if !std::fs::metadata(path).is_ok_and(|m| m.is_file()) {
        return None;
    }
    let mut version = command(path);
    version.arg("version");
    let out = run(version, timeout, 64 * 1024).ok()?;
    if !out.success {
        return None;
    }
    parse_version(&String::from_utf8_lossy(&out.stdout))
}
/// An adb command without input or a console window. `ANDROID_LOG_TAGS` in
/// this app's environment would filter what `logcat` shows, so it is
/// removed.
pub(super) fn command(adb: &Path) -> Command {
    let mut command = super::process::command(adb);
    command.env_remove("ANDROID_LOG_TAGS");
    command
}
/// The first candidate that runs as an adb.
pub fn locate(candidates: &[Candidate], timeout: Duration) -> Option<Adb> {
    candidates.iter().find_map(|c| {
        Some(Adb {
            path: c.path.clone(),
            origin: c.origin,
            version: check(&c.path, timeout)?,
        })
    })
}
