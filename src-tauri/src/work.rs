//! Saved work per opened file set: the page's filters, search, bookmarked
//! line numbers and positions, offered again when the same files, in the same
//! order, open later. Paths and file stamps stay native: the page saves and
//! asks by dataset ID, and only the state it saved comes back. No log text is
//! stored; restored bookmarks read their rows from the file again.
use logstudio_core::source::Grant;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
    time::UNIX_EPOCH,
};

/// File sets remembered, most recently used first.
const LIMIT: usize = 50;
/// One state's serialized size; 1,000 bookmarks and 32 full conditions fit.
const STATE_BYTES: usize = 192 * 1024;
/// Nesting a state uses: state, filters, condition list, condition, levels.
const DEPTH: usize = 5;
const TEXT_CHARS: usize = 1000;
const LIST_ITEMS: usize = 1000;
const OBJECT_KEYS: usize = 32;
/// Dataset IDs whose files are known, for saves after a source is replaced.
const KNOWN: usize = 16;

/// One file as it was opened: canonical path, size and modification time.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct Stamp {
    path: PathBuf,
    size: u64,
    /// Nanoseconds since the Unix epoch; None where the OS has no time.
    #[serde(default)]
    modified: Option<u64>,
}
/// Names opened files by canonical path so a picker, a drop, a CLI path and a
/// recent entry for one file agree. A path that no longer resolves stays as
/// given.
pub fn stamps(grants: Vec<Grant>) -> Vec<Stamp> {
    grants
        .into_iter()
        .map(|g| Stamp {
            path: std::fs::canonicalize(&g.path).unwrap_or(g.path),
            size: g.size,
            modified: g
                .modified
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .and_then(|d| u64::try_from(d.as_nanos()).ok()),
        })
        .collect()
}
fn same_paths(a: &[Stamp], b: &[Stamp]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.path == y.path)
}
#[derive(Serialize, Deserialize, Clone)]
struct Entry {
    files: Vec<Stamp>,
    state: Value,
}
#[derive(Default)]
struct Inner {
    entries: Vec<Entry>,
    /// Recent dataset IDs and their files, newest last.
    known: Vec<(u64, Vec<Stamp>)>,
    /// Datasets opened by a reload, and the dataset they reloaded.
    reloads: Vec<(u64, u64)>,
}
#[derive(Default)]
pub struct WorkStates {
    file: Mutex<Option<PathBuf>>,
    inner: Mutex<Inner>,
}

/// Bounds a state the page sent: nesting, keys, list and string lengths, and
/// total size. Anything else is the page's to validate when it restores.
pub(crate) fn bounded(value: Value, depth: usize) -> Option<Value> {
    Some(match value {
        Value::String(s) => Value::String(s.chars().take(TEXT_CHARS).collect()),
        Value::Array(items) if depth > 0 => Value::Array(
            items
                .into_iter()
                .take(LIST_ITEMS)
                .filter_map(|v| bounded(v, depth - 1))
                .collect(),
        ),
        Value::Object(map) if depth > 0 => Value::Object(
            map.into_iter()
                .filter(|(k, _)| k.len() <= 64)
                .take(OBJECT_KEYS)
                .filter_map(|(k, v)| Some((k, bounded(v, depth - 1)?)))
                .collect::<Map<_, _>>(),
        ),
        Value::Array(_) | Value::Object(_) => return None,
        other => other,
    })
}
pub fn sanitize(state: Value) -> Result<Value, String> {
    if !state.is_object() {
        return Err("Invalid work state".into());
    }
    let state = bounded(state, DEPTH).ok_or("Invalid work state")?;
    if !serde_json::to_vec(&state).is_ok_and(|b| b.len() <= STATE_BYTES) {
        return Err("Work state is too large".into());
    }
    Ok(state)
}

