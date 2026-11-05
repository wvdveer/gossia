// The Web Worker that runs Python (DESIGN.md, Web port, 0.3.0). It loads the
// engine, then takes the messages of the worker protocol in order and posts
// its own back as bytes.
import init, { WorkerSide, last_panic } from './pkg/gossia_web.js';

const ready = init().then(() => new WorkerSide((bytes) => postMessage(bytes, [bytes.buffer])));

// Messages that arrive while the engine loads, or while a job runs, wait
// their turn.
let queue = Promise.resolve();

self.onmessage = (e) => {
  const message = new Uint8Array(e.data);
  queue = queue.then(async () => {
    const side = await ready;
    try {
      side.handle(message);
    } catch (err) {
      let panic = null;
      try { panic = last_panic(); } catch (_) { /* the module is unusable */ }
      postMessage({ error: String(err), panic });
    }
  });
};
