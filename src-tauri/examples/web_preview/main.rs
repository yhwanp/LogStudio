//! Serves the desktop page from `dist/` to an ordinary browser, with the real
//! engine behind it, for UI review and browser tooling. `shim.js` stands in for
//! the Tauri globals: `invoke` posts to /__preview/invoke and `listen` reads
//! the /__preview/events stream. Development only: not part of the app or its
//! packages, and it listens on 127.0.0.1 only.
//!
//!   cargo run -p logstudio-desktop --example web_preview -- [--port N] [--no-csp] [--trace] [FILE...]
//!
//! There is no native picker: `로그 파일 열기` opens the FILEs given here, as one
//! combined log when there are several. Saved filters, recent files and saved
//! work use the desktop's own stores in a temporary folder, never the app's
//! data. The page gets the app's CSP (with the preview server as the only
//! connection); `--no-csp` drops it so tools can inject scripts. As the desktop
//! has one window, the page loaded last owns the engine and earlier tabs are
//! refused. Not covered: drag and drop, clipboard file paste, export, and
//! WKWebView rendering.
#[allow(dead_code)]
#[path = "../../src/filters.rs"]
mod filters;
#[allow(dead_code)]
#[path = "../../src/recent.rs"]
mod recent;
#[allow(dead_code)]
#[path = "../../src/work.rs"]
mod work;

use logstudio_core::Engine;
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    io::{self, BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
};

const SHIM: &str = include_str!("shim.js");
const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; object-src 'none'; base-uri 'none'; frame-src 'none'";
/// Largest request body; pasted text arrives in one.
const BODY_BYTES: usize = 256 << 20;

struct App {
    engine: Arc<Engine>,
    filters: filters::SavedFilters,
    recent: recent::Recent,
    works: work::WorkStates,
    /// What the open button opens in place of a native picker.
    picks: Vec<PathBuf>,
    dist: PathBuf,
    csp: bool,
    /// Event streams, by the page that opened each.
    clients: Mutex<Vec<(String, mpsc::Sender<String>)>>,
    pages: Mutex<Pages>,
    active: AtomicUsize,
    /// Prints each command and its outcome to stderr.
    trace: bool,
}

/// The page that owns the engine, and the pages that owned it before.
#[derive(Default)]
struct Pages {
    current: String,
    retired: HashSet<String>,
}

/// One running engine request, released on drop; the desktop allows four.
struct Permit<'a>(&'a AtomicUsize);
impl Drop for Permit<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

fn message(error: io::Error) -> String {
    logstudio_core::source::public_message(&error, "Request failed")
}
fn invalid(key: &str) -> String {
    format!("invalid args `{key}`")
}
fn u64_arg(args: &Value, key: &str) -> Result<u64, String> {
    args.get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid(key))
}
fn opt_u64_arg(args: &Value, key: &str) -> Result<Option<u64>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_u64().map(Some).ok_or_else(|| invalid(key)),
    }
}
fn str_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(key))
}

