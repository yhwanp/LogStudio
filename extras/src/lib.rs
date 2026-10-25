//! Optional extensions of the LogStudio desktop app, built in only with its
//! `extras` feature, so released builds hold none of them.
//!
//! An extension turns on when its settings file `extras/<id>.json` exists in
//! the app's data folder; the page's 확장 menu lists it and opens its own
//! window. That window loads its page from the `extras` URI scheme, which
//! serves the extension's embedded files and answers its requests, so the
//! page needs no Tauri IPC and has a content security policy of its own.
//! The main page lists and opens extensions and passes on the row it selects
//! with the `extras` command (`request`); a row an extension's page picks
//! reaches the main page as an `extras-locate` event.
//!
//! A new extension is a module with an `ID` and a `make` that `EXTRAS`
//! lists, and its page files under `web/<id>/`.
pub mod track;

use logstudio_core::Engine;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};
use tauri::{http, AppHandle, Emitter, Manager, Runtime, UriSchemeContext, UriSchemeResponder};

/// Makes an extension from its settings file's JSON, or says what is wrong
/// with them.
type Make = fn(&Value) -> Result<Arc<dyn Extra>, String>;
/// The extensions this build knows: ID, the name its menu item has while
/// its settings are broken, and its maker.
const EXTRAS: &[(&str, &str, Make)] = &[(track::ID, track::NAME, track::make)];
/// Settings files are small; a larger one is a mistake.
const MAX_SETTINGS: u64 = 64 * 1024;
/// How long a page's `host/state` request waits for a change.
const STATE_WAIT: Duration = Duration::from_secs(20);

/// One extension, made from its settings.
pub trait Extra: Send + Sync {
    /// Its menu item and window title.
    fn title(&self) -> String;
    /// Where its page loads images from besides its own files, as content
    /// security policy sources, such as a map's tile server.
    fn image_sources(&self) -> Vec<String> {
        vec![]
    }
    /// One of its page's files, by its path under the page's folder.
    fn file(&self, path: &str) -> Option<File>;
    /// Answers its page's request to `api/<path>`, off the main thread.
    fn api(&self, host: &Host, path: &str, query: &Query) -> Reply;
    /// Its window closed; work for its page can stop.
    fn closed(&self) {}
}

/// An embedded page file and its media type.
pub type File = (&'static [u8], &'static str);

/// Files every extension's page can load: `common/` (the host client and
/// shared styles) and `vendor/` (third-party libraries).
fn shared_file(path: &str) -> Option<File> {
    Some(match path {
        "common/host.js" => (include_bytes!("../web/common/host.js"), JS),
        "common/base.css" => (include_bytes!("../web/common/base.css"), CSS),
        "vendor/leaflet/leaflet.js" => (include_bytes!("../web/vendor/leaflet/leaflet.js"), JS),
        "vendor/leaflet/leaflet.css" => (include_bytes!("../web/vendor/leaflet/leaflet.css"), CSS),
        _ => return None,
    })
}
pub const HTML: &str = "text/html; charset=utf-8";
pub const JS: &str = "text/javascript; charset=utf-8";
pub const CSS: &str = "text/css; charset=utf-8";

