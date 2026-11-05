// The Web Worker that runs Python in the panel (DESIGN.md, Web port, 0.3.0).
// panel.js makes the worker from the wasm module's JavaScript glue followed by
// this text, so `initSync`, `WorkerSide` and `last_panic` are the glue's own.
// The first message is the compiled module, which the page fetched once; the
// rest are the messages of the worker protocol, as bytes.
let side = null;

self.onmessage = (e) => {
  if (side === null) {
    initSync({ module: e.data.module });
    side = new WorkerSide((bytes) => postMessage(bytes, [bytes.buffer]));
    return;
  }
  try {
    side.handle(new Uint8Array(e.data));
  } catch (err) {
    let panic = null;
    try { panic = last_panic(); } catch (_) { /* the module is unusable */ }
    postMessage({ error: String(err), panic });
  }
};
