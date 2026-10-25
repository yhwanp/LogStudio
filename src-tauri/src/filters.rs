//! Saved filters: filter conditions saved under a name and applied to any log
//! of the same format. The page owns what the conditions mean and validates
//! one when it applies it; native code bounds what it stores and keeps names
//! unique. The file is read again for every request and replaced through a
//! temporary file, so two running apps do not drop each other's filters.
use crate::work::bounded;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

/// Filters kept at most.
pub const LIMIT: usize = 100;
const NAME_CHARS: usize = 60;
/// One filter's serialized size; 32 full conditions fit, messages included.
const FILTER_BYTES: usize = 256 * 1024;
/// Nesting the page's filters use: filter, condition list, condition, levels.
const DEPTH: usize = 4;
/// The largest ID a page number holds exactly (2^53 - 1).
const MAX_ID: u64 = (1 << 53) - 1;
const FORMATS: [&str; 2] = ["text", "dlt"];

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
struct Entry {
    id: u64,
    name: String,
    format: String,
    filter: Value,
}
#[derive(Default)]
pub struct SavedFilters {
    /// The store's path; held across each read and write.
    file: Mutex<Option<PathBuf>>,
}

/// A name as shown: control characters dropped, outer spaces trimmed.
fn clean_name(name: &str) -> Result<String, String> {
    let name: String = name.chars().filter(|c| !c.is_control()).collect();
    let name = name.trim();
    if name.is_empty() {
        return Err("Enter a filter name".into());
    }
    if name.chars().count() > NAME_CHARS {
        return Err("Filter names are limited to 60 characters".into());
    }
    Ok(name.to_owned())
}
fn same_name(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}
fn clean_filter(filter: Value) -> Result<Value, String> {
    if !filter.is_object() {
        return Err("Invalid saved filter".into());
    }
    let filter = bounded(filter, DEPTH).ok_or("Invalid saved filter")?;
    if !serde_json::to_vec(&filter).is_ok_and(|b| b.len() <= FILTER_BYTES) {
        return Err("Saved filter is too large".into());
    }
    Ok(filter)
}
fn valid(e: &Entry) -> bool {
    (1..=MAX_ID).contains(&e.id)
        && FORMATS.contains(&e.format.as_str())
        && clean_name(&e.name).is_ok_and(|n| n == e.name)
        && clean_filter(e.filter.clone()).is_ok_and(|f| f == e.filter)
}
/// Saved filters on disk. An unreadable or malformed file reads as empty,
/// and malformed entries, repeated IDs and repeated names are skipped.
fn read(file: &Path) -> Vec<Entry> {
    let stored: Vec<Value> = std::fs::read(file)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let mut entries: Vec<Entry> = vec![];
    for entry in stored
        .into_iter()
        .filter_map(|v| serde_json::from_value::<Entry>(v).ok())
    {
        if entries.len() < LIMIT
            && valid(&entry)
            && !entries
                .iter()
                .any(|e| e.id == entry.id || same_name(&e.name, &entry.name))
        {
            entries.push(entry);
        }
    }
    entries
}
fn write(file: &Path, entries: &[Entry]) -> Result<(), String> {
    let written = (|| {
        let bytes = serde_json::to_vec(entries)?;
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let partial = file.with_extension("json.partial");
        std::fs::write(&partial, bytes)?;
        std::fs::rename(&partial, file)
    })();
    written.map_err(|_| "Could not save filters".to_string())
}
fn list(entries: &[Entry]) -> Value {
    json!(entries)
}
/// A new ID: after every ID in use and no earlier than the current time in
/// milliseconds, so an ID freed by a delete is not given to another filter.
fn next_id(entries: &[Entry]) -> u64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(0));
    let after = entries.iter().map(|e| e.id).max().unwrap_or(0) + 1;
    after.max(now).min(MAX_ID)
}

