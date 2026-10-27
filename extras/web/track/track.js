// The track window: the shown log's points on a map, drawn as they are
// read, linked with the main window both ways. Clicking the track, or [ and
// ] for the previous and next point, selects that point's row there;
// selecting a row there marks where the log's source was when it wrote that
// row (the last point at or before it), and the track after it fades.
import {api, locate, watchMain} from './common/host.js';
import {Points, describeTrack, describeSelected, describePass, describeTiles} from './track-model.js';

const L = window.L;
const $ = id => document.getElementById(id);
// Points per polyline: a growing track redraws only its last piece.
const CHUNK = 20000;
// How near a click must be to the track to pick a point, in pixels.
const PICK_RADIUS = 14;
const FINAL = new Set(['complete', 'full', 'failed', 'stopped']);
const COLOR = '#c2185b';
// The track up to the selected row, on a white casing so it stays apart
// from roads drawn in similar colors, and the track after it, faded.
const PAST = {color:COLOR, weight:3.5, opacity:0.95, interactive:false};
const CASING = {color:'#ffffff', weight:7, opacity:0.85, interactive:false};
const LATER = {color:COLOR, weight:3, opacity:0.3, interactive:false};
// A tile that failed shows hatched, apart from the map's empty background.
const MISSING = 'data:image/svg+xml,' + encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="256" height="256"><defs><pattern id="h" width="12" height="12" patternUnits="userSpaceOnUse" patternTransform="rotate(45)"><rect width="12" height="12" fill="#e4e8ed"/><rect width="4" height="12" fill="#d3d9e0"/></pattern></defs><rect width="256" height="256" fill="url(#h)"/></svg>');

const ui = {status:$('status'), selection:$('selection'), empty:$('empty'), alert:$('alert'), tip:$('tip'), announce:$('announce'), fit:$('fit'), follow:$('follow'), title:$('title')};
const points = new Points();
let settings = {markers:[], tiles:'', attribution:'', title:''};
let datasetId = null, generation = 0, selected = null, picked = null, status = null;
let moved = false, fitting = false, lastFit = 0, drawnKey = '', cycle = null;
let notice = '', noticeTimer = null, announced = '', failedTiles = 0;

// Tiles show as they arrive, without a fade: it waits for animation frames,
// which a covered window does not get.
const map = L.map('map', {preferCanvas:true, worldCopyJump:true, fadeAnimation:false, zoomSnap:0.5, zoomControl:false}).setView([20, 0], 2);
L.control.zoom({zoomInTitle:'확대', zoomOutTitle:'축소'}).addTo(map);
map.attributionControl.setPrefix('Leaflet');
L.control.scale({imperial:false}).addTo(map);
// Moving the map by hand ends fitting the track to it.
map.on('dragstart', () => {moved = true;});
map.on('zoomstart', () => {if (!fitting) moved = true;});

const latLngs = (from, to) => {
  const out = new Array(Math.max(0, to - from));
  for (let i = from; i < to; i++) out[i - from] = [points.lat[i], points.lon[i]];
  return out;
};

// One style of the track over points `start..end`, in pieces of CHUNK
// points. Each piece also takes the next piece's first point, so the pieces
// join; only the last piece changes as points arrive.
class Line {
  constructor(style) {
    this.style = style;
    this.start = 0;
    this.end = 0;
    this.revision = -1;
    this.pieces = [];
    this.tail = L.polyline([], style).addTo(map);
  }
  // Returns whether pieces were added, which the stacking then fixes.
  set(start, end) {
    if (end - start < 2) end = start;
    if (start !== this.start) {
      for (const piece of this.pieces) piece.remove();
      this.pieces = [];
      this.start = start;
    }
    while (this.pieces.length && end <= start + this.pieces.length * CHUNK) this.pieces.pop().remove();
    let added = false;
    while (end > start + (this.pieces.length + 1) * CHUNK) {
      const from = start + this.pieces.length * CHUNK;
      this.pieces.push(L.polyline(latLngs(from, from + CHUNK + 1), this.style).addTo(map));
      added = true;
    }
    if (added || end !== this.end || points.revision !== this.revision) {
      this.tail.setLatLngs(latLngs(start + this.pieces.length * CHUNK, end));
    }
    this.end = end;
    this.revision = points.revision;
    return added;
  }
  layers() {return [...this.pieces, this.tail];}
}
// Drawn bottom to top: casing, later part, past part.
const casing = new Line(CASING), later = new Line(LATER), past = new Line(PAST);
const restack = () => {
  for (const layer of later.layers()) layer.bringToBack();
  for (const layer of casing.layers()) layer.bringToBack();
};

