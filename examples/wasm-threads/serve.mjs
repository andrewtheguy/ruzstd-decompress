// node serve.mjs [PORT] [HOST] [--data DIR]: this directory over HTTP, cross-origin isolated, as
// a page must be for its memory to be shared: the demo at /demo/, and DIR, if given, at /data/.
import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { extname, normalize, sep } from "node:path";
import { fileURLToPath } from "node:url";

import { resolve } from "node:path";

const root = fileURLToPath(new URL(".", import.meta.url));
const args = process.argv.slice(2);
const dataAt = args.indexOf("--data");
const data = dataAt < 0 ? null : resolve(args.splice(dataAt, 2)[1]) + sep;
const port = Number(args[0] ?? 8000);
const host = args[1] ?? "127.0.0.1";
const types = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".mjs": "text/javascript; charset=utf-8",
  ".wasm": "application/wasm",
  ".css": "text/css; charset=utf-8",
  ".json": "application/json",
};

createServer(async (req, res) => {
  const path = decodeURIComponent(new URL(req.url, "http://x").pathname);
  if (path === "/") {
    res.writeHead(302, { Location: "/demo/" });
    return res.end();
  }
  const [base, rest] =
    data && path.startsWith("/data/") ? [data, path.slice(6)] : [root, path.slice(1)];
  const file = normalize(base + (rest === "" || rest.endsWith("/") ? rest + "index.html" : rest));
  if (!file.startsWith(base)) {
    res.writeHead(403);
    return res.end();
  }
  try {
    const body = await readFile(file);
    res.writeHead(200, {
      "Content-Type": types[extname(file)] ?? "application/octet-stream",
      "Cross-Origin-Opener-Policy": "same-origin",
      "Cross-Origin-Embedder-Policy": "require-corp",
      "Cache-Control": "no-store",
    });
    res.end(body);
  } catch {
    res.writeHead(404);
    res.end("not found");
  }
}).listen(port, host, () => console.log(`http://${host}:${port}/demo/`));
