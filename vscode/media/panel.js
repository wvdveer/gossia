// The Gossia panel in VS Code: the application drawn on a canvas, with a host
// object that hands what a web view cannot do to the extension host by
// messages (DESIGN.md, Web port, 0.3.0, In VS Code). extension.js puts this
// text in the panel's page, after `GOSSIA` (the addresses of the wasm module
// and its glue, and the worker's text).
//
// The extension sends `init` with the workbook, the storage and the secrets
// once the page says `ready`; storage and secrets are then answered here at
// once and written through to the extension host.

const vscode = acquireVsCodeApi();
const loading = document.getElementById('loading');

let gossia = null;
let wasmModule = null;
let workerUrl = null;
let worker = null;
const storage = new Map();
const secrets = new Map();

// Bytes from the extension host as a plain Uint8Array: what VS Code's
// messages give back for a Node.js Buffer is not always one.
function bytesOf(value) {
  if (value instanceof Uint8Array) return value;
  if (value instanceof ArrayBuffer) return new Uint8Array(value);
  if (value && typeof value === 'object') {
    if (value.buffer instanceof ArrayBuffer) {
      return new Uint8Array(value.buffer, value.byteOffset || 0, value.byteLength);
    }
    if (value.type === 'Buffer' && Array.isArray(value.data)) return Uint8Array.from(value.data);
    return Uint8Array.from(Object.values(value));
  }
  return new Uint8Array();
}

const host = {
  startWorker() {
    if (worker) worker.terminate();
    worker = new Worker(workerUrl, { type: 'module' });
    worker.postMessage({ module: wasmModule });
    worker.onmessage = (e) => {
      if (e.data instanceof Uint8Array) gossia.worker_message(e.data);
      else console.error('Gossia worker:', e.data);
    };
  },
  postWorker(bytes) { worker.postMessage(bytes, [bytes.buffer]); },
  // VS Code saves the file: these answer its requests (request_bytes), and a
  // save the application starts itself (its menu, Ctrl+S) asks VS Code to save.
  saveWorkbook(name, bytes) { vscode.postMessage({ type: 'saveWorkbook', name, bytes }); },
  backupWorkbook(bytes) { vscode.postMessage({ type: 'backupWorkbook', bytes }); },
  bytesFailed(kind, message) { vscode.postMessage({ type: 'bytesFailed', kind, message }); },
  setDirty(dirty) { vscode.postMessage({ type: 'dirty', dirty }); },
  pickFile(purpose, extensions) { vscode.postMessage({ type: 'pickFile', purpose, extensions }); },
  saveFile(name, bytes) { vscode.postMessage({ type: 'saveFile', name, bytes }); },
  readStorage(name) { return storage.has(name) ? storage.get(name) : null; },
  writeStorage(name, text) {
    storage.set(name, text);
    vscode.postMessage({ type: 'writeStorage', name, text });
  },
  readSecret(name) { return secrets.has(name) ? secrets.get(name) : null; },
  writeSecret(name, text) {
    if (text === null) secrets.delete(name);
    else secrets.set(name, text);
    vscode.postMessage({ type: 'writeSecret', name, text });
  },
  openOutside(target) { vscode.postMessage({ type: 'openOutside', target }); },
  // VS Code marks the body with the kind of its theme, and changes it when
  // the theme changes; the application looks each frame.
  dark() {
    const c = document.body.classList;
    return c.contains('vscode-dark')
      || (c.contains('vscode-high-contrast') && !c.contains('vscode-high-contrast-light'));
  },
  // The extension host makes the request, so CORS does not apply.
  fetch(id, method, url, headers, body, timeoutMs, maxBytes) {
    vscode.postMessage({ type: 'fetch', id, method, url, headers, body, timeoutMs, maxBytes });
  },
  // Modules are edited in VS Code's own editor, as `gossia:` files: having
  // openModule tells the application so.
  openModule(name, line, column) { vscode.postMessage({ type: 'openModule', name, line, column }); },
  modulesChanged(modules) { vscode.postMessage({ type: 'modulesChanged', modules }); },
  moduleAnswer(id, error) { vscode.postMessage({ type: 'moduleAnswer', id, error }); },
  // VS Code's display language.
  language() { return GOSSIA.language; },
  // The font for Japanese or Chinese, from beside the wasm, when the
  // application first needs it.
  loadFont(tag) {
    const url = GOSSIA.fonts[tag];
    if (!url) throw new Error(`no font for ${tag}`);
    fetch(url)
      .then((r) => {
        if (!r.ok) throw new Error(`${r.status} ${r.statusText}`);
        return r.arrayBuffer();
      })
      .then((b) => gossia.font_loaded(tag, new Uint8Array(b)))
      .catch((err) => gossia.font_failed(tag, String(err.message || err)));
  },
};

async function start(init) {
  try {
    // The glue is imported from a blob, and the worker made from the glue and
    // its own text, so that both load what the page fetched once.
    const glue = await (await fetch(GOSSIA.glueUrl)).text();
    const wasm = await fetch(GOSSIA.wasmUrl);
    wasmModule = await WebAssembly.compileStreaming(wasm);
    const blob = (text) => URL.createObjectURL(new Blob([text], { type: 'text/javascript' }));
    gossia = await import(blob(glue));
    workerUrl = blob(`${glue}\n${GOSSIA.workerText}`);
    await gossia.default({ module_or_path: wasmModule });
    for (const [name, text] of Object.entries(init.storage)) storage.set(name, text);
    for (const [name, text] of Object.entries(init.secrets)) secrets.set(name, text);
    await gossia.start_app(document.getElementById('app'), host, init.name, bytesOf(init.bytes), false);
    loading.remove();
    vscode.postMessage({ type: 'started', test: typeof gossia.control === 'function' });
  } catch (err) {
    let panic = null;
    try { panic = gossia && gossia.last_panic(); } catch (_) { /* unusable */ }
    const text = GOSSIA.texts.failed.replace('{0}', `${err}${panic ? '\n' + panic : ''}`);
    loading.textContent = text;
    vscode.postMessage({ type: 'failed', message: text });
  }
}

// The test build's commands, for the extension's tests.
async function control(id, cmd, args) {
  try {
    const result = JSON.parse(await gossia.control(cmd, JSON.stringify(args ?? null)));
    vscode.postMessage({ type: 'controlAnswer', id, ok: true, result });
  } catch (err) {
    vscode.postMessage({ type: 'controlAnswer', id, ok: false, error: String(err) });
  }
}

window.addEventListener('message', (e) => {
  const m = e.data;
  switch (m.type) {
    case 'init': start(m); break;
    case 'open': gossia.open_workbook(m.name, bytesOf(m.bytes)); break;
    case 'requestBytes': gossia.request_bytes(m.kind); break;
    case 'picked': gossia.file_picked(m.purpose, m.name, bytesOf(m.bytes)); break;
    case 'fetched': gossia.fetch_answered(m.id, m.status, m.reason, m.headers, bytesOf(m.body)); break;
    case 'fetchFailed': gossia.fetch_failed(m.id, m.message); break;
    case 'control': control(m.id, m.cmd, m.args); break;
    case 'command': gossia.run_command(m.name); break;
    case 'writeModule': gossia.module_written(m.id, m.name, m.text); break;
    case 'renameModule': gossia.module_renamed(m.id, m.from, m.to); break;
    case 'removeModule': gossia.module_removed(m.id, m.name); break;
  }
});

vscode.postMessage({ type: 'ready' });
