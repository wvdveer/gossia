// The Gossia extension for VS Code (DESIGN.md, Web port, 0.3.0, In VS Code).
//
// A custom editor for `.gossia` files: VS Code opens, saves, reverts and backs
// up the file, and the panel shows the application compiled to wasm (the
// `gossia-web` crate, in media/pkg). The panel's host object (media/panel.js)
// sends here what a web view cannot do: the file's bytes, storage
// (globalState), secrets (SecretStorage), the open and save dialogs, opening
// something outside, and HTTP requests for data sources, made here so that
// CORS does not apply.
'use strict';

const vscode = require('vscode');
const crypto = require('crypto');
const fs = require('fs');
const path = require('path');

const VIEW_TYPE = 'gossia.workbook';

// A plain Uint8Array of its own, which VS Code's messages carry as bytes
// (a Node.js Buffer, as files are read, is a view into a larger buffer).
const plain = (bytes) => new Uint8Array(bytes);
// Names of the application's storage and secrets, under this prefix.
const PREFIX = 'gossia:';
// The names of the secrets kept, since SecretStorage cannot list them.
const SECRET_NAMES = 'gossia.secretNames';

/// One open workbook file, and its panel once it has one.
class WorkbookDocument {
  constructor(uri, bytes) {
    this.uri = uri;
    this.bytes = bytes;
    this.panel = null;
    this.started = false;
    // Promises waiting for the workbook's bytes, by kind (save, backup).
    this.waiting = { save: [], backup: [] };
    this.onStarted = [];
  }

  dispose() {
    for (const kind of Object.keys(this.waiting)) {
      for (const w of this.waiting[kind]) w.reject(new Error('the Gossia panel was closed'));
      this.waiting[kind] = [];
    }
  }

  /// The workbook's bytes from the application: `save` counts it as saved,
  /// `backup` does not.
  bytesFor(kind) {
    if (!this.panel || !this.started) {
      return Promise.reject(new Error('the Gossia panel has not started'));
    }
    return new Promise((resolve, reject) => {
      this.waiting[kind].push({ resolve, reject });
      this.panel.webview.postMessage({ type: 'requestBytes', kind });
    });
  }

  answer(kind, error, bytes) {
    const waiting = this.waiting[kind];
    this.waiting[kind] = [];
    for (const w of waiting) {
      if (error) w.reject(new Error(error));
      else w.resolve(bytes);
    }
    return waiting.length > 0;
  }
}

class WorkbookEditorProvider {
  constructor(context) {
    this.context = context;
    this.changes = new vscode.EventEmitter();
    this.onDidChangeCustomDocument = this.changes.event;
    // The documents with a panel, the latest last (for the tests' commands).
    this.documents = [];
    this.controls = new Map();
    this.nextControl = 1;
  }

  async openCustomDocument(uri, openContext) {
    // A backup (hot exit) is opened in place of the file; an untitled file,
    // or a new empty one, is a blank workbook.
    const from = openContext.backupId ? vscode.Uri.parse(openContext.backupId) : uri;
    let bytes = new Uint8Array();
    if (from.scheme !== 'untitled') bytes = await vscode.workspace.fs.readFile(from);
    return new WorkbookDocument(uri, bytes);
  }

  async resolveCustomEditor(document, panel) {
    document.panel = panel;
    this.documents.push(document);
    const media = vscode.Uri.joinPath(this.context.extensionUri, 'media');
    panel.webview.options = { enableScripts: true, localResourceRoots: [media] };
    panel.webview.html = this.html(panel.webview, media);
    panel.onDidDispose(() => {
      document.panel = null;
      this.documents = this.documents.filter((d) => d !== document);
    });
    panel.webview.onDidReceiveMessage((m) => this.received(document, m));
  }

