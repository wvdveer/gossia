// The desktop's shared scenario (crates/gossia-app/tests/common/mod.rs,
// edit_and_recalculate) on the application in the page: the same commands
// and the same answers. The file is dropped as text, as the browser drops it.
'use strict';

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

async function runEditTests() {
  const h = window.harness;
  h.section('edit and recalculate (the desktop scenario)');
  h.clearStorage();

  let app;
  await h.step('the application starts in its tab', async () => {
    app = await h.openApp('gossia-edit');
    h.pass('started, with gossiaControl');
  });
  const value = async (path) => (await app.send('cell', { path })).value;
  const idle = () => app.send('wait_idle', { timeout_ms: 60000 });

  await h.step('hello, and a blank workbook ready', async () => {
    h.assertEqual((await app.send('hello')).protocol, 1, 'the protocol is version 1');
    await idle();
    const status = await app.send('status');
    h.assertEqual(status.text, 'Ready', 'the status bar says Ready');
    h.assertEqual(status.problems, 'No errors', 'no errors');
    h.assertEqual(status.tab, 'Data', 'the blank workbook shows Data');
  });

  await h.step('a dropped JSON file becomes a tab', async () => {
    const dropped = await app.send('drop', { files: [{ name: 'Orders.json', text: ORDERS }] });
    h.assertEqual(dropped.tabs, ['Orders'], 'dropping Orders.json makes the tab Orders');
    await app.send('tab', { name: 'Orders' });
    h.assertEqual((await app.send('status')).tab, 'Orders', 'Orders is shown');
    h.assertEqual(await value('subtotal'), 0.5, 'the dropped value is there');
  });

  await h.step('a formula is calculated in the worker', async () => {
    await app.send('select', { path: 'subtotal' });
    await app.send('enter', { text: '=sum(i.qty * i.price for i in Orders.items)' });
    await idle();
    h.assertEqual(await value('subtotal'), 52.5, '2 * 9.5 + 1 * 24.5 + 12 * 0.75 = 52.5');
    const cell = await app.send('cell', { path: 'subtotal' });
    h.assertTrue(String(cell.formula).startsWith('='), 'the cell keeps its formula', cell.formula);
    h.assertEqual(cell.pending, false, 'and is not pending');
  });

  await h.step('an edit recalculates what depends on it', async () => {
    await app.send('select', { path: 'items[0].qty' });
    await app.send('enter', { text: '3' });
    await idle();
    h.assertEqual(await value('items[0].qty'), 3, 'the quantity is 3');
    h.assertEqual(await value('subtotal'), 62, 'the subtotal follows: 62');
  });

  await h.step('undo and redo', async () => {
    await app.send('key', { chord: 'ctrl+z' });
    await idle();
    h.assertEqual(await value('subtotal'), 52.5, 'Ctrl+Z undoes the edit');
    await app.send('key', { chord: 'ctrl+y' });
    await idle();
    h.assertEqual(await value('subtotal'), 62, 'Ctrl+Y redoes it');
  });

  await h.step('a value that does not fit the schema is refused', async () => {
    await app.send('select', { path: 'items[0].qty' });
    const refused = await h.refusal(app, 'enter', { text: 'many' });
    h.assertTrue(refused.length > 'enter: '.length, 'refused with the reason', refused);
    await app.send('key', { chord: 'escape' });
    h.assertEqual(await value('items[0].qty'), 3, 'the value stays');
  });

  await h.step('another formula, and a subtree', async () => {
    await app.send('select', { path: 'tax_rate' });
    await app.send('enter', { text: '=0.5 / 2' });
    await idle();
    h.assertEqual(await value('tax_rate'), 0.25, '0.5 / 2 = 0.25');
    const tree = await app.send('tree', { path: 'customer', depth: 1 });
    h.assertTrue(tree && typeof tree === 'object', 'tree gives the subtree');
  });

  await h.step('commands refused as on the desktop', async () => {
    h.assertContains(await h.refusal(app, 'close'), 'not supported yet', 'close waits for the workbook layer');
    h.assertContains(await h.refusal(app, 'explode'), 'unknown command', 'an unknown command is refused');
    h.assertContains(await h.refusal(app, 'quit'), 'not done in the page', 'quit is not done in the page');
  });

  await h.step('a new workbook', async () => {
    const made = await app.send('new_workbook');
    h.assertEqual(made.tabs, ['Data', 'DataSchema'], 'File > New gives Data and DataSchema');
    const status = await app.send('status');
    h.assertEqual(status.tab, 'Data', 'Data is shown');
    h.assertEqual(status.modified, false, 'and it is not modified');
  });
}
