// What the track page works out apart from the map: its points as they
// arrive, the point a row was logged at, the points near a click, where the
// direction arrows go, and the words for where reading and the selected row
// stand. Node tests use it
// without Leaflet or a page.

const formatNumber = n => Number(n).toLocaleString('ko-KR');
const degrees = (lat, lon) => `${lat.toFixed(5)}, ${lon.toFixed(5)}`;

// How much nearer, in the units of a pass's distance (pixels), another point
// of a pass must be to win over the track's first or last point.
const END_SLACK = 2;

// Points in row order: each with its physical row, latitude and longitude.
export class Points {
  constructor() {
    this.count = 0;
    // Changes whenever points are added or dropped, so drawing can tell.
    this.revision = 0;
    this.lines = new Float64Array(1024);
    this.lat = new Float64Array(1024);
    this.lon = new Float64Array(1024);
  }
  // Adds points sent as row, latitude, longitude triples.
  append(triples) {
    const n = Math.floor(triples.length / 3);
    if (!n) return;
    if (this.count + n > this.lines.length) {
      let size = this.lines.length;
      while (size < this.count + n) size *= 2;
      for (const key of ['lines', 'lat', 'lon']) {
        const grown = new Float64Array(size);
        grown.set(this[key].subarray(0, this.count));
        this[key] = grown;
      }
    }
    for (let i = 0; i < n; i++) {
      this.lines[this.count + i] = triples[3 * i];
      this.lat[this.count + i] = triples[3 * i + 1];
      this.lon[this.count + i] = triples[3 * i + 2];
    }
    this.count += n;
    this.revision++;
  }
  truncate(n) {
    const kept = Math.max(0, Math.min(this.count, n));
    if (kept !== this.count) {this.count = kept;this.revision++;}
  }
  clear() {this.truncate(0);}
  // The last point at or before physical row `line`: where the log's
  // source was when it wrote that row. -1 before the first point.
  at(line) {
    let low = 0, high = this.count;
    while (low < high) {
      const mid = (low + high) >>> 1;
      if (this.lines[mid] <= line) low = mid + 1; else high = mid;
    }
    return low - 1;
  }
  // The points near a click, one for each pass of the track there: points
  // inside `box` (degrees: south, west, north, east) within `radius` by
  // `distance(i)`, and of each run of consecutive points the nearest. In
  // row order, so a track that comes back is picked in the order it went.
  passes(box, distance, radius) {
    const found = [];
    let last = -2;
    for (let i = 0; i < this.count; i++) {
      const lat = this.lat[i], lon = this.lon[i];
      if (lat < box.south || lat > box.north || lon < box.west || lon > box.east) continue;
      const d = distance(i);
      if (d >= radius) continue;
      if (i === last + 1 && found.length) {
        // The track's first and last points, which their marks show, win
        // over points of the same pass no more than END_SLACK nearer.
        const pass = found[found.length - 1], end = k => k === 0 || k === this.count - 1;
        const better = end(i) ? d <= pass.distance + END_SLACK : end(pass.index) ? d < pass.distance - END_SLACK : d < pass.distance;
        if (better) {pass.index = i;pass.distance = d;}
      } else {
        found.push({index:i, distance:d});
      }
      last = i;
    }
    return found;
  }
}

// How reading the shown log's points stands: the status line, whether it
// is an error, and the note over the map when there is nothing to draw.
// `markers` lists each source's marker in priority order; a source after
// the first shows its rank and marker on the status line. Sources may share
// a marker, which the note names once.
export function describeTrack(status, {markers = []} = {}) {
  const count = Number.isSafeInteger(status?.count) ? status.count : 0;
  const skipped = Number.isSafeInteger(status?.skipped) ? status.skipped : 0;
  const source = Number.isSafeInteger(status?.source) && status.source > 0 && markers[status.source] !== undefined
    ? ` · ${status.source + 1}순위 “${markers[status.source]}”` : '';
  const points = `지점 ${formatNumber(count)}개`;
  const unread = skipped ? ` · 좌표를 읽지 못한 행 ${formatNumber(skipped)}개` : '';
  const quoted = [...new Set(markers)].map(m => `“${m}”`);
  switch (status?.phase) {
    case 'reading':
      return {text:`읽는 중… ${points}${source}${unread}`, failed:false, note:null};
    case 'following':
      return {text:`새 줄을 기다리는 중 · ${points}${source}${unread}`, failed:false, note:null};
    case 'full':
      return {text:`${points}에서 멈춤${source} · 한 로그에 최대 ${formatNumber(count)}개까지 표시합니다${unread}`, failed:false, note:null};
    case 'failed':
      return {text:status.error || '로그를 읽지 못했습니다.', failed:true, note:null};
    case 'stopped':
      return {text:`${points}${source}${unread}`, failed:false, note:null};
    case 'complete':
      return {
        text:`${points}${source}${unread}`,
        failed:false,
        note:count ? null : skipped
          ? `${quoted.join(' 또는 ')}가 있는 행 ${formatNumber(skipped)}개에서 좌표를 읽지 못했습니다. 설정 파일의 latitude·longitude를 확인해 주세요.`
          : `이 로그에는 ${quoted.join(' 또는 ')}가 있는 행이 없습니다.`,
      };
    default:
      return {text:'', failed:false, note:null};
  }
}