/// A response to an extension page's request.
pub struct Reply {
    status: u16,
    kind: &'static str,
    body: Vec<u8>,
    headers: Vec<(&'static str, String)>,
}
impl Reply {
    pub fn json(value: Value) -> Self {
        Self::bytes("application/json", value.to_string().into_bytes())
    }
    pub fn bytes(kind: &'static str, body: Vec<u8>) -> Self {
        Self {
            status: 200,
            kind,
            body,
            headers: vec![],
        }
    }
    /// A failed request; `message` is shown to the user.
    pub fn error(status: u16, message: &str) -> Self {
        Self {
            status,
            ..Self::json(json!({"error":message}))
        }
    }
    pub fn header(mut self, name: &'static str, value: String) -> Self {
        self.headers.push((name, value));
        self
    }
}

/// A request's query parameters.
pub struct Query(Vec<(String, String)>);
impl Query {
    fn parse(query: Option<&str>) -> Self {
        Self(
            query
                .unwrap_or("")
                .split('&')
                .filter_map(|pair| pair.split_once('='))
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        )
    }
    /// Parameter `name` as a whole number; the pages send no other kind.
    pub fn number(&self, name: &str) -> Option<u64> {
        self.0
            .iter()
            .find(|(k, _)| k == name)
            .and_then(|(_, v)| v.parse().ok())
    }
}

/// The extensions' shared state, managed by the app.
pub struct Host {
    engine: Arc<Engine>,
    /// Windows open behind others, without focus or throttling (QA runs).
    background: bool,
    /// Extensions made from their settings by ID: the last listing's, and
    /// those whose window is open.
    made: Mutex<HashMap<&'static str, Arc<dyn Extra>>>,
    /// Where each extension's window was last, by ID; read on first use.
    places: Mutex<Option<HashMap<String, Place>>>,
    shared: Mutex<Shared>,
    changed: Condvar,
}
/// What the main page last said, which extension pages wait for.
#[derive(Default)]
struct Shared {
    revision: u64,
    /// The dataset and physical row the main page selected last.
    selected: Option<(u64, u64)>,
}
impl Host {
    /// The log the main window shows, which extensions read.
    pub fn engine(&self) -> &Arc<Engine> {
        &self.engine
    }
    /// The ID of the dataset the main window shows.
    pub fn shown(&self) -> Option<u64> {
        self.engine.shown().map(|d| d.id)
    }
    /// Waits until the main page selects another row or shows another log,
    /// or `STATE_WAIT` passes; the page asks again with what it got.
    fn state(&self, query: &Query) -> Reply {
        let after = query.number("after");
        let known = query.number("dataset").filter(|id| *id > 0);
        let deadline = Instant::now() + STATE_WAIT;
        let mut shared = self.shared.lock().unwrap();
        loop {
            let shown = self.shown();
            if after != Some(shared.revision) || shown != known || Instant::now() >= deadline {
                let selected = shared
                    .selected
                    .filter(|(d, _)| Some(*d) == shown)
                    .map(|(_, line)| line);
                return Reply::json(json!({
                    "revision":shared.revision,
                    "datasetId":shown,
                    "selected":selected,
                }));
            }
            // The engine does not tell the host when another log is shown.
            shared = self
                .changed
                .wait_timeout(shared, Duration::from_millis(250))
                .unwrap()
                .0;
        }
    }
    /// Shows a row an extension's page picked in the main window.
    fn locate<R: Runtime>(&self, app: &AppHandle<R>, query: &Query) -> Reply {
        let (Some(dataset), Some(line)) = (query.number("dataset"), query.number("line")) else {
            return Reply::error(400, "잘못된 요청입니다.");
        };
        if line == 0 || self.shown() != Some(dataset) {
            return Reply::error(409, "메인 창의 로그가 바뀌어 행을 선택하지 못했습니다.");
        }
        let _ = app.emit_to(
            "main",
            "extras-locate",
            json!({"datasetId":dataset,"line":line}),
        );
        Reply::json(json!({"ok":true}))
    }
    fn select(&self, dataset: u64, line: u64) {
        let mut shared = self.shared.lock().unwrap();
        if shared.selected != Some((dataset, line)) {
            shared.selected = Some((dataset, line));
            shared.revision += 1;
            self.changed.notify_all();
        }
    }
    fn made(&self, id: &str) -> Option<Arc<dyn Extra>> {
        self.made.lock().unwrap().get(id).cloned()
    }
}

/// Adds the host to the app: its state and the `extras` URI scheme. With
/// `background` (QA runs), extension windows open as the main window then
/// does: behind other windows, without focus or throttling.
pub fn install<R: Runtime>(
    builder: tauri::Builder<R>,
    engine: Arc<Engine>,
    background: bool,
) -> tauri::Builder<R> {
    builder
        .manage(Host {
            engine,
            background,
            made: Mutex::new(HashMap::new()),
            places: Mutex::new(None),
            shared: Mutex::new(Shared::default()),
            changed: Condvar::new(),
        })
        .register_asynchronous_uri_scheme_protocol("extras", serve)
}

/// The main page's `extras` command: `list` the extensions whose settings
/// exist, `open` one's window by `id`, tell their pages the row it
/// `select`s (`datasetId`, `line`), or `reveal` the settings folder.
pub fn request<R: Runtime>(app: &AppHandle<R>, request: Value) -> Result<Value, String> {
    let host = app.state::<Host>();
    match request["action"].as_str() {
        Some("list") => Ok(list(app, &host)),
        Some("open") => {
            let id = request["id"].as_str().unwrap_or("");
            open(app, &host, id)
        }
        Some("select") => {
            let (Some(dataset), Some(line)) =
                (request["datasetId"].as_u64(), request["line"].as_u64())
            else {
                return Err("Invalid selection".into());
            };
            host.select(dataset, line);
            Ok(json!({}))
        }
        Some("reveal") => reveal(app),
        _ => Err("Unsupported action".into()),
    }
}

/// The settings folder, `extras/` in the app's data folder.
fn folder<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join("extras"))
}

