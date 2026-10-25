//! Track: draws on a map the coordinates that rows of the shown log carry,
//! and links the map's points with their rows.
//!
//! A row holds a point when it contains the settings' `marker` followed by
//! a JSON object whose `latitude` and `longitude` paths are numbers; the
//! numbers times `scale` are degrees. The points are read in the background
//! as the page asks for them, block by block in row order, so the page
//! draws them while the log is still being read, and a followed log's new
//! rows add points as they arrive.
use crate::{Extra, File, Host, Query, Reply, CSS, HTML, JS};
use logstudio_core::{dataset::Dataset, source::ByteLines, Engine};
use memchr::memmem::Finder;
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

pub const ID: &str = "track";
/// Its title unless the settings give one.
pub const NAME: &str = "좌표 궤적";
/// Points kept for one log, 48 MiB: a map of more would not stay usable.
pub const MAX_POINTS: usize = 2_000_000;
/// OpenStreetMap's standard tiles, used unless the settings name others.
const OSM_TILES: &str = "https://tile.openstreetmap.org/{z}/{x}/{y}.png";
const OSM_ATTRIBUTION: &str = "© OpenStreetMap contributors";
/// How often a followed log's new rows are read.
const FOLLOW_POLL: Duration = Duration::from_millis(500);

/// What a settings file `track.json` says.
#[derive(Debug, PartialEq)]
pub struct Settings {
    pub title: String,
    pub marker: Vec<u8>,
    pub latitude: Vec<String>,
    pub longitude: Vec<String>,
    pub scale: f64,
    pub tiles: String,
    pub attribution: String,
}
impl Settings {
    pub fn from_json(value: &Value) -> Result<Self, String> {
        let object = value
            .as_object()
            .ok_or("설정 파일은 JSON 객체({ … })여야 합니다.")?;
        const KEYS: [&str; 7] = [
            "title",
            "marker",
            "latitude",
            "longitude",
            "scale",
            "tiles",
            "attribution",
        ];
        if let Some(key) = object.keys().find(|k| !KEYS.contains(&k.as_str())) {
            return Err(format!("알 수 없는 항목입니다 ({}).", clip(key)));
        }
        let text = |key: &str, max: usize| -> Result<Option<String>, String> {
            match object.get(key) {
                None => Ok(None),
                Some(Value::String(s)) if s.chars().count() <= max => Ok(Some(s.clone())),
                Some(Value::String(_)) => Err(format!("{key} 값은 {max}자까지입니다.")),
                Some(_) => Err(format!("{key} 값은 문자열이어야 합니다.")),
            }
        };
        let marker = text("marker", 200)?
            .filter(|m| !m.is_empty())
            .ok_or("marker(좌표가 있는 행을 찾을 문자열)를 적어 주세요.")?;
        let path = |key: &str| -> Result<Vec<String>, String> {
            let path = text(key, 200)?.ok_or(format!(
                "{key}(JSON 안에서 숫자가 있는 경로)를 적어 주세요."
            ))?;
            let parts: Vec<String> = path.split('.').map(str::to_string).collect();
            if parts.iter().any(String::is_empty) || parts.len() > 16 {
                return Err(format!(
                    "{key} 값은 점(.)으로 나눈 경로여야 합니다 (예: position.lat)."
                ));
            }
            Ok(parts)
        };
        let scale = match object.get("scale") {
            None => 1.0,
            Some(v) => v
                .as_f64()
                .filter(|s| s.is_finite() && *s > 0.0)
                .ok_or("scale 값은 0보다 큰 수여야 합니다.")?,
        };
        let tiles = text("tiles", 300)?;
        let attribution = text("attribution", 200)?;
        let (tiles, attribution) = match tiles {
            None => (
                OSM_TILES.to_string(),
                attribution.unwrap_or_else(|| OSM_ATTRIBUTION.into()),
            ),
            Some(t) => {
                tile_origin(&t)?;
                (t, attribution.unwrap_or_default())
            }
        };
        Ok(Self {
            title: text("title", 40)?
                .filter(|t| !t.trim().is_empty())
                .unwrap_or_else(|| NAME.into()),
            marker: marker.into_bytes(),
            latitude: path("latitude")?,
            longitude: path("longitude")?,
            scale,
            tiles,
            attribution,
        })
    }
    /// The coordinates in the text after a row's marker: the first JSON
    /// object there, read to its end. None when there is none, a path is not
    /// a number, or the degrees are out of range or 0, 0.
    pub fn point(&self, after: &[u8]) -> Option<(f64, f64)> {
        let start = memchr::memchr(b'{', after)?;
        let value = serde_json::Deserializer::from_slice(&after[start..])
            .into_iter::<Value>()
            .next()?
            .ok()?;
        let number = |path: &[String]| {
            path.iter()
                .try_fold(&value, |v, key| match v {
                    Value::Array(a) => a.get(key.parse::<usize>().ok()?),
                    _ => v.get(key),
                })?
                .as_f64()
        };
        let latitude = number(&self.latitude)? * self.scale;
        let longitude = number(&self.longitude)? * self.scale;
        // 0, 0 is what a position without a fix reads as.
        let place = latitude.abs() <= 90.0
            && longitude.abs() <= 180.0
            && !(latitude == 0.0 && longitude == 0.0);
        place.then_some((latitude, longitude))
    }
}