// The few marks draw as SVG or HTML, which update at once: canvas redraws
// wait for an animation frame, which a covered window does not get.
const marks = L.svg();
const start = L.circleMarker([0, 0], {renderer:marks, radius:5, color:COLOR, weight:2, fillColor:'#fff', fillOpacity:1, interactive:false});
const end = L.circleMarker([0, 0], {renderer:marks, radius:5, color:'#fff', weight:2, fillColor:COLOR, fillOpacity:1, interactive:false});
const hovered = L.circleMarker([0, 0], {renderer:marks, radius:6, color:'#fff', weight:2, fillColor:COLOR, fillOpacity:1, interactive:false});
const here = L.marker([0, 0], {interactive:false, keyboard:false, zIndexOffset:1000, icon:L.divIcon({
  className:'here', iconSize:[24, 24],
  html:'<svg viewBox="-12 -12 24 24" width="24" height="24" aria-hidden="true"><path d="M0 -12v5M0 7v5M-12 0h5M7 0h5"/><circle r="6.5"/></svg>',
})});

const selectedView = () => describeSelected(points, selected, status);
// Points after the selected row's point draw faded; without a selection,
// or for a row past the points read, the whole track is past.
function splitAt() {
  const view = selectedView();
  return view.mark === 'exact' || view.mark === 'before' ? points.at(selected) : points.count - 1;
}

function draw() {
  const split = points.count ? splitAt() : -1;
  const key = `${points.revision}:${split}`;
  if (key === drawnKey) return;
  drawnKey = key;
  const added = [casing.set(0, split + 1), later.set(Math.max(0, split), points.count), past.set(0, split + 1)];
  if (added.some(Boolean)) restack();
  for (const [marker, i] of [[start, 0], [end, points.count - 1]]) {
    if (points.count) marker.setLatLng([points.lat[i], points.lon[i]]).addTo(map);
    else marker.remove();
  }
  ui.fit.disabled = !points.count;
}

function fit() {
  if (!points.count) return;
  let south = 90, north = -90, west = 180, east = -180;
  for (let i = 0; i < points.count; i++) {
    const lat = points.lat[i], lon = points.lon[i];
    if (lat < south) south = lat; if (lat > north) north = lat;
    if (lon < west) west = lon; if (lon > east) east = lon;
  }
  fitting = true;
  try {map.fitBounds([[south, west], [north, east]], {padding:[32, 32], maxZoom:17, animate:false});}
  finally {fitting = false;}
  lastFit = Date.now();
}

// Marks where the selected row was logged. `pan` brings the mark into view:
// for a row selected in the main window while following is on, and always
// for a point chosen here with the keys.
function showSelected(pan) {
  const view = selectedView();
  ui.selection.hidden = !view.text;
  ui.selection.textContent = view.text;
  ui.selection.classList.toggle('pending', view.mark === 'pending');
  if (view.mark === 'none') here.remove();
  else {
    const i = points.at(selected), at = [points.lat[i], points.lon[i]];
    here.setLatLng(at).addTo(map);
    here.getElement()?.classList.toggle('pending', view.mark === 'pending');
    if (pan && !map.getBounds().pad(-0.1).contains(at)) {
      // A view chosen for the selection is the user's; fitting stops.
      moved = true;
      map.panTo(at);
    }
  }
  draw();
}

