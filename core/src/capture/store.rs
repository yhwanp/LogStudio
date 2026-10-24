//! Capture files are temporary until the user saves them. Each holds a
//! lock while its app runs, so files without one were left by a crash.
use crate::source::error;
use std::{
    fs::{File, OpenOptions},
    io,
    path::{Path, PathBuf},
    time::SystemTime,
};

pub struct Store {
    dir: PathBuf,
}
/// A new capture file and the lock that marks it in use.
pub struct Slot {
    pub path: PathBuf,
    _lock: File,
}
impl Slot {
    /// Deletes the file and its lock; open handles keep reading it.
    pub fn remove(&self) -> io::Result<()> {
        remove_with_lock(&self.path)
    }
}
/// A capture file no running app holds.
#[derive(Clone, Debug)]
pub struct Leftover {
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
    pub modified: Option<SystemTime>,
}
const PREFIX: &str = "logcat-";
const EXTENSION: &str = "log";
/// The lock sits beside the file: a lock on the file itself would keep
/// Windows readers from its bytes.
fn lock_path(path: &Path) -> PathBuf {
    path.with_extension("lock")
}
/// Takes the lock of a capture file; None while another app holds it.
fn hold(path: &Path) -> io::Result<Option<File>> {
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(lock_path(path))?;
    Ok(lock.try_lock().is_ok().then_some(lock))
}
fn remove_with_lock(path: &Path) -> io::Result<()> {
    std::fs::remove_file(path)?;
    let _ = std::fs::remove_file(lock_path(path));
    Ok(())
}
/// Characters a file name keeps of a device name.
fn clean(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(40)
        .collect();
    if s.trim_matches('_').is_empty() {
        "device".into()
    } else {
        s
    }
}
impl Store {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }
    /// An empty capture file named for the device and the local time.
    pub fn create(&self, device: &str) -> io::Result<Slot> {
        std::fs::create_dir_all(&self.dir)?;
        let time = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let stem = format!("{PREFIX}{}-{time}", clean(device));
        for n in 1..100 {
            let name = match n {
                1 => format!("{stem}.{EXTENSION}"),
                n => format!("{stem}-{n}.{EXTENSION}"),
            };
            let path = self.dir.join(name);
            let Some(lock) = hold(&path)? else {
                continue;
            };
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(_) => return Ok(Slot { path, _lock: lock }),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
        Err(error("Could not create a capture file"))
    }
    /// Holds a leftover capture file again, as while it is shown.
    pub fn adopt(&self, path: &Path) -> io::Result<Slot> {
        if !self.owns(path) {
            return Err(error("Not a capture file"));
        }
        let lock = hold(path)?.ok_or_else(|| error("The capture file is in use"))?;
        Ok(Slot {
            path: path.to_path_buf(),
            _lock: lock,
        })
    }
    /// Capture files whose lock no running app holds, oldest first.
    pub fn leftovers(&self) -> Vec<Leftover> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return vec![];
        };
        let mut found: Vec<Leftover> = entries
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                let name = path.file_name()?.to_str()?.to_string();
                if !self.owns(&path) {
                    return None;
                }
                let meta = std::fs::symlink_metadata(&path).ok()?;
                // Held only to see that no running app holds it.
                hold(&path).ok().flatten()?;
                Some(Leftover {
                    path,
                    name,
                    size: meta.len(),
                    modified: meta.modified().ok(),
                })
            })
            .collect();
        found.sort_by_key(|l| l.modified);
        found
    }
    /// Whether `path` names a regular capture file of this store.
    pub fn owns(&self, path: &Path) -> bool {
        path.parent() == Some(self.dir.as_path())
            && path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(PREFIX))
            && path.extension().is_some_and(|e| e == EXTENSION)
            && std::fs::symlink_metadata(path).is_ok_and(|m| m.is_file())
    }
    /// Deletes a capture file of this store and its lock.
    pub fn remove(&self, path: &Path) -> io::Result<()> {
        if !self.owns(path) {
            return Err(error("Not a capture file"));
        }
        remove_with_lock(path)
    }
}
