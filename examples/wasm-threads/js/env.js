// Workers and their messages, the same in a browser and in Node (worker_threads): a worker is
// `{ post(message, transfer), onMessage(handler), terminate() }`, and so is the port a worker
// talks to its owner through.
export const inNode = typeof process === "object" && !!process.versions?.node;

/** A worker running the module at `url`, a `URL`. */
export async function spawn(url) {
  if (inNode) {
    const { Worker } = await import("node:worker_threads");
    const worker = new Worker(url);
    return {
      post: (message, transfer) => worker.postMessage(message, transfer),
      onMessage: (handler) => worker.on("message", handler),
      onError: (handler) => worker.on("error", handler),
      terminate: () => worker.terminate(),
    };
  }
  const worker = new Worker(url, { type: "module" });
  return {
    post: (message, transfer) => worker.postMessage(message, transfer),
    onMessage: (handler) => worker.addEventListener("message", (e) => handler(e.data)),
    onError: (handler) => worker.addEventListener("error", (e) => handler(e.error ?? new Error(e.message))),
    terminate: () => worker.terminate(),
  };
}

/** The port the calling worker talks to its owner through. */
export async function ownerPort() {
  if (inNode) {
    const { parentPort } = await import("node:worker_threads");
    return {
      post: (message, transfer) => parentPort.postMessage(message, transfer),
      onMessage: (handler) => parentPort.on("message", handler),
    };
  }
  return {
    post: (message, transfer) => self.postMessage(message, transfer),
    onMessage: (handler) => self.addEventListener("message", (e) => handler(e.data)),
  };
}

/** The module, compiled once: it is handed to every worker, which each make an instance of it. */
export async function compileModule(url = new URL("../pkg/ruzstd_mt_bg.wasm", import.meta.url)) {
  if (inNode) {
    const { readFile } = await import("node:fs/promises");
    return WebAssembly.compile(await readFile(url));
  }
  return WebAssembly.compileStreaming(fetch(url));
}

/** Why something failed, as text, which every message can carry. */
export const reason = (error) => (error instanceof Error ? error.message : String(error));