fn clip(s: &str) -> String {
    s.chars().take(40).collect()
}

/// The origin of a tile URL template, which the page may load images from.
/// Templates are https URLs, or http ones of a tile server on this
/// computer, with `{z}`, `{x}` and `{y}` after the host.
pub fn tile_origin(template: &str) -> Result<String, String> {
    const BAD: &str = "tiles 값은 {z}, {x}, {y}가 들어간 https 주소여야 합니다. \
        이 컴퓨터의 타일 서버라면 http://localhost나 http://127.0.0.1 주소도 됩니다.";
    let (scheme, rest) = template.split_once("://").ok_or(BAD)?;
    let host = rest.split('/').next().unwrap_or("");
    let name = host.rsplit_once(':').map_or(host, |(name, port)| {
        if port.bytes().all(|b| b.is_ascii_digit()) {
            name
        } else {
            host
        }
    });
    let local = matches!(name, "localhost" | "127.0.0.1");
    let good_host = !name.is_empty()
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b':'));
    let has = |p: &str| rest[host.len()..].contains(p);
    if !(scheme == "https" || scheme == "http" && local)
        || !good_host
        || !has("{z}")
        || !has("{x}")
        || !has("{y}")
    {
        return Err(BAD.into());
    }
    Ok(format!("{scheme}://{host}"))
}

/// A row's point: its physical row number and degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub line: u64,
    pub latitude: f64,
    pub longitude: f64,
}

/// The points of a block's rows, whose first is row `first`, and how many
/// rows with the marker hold none.
pub fn points(text: &[u8], first: u64, settings: &Settings, marker: &Finder) -> (Vec<Point>, u32) {
    let (mut found, mut skipped) = (vec![], 0);
    if marker.find(text).is_none() {
        return (found, skipped);
    }
    for (j, (_, row)) in ByteLines::new(text).enumerate() {
        let Some(at) = marker.find(row) else { continue };
        match settings.point(&row[at + marker.needle().len()..]) {
            Some((latitude, longitude)) => found.push(Point {
                line: first + j as u64,
                latitude,
                longitude,
            }),
            None => skipped += 1,
        }
    }
    (found, skipped)
}

/// Where reading a log's points stands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Reading,
    /// All rows so far are read; a followed log may add more.
    Following,
    Complete,
    /// `MAX_POINTS` were found; later rows are not read.
    Full,
    Failed,
    Stopped,
}
impl Phase {
    fn as_str(self) -> &'static str {
        match self {
            Self::Reading => "reading",
            Self::Following => "following",
            Self::Complete => "complete",
            Self::Full => "full",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
        }
    }
}

