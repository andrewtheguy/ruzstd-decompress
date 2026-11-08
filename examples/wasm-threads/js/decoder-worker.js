// The decoder: the module's first instance, whose memory the pool's workers share, and the
// thread every decode is called on. A decode waits for the pool's threads, which a browser's
// main thread may not, so it is a worker of its own (see ruzstd-mt.js).
import { initSync, Decoder } from "../pkg/ruzstd_mt.js";
import { ownerPort, spawn, reason } from "./env.js";

const owner = await ownerPort();
let decoder = null;
const seats = [];

/** `count` pool workers, each seated on `memory`. */
async function seat(module, memory, count) {
  const url = new URL("./pool-worker.js", import.meta.url);
  const started = [];
  for (let i = 0; i < count; i++) {
    const worker = await spawn(url);
    seats.push(worker);
    started.push(
      new Promise((resolve, reject) => {
        worker.onError(reject);
        worker.onMessage((m) => {
          if (m.type === "seated") resolve();
          else if (m.type === "failed") reject(new Error(m.reason));
          else if (m.type === "stopped") worker.terminate();
        });
      }),
    );
    worker.post({ module, memory });
  }
  await Promise.all(started);
}

owner.onMessage(async (m) => {
  try {
    switch (m.type) {
      case "start": {
        const { memory } = initSync({ module: m.module });
        await seat(m.module, memory, m.threads - 1);
        decoder = new Decoder(m.threads);
        if (m.maxWindowSize) decoder.setMaxWindowSize(m.maxWindowSize);
        for (const dict of m.dictionaries) decoder.addDictionary(dict);
        owner.post({ type: "started", threads: decoder.threads() });
        break;
      }
      case "decode": {
        const start = performance.now();
        const output = decoder.decode(m.input);
        const ms = performance.now() - start;
        const decodeMs = decoder.lastDecodeMs();
        owner.post({ type: "decoded", id: m.id, output, ms, decodeMs }, [output.buffer]);
        break;
      }
      case "close": {
        // Its pool's threads return from their seats, and their workers are ended.
        decoder?.free();
        decoder = null;
        owner.post({ type: "closed" });
        break;
      }
    }
  } catch (error) {
    owner.post({ type: "failed", id: m.id, reason: reason(error) });
  }
});