impl SavedFilters {
    pub fn load(&self, dir: &Path) {
        *self.file.lock().unwrap() = Some(dir.join("saved-filters.json"));
    }
    /// Runs `change` on the stored filters and writes them when it succeeds.
    fn edit<T>(
        &self,
        change: impl FnOnce(&mut Vec<Entry>) -> Result<T, String>,
    ) -> Result<(T, Value), String> {
        let file = self.file.lock().unwrap();
        let file = file.as_deref().ok_or("Could not save filters")?;
        let mut entries = read(file);
        let result = change(&mut entries)?;
        write(file, &entries)?;
        Ok((result, list(&entries)))
    }
    pub fn list(&self) -> Value {
        let file = self.file.lock().unwrap();
        list(&file.as_deref().map(read).unwrap_or_default())
    }
    /// Saves `filter` under `name`: over the filter with `id`, or as a new
    /// one. An `id` that is no longer stored (deleted, here or by another
    /// running app) is saved again under that ID. Returns the ID and the list.
    pub fn save(
        &self,
        id: Option<u64>,
        name: &str,
        format: &str,
        filter: Value,
    ) -> Result<Value, String> {
        let name = clean_name(name)?;
        if !FORMATS.contains(&format) {
            return Err("Invalid saved filter".into());
        }
        let filter = clean_filter(filter)?;
        if id.is_some_and(|id| !(1..=MAX_ID).contains(&id)) {
            return Err("Saved filter not found".into());
        }
        let (id, filters) = self.edit(|entries| {
            if entries
                .iter()
                .any(|e| Some(e.id) != id && same_name(&e.name, &name))
            {
                return Err("A saved filter has this name".to_string());
            }
            let entry = Entry {
                id: id.unwrap_or_else(|| next_id(entries)),
                name,
                format: format.to_owned(),
                filter,
            };
            let id = entry.id;
            match entries.iter().position(|e| e.id == id) {
                Some(at) => entries[at] = entry,
                None if entries.len() >= LIMIT => {
                    return Err("Up to 100 filters can be saved".to_string())
                }
                None => entries.push(entry),
            }
            Ok(id)
        })?;
        Ok(json!({"id":id,"filters":filters}))
    }
    pub fn rename(&self, id: u64, name: &str) -> Result<Value, String> {
        let name = clean_name(name)?;
        self.edit(|entries| {
            if entries
                .iter()
                .any(|e| e.id != id && same_name(&e.name, &name))
            {
                return Err("A saved filter has this name".to_string());
            }
            let entry = entries
                .iter_mut()
                .find(|e| e.id == id)
                .ok_or("Saved filter not found")?;
            entry.name = name;
            Ok(())
        })
        .map(|(_, filters)| filters)
    }
    /// Deletes the filter with `id`; one already gone is not an error.
    pub fn delete(&self, id: u64) -> Result<Value, String> {
        self.edit(|entries| {
            entries.retain(|e| e.id != id);
            Ok(())
        })
        .map(|(_, filters)| filters)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(dir: &Path) -> SavedFilters {
        let filters = SavedFilters::default();
        filters.load(dir);
        filters
    }
    fn on_disk(dir: &Path) -> Value {
        serde_json::from_slice(&std::fs::read(dir.join("saved-filters.json")).unwrap()).unwrap()
    }
    fn filter(tag: &str) -> Value {
        json!({"v":1,"include":[{"tag":tag,"levels":["W","E"]}],"exclude":[],"includeRaw":true})
    }
    fn names(list: &Value) -> Vec<&str> {
        list.as_array()
            .unwrap()
            .iter()
            .map(|e| e["name"].as_str().unwrap())
            .collect()
    }

    #[test]
    fn filters_are_saved_overwritten_renamed_and_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let filters = store(dir.path());
        assert_eq!(filters.list(), json!([]), "no file reads as empty");
        let saved = filters
            .save(None, "  APP 분석 ", "dlt", filter("APP"))
            .unwrap();
        let id = saved["id"].as_u64().unwrap();
        assert!(id > 1_600_000_000_000 && id <= MAX_ID, "{id}");
        assert_eq!(
            saved["filters"],
            json!([{"id":id,"name":"APP 분석","format":"dlt","filter":filter("APP")}])
        );
        assert_eq!(on_disk(dir.path()), saved["filters"]);

        let other = filters.save(None, "HMI", "dlt", filter("HMI")).unwrap()["id"]
            .as_u64()
            .unwrap();
        assert!(other > id, "IDs only grow");
        let over = filters
            .save(Some(id), "APP 분석", "dlt", filter("APPS"))
            .unwrap();
        assert_eq!(over["id"], id);
        assert_eq!(over["filters"][0]["filter"], filter("APPS"));
        assert_eq!(over["filters"].as_array().unwrap().len(), 2);

        assert_eq!(
            names(&filters.rename(id, "내비게이션").unwrap()),
            ["내비게이션", "HMI"]
        );
        assert_eq!(names(&filters.delete(other).unwrap()), ["내비게이션"]);
        assert_eq!(
            names(&filters.delete(other).unwrap()),
            ["내비게이션"],
            "deleting twice is not an error"
        );
        // Undoing a delete saves the filter again under its ID.
        let back = filters
            .save(Some(other), "HMI", "dlt", filter("HMI"))
            .unwrap();
        assert_eq!(back["id"], other);
        assert_eq!(names(&store(dir.path()).list()), ["내비게이션", "HMI"]);
    }