/// The points read so far, by block.
#[derive(Default)]
pub struct Found {
    pub points: Vec<Point>,
    /// The first point of each block read.
    starts: Vec<usize>,
    /// The last row of each block read.
    ends: Vec<u64>,
    skipped: Vec<u32>,
    pub phase: Phase,
    error: String,
}
impl Found {
    fn add(&mut self, block: usize, points: Vec<Point>, skipped: u32, last: u64) {
        debug_assert_eq!(block, self.starts.len());
        self.starts.push(self.points.len());
        self.ends.push(last);
        self.skipped.push(skipped);
        let room = MAX_POINTS - self.points.len();
        if points.len() >= room {
            self.points.extend_from_slice(&points[..room]);
            self.phase = Phase::Full;
        } else {
            self.points.extend(points);
        }
    }
    /// Forgets the points of blocks `block..`, which are read again.
    fn forget(&mut self, block: usize) {
        if let Some(&start) = self.starts.get(block) {
            self.points.truncate(start);
            self.starts.truncate(block);
            self.ends.truncate(block);
            self.skipped.truncate(block);
        }
    }
    /// Points that stay as they are: a followed log's last block may be
    /// read again.
    fn stable(&self) -> usize {
        match self.phase {
            Phase::Reading | Phase::Following => self.starts.last().copied().unwrap_or(0),
            _ => self.points.len(),
        }
    }
    /// The last row read, after which a selected row has no point yet.
    fn through(&self) -> u64 {
        self.ends.last().copied().unwrap_or(0)
    }
    pub fn json(&self, dataset: u64) -> Value {
        json!({
            "datasetId":dataset,
            "phase":self.phase.as_str(),
            "count":self.points.len(),
            "stable":self.stable(),
            "through":self.through(),
            "skipped":self.skipped.iter().map(|&s| u64::from(s)).sum::<u64>(),
            "error":self.error,
        })
    }
}

/// Reading one dataset's points.
pub struct Job {
    pub dataset: u64,
    stop: AtomicBool,
    pub found: Mutex<Found>,
}
impl Job {
    fn end(&self, phase: Phase, error: String) {
        let mut found = self.found.lock().unwrap();
        found.phase = phase;
        found.error = error;
    }
    fn halted(&self, engine: &Engine, d: &Dataset) -> bool {
        self.stop.load(Ordering::Acquire)
            || d.stop.load(Ordering::Acquire)
            || engine.shown().map(|s| s.id) != Some(d.id)
    }
    /// Reads `d`'s points until its rows end, or, while it is followed,
    /// until another log is shown or the job stops.
    pub fn run(&self, engine: &Engine, d: &Dataset, settings: &Settings) {
        let marker = Finder::new(&settings.marker);
        let mut from = 0;
        loop {
            let read = d.read_blocks(
                from,
                &self.stop,
                |b, rows| {
                    let (points, skipped) = points(&rows.text, b.first, settings, &marker);
                    (points, skipped, b.first + (b.rows as u64).saturating_sub(1))
                },
                |i, (points, skipped, last)| {
                    let mut found = self.found.lock().unwrap();
                    found.add(i, points, skipped, last);
                    // Later blocks would find points that cannot be kept.
                    if found.phase == Phase::Full {
                        self.stop.store(true, Ordering::Release);
                    }
                },
            );
            match read {
                Ok(next) => from = next,
                // The core's reasons are English; the page shows Korean.
                Err(_) => {
                    let message = "로그를 읽지 못했습니다. 메인 창에서 로그를 다시 열어 주세요.";
                    return self.end(Phase::Failed, message.into());
                }
            }
            if self.found.lock().unwrap().phase == Phase::Full {
                return;
            }
            if self.halted(engine, d) {
                return self.end(Phase::Stopped, String::new());
            }
            let (done, live, blocks) = {
                let index = d.index.lock().unwrap();
                (index.done, index.live, index.blocks.len())
            };
            if done && !live && from >= blocks {
                return self.end(Phase::Complete, String::new());
            }
            self.found.lock().unwrap().phase = Phase::Following;
            std::thread::sleep(FOLLOW_POLL);
            if self.halted(engine, d) {
                return self.end(Phase::Stopped, String::new());
            }
            // A followed log's last block grows: it is replaced as rows
            // complete, so it is read again with the blocks after it.
            from = from.saturating_sub(1);
            self.found.lock().unwrap().forget(from);
        }
    }
}

