// node test.mjs: decode zstd's own test frames, the crate's decodecorpus files, on 1, 2 and 4
// threads, against the originals beside them; then all of them at once, and a damaged frame.
import { readdir, readFile } from "node:fs/promises";
import { createDecoder } from "./js/ruzstd-mt.js";
import { compileModule } from "./js/env.js";

const dir = new URL("../../decodecorpus_files/", import.meta.url);
const names = (await readdir(dir)).filter((n) => n.endsWith(".zst")).sort();
const cases = [];
for (const name of names) {
  const original = await readFile(new URL(name.slice(0, -4), dir)).catch(() => null);
  if (original) cases.push({ name, input: await readFile(new URL(name, dir)), original });
}
const same = (a, b) => a.length === b.length && Buffer.from(a.buffer, a.byteOffset, a.length).equals(b);
const module = await compileModule();
let failed = 0;
for (const threads of [1, 2, 4]) {
  const decoder = await createDecoder({ threads, module });
  for (const { name, input, original } of cases) {
    const output = await decoder.decode(input);
    if (!same(output, original)) {
      console.log(`FAIL ${name} on ${threads} threads`);
      failed++;
    }
  }
  const all = Buffer.concat(cases.map((c) => c.input));
  if (!same(await decoder.decode(all), Buffer.concat(cases.map((c) => c.original)))) {
    console.log(`FAIL all together on ${threads} threads`);
    failed++;
  }
  const damaged = Buffer.from(cases[cases.length - 1].input);
  damaged[damaged.length >> 1] ^= 0x55;
  const error = await decoder.decode(damaged).then(() => null, (e) => e);
  console.log(`${threads} threads: ${cases.length} files, damaged frame: ${error ? error.message : "no error"}`);
  await decoder.close();
}
console.log(failed ? `${failed} failed` : "all decoded to their originals");
process.exit(failed ? 1 : 0);
