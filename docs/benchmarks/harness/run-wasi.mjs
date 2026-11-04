// Runs a wasm32-wasip1 module under Node's V8, as the page's engine: run-wasi.mjs MODULE ARGS...
import { readFile } from 'node:fs/promises';
import { WASI } from 'node:wasi';
const [module, ...args] = process.argv.slice(2);
const wasi = new WASI({ version: 'preview1', args: ['decode_bench', ...args], preopens: { '/': '/' }, returnOnExit: true });
const instance = await WebAssembly.instantiate(await WebAssembly.compile(await readFile(module)), wasi.getImportObject());
process.exitCode = wasi.start(instance);
