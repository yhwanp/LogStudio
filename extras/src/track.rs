//! Track: draws on a map the coordinates that rows of the shown log carry,
//! and links the map's points with their rows.
//!
//! A row holds a point when it contains a source's `marker` followed by
//! its latitude and longitude numbers: along paths of the first JSON object
//! there, or in text after their names or at their places among its values.
//! The numbers times `scale` are degrees. A log's points all come from one
//! source: the first, in priority order, that has any point in the log.
//! The points are read in the background as the page asks for them, block
//! by block in row order, so the page draws them while the log is still
//! being read, and a followed log's new rows add points as they arrive.
use crate::{Extra, File, Host, Query, Reply, CSS, HTML, JS};
use logstudio_core::{dataset::Dataset, source::ByteLines, Engine};
use memchr::memmem::Finder;
use serde_json::{json, Map, Value};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
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
/// Keys of a `Source`, the only ones an item of `sources` may have.
const SOURCE_KEYS: [&str; 5] = ["marker", "format", "latitude", "longitude", "scale"];
/// Items `sources` may hold; each is searched for in every block until one
/// before it has points.
const MAX_SOURCES: usize = 8;
/// The last place among a text row's values a number may be read from.
const MAX_PLACE: u64 = 1000;

/// What a settings file `track.json` says.
#[derive(Debug, PartialEq)]
pub struct Settings {
    pub title: String,
    /// Where points come from, in priority order.
    pub sources: Vec<Source>,
    pub tiles: String,
    pub attribution: String,
}
impl Settings {
    pub fn from_json(value: &Value) -> Result<Self, String> {
        let object = value
            .as_object()
            .ok_or("설정 파일은 JSON 객체({ … })여야 합니다.")?;
        if object.keys().any(|k| SOURCE_KEYS.contains(&k.as_str())) {
            return Err(
                "marker·latitude·longitude 같은 좌표 항목은 sources 배열 안에 적어 주세요.".into(),
            );
        }
        let top = Fields {
            object,
            prefix: String::new(),
        };
        top.known(&["title", "tiles", "attribution", "sources"])?;
        let list = match object.get("sources") {
            Some(Value::Array(list)) if (1..=MAX_SOURCES).contains(&list.len()) => list,
            Some(Value::Array(_)) => {
                return Err(format!("sources 배열에는 1~{MAX_SOURCES}개를 적어 주세요."))
            }
            Some(_) => return Err("sources 값은 객체의 배열([{ … }, …])이어야 합니다.".into()),
            None => return Err("sources(좌표를 찾을 방법의 배열)를 적어 주세요.".into()),
        };
        let mut sources = Vec::with_capacity(list.len());
        for (i, value) in list.iter().enumerate() {
            let rank = format!("{}순위", i + 1);
            let object = value.as_object().ok_or(format!(
                "sources의 {rank} 값은 JSON 객체({{ … }})여야 합니다."
            ))?;
            let source = Fields {
                object,
                prefix: format!("{rank} "),
            };
            source.known(&SOURCE_KEYS)?;
            sources.push(Source::from_fields(&source)?);
        }
        let tiles = top.text("tiles", 300)?;
        let attribution = top.text("attribution", 200)?;
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
            title: top
                .text("title", 40)?
                .filter(|t| !t.trim().is_empty())
                .unwrap_or_else(|| NAME.into()),
            sources,
            tiles,
            attribution,
        })
    }
}

/// A JSON object in the settings; errors prefix its keys with `prefix`.
struct Fields<'a> {
    object: &'a Map<String, Value>,
    prefix: String,
}
impl Fields<'_> {
    /// Rejects any key not in `keys`.
    fn known(&self, keys: &[&str]) -> Result<(), String> {
        match self.object.keys().find(|k| !keys.contains(&k.as_str())) {
            Some(key) => Err(format!(
                "알 수 없는 항목입니다 ({}{}).",
                self.prefix,
                clip(key)
            )),
            None => Ok(()),
        }
    }
    fn text(&self, key: &str, max: usize) -> Result<Option<String>, String> {
        let name = format!("{}{key}", self.prefix);
        match self.object.get(key) {
            None => Ok(None),
            Some(Value::String(s)) if s.chars().count() <= max => Ok(Some(s.clone())),
            Some(Value::String(_)) => Err(format!("{name} 값은 {max}자까지입니다.")),
            Some(_) => Err(format!("{name} 값은 문자열이어야 합니다.")),
        }
    }
}