impl WorkStates {
    /// Loads saved states from `dir`; an unreadable or malformed file starts
    /// empty, and malformed entries are skipped.
    pub fn load(&self, dir: &Path) {
        let file = dir.join("work-states.json");
        let stored: Vec<Value> = std::fs::read(&file)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let mut inner = self.inner.lock().unwrap();
        inner.entries = stored
            .into_iter()
            .filter_map(|v| serde_json::from_value::<Entry>(v).ok())
            .filter(|e| {
                !e.files.is_empty()
                    && e.files.len() <= logstudio_core::source::MAX_FILES
                    && e.files.iter().all(|f| f.path.is_absolute())
            })
            .filter_map(|e| {
                Some(Entry {
                    state: sanitize(e.state).ok()?,
                    files: e.files,
                })
            })
            .take(LIMIT)
            .collect();
        *self.file.lock().unwrap() = Some(file);
    }
    /// Records that dataset `id` is a reload of dataset `from`.
    pub fn reloading(&self, id: u64, from: u64) {
        let mut inner = self.inner.lock().unwrap();
        inner.reloads.retain(|&(k, _)| k + KNOWN as u64 > id);
        inner.reloads.push((id, from));
    }
    /// Names dataset `id` by its files and returns the state saved for the
    /// same paths in the same order: `exact` when every size and modification
    /// time also match, else `changed`.
    pub fn lookup(&self, id: u64, files: Vec<Stamp>) -> Value {
        let mut inner = self.inner.lock().unwrap();
        inner.known.retain(|(k, _)| *k != id);
        inner.known.push((id, files.clone()));
        let excess = inner.known.len().saturating_sub(KNOWN);
        inner.known.drain(..excess);
        let reload = inner
            .reloads
            .iter()
            .find(|(k, _)| *k == id)
            .map(|(_, from)| *from);
        let found = inner
            .entries
            .iter()
            .position(|e| same_paths(&e.files, &files));
        let mut reply = json!({"reopenable":true,"match":null,"state":null,"reloadOf":reload});
        if let Some(at) = found {
            // A lookup counts as use; the order is written with the next save.
            let entry = inner.entries.remove(at);
            reply["match"] = json!(if entry.files == files {
                "exact"
            } else {
                "changed"
            });
            reply["state"] = entry.state.clone();
            inner.entries.insert(0, entry);
        }
        reply
    }
    /// Saves the page's state for dataset `id` under the files it was opened
    /// from, replacing what those paths had; `null` forgets it. Unknown IDs
    /// (pasted text) are ignored. Returns true when the file was written.
    pub fn save(&self, id: u64, state: Value) -> Result<bool, String> {
        let state = if state.is_null() {
            None
        } else {
            Some(sanitize(state)?)
        };
        let mut inner = self.inner.lock().unwrap();
        let Some(files) = inner
            .known
            .iter()
            .find(|(k, _)| *k == id)
            .map(|(_, f)| f.clone())
        else {
            return Ok(false);
        };
        let at = inner
            .entries
            .iter()
            .position(|e| same_paths(&e.files, &files));
        match (at, state) {
            (Some(0), Some(state))
                if inner.entries[0].files == files && inner.entries[0].state == state =>
            {
                return Ok(false)
            }
            (None, None) => return Ok(false),
            (at, state) => {
                if let Some(at) = at {
                    inner.entries.remove(at);
                }
                if let Some(state) = state {
                    inner.entries.insert(0, Entry { files, state });
                    inner.entries.truncate(LIMIT);
                }
            }
        }
        self.write(&inner);
        Ok(true)
    }
    /// Forgets the states of files opened from `paths` (as a recent entry
    /// records them), or every state.
    pub fn forget(&self, paths: Option<&[PathBuf]>) {
        let mut inner = self.inner.lock().unwrap();
        match paths {
            Some(paths) => {
                let canonical: Vec<PathBuf> = paths
                    .iter()
                    .map(|p| std::fs::canonicalize(p).unwrap_or_else(|_| p.clone()))
                    .collect();
                let before = inner.entries.len();
                inner.entries.retain(|e| {
                    e.files.len() != canonical.len()
                        || e.files.iter().zip(&canonical).any(|(f, p)| &f.path != p)
                });
                if inner.entries.len() == before {
                    return;
                }
            }
            None => inner.entries.clear(),
        }
        self.write(&inner);
    }
    /// Writes through a temporary file so a crash never truncates the store.
    /// Callers hold the lock, so two writes never share the temporary file.
    fn write(&self, inner: &Inner) {
        let Some(file) = self.file.lock().unwrap().clone() else {
            return;
        };
        let Ok(bytes) = serde_json::to_vec(&inner.entries) else {
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
    use std::time::{Duration, SystemTime};

    fn grant(path: &Path, size: u64, secs: u64) -> Grant {
        Grant {
            path: path.to_owned(),
            size,
            modified: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(secs)),
        }
    }
    fn store(dir: &Path) -> WorkStates {
        let works = WorkStates::default();
        works.load(dir);
        works
    }
    fn on_disk(dir: &Path) -> Value {
        serde_json::from_slice(&std::fs::read(dir.join("work-states.json")).unwrap()).unwrap()
    }

    #[test]
    fn same_files_restore_exactly_and_changed_stamps_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a.log"), dir.path().join("b.log"));
        std::fs::write(&a, "a").unwrap();
        std::fs::write(&b, "b").unwrap();
        let works = store(dir.path());
        let first = works.lookup(1, stamps(vec![grant(&a, 10, 5), grant(&b, 20, 6)]));
        assert_eq!(
            first,
            json!({"reopenable":true,"match":null,"state":null,"reloadOf":null})
        );
        let state = json!({"v":1,"filters":{"tag":"Net"},"bookmarks":[3,9]});
        assert_eq!(works.save(1, state.clone()), Ok(true));
        assert_eq!(
            works.save(1, state.clone()),
            Ok(false),
            "an unchanged state is not written again"
        );

        let reloaded = store(dir.path());
        let exact = reloaded.lookup(7, stamps(vec![grant(&a, 10, 5), grant(&b, 20, 6)]));
        assert_eq!(exact["match"], "exact");
        assert_eq!(exact["state"], state);
        let grown = reloaded.lookup(8, stamps(vec![grant(&a, 11, 5), grant(&b, 20, 6)]));
        assert_eq!(grown["match"], "changed");
        assert_eq!(grown["state"], state);
        let touched = reloaded.lookup(9, stamps(vec![grant(&a, 10, 7), grant(&b, 20, 6)]));
        assert_eq!(touched["match"], "changed");
        // Another order or another set is another log.
        assert_eq!(
            reloaded.lookup(10, stamps(vec![grant(&b, 20, 6), grant(&a, 10, 5)]))["match"],
            Value::Null
        );
        assert_eq!(
            reloaded.lookup(11, stamps(vec![grant(&a, 10, 5)]))["match"],
            Value::Null
        );

        // Saving the changed file replaces the entry for those paths.
        reloaded
            .save(8, json!({"v":1,"filters":{"tag":"Wifi"}}))
            .unwrap();
        let again = store(dir.path());
        assert_eq!(on_disk(dir.path()).as_array().unwrap().len(), 1);
        let now = again.lookup(1, stamps(vec![grant(&a, 11, 5), grant(&b, 20, 6)]));
        assert_eq!(now["match"], "exact");
        assert_eq!(now["state"]["filters"]["tag"], "Wifi");
        // A null state forgets it.
        assert_eq!(again.save(1, Value::Null), Ok(true));
        assert_eq!(on_disk(dir.path()), json!([]));
    }

