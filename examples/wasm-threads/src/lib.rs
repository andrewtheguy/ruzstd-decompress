//! ruzstd-decompress on several threads in WebAssembly.
//!
//! A page has no threads for the module to spawn: a thread is a worker, started by JavaScript,
//! running an instance of this module on the one shared memory. So the rayon pool's threads are
//! seats the workers take ([`run_pool_thread`]) and the pool is made of them when a [`Decoder`]
//! is made. Each decode then runs [`FrameDecoder::decode_all_parallel`] in that pool: frames side
//! by side, and the blocks of a frame where there are fewer frames than threads.
//!
//! Every call into a decoder waits for the pool's threads, which a browser's main thread may not
//! (`Atomics.wait` is refused there), so the decoder lives in a worker of its own:
//! `js/ruzstd-mt.js` sets all of it up, in a browser or in Node.

use std::io;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Mutex, OnceLock};

use rayon::{ThreadBuilder, ThreadPool, ThreadPoolBuilder};
use ruzstd_decompress::decoding::{Dictionary, FrameDecoder, decompressed_bound};
use wasm_bindgen::prelude::*;

/// The pool's threads on their way from the pool that makes them to the workers that run them.
struct Seats {
    offer: Mutex<Sender<ThreadBuilder>>,
    take: Mutex<Receiver<ThreadBuilder>>,
}

fn seats() -> &'static Seats {
    static SEATS: OnceLock<Seats> = OnceLock::new();
    SEATS.get_or_init(|| {
        let (offer, take) = channel();
        Seats {
            offer: Mutex::new(offer),
            take: Mutex::new(take),
        }
    })
}

/// Run one of a pool's threads on the worker that calls this: it waits for a [`Decoder`] to make
/// its pool, and returns once that decoder is freed (`free()` in JavaScript), when the worker can
/// be ended.
#[wasm_bindgen(js_name = runPoolThread)]
pub fn run_pool_thread() {
    let thread = seats()
        .take
        .lock()
        .expect("no thread panics holding a seat")
        .recv();
    if let Ok(thread) = thread {
        thread.run();
    }
}

/// A zstd decoder whose decodes run on a pool of threads.
#[wasm_bindgen]
extern "C" {
    /// `performance.now()`, which a browser's workers and Node both have.
    #[wasm_bindgen(js_namespace = performance)]
    fn now() -> f64;
}

#[wasm_bindgen]
pub struct Decoder {
    decoder: FrameDecoder,
    pool: ThreadPool,
    /// How long the last decode took, in milliseconds, without its input's copy in or its
    /// output's copy out.
    last_decode_ms: f64,
}

#[wasm_bindgen]
impl Decoder {
    /// A decoder on `threads` threads: the calling one, and `threads - 1` workers that have
    /// called (or will call) [`run_pool_thread`]. The calling thread takes part only while it
    /// decodes, so it must be the one that calls [`Decoder::decode`].
    #[wasm_bindgen(constructor)]
    pub fn new(threads: usize) -> Result<Decoder, JsError> {
        let offer = seats()
            .offer
            .lock()
            .expect("no thread panics holding a seat")
            .clone();
        let pool = ThreadPoolBuilder::new()
            .num_threads(threads.max(1))
            .use_current_thread()
            .spawn_handler(move |thread| {
                offer
                    .send(thread)
                    .map_err(|_| io::Error::other("the pool's seats are gone"))
            })
            .build()
            .map_err(|e| JsError::new(&e.to_string()))?;
        Ok(Decoder {
            decoder: FrameDecoder::new(),
            pool,
            last_decode_ms: 0.0,
        })
    }

    /// The threads it decodes on.
    pub fn threads(&self) -> usize {
        self.pool.current_num_threads()
    }

    /// Add a dictionary (the raw bytes of one, as `zstd --train` writes it), for the frames that
    /// name its id.
    #[wasm_bindgen(js_name = addDictionary)]
    pub fn add_dictionary(&mut self, raw: &[u8]) -> Result<(), JsError> {
        let dict = Dictionary::decode_dict(raw).map_err(|e| JsError::new(&e.to_string()))?;
        self.decoder
            .add_dict(dict)
            .map_err(|e| JsError::new(&e.to_string()))
    }

    /// Raise the largest window a frame may ask for (128 MiB unless raised): a frame's window is
    /// memory the decoder holds while it decodes it.
    #[wasm_bindgen(js_name = setMaxWindowSize)]
    pub fn set_max_window_size(&mut self, bytes: f64) {
        self.decoder.set_max_window_size(bytes as u64);
    }

    /// Every frame of `input`, decoded, each one's checksum checked: into room for what its
    /// headers say it decodes to, up to 1 GiB.
    pub fn decode(&mut self, input: &[u8]) -> Result<Vec<u8>, JsError> {
        let bound = decompressed_bound(input).map_err(|e| JsError::new(&e.to_string()))?;
        if bound > 1 << 30 {
            return Err(JsError::new(&format!(
                "the frames say they decode to {bound} bytes, past 1 GiB"
            )));
        }
        let mut output = Vec::with_capacity(bound as usize);
        let start = now();
        let result = self
            .pool
            .install(|| self.decoder.decode_all_parallel(input, &mut output));
        self.last_decode_ms = now() - start;
        result.map_err(|e| JsError::new(&e.to_string()))?;
        Ok(output)
    }

    /// How long the last decode took, in milliseconds, in the module: without the copy of its
    /// input into the module's memory, or of its output out of it.
    #[wasm_bindgen(js_name = lastDecodeMs)]
    pub fn last_decode_ms(&self) -> f64 {
        self.last_decode_ms
    }
}
