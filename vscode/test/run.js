// Runs the extension's tests in a VS Code downloaded for them (DESIGN.md, Web
// port, 0.3.0, Testing). Insiders by default, which runs beside an open VS
// Code (stable refuses to run tests while one is open); GOSSIA_VSCODE_VERSION
// names another (`stable`, or a version). The download is kept in
// target/vscode-test. media/pkg must hold the test build
// (cargo xtask vscode --test-control).
'use strict';

const fs = require('fs');
const os = require('os');
const path = require('path');
const { runTests } = require('@vscode/test-electron');

(async () => {
  const extension = path.resolve(__dirname, '..');
  const root = path.resolve(extension, '..');
  if (!fs.existsSync(path.join(extension, 'media', 'pkg', 'gossia_web_bg.wasm'))) {
    throw new Error('media/pkg is empty: run cargo xtask vscode --test-control first');
  }
  // A folder of its own for the workbooks, and a fresh profile, so that
  // nothing is kept from an earlier run.
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'gossia-vscode-'));
  const out = path.join(dir, 'results.json');
  fs.mkdirSync(path.join(dir, 'workspace'));
  // The download is used again once it is there: the downloader does not
  // recognise its own Insiders folder and would fetch it again each run.
  const version = process.env.GOSSIA_VSCODE_VERSION || 'insiders';
  const cachePath = path.join(root, 'target', 'vscode-test');
  const exe = process.platform === 'win32' && version === 'insiders'
    ? path.join(cachePath, 'vscode-win32-x64-archive-insiders', 'Code - Insiders.exe')
    : null;
  const where = exe && fs.existsSync(exe) ? { vscodeExecutablePath: exe } : { version, cachePath };
  try {
    await runTests({
      ...where,
      extensionDevelopmentPath: extension,
      extensionTestsPath: path.join(__dirname, 'suite.js'),
      extensionTestsEnv: { GOSSIA_VSCODE_TEST: '1', GOSSIA_TEST_DIR: dir, GOSSIA_TEST_OUT: out },
      launchArgs: [
        path.join(dir, 'workspace'),
        '--disable-extensions',
        '--disable-workspace-trust',
        '--user-data-dir', path.join(dir, 'user'),
        '--extensions-dir', path.join(dir, 'extensions'),
      ],
    });
  } finally {
    if (fs.existsSync(out)) {
      const results = JSON.parse(fs.readFileSync(out, 'utf8'));
      const failed = results.filter((r) => !r.ok).length;
      console.log(`== Summary: ${results.length - failed} passed, ${failed} failed ==`);
    }
    fs.rmSync(dir, { recursive: true, force: true });
  }
})().catch((err) => {
  console.error(err.message || err);
  process.exit(1);
});
