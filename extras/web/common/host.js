// The LogStudio host as an extension's page sees it (extras/src/lib.rs):
// the log and row the main window shows, showing a row there, and the
// extension's own requests. Paths are relative to the page, so they reach
// its own `extras` scheme on every platform.

// A failed request's message is the host's, which is shown as it is.
async function failed(response) {
  let message = '요청을 처리하지 못했습니다';
  try {message = (await response.json()).error || message;} catch {}
  return Object.assign(new Error(message), {status:response.status});
}
// The extension's `api/<path>` with whole-number parameters: its JSON, or
// the response itself for `{raw:true}`.
export async function api(path, params = {}, {raw = false, signal} = {}) {
  const query = new URLSearchParams(Object.entries(params).map(([k, v]) => [k, String(v)]));
  const response = await fetch(`api/${path}?${query}`, {signal, cache:'no-store'});
  if (!response.ok) throw await failed(response);
  return raw ? response : response.json();
}
// Selects row `line` of dataset `datasetId` in the main window. Rejects when
// the main window shows another log by now.
export async function locate(datasetId, line) {
  const query = new URLSearchParams({dataset:String(datasetId), line:String(line)});
  const response = await fetch(`host/locate?${query}`, {cache:'no-store'});
  if (!response.ok) throw await failed(response);
}
// Calls `onChange({datasetId, selected})` with the dataset the main window
// shows (null when none) and the physical row it selected there (null when
// none), now and after every change, until `stop()`.
export function watchMain(onChange) {
  let stopped = false, revision = null, datasetId = null, first = true;
  const controller = new AbortController();
  (async () => {
    while (!stopped) {
      try {
        const query = new URLSearchParams({dataset:String(datasetId ?? 0)});
        if (revision !== null) query.set('after', String(revision));
        const response = await fetch(`host/state?${query}`, {signal:controller.signal, cache:'no-store'});
        if (!response.ok) throw await failed(response);
        const state = await response.json();
        const changed = first || state.revision !== revision || (state.datasetId ?? null) !== datasetId;
        first = false;
        revision = state.revision;datasetId = state.datasetId ?? null;
        if (changed && !stopped) onChange({datasetId, selected:Number.isSafeInteger(state.selected) ? state.selected : null});
      } catch {
        if (stopped) return;
        await new Promise(resolve => setTimeout(resolve, 1000));
      }
    }
  })();
  return {stop() {stopped = true;controller.abort();}};
}
