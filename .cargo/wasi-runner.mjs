// The runner of wasm32-wasip1 builds (see config.toml), under Node's V8: the module with its
// arguments, the working directory as its root, as `cargo test` runs a test binary from the
// package's directory, whose files the tests read.
import { readFile } from 'node:fs/promises';
import { WASI } from 'node:wasi';
const [module, ...args] = process.argv.slice(2);
const wasi = new WASI({ version: 'preview1', args: [module, ...args], env: process.env, preopens: { '/': process.cwd() }, returnOnExit: true });
const instance = await WebAssembly.instantiate(await WebAssembly.compile(await readFile(module)), wasi.getImportObject());
process.exitCode = wasi.start(instance);