    #[test]
    fn names_are_required_bounded_and_unique_ignoring_case() {
        let dir = tempfile::tempdir().unwrap();
        let filters = store(dir.path());
        for blank in ["", "   ", "\n\t"] {
            assert_eq!(
                filters.save(None, blank, "text", filter("a")),
                Err("Enter a filter name".into())
            );
        }
        let long = "가".repeat(61);
        assert_eq!(
            filters.save(None, &long, "text", filter("a")),
            Err("Filter names are limited to 60 characters".into())
        );
        let id = filters
            .save(None, &"가".repeat(60), "text", filter("a"))
            .unwrap()["id"]
            .as_u64()
            .unwrap();
        filters
            .save(None, "My\u{7}App", "text", filter("b"))
            .unwrap();
        assert_eq!(
            names(&filters.list())[1],
            "MyApp",
            "control characters are dropped"
        );
        assert_eq!(
            filters.save(None, "myapp", "dlt", filter("c")),
            Err("A saved filter has this name".into())
        );
        assert_eq!(
            filters.rename(id, " MYAPP "),
            Err("A saved filter has this name".into())
        );
        assert_eq!(
            filters
                .rename(id, &"가".repeat(60))
                .map(|l| l.as_array().unwrap().len()),
            Ok(2),
            "a filter keeps its own name"
        );
        assert_eq!(
            filters.rename(12345, "x"),
            Err("Saved filter not found".into())
        );
    }