/// Reads extension `id`'s settings: None when it has no settings file.
fn settings<R: Runtime>(app: &AppHandle<R>, id: &str) -> Option<Result<Value, String>> {
    let path = folder(app)?.join(format!("{id}.json"));
    let read = (|| {
        let meta = std::fs::metadata(&path)?;
        if meta.len() > MAX_SETTINGS {
            return Ok(Err("설정 파일이 64 KiB보다 큽니다.".to_string()));
        }
        Ok::<_, std::io::Error>(Ok(std::fs::read(&path)?))
    })();
    match read {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => Some(Err("설정 파일을 읽을 수 없습니다.".into())),
        Ok(Err(e)) => Some(Err(e)),
        Ok(Ok(bytes)) => Some(
            serde_json::from_slice(&bytes)
                .map_err(|e| format!("설정 파일이 올바른 JSON이 아닙니다 ({}행).", e.line())),
        ),
    }
}

fn label(id: &str) -> String {
    format!("extra-{id}")
}

/// The extensions whose settings file exists, made afresh unless their
/// window is open; a broken one is listed with its error.
fn list<R: Runtime>(app: &AppHandle<R>, host: &Host) -> Value {
    let mut listed = vec![];
    let mut made = host.made.lock().unwrap();
    for (id, name, make) in EXTRAS {
        let open = app.get_webview_window(&label(id)).is_some();
        if open {
            if let Some(extra) = made.get(id) {
                listed.push(json!({"id":id,"title":extra.title()}));
                continue;
            }
        }
        made.remove(id);
        let Some(value) = settings(app, id) else {
            continue;
        };
        match value.and_then(|v| make(&v)) {
            Ok(extra) => {
                listed.push(json!({"id":id,"title":extra.title()}));
                made.insert(id, extra);
            }
            Err(error) => listed.push(json!({"id":id,"title":name,"error":error})),
        }
    }
    Value::Array(listed)
}

/// Shows extension `id`'s window, opening it when it is not open.
fn open<R: Runtime>(app: &AppHandle<R>, host: &Host, id: &str) -> Result<Value, String> {
    let label = label(id);
    if let Some(window) = app.get_webview_window(&label) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(json!({}));
    }
    // Settings edited since the menu listed them apply now.
    let listed = list(app, host);
    if let Some(error) = listed
        .as_array()
        .and_then(|l| l.iter().find(|e| e["id"] == id))
        .and_then(|e| e["error"].as_str())
    {
        return Err(error.to_string());
    }
    let (id, extra) = {
        let made = host.made.lock().unwrap();
        let (&id, extra) = made
            .get_key_value(id)
            .ok_or("이 확장 기능의 설정 파일이 없습니다.")?;
        (id, extra.clone())
    };
    let url = page_url(id)?;
    let version = &app.package_info().version;
    let mut window =
        tauri::WebviewWindowBuilder::new(app, &label, tauri::WebviewUrl::CustomProtocol(url))
            .title(format!("{} · LogStudio", extra.title()))
            .inner_size(960.0, 680.0)
            .min_inner_size(480.0, 360.0)
            // Tile servers ask apps to name themselves.
            .user_agent(&format!(
                "LogStudio/{version} (+https://github.com/yhwanp/LogStudio)"
            ))
            .on_navigation(move |url| page(url).is_some_and(|(page_id, _)| page_id == id));
    // It opens where it was last, if that is on a screen now. The monitor
    // list can come back empty, as while the display sleeps; the main
    // monitor then decides, as it does for the main window.
    let screens: Vec<Place> = app
        .available_monitors()
        .ok()
        .filter(|m| !m.is_empty())
        .or_else(|| app.primary_monitor().ok().flatten().map(|m| vec![m]))
        .unwrap_or_default()
        .iter()
        .map(|m| {
            let (at, size) = (m.work_area().position, m.work_area().size);
            let scale = m.scale_factor();
            let (at, size) = (at.to_logical::<f64>(scale), size.to_logical::<f64>(scale));
            [at.x, at.y, size.width, size.height]
        })
        .collect();
    if let Some([x, y, width, height]) = host.place(app, id).filter(|p| visible(*p, &screens)) {
        window = window
            .position(x, y)
            .inner_size(width.max(480.0), height.max(360.0));
    }
    if host.background {
        window = window
            .focused(false)
            .always_on_bottom(true)
            .background_throttling(tauri::utils::config::BackgroundThrottlingPolicy::Disabled);
    }
    let window = window
        .build()
        .map_err(|_| "확장 기능 창을 열 수 없습니다.".to_string())?;
    let (handle, shown) = (app.clone(), window.clone());
    window.on_window_event(move |event| {
        let host = handle.state::<Host>();
        match event {
            tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_) => {
                host.remember(id, &shown);
            }
            tauri::WindowEvent::Destroyed => {
                host.save_places(&handle);
                if let Some(extra) = host.made(id) {
                    extra.closed();
                }
            }
            _ => {}
        }
    });
    Ok(json!({}))
}

