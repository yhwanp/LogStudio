//! Recently opened log files. Paths stay native: the page receives display
//! names and opaque IDs, and can only ask to reopen an entry recorded after a
//! user-chosen file finished validation. Files opened together form one entry
//! and reopen together in the same order.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Mutex,
};

const LIMIT: usize = 10;

/// `path` alone is the earlier one-file record; it still names the first file
/// so an older build reopens at least that one.
#[derive(Serialize, Deserialize, Clone)]
struct Stored {
    path: PathBuf,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    paths: Vec<PathBuf>,
}
struct Entry {
    id: u64,
    paths: Vec<PathBuf>,
}
#[derive(Default)]
struct Inner {
    entries: Vec<Entry>,
    pending: HashMap<u64, Vec<PathBuf>>,
    next: u64,
}
#[derive(Default)]
pub struct Recent {
    file: Mutex<Option<PathBuf>>,
    inner: Mutex<Inner>,
}

impl Recent {
    /// Loads the list from `dir`; an unreadable or malformed file starts empty.
    pub fn load(&self, dir: &Path) {
        let file = dir.join("recent-files.json");
        let stored: Vec<Stored> = std::fs::read(&file)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let mut inner = self.inner.lock().unwrap();
        for paths in stored
            .into_iter()
            .map(|s| {
                if s.paths.is_empty() {
                    vec![s.path]
                } else {
                    s.paths
                }
            })
            .filter(|paths| paths.iter().all(|p| p.is_absolute()))
            .take(LIMIT)
        {
            inner.next += 1;
            let id = inner.next;
            inner.entries.push(Entry { id, paths });
        }
        *self.file.lock().unwrap() = Some(file);
    }
    /// Remembers which paths a source ID was opened from until it validates.
    pub fn expect(&self, id: u64, paths: &[PathBuf]) {
        let mut inner = self.inner.lock().unwrap();
        inner.pending.retain(|&k, _| k + 8 > id);
        inner.pending.insert(id, paths.to_vec());
    }
    /// Records a validated source at the top; a rollback only forgets it.
    /// Returns true when the list changed.
    pub fn settle(&self, id: u64, validated: bool) -> bool {
        let mut inner = self.inner.lock().unwrap();
        let Some(paths) = inner.pending.remove(&id) else {
            return false;
        };
        if !validated {
            return false;
        }
        inner.entries.retain(|e| e.paths != paths);
        inner.next += 1;
        let id = inner.next;
        inner.entries.insert(0, Entry { id, paths });
        inner.entries.truncate(LIMIT);
        self.save(&inner);
        true
    }
    pub fn paths(&self, id: u64) -> Option<Vec<PathBuf>> {
        let inner = self.inner.lock().unwrap();
        inner
            .entries
            .iter()
            .find(|e| e.id == id)
            .map(|e| e.paths.clone())
    }
    pub fn forget(&self, id: Option<u64>) {
        let mut inner = self.inner.lock().unwrap();
        match id {
            Some(id) => inner.entries.retain(|e| e.id != id),
            None => inner.entries.clear(),
        }
        self.save(&inner);
    }
    pub fn list(&self) -> Value {
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from);
        let inner = self.inner.lock().unwrap();
        Value::Array(
            inner
                .entries
                .iter()
                .map(|e| {
                    let name = |p: &PathBuf| {
                        p.file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned()
                    };
                    let first = &e.paths[0];
                    let folder = first.parent().unwrap_or(Path::new(""));
                    let folder = match home.as_deref().and_then(|h| folder.strip_prefix(h).ok()) {
                        Some(rest) => Path::new("~").join(rest),
                        None => folder.to_owned(),
                    };
                    json!({
                        "id": e.id,
                        "name": match e.paths.len() {
                            1 => name(first),
                            n => format!("{} 외 {}개", name(first), n - 1),
                        },
                        "folder": folder.to_string_lossy(),
                        "files": e.paths.iter().map(name).collect::<Vec<_>>(),
                    })
                })
                .collect(),
        )
    }
    /// Writes through a temporary file so a crash never truncates the list.
    fn save(&self, inner: &Inner) {
        let Some(file) = self.file.lock().unwrap().clone() else {
            return;
        };
        let stored: Vec<Stored> = inner
            .entries
            .iter()
            .map(|e| Stored {
                path: e.paths[0].clone(),
                paths: if e.paths.len() > 1 {
                    e.paths.clone()
                } else {
                    vec![]
                },
            })
            .collect();
        let Ok(bytes) = serde_json::to_vec_pretty(&stored) else {
            return;
        };
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let partial = file.with_extension("json.partial");
        if std::fs::write(&partial, bytes).is_ok() {
            let _ = std::fs::rename(&partial, &file);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validated_sources_are_recorded_newest_first_bounded_and_persisted() {
        let dir = tempfile::tempdir().unwrap();
        let recent = Recent::default();
        recent.load(dir.path());
        let path = |n: usize| dir.path().join(format!("log-{n}.log"));
        for n in 0..12 {
            recent.expect(n as u64 + 1, &[path(n)]);
            assert!(recent.settle(n as u64 + 1, true));
        }
        recent.expect(20, &[path(99)]);
        assert!(
            !recent.settle(20, false),
            "rolled back sources are not recorded"
        );
        assert!(
            !recent.settle(21, true),
            "only expected sources are recorded"
        );
        recent.expect(22, &[path(3)]);
        recent.settle(22, true);
        let list = recent.list();
        let names: Vec<&str> = list
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "log-3.log",
                "log-11.log",
                "log-10.log",
                "log-9.log",
                "log-8.log",
                "log-7.log",
                "log-6.log",
                "log-5.log",
                "log-4.log",
                "log-2.log"
            ]
        );
        let first = list[0]["id"].as_u64().unwrap();
        assert_eq!(recent.paths(first), Some(vec![path(3)]));
        recent.forget(Some(first));
        assert_eq!(recent.paths(first), None);

        let reloaded = Recent::default();
        reloaded.load(dir.path());
        assert_eq!(reloaded.list().as_array().unwrap().len(), 9);
        assert_eq!(reloaded.list()[0]["name"], "log-11.log");
        reloaded.forget(None);
        let empty = Recent::default();
        empty.load(dir.path());
        assert_eq!(empty.list(), json!([]));
    }

    #[test]
    fn malformed_or_relative_entries_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let absolute = dir.path().join("a.log");
        let stored = json!([{"path":"relative.log"},{"path":absolute}]);
        std::fs::write(dir.path().join("recent-files.json"), stored.to_string()).unwrap();
        let recent = Recent::default();
        recent.load(dir.path());
        assert_eq!(recent.list().as_array().unwrap().len(), 1);
        std::fs::write(dir.path().join("recent-files.json"), b"not json").unwrap();
        let broken = Recent::default();
        broken.load(dir.path());
        assert_eq!(broken.list(), json!([]));
    }

    #[test]
    fn files_opened_together_are_one_entry_and_older_records_still_load() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("b.log"), dir.path().join("a.log"));
        let recent = Recent::default();
        recent.load(dir.path());
        recent.expect(1, &[a.clone(), b.clone()]);
        recent.settle(1, true);
        recent.expect(2, std::slice::from_ref(&a));
        recent.settle(2, true);
        let list = recent.list();
        assert_eq!(list[0]["name"], "b.log");
        assert_eq!(list[1]["name"], "b.log 외 1개");
        assert_eq!(list[1]["files"], json!(["b.log", "a.log"]));
        let group = list[1]["id"].as_u64().unwrap();
        assert_eq!(recent.paths(group), Some(vec![a.clone(), b.clone()]));

        // The stored record keeps the first file for builds that read `path`.
        let stored: Value =
            serde_json::from_slice(&std::fs::read(dir.path().join("recent-files.json")).unwrap())
                .unwrap();
        assert_eq!(stored[1]["path"], json!(a));
        assert!(stored[0].get("paths").is_none());
        let reloaded = Recent::default();
        reloaded.load(dir.path());
        assert_eq!(reloaded.list()[1]["name"], "b.log 외 1개");
    }
}