struct Track {
    settings: Arc<Settings>,
    job: Mutex<Option<Arc<Job>>>,
}

pub fn make(value: &Value) -> Result<Arc<dyn Extra>, String> {
    Ok(Arc::new(Track {
        settings: Arc::new(Settings::from_json(value)?),
        job: Mutex::new(None),
    }))
}

impl Track {
    /// The job reading the shown dataset `id`'s points, started on first
    /// request; it replaces the job of a log shown before.
    fn job(&self, host: &Host, id: u64) -> Option<Arc<Job>> {
        let mut slot = self.job.lock().unwrap();
        // A log shown again after another failed to open is read again.
        let current =
            |j: &&Arc<Job>| j.dataset == id && j.found.lock().unwrap().phase != Phase::Stopped;
        if let Some(job) = slot.as_ref().filter(current) {
            return Some(job.clone());
        }
        let d = host.engine().shown().filter(|d| d.id == id)?;
        if let Some(old) = slot.take() {
            old.stop.store(true, Ordering::Release);
        }
        let job = Arc::new(Job {
            dataset: id,
            stop: AtomicBool::new(false),
            found: Mutex::new(Found::default()),
        });
        *slot = Some(job.clone());
        let (running, engine, settings) =
            (job.clone(), host.engine().clone(), self.settings.clone());
        std::thread::spawn(move || running.run(&engine, &d, &settings));
        Some(job)
    }
}