/// Rows that hold points, and how to read their numbers.
#[derive(Debug, PartialEq)]
pub struct Source {
    pub marker: Vec<u8>,
    pub format: Format,
    pub scale: f64,
}
/// Where a source's numbers are in the text after its marker.
#[derive(Debug, PartialEq)]
pub enum Format {
    /// Along dotted paths of the first JSON object there.
    Json {
        latitude: Vec<String>,
        longitude: Vec<String>,
    },
    /// In text, after their names or at their places among its values.
    Text { latitude: Field, longitude: Field },
}
/// Where a `text` source's number is.
#[derive(Debug, PartialEq)]
pub enum Field {
    /// After the name and `:` or `=`, as in `lat : 52.5, lon=13.4`.
    Name(Vec<u8>),
    /// The value at this place, from 1, as `52.5` is the second of
    /// `(sim 52.5 13.4)`.
    Place(usize),
}
impl Field {
    fn read(&self, text: &[u8]) -> Option<f64> {
        match self {
            Self::Name(name) => named(text, name),
            Self::Place(place) => placed(text, *place),
        }
    }
}
impl Source {
    fn from_fields(f: &Fields) -> Result<Self, String> {
        let p = &f.prefix;
        let marker = f
            .text("marker", 200)?
            .filter(|m| !m.is_empty())
            .ok_or(format!(
                "{p}marker(좌표가 있는 행을 찾을 문자열)를 적어 주세요."
            ))?;
        let format = match f.object.get("format") {
            None => "json",
            Some(Value::String(s)) if s == "json" || s == "text" => s.as_str(),
            Some(_) => return Err(format!("{p}format 값은 \"json\"이나 \"text\"여야 합니다.")),
        };
        let format = if format == "json" {
            let path = |key: &str| -> Result<Vec<String>, String> {
                if let Some(Value::Number(_)) = f.object.get(key) {
                    return Err(format!(
                        "{p}{key}의 순서 번호는 format이 \"text\"일 때만 쓸 수 있습니다."
                    ));
                }
                let path = f.text(key, 200)?.ok_or(format!(
                    "{p}{key}(JSON 안에서 숫자가 있는 경로)를 적어 주세요."
                ))?;
                let parts: Vec<String> = path.split('.').map(str::to_string).collect();
                if parts.iter().any(String::is_empty) || parts.len() > 16 {
                    return Err(format!(
                        "{p}{key} 값은 점(.)으로 나눈 경로여야 합니다 (예: position.lat)."
                    ));
                }
                Ok(parts)
            };
            Format::Json {
                latitude: path("latitude")?,
                longitude: path("longitude")?,
            }
        } else {
            let field = |key: &str| -> Result<Field, String> {
                match f.object.get(key) {
                    Some(Value::Number(n)) => n
                        .as_u64()
                        .filter(|n| (1..=MAX_PLACE).contains(n))
                        .map(|n| Field::Place(n as usize))
                        .ok_or(format!(
                            "{p}{key}의 순서 번호는 1부터 {MAX_PLACE}까지의 정수여야 합니다."
                        )),
                    None | Some(Value::String(_)) => f
                        .text(key, 200)?
                        .filter(|n| !n.trim().is_empty())
                        .map(|n| Field::Name(n.into_bytes()))
                        .ok_or(format!(
                            "{p}{key}(숫자 앞의 이름이나 몇 번째 값인지)를 적어 주세요."
                        )),
                    Some(_) => Err(format!(
                        "{p}{key} 값은 숫자 앞의 이름(문자열)이나 순서 번호(정수)여야 합니다."
                    )),
                }
            };
            Format::Text {
                latitude: field("latitude")?,
                longitude: field("longitude")?,
            }
        };
        let scale = match f.object.get("scale") {
            None => 1.0,
            Some(v) => v
                .as_f64()
                .filter(|s| s.is_finite() && *s > 0.0)
                .ok_or(format!("{p}scale 값은 0보다 큰 수여야 합니다."))?,
        };
        Ok(Self {
            marker: marker.into_bytes(),
            format,
            scale,
        })
    }
    /// The coordinates in the text after a row's marker. None when a number
    /// is missing, or the degrees are out of range or 0, 0.
    pub fn point(&self, after: &[u8]) -> Option<(f64, f64)> {
        let (latitude, longitude) = match &self.format {
            Format::Json {
                latitude,
                longitude,
            } => {
                // The first JSON object there, read to its end.
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
                (number(latitude)?, number(longitude)?)
            }
            Format::Text {
                latitude,
                longitude,
            } => (latitude.read(after)?, longitude.read(after)?),
        };
        let (latitude, longitude) = (latitude * self.scale, longitude * self.scale);
        // 0, 0 is what a position without a fix reads as.
        let place = latitude.abs() <= 90.0
            && longitude.abs() <= 180.0
            && !(latitude == 0.0 && longitude == 0.0);
        place.then_some((latitude, longitude))
    }
}

