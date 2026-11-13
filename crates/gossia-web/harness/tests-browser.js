// The browser host (DESIGN.md, The web version on GitHub Pages, 17a): the
// workbook saved and kept in the recent list with a copy in IndexedDB, opened
// again from that copy and from a file chosen, secrets kept for the session
// unless remembered, and the error for a site that does not allow this page's
// origin. Files go through the fallback path (the tests answer instead of the
// browser's dialogs), as on Firefox and Safari.
'use strict';

const NUMBERS = JSON.stringify({ a: 2, b: 3, sum: 0 });

// Waits until an async check holds (harness.waitFor takes a plain one).
async function until(check, label, timeout = 20000) {
  const start = Date.now();
  while (!(await check())) {
    if (Date.now() - start > timeout) throw new Error(`timed out waiting for ${label}`);
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
}

function deleteDb() {
  return new Promise((resolve) => {
    const req = indexedDB.deleteDatabase('gossia');
    req.onsuccess = req.onerror = req.onblocked = () => resolve();
  });
}

async function runBrowserTests() {
  const h = window.harness;
  h.section('the browser host: files, recent workbooks, secrets and CORS');
  h.clearStorage();
  await deleteDb();
  const other = new URLSearchParams(location.search).get('other');
  if (other) {
    // A source on another origin, which does not allow this one.
    localStorage.setItem('gossia:data-sources.json', JSON.stringify({
      version: 1,
      sources: [
        { name: 'Elsewhere', kind: 'http', url: `http://127.0.0.1:${other}` },
        { name: 'Nowhere', kind: 'http', url: 'http://127.0.0.1:9' },
      ],
    }));
  }

  let app;
  await h.step('the application starts with the browser host', async () => {
    app = await h.openApp('gossia-browser');
    await app.send('wait_idle', { timeout_ms: 60000 });
    h.assertEqual(app.win.testHost.host.keepsFiles(), true, 'the host keeps files');
  });

  await h.step('the loading screen knows the size of the module, and goes once started', async () => {
    const build = await (await fetch('build.json', { cache: 'no-store' })).json();
    h.assertTrue(build.wasmBytes > 1000000, `build.json gives the module's size (${build.wasmBytes})`);
    h.assertTrue(/^\d+\.\d+\.\d+$/.test(build.version), `and the version (${build.version})`);
    h.assertEqual(app.win.document.getElementById('loading'), null, 'the loading screen is gone');
  });

  await h.step('the service worker keeps the build for offline use (17c)', async () => {
    const build = await (await fetch('build.json', { cache: 'no-store' })).json();
    h.assertTrue(/^[0-9a-f]{16}$/.test(build.build), `build.json names the build (${build.build})`);
    await until(() => app.win.testHost.serviceWorker, 'the service worker to be registered');
    await app.win.navigator.serviceWorker.ready;
    h.pass('it is active');
    const name = `gossia-${build.build}`;
    await until(async () => (await app.win.caches.keys()).includes(name), 'the cache of this build');
    const cache = await app.win.caches.open(name);
    for (const file of ['./', 'index.html', 'pkg/gossia_web_bg.wasm', 'pkg/gossia_web.js',
                        'worker.js', 'host.js', 'manifest.webmanifest', 'icons/192.png']) {
      h.assertTrue(Boolean(await cache.match(file)), `${file} is in the cache`);
    }
    const manifest = await (await fetch('manifest.webmanifest')).json();
    h.assertEqual([manifest.name, manifest.display, manifest.start_url],
                  ['Gossia', 'standalone', './'], 'the manifest makes it installable');
  });

  await h.step('a new build is said to be ready, and Later puts the notice away', async () => {
    app.win.testHost.showUpdate(app.win.testHost.serviceWorker);
    const box = app.win.document.getElementById('update');
    h.assertContains(box && box.textContent, 'new version of Gossia is ready', 'the notice');
    const buttons = [...box.querySelectorAll('button')].map((b) => b.textContent);
    h.assertEqual(buttons, ['Later', 'Restart now'], 'with its buttons');
    box.querySelector('button').click();
    h.assertEqual(app.win.document.getElementById('update'), null, 'Later puts it away');
  });
  const host = app.win.testHost;
  const browser = host.host;
  const idle = () => app.send('wait_idle', { timeout_ms: 60000 });
  const status = () => app.send('status');

  let saved;
  await h.step('a save is written and kept in the recent list', async () => {
    await app.send('drop', { files: [{ name: 'Numbers.json', text: NUMBERS }] });
    await app.send('tab', { name: 'Numbers' });
    await app.send('select', { path: 'sum' });
    await app.send('enter', { text: '=Numbers.a + Numbers.b' });
    await idle();
    await app.send('save');
    await h.waitFor(() => host.dirty === false, { label: 'the save to be answered' });
    saved = host.saves[host.saves.length - 1];
    h.assertEqual((await status()).modified, false, 'no longer modified once the page wrote it');
    await h.waitFor(() => browser.recentWorkbooks().length === 2, { label: 'the recent list' });
    h.assertEqual(browser.recentWorkbooks()[1], 'Untitled.gossia', 'it is in the recent list');
  });

  await h.step('a recent workbook opens again from its copy', async () => {
    await app.send('new_workbook');
    h.assertEqual((await status()).file, null, 'a new workbook has no file');
    await app.send('open_recent', { key: browser.recentWorkbooks()[0] });
    await until(async () => (await status()).file === 'Untitled.gossia', 'the workbook to open');
    await app.send('wait_idle', { timeout_ms: 60000 });
    await app.send('tab', { name: 'Numbers' });
    h.assertEqual((await app.send('cell', { path: 'sum' })).value, 5, 'with its formula: 2 + 3');
  });

  await h.step('Open asks for a workbook, and the one chosen opens', async () => {
    await app.send('open');
    const pick = host.picks[host.picks.length - 1];
    h.assertEqual([pick.purpose, pick.extensions], ['open', ['gossia', 'html']],
                  'a workbook or a share file is asked for');
    host.answerPick('Chosen.gossia', saved.bytes);
    await until(async () => (await status()).file === 'Chosen.gossia', 'the chosen workbook to open');
    await h.waitFor(() => browser.recentWorkbooks()[1] === 'Chosen.gossia',
                    { label: 'the recent list' });
    h.pass('it is first in the recent list');
    h.assertEqual(browser.recentWorkbooks().length, 4, 'beside the one saved before');
  });

  await h.step('Save As asks again, and the changes count as saved', async () => {
    await app.send('wait_idle', { timeout_ms: 60000 });
    await app.send('tab', { name: 'Numbers' });
    await app.send('select', { path: 'a' });
    await app.send('enter', { text: '10' });
    await idle();
    const before = host.saves.length;
    await app.send('save_as');
    h.assertEqual(host.saves[before]?.asNew, true, 'the page was asked where, as a new file');
    await until(async () => (await status()).modified === false, 'the save');
    h.pass('saved');
  });

  await h.step('a share file is saved, and its page hands the workbook to the site (17e)', async () => {
    const before = host.files.length;
    await app.send('command', { name: 'saveShare' });
    const file = host.files[before];
    h.assertEqual(file && file.name, 'Chosen.html', "saved as a file, under the workbook's name");
    const html = new TextDecoder().decode(file.bytes);
    h.assertContains(html, '<meta name="gossia-share-file" content="1">', 'a share file');
    h.assertEqual((await status()).modified, false, 'a copy: the workbook is as it was');
    // For run.py, which opens it from disk.
    window.harnessShare = html;
    // The page, from this origin, with the site served here in place of the
    // official one.
    const site = new URL('index.html', location.href).href;
    const local = html.replace(/const SITE = ".*?";/, `const SITE = ${JSON.stringify(site)};`);
    const page = window.open(URL.createObjectURL(new Blob([local], { type: 'text/html' })),
                             'gossia-share');
    await h.waitFor(() => page.document.readyState === 'complete'
                          && !page.document.getElementById('open').hidden,
                    { label: 'the share page', timeout: 20000 });
    let opened = null;
    const open = page.open.bind(page);
    page.open = (...args) => (opened = open(...args));
    page.document.getElementById('open').click();
    h.assertTrue(opened, 'the button opens the site');
    await h.waitFor(() => opened.appStarted && opened.testHost.shared === 1,
                    { label: 'the site to take the workbook', timeout: 120000 });
    await h.waitFor(() => page.document.getElementById('status').textContent.includes('open in Gossia'),
                    { label: 'the page to say so' });
    h.pass('the page says the workbook is open in the other window');
    const shown = await opened.gossiaControl('status');
    h.assertContains(shown.notice, 'came from a share file', 'the site opened it from the share file');
    h.assertEqual([shown.file, shown.modified], [null, true], 'as a new workbook, not saved yet');
    await opened.gossiaControl('wait_idle', { timeout_ms: 60000 });
    await opened.gossiaControl('tab', { name: 'Numbers' });
    h.assertEqual((await opened.gossiaControl('cell', { path: 'a' })).value, 10, 'with its data');
    h.assertEqual(opened.location.hash, '', 'the address no longer asks for a share');
    opened.close();
    page.close();
  });

  await h.step('secrets are kept for the session unless remembered', async () => {
    browser.writeSecret('source:Test', '{"token": "t"}');
    h.assertEqual(localStorage.getItem('gossia-secret:source:Test'), null,
                  'a secret is not in localStorage');
    h.assertEqual(browser.readSecret('source:Test'), '{"token": "t"}', 'but kept in the page');
    browser.rememberSecret('source:Test', true);
    h.assertEqual(localStorage.getItem('gossia-secret:source:Test'), '{"token": "t"}',
                  'remembered: moved to localStorage');
    h.assertEqual(browser.secretRemembered('source:Test'), true, 'and said to be remembered');
    browser.rememberSecret('source:Test', false);
    h.assertEqual(localStorage.getItem('gossia-secret:source:Test'), null, 'forgotten again');
    h.assertEqual(browser.readSecret('source:Test'), '{"token": "t"}', 'kept for the session');
    browser.writeSecret('source:Test', null);
    h.assertEqual(browser.readSecret('source:Test'), null, 'removed');
  });

  if (!other) {
    h.logLine('  (skipped: the CORS error, which needs ?other=<port> of run.py)');
    return;
  }
  await h.step('a site that does not allow this page says so', async () => {
    await app.send('select', { path: 'sum' });
    await app.send('enter', { text: "=source('Elsewhere').get('fixture.json').status" });
    await idle();
    const cell = await app.send('cell', { path: 'sum' });
    h.assertEqual(cell.error && cell.error.code, '#SOURCE!', 'the read is #SOURCE!');
    h.assertContains(cell.error && cell.error.message, 'does not allow requests from this web page',
                     'naming the cause');
    h.assertContains(cell.error && cell.error.message, 'desktop application',
                     'and where there is no such limit');
  });

  await h.step('a site that is not there is not called a CORS refusal', async () => {
    await app.send('enter', { text: "=source('Nowhere').get('x').status" });
    await idle();
    const cell = await app.send('cell', { path: 'sum' });
    h.assertContains(cell.error && cell.error.message, 'cannot be reached', 'it cannot be reached');
  });
}
