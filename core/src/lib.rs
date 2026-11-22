pub mod capture;
pub mod dataset;
pub mod dlt;
pub mod encoding;
pub mod fields_index;
pub mod launch;
pub mod order;
pub mod parser;
pub mod search;
pub mod source;
use capture::{Capture, Cut};
use dataset::{Dataset, ExportKind};
use search::{Filter, Filters};
use serde_json::{json, Value};
use source::{error, Source};
use std::{
    io,
    path::Path,
    sync::{atomic::AtomicBool, Arc, Condvar, Mutex},
};
/// The shown log holds no capture a clear can cut.
pub const NOTHING_TO_CLEAR: &str = "No device capture to clear";
#[derive(Default)]
struct State {
    active: Option<Arc<Dataset>>,
    previous: Option<Arc<Dataset>>,
    sequence: u64,
    /// Whether `previous` was followed when it was kept.
    followed_previous: bool,
    /// Device captures by the dataset that shows their file. A capture
    /// being cleared is listed under both its datasets.
    captures: Vec<(u64, Arc<Capture>)>,
    /// Captures being cleared: the dataset that opens the new file, the
    /// dataset that shows the rows so far, and the new file. Without a new
    /// file, the dataset opens the rows a clear of the stopped capture set
    /// aside, to take them back.
    clears: Vec<(u64, u64, Option<Cut>)>,
}
impl State {
    /// Whether dataset `id` is shown or kept for rollback.
    fn keeps(&self, id: u64) -> bool {
        [&self.active, &self.previous]
            .into_iter()
            .flatten()
            .any(|d| d.id == id)
    }
}
/// Receives every engine notification.
type Observer = Arc<dyn Fn(Value) + Send + Sync>;
pub struct Engine {
    state: Mutex<State>,
    open_gate: Mutex<()>,
    notify: Mutex<Option<Observer>>,
    /// Held while a clear, or its undoing, switches its capture's file.
    cutting: Mutex<()>,
    /// Signalled when a pending clear is made or dropped.
    cut_made: Condvar,
}
impl Default for Engine {
    fn default() -> Self {
        Self {
            state: Mutex::new(State::default()),
            open_gate: Mutex::new(()),
            notify: Mutex::new(None),
            cutting: Mutex::new(()),
            cut_made: Condvar::new(),
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
                    let live = index.live;
                    drop(index);
                    if valid {
                        s.previous = Some(old);
                        s.followed_previous = live;
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
            self.retire(old);
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
                    let mut state = state;
                    if let Some(c) = engine.capture(id) {
                        state["capture"] = c.json();
                    }
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
        Ok(self.status_of(&d, 0))
    }
    fn finish(self: &Arc<Self>, id: u64, valid: bool) {
        if let Some((discarded, followed)) = self.switch(id, valid) {
            self.settle(id, valid, discarded, followed);
        }
    }
    /// Keeps dataset `id` that opened, or shows the one before again when it
    /// did not. Returns the dataset to close, and whether the one shown
    /// again was followed; None when another log opens by now.
    fn switch(&self, id: u64, valid: bool) -> Option<(Option<Arc<Dataset>>, bool)> {
        let mut s = self.state.lock().unwrap();
        if s.active.as_ref().is_none_or(|d| d.id != id) {
            return None;
        }
        let followed = std::mem::take(&mut s.followed_previous);
        Some(if valid {
            (s.previous.take(), false)
        } else {
            let failed = s.active.take();
            s.active = s.previous.take();
            (failed, followed)
        })
    }
    /// Completes `switch`. Another log may open in between.
    fn settle(
        self: &Arc<Self>,
        id: u64,
        valid: bool,
        discarded: Option<Arc<Dataset>>,
        followed: bool,
    ) {
        let mut cleared = None;
        if valid {
            // Another log opening by now keeps it for rollback.
            let opened = {
                let s = self.state.lock().unwrap();
                let mut kept = [&s.active, &s.previous].into_iter().flatten();
                kept.find(|d| d.id == id).cloned()
            };
            if let Some(d) = opened {
                cleared = self.cut(&d);
                self.follow_opened(&d);
            }
        } else {
            let shown = self.state.lock().unwrap().active.clone();
            if let Some(d) = shown {
                self.revive(&d, followed);
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
            self.retire(d);
        }
        if !valid {
            self.revert_restore();
        }
        // The rows before a clear go once the log that showed them closed.
        if let Some(slot) = cleared {
            let _ = slot.remove();
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
        let (discarded, followed) = {
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
                (failed, std::mem::take(&mut s.followed_previous))
            } else {
                (None, false)
            }
        };
        let previous = self.state.lock().unwrap().active.clone();
        if let Some(previous) = previous {
            self.revive(&previous, followed);
        }
        if let Some(d) = discarded {
            self.retire(d);
            self.revert_restore();
            self.emit(json!({"status":"rollback","error":"Opening cancelled","datasetId":id}));
        }
    }
    pub fn close(&self) {
        let _gate = self.open_gate.lock().unwrap();
        let (a, b) = {
            let mut s = self.state.lock().unwrap();
            s.sequence += 1;
            s.followed_previous = false;
            (s.active.take(), s.previous.take())
        };
        if let Some(d) = a {
            self.retire(d);
        }
        if let Some(d) = b {
            self.retire(d);
        }
    }
    /// Shows a started capture's file as dataset `id`, following it while
    /// the capture records. Closing the dataset stops the capture.
    pub fn open_capture(self: &Arc<Self>, id: u64, capture: Arc<Capture>) -> io::Result<Value> {
        let source = match Source::granted(&capture.path()) {
            Ok(source) => source,
            Err(e) => {
                capture.stop();
                return Err(e);
            }
        };
        self.watch(id, &capture);
        self.state
            .lock()
            .unwrap()
            .captures
            .push((id, capture.clone()));
        let opened = self.install(id, source);
        if opened.is_err() {
            let mut s = self.state.lock().unwrap();
            s.captures.retain(|(d, _)| *d != id);
            drop(s);
            capture.stop();
        }
        opened
    }
    /// Tells the page of the capture's changes while dataset `id` shows it.
    fn watch(self: &Arc<Self>, id: u64, capture: &Capture) {
        let weak = Arc::downgrade(self);
        capture.observe(move |c| {
            let Some(engine) = weak.upgrade() else { return };
            let active = engine.state.lock().unwrap().active.clone();
            let Some(d) = active.filter(|d| d.id == id) else {
                return;
            };
            // The rows the capture wrote last complete the dataset, even
            // one whose following was paused.
            if !c.live() {
                engine.follow_capture(&d);
            }
            engine.emit(json!({"status":"capture","datasetId":id,"capture":c.json()}));
        });
    }
    /// Clears the capture that the shown dataset `from` holds, recording or
    /// stopped by the user: opens `cut`'s empty file as dataset `id`, and
    /// once that opens the capture records into it, at once or from its
    /// resume, and `from` closes with the rows so far. If it does not open,
    /// the capture goes on in its file.
    pub fn clear_capture(self: &Arc<Self>, from: u64, id: u64, cut: Cut) -> io::Result<Value> {
        let capture = match self.shown_capture() {
            Some((shown, c)) if shown == from && c.clearable() => c,
            _ => {
                cut.remove();
                return Err(error(NOTHING_TO_CLEAR));
            }
        };
        let source = match Source::granted(cut.path()) {
            Ok(source) => source,
            Err(e) => {
                cut.remove();
                return Err(e);
            }
        };
        {
            let mut s = self.state.lock().unwrap();
            s.captures.push((id, capture));
            s.clears.push((id, from, Some(cut)));
        }
        let opened = self.install(id, source);
        if opened.is_err() {
            let (capture, cut) = self.forget(id);
            if let Some(c) = capture {
                c.close();
            }
            if let Some(cut) = cut {
                cut.remove();
            }
        }
        opened
    }
    /// Undoes the last clear of the stopped capture shown as dataset `from`:
    /// switches the capture back to the set-aside file and opens it as
    /// dataset `id`, which closes `from`. If the file does not open, the
    /// capture is cleared again.
    pub fn restore_capture(self: &Arc<Self>, from: u64, id: u64) -> io::Result<Value> {
        let path = {
            // Held so that `revert_restore` sees the undoing listed.
            let mut s = self.state.lock().unwrap();
            let shown = s.active.as_ref().is_some_and(|d| d.id == from);
            let capture = s
                .captures
                .iter()
                .find(|(d, _)| *d == from && shown)
                .map(|(_, c)| c.clone())
                .ok_or_else(|| error(capture::NOTHING_TO_RESTORE))?;
            let path = capture.restore()?;
            s.captures.push((id, capture));
            s.clears.push((id, from, None));
            path
        };
        let opened = Source::granted(&path).and_then(|source| self.install(id, source));
        if opened.is_err() {
            // The log that showed the capture may have closed meanwhile.
            if let Some(c) = self.forget(id).0 {
                c.close();
            }
            self.revert_restore();
        }
        opened
    }
    /// Reverts the shown capture's restore, unless a log is still opening
    /// for it: reopening the cleared rows failed or was cancelled.
    fn revert_restore(&self) {
        let c = {
            let s = self.state.lock().unwrap();
            let Some(d) = &s.active else { return };
            let shows = |id: u64| s.captures.iter().find(|(d, _)| *d == id).map(|(_, c)| c);
            let Some(c) = shows(d.id).cloned() else {
                return;
            };
            if s.clears
                .iter()
                .any(|(id, _, _)| shows(*id).is_some_and(|o| Arc::ptr_eq(o, &c)))
            {
                return;
            }
            c
        };
        c.revert_restore();
    }
    /// The dataset that `id` replaces while a cleared capture opens.
    pub fn cleared_from(&self, id: u64) -> Option<u64> {
        let s = self.state.lock().unwrap();
        s.clears
            .iter()
            .find(|(to, _, _)| *to == id)
            .map(|(_, from, _)| *from)
    }
    /// Makes the clear, or its undoing, that dataset `d` opened for: its
    /// capture writes `d`'s file from now on. Returns the file to delete
    /// once nothing reads it. One whose log closes by now, as after a later
    /// clear, is not made, and goes as that log closes.
    fn cut(self: &Arc<Self>, d: &Arc<Dataset>) -> Option<capture::store::Slot> {
        let _cutting = self.cutting.lock().unwrap();
        let (cut, capture, undone) = {
            let mut s = self.state.lock().unwrap();
            if !s.keeps(d.id) {
                return None;
            }
            let k = s.clears.iter().position(|(id, _, _)| *id == d.id)?;
            let cut = s.clears.remove(k).2;
            let capture = s
                .captures
                .iter()
                .find(|(id, _)| *id == d.id)
                .map(|(_, c)| c.clone());
            // Confirmed as it leaves the list, so that nothing reverts it.
            let undone = match &capture {
                Some(c) if cut.is_none() => c.confirm_restore(),
                _ => None,
            };
            self.cut_made.notify_all();
            (cut, capture, undone)
        };
        let Some(c) = capture else {
            return cut.and_then(|cut| {
                cut.remove();
                None
            });
        };
        self.watch(d.id, &c);
        match cut {
            Some(cut) => c.cut(cut),
            None => undone,
        }
    }
    /// Drops dataset `id`'s capture and pending clear; the capture is
    /// returned unless another dataset still shows it.
    fn forget(&self, id: u64) -> (Option<Arc<Capture>>, Option<Cut>) {
        let mut s = self.state.lock().unwrap();
        let k = s.captures.iter().position(|(d, _)| *d == id);
        let capture = k
            .map(|k| s.captures.remove(k).1)
            .filter(|c| !s.captures.iter().any(|(_, o)| Arc::ptr_eq(o, c)));
        let k = s.clears.iter().position(|(d, _, _)| *d == id);
        let cut = k.and_then(|k| s.clears.remove(k).2);
        if k.is_some() {
            self.cut_made.notify_all();
        }
        (capture, cut)
    }
    /// Waits until a clear, or its undoing, that dataset `id` opens for is
    /// made or dropped, and no capture is switching its file.
    fn wait_for_cut(&self, id: u64) {
        let s = self.state.lock().unwrap();
        let s = self
            .cut_made
            .wait_while(s, |s| s.clears.iter().any(|(to, _, _)| *to == id))
            .unwrap();
        drop(s);
        // `cut` takes the clear from the list before it switches the file.
        drop(self.cutting.lock().unwrap());
    }
    /// The dataset the window shows, for app extensions that read its rows
    /// (`Dataset::read_blocks`): the log being opened from the moment it
    /// opens, or the one shown again after an open fails.
    pub fn shown(&self) -> Option<Arc<Dataset>> {
        self.state.lock().unwrap().active.clone()
    }
    /// The capture dataset `id` shows, open or kept for rollback.
    pub fn capture(&self, id: u64) -> Option<Arc<Capture>> {
        let s = self.state.lock().unwrap();
        s.captures
            .iter()
            .find(|(d, _)| *d == id)
            .map(|(_, c)| c.clone())
    }
    /// The capture the shown log holds, with the log's dataset ID.
    pub fn shown_capture(&self) -> Option<(u64, Arc<Capture>)> {
        let id = self.state.lock().unwrap().active.as_ref()?.id;
        Some((id, self.capture(id)?))
    }
    /// Stops the capture dataset `id` shows; its rows stay shown.
    pub fn stop_capture(&self, id: u64) -> io::Result<Value> {
        let c = self
            .capture(id)
            .ok_or_else(|| error("No device capture is open"))?;
        c.stop();
        Ok(c.json())
    }
    /// Records again into the stopped capture that the shown dataset `id`
    /// holds, which then follows its new rows.
    pub fn resume_capture(&self, id: u64) -> io::Result<Value> {
        // A clear of this log is made once its file opened. Recording on
        // before then would write rows into the file the clear discards.
        self.wait_for_cut(id);
        let (d, c) = {
            let s = self.state.lock().unwrap();
            let d = s.active.clone().filter(|d| d.id == id);
            let c = s
                .captures
                .iter()
                .find(|(d, _)| *d == id)
                .map(|(_, c)| c.clone());
            (d, c)
        };
        let (Some(d), Some(c)) = (d, c) else {
            return Err(error("No device capture is open"));
        };
        c.resume()?;
        self.follow_capture(&d);
        Ok(c.json())
    }
    /// A dataset's status with its capture's.
    fn status_of(&self, d: &Dataset, pane: usize) -> Value {
        let mut status = d.status(pane);
        if let Some(c) = self.capture(d.id) {
            status["capture"] = c.json();
        }
        status
    }
    /// Closes a dataset, then the capture it shows, which may delete its
    /// file once nothing reads it. A capture being cleared goes on under
    /// its other dataset; a clear not made deletes its new file.
    fn retire(&self, d: Arc<Dataset>) {
        let (capture, cut) = self.forget(d.id);
        d.close();
        if let Some(c) = capture {
            c.close();
        }
        if let Some(cut) = cut {
            cut.remove();
        }
    }
    /// Resumes a dataset kept for rollback. A capture that was followed is
    /// followed again: its rows arrived meanwhile.
    fn revive(&self, d: &Arc<Dataset>, followed: bool) {
        d.resume();
        if followed {
            self.follow_capture(d);
        }
    }
    /// Follows the file of a capture that dataset `d` opened for, whose rows
    /// arrive after that, or has it followed once a rollback shows it.
    fn follow_opened(&self, d: &Arc<Dataset>) {
        if self.capture(d.id).is_none() {
            return;
        }
        {
            let mut s = self.state.lock().unwrap();
            if !s.keeps(d.id) {
                return;
            }
            if s.previous.as_ref().is_some_and(|p| p.id == d.id) {
                s.followed_previous = true;
                return;
            }
        }
        self.follow_capture(d);
    }
    /// Follows a capture's file; one that ended is read to its end.
    fn follow_capture(&self, d: &Arc<Dataset>) {
        let Some(c) = self.capture(d.id) else { return };
        let _ = d.follow(true);
        // A capture that ended meanwhile does not turn following off again.
        if !c.live() {
            let _ = d.follow(false);
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
            return Ok(d
                .map(|d| self.status_of(&d, pane))
                .unwrap_or(json!({"empty":true})));
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
            return Ok(self.status_of(&d, pane));
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
#[cfg(test)]
mod tests {
    //! Moments of a clear's undoing that only a held lock can reach.
    use super::*;
    use capture::{fake::FakeLink, link::Platform, store::Store, Options, Phase};
    use std::{
        path::PathBuf,
        thread,
        time::{Duration, Instant},
    };

    const ROWS: &str =
        "10-23 14:00:01.000  1234  5678 I Tag: one\n10-23 14:00:02.000  1234  5678 I Tag: two\n";
    fn wait(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !done() {
            assert!(Instant::now() < deadline, "timed out: {what}");
            thread::sleep(Duration::from_millis(1));
        }
    }
    struct Setup {
        _dir: tempfile::TempDir,
        store: Store,
        link: Arc<FakeLink>,
        engine: Arc<Engine>,
        capture: Arc<Capture>,
        /// The dataset of the cleared log.
        cleared: u64,
        /// The file of the rows cleared.
        rows: PathBuf,
    }
    /// A stopped capture of two rows, cleared.
    fn cleared() -> Setup {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("captures"));
        let link = FakeLink::new();
        link.add("SER1", "Pixel_8", "SER1");
        let engine = Arc::new(Engine::default());
        let id = engine.begin_open();
        let slot = store.create(Platform::Android, "Pixel 8").unwrap();
        let capture = Capture::start(link.clone(), "SER1", slot, Options::default()).unwrap();
        engine.open_capture(id, capture.clone()).unwrap();
        link.log("SER1", ROWS);
        wait("rows", || capture.rows() == 2);
        engine.stop_capture(id).unwrap();
        let rows = capture.path();
        let cut = Cut::open(store.create(Platform::Android, "Pixel 8").unwrap()).unwrap();
        let cleared = engine.begin_open();
        engine.clear_capture(id, cleared, cut).unwrap();
        wait("cleared", || {
            capture.kept_path().is_some() && engine.capture(id).is_none()
        });
        Setup {
            _dir: dir,
            store,
            link,
            engine,
            capture,
            cleared,
            rows,
        }
    }
    impl Setup {
        /// Opens the cleared rows again and runs `between` once they are
        /// verified, before the capture takes them back.
        fn restore(&self, between: impl FnOnce()) -> u64 {
            let cutting = self.engine.cutting.lock().unwrap();
            let back = self.engine.begin_open();
            self.engine.restore_capture(self.cleared, back).unwrap();
            wait("verified", || {
                self.engine.state.lock().unwrap().previous.is_none()
            });
            between();
            drop(cutting);
            back
        }
        fn status(&self) -> Value {
            self.engine.request(json!({"action":"status"})).unwrap()
        }
        fn shows_rows_again(&self, back: u64) {
            wait("restored", || {
                let s = self.status();
                s["datasetId"] == back && s["total"] == 2 && s["capture"]["rows"] == 2
            });
            assert_eq!(self.capture.path(), self.rows);
            assert_eq!(std::fs::read(&self.rows).unwrap(), ROWS.as_bytes());
        }
    }
    #[test]
    fn rows_verified_to_be_taken_back_are_not_let_go() {
        let s = cleared();
        let back = s.restore(|| s.capture.forget_kept());
        s.shows_rows_again(back);
        assert!(s.capture.unsaved());
    }
    #[test]
    fn rows_being_taken_back_are_not_let_go_by_recording_on() {
        let s = cleared();
        let back = s.restore(|| assert!(s.capture.resume().is_err()));
        s.shows_rows_again(back);
        assert_eq!(s.capture.phase(), Phase::Stopped);
        s.engine.resume_capture(back).unwrap();
        assert_eq!(s.capture.path(), s.rows);
    }
    #[test]
    fn rows_being_taken_back_are_asked_about_and_saved_as_they_are() {
        let s = cleared();
        let to = s._dir.path().join("saved.log");
        let back = s.restore(|| {
            let shown = s.status();
            assert_eq!(shown["capture"]["unsaved"], true);
            assert_eq!(shown["capture"]["bytes"], ROWS.len());
            assert_eq!(shown["capture"]["rows"], 2);
            s.capture.save(&to, &AtomicBool::new(false)).unwrap();
        });
        s.shows_rows_again(back);
        assert_eq!(std::fs::read(&to).unwrap(), ROWS.as_bytes());
        assert!(!s.capture.unsaved());
    }
    #[test]
    fn closing_as_rows_are_taken_back_keeps_them_unless_discarded() {
        for discard in [false, true] {
            let s = cleared();
            let empty = s.capture.path();
            let mut closing = None;
            s.restore(|| {
                s.capture.discard(discard);
                // It waits for the log's thread, which the held lock keeps.
                let engine = s.engine.clone();
                closing = Some(thread::spawn(move || engine.close()));
                wait("closing", || {
                    s.engine.state.lock().unwrap().active.is_none()
                });
            });
            closing.unwrap().join().unwrap();
            wait("closed", || s.engine.capture(s.cleared).is_none());
            wait("cleared file removed", || !empty.exists());
            let (store, rows) = (s.store, s.rows);
            drop((s.capture, s.engine));
            if discard {
                assert!(!rows.exists());
                continue;
            }
            assert_eq!(std::fs::read(&rows).unwrap(), ROWS.as_bytes());
            let left: Vec<PathBuf> = store.leftovers().into_iter().map(|l| l.path).collect();
            assert_eq!(left, [rows]);
        }
    }
    impl Setup {
        /// Shows `path` as dataset `id` as an open does, read to its end but
        /// not finished.
        fn read(&self, id: u64, path: &Path) {
            let d = Dataset::new(id, Source::granted(path).unwrap());
            let shown = {
                let mut s = self.engine.state.lock().unwrap();
                let shown = s.active.replace(d.clone()).unwrap();
                s.previous = Some(shown.clone());
                shown
            };
            shown.suspend();
            d.start(|_| {});
            wait("read", || d.index.lock().unwrap().done);
        }
        /// Shows `path` as dataset `id` for a clear, or its undoing without
        /// `cut`, which is made only once another log opened over it. That
        /// log then does not open after all.
        fn made_under_another(&self, id: u64, path: &Path, cut: Option<Cut>) {
            let from = self.engine.shown().unwrap().id;
            self.read(id, path);
            {
                let mut s = self.engine.state.lock().unwrap();
                s.captures.push((id, self.capture.clone()));
                s.clears.push((id, from, cut));
            }
            let (discarded, followed) = self.engine.switch(id, true).unwrap();
            let (other, file) = (self.engine.begin_open(), self._dir.path().join("other.log"));
            std::fs::write(&file, ROWS).unwrap();
            self.read(other, &file);
            self.engine.settle(id, true, discarded, followed);
            self.engine.finish(other, false);
            assert_eq!(self.status()["datasetId"], id);
        }
        /// Records on, which must not wait for a clear that is never made.
        fn records_on(&self, id: u64) {
            let (done, answer) = std::sync::mpsc::channel();
            let engine = self.engine.clone();
            thread::spawn(move || done.send(engine.resume_capture(id)));
            let answer = answer.recv_timeout(Duration::from_secs(5));
            answer.expect("resume waits").unwrap();
        }
    }
    #[test]
    fn rows_taken_back_record_on_after_another_log_failed_to_open() {
        let s = cleared();
        let empty = s.capture.path();
        let back = s.engine.begin_open();
        let path = s.capture.restore().unwrap();
        s.made_under_another(back, &path, None);
        s.shows_rows_again(back);
        s.records_on(back);
        assert_eq!(s.capture.path(), s.rows);
        assert!(s.rows.exists() && !empty.exists());
    }
    #[test]
    fn a_cleared_log_records_on_and_is_followed_after_another_log_failed_to_open() {
        let s = cleared();
        let again = |live: bool| {
            let shown = s.status()["datasetId"].as_u64().unwrap();
            if live {
                s.engine.resume_capture(shown).unwrap();
            }
            let slot = s.store.create(Platform::Android, "Pixel 8").unwrap();
            let (id, path) = (s.engine.begin_open(), slot.path.clone());
            s.made_under_another(id, &path, Some(Cut::open(slot).unwrap()));
            assert_eq!(s.capture.path(), path);
            id
        };
        let id = again(true);
        wait("followed", || s.status()["following"] == true);
        s.engine.stop_capture(id).unwrap();
        let id = again(false);
        s.records_on(id);
        s.engine.close();
    }
    #[test]
    fn a_clear_made_late_does_not_undo_the_one_after_it() {
        let s = cleared();
        let pending = |from: u64| {
            let slot = s.store.create(Platform::Android, "Pixel 8").unwrap();
            let (id, path) = (s.engine.begin_open(), slot.path.clone());
            s.read(id, &path);
            let mut state = s.engine.state.lock().unwrap();
            state.captures.push((id, s.capture.clone()));
            let cut = Some(Cut::open(slot).unwrap());
            state.clears.push((id, from, cut));
            (id, path, state.active.clone().unwrap())
        };
        s.engine.resume_capture(s.cleared).unwrap();
        // The first clear's log is kept, but the clear is not made before
        // the second one's log is, and the second is made first.
        let (first, first_path, first_log) = pending(s.cleared);
        let (before, _) = s.engine.switch(first, true).unwrap();
        let (second, second_path, second_log) = pending(first);
        let (discarded, _) = s.engine.switch(second, true).unwrap();
        let old = s.engine.cut(&second_log);
        assert!(s.engine.cut(&first_log).is_none());
        assert_eq!(s.capture.path(), second_path);
        // The rest of both, as `settle` does it.
        for d in [before, discarded].into_iter().flatten() {
            s.engine.retire(d);
        }
        old.unwrap().remove().unwrap();
        assert!(!first_path.exists() && second_path.exists());
        s.engine.follow_opened(&second_log);
        // Rows the device holds already would not be recorded again.
        s.link.log("SER1", &ROWS.replace("14:00", "14:01"));
        wait("shown", || {
            let shown = s.status();
            shown["datasetId"] == second && shown["total"] == 2
        });
        s.engine.close();
    }
}
