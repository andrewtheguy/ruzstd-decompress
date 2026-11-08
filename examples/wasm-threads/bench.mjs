// node bench.mjs FILE [--dict DICT] [--threads 1,2,4,8] [--seconds 2]
//
// Decodes FILE (one or more zstd frames) on each number of threads, again and again for at least
// SECONDS and 3 times, and prints a JSON line for each: the median and the least of the decode
// alone (`decode_ms`, in the module) and of the whole call (`call_ms`, its copies into the
// module's memory and out of it included). Every decode must give the bytes the first did.
import { readFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import { availableParallelism } from "node:os";
import { createDecoder } from "./js/ruzstd-mt.js";
import { compileModule } from "./js/env.js";

const args = process.argv.slice(2);
const option = (name, fallback) => {
  const at = args.indexOf(name);
  return at < 0 ? fallback : args.splice(at, 2)[1];
};
const dict = option("--dict", null);
const threadCounts = option("--threads", `1,2,4,${availableParallelism()}`).split(",").map(Number);
const seconds = Number(option("--seconds", "2"));
const [file] = args;
if (!file) {
  console.error("node bench.mjs FILE [--dict DICT] [--threads 1,2,4,8] [--seconds 2]");
  process.exit(2);
}
const input = await readFile(file);
const dictionaries = dict ? [await readFile(dict)] : [];
const module = await compileModule();
const median = (xs) => [...xs].sort((a, b) => a - b)[xs.length >> 1];
let expected = null;
for (const threads of threadCounts) {
  const decoder = await createDecoder({ threads, dictionaries, module });
  const first = await decoder.decodeTimed(input);
  const digest = createHash("sha256").update(first.output).digest("hex");
  expected ??= digest;
  if (digest !== expected) throw new Error(`${threads} threads decoded other bytes`);
  const decodeMs = [];
  const callMs = [];
  const until = performance.now() + seconds * 1000;
  while (decodeMs.length < 3 || performance.now() < until) {
    const { ms, decodeMs: d } = await decoder.decodeTimed(input);
    decodeMs.push(d);
    callMs.push(ms);
  }
  await decoder.close();
  console.log(
    JSON.stringify({
      file,
      threads,
      reps: decodeMs.length,
      decode_ms: +median(decodeMs).toFixed(2),
      decode_min_ms: +Math.min(...decodeMs).toFixed(2),
      call_ms: +median(callMs).toFixed(2),
      bytes_in: input.length,
      bytes_out: first.output.length,
    }),
  );
}
process.exit(0);