impl Extra for Track {
    fn title(&self) -> String {
        self.settings.title.clone()
    }
    fn image_sources(&self) -> Vec<String> {
        tile_origin(&self.settings.tiles).into_iter().collect()
    }
    fn file(&self, path: &str) -> Option<File> {
        Some(match path {
            "index.html" => (include_bytes!("../web/track/index.html"), HTML),
            "track.js" => (include_bytes!("../web/track/track.js"), JS),
            "track-model.js" => (include_bytes!("../web/track/track-model.js"), JS),
            "track.css" => (include_bytes!("../web/track/track.css"), CSS),
            _ => return None,
        })
    }
    fn api(&self, host: &Host, path: &str, query: &Query) -> Reply {
        match path {
            "settings" => Reply::json(json!({
                "title":self.settings.title,
                "tiles":self.settings.tiles,
                "attribution":self.settings.attribution,
                "marker":String::from_utf8_lossy(&self.settings.marker),
            })),
            // Where reading the points of dataset `dataset` stands.
            "status" => {
                let Some(id) = query.number("dataset") else {
                    return Reply::error(400, "잘못된 요청입니다.");
                };
                match self.job(host, id) {
                    Some(job) => Reply::json(job.found.lock().unwrap().json(id)),
                    None => Reply::error(409, "메인 창의 로그가 바뀌었습니다."),
                }
            }
            // Points `from..` as little-endian f64 triples of row,
            // latitude and longitude, with how many points there are and
            // how many of them stay (`Found::stable`).
            "points" => {
                let (Some(id), Some(from)) = (query.number("dataset"), query.number("from")) else {
                    return Reply::error(400, "잘못된 요청입니다.");
                };
                let job = self.job.lock().unwrap().clone();
                let Some(job) = job.filter(|j| j.dataset == id) else {
                    return Reply::error(409, "메인 창의 로그가 바뀌었습니다.");
                };
                let found = job.found.lock().unwrap();
                let from = (from as usize).min(found.points.len());
                let mut body = Vec::with_capacity((found.points.len() - from) * 24);
                for p in &found.points[from..] {
                    body.extend_from_slice(&(p.line as f64).to_le_bytes());
                    body.extend_from_slice(&p.latitude.to_le_bytes());
                    body.extend_from_slice(&p.longitude.to_le_bytes());
                }
                let (count, stable) = (found.points.len(), found.stable());
                drop(found);
                Reply::bytes("application/octet-stream", body)
                    .header("X-Count", count.to_string())
                    .header("X-Stable", stable.to_string())
            }
            _ => Reply::error(404, "Not found"),
        }
    }
    fn closed(&self) {
        if let Some(job) = self.job.lock().unwrap().take() {
            job.stop.store(true, Ordering::Release);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> Settings {
        Settings::from_json(&json!({
            "marker":"onPositionChanged",
            "latitude":"fix.lat",
            "longitude":"fix.lon",
            "scale":1e-7
        }))
        .unwrap()
    }

    #[test]
    fn settings_take_defaults_and_reject_mistakes() {
        let s = settings();
        assert_eq!(s.title, "좌표 궤적");
        assert_eq!(s.tiles, OSM_TILES);
        assert_eq!(s.attribution, OSM_ATTRIBUTION);
        assert_eq!(s.latitude, ["fix", "lat"]);
        for (bad, why) in [
            (json!([]), "객체"),
            (json!({"latitude":"a","longitude":"b"}), "marker"),
            (json!({"marker":"m","longitude":"b"}), "latitude"),
            (
                json!({"marker":"m","latitude":"a..b","longitude":"b"}),
                "경로",
            ),
            (
                json!({"marker":"m","latitude":"a","longitude":"b","scale":0}),
                "scale",
            ),
            (
                json!({"marker":"m","latitude":"a","longitude":"b","colour":"red"}),
                "colour",
            ),
            (
                json!({"marker":"m","latitude":"a","longitude":"b","tiles":"http://t.example/{z}/{x}/{y}.png"}),
                "https",
            ),
            (
                json!({"marker":"m","latitude":"a","longitude":"b","tiles":"https://t.example/tiles.png"}),
                "https",
            ),
            (json!({"marker":7,"latitude":"a","longitude":"b"}), "문자열"),
        ] {
            let error = Settings::from_json(&bad).unwrap_err();
            assert!(error.contains(why), "{bad}: {error}");
        }
        // Other tiles carry no OpenStreetMap credit unless given one.
        let other = Settings::from_json(&json!({"marker":"m","latitude":"a","longitude":"b","tiles":"https://tiles.example.org:8443/{z}/{x}/{y}.png"})).unwrap();
        assert_eq!(other.attribution, "");
        assert_eq!(
            tile_origin(&other.tiles).unwrap(),
            "https://tiles.example.org:8443"
        );
        assert!(tile_origin("https://{s}.example.org/{z}/{x}/{y}.png").is_err());
        // Plain http only reaches a tile server on this computer.
        assert_eq!(
            tile_origin("http://127.0.0.1:8080/{z}/{x}/{y}.png").unwrap(),
            "http://127.0.0.1:8080"
        );
        assert_eq!(
            tile_origin("http://localhost/t/{z}/{x}/{y}").unwrap(),
            "http://localhost"
        );
        assert!(tile_origin("http://tiles.example.org/{z}/{x}/{y}.png").is_err());
        assert!(tile_origin("http://localhost.example.org/{z}/{x}/{y}.png").is_err());
        assert!(tile_origin("ftp://localhost/{z}/{x}/{y}.png").is_err());
    }

    #[test]
    fn a_point_is_the_first_object_after_the_marker() {
        let s = settings();
        let row = br#" {"fix":{"lat":374512345,"lon":1270123456,"alt":0},"t":5} trailing"#;
        let (lat, lon) = s.point(row).unwrap();
        assert!((lat - 37.4512345).abs() < 1e-9 && (lon - 127.0123456).abs() < 1e-9);
        // Arrays are indexed by number.
        let arrays =
            Settings::from_json(&json!({"marker":"m","latitude":"p.0","longitude":"p.1"})).unwrap();
        assert_eq!(arrays.point(br#"{"p":[48.5,-2.25]}"#), Some((48.5, -2.25)));
        for none in [
            &br#" no object"#[..],
            br#" {"fix":{"lat":"374512345","lon":1270123456}}"#,
            br#" {"fix":{"lat":374512345}}"#,
            br#" {"fix":{"lat":0,"lon":0}}"#,
            br#" {"fix":{"lat":950000000,"lon":0}}"#,
            br#" {"fix":{"lat":1,"lon":"#,
        ] {
            assert_eq!(s.point(none), None, "{}", String::from_utf8_lossy(none));
        }
    }

    #[test]
    fn points_keep_physical_rows_across_line_endings() {
        let s = settings();
        let marker = Finder::new(&s.marker);
        let text = b"10-25 09:00:00.000  100  101 D Nav: start\r\n\
10-25 09:00:00.100  100  101 D Nav: onPositionChanged {\"fix\":{\"lat\":374512345,\"lon\":1270123456}}\r\
10-25 09:00:00.200  100  101 D Nav: onPositionChanged {\"fix\":{\"lat\":\"bad\"}}\n\
10-25 09:00:00.300  100  101 D Nav: onPositionChanged {\"fix\":{\"lat\":374512400,\"lon\":1270123500}}";
        let (found, skipped) = points(text, 41, &s, &marker);
        assert_eq!(skipped, 1);
        assert_eq!(found.iter().map(|p| p.line).collect::<Vec<_>>(), [42, 44]);
        assert_eq!(points(b"no marker here\n", 1, &s, &marker), (vec![], 0));
    }

    fn wait(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while !done() {
            assert!(std::time::Instant::now() < deadline, "timed out: {what}");
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn position(i: u64) -> String {
        format!(
            "10-25 09:00:00.000  100  101 D Nav: onPositionChanged {{\"fix\":{{\"lat\":{},\"lon\":{}}}}}\n",
            374_000_000 + i,
            1_270_000_000 + i
        )
    }
    /// Opens `text` in an engine, as the main window shows a log.
    fn shown(text: &str) -> (tempfile::TempDir, Arc<Engine>, Arc<Dataset>) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nav.log");
        std::fs::write(&path, text).unwrap();
        let engine = Arc::new(Engine::default());
        let id = engine.begin_open();
        engine.open_file(id, &path).unwrap();
        let d = engine.shown().unwrap();
        (dir, engine, d)
    }
    fn job(d: &Dataset) -> Arc<Job> {
        Arc::new(Job {
            dataset: d.id,
            stop: AtomicBool::new(false),
            found: Mutex::new(Found::default()),
        })
    }
    fn start(
        job: &Arc<Job>,
        engine: &Arc<Engine>,
        d: &Arc<Dataset>,
    ) -> std::thread::JoinHandle<()> {
        let (job, engine, d) = (job.clone(), engine.clone(), d.clone());
        std::thread::spawn(move || job.run(&engine, &d, &settings()))
    }

    #[test]
    fn a_job_reads_every_block_of_the_shown_log() {
        // Several blocks of filler with a position every tenth row.
        let mut text = String::new();
        for i in 0..30_000u64 {
            if i % 10 == 0 {
                text += &position(i);
            } else {
                text += "10-25 09:00:00.000  100  102 I App: synthetic filler row of text\n";
            }
        }
        let (_dir, engine, d) = shown(&text);
        let job = job(&d);
        start(&job, &engine, &d).join().unwrap();
        let found = job.found.lock().unwrap();
        assert_eq!(found.phase, Phase::Complete, "{}", found.error);
        assert_eq!(found.points.len(), 3_000);
        assert!(found
            .points
            .iter()
            .enumerate()
            .all(|(k, p)| p.line == 10 * k as u64 + 1));
        assert_eq!(found.json(d.id)["stable"], 3_000);
    }

    #[test]
    fn a_followed_log_adds_points_and_another_log_stops_the_job() {
        let (dir, engine, d) = shown(&(0..5).map(position).collect::<String>());
        wait("index", || d.index.lock().unwrap().done);
        d.follow_poll_ms.store(2, Ordering::Relaxed);
        d.follow(true).unwrap();
        let job = job(&d);
        let running = start(&job, &engine, &d);
        wait("following", || {
            let f = job.found.lock().unwrap();
            f.phase == Phase::Following && f.points.len() == 5
        });
        // Rows written meanwhile, the last one completed later.
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(dir.path().join("nav.log"))
            .unwrap();
        use std::io::Write;
        let late = position(7);
        file.write_all((position(5) + &position(6) + &late[..20]).as_bytes())
            .unwrap();
        file.flush().unwrap();
        wait("two more", || job.found.lock().unwrap().points.len() == 7);
        file.write_all(&late.as_bytes()[20..]).unwrap();
        wait("the completed row", || {
            job.found.lock().unwrap().points.len() == 8
        });
        {
            let f = job.found.lock().unwrap();
            assert!(
                f.stable() < 8,
                "a followed log's last block may be read again"
            );
            let lines: Vec<u64> = f.points.iter().map(|p| p.line).collect();
            assert_eq!(lines, (1..=8).collect::<Vec<_>>());
        }
        // Showing another log ends the job.
        let other = dir.path().join("other.log");
        std::fs::write(&other, position(9)).unwrap();
        let id = engine.begin_open();
        engine.open_file(id, &other).unwrap();
        running.join().unwrap();
        assert_eq!(job.found.lock().unwrap().phase, Phase::Stopped);
    }

    /// How long reading points takes beside opening a large log, as a track
    /// window open while the log opens does. A measurement, not a check:
    /// `cargo test --release -p logstudio-extras -- --ignored --nocapture measure`
    /// (`TRACK_ROWS` rows, a third of them positions; 3,000,000 by default).
    #[test]
    #[ignore]
    fn measure_reading_points_of_a_large_log() {
        use std::time::Instant;
        let rows: u64 = std::env::var("TRACK_ROWS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3_000_000);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("large.log");
        {
            use std::io::Write;
            let mut out = std::io::BufWriter::new(std::fs::File::create(&path).unwrap());
            for i in 0..rows {
                if i % 3 == 0 {
                    out.write_all(position(i).as_bytes()).unwrap();
                } else {
                    writeln!(out, "10-25 09:00:00.000  100  102 I App: synthetic filler row {i} with some words").unwrap();
                }
            }
        }
        let bytes = std::fs::metadata(&path).unwrap().len();
        let engine = Arc::new(Engine::default());
        let started = Instant::now();
        let id = engine.begin_open();
        engine.open_file(id, &path).unwrap();
        let d = engine.shown().unwrap();
        let job = job(&d);
        let running = start(&job, &engine, &d);
        let (mut indexed, mut first) = (None, None);
        while !running.is_finished() {
            if indexed.is_none() && d.index.lock().unwrap().done {
                indexed = Some(started.elapsed());
            }
            if first.is_none() && !job.found.lock().unwrap().points.is_empty() {
                first = Some(started.elapsed());
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        let read = started.elapsed();
        let found = job.found.lock().unwrap();
        assert_eq!(found.phase, Phase::Complete);
        eprintln!(
            "{rows} rows, {:.0} MiB: first point {:?}, indexed {:?}, {} points read {:?} ({:.0} MiB/s); points response {:.1} MiB",
            bytes as f64 / 1048576.0,
            first.unwrap(),
            indexed,
            found.points.len(),
            read,
            bytes as f64 / 1048576.0 / read.as_secs_f64(),
            found.points.len() as f64 * 24.0 / 1048576.0
        );
    }

    #[test]
    fn found_points_forget_reread_blocks_and_stop_when_full() {
        let p = |line| Point {
            line,
            latitude: 1.0,
            longitude: 2.0,
        };
        let mut f = Found::default();
        f.add(0, vec![p(1), p(2)], 1, 100);
        f.add(1, vec![p(9)], 0, 200);
        assert_eq!(f.json(5)["count"], 3);
        assert_eq!(f.json(5)["stable"], 2);
        assert_eq!(f.json(5)["skipped"], 1);
        assert_eq!(f.json(5)["through"], 200);
        f.forget(1);
        assert_eq!(f.json(5)["through"], 100);
        assert_eq!(f.points.len(), 2);
        f.add(1, vec![p(9), p(10)], 2, 210);
        assert_eq!(f.json(5)["skipped"], 3);
        f.phase = Phase::Complete;
        assert_eq!(f.json(5)["stable"], 4);
        let mut full = Found::default();
        full.add(0, vec![p(1); MAX_POINTS + 3], 0, 9);
        assert_eq!(full.points.len(), MAX_POINTS);
        assert_eq!(full.phase, Phase::Full);
    }
}
