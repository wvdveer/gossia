// The Gossia extension for VS Code (DESIGN.md, Web port, 0.3.0, In VS Code).
//
// A custom editor for `.gossia` files: VS Code opens, saves, reverts and backs
// up the file, and the panel shows the application compiled to wasm (the
// `gossia-web` crate, in media/pkg). The panel's host object (media/panel.js)
// sends here what a web view cannot do: the file's bytes, storage
// (globalState), secrets (SecretStorage), the open and save dialogs, opening
// something outside, and HTTP requests for data sources, made here so that
// CORS does not apply.
//
// Modules open in VS Code's own editor, as files of a `gossia:` file system
// (gossia://Book.gossia/Tax.py): the panel says what the modules are each
// time they change, and saving such a file puts the module in the workbook.
// Gossia's commands are in the Command Palette, run in the active workbook.
'use strict';

const vscode = require('vscode');
const crypto = require('crypto');
const fs = require('fs');
const path = require('path');

const VIEW_TYPE = 'gossia.workbook';
// The scheme of the modules' files.
const SCHEME = 'gossia';

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
    // The modules as the panel last said, by name: their source and when it
    // last changed; and the folder of the `gossia:` file system they are in.
    this.modules = new Map();
    this.key = null;
    // Module edits waiting for the panel's answer, by id.
    this.moduleEdits = new Map();
  }

  dispose() {
    for (const done of this.moduleEdits.values()) done('the Gossia panel was closed');
    this.moduleEdits.clear();
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

/// The modules of the open workbooks as files, a folder for each workbook
/// (DESIGN.md, Web port, 0.3.0, In VS Code). Reading answers from what the
/// panel last said; writing, renaming and deleting ask the panel, which makes
/// the change in the workbook (one entry in its history) and answers.
class ModuleFileSystem {
  constructor(provider) {
    this.provider = provider;
    this.emitter = new vscode.EventEmitter();
    this.onDidChangeFile = this.emitter.event;
    this.nextEdit = 1;
  }

  /// The workbook and the module a file is.
  place(uri) {
    const document = this.provider.byKey(uri.authority);
    if (!document) throw vscode.FileSystemError.FileNotFound(uri);
    const file = uri.path.replace(/^\/+/, '');
    return { document, file, name: file.replace(/\.py$/, '') };
  }

  watch() {
    return new vscode.Disposable(() => {});
  }

  stat(uri) {
    const { document, file, name } = this.place(uri);
    if (file === '') return { type: vscode.FileType.Directory, ctime: 0, mtime: 0, size: 0 };
    const module = file.endsWith('.py') && document.modules.get(name);
    if (!module) throw vscode.FileSystemError.FileNotFound(uri);
    return {
      type: vscode.FileType.File, ctime: module.changed, mtime: module.changed,
      size: Buffer.byteLength(module.text),
    };
  }

  readDirectory(uri) {
    const { document } = this.place(uri);
    return [...document.modules.keys()].map((name) => [`${name}.py`, vscode.FileType.File]);
  }

  readFile(uri) {
    const { document, file, name } = this.place(uri);
    const module = file.endsWith('.py') && document.modules.get(name);
    if (!module) throw vscode.FileSystemError.FileNotFound(uri);
    return new TextEncoder().encode(module.text);
  }

  async writeFile(uri, content) {
    const { document, file, name } = this.place(uri);
    if (!file.endsWith('.py') || file.includes('/')) {
      throw vscode.FileSystemError.NoPermissions('a workbook holds only modules: NAME.py');
    }
    const text = new TextDecoder().decode(content);
    await this.edit(document, { type: 'writeModule', name, text });
    // Known at once, so that VS Code finds the file as it wrote it; the
    // panel's word on the modules follows.
    const list = [...document.modules].map(([n, m]) => [n, m.text]).filter(([n]) => n !== name);
    this.provider.modulesSaid(document, list.concat([[name, text]]), false);
  }

  async rename(from, to) {
    const old = this.place(from);
    const target = this.place(to);
    if (old.document !== target.document || !target.file.endsWith('.py')) {
      throw vscode.FileSystemError.NoPermissions('a module is renamed within its workbook, as NAME.py');
    }
    await this.edit(old.document, { type: 'renameModule', from: old.name, to: target.name });
  }

  async delete(uri) {
    const { document, name } = this.place(uri);
    await this.edit(document, { type: 'removeModule', name });
  }

  createDirectory() {
    throw vscode.FileSystemError.NoPermissions('a workbook holds no folders');
  }

  /// Asks the panel to make a module edit, and waits for its answer.
  edit(document, message) {
    if (!document.panel || !document.started) {
      return Promise.reject(vscode.FileSystemError.Unavailable('the Gossia panel has not started'));
    }
    const id = this.nextEdit++;
    return new Promise((resolve, reject) => {
      document.moduleEdits.set(id, (error) => (error
        ? reject(vscode.FileSystemError.NoPermissions(error))
        : resolve()));
      document.panel.webview.postMessage({ ...message, id });
    });
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
    // The workbook whose panel was active last, for the Command Palette.
    this.active = null;
    this.files = new ModuleFileSystem(this);
  }

  /// The folder of a workbook's modules: its file's name, with a number
  /// after it when another open workbook has that name. VS Code writes the
  /// authority of an address in lower case, so it is compared so.
  keyFor(document) {
    const base = (path.basename(document.uri.path) || 'Untitled.gossia').toLowerCase();
    let key = base;
    for (let n = 2; this.byKey(key, document); n++) key = `${base}~${n}`;
    return key;
  }

  /// The open workbook whose modules are in the folder `key` (other than
  /// `except`).
  byKey(key, except) {
    const wanted = key.toLowerCase();
    return this.documents.find((d) => d !== except && d.key === wanted);
  }

  /// The file of a module of a workbook.
  moduleUri(document, name) {
    return vscode.Uri.from({ scheme: SCHEME, authority: document.key, path: `/${name}.py` });
  }

  /// The modules as the panel says they are now: the files that changed,
  /// came or went are announced, so that VS Code shows them as they are
  /// (after an undo in the workbook, say).
  modulesSaid(document, list, announce = true) {
    const now = Date.now();
    const events = [];
    const seen = new Set();
    for (const [name, text] of list) {
      seen.add(name);
      const old = document.modules.get(name);
      if (old && old.text === text) continue;
      document.modules.set(name, { text, changed: now });
      events.push({
        type: old ? vscode.FileChangeType.Changed : vscode.FileChangeType.Created,
        uri: this.moduleUri(document, name),
      });
    }
    for (const name of [...document.modules.keys()]) {
      if (seen.has(name)) continue;
      document.modules.delete(name);
      events.push({ type: vscode.FileChangeType.Deleted, uri: this.moduleUri(document, name) });
    }
    if (announce && events.length) this.files.emitter.fire(events);
  }

  /// The workbook the Command Palette's commands are for: the one whose
  /// module is being edited, or else whose panel was active last.
  target() {
    const editor = vscode.window.activeTextEditor;
    if (editor && editor.document.uri.scheme === SCHEME) {
      const document = this.byKey(editor.document.uri.authority);
      if (document) return document;
    }
    if (this.active && this.documents.includes(this.active)) return this.active;
    return this.documents[this.documents.length - 1];
  }

  /// Runs one of Gossia's commands in the target workbook.
  command(name) {
    const document = this.target();
    if (!document || !document.started) {
      vscode.window.showInformationMessage('Open a Gossia workbook first.');
      return;
    }
    document.panel.reveal(undefined, false);
    document.panel.webview.postMessage({ type: 'command', name });
  }

  /// *Gossia: Open Module...*: a module of the target workbook, chosen from
  /// a list, in VS Code's editor.
  async pickModule() {
    const document = this.target();
    if (!document || !document.started) {
      vscode.window.showInformationMessage('Open a Gossia workbook first.');
      return;
    }
    const names = [...document.modules.keys()];
    if (!names.length) {
      vscode.window.showInformationMessage(
        'This workbook has no modules: add one with Insert > Module in its code editor.');
      return;
    }
    const name = await vscode.window.showQuickPick(names, { placeHolder: 'The module to open' });
    if (name) await this.openModule(document, name, 1, 1);
  }

  /// Opens a module of a workbook in VS Code's editor, beside the workbook,
  /// at a line and column (1-based).
  async openModule(document, name, line, column) {
    const at = new vscode.Position(Math.max(0, line - 1), Math.max(0, column - 1));
    await vscode.window.showTextDocument(this.moduleUri(document, name), {
      viewColumn: vscode.ViewColumn.Beside,
      selection: new vscode.Range(at, at),
      preview: false,
    });
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
    document.key = this.keyFor(document);
    this.documents.push(document);
    this.active = document;
    panel.onDidChangeViewState((e) => {
      if (e.webviewPanel.active) this.active = document;
    });
    const media = vscode.Uri.joinPath(this.context.extensionUri, 'media');
    panel.webview.options = { enableScripts: true, localResourceRoots: [media] };
    panel.webview.html = this.html(panel.webview, media);
    panel.onDidDispose(() => {
      document.panel = null;
      document.dispose();
      // Its modules' files go with it.
      this.modulesSaid(document, []);
      this.documents = this.documents.filter((d) => d !== document);
      if (this.active === document) this.active = null;
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
        case 'modulesChanged':
          this.modulesSaid(document, m.modules);
          break;
        case 'openModule':
          await this.openModule(document, m.name, m.line, m.column);
          break;
        case 'moduleAnswer': {
          const done = document.moduleEdits.get(m.id);
          document.moduleEdits.delete(m.id);
          if (done) done(m.error);
          break;
        }
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
    vscode.workspace.registerFileSystemProvider(SCHEME, provider.files, { isCaseSensitive: true }),
    vscode.commands.registerCommand('gossia.openModule', () => provider.pickModule()),
  );
  // Gossia's own commands (gossia_app::commands), as package.json lists them.
  for (const { command } of require('./package.json').contributes.commands) {
    const name = command.slice('gossia.'.length);
    if (name === 'openModule') continue;
    context.subscriptions.push(vscode.commands.registerCommand(command, () => provider.command(name)));
  }
  // The tests' commands, only when the tests start VS Code.
  if (process.env.GOSSIA_VSCODE_TEST) {
    context.subscriptions.push(
      vscode.commands.registerCommand('gossia.test.started', () => provider.started()),
      vscode.commands.registerCommand('gossia.test.control', (cmd, args) => provider.control(cmd, args)),
      vscode.commands.registerCommand('gossia.test.storage', (name) => context.globalState.get(PREFIX + name)),
      vscode.commands.registerCommand('gossia.test.setStorage', (name, text) =>
        context.globalState.update(PREFIX + name, text)),
      vscode.commands.registerCommand('gossia.test.secret', (name) => context.secrets.get(PREFIX + name)),
      vscode.commands.registerCommand('gossia.test.moduleUri', (name) => {
        const document = provider.documents[provider.documents.length - 1];
        return document ? provider.moduleUri(document, name).toString() : null;
      }),
    );
  }
}

function deactivate() {}

module.exports = { activate, deactivate };
