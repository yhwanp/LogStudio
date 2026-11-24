//! A file written beside its destination, which it replaces only once whole.
//! Each write makes a new file under a name of its own, so that a file of
//! the user's or of another running app is never written or deleted; one a
//! crash left cannot be told apart from those, and stays.
use crate::source::error;
use std::{
    fs::{File, OpenOptions},
    io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

/// Deleted when dropped, unless it replaced its destination.
pub(crate) struct Partial {
    /// None once closed; the file is this value's to delete until it moved.
    file: Option<File>,
    path: PathBuf,
}
impl Partial {
    /// A new empty file in `destination`'s folder.
    pub fn beside(destination: &Path) -> io::Result<Self> {
        loop {
            // Short, as the destination's name may be as long as names get.
            let name = format!(
                ".logstudio-{:x}-{:x}-{:x}.partial",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |d| d.as_nanos() as u64),
                NEXT.fetch_add(1, Ordering::Relaxed),
            );
            let path = destination.with_file_name(name);
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => {
                    return Ok(Self {
                        file: Some(file),
                        path,
                    })
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
    }
    pub fn file(&self) -> &File {
        self.file.as_ref().expect("a partial file is open")
    }
    /// Replaces `destination` with the file, written in full, unless it was
    /// cancelled by then; the error is `cancelled`.
    pub fn replace(
        mut self,
        destination: &Path,
        cancel: &AtomicBool,
        cancelled: &'static str,
    ) -> io::Result<()> {
        self.file().sync_all()?;
        // The app's end does not wait for a disk that takes long.
        if cancel.load(Ordering::Acquire) {
            return Err(error(cancelled));
        }
        // Windows moves no file that is open for writing.
        self.file = None;
        std::fs::rename(&self.path, destination)?;
        self.path.clear();
        Ok(())
    }
}
impl Drop for Partial {
    fn drop(&mut self) {
        self.file = None;
        if !self.path.as_os_str().is_empty() {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn a_file_cancelled_once_written_replaces_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let to = dir.path().join("out.log");
        std::fs::write(&to, b"before").unwrap();
        let write = |cancelled| {
            let partial = Partial::beside(&to).unwrap();
            partial.file().write_all(b"after").unwrap();
            partial.replace(&to, &AtomicBool::new(cancelled), "Save cancelled")
        };
        assert_eq!(write(true).unwrap_err().to_string(), "Save cancelled");
        assert_eq!(std::fs::read(&to).unwrap(), b"before");
        write(false).unwrap();
        assert_eq!(std::fs::read(&to).unwrap(), b"after");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
