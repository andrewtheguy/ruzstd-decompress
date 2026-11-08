// node browser-test.mjs [--chrome PATH] [--threads 1,2,4,8] [--dict DICT] DIR FILE...
//
// The module in a real browser: headless Chrome (CPU only; nothing here needs a GPU), driven
// over the DevTools protocol. Serves this directory and DIR (serve.mjs), opens the demo page,
// and there decodes each FILE of DIR on each number of threads, three times, checking that every
// decode gives the same bytes, with the dictionary DICT of DIR if given. Prints a JSON line for
// each.
import { spawn } from "node:child_process";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const args = process.argv.slice(2);
const option = (name, fallback) => {
  const at = args.indexOf(name);
  if (at < 0) return fallback;
  if (at + 1 >= args.length) {
    console.error(`${name} needs a value`);
    process.exit(2);
  }
  return args.splice(at, 2)[1];
};
const chrome = option("--chrome", "google-chrome");
const threadList = option("--threads", "");
const dict = option("--dict", null);
const [dir, ...files] = args;
if (!dir || files.length === 0) {
  console.error("node browser-test.mjs [--chrome PATH] [--threads 1,2,4,8] [--dict DICT] DIR FILE...");
  process.exit(2);
}
const port = 8000 + Math.floor(Math.random() * 1000);
const here = fileURLToPath(new URL(".", import.meta.url));
const server = spawn(process.execPath, [join(here, "serve.mjs"), String(port), "127.0.0.1", "--data", dir], {
  stdio: ["ignore", "pipe", "inherit"],
});
await new Promise((resolve) => server.stdout.once("data", resolve));

const profile = await mkdtemp(join(tmpdir(), "ruzstd-mt-chrome-"));
const debugPort = port + 1000;
const browser = spawn(chrome, [
  "--headless=new", `--remote-debugging-port=${debugPort}`, `--user-data-dir=${profile}`,
  "--no-first-run", "--no-default-browser-check", "about:blank",
], { stdio: "ignore" });
let version;
for (let i = 0; i < 100 && !version; i++) {
  version = await fetch(`http://127.0.0.1:${debugPort}/json/version`).then((r) => r.json(), () => null);
  if (!version) await new Promise((r) => setTimeout(r, 100));
}
const finish = async (code) => {
  browser.kill();
  server.kill();
  await new Promise((r) => setTimeout(r, 300));
  await rm(profile, { recursive: true, force: true });
  process.exit(code);
};
if (!version) {
  console.error(`no DevTools from ${chrome}`);
  await finish(1);
}

// The DevTools protocol, one session on one page.
const ws = new WebSocket(version.webSocketDebuggerUrl);
await new Promise((resolve) => ws.addEventListener("open", resolve, { once: true }));
let nextId = 0;
const pending = new Map();
ws.addEventListener("message", ({ data }) => {
  const m = JSON.parse(data);
  if (m.id !== undefined && pending.has(m.id)) {
    const { resolve, reject } = pending.get(m.id);
    pending.delete(m.id);
    m.error ? reject(new Error(m.error.message)) : resolve(m.result);
  }
});
const send = (method, params = {}, sessionId) =>
  new Promise((resolve, reject) => {
    const id = nextId++;
    pending.set(id, { resolve, reject });
    ws.send(JSON.stringify({ id, method, params, sessionId }));
  });
const { targetId } = await send("Target.createTarget", { url: `http://127.0.0.1:${port}/demo/` });
const { sessionId } = await send("Target.attachToTarget", { targetId, flatten: true });
await new Promise((r) => setTimeout(r, 1500));

const script = (file) => `(async () => {
  const { createDecoder, availableThreads } = await import("/js/ruzstd-mt.js");
  const { compileModule } = await import("/js/env.js");
  const module = await compileModule();
  const input = new Uint8Array(await (await fetch("/data/${file}")).arrayBuffer());
  const dictionaries = ${dict ? `[new Uint8Array(await (await fetch("/data/${dict}")).arrayBuffer())]` : "[]"};
  const most = availableThreads();
  const counts = ${threadList ? JSON.stringify(threadList.split(",").map(Number)) : "[...new Set([1, 2, 4, most])]"};
  const lines = [];
  let expected = null;
  for (const threads of counts) {
    const decoder = await createDecoder({ threads, module, dictionaries });
    const times = [];
    for (let i = 0; i < 3; i++) {
      const { output, decodeMs, ms } = await decoder.decodeTimed(input);
      const digest = [...new Uint8Array(await crypto.subtle.digest("SHA-256", output))]
        .map((b) => b.toString(16).padStart(2, "0")).join("");
      expected ??= digest;
      if (digest !== expected) throw new Error(threads + " threads decoded other bytes");
      times.push({ decodeMs, ms, out: output.length });
    }
    await decoder.close();
    times.sort((a, b) => a.decodeMs - b.decodeMs);
    lines.push({ file: "${file}", threads, decode_ms: +times[1].decodeMs.toFixed(2),
                 call_ms: +times[1].ms.toFixed(2), bytes_out: times[1].out });
  }
  return JSON.stringify({ crossOriginIsolated, userAgent: navigator.userAgent, lines });
})()`;

let code = 0;
for (const file of files) {
  const { result, exceptionDetails } = await send(
    "Runtime.evaluate",
    { expression: script(file), awaitPromise: true, returnByValue: true },
    sessionId,
  );
  if (exceptionDetails) {
    console.log(`FAIL ${file}: ${exceptionDetails.exception?.description ?? exceptionDetails.text}`);
    code = 1;
    continue;
  }
  const { crossOriginIsolated, userAgent, lines } = JSON.parse(result.value);
  if (file === files[0]) console.log(`# ${userAgent}, crossOriginIsolated: ${crossOriginIsolated}`);
  for (const line of lines) console.log(JSON.stringify(line));
}
ws.close();
await finish(code);
