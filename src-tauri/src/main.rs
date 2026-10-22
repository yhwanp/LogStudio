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
/// Cancellation flag of the one export that may run at a time.
#[derive(Default)]
struct Exports(Mutex<Option<Arc<AtomicBool>>>);
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
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut slot = exports.0.lock().unwrap();
        if slot.is_some() {
            return Err("An export is already running".into());
        }
        *slot = Some(cancel.clone());
    }
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
    let result = tauri::async_runtime::spawn_blocking(move || {
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
    .map_err(|_| "Export failed".to_string())
    .and_then(|r| r);
    *exports.0.lock().unwrap() = None;
    result
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
    match app.state::<AppState>().0.grants(id) {
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
    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState(Arc::new(Engine::default())))
        .manage(clipboard::Grants::default())
        .manage(Exports::default())
        .manage(recent::Recent::default())
        .manage(geometry::Geometry::default())
        .manage(work::WorkStates::default())
        .manage(filters::SavedFilters::default())
        .manage(Startup(Mutex::new(launch_error)))
        .setup(move |app| {
            #[cfg(target_os = "macos")]
            clipboard::mac::install_menu(app.handle())?;
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
                let channel = if event["status"] == "query-progress" {
                    "query-progress"
                } else {
                    "source-changed"
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
            }
            #[cfg(not(target_os = "macos"))]
            let _ = (app, event);
        })
        .on_window_event(|window, event| {
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
                        // Several dropped files open as one log; heads are read
                        // off the event loop.
                        let app = window.app_handle().clone();
                        let id = engine.begin_open();
                        std::thread::spawn(move || {
                            if let Err(error) = open_arranged(&app, &engine, id, paths) {
                                engine.emit_error(error);
                            }
                        });
                    }
                    drag::Drag::Ignore => {}
                },
                tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_) => {
                    window
                        .state::<geometry::Geometry>()
                        .changed(window.app_handle());
                }
                tauri::WindowEvent::CloseRequested { .. } => {
                    window
                        .state::<geometry::Geometry>()
                        .flush(window.app_handle());
                }
                tauri::WindowEvent::Destroyed => {
                    window.state::<clipboard::Grants>().clear();
                    engine.close();
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
            take_startup_error
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
            // Quitting from the menu closes no window first.
            handle.state::<geometry::Geometry>().flush(handle);
            handle.state::<AppState>().0.close();
        }
    });
}
