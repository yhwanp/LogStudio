// Stands in for the Tauri globals when web_preview serves the page: invoke
// posts to the preview server and events arrive on one EventSource. A failed
// command rejects with its message string, as Tauri's invoke does. The page
// ID lets the server keep one page on the engine, as the desktop has one window.
(() => {
  const page = Math.random().toString(36).slice(2) + Date.now().toString(36);
  const events = new EventSource(`./__preview/events?page=${page}`);
  window.__TAURI__ = {
    core: {
      async invoke(cmd, args = {}) {
        const response = await fetch('./__preview/invoke', {
          method: 'POST',
          headers: {'content-type': 'application/json'},
          body: JSON.stringify({page, cmd, args}),
        });
        const body = await response.json();
        if ('error' in body) throw body.error;
        return body.ok;
      },
    },
    event: {
      async listen(name, handler) {
        const relay = message => handler({event: name, payload: JSON.parse(message.data)});
        events.addEventListener(name, relay);
        return () => events.removeEventListener(name, relay);
      },
    },
    webview: {
      getCurrentWebview: () => ({
        async setZoom(level) { document.documentElement.style.zoom = String(level); },
      }),
    },
  };
})();