  html(webview, media) {
    const nonce = crypto.randomBytes(16).toString('base64');
    const read = (name) => fs.readFileSync(path.join(this.context.extensionPath, 'media', name), 'utf8');
    const config = {
      glueUrl: webview.asWebviewUri(vscode.Uri.joinPath(media, 'pkg', 'gossia_web.js')).toString(),
      wasmUrl: webview.asWebviewUri(vscode.Uri.joinPath(media, 'pkg', 'gossia_web_bg.wasm')).toString(),
      workerText: read('worker.js'),
    };
    // `<` written as an escape, so the configuration cannot end the script.
    const json = JSON.stringify(config).replace(/</g, '\\u003c');
    const csp = [
      "default-src 'none'",
      `script-src 'nonce-${nonce}' blob: 'wasm-unsafe-eval'`,
      'worker-src blob:',
      `connect-src ${webview.cspSource}`,
      "style-src 'unsafe-inline'",
      `img-src ${webview.cspSource} data: blob:`,
      `font-src ${webview.cspSource} data:`,
    ].join('; ');
    return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="${csp}">
<style>
  html, body { margin: 0 !important; padding: 0 !important; height: 100%; overflow: hidden; }
  canvas { display: block; width: 100%; height: 100%; outline: none; }
  #loading { position: absolute; inset: 0; display: grid; place-items: center; white-space: pre-wrap;
             font-family: var(--vscode-font-family); color: var(--vscode-descriptionForeground); }
</style>
</head>
<body>
<canvas id="app"></canvas>
<div id="loading">Loading Gossia...</div>
<script nonce="${nonce}">const GOSSIA = ${json};</script>
<script nonce="${nonce}" type="module">${read('panel.js')}</script>
</body>
</html>`;
  }

  async received(document, m) {
    try {
      switch (m.type) {
        case 'ready':
          await this.init(document);
          break;
        case 'started':
          document.started = true;
          for (const f of document.onStarted.splice(0)) f(null);
          break;
        case 'failed':
          for (const f of document.onStarted.splice(0)) f(m.message);
          vscode.window.showErrorMessage(m.message);
          break;
        case 'dirty':
          // Undo stays the application's own, so VS Code is only told that
          // the workbook changed; it clears the mark when it saves.
          if (m.dirty) this.changes.fire({ document });
          break;
        case 'saveWorkbook':
          // Asked for by a save of VS Code's, or else the application's own
          // Save (its menu, Ctrl+S), which is VS Code's Save.
          if (!document.answer('save', null, m.bytes)) {
            await vscode.commands.executeCommand('workbench.action.files.save');
          }
          break;
        case 'backupWorkbook':
          document.answer('backup', null, m.bytes);
          break;
        case 'bytesFailed':
          document.answer(m.kind, m.message);
          break;
        case 'writeStorage':
          await this.context.globalState.update(PREFIX + m.name, m.text);
          break;
        case 'writeSecret':
          await this.writeSecret(m.name, m.text);
          break;
        case 'pickFile':
          await this.pickFile(document, m.purpose, m.extensions);
          break;
        case 'saveFile':
          await this.saveFile(document, m.name, m.bytes);
          break;
        case 'openOutside': {
          const target = /^[a-z][a-z0-9+.-]*:\/\//i.test(m.target)
            ? vscode.Uri.parse(m.target)
            : vscode.Uri.file(m.target);
          await vscode.env.openExternal(target);
          break;
        }
        case 'fetch':
          this.fetch(document, m);
          break;
        case 'controlAnswer': {
          const pending = this.controls.get(m.id);
          this.controls.delete(m.id);
          if (pending) pending(m);
          break;
        }
      }
    } catch (err) {
      vscode.window.showErrorMessage(`Gossia: ${err.message || err}`);
    }
  }

  /// The workbook, the storage and the secrets, for the panel to start with.
  async init(document) {
    const storage = {};
    for (const key of this.context.globalState.keys()) {
      if (key.startsWith(PREFIX)) storage[key.slice(PREFIX.length)] = this.context.globalState.get(key);
    }
    const secrets = {};
    for (const name of this.context.globalState.get(SECRET_NAMES, [])) {
      const text = await this.context.secrets.get(PREFIX + name);
      if (text !== undefined) secrets[name] = text;
    }
    document.panel.webview.postMessage({
      type: 'init',
      name: path.basename(document.uri.path),
      bytes: plain(document.bytes),
      storage,
      secrets,
    });
  }

  async writeSecret(name, text) {
    const names = new Set(this.context.globalState.get(SECRET_NAMES, []));
    if (text === null) {
      await this.context.secrets.delete(PREFIX + name);
      names.delete(name);
    } else {
      await this.context.secrets.store(PREFIX + name, text);
      names.add(name);
    }
    await this.context.globalState.update(SECRET_NAMES, [...names]);
  }

  /// The folder of the workbook, where the dialogs start.
  folderOf(document) {
    return document.uri.scheme === 'file' ? vscode.Uri.joinPath(document.uri, '..') : undefined;
  }

  async pickFile(document, purpose, extensions) {
    const chosen = await vscode.window.showOpenDialog({
      canSelectMany: false,
      defaultUri: this.folderOf(document),
      filters: extensions.length ? { Files: extensions, 'All files': ['*'] } : undefined,
    });
    if (!chosen || !chosen.length || !document.panel) return;
    const bytes = await vscode.workspace.fs.readFile(chosen[0]);
    document.panel.webview.postMessage({
      type: 'picked', purpose, name: path.basename(chosen[0].path), bytes: plain(bytes),
    });
  }

  async saveFile(document, name, bytes) {
    const folder = this.folderOf(document);
    const target = await vscode.window.showSaveDialog({
      defaultUri: folder ? vscode.Uri.joinPath(folder, name) : undefined,
    });
    if (target) await vscode.workspace.fs.writeFile(target, bytes);
  }

  /// An HTTP request for a data source: a redirect is given back as it is,
  /// and the body is read up to the limit.
  async fetch(document, m) {
    const shown = m.url.split('?')[0];
    const post = (message) => document.panel && document.panel.webview.postMessage(message);
    try {
      const headers = [];
      for (let i = 0; i + 1 < m.headers.length; i += 2) headers.push([m.headers[i], m.headers[i + 1]]);
      const reply = await fetch(m.url, {
        method: m.method,
        headers,
        body: m.body ?? undefined,
        redirect: 'manual',
        signal: AbortSignal.timeout(m.timeoutMs),
      });
      const chunks = [];
      let size = 0;
      if (reply.body) {
        for await (const chunk of reply.body) {
          size += chunk.length;
          if (size > m.maxBytes) {
            post({
              type: 'fetchFailed', id: m.id,
              message: `the response from ${shown} is larger than the maximum response size of ${m.maxBytes} bytes`,
            });
            return;
          }
          chunks.push(chunk);
        }
      }
      const body = new Uint8Array(size);
      let at = 0;
      for (const chunk of chunks) { body.set(chunk, at); at += chunk.length; }
      const flat = [];
      reply.headers.forEach((value, name) => flat.push(name, value));
      post({ type: 'fetched', id: m.id, status: reply.status, reason: reply.statusText, headers: flat, body });
    } catch (err) {
      const timedOut = err && (err.name === 'TimeoutError' || err.name === 'AbortError');
      const why = (err && err.cause && err.cause.message) || (err && err.message) || String(err);
      post({
        type: 'fetchFailed', id: m.id,
        message: timedOut ? `${shown} did not answer within ${m.timeoutMs / 1000} s` : `${shown} cannot be reached (${why})`,
      });
    }
  }

  async saveCustomDocument(document) {
    const bytes = await document.bytesFor('save');
    await vscode.workspace.fs.writeFile(document.uri, bytes);
  }

  async saveCustomDocumentAs(document, destination) {
    const bytes = await document.bytesFor('save');
    await vscode.workspace.fs.writeFile(destination, bytes);
  }

  async revertCustomDocument(document) {
    const bytes = await vscode.workspace.fs.readFile(document.uri);
    document.bytes = bytes;
    if (document.panel) {
      document.panel.webview.postMessage({ type: 'open', name: path.basename(document.uri.path), bytes: plain(bytes) });
    }
  }

  async backupCustomDocument(document, context) {
    const bytes = await document.bytesFor('backup');
    await vscode.workspace.fs.writeFile(context.destination, bytes);
    return {
      id: context.destination.toString(),
      delete: () => vscode.workspace.fs.delete(context.destination).then(undefined, () => {}),
    };
  }

  /// For the tests: waits until the latest panel has started.
  started() {
    const document = this.documents[this.documents.length - 1];
    if (!document) return Promise.reject(new Error('no Gossia panel is open'));
    if (document.started) return Promise.resolve();
    return new Promise((resolve, reject) => {
      document.onStarted.push((error) => (error ? reject(new Error(error)) : resolve()));
    });
  }

  /// For the tests: a command of the test control protocol, run in the
  /// latest panel (a test build only).
  control(cmd, args) {
    const document = this.documents[this.documents.length - 1];
    if (!document || !document.started) return Promise.reject(new Error('no Gossia panel has started'));
    const id = this.nextControl++;
    return new Promise((resolve, reject) => {
      this.controls.set(id, (m) => (m.ok ? resolve(m.result) : reject(new Error(`${cmd}: ${m.error}`))));
      document.panel.webview.postMessage({ type: 'control', id, cmd, args });
    });
  }
}

function activate(context) {
  const provider = new WorkbookEditorProvider(context);
  context.subscriptions.push(
    vscode.window.registerCustomEditorProvider(VIEW_TYPE, provider, {
      // The application keeps its state while its tab is hidden.
      webviewOptions: { retainContextWhenHidden: true },
      supportsMultipleEditorsPerDocument: false,
    }),
  );
  // The tests' commands, only when the tests start VS Code.
  if (process.env.GOSSIA_VSCODE_TEST) {
    context.subscriptions.push(
      vscode.commands.registerCommand('gossia.test.started', () => provider.started()),
      vscode.commands.registerCommand('gossia.test.control', (cmd, args) => provider.control(cmd, args)),
      vscode.commands.registerCommand('gossia.test.storage', (name) => context.globalState.get(PREFIX + name)),
      vscode.commands.registerCommand('gossia.test.setStorage', (name, text) =>
        context.globalState.update(PREFIX + name, text)),
      vscode.commands.registerCommand('gossia.test.secret', (name) => context.secrets.get(PREFIX + name)),
    );
  }
}

function deactivate() {}

module.exports = { activate, deactivate };