impl App {
    /// Whether `page` may use the engine; a page not seen before takes it over.
    fn claim(&self, page: &str) -> bool {
        let mut pages = self.pages.lock().unwrap();
        if page == pages.current {
            return true;
        }
        if page.is_empty() || pages.retired.contains(page) {
            return false;
        }
        let old = std::mem::replace(&mut pages.current, page.to_string());
        pages.retired.insert(old);
        true
    }
    /// Sends to the current page; streams of earlier pages close.
    fn emit(&self, channel: &str, payload: &Value) {
        let text = format!("event: {channel}\ndata: {payload}\n\n");
        let current = self.pages.lock().unwrap().current.clone();
        self.clients
            .lock()
            .unwrap()
            .retain(|(page, client)| *page == current && client.send(text.clone()).is_ok());
    }
    /// The desktop's engine observer: saved work rides on `opened`, and
    /// validation settles the recent list.
    fn engine_event(&self, mut event: Value) {
        if let (Some("opened"), Some(id)) = (event["status"].as_str(), event["datasetId"].as_u64())
        {
            event["work"] = self.work_info(id);
        }
        let channel = if event["status"] == "query-progress" {
            "query-progress"
        } else {
            "source-changed"
        };
        if let (Some(status @ ("validated" | "rollback")), Some(id)) =
            (event["status"].as_str(), event["datasetId"].as_u64())
        {
            if self.recent.settle(id, status == "validated") {
                self.emit("recent-changed", &self.recent.list());
            }
        }
        self.emit(channel, &event);
    }
    fn work_info(&self, id: u64) -> Value {
        match self.engine.grants(id) {
            Some(grants) => self.works.lookup(id, work::stamps(grants)),
            None => json!({"reopenable":false}),
        }
    }
    fn permit(&self) -> Result<Permit<'_>, String> {
        if self.active.fetch_add(1, Ordering::AcqRel) >= 4 {
            self.active.fetch_sub(1, Ordering::AcqRel);
            return Err("Local engine is busy; retry".into());
        }
        Ok(Permit(&self.active))
    }
    fn open_arranged(&self, id: u64, paths: Vec<PathBuf>) -> io::Result<Value> {
        let (paths, by) = logstudio_core::order::arrange(paths);
        if let Some(by) = by {
            let note = json!({"datasetId":id,"count":paths.len(),"by":by.as_str()});
            self.emit("files-ordered", &note);
        }
        self.recent.expect(id, &paths);
        self.engine.open_files(id, &paths)
    }
    /// The desktop commands the page invokes, with the same arguments and results.
    fn invoke(&self, cmd: &str, args: &Value) -> Result<Value, String> {
        let engine = &self.engine;
        match cmd {
            "log_request" => {
                let request = args
                    .get("request")
                    .cloned()
                    .ok_or_else(|| invalid("request"))?;
                let _permit = match request["action"].as_str() {
                    Some("cancel" | "close") => None,
                    _ => Some(self.permit()?),
                };
                engine.request(request).map_err(message)
            }
            "pick_file" => {
                let id = engine.begin_open();
                if self.picks.is_empty() {
                    engine.cancel_open(id);
                    return Ok(json!({"cancelled":true}));
                }
                self.open_arranged(id, self.picks.clone()).map_err(message)
            }
            "open_text" => {
                let (text, name) = (str_arg(args, "text")?, str_arg(args, "name")?);
                let id = engine.begin_open();
                engine
                    .open_text(id, text, name)
                    .map_err(|e| logstudio_core::source::public_message(&e, "Paste failed"))
            }
            "cancel_open" => {
                match opt_u64_arg(args, "datasetId")? {
                    Some(id) => engine.cancel_open(id),
                    None => engine.cancel_pending(),
                }
                Ok(Value::Null)
            }
            "reopen_source" => {
                let from = u64_arg(args, "datasetId")?;
                let grants = engine
                    .grants(from)
                    .ok_or("Nothing to reopen; select the file again")?;
                let paths: Vec<_> = grants.into_iter().map(|g| g.path).collect();
                let id = engine.begin_open();
                self.works.reloading(id, from);
                self.recent.expect(id, &paths);
                engine.reopen(id, from).map_err(message)
            }
            "recent_files" => Ok(self.recent.list()),
            "open_recent" => {
                let id = u64_arg(args, "id")?;
                let paths = self
                    .recent
                    .paths(id)
                    .ok_or("Recent file is no longer listed")?;
                let source = engine.begin_open();
                self.recent.expect(source, &paths);
                let result = engine.open_files(source, &paths);
                if result
                    .as_ref()
                    .is_err_and(|e| e.kind() == io::ErrorKind::NotFound)
                {
                    self.recent.forget(Some(id));
                    self.emit("recent-changed", &self.recent.list());
                }
                result.map_err(message)
            }
            "forget_recent" => {
                let id = opt_u64_arg(args, "id")?;
                match id {
                    Some(id) => {
                        if let Some(paths) = self.recent.paths(id) {
                            self.works.forget(Some(&paths));
                        }
                    }
                    None => self.works.forget(None),
                }
                self.recent.forget(id);
                Ok(self.recent.list())
            }
            "work_state" => Ok(self.work_info(u64_arg(args, "datasetId")?)),
            "save_work_state" => {
                let state = args.get("state").cloned().ok_or_else(|| invalid("state"))?;
                self.works
                    .save(u64_arg(args, "datasetId")?, state)
                    .map(Value::Bool)
            }
            "saved_filters" => Ok(self.filters.list()),
            "save_filter" => {
                let filter = args
                    .get("filter")
                    .cloned()
                    .ok_or_else(|| invalid("filter"))?;
                self.filters.save(
                    opt_u64_arg(args, "id")?,
                    str_arg(args, "name")?,
                    str_arg(args, "format")?,
                    filter,
                )
            }
            "rename_filter" => self
                .filters
                .rename(u64_arg(args, "id")?, str_arg(args, "name")?),
            "delete_filter" => self.filters.delete(u64_arg(args, "id")?),
            "take_startup_error" => Ok(Value::Null),
            "export_results"
            | "cancel_export"
            | "hold_file_paste"
            | "finish_file_paste"
            | "finish_prefix_paste" => Err("Not available in the web preview".into()),
            _ => Err(format!("Unknown command {cmd}")),
        }
    }
}

fn respond(
    stream: &mut TcpStream,
    status: &str,
    kind: &str,
    extra: &str,
    body: &[u8],
) -> io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n{extra}\r\n",
        body.len()
    )?;
    stream.write_all(body)
}

fn content_type(name: &str) -> &'static str {
    match name.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("json") => "application/json",
        _ => "application/octet-stream",
    }
}

