#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use logstudio_core::{dataset::ExportKind, Engine};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Mutex,
};
use tauri::{Emitter, Manager};
use tauri_plugin_dialog::DialogExt;
mod clipboard;
mod devices;
mod drag;
mod filters;
mod geometry;
mod recent;
mod work;
struct AppState(Arc<Engine>);
/// An error from startup arguments or the file they name, kept until the page
/// asks for it: it can occur before the page listens for events.
#[derive(Default)]
struct Startup(Mutex<Option<String>>);
impl Startup {
    fn report(&self, handle: &tauri::AppHandle, error: String) {
        *self.0.lock().unwrap() = Some(error.clone());
        let _ = handle.emit_to(
            "main",
            "source-changed",
            json!({"status":"error","error":error}),
        );
    }
}
/// While a log shows a device capture, dropped files and closing the window
/// wait for the page to ask about the capture's rows.
#[derive(Default)]
struct Held {
    drop: Mutex<Option<(u64, Vec<std::path::PathBuf>)>>,
    tokens: AtomicUsize,
    /// When the page was asked about closing; it may close once it answers.
    asked: Mutex<Option<std::time::Instant>>,
    closing: AtomicBool,
}
/// A page that has not answered for this long lets the window close: the
/// capture file then stays for the next run to offer.
const CLOSE_ANSWER: std::time::Duration = std::time::Duration::from_secs(5);
impl Held {
    fn hold(&self, paths: Vec<std::path::PathBuf>) -> u64 {
        let token = self.tokens.fetch_add(1, Ordering::AcqRel) as u64 + 1;
        *self.drop.lock().unwrap() = Some((token, paths));
        token
    }
    /// Whether closing waits for the page: once per question, until it
    /// answers or stops answering.
    fn delays_close(&self) -> bool {
        if self.closing.load(Ordering::Acquire) {
            return false;
        }
        let mut asked = self.asked.lock().unwrap();
        match *asked {
            Some(at) if at.elapsed() >= CLOSE_ANSWER => false,
            Some(_) => true,
            None => {
                *asked = Some(std::time::Instant::now());
                true
            }
        }
    }
}
/// The app menu's Quit, which closes the window first as its close button
/// does: a shown capture's unsaved rows are asked about before the app goes.
/// The Dock's Quit and logging out still end the app at once; their capture
/// files stay for the next run to offer.
#[cfg(target_os = "macos")]
const QUIT: &str = "logstudio-quit";
#[cfg(target_os = "macos")]
fn install_quit(app: &tauri::AppHandle) -> tauri::Result<()> {
    use tauri::menu::MenuItem;
    let Some(menu) = app.menu() else {
        return Ok(());
    };
    let Some(app_menu) = menu
        .items()?
        .into_iter()
        .find_map(|item| item.as_submenu().cloned())
    else {
        return Ok(());
    };
    let items = app_menu.items()?;
    let Some((position, item)) = items.iter().enumerate().rev().find(|(_, item)| {
        item.as_predefined_menuitem()
            .is_some_and(|p| p.text().is_ok_and(|t| t.contains("Quit")))
    }) else {
        return Ok(());
    };
    app_menu.remove(item)?;
    let name = &app.package_info().name;
    let quit = MenuItem::with_id(app, QUIT, format!("Quit {name}"), true, Some("CmdOrCtrl+Q"))?;
    app_menu.insert(&quit, position)
}
/// Quits by closing the window; the app ends with its last window.
#[cfg(any(target_os = "macos", feature = "qa"))]
fn quit(app: &tauri::AppHandle) {
    match app.get_webview_window("main") {
        Some(window) => {
            let _ = window.close();
        }
        None => app.exit(0),
    }
}
/// Opens dropped files as one log, off the event loop.
fn open_dropped(app: &tauri::AppHandle, engine: &Arc<Engine>, paths: Vec<std::path::PathBuf>) {
    let app = app.clone();
    let engine = engine.clone();
    let id = engine.begin_open();
    std::thread::spawn(move || {
        if let Err(error) = open_arranged(&app, &engine, id, paths) {
            engine.emit_error(error);
        }
    });
}
/// Cancellation flag of the one export that may run at a time.
#[derive(Default)]
struct Exports(Mutex<Option<Arc<AtomicBool>>>);
impl Exports {
    /// Takes the one export slot until the returned export ends.
    fn start(&self) -> Result<Running<'_>, String> {
        let mut slot = self.0.lock().unwrap();
        if slot.is_some() {
            return Err("An export is already running".into());
        }
        let cancel = Arc::new(AtomicBool::new(false));
        *slot = Some(cancel.clone());
        Ok(Running {
            exports: self,
            cancel,
        })
    }
}
/// A running export or save; `cancel` stops it, and dropping it frees the
/// slot even when the request itself is dropped.
struct Running<'a> {
    exports: &'a Exports,
    cancel: Arc<AtomicBool>,
}
impl Drop for Running<'_> {
    fn drop(&mut self) {
        *self.exports.0.lock().unwrap() = None;
    }
}
static ACTIVE: AtomicUsize = AtomicUsize::new(0);
struct Permit;
impl Drop for Permit {
    fn drop(&mut self) {
        ACTIVE.fetch_sub(1, Ordering::AcqRel);
    }
}
fn permit() -> Result<Permit, String> {
    let mut n = ACTIVE.load(Ordering::Acquire);
    loop {
        if n >= 4 {
            return Err("Local engine is busy; retry".into());
        }
        match ACTIVE.compare_exchange_weak(n, n + 1, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return Ok(Permit),
            Err(actual) => n = actual,
        }
    }
}
/// Commands serve only the one app window.
fn main_window(window: &tauri::WebviewWindow) -> Result<(), String> {
    if window.label() == "main" {
        Ok(())
    } else {
        Err("Invalid window".into())
    }
}
fn message(error: std::io::Error) -> String {
    logstudio_core::source::public_message(&error, "Request failed")
}
fn message_or(fallback: &'static str) -> impl Fn(std::io::Error) -> String {
    move |error| logstudio_core::source::public_message(&error, fallback)
}
#[tauri::command]
async fn log_request(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, AppState>,
    request: Value,
) -> Result<Value, String> {
    main_window(&window)?;
    if request["action"].as_str() == Some("close") {
        window.state::<clipboard::Grants>().clear();
    }
    let permit = if matches!(request["action"].as_str(), Some("cancel" | "close")) {
        None
    } else {
        Some(permit()?)
    };
    let engine = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        engine.request(request).map_err(message)
    })
    .await
    .map_err(|_| "Request failed".to_string())?
}
/// Files picked or dropped together open in the order `order::arrange`
/// chooses, which the recent list keeps. A changed order is announced just
/// before the candidate opens, so the page can say so once it does.
fn open_arranged(
    app: &tauri::AppHandle,
    engine: &Arc<Engine>,
    id: u64,
    paths: Vec<std::path::PathBuf>,
) -> std::io::Result<Value> {
    let (paths, by) = logstudio_core::order::arrange(paths);
    if let Some(by) = by {
        let note = json!({"datasetId":id,"count":paths.len(),"by":by.as_str()});
        let _ = app.emit_to("main", "files-ordered", note);
    }
    app.state::<recent::Recent>().expect(id, &paths);
    engine.open_files(id, &paths)
}
#[tauri::command]
async fn pick_file(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, AppState>,
) -> Result<Value, String> {
    main_window(&window)?;
    let engine = state.0.clone();
    let id = engine.begin_open();
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let chosen = app
            .dialog()
            .file()
            .set_title("로그 파일 열기")
            .blocking_pick_files();
        if let Some(paths) = chosen {
            let paths = paths
                .into_iter()
                .map(|p| p.into_path())
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| "Select a local file".to_string())?;
            open_arranged(&app, &engine, id, paths).map_err(message)
        } else {
            engine.cancel_open(id);
            Ok(json!({"cancelled":true}))
        }
    })
    .await
    .map_err(|_| "File dialog failed".to_string())?
}
#[tauri::command]
// The parameters are the command's IPC arguments.
#[allow(clippy::too_many_arguments)]
async fn export_results(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, AppState>,
    exports: tauri::State<'_, Exports>,
    pane: String,
    dataset_id: u64,
    query_id: u64,
    suggested_name: String,
    format: Option<String>,
) -> Result<Value, String> {
    main_window(&window)?;
    // The page picks the format first: save panels may merge their filters
    // and do not report which one the user chose. Without one, rows are
    // written as their source bytes, as before formats existed.
    let (kind, filter) = match format.as_deref() {
        None => (ExportKind::Original, None),
        Some("dlt") => (ExportKind::Original, Some(("DLT", "dlt"))),
        Some("text") => (ExportKind::Text, Some(("텍스트", "txt"))),
        Some(_) => return Err("Invalid export format".into()),
    };
    let running = exports.start()?;
    let cancel = running.cancel.clone();
    // The name only seeds the save dialog; the user chooses the location.
    let name: String = suggested_name
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '/' | '\\' | ':') {
                '_'
            } else {
                c
            }
        })
        .take(200)
        .collect();
    let engine = state.0.clone();
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut dialog = app
            .dialog()
            .file()
            .set_title("결과 내보내기")
            .set_file_name(if name.trim().is_empty() {
                "logstudio-export.log"
            } else {
                &name
            });
        if let Some((label, extension)) = filter {
            dialog = dialog.add_filter(label, &[extension]);
        }
        let chosen = dialog.blocking_save_file();
        let Some(path) = chosen else {
            return Ok(json!({"cancelled":true}));
        };
        let path = path
            .into_path()
            .map_err(|_| "Select a local file".to_string())?;
        let mut last = std::time::Instant::now();
        engine
            .export_as(
                &pane,
                dataset_id,
                query_id,
                &path,
                kind,
                &cancel,
                |rows, bytes| {
                    if last.elapsed() >= std::time::Duration::from_millis(100) {
                        last = std::time::Instant::now();
                        let _ = window.emit("export-progress", json!({"rows":rows,"bytes":bytes}));
                    }
                },
            )
            .map_err(message_or("Export failed"))
    })
    .await
    .map_err(|_| "Export failed".to_string())?
}
#[tauri::command]
fn cancel_export(window: tauri::WebviewWindow, exports: tauri::State<'_, Exports>) {
    if main_window(&window).is_err() {
        return;
    }
    if let Some(cancel) = exports.0.lock().unwrap().as_ref() {
        cancel.store(true, Ordering::Release);
    }
}
#[tauri::command]
async fn open_text(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, AppState>,
    text: String,
    name: String,
) -> Result<Value, String> {
    main_window(&window)?;
    let engine = state.0.clone();
    let id = engine.begin_open();
    tauri::async_runtime::spawn_blocking(move || {
        engine
            .open_text(id, &text, &name)
            .map_err(message_or("Paste failed"))
    })
    .await
    .map_err(|_| "Paste failed".to_string())?
}
#[tauri::command]
async fn cancel_open(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, AppState>,
    dataset_id: Option<u64>,
) -> Result<(), String> {
    main_window(&window)?;
    window.state::<clipboard::Grants>().clear();
    let engine = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if let Some(id) = dataset_id {
            engine.cancel_open(id);
        } else {
            engine.cancel_pending();
        }
    })
    .await
    .map_err(|_| "Cancel failed".into())
}
/// Keeps a file paste's grant while the page asks whether the pasted file
/// replaces the open log.
#[tauri::command]
fn hold_file_paste(window: tauri::WebviewWindow, grant: String) -> Result<(), String> {
    if window.label() != "main" || !window.is_focused().unwrap_or(false) {
        return Err("Paste requires the main window".into());
    }
    window.state::<clipboard::Grants>().hold(&grant)
}
/// Ends a file paste: `open` opens the granted file, `paste` passes the key
/// press on as the standard Paste (a text field has focus), and `discard`
/// only drops the grant (the user kept the open log).
#[tauri::command]
async fn finish_file_paste(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, AppState>,
    grant: String,
    action: String,
) -> Result<Value, String> {
    main_window(&window)?;
    let grants = window.state::<clipboard::Grants>();
    if action == "discard" {
        grants.discard(&grant);
        return Ok(json!({"ignored":true}));
    }
    if !matches!(action.as_str(), "open" | "paste") {
        return Err("Invalid paste action".into());
    }
    if !window.is_focused().unwrap_or(false) {
        return Err("Paste requires the main window".into());
    }
    let path = grants.consume(&grant)?;
    if action == "paste" {
        #[cfg(target_os = "macos")]
        window
            .app_handle()
            .run_on_main_thread(clipboard::mac::standard_paste)
            .map_err(|_| "Paste failed".to_string())?;
        return Ok(json!({"ignored":true}));
    }
    let engine = state.0.clone();
    let id = engine.begin_open();
    window
        .state::<recent::Recent>()
        .expect(id, std::slice::from_ref(&path));
    tauri::async_runtime::spawn_blocking(move || engine.open_file(id, &path).map_err(message))
        .await
        .map_err(|_| "File paste failed".to_string())?
}
#[tauri::command]
async fn finish_prefix_paste(window: tauri::WebviewWindow, grant: String) -> Result<(), String> {
    if window.label() != "main" || !window.is_focused().unwrap_or(false) {
        return Err("Paste requires the main window".into());
    }
    window.state::<clipboard::Grants>().consume_prefix(&grant)?;
    #[cfg(target_os = "macos")]
    window
        .app_handle()
        .run_on_main_thread(clipboard::mac::standard_paste)
        .map_err(|_| "Paste failed".to_string())?;
    Ok(())
}
#[tauri::command]
fn recent_files(
    window: tauri::WebviewWindow,
    recent: tauri::State<'_, recent::Recent>,
) -> Result<Value, String> {
    main_window(&window)?;
    Ok(recent.list())
}
/// Returns, once, an error from startup arguments or the file they named.
#[tauri::command]
fn take_startup_error(
    window: tauri::WebviewWindow,
    startup: tauri::State<'_, Startup>,
) -> Result<Option<String>, String> {
    main_window(&window)?;
    Ok(startup.0.lock().unwrap().take())
}
/// Reopens a recorded file by its opaque ID. A file that no longer exists is
/// dropped from the list.
#[tauri::command]
async fn open_recent(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, AppState>,
    id: u64,
) -> Result<Value, String> {
    main_window(&window)?;
    let recent = window.state::<recent::Recent>();
    let paths = recent
        .paths(id)
        .ok_or_else(|| "Recent file is no longer listed".to_string())?;
    let engine = state.0.clone();
    let source = engine.begin_open();
    recent.expect(source, &paths);
    let result = tauri::async_runtime::spawn_blocking(move || engine.open_files(source, &paths))
        .await
        .map_err(|_| "Request failed".to_string())?;
    if result
        .as_ref()
        .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
    {
        recent.forget(Some(id));
        let _ = window.emit("recent-changed", recent.list());
    }
    result.map_err(message)
}
/// Removing a recent entry, or clearing the list, also forgets saved work.
#[tauri::command]
fn forget_recent(window: tauri::WebviewWindow, id: Option<u64>) -> Result<Value, String> {
    main_window(&window)?;
    let recent = window.state::<recent::Recent>();
    let works = window.state::<work::WorkStates>();
    match id {
        Some(id) => {
            if let Some(paths) = recent.paths(id) {
                works.forget(Some(&paths));
            }
        }
        None => works.forget(None),
    }
    recent.forget(id);
    Ok(recent.list())
}
/// Whether a dataset can be reopened, and the work saved for its files. The
/// page names the dataset; its paths and stamps stay here.
#[tauri::command]
async fn work_state(window: tauri::WebviewWindow, dataset_id: u64) -> Result<Value, String> {
    main_window(&window)?;
    Ok(work_info(window.app_handle(), dataset_id))
}
fn work_info(app: &tauri::AppHandle, id: u64) -> Value {
    let engine = &app.state::<AppState>().0;
    // A capture file is temporary and tied to its capture.
    if engine.capture(id).is_some() {
        return json!({"reopenable":false,"capture":true});
    }
    match engine.grants(id) {
        Some(grants) => app
            .state::<work::WorkStates>()
            .lookup(id, work::stamps(grants)),
        None => json!({"reopenable":false}),
    }
}
#[tauri::command]
async fn save_work_state(
    window: tauri::WebviewWindow,
    dataset_id: u64,
    state: Value,
) -> Result<bool, String> {
    main_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<work::WorkStates>().save(dataset_id, state)
    })
    .await
    .map_err(|_| "Request failed".to_string())?
}
/// The adb captures use: found where installers put it, or picked.
#[tauri::command]
async fn adb_status(window: tauri::WebviewWindow) -> Result<Value, String> {
    main_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || app.state::<devices::Devices>().status())
        .await
        .map_err(|_| "Request failed".to_string())
}
#[tauri::command]
async fn pick_adb(window: tauri::WebviewWindow) -> Result<Value, String> {
    main_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let chosen = app
            .dialog()
            .file()
            .set_title("adb 위치 지정")
            .blocking_pick_file();
        let Some(path) = chosen else {
            return Ok(json!({"cancelled":true}));
        };
        let path = path
            .into_path()
            .map_err(|_| "Select a local file".to_string())?;
        app.state::<devices::Devices>().pick(&path)
    })
    .await
    .map_err(|_| "File dialog failed".to_string())?
}
/// The Android devices and iPhones connected now, and why a kind cannot
/// be listed.
#[tauri::command]
async fn list_devices(window: tauri::WebviewWindow) -> Result<Value, String> {
    main_window(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || app.state::<devices::Devices>().list())
        .await
        .map_err(|_| "Request failed".to_string())
}
/// Starts capturing a device of the last listing and shows its rows.
#[tauri::command]
async fn start_capture(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, AppState>,
    serial: String,
) -> Result<Value, String> {
    main_window(&window)?;
    let engine = state.0.clone();
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let id = engine.begin_open();
        let capture = match app.state::<devices::Devices>().start(&serial) {
            Ok(capture) => capture,
            Err(e) => {
                engine.cancel_open(id);
                return Err(e);
            }
        };
        engine.open_capture(id, capture).map_err(message)
    })
    .await
    .map_err(|_| "Request failed".to_string())?
}
#[tauri::command]
async fn stop_capture(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, AppState>,
    dataset_id: u64,
) -> Result<Value, String> {
    main_window(&window)?;
    let engine = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || engine.stop_capture(dataset_id).map_err(message))
        .await
        .map_err(|_| "Request failed".to_string())?
}
/// Why no app can be started for a dataset: it shows no iPhone or iPad
/// being recorded.
const NOT_RECORDING_IOS: &str = "Apps start only while an iPhone or iPad is recording";
/// The capture of the iPhone or iPad that dataset `id` records now.
fn recorded_iphone(
    engine: &Engine,
    id: u64,
) -> Result<Arc<logstudio_core::capture::Capture>, String> {
    use logstudio_core::capture::{link::Platform, Phase};
    let capture = engine.capture(id).ok_or("No device capture is open")?;
    if capture.platform != Platform::Ios || capture.phase() != Phase::Recording {
        return Err(NOT_RECORDING_IOS.into());
    }
    Ok(capture)
}
/// The developer builds on the iPhone or iPad a capture records.
#[tauri::command]
async fn capture_apps(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, AppState>,
    dataset_id: u64,
) -> Result<Value, String> {
    main_window(&window)?;
    let serial = recorded_iphone(&state.0, dataset_id)?.serial();
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || app.state::<devices::Devices>().apps(&serial))
        .await
        .map_err(|_| "Request failed".to_string())?
}
/// Starts a listed app on the iPhone or iPad a capture records, ending a
/// running copy first, so that its rows keep their private values. The
/// capture marks the first row recorded after the start began, which the
/// page shows; the new process logs only after it.
#[tauri::command]
async fn launch_capture_app(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, AppState>,
    dataset_id: u64,
    bundle: String,
) -> Result<Value, String> {
    main_window(&window)?;
    let capture = recorded_iphone(&state.0, dataset_id)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let row = capture.rows() + 1;
        let at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        let started = app
            .state::<devices::Devices>()
            .launch(&capture.serial(), &bundle)?;
        capture.mark_launch(logstudio_core::capture::Launch {
            row,
            at,
            app: started.name.clone(),
            process: started.executable.clone(),
        });
        Ok(json!({"name":started.name,"process":started.executable}))
    })
    .await
    .map_err(|_| "Request failed".to_string())?
}
/// Saves a copy of the rows a capture recorded so far.
#[tauri::command]
async fn save_capture(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, AppState>,
    exports: tauri::State<'_, Exports>,
    dataset_id: u64,
) -> Result<Value, String> {
    main_window(&window)?;
    let capture = state
        .0
        .capture(dataset_id)
        .ok_or("No device capture is open")?;
    let running = exports.start()?;
    let cancel = running.cancel.clone();
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let name = capture
            .path()
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "device.log".into());
        let chosen = app
            .dialog()
            .file()
            .set_title("기기 로그 저장")
            .set_file_name(&name)
            .blocking_save_file();
        let Some(path) = chosen else {
            return Ok(json!({"cancelled":true}));
        };
        let path = path
            .into_path()
            .map_err(|_| "Select a local file".to_string())?;
        let bytes = capture
            .save(&path, &cancel)
            .map_err(message_or("Save failed"))?;
        Ok(json!({"bytes":bytes,"capture":capture.json()}))
    })
    .await
    .map_err(|_| "Save failed".to_string())?
}
/// Whether closing the capture's log deletes its file: the user chose not
/// to keep the rows.
#[tauri::command]
fn discard_capture(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, AppState>,
    dataset_id: u64,
    discard: bool,
) -> Result<(), String> {
    main_window(&window)?;
    let capture = state
        .0
        .capture(dataset_id)
        .ok_or("No device capture is open")?;
    capture.discard(discard);
    Ok(())
}
#[tauri::command]
fn capture_leftovers(window: tauri::WebviewWindow) -> Result<Value, String> {
    main_window(&window)?;
    Ok(window.state::<devices::Devices>().leftovers())
}
#[tauri::command]
async fn open_leftover(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, AppState>,
    id: u64,
) -> Result<Value, String> {
    main_window(&window)?;
    let engine = state.0.clone();
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let capture = app.state::<devices::Devices>().adopt(id)?;
        let dataset = engine.begin_open();
        engine.open_capture(dataset, capture).map_err(message)
    })
    .await
    .map_err(|_| "Request failed".to_string())?
}
#[tauri::command]
fn delete_leftover(window: tauri::WebviewWindow, id: u64) -> Result<Value, String> {
    main_window(&window)?;
    window.state::<devices::Devices>().remove(id)
}
/// Opens or drops the files that were dropped onto a capture's log.
#[tauri::command]
fn finish_drop(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, AppState>,
    token: u64,
    open: bool,
) -> Result<(), String> {
    main_window(&window)?;
    let held = window.state::<Held>();
    let paths = {
        let mut slot = held.drop.lock().unwrap();
        match slot.take() {
            Some((t, paths)) if t == token => paths,
            other => {
                *slot = other;
                return Err("The dropped files are gone; drop them again".into());
            }
        }
    };
    if open {
        open_dropped(window.app_handle(), &state.0, paths);
    }
    Ok(())
}
/// The page answered about closing: `close` closes the window, else it stays.
#[tauri::command]
fn answer_close(window: tauri::WebviewWindow, close: bool) -> Result<(), String> {
    main_window(&window)?;
    let held = window.state::<Held>();
    *held.asked.lock().unwrap() = None;
    if close {
        held.closing.store(true, Ordering::Release);
        window
            .close()
            .map_err(|_| "Could not close the window".to_string())?;
    }
    Ok(())
}
/// Saved filters, named by opaque ID; the page validates what it applies.
#[tauri::command]
fn saved_filters(
    window: tauri::WebviewWindow,
    filters: tauri::State<'_, filters::SavedFilters>,
) -> Result<Value, String> {
    main_window(&window)?;
    Ok(filters.list())
}
#[tauri::command]
fn save_filter(
    window: tauri::WebviewWindow,
    filters: tauri::State<'_, filters::SavedFilters>,
    id: Option<u64>,
    name: String,
    format: String,
    filter: Value,
) -> Result<Value, String> {
    main_window(&window)?;
    filters.save(id, &name, &format, filter)
}
#[tauri::command]
fn rename_filter(
    window: tauri::WebviewWindow,
    filters: tauri::State<'_, filters::SavedFilters>,
    id: u64,
    name: String,
) -> Result<Value, String> {
    main_window(&window)?;
    filters.rename(id, &name)
}
#[tauri::command]
fn delete_filter(
    window: tauri::WebviewWindow,
    filters: tauri::State<'_, filters::SavedFilters>,
    id: u64,
) -> Result<Value, String> {
    main_window(&window)?;
    filters.delete(id)
}
/// Reopens the open log's own files (same paths and order) so a
/// changed file can be read again; the page keeps its work across it.
#[tauri::command]
async fn reopen_source(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, AppState>,
    dataset_id: u64,
) -> Result<Value, String> {
    main_window(&window)?;
    let engine = state.0.clone();
    let grants = engine
        .grants(dataset_id)
        .ok_or_else(|| "Nothing to reopen; select the file again".to_string())?;
    let paths: Vec<_> = grants.into_iter().map(|g| g.path).collect();
    let id = engine.begin_open();
    window.state::<work::WorkStates>().reloading(id, dataset_id);
    window.state::<recent::Recent>().expect(id, &paths);
    tauri::async_runtime::spawn_blocking(move || engine.reopen(id, dataset_id).map_err(message))
        .await
        .map_err(|_| "Request failed".to_string())?
}
/// The page's app extensions, which only `extras` builds have
/// (`logstudio_extras::request`): `list` lists none in other builds. It is
/// async so it runs off the main thread: `open` builds a window, which on
/// Windows waits for the main thread and hangs a command running on it.
#[tauri::command]
async fn extras(window: tauri::WebviewWindow, request: Value) -> Result<Value, String> {
    main_window(&window)?;
    extras_request(window.app_handle(), request)
}
#[cfg(feature = "extras")]
fn extras_request(app: &tauri::AppHandle, request: Value) -> Result<Value, String> {
    logstudio_extras::request(app, request)
}
#[cfg(not(feature = "extras"))]
fn extras_request(_: &tauri::AppHandle, request: Value) -> Result<Value, String> {
    match request["action"].as_str() {
        Some("list") => Ok(json!([])),
        _ => Err("This build has no extensions".into()),
    }
}
#[cfg(feature = "qa")]
#[tauri::command]
async fn qa_open_fixture(state: tauri::State<'_, AppState>, name: String) -> Result<Value, String> {
    if name.contains('/') || name.contains('\\') || name.contains("..") || !fixture_name(&name) {
        return Err("Fixture name only".into());
    }
    let root = std::env::var("LOGSTUDIO_QA_FIXTURES")
        .map_err(|_| "QA fixture root required".to_string())?;
    let path = std::path::PathBuf::from(root).join(name);
    let engine = state.0.clone();
    let id = engine.begin_open();
    tauri::async_runtime::spawn_blocking(move || engine.open_file(id, &path).map_err(message))
        .await
        .map_err(|_| "QA grant failed".to_string())?
}
/// Chooses the app menu's Quit (QA builds only), as ⌘Q does; returns the
/// app menu's item IDs so a run can see the Quit it chose is installed.
#[cfg(feature = "qa")]
#[tauri::command]
fn qa_quit(app: tauri::AppHandle) -> Vec<String> {
    let ids = app
        .menu()
        .and_then(|menu| menu.items().ok())
        .and_then(|items| items.into_iter().find_map(|i| i.as_submenu().cloned()))
        .and_then(|app_menu| app_menu.items().ok())
        .map(|items| items.iter().map(|i| i.id().0.clone()).collect())
        .unwrap_or_default();
    quit(&app);
    ids
}
/// Scripted devices instead of adb and Apple's device service (QA builds
/// only): `fake` lists a ready Android device that logs synthetic rows and
/// one awaiting authorization; `unplug`, `plug`, `reboot`, `unauthorized`
/// and `authorized` act on the ready one; `conflict` drops it while another
/// adb version holds the server, which ends a capture; `missing` acts as if
/// no adb were installed; `real` uses adb again. `ios` lists a ready iPhone
/// that logs synthetic rows and an iPad that does not trust the computer;
/// `ios-unplug`, `ios-plug`, `ios-untrusted` and `ios-trusted` act on the
/// iPhone and `ios-ipad-trusted` on the iPad; `ios-missing` acts as if no
/// device service were installed, and `ios-real` uses the service again.
/// The iPhone has two developer builds; starting SampleApp shows its
/// private values. `ios-no-xcode` acts as if Xcode were not installed, and
/// `ios-xcode` as if it were again; `ios-slow-launch` makes starting an app
/// take 6 s.
#[cfg(feature = "qa")]
#[tauri::command]
fn qa_devices(app: tauri::AppHandle, action: String) -> Result<(), String> {
    use logstudio_core::capture::fake::FakeLink;
    type Fake = Option<(Arc<FakeLink>, Arc<AtomicBool>)>;
    static FAKE: Mutex<Fake> = Mutex::new(None);
    static FAKE_IOS: Mutex<Fake> = Mutex::new(None);
    // A new script replaces the old one, whose rows stop.
    fn stop(fake: &mut Fake) {
        if let Some((_, feeding)) = fake.take() {
            feeding.store(true, Ordering::Release);
        }
    }
    const READY: &str = "emulator-5554";
    const IPHONE: &str = "QA0UDID0000000000000001";
    const IPAD: &str = "QA0UDID0000000000000002";
    let devices = app.state::<devices::Devices>();
    if let Some(action) = action.strip_prefix("ios") {
        let mut fake = FAKE_IOS.lock().unwrap();
        match action {
            "" => {
                stop(&mut fake);
                let link = FakeLink::live();
                link.add_ios(IPHONE, "QA iPhone", "27.0");
                link.add_ios(IPAD, "QA iPad", "27.0");
                link.set_state(IPAD, "untrusted");
                link.add_app(IPHONE, "com.example.sample", "SampleApp", "1.4.0");
                link.add_app(
                    IPHONE,
                    "com.example.sample.widgets",
                    "Sample Widgets",
                    "0.9.1",
                );
                let feeding = link.feed(IPHONE, std::time::Duration::from_millis(150));
                devices.replace_ios(Some(devices::Replaced::Link(link.clone())));
                devices.replace_apps(Some(link.clone()));
                *fake = Some((link, feeding));
            }
            "-no-xcode" | "-xcode" => {
                let (link, _) = fake.as_ref().ok_or("No fake devices")?;
                let missing = logstudio_core::capture::apps::NO_XCODE;
                link.apps_error((action == "-no-xcode").then_some(missing));
            }
            "-slow-launch" => {
                let (link, _) = fake.as_ref().ok_or("No fake devices")?;
                link.launch_delay(std::time::Duration::from_secs(6));
            }
            "-unplug" | "-plug" | "-untrusted" | "-trusted" | "-ipad-trusted" => {
                let (link, _) = fake.as_ref().ok_or("No fake devices")?;
                match action {
                    "-unplug" => link.unplug(IPHONE),
                    "-plug" => link.plug(IPHONE),
                    "-untrusted" => link.set_state(IPHONE, "untrusted"),
                    "-ipad-trusted" => link.set_state(IPAD, "device"),
                    _ => link.set_state(IPHONE, "device"),
                }
            }
            "-missing" => devices.replace_ios(Some(devices::Replaced::Missing)),
            "-real" => {
                devices.replace_ios(None);
                devices.replace_apps(None);
            }
            _ => return Err("Unknown QA device action".into()),
        }
        return Ok(());
    }
    let mut fake = FAKE.lock().unwrap();
    match action.as_str() {
        "fake" => {
            stop(&mut fake);
            let link = FakeLink::new();
            link.add(READY, "Pixel_8", "QA-PIXEL-8");
            link.add("QA0SERIAL01", "Tablet_X1", "QA-TABLET");
            link.set_state("QA0SERIAL01", "unauthorized");
            let feeding = link.feed(READY, std::time::Duration::from_millis(150));
            devices.replace(Some(devices::Replaced::Link(link.clone())));
            *fake = Some((link, feeding));
        }
        "unplug" | "plug" | "reboot" | "conflict" | "unauthorized" | "authorized" => {
            let (link, _) = fake.as_ref().ok_or("No fake devices")?;
            match action.as_str() {
                "unplug" => link.unplug(READY),
                "plug" => link.plug(READY),
                "unauthorized" => link.set_state(READY, "unauthorized"),
                "authorized" => link.set_state(READY, "device"),
                "conflict" => {
                    link.conflict(true);
                    link.unplug(READY);
                }
                _ => link.reboot(READY),
            }
        }
        "missing" => devices.replace(Some(devices::Replaced::Missing)),
        "real" => devices.replace(None),
        _ => return Err("Unknown QA device action".into()),
    }
    Ok(())
}
/// A QA run in the background (QA builds only) keeps out of the way of the
/// person at the desk: the app does not activate when it starts, has no Dock
/// icon, and its window opens without focus below other windows. Its page is
/// not throttled while covered, so checks run at the usual pace.
#[cfg(feature = "qa")]
fn qa_background() -> bool {
    std::env::var_os("LOGSTUDIO_QA_BACKGROUND").is_some_and(|v| v == "1")
}
/// QA fixtures are text logs or DLT files.
#[cfg(feature = "qa")]
fn fixture_name(name: &str) -> bool {
    name.ends_with(".log") || name.ends_with(".dlt")
}
/// Opens several fixtures as one combined log (QA builds only): in the given
/// order, or with `arrange` ordered as a pick or drop would be.
#[cfg(feature = "qa")]
#[tauri::command]
async fn qa_open_fixtures(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    names: Vec<String>,
    arrange: Option<bool>,
) -> Result<Value, String> {
    let root = std::env::var("LOGSTUDIO_QA_FIXTURES")
        .map_err(|_| "QA fixture root required".to_string())?;
    let mut paths = vec![];
    for name in names {
        if name.contains('/') || name.contains('\\') || name.contains("..") || !fixture_name(&name)
        {
            return Err("Fixture name only".into());
        }
        paths.push(std::path::PathBuf::from(&root).join(name));
    }
    let engine = state.0.clone();
    let id = engine.begin_open();
    tauri::async_runtime::spawn_blocking(move || {
        if arrange == Some(true) {
            open_arranged(&app, &engine, id, paths)
        } else {
            engine.open_files(id, &paths)
        }
        .map_err(message)
    })
    .await
    .map_err(|_| "QA grant failed".to_string())?
}
fn main() {
    // Invalid arguments still open the app, which shows the error: a release
    // build on Windows has no console for it.
    let (launch, launch_error) =
        match logstudio_core::launch::Launch::parse(std::env::args_os().skip(1)) {
            Ok(c) => (c, None),
            Err(e) => {
                eprintln!("LogStudio: {e}");
                (
                    Default::default(),
                    Some(logstudio_core::source::public_message(
                        &e,
                        "Unsupported command-line arguments",
                    )),
                )
            }
        };
    let engine = Arc::new(Engine::default());
    let mut builder = tauri::Builder::default();
    #[cfg(feature = "extras")]
    {
        #[cfg(feature = "qa")]
        let background = qa_background();
        #[cfg(not(feature = "qa"))]
        let background = false;
        builder = logstudio_extras::install(builder, engine.clone(), background);
    }
    builder = builder
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState(engine))
        .manage(clipboard::Grants::default())
        .manage(Exports::default())
        .manage(recent::Recent::default())
        .manage(geometry::Geometry::default())
        .manage(work::WorkStates::default())
        .manage(filters::SavedFilters::default())
        .manage(devices::Devices::default())
        .manage(Held::default())
        .manage(Startup(Mutex::new(launch_error)))
        .setup(move |app| {
            #[cfg(target_os = "macos")]
            {
                clipboard::mac::install_menu(app.handle())?;
                install_quit(app.handle())?;
            }
            let window =
                tauri::WebviewWindowBuilder::from_config(app, &app.config().app.windows[0])?
                    .on_navigation(|url| {
                        (url.scheme() == "tauri" && url.host_str() == Some("localhost"))
                            || (url.scheme() == "http" && url.host_str() == Some("tauri.localhost"))
                    });
            #[cfg(feature = "qa")]
            let window = if qa_background() {
                window
                    .focused(false)
                    .always_on_bottom(true)
                    .background_throttling(
                        tauri::utils::config::BackgroundThrottlingPolicy::Disabled,
                    )
            } else {
                window
            };
            // The window opens where it was last closed.
            let geometry = app.state::<geometry::Geometry>();
            if let Ok(dir) = app.path().app_data_dir() {
                geometry.load(&dir);
            }
            geometry.open(app.handle(), window)?;
            let engine = app.state::<AppState>().0.clone();
            let handle = app.handle().clone();
            if let (Ok(data), Ok(cache)) = (app.path().app_data_dir(), app.path().app_cache_dir()) {
                app.state::<devices::Devices>().load(&data, &cache);
            }
            if let Ok(dir) = app.path().app_data_dir() {
                app.state::<recent::Recent>().load(&dir);
                app.state::<work::WorkStates>().load(&dir);
                app.state::<filters::SavedFilters>().load(&dir);
            }
            engine.observe(move |mut event| {
                // A new source carries its saved work, so the page can apply
                // it as it switches; startup sources ask with `work_state`.
                if let (Some("opened"), Some(id)) =
                    (event["status"].as_str(), event["datasetId"].as_u64())
                {
                    event["work"] = work_info(&handle, id);
                }
                let channel = match event["status"].as_str() {
                    Some("query-progress") => "query-progress",
                    Some("capture") => "capture-changed",
                    _ => "source-changed",
                };
                // Validation decides whether an expected file joins the recent list.
                if let (Some(status @ ("validated" | "rollback")), Some(id)) =
                    (event["status"].as_str(), event["datasetId"].as_u64())
                {
                    let recent = handle.state::<recent::Recent>();
                    if recent.settle(id, status == "validated") {
                        let _ = handle.emit_to("main", "recent-changed", recent.list());
                    }
                }
                let _ = handle.emit_to("main", channel, event);
            });
            if !launch.files.is_empty() {
                let paths = launch.files.clone();
                let id = engine.begin_open();
                app.state::<recent::Recent>().expect(id, &paths);
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    if let Err(e) = engine.open_files(id, &paths) {
                        let error = logstudio_core::source::public_message(
                            &e,
                            "Could not open selected file; check availability and retry",
                        );
                        handle.state::<Startup>().report(&handle, error);
                    }
                });
            }
            Ok(())
        })
        .on_menu_event(|app, event| {
            #[cfg(target_os = "macos")]
            if event.id().as_ref() == clipboard::mac::PASTE {
                clipboard::mac::menu_paste(app);
            } else if event.id().as_ref() == QUIT {
                quit(app);
            }
            #[cfg(not(target_os = "macos"))]
            let _ = (app, event);
        })
        .on_window_event(|window, event| {
            // Other windows, such as an extension's, handle their own events.
            if window.label() != "main" {
                return;
            }
            let engine = window.state::<AppState>().0.clone();
            match event {
                // Entry and exit only drive the drop hint; no path is read for
                // it. A drag without files, such as page text, opens nothing.
                tauri::WindowEvent::DragDrop(event) => match drag::decide(event) {
                    drag::Drag::Hint(active) => {
                        let _ = window.emit("drag-state", json!({"active":active}));
                    }
                    drag::Drag::Open(paths) => {
                        let _ = window.emit("drag-state", json!({"active":false}));
                        // A capture's rows are asked about first.
                        if engine.shown_capture().is_some() {
                            let count = paths.len();
                            let token = window.state::<Held>().hold(paths);
                            let _ = window.emit("drop-held", json!({"token":token,"count":count}));
                        } else {
                            open_dropped(window.app_handle(), &engine, paths);
                        }
                    }
                    drag::Drag::Ignore => {}
                },
                tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_) => {
                    window
                        .state::<geometry::Geometry>()
                        .changed(window.app_handle());
                }
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    if engine.shown_capture().is_some() && window.state::<Held>().delays_close() {
                        api.prevent_close();
                        let _ = window.emit("close-held", json!({}));
                        return;
                    }
                    window
                        .state::<geometry::Geometry>()
                        .flush(window.app_handle());
                }
                tauri::WindowEvent::Destroyed => {
                    window.state::<clipboard::Grants>().clear();
                    engine.close();
                    // The app ends with the main window, as it does with its
                    // last window.
                    for (label, other) in window.app_handle().webview_windows() {
                        if label != "main" {
                            let _ = other.destroy();
                        }
                    }
                }
                _ => {}
            }
        });
    #[cfg(feature = "qa")]
    {
        builder = builder
            .plugin(tauri_plugin_wdio_webdriver::init())
            .invoke_handler(tauri::generate_handler![
                log_request,
                pick_file,
                open_text,
                cancel_open,
                hold_file_paste,
                finish_file_paste,
                finish_prefix_paste,
                export_results,
                cancel_export,
                recent_files,
                open_recent,
                forget_recent,
                work_state,
                save_work_state,
                saved_filters,
                save_filter,
                rename_filter,
                delete_filter,
                reopen_source,
                take_startup_error,
                adb_status,
                pick_adb,
                list_devices,
                start_capture,
                stop_capture,
                save_capture,
                discard_capture,
                capture_leftovers,
                capture_apps,
                launch_capture_app,
                open_leftover,
                delete_leftover,
                finish_drop,
                answer_close,
                extras,
                qa_devices,
                qa_quit,
                qa_open_fixture,
                qa_open_fixtures
            ]);
    }
    #[cfg(not(feature = "qa"))]
    {
        builder = builder.invoke_handler(tauri::generate_handler![
            log_request,
            pick_file,
            open_text,
            cancel_open,
            hold_file_paste,
            finish_file_paste,
            finish_prefix_paste,
            export_results,
            cancel_export,
            recent_files,
            open_recent,
            forget_recent,
            work_state,
            save_work_state,
            saved_filters,
            save_filter,
            rename_filter,
            delete_filter,
            reopen_source,
            take_startup_error,
            adb_status,
            pick_adb,
            list_devices,
            start_capture,
            stop_capture,
            save_capture,
            discard_capture,
            capture_leftovers,
            capture_apps,
            launch_capture_app,
            open_leftover,
            delete_leftover,
            finish_drop,
            answer_close,
            extras
        ]);
    }
    #[allow(unused_mut)]
    let mut app = builder
        .build(tauri::generate_context!())
        .expect("Build LogStudio app");
    // Set before launch: an app that may not activate takes no focus when it
    // starts, which an activation request at launch would otherwise do.
    #[cfg(all(feature = "qa", target_os = "macos"))]
    if qa_background() {
        app.set_activation_policy(tauri::ActivationPolicy::Prohibited);
    }
    app.run(|handle, event| {
        if matches!(event, tauri::RunEvent::Exit) {
            // Quitting from the Dock or logging out closes no window first.
            handle.state::<geometry::Geometry>().flush(handle);
            handle.state::<AppState>().0.close();
        }
    });
}
