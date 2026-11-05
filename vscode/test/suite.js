// The extension's tests, run inside VS Code by test/run.js. They open a
// workbook in the custom editor and drive the application in its panel with
// the commands of the test control protocol (the test build's), through the
// extension's test commands; and they use VS Code's own commands for what VS
// Code does: Save, Revert, closing and opening the editor.
'use strict';

const fs = require('fs');
const http = require('http');
const path = require('path');
const vscode = require('vscode');

const ORDERS = JSON.stringify({
  customer: { name: 'Acme Ltd', city: 'Delft' },
  items: [
    { sku: 'A1', qty: 2, price: 9.5 },
    { sku: 'B2', qty: 1, price: 24.5 },
    { sku: 'C3', qty: 12, price: 0.75 },
  ],
  tax_rate: 0.21,
  subtotal: 0.5,
});

const results = [];

function check(ok, desc, detail) {
  results.push({ ok: !!ok, desc, detail });
  console.log(`  ${ok ? 'ok' : 'FAIL'} - ${desc}${!ok && detail !== undefined ? ` (${detail})` : ''}`);
}

function equal(actual, expected, desc) {
  const ok = JSON.stringify(actual) === JSON.stringify(expected);
  check(ok, desc, `expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`);
}

// Waits until `predicate` gives something true; one that throws (the panel
// is still opening the file) is asked again.
async function waitFor(predicate, label, timeout = 10000) {
  const start = Date.now();
  const truthy = async () => {
    try { return await predicate(); } catch (_) { return false; }
  };
  while (!(await truthy())) {
    if (Date.now() - start > timeout) throw new Error(`timed out waiting for ${label}`);
    await new Promise((r) => setTimeout(r, 50));
  }
}

const command = (...args) => vscode.commands.executeCommand(...args);
const send = (cmd, args = null) => command('gossia.test.control', cmd, args);
const idle = () => send('wait_idle', { timeout_ms: 60000 });
const value = async (path) => (await send('cell', { path })).value;
const activeTab = () => vscode.window.tabGroups.activeTabGroup.activeTab;

async function open(uri) {
  await command('vscode.openWith', uri, 'gossia.workbook');
  await command('gossia.test.started');
  await idle();
}

// A server on this machine that sends no CORS headers: a page could not read
// it, the extension host can.
function startServer() {
  return new Promise((resolve) => {
    const server = http.createServer((req, res) => {
      if (req.url === '/n.json') {
        res.writeHead(200, { 'content-type': 'application/json' });
        res.end('{"n": 41}');
      } else {
        res.writeHead(404);
        res.end('not here');
      }
    });
    server.listen(0, '127.0.0.1', () => resolve(server));
  });
}