/// The number after the first `name` in `text` that `:` or `=` follows,
/// with spaces or tabs around it; a name inside a longer word does not
/// count. None when that value is not a number.
fn named(text: &[u8], name: &[u8]) -> Option<f64> {
    let word = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80;
    let blanks = |s: &[u8]| s.iter().take_while(|b| matches!(b, b' ' | b'\t')).count();
    for at in memchr::memmem::find_iter(text, name) {
        if at > 0 && word(text[at - 1]) && word(name[0]) {
            continue;
        }
        let rest = &text[at + name.len()..];
        let rest = &rest[blanks(rest)..];
        let Some(rest) = rest.strip_prefix(b":").or_else(|| rest.strip_prefix(b"=")) else {
            continue;
        };
        return number(&rest[blanks(rest)..]).map(|(n, _)| n);
    }
    None
}

/// The value at `place` (from 1) in `text`, whose values spaces, tabs,
/// commas and brackets separate. None when that value is not a number.
fn placed(text: &[u8], place: usize) -> Option<f64> {
    let gap = |b: &u8| {
        matches!(
            b,
            b' ' | b'\t' | b',' | b'(' | b')' | b'[' | b']' | b'{' | b'}'
        )
    };
    let value = text.split(gap).filter(|v| !v.is_empty()).nth(place - 1)?;
    number(value)
        .filter(|&(_, used)| used == value.len())
        .map(|(n, _)| n)
}

