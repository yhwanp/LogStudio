pub mod dataset;
pub mod dlt;
pub mod encoding;
pub mod fields_index;
pub mod launch;
pub mod order;
pub mod parser;
pub mod search;
pub mod source;
use dataset::{Dataset, ExportKind};
use search::{Filter, Filters};
use serde_json::{json, Value};
use source::{error, Source};
use std::{
    io,
    path::Path,
    sync::{atomic::AtomicBool, Arc, Mutex},
};
#[derive(Default)]
struct State {
    active: Option<Arc<Dataset>>,
    previous: Option<Arc<Dataset>>,
    sequence: u64,
}
/// Receives every engine notification.
type Observer = Arc<dyn Fn(Value) + Send + Sync>;
pub struct Engine {
    state: Mutex<State>,
    open_gate: Mutex<()>,
    notify: Mutex<Option<Observer>>,
}
impl Default for Engine {
    fn default() -> Self {
        Self {
            state: Mutex::new(State::default()),
            open_gate: Mutex::new(()),
            notify: Mutex::new(None),
        }
    }
}
impl Engine {
    pub fn observe(&self, f: impl Fn(Value) + Send + Sync + 'static) {
        *self.notify.lock().unwrap() = Some(Arc::new(f));
    }
    fn emit(&self, v: Value) {
        let f = self.notify.lock().unwrap().clone();
        if let Some(f) = f {
            f(v);
        }
    }
    pub fn emit_error(&self, error: io::Error) {
        self.emit(json!({"status":"error","error":source::public_message(&error, "Could not open selected file; check availability and retry")}));
    }
    pub fn cancel_pending(&self) {
        let id = self.state.lock().unwrap().sequence;
        self.cancel_open(id);
    }
    pub fn begin_open(&self) -> u64 {
        let mut s = self.state.lock().unwrap();
        s.sequence += 1;
        s.sequence
    }
    pub fn open_file(self: &Arc<Self>, id: u64, path: &Path) -> io::Result<Value> {
        self.install(id, Source::granted(path)?)
    }
    /// Opens the files, in order, as one combined log.
    pub fn open_files(
        self: &Arc<Self>,
        id: u64,
        paths: &[std::path::PathBuf],
    ) -> io::Result<Value> {
        self.install(id, Source::granted_files(paths)?)
    }
    pub fn open_text(self: &Arc<Self>, id: u64, text: &str, name: &str) -> io::Result<Value> {
        self.install(id, Source::text(text, name)?)
    }
    /// The granted files of dataset `id`, open or kept for rollback. None for
    /// pasted text or a dataset that is gone.
    pub fn grants(&self, id: u64) -> Option<Vec<source::Grant>> {
        let s = self.state.lock().unwrap();
        let d = [&s.active, &s.previous]
            .into_iter()
            .flatten()
            .find(|d| d.id == id)?;
        d.source.grants()
    }
    /// Opens dataset `from`'s files again as `id` in the same order, keeping
    /// `from` for rollback as any open does.
    pub fn reopen(self: &Arc<Self>, id: u64, from: u64) -> io::Result<Value> {
        let paths: Vec<_> = self
            .grants(from)
            .ok_or_else(|| error("Nothing to reopen; select the file again"))?
            .into_iter()
            .map(|g| g.path)
            .collect();
        self.install(id, Source::granted_files(&paths)?)
    }
    fn install(self: &Arc<Self>, id: u64, source: Source) -> io::Result<Value> {
        let _gate = self.open_gate.lock().unwrap();
        let (mut obsolete, d) = {
            let mut s = self.state.lock().unwrap();
            if s.sequence != id {
                return Err(error("Opening superseded"));
            }
            let old = s.active.take();
            let obsolete = if s.previous.is_some() {
                old
            } else {
                if let Some(old) = old {
                    let index = old.index.lock().unwrap();
                    let valid = index.done && index.error.is_empty();
                    drop(index);
                    if valid {
                        s.previous = Some(old);
                        None
                    } else {
                        Some(old)
                    }
                } else {
                    None
                }
            };
            let d = Dataset::new(id, source);
            s.active = Some(d.clone());
            (obsolete, d)
        };
        if let Some(old) = obsolete.take() {
            old.close();
        }
        // A query's terminal notification checks the active dataset. Do not
        // hold that lock while suspension joins its worker.
        let previous = self.state.lock().unwrap().previous.clone();
        if let Some(old) = previous {
            old.suspend();
        }
        let weak = Arc::downgrade(self);
        let progress_engine = weak.clone();
        d.observe_progress(move |pane, state| {
            if let Some(engine) = progress_engine.upgrade() {
                let current = engine
                    .state
                    .lock()
                    .unwrap()
                    .active
                    .as_ref()
                    .is_some_and(|d| d.id == id);
                if current {
                    engine.emit(json!({"status":"query-progress","pane":if pane==0{"logs"}else{"search"},"state":state}));
                }
            }
        });
        // Publish the candidate before its worker can publish a terminal event.
        self.emit(json!({"status":"opened","datasetId":id}));
        d.start(move |valid| {
            if let Some(e) = weak.upgrade() {
                e.finish(id, valid);
            }
        });
        Ok(d.status(0))
    }
    fn finish(&self, id: u64, valid: bool) {
        let discarded = {
            let mut s = self.state.lock().unwrap();
            if s.active.as_ref().is_none_or(|d| d.id != id) {
                return;
            }
            if valid {
                s.previous.take()
            } else {
                let failed = s.active.take();
                s.active = s.previous.take();
                failed
            }
        };
        if !valid {
            if let Some(previous) = self.state.lock().unwrap().active.clone() {
                previous.resume();
            }
        }
        let (message, file) = discarded
            .as_ref()
            .filter(|_| !valid)
            .map(|d| {
                let index = d.index.lock().unwrap();
                (index.error.clone(), index.error_file.clone())
            })
            .unwrap_or_default();
        if let Some(d) = discarded {
            d.close();
        }
        let mut event =
            json!({"status":if valid{"validated"}else{"rollback"},"error":message,"datasetId":id});
        // Which of several files failed to open.
        if !file.is_empty() {
            event["file"] = json!(file);
        }
        self.emit(event);
    }
    pub fn cancel_open(&self, id: u64) {
        let _gate = self.open_gate.lock().unwrap();
        let discarded = {
            let mut s = self.state.lock().unwrap();
            if s.sequence != id {
                return;
            }
            s.sequence += 1;
            if s.active.as_ref().is_some_and(|d| d.id == id)
                && !s.active.as_ref().unwrap().index.lock().unwrap().done
            {
                let failed = s.active.take();
                s.active = s.previous.take();
                failed
            } else {
                None
            }
        };
        if let Some(previous) = self.state.lock().unwrap().active.clone() {
            previous.resume();
        }
        if let Some(d) = discarded {
            d.close();
            self.emit(json!({"status":"rollback","error":"Opening cancelled","datasetId":id}));
        }
    }
    pub fn close(&self) {
        let _gate = self.open_gate.lock().unwrap();
        let (a, b) = {
            let mut s = self.state.lock().unwrap();
            s.sequence += 1;
            (s.active.take(), s.previous.take())
        };
        if let Some(d) = a {
            d.close();
        }
        if let Some(d) = b {
            d.close();
        }
    }
    /// Exports a pane's completed results to `path` through a temporary file
    /// in the same folder, so a failed or cancelled export never leaves a
    /// partial file at the destination.
    pub fn export(
        &self,
        pane: &str,
        dataset_id: u64,
        query_id: u64,
        path: &Path,
        cancel: &AtomicBool,
        progress: impl FnMut(u64, u64),
    ) -> io::Result<Value> {
        self.export_as(
            pane,
            dataset_id,
            query_id,
            path,
            ExportKind::Original,
            cancel,
            progress,
        )
    }
    /// `export` writing `kind`: source bytes, or a DLT log's row text.
    /// `skipped`, when not zero, counts DLT result rows of other bytes,
    /// which have no message to write.
    #[allow(clippy::too_many_arguments)]
    pub fn export_as(
        &self,
        pane: &str,
        dataset_id: u64,
        query_id: u64,
        path: &Path,
        kind: ExportKind,
        cancel: &AtomicBool,
        progress: impl FnMut(u64, u64),
    ) -> io::Result<Value> {
        let pane = match pane {
            "logs" => 0,
            "search" => 1,
            _ => return Err(error("Invalid pane")),
        };
        let d = self
            .state
            .lock()
            .unwrap()
            .active
            .clone()
            .ok_or_else(|| error("No log is open"))?;
        if d.id != dataset_id {
            return Err(error("Stale source"));
        }
        let q = d.selected(pane, query_id)?;
        if d.source.is_same_file(path) {
            return Err(error("Choose a file other than the open log"));
        }
        let name = path.file_name().ok_or_else(|| error("Export failed"))?;
        let mut partial = path.to_path_buf();
        partial.set_file_name(format!(".{}.logstudio-partial", name.to_string_lossy()));
        let result = (|| {
            let file = std::fs::File::create(&partial)?;
            let mut out = io::BufWriter::with_capacity(1 << 20, file);
            let written = d.export_as(&q, kind, &mut out, cancel, progress)?;
            out.into_inner().map_err(|e| e.into_error())?.sync_all()?;
            std::fs::rename(&partial, path)?;
            Ok::<_, io::Error>(written)
        })();
        match result {
            Ok(e) => {
                let mut v = json!({"rows":e.rows,"bytes":e.bytes});
                if e.skipped > 0 {
                    v["skipped"] = json!(e.skipped);
                }
                Ok(v)
            }
            Err(e) => {
                let _ = std::fs::remove_file(&partial);
                // Policy errors pass through; OS write failures get a public
                // message without the private path.
                if !source::public_message(&e, "").is_empty() {
                    return Err(e);
                }
                Err(error(match e.kind() {
                    io::ErrorKind::PermissionDenied => {
                        "Write permission denied for the export file"
                    }
                    io::ErrorKind::StorageFull => "Not enough disk space for the export",
                    _ => "Export failed",
                }))
            }
        }
    }
    pub fn request(&self, v: Value) -> io::Result<Value> {
        if serde_json::to_vec(&v)?.len() > 16384 {
            return Err(error("Request budget exceeded"));
        }
        let action = v["action"].as_str().unwrap_or("");
        let pane = match v["pane"].as_str().unwrap_or("logs") {
            "logs" => 0,
            "search" => 1,
            _ => return Err(error("Invalid pane")),
        };
        let d = self.state.lock().unwrap().active.clone();
        if action == "status" {
            return Ok(d.map(|d| d.status(pane)).unwrap_or(json!({"empty":true})));
        }
        if action == "close" {
            self.close();
            return Ok(json!({"closed":true}));
        }
        let d = d.ok_or_else(|| error("No log is open"))?;
        if v["datasetId"].as_u64() != Some(d.id) {
            return Err(error("Stale source"));
        }
        if action == "follow" {
            let on = v["follow"]
                .as_bool()
                .ok_or_else(|| error("Invalid follow"))?;
            d.follow(on)?;
            return Ok(d.status(pane));
        }
        let id = v["queryId"]
            .as_u64()
            .ok_or_else(|| error("Missing query ID"))?;
        if action == "query" {
            let f: Filters = serde_json::from_value(v["filters"].clone())?;
            let q = d.query(pane, id, Filter::new(f)?)?;
            return Ok(json!({"datasetId":d.id,"queryId":id,"query":q.json()}));
        }
        let q = d.selected(pane, id)?;
        let integer = |k: &str, default: u64, max: u64| -> io::Result<u64> {
            v.get(k).map_or(Ok(default), |n| {
                n.as_u64()
                    .filter(|n| *n <= max)
                    .ok_or_else(|| error(format!("Invalid {k}")))
            })
        };
        match action {
            "cancel" => {
                q.cancel();
                Ok(json!({"datasetId":d.id,"queryId":id,"query":q.json()}))
            }
            "page" => {
                // The upper logs may mark the search pane's current results;
                // a replaced search identity just leaves the rows unmarked.
                let search = match v.get("searchQueryId").filter(|_| pane == 0) {
                    Some(n) => {
                        let id = n.as_u64().ok_or_else(|| error("Invalid searchQueryId"))?;
                        d.selected(1, id).ok()
                    }
                    None => None,
                };
                d.page_marked(
                    &q,
                    integer("start", 0, u64::MAX)?,
                    integer("count", 200, 200)? as usize,
                    search.as_deref(),
                )
            }
            "locate" => d.locate(&q, integer("lineNumber", 1, u64::MAX)?),
            "range" => d.range(
                &q,
                integer("lineNumber", 1, u64::MAX)?,
                integer("startByte", 0, source::MAX_LINE as u64)? as usize,
            ),
            "record" => d.record(
                &q,
                integer("lineNumber", 1, u64::MAX)?,
                integer("startByte", 0, dataset::RECORD_BYTES as u64)? as usize,
            ),
            "lines" => {
                let lines: Vec<u64> = v["lineNumbers"]
                    .as_array()
                    .filter(|a| a.len() <= dataset::LINES_PER_REQUEST)
                    .and_then(|a| a.iter().map(|n| n.as_u64().filter(|&n| n > 0)).collect())
                    .filter(|l: &Vec<u64>| l.windows(2).all(|w| w[0] < w[1]))
                    .ok_or_else(|| error("Invalid lineNumbers"))?;
                d.lines(&q, &lines)
            }
            _ => Err(error("Unsupported action")),
        }
    }
}