// The selected row's chip: its words and how the map marks it. `pending`
// is a row the reading has not reached, which may still get its own
// point; `outside` is one past the points a full log keeps.
export function describeSelected(points, line, status) {
  if (!Number.isSafeInteger(line)) return {text:'', mark:'none'};
  const row = `${formatNumber(line)}행`;
  const through = Number.isSafeInteger(status?.through) ? status.through : 0;
  if (line > through && status?.phase === 'full') return {text:`${row}은 표시 한도 밖입니다`, mark:'none'};
  const index = points.at(line);
  if (line > through && (status?.phase === 'reading' || status?.phase === 'following')) {
    return {text:`${row}은 아직 읽지 않았습니다 · ${formatNumber(through)}행까지 읽음`, mark:index < 0 ? 'none' : 'pending'};
  }
  if (index < 0) return {text:`${row} 앞에는 지점이 없습니다`, mark:'none'};
  const at = points.lines[index], where = degrees(points.lat[index], points.lon[index]);
  return at === line
    ? {text:`${row} 위치 · ${where}`, mark:'exact'}
    : {text:`${row} → ${formatNumber(at)}행 지점 · ${where}`, mark:'before'};
}

// What hovering a pass shows: its row, whether it starts or ends the
// track, and how often the track passes there.
export function describePass(points, passes, k) {
  const index = passes[k].index, line = formatNumber(points.lines[index]);
  const label = index === 0 ? `시작 · ${line}행` : index === points.count - 1 ? `끝 · ${line}행` : `${line}행`;
  return passes.length > 1 ? `${label} · 이곳을 ${passes.length}번 지남 (${k + 1}/${passes.length})` : label;
}

// Why tiles are missing, for the note over the map.
export function describeTiles(failed, template) {
  if (!failed) return '';
  const local = /^http:\/\/(localhost|127\.0\.0\.1)(:\d+)?\//.test(template);
  return `지도 타일 ${formatNumber(failed)}개를 받지 못했습니다 · ${local ? '이 컴퓨터의 타일 서버를' : '네트워크 연결을'} 확인해 주세요`;
}

// Where the direction arrows go on a canvas `width` by `height` pixels.
// `scale` is 256 * 2^zoom and `offX`, `offY` move Web Mercator pixels (as
// Leaflet projects) to the canvas; `south`..`east` bound the canvas in
// degrees. An arrow sits every `gap` pixels along the track, pointing the way
// it goes, at most one in each `cell` square so a track winding in a small
// space does not fill it. Points nearer than `step` pixels to the last one
// taken are skipped, so an arrow's direction comes from a real step. Only
// segments that may cross the canvas are projected; one whose ends both lie
// off it still gets its arrows. Arrows past point `split` are `later`.
export function placeArrows(points, {scale, offX, offY, south, north, west, east, width, height, split, gap = 90, cell = 36, step = 4}) {
  const LIMIT = 85.0511287798, RAD = Math.PI / 180;
  const projectY = lat => scale * (0.5 - Math.atanh(Math.sin(Math.max(-LIMIT, Math.min(LIMIT, lat)) * RAD)) / (2 * Math.PI)) + offY;
  const projectX = lon => scale * (0.5 + lon / 360) + offX;
  const found = [], taken = new Set();
  if (points.count < 2) return found;
  // The last point taken, in degrees, and in pixels once projected.
  let lat0 = points.lat[0], lon0 = points.lon[0], x0 = 0, y0 = 0, projected = false;
  let run = gap / 2;
  for (let i = 1; i < points.count; i++) {
    const lat = points.lat[i], lon = points.lon[i];
    if (Math.max(lat, lat0) < south || Math.min(lat, lat0) > north || Math.max(lon, lon0) < west || Math.min(lon, lon0) > east) {
      lat0 = lat;lon0 = lon;projected = false;
      continue;
    }
    if (!projected) {x0 = projectX(lon0);y0 = projectY(lat0);projected = true;}
    const x = projectX(lon), y = projectY(lat), dx = x - x0, dy = y - y0, length = Math.hypot(dx, dy);
    if (length < step) continue;
    // The arrows' places on this segment lie `gap` apart, counted on from
    // the last place before it (`first`, measured along it); only those on
    // the canvas are visited, so a long segment far zoomed in costs no more
    // than a short one.
    const first = -run;
    run = (run + length) % gap;
    let enter = 0, exit = 1;
    for (const [p, q] of [[-dx, x0], [dx, width - x0], [-dy, y0], [dy, height - y0]]) {
      if (p === 0) {if (q < 0) exit = -1;continue;}
      if (p < 0) enter = Math.max(enter, q / p); else exit = Math.min(exit, q / p);
    }
    for (let k = Math.max(1, Math.ceil((enter * length - first) / gap)); first + k * gap <= exit * length; k++) {
      const t = (first + k * gap) / length, ax = x0 + dx * t, ay = y0 + dy * t;
      const key = `${Math.floor(ax / cell)},${Math.floor(ay / cell)}`;
      if (taken.has(key)) continue;
      taken.add(key);
      found.push({x:ax, y:ay, angle:Math.atan2(dy, dx), later:i > split});
    }
    lat0 = lat;lon0 = lon;x0 = x;y0 = y;
  }
  return found;
}
