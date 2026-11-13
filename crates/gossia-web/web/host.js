// The browser host (DESIGN.md, The web version on GitHub Pages, 17a): what
// the page gives the application behind the host interface (see page.rs in
// gossia-web). Workbook files through the File System Access API where the
// browser has it (Chrome, Edge), saved again in place; elsewhere chosen or
// dropped, and saved by downloading. A list of recent workbooks with a copy
// of each in IndexedDB. Preferences and the like in localStorage, secrets
// for the session unless one is remembered, and HTTP with fetch, so that
// CORS applies.
//
// `record` is what the tests look at and answer through (window.testHost):
// with `record.answerPicks` set, files are neither chosen nor written, and
// the tests answer instead.

const STORAGE = 'gossia:';
const SECRET = 'gossia-secret:';
const RECENT_MAX = 10;
const TYPES = [{ description: 'Gossia workbook', accept: { 'application/octet-stream': ['.gossia'] } }];
// Opening also takes share files (DESIGN.md, 17e), which open as new
// workbooks; saving writes workbooks only.
const OPEN_TYPES = [
  { description: 'Gossia workbook or share file', accept: { 'application/octet-stream': ['.gossia'], 'text/html': ['.html', '.htm'] } },
];
const ACCESS = typeof window.showOpenFilePicker === 'function'
  && typeof window.showSaveFilePicker === 'function';

// ------------------------------------------------------------- IndexedDB

