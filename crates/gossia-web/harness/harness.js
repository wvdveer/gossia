// The helpers of the harness page, as in the owner's other web projects
// (keyscarf's tools/web-test-harness): the application is opened in another
// tab with window.open, from the same origin, so this page can reach its
// window. Gossia draws on a canvas, so instead of querying elements the tests
// send it the commands of the test control protocol through the test build's
// window.gossiaControl(cmd, args).
'use strict';

const results = { pass: 0, fail: 0 };
let logEl = null;
let lines = [];
const opened = [];

function attachLog(el) {
  logEl = el;
}

function logLine(text) {
  lines.push(text);
  if (logEl) {
    logEl.textContent += text + '\n';
    logEl.scrollTop = logEl.scrollHeight;
  }
  console.log(text);
}

function logText() {
  return lines.join('\n');
}

function section(name) {
  logLine(`\n== ${name} ==`);
}

function pass(desc) {
  results.pass++;
  logLine(`  ok - ${desc}`);
}

function fail(desc, detail) {
  results.fail++;
  logLine(`  FAIL - ${desc}${detail ? ` (${detail})` : ''}`);
}

function resetResults() {
  results.pass = 0;
  results.fail = 0;
  lines = [];
  if (logEl) logEl.textContent = '';
}

function summary() {
  logLine(`\n== Summary: ${results.pass} passed, ${results.fail} failed ==`);
  return results.fail === 0;
}

// The text of something thrown: the application rejects with plain strings.
function message(e) {
  return e instanceof Error ? e.message : String(e);
}

function waitFor(predicate, { timeout = 5000, interval = 50, label = 'condition' } = {}) {
  return new Promise((resolve, reject) => {
    const start = Date.now();
    (function poll() {
      let value;
      try {
        value = predicate();
      } catch (_) {
        value = undefined; // the window is still loading; keep polling
      }
      if (value) return resolve(value);
      if (Date.now() - start > timeout) return reject(new Error(`timed out waiting for ${label}`));
      setTimeout(poll, interval);
    })();
  });
}

// Gossia's storage for this origin, which the application's tabs share with
// this page: the preferences, the trust record and the data sources
// (`gossia:`), and the secrets (`gossia-secret:`).
function clearStorage() {
  for (const key of Object.keys(localStorage)) {
    if (key.startsWith('gossia:') || key.startsWith('gossia-secret:')) localStorage.removeItem(key);
  }
}

// Opens the application in a tab of its own, named `name`, and waits until it
// has started. Gives an object with the window, `send(cmd, args)` (the result,
// or a thrown Error with the application's message) and `ok(cmd, args)`,
// which also fails the suite's step when the command fails.
async function openApp(name, query = '') {
  const win = window.open(`index.html${query}`, name);
  if (!win) throw new Error('window.open() was blocked: allow popups for this page');
  opened.push(win);
  await waitFor(() => win.appStarted || win.appError, { timeout: 120000, label: 'the application to start' });
  if (win.appError) throw new Error(win.appError);
  if (typeof win.gossiaControl !== 'function') {
    throw new Error('the page has no gossiaControl: build it with cargo xtask web --test-control');
  }
  // The test host records files to save and to open instead of asking.
  win.testHost.answerPicks = true;
  const send = async (cmd, args = null) => {
    try {
      return await win.gossiaControl(cmd, args);
    } catch (e) {
      throw new Error(`${cmd}: ${message(e)}`);
    }
  };
  return { win, send };
}

function closeApps() {
  while (opened.length) {
    try { opened.pop().close(); } catch (_) { /* already gone */ }
  }
}

// What a command refuses with, or a thrown Error if it does not refuse.
async function refusal(app, cmd, args) {
  try {
    await app.send(cmd, args);
  } catch (e) {
    return message(e);
  }
  throw new Error(`${cmd} was expected to be refused`);
}

function same(a, b) {
  return JSON.stringify(a) === JSON.stringify(b);
}

function assertEqual(actual, expected, desc) {
  if (same(actual, expected)) pass(desc);
  else fail(desc, `expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`);
}

function assertTrue(value, desc, detail) {
  if (value) pass(desc);
  else fail(desc, detail);
}

function assertContains(actual, needle, desc) {
  if (String(actual).includes(needle)) pass(desc);
  else fail(desc, `expected to find '${needle}' in: ${actual}`);
}

// Runs one step; a step that throws fails and ends the suite, since later
// steps build on it.
async function step(desc, fn) {
  try {
    await fn();
  } catch (e) {
    fail(desc, message(e));
    throw e;
  }
}

window.harness = {
  results,
  attachLog,
  logLine,
  logText,
  section,
  pass,
  fail,
  resetResults,
  summary,
  message,
  waitFor,
  clearStorage,
  openApp,
  closeApps,
  refusal,
  assertEqual,
  assertTrue,
  assertContains,
  step,
};
