// ruzstd-decompress on several threads, in a browser or in Node:
//
//   import { createDecoder } from "./js/ruzstd-mt.js";
//   const decoder = await createDecoder({ threads: 4 });
//   const bytes = await decoder.decode(compressed);   // a Uint8Array of every frame decoded
//   decoder.close();
//
// What runs: a decoder worker, the module's first instance and the thread decodes are called
// on, and `threads - 1` pool workers, each an instance of the module on the decoder's memory,
// which is shared. A decode is copied into that memory and out of it; it runs on all of them.
//
// In a browser the page must be cross-origin isolated, for a memory to be shared:
// `Cross-Origin-Opener-Policy: same-origin` and `Cross-Origin-Embedder-Policy: require-corp`
// (serve.mjs sends both). Then `crossOriginIsolated` is true.
import { compileModule, inNode, reason, spawn } from "./env.js";

/** How many threads there are to decode on, by default all of them. */
export function availableThreads() {
  return globalThis.navigator?.hardwareConcurrency ?? 1;
}

/**
 * A decoder on `threads` threads.
 *
 * - `dictionaries`: raw dictionaries (Uint8Array), for the frames that name their ids.
 * - `maxWindowSize`: the largest window a frame may ask for, 128 MiB unless raised.
 * - `module`: the compiled module, to make several decoders of one compile.
 */
export async function createDecoder({
  threads = availableThreads(),
  dictionaries = [],
  maxWindowSize = 0,
  module,
} = {}) {
  if (!inNode && !globalThis.crossOriginIsolated) {
    throw new Error("the page is not cross-origin isolated, so its memory cannot be shared");
  }
  module ??= await compileModule();
  const worker = await spawn(new URL("./decoder-worker.js", import.meta.url));
  const waiting = new Map();
  let next = 0;
  let started;
  let failure = null;
  const fail = (error) => {
    failure = error;
    for (const { reject } of waiting.values()) reject(error);
    waiting.clear();
  };
  worker.onError(fail);
  worker.onMessage((m) => {
    const id = m.type === "started" || m.type === "closed" ? m.type : m.id;
    const entry = waiting.get(id);
    if (!entry) {
      if (m.type === "failed") fail(new Error(m.reason));
      return;
    }
    waiting.delete(id);
    if (m.type === "failed") entry.reject(new Error(m.reason));
    else entry.resolve(m);
  });
  const ask = (id, message, transfer) =>
    new Promise((resolve, reject) => {
      if (failure) return reject(failure);
      waiting.set(id, { resolve, reject });
      worker.post(message, transfer);
    });

  try {
    started = await ask("started", { type: "start", module, threads, dictionaries, maxWindowSize });
  } catch (error) {
    worker.terminate();
    throw error;
  }

  return {
    /** The threads it decodes on. */
    threads: started.threads,
    /**
     * Every frame of `input` decoded, each one's checksum checked, with how long it took in the
     * decoder's worker, in milliseconds: `ms` with the copies of the input into the module's
     * memory and of the output out of it, `decodeMs` without. `transfer` hands `input`'s buffer
     * over instead of copying it, leaving it empty here.
     */
    async decodeTimed(input, { transfer = false } = {}) {
      const id = next++;
      const { output, ms, decodeMs } = await ask(
        id,
        { type: "decode", id, input },
        transfer ? [input.buffer] : [],
      );
      return { output, ms, decodeMs };
    },
    /** Every frame of `input` decoded (see `decodeTimed`). */
    async decode(input, options) {
      return (await this.decodeTimed(input, options)).output;
    },
    /** End its workers. */
    async close() {
      try {
        await ask("closed", { type: "close" });
      } catch (error) {
        console.warn(`closing the decoder: ${reason(error)}`);
      }
      // The pool's workers end once their threads are out of the module (decoder-worker.js).
      setTimeout(() => worker.terminate(), 100);
    },
  };
}