// The recent workbooks: `recent` holds each one's key, name, file handle
// (where the browser has them) and time; `copies` its file, by key.
function openDb() {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open('gossia', 1);
    req.onupgradeneeded = () => {
      req.result.createObjectStore('recent', { keyPath: 'key' });
      req.result.createObjectStore('copies');
    };
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

function request(req) {
  return new Promise((resolve, reject) => {
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

function store(db, name, mode = 'readonly') {
  return db.transaction(name, mode).objectStore(name);
}

// --------------------------------------------------------------- helpers

function newKey() {
  return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
}

// Whether an address is on this computer or a private network, which the
// browser keeps a public page from reaching.
function isPrivate(href) {
  let host;
  try {
    host = new URL(href).hostname.replace(/^\[|\]$/g, '').toLowerCase();
  } catch (_) {
    return false;
  }
  if (host === 'localhost' || host.endsWith('.localhost') || host.endsWith('.local')) return true;
  if (host === '::1' || /^f[cd]/.test(host) || host.startsWith('fe80:')) return true;
  const ip = host.match(/^(\d+)\.(\d+)\.(\d+)\.(\d+)$/);
  if (!ip) return false;
  const [a, b] = [Number(ip[1]), Number(ip[2])];
  return a === 127 || a === 10 || (a === 192 && b === 168) || (a === 172 && b >= 16 && b <= 31)
    || (a === 169 && b === 254);
}

async function writeHandle(handle, bytes) {
  const writable = await handle.createWritable();
  await writable.write(bytes);
  await writable.close();
}

// Whether the browser already lets the page read (or write) the file. The
// page never asks (requestPermission), which shows the browser's own prompt
// (DESIGN.md, 17a): without it, a save goes through the save dialog and a
// recent workbook opens from its copy.
async function granted(handle, mode) {
  try {
    return (await handle.queryPermission({ mode })) === 'granted';
  } catch (_) {
    return false;
  }
}

function download(name, bytes) {
  const url = URL.createObjectURL(new Blob([bytes]));
  const a = document.createElement('a');
  a.href = url;
  a.download = name;
  a.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

// ------------------------------------------------------------------ host

/// Makes the browser host. `gossia` is the module's exports. Reads the list
/// of recent workbooks first, since the menu asks for it without waiting.
export async function browserHost(gossia, record) {
  const {
    worker_message, file_picked, fetch_answered, fetch_problem, workbook_saved,
    workbook_not_saved,
  } = gossia;

  let db = null;
  let recent = [];
  try {
    db = await openDb();
    recent = await request(store(db, 'recent').getAll());
    recent.sort((x, y) => y.time - x.time);
  } catch (err) {
    console.error(`the recent workbooks cannot be kept: ${err}`);
  }

  // The workbook shown: its key in the recent list, name and file handle.
  let current = null;
  // The workbook last given to the application to open, until it says it
  // opened it.
  let pending = null;
  let dirty = false;
  let worker = null;
  const sessionSecrets = new Map();
  // Secrets to remember, from when that was chosen until they are written.
  const remember = new Set();

  function title(name) {
    document.title = name ? `${name} - Gossia` : 'Gossia';
  }

  // Puts a workbook at the top of the recent list, with a copy of its file.
  async function keepRecent(entry, bytes) {
    const old = recent.find((r) => r.key === entry.key);
    // A copy opened without its file keeps the handle it had.
    const kept = { key: entry.key, name: entry.name, handle: entry.handle || old?.handle || null,
                   time: Date.now() };
    recent = [kept, ...recent.filter((r) => r.key !== entry.key)];
    const gone = recent.slice(RECENT_MAX);
    recent = recent.slice(0, RECENT_MAX);
    if (!db) return;
    try {
      const tx = db.transaction(['recent', 'copies'], 'readwrite');
      tx.objectStore('recent').put(kept);
      tx.objectStore('copies').put(bytes, kept.key);
      for (const g of gone) {
        tx.objectStore('recent').delete(g.key);
        tx.objectStore('copies').delete(g.key);
      }
      await new Promise((resolve, reject) => {
        tx.oncomplete = resolve;
        tx.onerror = () => reject(tx.error);
      });
    } catch (err) {
      console.error(`the copy of ${entry.name} cannot be kept: ${err}`);
    }
  }

  // The key of the recent workbook whose file `handle` is, if one is.
  async function keyOf(handle) {
    for (const r of recent) {
      try {
        if (r.handle && (await r.handle.isSameEntry(handle))) return r.key;
      } catch (_) { /* a handle of another kind */ }
    }
    return null;
  }

  async function save(name, bytes, asNew) {
    record.saves.push({ name, bytes, asNew });
    try {
      let entry;
      if (record.answerPicks) {
        // The tests: kept, as a download would be.
        entry = { key: asNew || !current ? newKey() : current.key, name, handle: null };
      } else if (!asNew && current?.handle && (await granted(current.handle, 'readwrite'))) {
        await writeHandle(current.handle, bytes);
        entry = current;
      } else if (ACCESS) {
        // In the folder of the file the workbook came from, if it has one;
        // the file chosen there may be written again without asking.
        const options = { suggestedName: name, types: TYPES };
        if (current?.handle) options.startIn = current.handle;
        const handle = await window.showSaveFilePicker(options);
        await writeHandle(handle, bytes);
        entry = { key: (await keyOf(handle)) || newKey(), name: handle.name, handle };
      } else {
        download(name, bytes);
        entry = { key: asNew || !current ? newKey() : current.key, name, handle: null };
      }
      current = entry;
      title(entry.name);
      await keepRecent(entry, bytes);
      workbook_saved(entry.name);
    } catch (err) {
      if (err?.name === 'AbortError') workbook_not_saved(null);
      else workbook_not_saved(String(err?.message || err));
    }
  }

  // An HTTP request for a data source, with the browser's fetch: a redirect
  // is not followed, and the site has to allow the page's origin (CORS)
  // unless it is the page's own. What goes wrong is said by its kind, in
  // Gossia's words (fetch_problem).
  async function makeRequest(id, method, url, headers, body, timeoutMs, maxBytes) {
    const shown = url.split('?')[0];
    const abort = new AbortController();
    const timer = setTimeout(() => abort.abort(), timeoutMs);
    try {
      const pairs = [];
      for (let i = 0; i + 1 < headers.length; i += 2) pairs.push([headers[i], headers[i + 1]]);
      const reply = await fetch(url, {
        method, headers: pairs, body: body ?? undefined, redirect: 'manual',
        signal: abort.signal, credentials: 'omit', cache: 'no-store',
      });
      if (reply.type === 'opaqueredirect') {
        fetch_problem(id, 'redirect', shown, '');
        return;
      }
      const bytes = new Uint8Array(await reply.arrayBuffer());
      if (bytes.length > maxBytes) {
        fetch_problem(id, 'too-large', shown, String(maxBytes));
        return;
      }
      const flat = [];
      reply.headers.forEach((value, name) => flat.push(name, value));
      fetch_answered(id, reply.status, reply.statusText, flat, bytes);
    } catch (err) {
      if (abort.signal.aborted) {
        fetch_problem(id, 'timeout', shown, String(timeoutMs / 1000));
        return;
      }
      // fetch says the same for a site that refuses this page's origin and
      // one that cannot be reached. A request the page may not read the
      // answer of tells them apart: it gets through only to a site there.
      let reachable = false;
      try {
        await fetch(url, { method: 'HEAD', mode: 'no-cors', credentials: 'omit', cache: 'no-store',
                           signal: AbortSignal.timeout(timeoutMs) });
        reachable = true;
      } catch (_) { /* not there, or kept from the page */ }
      const kind = reachable ? 'cors'
        : isPrivate(url) && !isPrivate(location.href) ? 'private' : 'unreachable';
      fetch_problem(id, kind, shown, String(err?.message || err));
    } finally {
      clearTimeout(timer);
    }
  }

  window.addEventListener('beforeunload', (e) => {
    if (dirty) {
      e.preventDefault();
      e.returnValue = '';
    }
  });

  const host = {
    keepsFiles() { return true; },
    // A phone's or a tablet's browser: the larger targets for touch from the
    // start (DESIGN.md, 17d).
    coarsePointer() { return window.matchMedia('(pointer: coarse)').matches; },
    startWorker() {
      if (worker) worker.terminate();
      worker = new Worker('./worker.js', { type: 'module' });
      worker.onmessage = (e) => {
        if (e.data instanceof Uint8Array) worker_message(e.data);
        else record.workerErrors.push(e.data);
      };
      record.workers++;
    },
    postWorker(bytes) { worker.postMessage(bytes, [bytes.buffer]); },
    saveWorkbook(name, bytes) { save(name, bytes, false); },
    saveWorkbookAs(name, bytes) { save(name, bytes, true); },
    setDirty(value) {
      dirty = value;
      record.dirty = value;
    },
    workbookOpened(name, bytes, picked) {
      const from = picked && pending ? pending : { key: null, handle: null };
      pending = null;
      current = { key: from.key || newKey(), name, handle: from.handle || null };
      title(name);
      keepRecent(current, bytes);
      record.workbooks.push(name);
    },
    newWorkbook() {
      current = null;
      pending = null;
      title(null);
    },
    recentWorkbooks() {
      return recent.flatMap((r) => [r.key, r.name]);
    },
    async openRecent(key) {
      const entry = recent.find((r) => r.key === key);
      if (!entry) return;
      // The file itself where the browser still lets the page read it, or
      // else the copy, which saves through the save dialog.
      if (entry.handle && !record.answerPicks) {
        try {
          if (await granted(entry.handle, 'read')) {
            const file = await entry.handle.getFile();
            pending = { key, handle: entry.handle };
            file_picked('open', file.name, new Uint8Array(await file.arrayBuffer()));
            return;
          }
        } catch (_) { /* moved, deleted, or refused: the copy instead */ }
      }
      try {
        const bytes = db && (await request(store(db, 'copies').get(key)));
        if (!bytes) throw new Error('no copy is kept');
        pending = { key, handle: entry.handle || null };
        file_picked('open', entry.name, bytes);
      } catch (err) {
        console.error(`${entry.name} cannot be opened: ${err}`);
      }
    },
    forgetRecent(key) {
      const gone = key === null ? recent : recent.filter((r) => r.key === key);
      recent = recent.filter((r) => !gone.includes(r));
      if (!db) return;
      const tx = db.transaction(['recent', 'copies'], 'readwrite');
      for (const g of gone) {
        tx.objectStore('recent').delete(g.key);
        tx.objectStore('copies').delete(g.key);
      }
    },
    // The browser's own file chooser; the tests answer through
    // window.testHost.answerPick(name, bytes) instead. A workbook to open is
    // chosen with the File System Access API where there is one, so that it
    // is saved again in place.
    async pickFile(purpose, extensions) {
      record.picks.push({ purpose, extensions });
      if (record.answerPicks) {
        if (purpose === 'open') pending = { key: null, handle: null };
        return;
      }
      if (purpose === 'open' && ACCESS) {
        try {
          const [handle] = await window.showOpenFilePicker({ types: OPEN_TYPES });
          const file = await handle.getFile();
          pending = { key: await keyOf(handle), handle };
          file_picked(purpose, file.name, new Uint8Array(await file.arrayBuffer()));
        } catch (err) {
          if (err?.name !== 'AbortError') console.error(`the workbook cannot be read: ${err}`);
        }
        return;
      }
      const input = document.createElement('input');
      input.type = 'file';
      if (extensions.length) input.accept = extensions.map((e) => '.' + e).join(',');
      input.onchange = async () => {
        const file = input.files[0];
        if (!file) return;
        if (purpose === 'open') pending = { key: null, handle: null };
        file_picked(purpose, file.name, new Uint8Array(await file.arrayBuffer()));
      };
      input.click();
    },
    // Exports and the like: downloaded, as the browser saves files.
    saveFile(name, bytes) {
      record.files.push({ name, bytes });
      if (!record.answerPicks) download(name, bytes);
    },
    // Secrets: for the session in the page's memory, or remembered in
    // localStorage when the user chose that for the data source.
    secretsPerSession() { return true; },
    secretRemembered(name) {
      return remember.has(name) || localStorage.getItem(SECRET + name) !== null;
    },
    rememberSecret(name, on) {
      if (on) {
        remember.add(name);
        const text = sessionSecrets.get(name);
        if (text !== undefined) {
          localStorage.setItem(SECRET + name, text);
          sessionSecrets.delete(name);
        }
      } else {
        remember.delete(name);
        const text = localStorage.getItem(SECRET + name);
        if (text !== null) {
          sessionSecrets.set(name, text);
          localStorage.removeItem(SECRET + name);
        }
      }
    },
    readSecret(name) {
      return sessionSecrets.get(name) ?? localStorage.getItem(SECRET + name);
    },
    writeSecret(name, text) {
      if (text === null) {
        sessionSecrets.delete(name);
        localStorage.removeItem(SECRET + name);
        remember.delete(name);
      } else if (host.secretRemembered(name)) {
        localStorage.setItem(SECRET + name, text);
      } else {
        sessionSecrets.set(name, text);
      }
    },
    readStorage(name) { return localStorage.getItem(STORAGE + name); },
    writeStorage(name, text) { localStorage.setItem(STORAGE + name, text); },
    openOutside(target) {
      record.opened.push(target);
      window.open(target, '_blank', 'noopener');
    },
    dark() { return window.matchMedia('(prefers-color-scheme: dark)').matches; },
    fetch(id, method, url, headers, body, timeoutMs, maxBytes) {
      record.fetches.push({ id, method, url, headers, body });
      if (record.answerFetches) return;
      makeRequest(id, method, url, headers, body, timeoutMs, maxBytes);
    },
  };
  return host;
}