/// The finite number `text` starts with, and how many bytes it takes.
fn number(text: &[u8]) -> Option<(f64, usize)> {
    let end = text
        .iter()
        .take_while(|b| b.is_ascii_digit() || matches!(b, b'.' | b'-' | b'+' | b'e' | b'E'))
        .count();
    let n: f64 = std::str::from_utf8(&text[..end]).ok()?.parse().ok()?;
    n.is_finite().then_some((n, end))
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

/// What a block's rows, whose first is row `first`, hold: by source, its
/// points and the rows with its marker that hold none; and how many rows
/// hold any source's marker, which sources sharing a marker count once.
pub fn points(
    text: &[u8],
    first: u64,
    sources: &[Source],
    markers: &[Finder],
) -> (Vec<(Vec<Point>, u32)>, u32) {
    let mut found: Vec<(Vec<Point>, u32)> = sources.iter().map(|_| (vec![], 0)).collect();
    let mut marked = 0;
    let present: Vec<usize> = (0..sources.len())
        .filter(|&k| markers[k].find(text).is_some())
        .collect();
    if present.is_empty() {
        return (found, marked);
    }
    for (j, (_, row)) in ByteLines::new(text).enumerate() {
        let mut any = false;
        for &k in &present {
            let Some(at) = markers[k].find(row) else {
                continue;
            };
            any = true;
            match sources[k].point(&row[at + markers[k].needle().len()..]) {
                Some((latitude, longitude)) => found[k].0.push(Point {
                    line: first + j as u64,
                    latitude,
                    longitude,
                }),
                None => found[k].1 += 1,
            }
        }
        marked += u32::from(any);
    }
    (found, marked)
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

/// One source's points read so far, by block.
#[derive(Default)]
struct Seen {
    points: Vec<Point>,
    /// The first point of each block read.
    starts: Vec<usize>,
    skipped: Vec<u32>,
}

/// The points read so far, by block. Each source's stay apart, and once one
/// has points the sources after it are dropped: a log's track never mixes
/// them, and a source's points replace those of the sources after it.
#[derive(Default)]
pub struct Found {
    /// The sources still read, in priority order. None but the last can
    /// have points.
    seen: Vec<Seen>,
    /// The last row of each block read.
    ends: Vec<u64>,
    /// The rows of each block read that hold any source's marker.
    marked: Vec<u32>,
    pub phase: Phase,
    error: String,
}
impl Found {
    fn new(sources: usize) -> Self {
        Self {
            seen: (0..sources).map(|_| Seen::default()).collect(),
            ..Self::default()
        }
    }
    /// Adds what `points` read in a block whose last row is `last`; `found`
    /// may hold sources dropped meanwhile.
    fn add(&mut self, block: usize, (found, marked): (Vec<(Vec<Point>, u32)>, u32), last: u64) {
        debug_assert_eq!(block, self.ends.len());
        debug_assert!(found.len() >= self.seen.len());
        self.ends.push(last);
        self.marked.push(marked);
        for (seen, (points, skipped)) in self.seen.iter_mut().zip(found) {
            seen.starts.push(seen.points.len());
            seen.skipped.push(skipped);
            let room = MAX_POINTS - seen.points.len();
            seen.points
                .extend_from_slice(&points[..points.len().min(room)]);
        }
        if let Some(k) = self.seen.iter().position(|s| !s.points.is_empty()) {
            self.seen.truncate(k + 1);
            // A full lower source ends reading too: its points stay shown.
            if self.seen[k].points.len() == MAX_POINTS {
                self.phase = Phase::Full;
            }
        }
    }
    /// Forgets the points of blocks `block..`, which are read again.
    fn forget(&mut self, block: usize) {
        if block < self.ends.len() {
            self.ends.truncate(block);
            self.marked.truncate(block);
            for seen in &mut self.seen {
                seen.points.truncate(seen.starts[block]);
                seen.starts.truncate(block);
                seen.skipped.truncate(block);
            }
        }
    }
    /// The source whose points are shown: the one with any, else the first.
    fn source(&self) -> usize {
        self.seen
            .iter()
            .position(|s| !s.points.is_empty())
            .unwrap_or(0)
    }
    pub fn points(&self) -> &[Point] {
        &self.seen[self.source()].points
    }
    /// Points that stay as they are: a followed log's last block may be
    /// read again.
    fn stable(&self) -> usize {
        let seen = &self.seen[self.source()];
        match self.phase {
            Phase::Reading | Phase::Following => seen.starts.last().copied().unwrap_or(0),
            _ => seen.points.len(),
        }
    }
    /// The last row read, after which a selected row has no point yet.
    fn through(&self) -> u64 {
        self.ends.last().copied().unwrap_or(0)
    }
    pub fn json(&self, dataset: u64) -> Value {
        let source = self.source();
        let sum = |counts: &[u32]| counts.iter().map(|&n| u64::from(n)).sum::<u64>();
        // Before any point, every row with a marker is one without a point.
        let skipped = match self.points() {
            [] => sum(&self.marked),
            _ => sum(&self.seen[source].skipped),
        };
        json!({
            "datasetId":dataset,
            "phase":self.phase.as_str(),
            "source":source,
            "count":self.points().len(),
            "stable":self.stable(),
            "through":self.through(),
            "skipped":skipped,
            "error":self.error,
        })
    }
}

/// Reading one dataset's points.
pub struct Job {
    pub dataset: u64,
    stop: AtomicBool,
    /// How many sources are still being read (`Found::seen.len()`).
    sources: AtomicUsize,
    pub found: Mutex<Found>,
}
impl Job {
    fn new(dataset: u64, sources: usize) -> Self {
        Self {
            dataset,
            stop: AtomicBool::new(false),
            sources: AtomicUsize::new(sources),
            found: Mutex::new(Found::new(sources)),
        }
    }
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
        let markers: Vec<Finder> = settings
            .sources
            .iter()
            .map(|s| Finder::new(&s.marker))
            .collect();
        let mut from = 0;
        loop {
            let read = d.read_blocks(
                from,
                &self.stop,
                |b, rows| {
                    let n = self.sources.load(Ordering::Acquire);
                    let read = points(&rows.text, b.first, &settings.sources[..n], &markers[..n]);
                    (read, b.first + (b.rows as u64).saturating_sub(1))
                },
                |i, (points, last)| {
                    let mut found = self.found.lock().unwrap();
                    found.add(i, points, last);
                    self.sources.store(found.seen.len(), Ordering::Release);
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
        let job = Arc::new(Job::new(id, self.settings.sources.len()));
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
                "markers":self.settings.sources.iter()
                    .map(|s| String::from_utf8_lossy(&s.marker))
                    .collect::<Vec<_>>(),
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
            // Points `from..` of source `source` as little-endian f64
            // triples of row, latitude and longitude, with how many points
            // there are and how many of them stay (`Found::stable`). None
            // once another source's points are shown instead.
            "points" => {
                let (Some(id), Some(source), Some(from)) = (
                    query.number("dataset"),
                    query.number("source"),
                    query.number("from"),
                ) else {
                    return Reply::error(400, "잘못된 요청입니다.");
                };
                let job = self.job.lock().unwrap().clone();
                let Some(job) = job.filter(|j| j.dataset == id) else {
                    return Reply::error(409, "메인 창의 로그가 바뀌었습니다.");
                };
                let found = job.found.lock().unwrap();
                let points = if found.source() as u64 == source {
                    found.points()
                } else {
                    &[]
                };
                let from = (from as usize).min(points.len());
                let mut body = Vec::with_capacity((points.len() - from) * 24);
                for p in &points[from..] {
                    body.extend_from_slice(&(p.line as f64).to_le_bytes());
                    body.extend_from_slice(&p.latitude.to_le_bytes());
                    body.extend_from_slice(&p.longitude.to_le_bytes());
                }
                let (count, stable) = (points.len(), found.stable().min(points.len()));
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
        one(json!({
            "marker":"onPositionChanged",
            "latitude":"fix.lat",
            "longitude":"fix.lon",
            "scale":1e-7
        }))
    }
    /// The settings of `sources`, in priority order.
    fn of(sources: Value) -> Settings {
        Settings::from_json(&json!({ "sources": sources })).unwrap()
    }
    fn one(source: Value) -> Settings {
        of(json!([source]))
    }

    #[test]
    fn settings_take_defaults_and_reject_mistakes() {
        let s = settings();
        assert_eq!(s.title, "좌표 궤적");
        assert_eq!(s.tiles, OSM_TILES);
        assert_eq!(s.attribution, OSM_ATTRIBUTION);
        assert_eq!(
            s.sources,
            [Source {
                marker: b"onPositionChanged".to_vec(),
                format: Format::Json {
                    latitude: vec!["fix".into(), "lat".into()],
                    longitude: vec!["fix".into(), "lon".into()],
                },
                scale: 1e-7,
            }]
        );
        let valid = || json!({"marker":"m","latitude":"a","longitude":"b"});
        let with = |source: Value| json!({ "sources": [source] });
        for (bad, why) in [
            (json!([]), "객체"),
            (json!({"title":"t"}), "sources(좌표를 찾을 방법"),
            // A source's keys go in the list, never beside it.
            (
                valid(),
                "marker·latitude·longitude 같은 좌표 항목은 sources 배열 안에",
            ),
            (json!({"sources":valid()}), "객체의 배열"),
            (json!({"sources":[]}), "1~8개"),
            (
                json!({ "sources": (0..9).map(|_| valid()).collect::<Vec<_>>() }),
                "1~8개",
            ),
            (json!({"sources":["m"]}), "sources의 1순위 값은 JSON 객체"),
            (
                json!({"sources":[valid()],"fallback":[valid()]}),
                "(fallback)",
            ),
            (
                with(json!({"latitude":"a","longitude":"b"})),
                "1순위 marker",
            ),
            (
                with(json!({"marker":"m","longitude":"b"})),
                "1순위 latitude",
            ),
            (
                with(json!({"marker":"m","latitude":"a..b","longitude":"b"})),
                "경로",
            ),
            (
                with(json!({"marker":"m","latitude":"a","longitude":"b","scale":0})),
                "1순위 scale",
            ),
            (
                with(json!({"marker":"m","latitude":"a","longitude":"b","colour":"red"})),
                "(1순위 colour)",
            ),
            // A source is one level deep, and the map's keys are not its.
            (
                with(json!({"marker":"m","latitude":"a","longitude":"b","sources":[]})),
                "(1순위 sources)",
            ),
            (
                with(json!({"marker":"m","latitude":"a","longitude":"b","title":"t"})),
                "(1순위 title)",
            ),
            (
                json!({"sources":[valid()],"tiles":"http://t.example/{z}/{x}/{y}.png"}),
                "https",
            ),
            (
                json!({"sources":[valid()],"tiles":"https://t.example/tiles.png"}),
                "https",
            ),
            (
                with(json!({"marker":7,"latitude":"a","longitude":"b"})),
                "1순위 marker 값은 문자열",
            ),
            (
                with(json!({"marker":"m","format":"csv","latitude":"a","longitude":"b"})),
                "1순위 format",
            ),
            (
                with(json!({"marker":"g","format":"text","latitude":" ","longitude":"b"})),
                "1순위 latitude",
            ),
        ] {
            let error = Settings::from_json(&bad).unwrap_err();
            assert!(error.contains(why), "{bad}: {error}");
        }
        // Other tiles carry no OpenStreetMap credit unless given one.
        let other = Settings::from_json(
            &json!({"sources":[valid()],"tiles":"https://tiles.example.org:8443/{z}/{x}/{y}.png"}),
        )
        .unwrap();
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
        // Each source has its own format and scale; text numbers may be
        // places.
        let list = of(json!([
            valid(),
            {"marker":"Fix","format":"text","latitude":"lat","longitude":"lon"},
            {"marker":"Fix","format":"text","latitude":3,"longitude":4,"scale":0.5}
        ]));
        assert_eq!(list.sources.len(), 3);
        assert_eq!(
            list.sources[1],
            Source {
                marker: b"Fix".to_vec(),
                format: Format::Text {
                    latitude: Field::Name(b"lat".to_vec()),
                    longitude: Field::Name(b"lon".to_vec()),
                },
                scale: 1.0,
            }
        );
        assert_eq!(
            (&list.sources[2].format, list.sources[2].scale),
            (
                &Format::Text {
                    latitude: Field::Place(3),
                    longitude: Field::Place(4),
                },
                0.5
            )
        );
        let second = |v: Value| json!({ "sources": [valid(), v] });
        for (bad, why) in [
            (second(json!("g")), "sources의 2순위 값은 JSON 객체"),
            (
                second(json!({"latitude":"a","longitude":"b"})),
                "2순위 marker",
            ),
            // Places are whole numbers from 1, and only text reads them.
            (
                second(json!({"marker":"g","format":"text","latitude":0,"longitude":2})),
                "2순위 latitude의 순서 번호",
            ),
            (
                second(json!({"marker":"g","format":"text","latitude":1.5,"longitude":2})),
                "순서 번호",
            ),
            (
                second(json!({"marker":"g","format":"text","latitude":true,"longitude":2})),
                "숫자 앞의 이름(문자열)이나 순서 번호",
            ),
            (
                second(json!({"marker":"g","latitude":1,"longitude":2})),
                "2순위 latitude의 순서 번호는 format이 \"text\"일 때만",
            ),
        ] {
            let error = Settings::from_json(&bad).unwrap_err();
            assert!(error.contains(why), "{bad}: {error}");
        }
    }

    #[test]
    fn a_point_is_the_first_object_after_the_marker() {
        let s = &settings().sources[0];
        let row = br#" {"fix":{"lat":374512345,"lon":1270123456,"alt":0},"t":5} trailing"#;
        let (lat, lon) = s.point(row).unwrap();
        assert!((lat - 37.4512345).abs() < 1e-9 && (lon - 127.0123456).abs() < 1e-9);
        // Arrays are indexed by number.
        let arrays = one(json!({"marker":"m","latitude":"p.0","longitude":"p.1"}));
        assert_eq!(
            arrays.sources[0].point(br#"{"p":[48.5,-2.25]}"#),
            Some((48.5, -2.25))
        );
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
    fn a_text_point_is_the_numbers_after_their_names() {
        let s = one(json!({"marker":"Fix","format":"text","latitude":"lat","longitude":"lon"}));
        let s = &s.sources[0];
        let row = b"source : sim, time : 1700000000000, latAccuracy : 3.5, lat : 52.500000, lon : 13.400000, alt : 9.0)";
        assert_eq!(s.point(row), Some((52.5, 13.4)));
        for (some, at) in [
            (&b"lat=-33.8568,lon=151.2153"[..], (-33.8568, 151.2153)),
            (b"lat\t:\t1.5e1, lon :-2", (15.0, -2.0)),
            // A name inside a longer word, or without : or =, is not it.
            (
                b"rawlat : 1, lat_ok : 2, lat 3, lat : 4, lon : 5",
                (4.0, 5.0),
            ),
        ] {
            assert_eq!(s.point(some), Some(at), "{}", String::from_utf8_lossy(some));
        }
        for none in [
            &b"lat : 52.5"[..],
            b"lat : , lon : 13.4",
            b"lat : \"52.5\", lon : 13.4",
            b"lat : north, lat : 52.5, lon : 13.4",
            b"lat : 0.0, lon : 0.0",
            b"lat : 1e999, lon : 1",
            b"latitude : 52.5, longitude : 13.4",
        ] {
            assert_eq!(s.point(none), None, "{}", String::from_utf8_lossy(none));
        }
    }

    #[test]
    fn a_placed_point_is_the_values_at_their_places() {
        let s = one(json!({"marker":"Fix","format":"text","latitude":4,"longitude":5}));
        let s = &s.sources[0];
        // Spaces, tabs, commas and brackets separate values, and runs of
        // them count once.
        for (some, at) in [
            (
                &b"(sim 1700000000000 12345 52.5000001 13.4000002 21 36.5 0.2 1)"[..],
                (52.5000001, 13.4000002),
            ),
            (b"[sim,  1,\t2, -33.8568,151.2153]", (-33.8568, 151.2153)),
            (b"(a b c 1.5e1 -2)", (15.0, -2.0)),
        ] {
            assert_eq!(s.point(some), Some(at), "{}", String::from_utf8_lossy(some));
        }
        for none in [
            // A value that is not wholly a number, or none at the place.
            &b"(sim 1 2 52.5N 13.4)"[..],
            b"(sim 1 2 \"52.5\" 13.4)",
            b"(sim 1 2 52.5)",
            b"(sim 1 2 0 0)",
        ] {
            assert_eq!(s.point(none), None, "{}", String::from_utf8_lossy(none));
        }
        // A space inside an earlier value moves the places: other numbers
        // are read, and nothing can tell.
        assert_eq!(s.point(b"(sim card 1 2 52.5 13.4)"), Some((2.0, 52.5)));
    }

    #[test]
    fn points_keep_physical_rows_across_line_endings() {
        let settings = settings();
        let markers = [Finder::new(&settings.sources[0].marker)];
        let text = b"10-25 09:00:00.000  100  101 D Nav: start\r\n\
10-25 09:00:00.100  100  101 D Nav: onPositionChanged {\"fix\":{\"lat\":374512345,\"lon\":1270123456}}\r\
10-25 09:00:00.200  100  101 D Nav: onPositionChanged {\"fix\":{\"lat\":\"bad\"}}\n\
10-25 09:00:00.300  100  101 D Nav: onPositionChanged {\"fix\":{\"lat\":374512400,\"lon\":1270123500}}";
        let (found, marked) = points(text, 41, &settings.sources, &markers);
        let (found, skipped) = &found[0];
        assert_eq!((*skipped, marked), (1, 3));
        assert_eq!(found.iter().map(|p| p.line).collect::<Vec<_>>(), [42, 44]);
        assert_eq!(
            points(b"no marker here\n", 1, &settings.sources, &markers),
            (vec![(vec![], 0)], 0)
        );
    }

    #[test]
    fn sources_sharing_a_marker_count_a_row_once() {
        let s = of(json!([
            {"marker":"Fix","format":"text","latitude":"lat","longitude":"lon"},
            {"marker":"Fix","format":"text","latitude":2,"longitude":3}
        ]));
        let markers: Vec<Finder> = s.sources.iter().map(|s| Finder::new(&s.marker)).collect();
        let text = b"a Fix(sim 52.5 13.4)\nb Fix(lat : 48.1, lon : 2.2)\nc Fix(sim pending)\n";
        let (found, marked) = points(text, 1, &s.sources, &markers);
        assert_eq!(marked, 3);
        assert_eq!((found[0].0.len(), found[0].1), (1, 2));
        assert_eq!((found[1].0.len(), found[1].1), (1, 2));
        assert_eq!(found[1].0[0].line, 1);
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
        Arc::new(Job::new(d.id, 1))
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
        assert_eq!(found.points().len(), 3_000);
        assert!(found
            .points()
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
            f.phase == Phase::Following && f.points().len() == 5
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
        wait("two more", || job.found.lock().unwrap().points().len() == 7);
        file.write_all(&late.as_bytes()[20..]).unwrap();
        wait("the completed row", || {
            job.found.lock().unwrap().points().len() == 8
        });
        {
            let f = job.found.lock().unwrap();
            assert!(
                f.stable() < 8,
                "a followed log's last block may be read again"
            );
            let lines: Vec<u64> = f.points().iter().map(|p| p.line).collect();
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
            if first.is_none() && !job.found.lock().unwrap().points().is_empty() {
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
            found.points().len(),
            read,
            bytes as f64 / 1048576.0 / read.as_secs_f64(),
            found.points().len() as f64 * 24.0 / 1048576.0
        );
    }

    #[test]
    fn found_points_forget_reread_blocks_and_stop_when_full() {
        let p = |line| Point {
            line,
            latitude: 1.0,
            longitude: 2.0,
        };
        let mut f = Found::new(1);
        f.add(0, (vec![(vec![p(1), p(2)], 1)], 3), 100);
        f.add(1, (vec![(vec![p(9)], 0)], 1), 200);
        assert_eq!(f.json(5)["count"], 3);
        assert_eq!(f.json(5)["stable"], 2);
        assert_eq!(f.json(5)["skipped"], 1);
        assert_eq!(f.json(5)["through"], 200);
        f.forget(1);
        assert_eq!(f.json(5)["through"], 100);
        assert_eq!(f.points().len(), 2);
        f.add(1, (vec![(vec![p(9), p(10)], 2)], 4), 210);
        assert_eq!(f.json(5)["skipped"], 3);
        f.phase = Phase::Complete;
        assert_eq!(f.json(5)["stable"], 4);
        let mut full = Found::new(1);
        full.add(0, (vec![(vec![p(1); MAX_POINTS + 3], 0)], 0), 9);
        assert_eq!(full.points().len(), MAX_POINTS);
        assert_eq!(full.phase, Phase::Full);
    }

    #[test]
    fn a_lower_source_is_shown_until_a_higher_one_has_points_and_never_mixed() {
        let p = |line| Point {
            line,
            latitude: 1.0,
            longitude: 2.0,
        };
        let mut f = Found::new(2);
        f.add(0, (vec![(vec![], 1), (vec![], 2)], 2), 100);
        // Before any point, every row with a marker counts, once.
        assert_eq!(f.json(5)["source"], 0);
        assert_eq!(f.json(5)["skipped"], 2);
        f.add(1, (vec![(vec![], 0), (vec![p(150), p(160)], 0)], 2), 200);
        assert_eq!(f.json(5)["source"], 1);
        assert_eq!(f.json(5)["skipped"], 2);
        assert_eq!(f.json(5)["stable"], 0);
        // The first source's first point drops the second's.
        f.add(
            2,
            (vec![(vec![p(250)], 0), (vec![p(240), p(260)], 0)], 3),
            300,
        );
        assert_eq!(f.json(5)["source"], 0);
        assert_eq!(f.points(), [p(250)]);
        assert_eq!(f.seen.len(), 1);
        // A block read for both then adds the first source's alone.
        f.add(3, (vec![(vec![p(350)], 0), (vec![p(340)], 0)], 2), 400);
        assert_eq!(f.points(), [p(250), p(350)]);
        f.forget(3);
        assert_eq!(f.points(), [p(250)]);
        // A full lower source stops reading.
        let mut full = Found::new(2);
        full.add(0, (vec![(vec![], 0), (vec![p(1); MAX_POINTS], 0)], 0), 9);
        assert_eq!((full.phase, full.source()), (Phase::Full, 1));
    }

    #[test]
    fn a_job_draws_a_lower_source_only_for_a_log_without_a_higher_ones_points() {
        let named = |i: u64| {
            format!("10-25 09:00:01.000  100  103 I Gps: Fix(lat : 48.{i:06}, lon : 2.{i:06})\n")
        };
        let placed = |i: u64| {
            format!("10-25 09:00:02.000  100  103 I Gps: Fix(sim 47.{i:06} 3.{i:06} 21)\n")
        };
        let three = of(json!([
            {"marker":"onPositionChanged","latitude":"fix.lat","longitude":"fix.lon","scale":1e-7},
            {"marker":"Fix","format":"text","latitude":"lat","longitude":"lon"},
            {"marker":"Fix","format":"text","latitude":2,"longitude":3}
        ]));
        let read = |text: &str| {
            let (_dir, engine, d) = shown(text);
            let job = Arc::new(Job::new(d.id, 3));
            job.run(&engine, &d, &three);
            let found = job.found.lock().unwrap();
            assert_eq!(found.phase, Phase::Complete, "{}", found.error);
            (
                found.source(),
                found.points().iter().map(|p| p.line).collect::<Vec<_>>(),
            )
        };
        let only = (0..20_000).map(named).collect::<String>();
        let (source, lines) = read(&only);
        assert_eq!((source, lines.len(), lines[0]), (1, 20_000, 1));
        // The first source's points later in the log replace every point of
        // the second.
        let both = only.clone() + &(0..3).map(|i| position(i) + &named(i)).collect::<String>();
        assert_eq!(read(&both), (0, vec![20_001, 20_003, 20_005]));
        // A third source draws only a log that has no point of the others.
        let third = (0..10).map(placed).collect::<String>();
        assert_eq!(read(&third), (2, (1..=10).collect()));
        assert_eq!(read(&(third + &named(1))), (1, vec![11]));
    }
}