/// Opens the settings folder in the system's file manager, making it first
/// if needed. Nothing the page sends names the folder or the program.
fn reveal<R: Runtime>(app: &AppHandle<R>) -> Result<Value, String> {
    let dir = folder(app).ok_or("설정 폴더를 찾을 수 없습니다.")?;
    std::fs::create_dir_all(&dir).map_err(|_| "설정 폴더를 만들 수 없습니다.".to_string())?;
    let program = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    let mut child = std::process::Command::new(program)
        .arg(&dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|_| "설정 폴더를 열 수 없습니다.".to_string())?;
    std::thread::spawn(move || child.wait());
    Ok(json!({}))
}

/// Where a window was, in logical pixels: x, y, width and height.
type Place = [f64; 4];

/// Whether a window at `place` would show on one of `screens` (their work
/// areas, as places): the first 100 points of its top edge, where it is
/// dragged from, lie on one.
fn visible(place: Place, screens: &[Place]) -> bool {
    let [x, y, width, height] = place;
    let sane = [x, y, width, height].iter().all(|v| v.is_finite()) && width > 0.0 && height > 0.0;
    sane && screens.iter().any(|&[sx, sy, sw, sh]| {
        let reach = (x + width.min(100.0)).min(sx + sw) - x.max(sx);
        reach >= 50.0 && y >= sy && y < sy + sh - 20.0
    })
}

impl Host {
    /// Where extension `id`'s window was last, from `extras-windows.json`.
    fn place<R: Runtime>(&self, app: &AppHandle<R>, id: &str) -> Option<Place> {
        let mut places = self.places.lock().unwrap();
        places
            .get_or_insert_with(|| read_places(app))
            .get(id)
            .copied()
    }
    /// Notes where a window is, unless it is minimized, maximized or full
    /// screen: it opens there next time.
    fn remember<R: Runtime>(&self, id: &str, window: &tauri::WebviewWindow<R>) {
        let full = window.is_minimized().unwrap_or(true)
            || window.is_maximized().unwrap_or(true)
            || window.is_fullscreen().unwrap_or(true);
        let (Ok(scale), Ok(at), Ok(size)) = (
            window.scale_factor(),
            window.outer_position(),
            window.inner_size(),
        ) else {
            return;
        };
        if full {
            return;
        }
        let (at, size) = (at.to_logical::<f64>(scale), size.to_logical::<f64>(scale));
        let mut places = self.places.lock().unwrap();
        if let Some(places) = places.as_mut() {
            places.insert(id.to_string(), [at.x, at.y, size.width, size.height]);
        }
    }
    /// Writes the windows' places through a temporary file.
    fn save_places<R: Runtime>(&self, app: &AppHandle<R>) {
        let Some(places) = self.places.lock().unwrap().clone() else {
            return;
        };
        let Ok(dir) = app.path().app_data_dir() else {
            return;
        };
        let value: serde_json::Map<String, Value> = places
            .into_iter()
            .map(|(id, [x, y, width, height])| {
                (id, json!({"x":x,"y":y,"width":width,"height":height}))
            })
            .collect();
        let path = dir.join(PLACES);
        let partial = dir.join(format!("{PLACES}.partial"));
        let written = std::fs::create_dir_all(&dir)
            .and_then(|_| std::fs::write(&partial, Value::Object(value).to_string()))
            .and_then(|_| std::fs::rename(&partial, &path));
        if written.is_err() {
            let _ = std::fs::remove_file(&partial);
        }
    }
}

