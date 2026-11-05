// The Web Worker that runs Python (DESIGN.md, Web port, 0.3.0). Build step
// 13a: it loads the engine and answers probe requests from the page.
import init, { Probe, last_panic } from './pkg/cliotree_worker.js';

let probe = null;

self.onmessage = async (e) => {
  const m = e.data;
  try {
    if (m.type === 'start') {
      const t0 = performance.now();
      await init();
      const t1 = performance.now();
      probe = new Probe();
      const t2 = performance.now();
      postMessage({ type: 'started', loadMs: Math.round(t1 - t0), engineMs: Math.round(t2 - t1) });
    } else if (m.type === 'run') {
      const t0 = performance.now();
      const reply = JSON.parse(probe.run(m.schema, m.data, m.path, m.formula));
      postMessage({ type: 'ran', id: m.id, ms: Math.round(performance.now() - t0), ...reply });
    } else if (m.type === 'ping') {
      postMessage({ type: 'pong' });
    }
  } catch (err) {
    let panic = null;
    try { panic = last_panic(); } catch (_) { /* the module is unusable */ }
    const stack = err && err.stack ? err.stack.split('\n').slice(0, 12).join('\n') : null;
    postMessage({ type: 'failed', id: m.id, error: String(err), panic, stack });
  }
};