    #[test]
    fn paths_are_canonical_and_never_part_of_a_reply() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("x.log");
        std::fs::write(&file, "x").unwrap();
        let works = store(dir.path());
        works.lookup(1, stamps(vec![grant(&file, 1, 1)]));
        works.save(1, json!({"v":1})).unwrap();
        let indirect = dir.path().join(".").join("x.log");
        let reply = works.lookup(2, stamps(vec![grant(&indirect, 1, 1)]));
        assert_eq!(reply["match"], "exact");
        let text = reply.to_string();
        assert!(!text.contains("x.log") && !text.contains(&*dir.path().to_string_lossy()));
        let stored = on_disk(dir.path());
        assert_eq!(
            stored[0]["files"][0]["path"],
            json!(std::fs::canonicalize(&file).unwrap())
        );
    }

    #[test]
    fn unknown_datasets_and_bad_states_are_rejected_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let works = store(dir.path());
        assert_eq!(
            works.save(42, json!({"v":1})),
            Ok(false),
            "pasted text has no files"
        );
        assert!(!dir.path().join("work-states.json").exists());
        works.lookup(1, stamps(vec![grant(&dir.path().join("a.log"), 1, 1)]));
        for bad in [json!([1]), json!("x"), json!(3)] {
            assert_eq!(works.save(1, bad), Err("Invalid work state".into()));
        }
        let huge = json!(vec!["x".repeat(1000); 10]);
        let mut big = json!({});
        for k in 0..32 {
            big[format!("k{k}")] = huge.clone();
        }
        assert_eq!(works.save(1, big), Err("Work state is too large".into()));
        // Strings, lists and nesting are bounded rather than rejected.
        let long = json!({"filters":{"query":"q".repeat(5000),"deep":{"a":{"b":{"c":{"d":1}}}}},"bookmarks":(1..=1500).collect::<Vec<u64>>()});
        assert_eq!(works.save(1, long), Ok(true));
        let saved =
            works.lookup(1, stamps(vec![grant(&dir.path().join("a.log"), 1, 1)]))["state"].clone();
        assert_eq!(saved["filters"]["query"].as_str().unwrap().len(), 1000);
        assert_eq!(saved["bookmarks"].as_array().unwrap().len(), 1000);
        assert_eq!(
            saved["filters"]["deep"],
            json!({"a":{"b":{}}}),
            "containers below five levels are dropped"
        );
    }

    #[test]
    fn filters_of_32_full_conditions_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let works = store(dir.path());
        let file = stamps(vec![grant(&dir.path().join("a.log"), 1, 1)]);
        works.lookup(1, file.clone());
        let condition = json!({"levels":["V","D","I","W","E","F","A","U"],"tag":"가".repeat(1000),"pid":"1".repeat(400),"tid":"2".repeat(400),"timeFrom":"06-18 19:24:00.000","timeTo":"06-18 19:25:00.000","enabled":true});
        let state = json!({"v":1,"filters":{"include":vec![condition.clone(); 16],"exclude":vec![condition; 16],"query":"q".repeat(1000)},"bookmarks":(1..=1000).collect::<Vec<u64>>()});
        assert_eq!(works.save(1, state.clone()), Ok(true));
        assert_eq!(works.lookup(2, file)["state"], state);
    }

    #[test]
    fn the_store_is_bounded_least_recently_used_first_and_survives_corruption() {
        let dir = tempfile::tempdir().unwrap();
        let works = store(dir.path());
        let file = |n: u64| stamps(vec![grant(&dir.path().join(format!("{n}.log")), n, n)]);
        for n in 0..55 {
            works.lookup(n, file(n));
            works.save(n, json!({"v":1,"n":n})).unwrap();
        }
        // Looking up an old entry keeps it when later saves push others out.
        assert_eq!(works.lookup(100, file(5))["state"]["n"], 5);
        works.lookup(101, file(60));
        works.save(101, json!({"v":1,"n":60})).unwrap();
        let stored = on_disk(dir.path());
        let kept: Vec<u64> = stored
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["state"]["n"].as_u64().unwrap())
            .collect();
        assert_eq!(kept.len(), LIMIT);
        assert_eq!(&kept[..3], &[60, 5, 54]);
        assert!(!kept.contains(&6) && kept.contains(&7));
        // Saves still find datasets replaced since: the last 16 IDs are known.
        assert_eq!(works.save(40, json!({})), Ok(false));
        assert_eq!(works.save(101, json!({"v":1,"n":61})), Ok(true));

        std::fs::write(dir.path().join("work-states.json"), b"{not json").unwrap();
        let broken = store(dir.path());
        assert_eq!(broken.lookup(1, file(1))["match"], Value::Null);
        let mixed = json!([{"files":[{"path":"relative.log","size":1}],"state":{}},{"files":[],"state":{}},{"files":[{"path":dir.path().join("1.log"),"size":1,"modified":1_000_000_000}],"state":{"n":1}},{"nonsense":true},{"files":[{"path":dir.path().join("2.log"),"size":2}],"state":[1]}]);
        std::fs::write(dir.path().join("work-states.json"), mixed.to_string()).unwrap();
        let partial = store(dir.path());
        let one = stamps(vec![grant(&dir.path().join("1.log"), 1, 1)]);
        assert_eq!(partial.lookup(1, one)["state"]["n"], 1);
        assert_eq!(partial.inner.lock().unwrap().entries.len(), 1);
    }

    #[test]
    fn forgetting_follows_the_recent_list() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a.log"), dir.path().join("b.log"));
        std::fs::write(&a, "a").unwrap();
        let works = store(dir.path());
        works.lookup(1, stamps(vec![grant(&a, 1, 1)]));
        works.save(1, json!({"v":1})).unwrap();
        works.lookup(2, stamps(vec![grant(&a, 1, 1), grant(&b, 1, 1)]));
        works.save(2, json!({"v":2})).unwrap();
        works.forget(Some(std::slice::from_ref(&a)));
        assert_eq!(on_disk(dir.path()).as_array().unwrap().len(), 1);
        assert_eq!(
            works.lookup(3, stamps(vec![grant(&a, 1, 1)]))["match"],
            Value::Null
        );
        works.forget(None);
        assert_eq!(on_disk(dir.path()), json!([]));
    }

    #[test]
    fn a_reload_names_the_dataset_it_replaces() {
        let dir = tempfile::tempdir().unwrap();
        let works = store(dir.path());
        works.reloading(5, 4);
        let a = stamps(vec![grant(&dir.path().join("a.log"), 1, 1)]);
        assert_eq!(works.lookup(5, a.clone())["reloadOf"], 4);
        assert_eq!(works.lookup(6, a)["reloadOf"], Value::Null);
    }
}