/// The windows' places, in the app's data folder.
const PLACES: &str = "extras-windows.json";

fn read_places<R: Runtime>(app: &AppHandle<R>) -> HashMap<String, Place> {
    let Some(bytes) = app
        .path()
        .app_data_dir()
        .ok()
        .and_then(|dir| std::fs::read(dir.join(PLACES)).ok())
        .filter(|b| b.len() as u64 <= MAX_SETTINGS)
    else {
        return HashMap::new();
    };
    let Ok(Value::Object(object)) = serde_json::from_slice(&bytes) else {
        return HashMap::new();
    };
    object
        .into_iter()
        .filter_map(|(id, v)| {
            let n = |k: &str| v[k].as_f64();
            Some((id, [n("x")?, n("y")?, n("width")?, n("height")?]))
        })
        .collect()
}

/// Where extension `id`'s page loads from. Windows serves custom schemes
/// as `http://<scheme>.localhost`.
fn page_url(id: &str) -> Result<tauri::Url, String> {
    let base = if cfg!(windows) {
        "http://extras.localhost/"
    } else {
        "extras://localhost/"
    };
    tauri::Url::parse(&format!("{base}{id}/index.html")).map_err(|_| "Invalid page".into())
}

/// The extension and path under its folder that a URL of the `extras`
/// scheme names; None for any other URL.
fn page(url: &tauri::Url) -> Option<(&'static str, String)> {
    let ours = match url.scheme() {
        "extras" => url.host_str() == Some("localhost"),
        "http" | "https" => url.host_str() == Some("extras.localhost"),
        _ => false,
    };
    if !ours {
        return None;
    }
    let (id, path) = url.path().strip_prefix('/')?.split_once('/')?;
    let (id, _, _) = EXTRAS.iter().find(|(known, _, _)| *known == id)?;
    Some((id, path.to_string()))
}

/// Answers a request of the `extras` scheme off the main thread.
fn serve<R: Runtime>(
    context: UriSchemeContext<'_, R>,
    request: http::Request<Vec<u8>>,
    responder: UriSchemeResponder,
) {
    let app = context.app_handle().clone();
    let webview = context.webview_label().to_string();
    std::thread::spawn(move || {
        let reply = answer(&app, &webview, &request);
        let mut response = http::Response::builder()
            .status(reply.status)
            .header("Content-Type", reply.kind)
            .header("Cache-Control", "no-store");
        for (name, value) in reply.headers {
            response = response.header(name, value);
        }
        responder.respond(response.body(reply.body).unwrap_or_default());
    });
}

fn answer<R: Runtime>(
    app: &AppHandle<R>,
    webview: &str,
    request: &http::Request<Vec<u8>>,
) -> Reply {
    let Some((id, path)) = tauri::Url::parse(&request.uri().to_string())
        .ok()
        .and_then(|url| page(&url))
    else {
        return Reply::error(404, "Not found");
    };
    // Only an extension's own window reads its files and requests.
    if webview != label(id) || request.method() != http::Method::GET {
        return Reply::error(403, "Forbidden");
    }
    let host = app.state::<Host>();
    let Some(extra) = host.made(id) else {
        return Reply::error(404, "Not found");
    };
    let query = Query::parse(request.uri().query());
    match path.as_str() {
        "host/state" => host.state(&query),
        "host/locate" => host.locate(app, &query),
        _ if path.starts_with("api/") => extra.api(&host, &path[4..], &query),
        _ => match shared_file(&path).or_else(|| extra.file(&path)) {
            Some((body, kind)) if kind == HTML => {
                Reply::bytes(kind, with_title(body, &extra.title()))
                    .header("Content-Security-Policy", policy(&extra.image_sources()))
            }
            Some((body, kind)) => Reply::bytes(kind, body.to_vec()),
            None => Reply::error(404, "Not found"),
        },
    }
}