/// A file directly in `dist/`; the page has no folders.
fn static_file(app: &App, stream: &mut TcpStream, path: &str) -> io::Result<()> {
    let name = match path.trim_start_matches('/') {
        "" => "index.html",
        name => name,
    };
    let plain = !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c));
    let body = match plain.then(|| std::fs::read(app.dist.join(name))) {
        Some(Ok(body)) => body,
        _ => return respond(stream, "404 Not Found", "text/plain", "", b"Not found"),
    };
    if name != "index.html" {
        return respond(stream, "200 OK", content_type(name), "", &body);
    }
    // The shim must define the globals before the page's module runs.
    let html = String::from_utf8_lossy(&body);
    let Some(at) = html.find("<script type=\"module\"") else {
        let text = b"index.html has no module script for the shim to precede";
        return respond(stream, "500 Internal Server Error", "text/plain", "", text);
    };
    let html = format!(
        "{}<script src=\"./__preview/shim.js\"></script>{}",
        &html[..at],
        &html[at..]
    );
    let csp = if app.csp {
        format!("Content-Security-Policy: {CSP}\r\n")
    } else {
        String::new()
    };
    respond(stream, "200 OK", content_type(name), &csp, html.as_bytes())
}

fn handle(app: &App, mut stream: TcpStream) -> io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or("/"));
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let (path, query) = (path.to_string(), query.to_string());
    let method = method.to_string();
    let mut length = 0usize;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 || header.trim_end().is_empty() {
            break;
        }
        if let Some((key, value)) = header.split_once(':') {
            if key.eq_ignore_ascii_case("content-length") {
                length = value.trim().parse().unwrap_or(0);
            }
        }
    }
    match (method.as_str(), path.as_str()) {
        ("POST", "/__preview/invoke") => {
            if length > BODY_BYTES {
                return respond(
                    &mut stream,
                    "413 Payload Too Large",
                    "text/plain",
                    "",
                    b"Too large",
                );
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body)?;
            let call: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            let page = call["page"].as_str().unwrap_or("");
            let reply = match call["cmd"].as_str() {
                _ if !app.claim(page) => {
                    json!({"error":"Another preview tab is in use; reload this one"})
                }
                Some(cmd) => match app.invoke(cmd, &call["args"]) {
                    Ok(value) => json!({"ok":value}),
                    Err(error) => json!({"error":error}),
                },
                None => json!({"error":"Invalid call"}),
            };
            if app.trace {
                let request = &call["args"]["request"];
                eprintln!(
                    "{} {} {} query={} -> {}",
                    call["cmd"],
                    request["action"],
                    request["pane"],
                    request["queryId"],
                    reply.get("error").unwrap_or(&json!("ok"))
                );
            }
            respond(
                &mut stream,
                "200 OK",
                "application/json",
                "",
                reply.to_string().as_bytes(),
            )
        }
        ("GET", "/__preview/events") => {
            let page = query.strip_prefix("page=").unwrap_or("");
            if !app.claim(page) {
                return respond(&mut stream, "409 Conflict", "text/plain", "", b"Superseded");
            }
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\n\r\nretry: 1000\n\n")?;
            let (tx, rx) = mpsc::channel();
            app.clients.lock().unwrap().push((page.to_string(), tx));
            for text in rx {
                stream.write_all(text.as_bytes())?;
            }
            Ok(())
        }
        ("GET", "/__preview/shim.js") => respond(
            &mut stream,
            "200 OK",
            content_type("shim.js"),
            "",
            SHIM.as_bytes(),
        ),
        ("GET", _) => static_file(app, &mut stream, &path),
        _ => respond(
            &mut stream,
            "405 Method Not Allowed",
            "text/plain",
            "",
            b"Method not allowed",
        ),
    }
}

fn main() -> io::Result<()> {
    let (mut port, mut csp, mut trace, mut picks) = (8790u16, true, false, vec![]);
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--port" => {
                port = args
                    .next()
                    .and_then(|p| p.parse().ok())
                    .ok_or_else(|| io::Error::other("--port needs a number"))?
            }
            "--no-csp" => csp = false,
            "--trace" => trace = true,
            _ => picks.push(PathBuf::from(arg)),
        }
    }
    // Kept until exit; an interrupted run leaves the folder to the OS.
    let data = tempfile::Builder::new()
        .prefix("logstudio-web-preview")
        .tempdir()?;
    let app = Arc::new(App {
        engine: Arc::new(Engine::default()),
        filters: filters::SavedFilters::default(),
        recent: recent::Recent::default(),
        works: work::WorkStates::default(),
        picks,
        dist: Path::new(env!("CARGO_MANIFEST_DIR")).join("../dist"),
        csp,
        clients: Mutex::new(vec![]),
        pages: Mutex::new(Pages::default()),
        active: AtomicUsize::new(0),
        trace,
    });
    app.recent.load(data.path());
    app.works.load(data.path());
    app.filters.load(data.path());
    let observer = Arc::clone(&app);
    app.engine
        .observe(move |event| observer.engine_event(event));
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    println!("LogStudio web preview: http://127.0.0.1:{port}/");
    println!("App data (temporary): {}", data.path().display());
    for stream in listener.incoming().flatten() {
        let app = Arc::clone(&app);
        thread::spawn(move || {
            let _ = handle(&app, stream);
        });
    }
    Ok(())
}