function render() {
  const words = status ? describeTrack(status, settings) : {text:'', failed:false, note:null};
  ui.status.textContent = words.text;
  ui.status.title = words.text;
  ui.status.classList.toggle('failed', words.failed);
  const note = datasetId === null ? '메인 창에 열린 로그가 없습니다.' : words.note;
  ui.empty.hidden = !note;
  ui.empty.textContent = note ?? '';
  const alert = notice || describeTiles(failedTiles, settings.tiles);
  ui.alert.hidden = !alert;
  ui.alert.textContent = alert;
}

// Screen readers hear when reading ends or fails and what a choice
// selected, not every count on the way.
function announce(text) {
  if (text && text !== announced) ui.announce.textContent = announced = text;
}
// A short-lived note over the map, such as a pick the main window refused.
function tell(text) {
  notice = text;
  if (noticeTimer !== null) clearTimeout(noticeTimer);
  noticeTimer = setTimeout(() => {notice = '';noticeTimer = null;render();}, 5000);
  render();
  announce(text);
}

function reset() {
  generation++;
  points.clear();
  status = null;moved = false;lastFit = 0;cycle = null;
  hideHover();
  showSelected(false);
}

const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));

// Reads the points of dataset `id` as they are found, until reading ends
// or another log is shown.
async function follow(id, run) {
  while (run === generation) {
    let next;
    try {
      next = await api('status', {dataset:id});
      if (run !== generation) return;
      // A higher source's points replace a lower one's: a track never
      // mixes them.
      if (status && next.source !== status.source) {
        points.clear();cycle = null;
        hideHover();
      }
      // Points after `stable` may be read again (a followed log's last rows).
      if (points.count > next.stable) points.truncate(next.stable);
      if (next.count > points.count) {
        const response = await api('points', {dataset:id, source:next.source, from:points.count}, {raw:true});
        const triples = new Float64Array(await response.arrayBuffer());
        if (run !== generation) return;
        points.append(triples);
      }
    } catch (error) {
      if (run !== generation) return;
      // The main window shows another log; its watch starts that one.
      if (error.status === 409) return;
      status = {...status, phase:'failed', error:error.message};
      render();
      await sleep(2000);
      continue;
    }
    const first = (!status?.count || next.source !== status.source) && points.count > 0, ended = next.phase !== status?.phase && FINAL.has(next.phase);
    status = next;
    showSelected(false);
    render();
    if (ended) announce(describeTrack(status, settings).text);
    const done = FINAL.has(status.phase) && points.count === status.count;
    if (!moved && points.count && (first || done || Date.now() - lastFit > 2000)) fit();
    if (done) return;
    await sleep(status.phase === 'following' ? 1000 : 300);
  }
}

// Selects point `i`'s row in the main window; its selection comes back
// through the watch.
function choose(i, pan) {
  if (datasetId === null) return;
  picked = selected = points.lines[i];
  showSelected(pan);
  announce(ui.selection.textContent);
  locate(datasetId, picked).catch(error => tell(error.message));
}

// The passes of the track near a point of the map, in row order.
function near(point) {
  if (datasetId === null || !points.count) return [];
  const a = map.containerPointToLatLng([point.x - PICK_RADIUS, point.y + PICK_RADIUS]);
  const b = map.containerPointToLatLng([point.x + PICK_RADIUS, point.y - PICK_RADIUS]);
  const box = {south:Math.min(a.lat, b.lat), north:Math.max(a.lat, b.lat), west:Math.min(a.lng, b.lng), east:Math.max(a.lng, b.lng)};
  return points.passes(box, k => map.latLngToContainerPoint([points.lat[k], points.lon[k]]).distanceTo(point), PICK_RADIUS);
}
// The pass a click picks: the nearest one, or, clicking where the last
// click picked, the next one there in row order. Of passes about as near,
// the track's start or end wins, whose marks show them.
function nextPass(point, passes) {
  const again = cycle && cycle.count === passes.length && point.distanceTo(cycle.point) < 8;
  if (again) return (cycle.k + 1) % passes.length;
  const best = Math.min(...passes.map(pass => pass.distance));
  const nearest = passes.map((pass, k) => ({pass, k})).filter(({pass}) => pass.distance <= best + 1);
  const end = nearest.find(({pass}) => pass.index === 0 || pass.index === points.count - 1);
  return (end ?? nearest[0]).k;
}