/// A page with `title` in its `<title>`, so it shows from the start.
fn with_title(page: &[u8], title: &str) -> Vec<u8> {
    let find =
        |needle: &[u8], from: usize| memchr::memmem::find(&page[from..], needle).map(|i| from + i);
    let Some(start) = find(b"<title>", 0).map(|i| i + 7) else {
        return page.to_vec();
    };
    let Some(end) = find(b"</title>", start) else {
        return page.to_vec();
    };
    let escaped = title
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    [&page[..start], escaped.as_bytes(), &page[end..]].concat()
}

/// An extension page's content security policy: its own files and
/// requests, plus the image sources it names.
fn policy(images: &[String]) -> String {
    let mut img = String::from("'self' data:");
    for source in images {
        img.push(' ');
        img.push_str(source);
    }
    format!(
        "default-src 'none'; script-src 'self'; style-src 'self'; img-src {img}; \
         connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> tauri::Url {
        tauri::Url::parse(s).unwrap()
    }

    #[test]
    fn pages_name_a_known_extension_and_a_path() {
        assert_eq!(
            page(&url("extras://localhost/track/index.html")),
            Some(("track", "index.html".into()))
        );
        assert_eq!(
            page(&url("http://extras.localhost/track/api/points?from=0")),
            Some(("track", "api/points".into()))
        );
        assert_eq!(
            page(&url("extras://localhost/track/vendor/leaflet/leaflet.js")),
            Some(("track", "vendor/leaflet/leaflet.js".into()))
        );
        for other in [
            "extras://localhost/unknown/index.html",
            "extras://elsewhere/track/index.html",
            "https://tile.openstreetmap.org/track/1/2/3.png",
            "tauri://localhost/track/index.html",
            "extras://localhost/track",
        ] {
            assert_eq!(page(&url(other)), None, "{other}");
        }
    }

    #[test]
    fn queries_read_whole_numbers() {
        let q = Query::parse(Some("dataset=3&from=120&bad=-1&x"));
        assert_eq!(q.number("dataset"), Some(3));
        assert_eq!(q.number("from"), Some(120));
        assert_eq!(q.number("bad"), None);
        assert_eq!(q.number("x"), None);
        assert_eq!(Query::parse(None).number("dataset"), None);
    }

    #[test]
    fn the_policy_adds_only_named_image_sources() {
        let p = policy(&["https://tile.example.org".into()]);
        assert!(p.contains("img-src 'self' data: https://tile.example.org;"));
        assert!(p.contains("script-src 'self';"));
        assert!(p.contains("connect-src 'self';"));
        assert!(!p.contains("unsafe"));
    }

    #[test]
    fn pages_have_their_titles_from_the_start() {
        let page = b"<head><title>x</title></head><title>y</title>";
        assert_eq!(
            with_title(page, "A & <B>"),
            b"<head><title>A &amp; &lt;B&gt;</title></head><title>y</title>".to_vec()
        );
        assert_eq!(
            with_title(b"<p>no title</p>", "A"),
            b"<p>no title</p>".to_vec()
        );
    }

    #[test]
    fn windows_reopen_only_where_a_screen_shows_them() {
        let screens = [[0.0, 25.0, 1512.0, 920.0], [1512.0, 0.0, 1920.0, 1080.0]];
        assert!(visible([100.0, 100.0, 960.0, 680.0], &screens));
        assert!(visible([2000.0, 40.0, 960.0, 680.0], &screens));
        // Off every screen, above one, or with a broken size.
        assert!(!visible([-2000.0, 100.0, 960.0, 680.0], &screens));
        assert!(!visible([100.0, 5.0, 960.0, 680.0], &screens));
        assert!(!visible([100.0, 2000.0, 960.0, 680.0], &screens));
        assert!(!visible([100.0, 100.0, 0.0, 680.0], &screens));
        assert!(!visible([f64::NAN, 100.0, 960.0, 680.0], &screens));
        assert!(!visible([100.0, 100.0, 960.0, 680.0], &[]));
    }

    #[test]
    fn a_page_url_names_its_extension_and_page() {
        let u = page_url("track").unwrap();
        assert_eq!(page(&u), Some(("track", "index.html".into())));
    }
}
