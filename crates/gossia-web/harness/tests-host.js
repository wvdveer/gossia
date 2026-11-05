// What only the page has: the test host behind the host interface (DESIGN.md,
// Web port, 0.3.0). The workbook is saved through the host and opened again in
// a new tab from the bytes the host was given; the preferences are kept in the
// host's storage; and a data source is read over HTTP through the host, once
// answered by the test and once by the page's own server.
'use strict';

// The total starts as a decimal, so the schema inferred for it is a number.
const PRICES = JSON.stringify({ name: 'Widget', price: 2.5, quantity: 4, total: 0.5 });

async function runHostTests() {
  const h = window.harness;
  h.section('the test host: saving, storage and HTTP');
  h.clearStorage();
  // An HTTP source on the page's own server, defined before the start as the
  // data sources panel would have kept it.
  const base = new URL('.', location.href).href.replace(/\/$/, '');
  localStorage.setItem('gossia:data-sources.json', JSON.stringify({
    version: 1,
    sources: [{ name: 'Api', kind: 'http', url: base }],
  }));

  let app;
  await h.step('the application starts with the source defined', async () => {
    app = await h.openApp('gossia-host');
    await app.send('wait_idle', { timeout_ms: 60000 });
    h.pass('started');
  });
  const value = async (path) => (await app.send('cell', { path })).value;
  const idle = () => app.send('wait_idle', { timeout_ms: 60000 });
  const host = app.win.testHost;

  await h.step('an edit marks the workbook modified in the host', async () => {
    await app.send('drop', { files: [{ name: 'Prices.json', text: PRICES }] });
    await app.send('tab', { name: 'Prices' });
    await app.send('select', { path: 'total' });
    await app.send('enter', { text: '=Prices.price * Prices.quantity' });
    await idle();
    h.assertEqual(await value('total'), 10, 'the formula gives 2.5 * 4 = 10');
    await h.waitFor(() => host.dirty === true, { label: 'setDirty(true)' });
    h.pass('the host was told the workbook has unsaved changes');
  });

  let saved;
  await h.step('save gives the workbook to the host', async () => {
    await app.send('save');
    h.assertEqual(host.saves.length, 1, 'the host was given one workbook');
    saved = host.saves[0];
    h.assertTrue(saved.name.endsWith('.gossia'), 'named as a workbook', saved.name);
    h.assertEqual(Array.from(saved.bytes.slice(0, 2)), [0x50, 0x4b], 'the bytes are a zip');
    await h.waitFor(() => host.dirty === false, { label: 'setDirty(false)' });
    h.assertEqual((await app.send('status')).modified, false, 'and it is no longer modified');
  });

  await h.step('the preferences are kept in the host\'s storage', async () => {
    await app.send('set_option', { name: 'theme', value: 'light' });
    const prefs = localStorage.getItem('gossia:preferences.json');
    h.assertContains(prefs, 'light', 'preferences.json in storage has the theme');
    h.assertTrue(localStorage.getItem('gossia:trust.json'), 'the save wrote the trust record');
  });

  await h.step('a source read answered by the test', async () => {
    host.answerFetches = true;
    const before = host.fetches.length;
    await app.send('select', { path: 'price' });
    await app.send('enter', { text: "=source('Api').get('scripted').json()['n']" });
    await h.waitFor(() => host.fetches.length > before, { label: 'the request to reach the host' });
    const request = host.fetches[before];
    h.assertEqual(request.method, 'GET', 'the host is asked for a GET');
    h.assertTrue(request.url.endsWith('/scripted'), 'of the source\'s URL and the path', request.url);
    host.answerFetch(before, 200, '{"n": 41}', ['content-type', 'application/json']);
    await idle();
    h.assertEqual(await value('price'), 41, 'the answer reaches the formula (run again)');
    h.assertEqual(await value('total'), 164, 'and what depends on it: 41 * 4');
  });

  await h.step('a source read from the page\'s own server', async () => {
    host.answerFetches = false;
    await app.send('select', { path: 'price' });
    await app.send('enter', { text: "=source('Api').get('fixture.json').json()['n']" });
    await idle();
    h.assertEqual(await value('price'), 7.25, 'fixture.json was fetched: 7.25');
    const failed = host.fetches.length;
    await app.send('enter', { text: "=source('Api').get('missing.json').status" });
    await idle();
    const missing = await app.send('cell', { path: 'price' });
    h.assertEqual(missing.error && missing.error.code, '#SOURCE!', 'a missing file is #SOURCE!');
    h.assertContains(missing.error && missing.error.message, '404', 'saying it answered 404');
    h.assertTrue(host.fetches.length > failed, 'that request went through the host too');
  });

  await h.step('the saved workbook opens again in a new tab', async () => {
    const url = URL.createObjectURL(new Blob([saved.bytes]));
    const again = await h.openApp('gossia-reopen', `?open=${encodeURIComponent(url)}`);
    await again.send('wait_idle', { timeout_ms: 60000 });
    const status = await again.send('status');
    h.assertEqual(status.trusted, true, 'it is trusted, from the record the save wrote');
    await again.send('tab', { name: 'Prices' });
    const total = await again.send('cell', { path: 'total' });
    h.assertEqual(total.formula, '=Prices.price * Prices.quantity', 'the formula was saved');
    h.assertEqual(total.value, 10, 'and is calculated again: 10');
    URL.revokeObjectURL(url);
  });
}