function hideHover() {
  hovered.remove();
  ui.tip.hidden = true;
  map.getContainer().classList.remove('near-track');
}
function hover(point) {
  const passes = near(point);
  if (!passes.length) return hideHover();
  const k = nextPass(point, passes), i = passes[k].index;
  hovered.setLatLng([points.lat[i], points.lon[i]]).addTo(map);
  map.getContainer().classList.add('near-track');
  ui.tip.textContent = describePass(points, passes, k);
  ui.tip.hidden = false;
  const box = map.getSize(), width = ui.tip.offsetWidth;
  ui.tip.style.left = `${Math.max(4, Math.min(point.x + 14, box.x - width - 4))}px`;
  ui.tip.style.top = `${Math.max(4, point.y - 34)}px`;
}
let hoverAt = 0;
map.on('mousemove', event => {
  const now = performance.now();
  if (now - hoverAt < 40) return;
  hoverAt = now;
  hover(event.containerPoint);
});
map.on('mouseout', hideHover);
map.on('movestart', hideHover);
map.on('click', event => {
  const passes = near(event.containerPoint);
  if (!passes.length) return;
  const k = nextPass(event.containerPoint, passes);
  cycle = {point:event.containerPoint, count:passes.length, k};
  choose(passes[k].index, false);
  hover(event.containerPoint);
});

// [ and ] step to the previous and next point from the selected row.
function step(forward) {
  if (datasetId === null || !points.count) return;
  let i;
  if (selected === null) i = forward ? 0 : points.count - 1;
  else {
    const at = points.at(selected), exact = at >= 0 && points.lines[at] === selected;
    i = forward ? at + 1 : exact ? at - 1 : at;
  }
  if (i < 0 || i >= points.count) return tell(forward ? '마지막 지점입니다.' : '첫 지점입니다.');
  choose(i, true);
}
document.addEventListener('keydown', event => {
  if (event.metaKey || event.ctrlKey || event.altKey || event.isComposing) return;
  if (event.key !== '[' && event.key !== ']') return;
  event.preventDefault();
  step(event.key === ']');
});
ui.fit.addEventListener('click', () => {moved = false;fit();});
ui.follow.addEventListener('change', () => showSelected(ui.follow.checked));

async function main() {
  try {settings = await api('settings');}
  catch (error) {ui.status.textContent = error.message;ui.status.classList.add('failed');return;}
  ui.title.textContent = settings.title;
  const tiles = L.tileLayer(settings.tiles, {maxZoom:19, errorTileUrl:MISSING, attribution:settings.attribution.replace(/[&<>]/g, c => ({'&':'&amp;', '<':'&lt;', '>':'&gt;'})[c])}).addTo(map);
  // A failed tile is marked until it loads; tiles that leave the view, or
  // whose loading a zoom ends, leave the count with them. The hatched
  // stand-in loading is no tile.
  const count = () => {
    const failed = map.getContainer().querySelectorAll('.leaflet-tile[data-failed]').length;
    if (failed !== failedTiles) {failedTiles = failed;render();}
  };
  tiles.on('tileerror', event => {event.tile.dataset.failed = '1';count();});
  tiles.on('tileload', event => {if (!event.tile.src.startsWith('data:')) delete event.tile.dataset.failed;count();});
  tiles.on('tileunload tileabort load', count);
  watchMain(({datasetId:id, selected:line}) => {
    if (id !== datasetId) {
      datasetId = id;
      reset();
      if (id !== null) void follow(id, generation);
    }
    // A row chosen here comes back as the main window's selection; the
    // map already shows it.
    const own = line !== null && line === picked;
    picked = null;
    if (line === selected && !own) return;
    selected = line;
    showSelected(!own && ui.follow.checked);
    if (!own && line !== null) announce(ui.selection.textContent);
  });
  render();
}
void main();