async function suite(dir) {
  // The extension activates when a workbook opens; its test commands are
  // wanted before that.
  await vscode.extensions.getExtension('wvdveer.gossia').activate();
  const server = await startServer();
  const port = server.address().port;
  // An HTTP source, kept as the data sources panel keeps it.
  await command('gossia.test.setStorage', 'data-sources.json', JSON.stringify({
    version: 1,
    sources: [{ name: 'Api', kind: 'http', url: `http://127.0.0.1:${port}` }],
  }));

  // A new empty file opens as a blank workbook.
  const workspace = path.join(dir, 'workspace');
  const file = vscode.Uri.file(path.join(workspace, 'Orders.gossia'));
  fs.writeFileSync(file.fsPath, '');
  await open(file);
  const status = await send('status');
  equal(status.text, 'Ready', 'a new empty file opens as a blank workbook, ready');
  equal(status.tab, 'Data', 'showing Data');

  // The desktop scenario's start, calculated in the panel's worker.
  await send('drop', { files: [{ name: 'Orders.json', text: ORDERS }] });
  await send('tab', { name: 'Orders' });
  await send('select', { path: 'subtotal' });
  await send('enter', { text: '=sum(i.qty * i.price for i in Orders.items)' });
  await idle();
  equal(await value('subtotal'), 52.5, 'a formula is calculated in the worker: 52.5');
  await waitFor(() => activeTab().isDirty, 'the unsaved mark');
  check(true, 'VS Code shows the unsaved mark');

  // HTTP through the extension host, from a server without CORS.
  await send('select', { path: 'tax_rate' });
  await send('enter', { text: "=source('Api').get('n.json').json()['n']" });
  await idle();
  equal(await value('tax_rate'), 41, 'a source read through the extension host (no CORS): 41');

  // VS Code's Save writes the application's bytes.
  await command('workbench.action.files.save');
  await waitFor(() => !activeTab().isDirty, 'the unsaved mark to clear');
  const saved = fs.readFileSync(file.fsPath);
  equal([...saved.subarray(0, 2)], [0x50, 0x4b], 'Save writes the workbook (a zip) to the file');
  check(await command('gossia.test.storage', 'trust.json'), 'the trust record is in globalState');

  // Revert gives back the file as it was saved.
  await send('select', { path: 'items[0].qty' });
  await send('enter', { text: '3' });
  await idle();
  equal(await value('subtotal'), 62, 'an edit after saving: 62');
  await command('workbench.action.files.revert');
  try {
    await waitFor(async () => {
      await send('tab', { name: 'Orders' });
      return (await value('subtotal')) === 52.5;
    }, 'the reverted value');
  } catch (err) {
    const status = await send('status');
    throw new Error(`${err.message}: dirty ${activeTab().isDirty}, tab ${status.tab}, `
      + `notice ${status.notice}, tab Orders: ${await send('tab', { name: 'Orders' }).then(() => 'ok', (e) => e.message)}, `
      + `new: ${JSON.stringify(await send('tree', { path: '', depth: 1 }).catch((e) => e.message))}`);
  }
  check(true, 'Revert opens the saved file again: 52.5');
  check(!activeTab().isDirty, 'and clears the unsaved mark');

  // Closed and opened again: trusted, and calculated again.
  await command('workbench.action.closeActiveEditor');
  await open(file);
  equal((await send('status')).trusted, true, 'opened again, it is trusted');
  await send('tab', { name: 'Orders' });
  const subtotal = await send('cell', { path: 'subtotal' });
  equal(subtotal.formula, '=sum(i.qty * i.price for i in Orders.items)', 'the formula was saved');
  equal(subtotal.value, 52.5, 'and is calculated again');

  // Gossia's commands in the Command Palette, run in the active workbook.
  await command('gossia.codeEditor');
  await waitFor(async () => (await send('status')).code_editor, 'the code editor');
  check(true, 'Gossia: Code Editor opens the code editor');

  // A module is a `gossia:` file: writing one makes the module, and a
  // formula calls its function.
  const taxUri = vscode.Uri.parse(await command('gossia.test.moduleUri', 'Tax'));
  const encode = (text) => new TextEncoder().encode(text);
  await vscode.workspace.fs.writeFile(taxUri, encode('def vat(x):\n    return x * 0.21\n'));
  await send('select', { path: 'tax_rate' });
  await send('enter', { text: '=vat(100)' });
  await idle();
  equal(await value('tax_rate'), 21, 'a module written as a gossia: file is called by a formula: 21');

  // Opening it from Gossia opens it in VS Code's editor.
  await send('open_module', { name: 'Tax' });
  await waitFor(() => vscode.window.activeTextEditor
    && vscode.window.activeTextEditor.document.uri.toString() === taxUri.toString(), "the module's editor");
  const editor = vscode.window.activeTextEditor;
  equal(editor.document.languageId, 'python', "the module opens in VS Code's editor, as Python");

  // Saving it there puts it in the workbook, and the formula follows.
  const text = editor.document.getText();
  const edit = new vscode.WorkspaceEdit();
  const at = text.indexOf('0.21');
  edit.replace(taxUri, new vscode.Range(editor.document.positionAt(at), editor.document.positionAt(at + 4)), '0.25');
  await vscode.workspace.applyEdit(edit);
  await editor.document.save();
  await idle();
  await waitFor(async () => {
    await idle();
    return (await value('tax_rate')) === 25;
  }, 'the formula after the save');
  check(true, 'saving the module in VS Code puts it in the workbook: 25');

  // An undo in the workbook is the module's too, and the open file follows.
  await command('gossia.undo');
  await waitFor(() => editor.document.getText().includes('0.21'), 'the file after the undo');
  await idle();
  equal(await value('tax_rate'), 21, 'Gossia: Undo takes the module back, and the open file shows it');

  // A file that is not a module name is refused, with the workbook's reason.
  let refused = null;
  try {
    await vscode.workspace.fs.writeFile(taxUri.with({ path: '/not a name.py' }), encode(''));
  } catch (err) {
    refused = err.message || String(err);
  }
  check(refused, 'a file whose name is no module name is refused', refused);

  server.close();
}

exports.run = async function () {
  const dir = process.env.GOSSIA_TEST_DIR;
  try {
    await suite(dir);
  } catch (err) {
    check(false, 'the suite ended', err.message || String(err));
  } finally {
    fs.writeFileSync(process.env.GOSSIA_TEST_OUT, JSON.stringify(results, null, 2));
  }
  if (results.some((r) => !r.ok)) throw new Error('some checks failed');
};