    #[test]
    fn bad_requests_change_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let filters = store(dir.path());
        filters.save(None, "a", "text", filter("a")).unwrap();
        let before = on_disk(dir.path());
        for (format, value) in [
            ("csv", filter("x")),
            ("text", json!([1])),
            ("text", json!("x")),
        ] {
            assert_eq!(
                filters.save(None, "b", format, value),
                Err("Invalid saved filter".into())
            );
        }
        let big = json!({"include":(0..40).map(|_| json!({"tag":"가".repeat(1000),"pid":"가".repeat(1000),"tid":"가".repeat(1000)})).collect::<Vec<_>>()});
        assert_eq!(
            filters.save(None, "b", "text", big),
            Err("Saved filter is too large".into())
        );
        assert_eq!(
            filters.save(Some(MAX_ID + 1), "b", "text", filter("b")),
            Err("Saved filter not found".into())
        );
        assert_eq!(on_disk(dir.path()), before);
        // Strings and nesting are bounded rather than rejected.
        let deep = json!({"include":[{"tag":"t".repeat(5000),"x":{"y":[1]}}]});
        let saved = filters.save(None, "b", "text", deep).unwrap();
        assert_eq!(
            saved["filters"][1]["filter"],
            json!({"include":[{"tag":"t".repeat(1000),"x":{}}]})
        );
    }

    #[test]
    fn a_filter_of_32_full_conditions_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let filters = store(dir.path());
        let condition = json!({"levels":["V","D","I","W","E","F","A","U"],"tag":"가".repeat(1000),"pid":"1".repeat(400),"tid":"2".repeat(400),"timeFrom":"06-18 19:24:00.000","timeTo":"06-18 19:25:00.000","message":"가".repeat(1000),"messageRegex":true,"enabled":false});
        let filter = json!({"v":1,"include":vec![condition.clone(); 16],"exclude":vec![condition; 16],"includeRaw":false});
        let saved = filters.save(None, "full", "text", filter.clone()).unwrap();
        assert_eq!(saved["filters"][0]["filter"], filter);
    }

    #[test]
    fn the_store_is_bounded_and_survives_corruption_and_another_app() {
        let dir = tempfile::tempdir().unwrap();
        let filters = store(dir.path());
        for n in 0..LIMIT {
            filters
                .save(None, &format!("f{n}"), "text", filter("a"))
                .unwrap();
        }
        assert_eq!(
            filters.save(None, "more", "text", filter("a")),
            Err("Up to 100 filters can be saved".into())
        );
        let first = filters.list()[0]["id"].as_u64().unwrap();
        assert!(filters.save(Some(first), "f0", "text", filter("b")).is_ok());

        // Another running app's change is seen by the next request.
        let elsewhere = store(dir.path());
        elsewhere.delete(first).unwrap();
        assert_eq!(filters.list().as_array().unwrap().len(), LIMIT - 1);
        filters.save(None, "new", "text", filter("a")).unwrap();
        assert_eq!(elsewhere.list().as_array().unwrap().len(), LIMIT);

        std::fs::write(dir.path().join("saved-filters.json"), b"{not json").unwrap();
        assert_eq!(filters.list(), json!([]));
        let mixed = json!([
            {"id":1,"name":"ok","format":"text","filter":{"v":1}},
            {"id":1,"name":"same id","format":"text","filter":{"v":1}},
            {"id":2,"name":"OK","format":"text","filter":{"v":1}},
            {"id":3,"name":"bad format","format":"csv","filter":{"v":1}},
            {"id":4,"name":" padded","format":"text","filter":{"v":1}},
            {"id":5,"name":"not an object","format":"text","filter":[1]},
            {"id":0,"name":"zero","format":"text","filter":{}},
            {"nonsense":true},
            {"id":6,"name":"kept","format":"dlt","filter":{}}
        ]);
        std::fs::write(dir.path().join("saved-filters.json"), mixed.to_string()).unwrap();
        assert_eq!(names(&filters.list()), ["ok", "kept"]);
        // Saving writes only the readable entries back.
        filters.delete(1).unwrap();
        assert_eq!(names(&on_disk(dir.path())), ["kept"]);
    }

    #[test]
    fn an_unwritable_store_reports_the_failure() {
        let dir = tempfile::tempdir().unwrap();
        let blocked = dir.path().join("file");
        std::fs::write(&blocked, "").unwrap();
        // The app data folder is a file, so nothing can be written in it.
        let filters = store(&blocked);
        assert_eq!(
            filters.save(None, "a", "text", filter("a")),
            Err("Could not save filters".into())
        );
        assert_eq!(filters.list(), json!([]));
        assert_eq!(
            SavedFilters::default().save(None, "a", "text", filter("a")),
            Err("Could not save filters".into()),
            "no app data folder"
        );
    }
}
