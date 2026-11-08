//! `FrameDecoder::decode_all_parallel` timed on rayon pools of each number of threads:
//!
//! ```text
//! cargo run --release --features rayon --example decode_parallel -- FILE [--dict DICT] [--threads 1,2,4,8] [--seconds 2]
//! ```
//!
//! Decodes FILE again and again on each pool, for at least SECONDS and 3 times, into one buffer
//! kept from decode to decode, and prints a JSON line for each: the median and the least ms of a
//! decode. Every decode must give the bytes the first did. The same lines as
//! `examples/wasm-threads/bench.mjs` prints for the module in Node.
use ruzstd_decompress::decoding::{Dictionary, FrameDecoder};
use std::time::{Duration, Instant};

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut option = |name: &str| {
        let at = args.iter().position(|a| a == name)?;
        let Some(value) = args.get(at + 1).cloned() else {
            eprintln!("{name} needs a value");
            std::process::exit(2);
        };
        args.drain(at..at + 2);
        Some(value)
    };
    let dict = option("--dict");
    let threads = option("--threads").unwrap_or_else(|| {
        let most = std::thread::available_parallelism().map_or(1, |n| n.get());
        format!("1,2,4,{most}")
    });
    let seconds: f64 = option("--seconds").map_or(2.0, |s| s.parse().expect("seconds"));
    let Some(file) = args.first() else {
        eprintln!("decode_parallel FILE [--dict DICT] [--threads 1,2,4,8] [--seconds 2]");
        std::process::exit(2);
    };
    let input = std::fs::read(file).expect("FILE");
    let mut decoder = FrameDecoder::new();
    if let Some(dict) = dict {
        let raw = std::fs::read(dict).expect("DICT");
        decoder
            .add_dict(Dictionary::decode_dict(&raw).expect("a dictionary"))
            .unwrap();
    }
    let mut expected: Option<Vec<u8>> = None;
    let mut output = Vec::new();
    for threads in threads
        .split(',')
        .map(|t| t.parse::<usize>().expect("threads"))
    {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        let mut decode = || {
            output.clear();
            let start = Instant::now();
            pool.install(|| decoder.decode_all_parallel(&input, &mut output))
                .expect("FILE decodes");
            start.elapsed().as_secs_f64() * 1e3
        };
        decode();
        let mut times = Vec::new();
        let until = Instant::now() + Duration::from_secs_f64(seconds);
        while times.len() < 3 || Instant::now() < until {
            times.push(decode());
        }
        match &expected {
            Some(expected) => assert!(*expected == output, "{threads} threads decoded other bytes"),
            None => expected = Some(output.clone()),
        }
        times.sort_by(f64::total_cmp);
        println!(
            "{{\"file\":{file:?},\"threads\":{threads},\"reps\":{},\"decode_ms\":{:.2},\"decode_min_ms\":{:.2},\"bytes_in\":{},\"bytes_out\":{}}}",
            times.len(),
            times[times.len() / 2],
            times[0],
            input.len(),
            output.len()
        );
    }
}
