//! The libzstd reference for the scratch decode benchmark: the same workloads,
//! timing and output as `decode_bench.rs`, the decoders libzstd's.
//!
//! ```text
//! decode_bench tiles FILE [MAX_CORPUS_BYTES]   wlshare's lossless tile bodies, replayed as zDesk's page decodes them
//! decode_bench frame FILE                      one whole .zst frame, a block at a time to a sink
//! decode_bench frames FILE [DICT]              small frames back to back, each decoded alone, with DICT if given
//! ```
//!
//! Prints one JSON line: reps, median and min ms, bytes in and out, and a check
//! (an FNV hash of every tile's bitmap or every small frame's content, or the
//! frame's checksum matching).
use zstd::stream::raw::{Decoder, InBuffer, Operation, OutBuffer};
use std::hint::black_box;
use std::io::{self, Write};
use std::time::{Duration, Instant};

const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];
/// 8 MiB, the still stream's promised window most, as a window log.
const STILL_WINDOW_LOG_MOST: u32 = 23;

struct Tile {
    kind: u8,
    w: u16,
    h: u16,
    at: usize,
    len: usize,
}

fn planar_most(w: u16, h: u16) -> usize {
    1 + 4 * 2 * usize::from(w) * usize::from(h) + 1
}

fn load_tiles(path: &str, most: usize) -> (Vec<u8>, Vec<Tile>) {
    let data = std::fs::read(path).unwrap();
    let mut tiles = Vec::new();
    let mut at = 0;
    while at + 9 <= data.len() {
        let len = u32::from_le_bytes(data[at + 5..at + 9].try_into().unwrap()) as usize;
        if at + 9 + len > data.len() || at + 9 + len > most {
            break;
        }
        tiles.push(Tile {
            kind: data[at],
            w: u16::from_le_bytes([data[at + 1], data[at + 2]]),
            h: u16::from_le_bytes([data[at + 3], data[at + 4]]),
            at: at + 9,
            len,
        });
        at += 9 + len;
    }
    // A still stream cut short at the end is fine: every record is a whole body.
    (data, tiles)
}

/// The decoders and the bitmap's buffer, made once for all the passes, as the
/// ruzstd-decompress harness's `Session` is.
struct Session {
    planar: zstd::bulk::Decompressor<'static>,
    bitmap: Vec<u8>,
    still: Decoder<'static>,
}

impl Session {
    fn new() -> Session {
        let mut still = Decoder::new().unwrap();
        still.set_parameter(zstd::zstd_safe::DParameter::WindowLogMax(STILL_WINDOW_LOG_MOST)).unwrap();
        Session { planar: zstd::bulk::Decompressor::new().unwrap(), bitmap: Vec::new(), still }
    }
}

/// The tiles decoded by libzstd: a planar tile's frame in one call into a
/// buffer of the most it can be, and the still stream's bodies fed in order to
/// one streaming context, the bitmap what each body's blocks put out. One
/// context for every planar tile and one for the still stream, reset for a new
/// stream, as the ruzstd-decompress harness keeps its decoders.
fn replay(data: &[u8], tiles: &[Tile], session: &mut Session, check: bool) -> (u64, u64) {
    let Session { planar, bitmap, still } = session;
    let mut in_still = false;
    let (mut out, mut hash) = (0u64, 0xcbf29ce484222325u64);
    for t in tiles {
        let body = &data[t.at..t.at + t.len];
        let most = planar_most(t.w, t.h);
        bitmap.clear();
        bitmap.reserve(most);
        if t.kind == 0 {
            planar.decompress_to_buffer(body, bitmap).unwrap();
        } else {
            if body.starts_with(&ZSTD_MAGIC) {
                // A session reset: the window's most stays set.
                still.reinit().unwrap();
                in_still = true;
            }
            assert!(in_still, "a body that continues no stream");
            let d = &mut *still;
            bitmap.resize(most, 0);
            let mut input = InBuffer::around(body);
            let mut output = OutBuffer::around(&mut bitmap[..]);
            while input.pos() < body.len() {
                d.run(&mut input, &mut output).unwrap();
                assert!(output.pos() < most, "a body of more than a bitmap");
            }
            // What the blocks decoded may still be held: flush it out.
            loop {
                let before = output.pos();
                d.run(&mut InBuffer::around(&[]), &mut output).unwrap();
                if output.pos() == before {
                    break;
                }
            }
            let n = output.pos();
            bitmap.truncate(n);
        }
        out += bitmap.len() as u64;
        if check {
            for &x in bitmap.iter() {
                hash = (hash ^ u64::from(x)).wrapping_mul(0x100000001b3);
            }
        }
        black_box(&bitmap);
    }
    (out, hash)
}

struct Count(u64);
impl Write for Count {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0 += buf.len() as u64;
        black_box(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// One whole frame, streamed to a sink; libzstd checks the checksum itself.
fn frame(data: &[u8]) -> (u64, bool) {
    let mut sink = Count(0);
    zstd::stream::copy_decode(data, &mut sink).unwrap();
    (sink.0, true)
}

/// Independent frames back to back, each decoded in one call into one reused
/// buffer of its content size, by one context (holding the dictionary, if one is
/// given), as the ruzstd-decompress harness keeps its decoder.
fn frames(data: &[u8], dec: &mut zstd::bulk::Decompressor, check: bool) -> (u64, u64, u64) {
    use zstd::zstd_safe;
    let mut record: Vec<u8> = Vec::new();
    let mut source = data;
    let (mut n, mut out, mut hash) = (0u64, 0u64, 0xcbf29ce484222325u64);
    while !source.is_empty() {
        let len = zstd_safe::find_frame_compressed_size(source).unwrap();
        let size = zstd_safe::get_frame_content_size(source).unwrap().expect("a content size");
        record.clear();
        record.reserve(size as usize);
        dec.decompress_to_buffer(&source[..len], &mut record).unwrap();
        source = &source[len..];
        n += 1;
        out += record.len() as u64;
        if check {
            for &x in &record {
                hash = (hash ^ u64::from(x)).wrapping_mul(0x100000001b3);
            }
        }
        black_box(&record);
    }
    (n, out, hash)
}

fn time(mut f: impl FnMut()) -> Vec<f64> {
    let all = Instant::now();
    let mut times = Vec::new();
    while times.len() < 3 || (all.elapsed() < Duration::from_secs(2) && times.len() < 5000) {
        let t = Instant::now();
        f();
        times.push(t.elapsed().as_secs_f64() * 1e3);
    }
    times.sort_by(f64::total_cmp);
    times
}

/// glibc's malloc moves its mmap and trim thresholds as blocks are freed, so whether a pass's new
/// decoder gets its buffers from the heap or as fresh pages to fault in turns on the build's
/// allocation pattern: up to twice the time of a small workload, and a different answer for two
/// builds of the same code. Both thresholds fixed keep them on the heap; either one alone does not.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn fix_malloc_thresholds() {
    unsafe extern "C" {
        fn mallopt(param: i32, value: i32) -> i32;
    }
    const M_TRIM_THRESHOLD: i32 = -1;
    const M_MMAP_THRESHOLD: i32 = -3;
    // SAFETY: mallopt only sets malloc's tuning parameters. The values are the ones the
    // measurements of 2026-11-04 were made with, through GLIBC_TUNABLES.
    let set = unsafe {
        mallopt(M_MMAP_THRESHOLD, 64 << 20) == 1 && mallopt(M_TRIM_THRESHOLD, 256 << 20) == 1
    };
    assert!(set, "mallopt refused the thresholds");
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn fix_malloc_thresholds() {}

fn main() {
    fix_malloc_thresholds();
    let args: Vec<String> = std::env::args().collect();
    let (mode, path) = (args[1].as_str(), args[2].as_str());
    let (times, bytes_in, bytes_out, check) = match mode {
        "tiles" => {
            let most = args.get(3).map_or(usize::MAX, |s| s.parse().unwrap());
            let (data, tiles) = load_tiles(path, most);
            let bytes_in: usize = tiles.iter().map(|t| t.len).sum();
            let mut session = Session::new();
            let (out, hash) = replay(&data, &tiles, &mut session, true);
            let times = time(|| {
                black_box(replay(&data, &tiles, &mut session, false));
            });
            (times, bytes_in as u64, out, format!("\"{hash:016x}\""))
        }
        "frame" => {
            let data = std::fs::read(path).unwrap();
            let (out, ok) = frame(&data);
            assert!(ok, "checksum mismatch");
            let times = time(|| {
                black_box(frame(&data));
            });
            (times, data.len() as u64, out, "\"checksum ok\"".to_string())
        }
        "frames" => {
            let data = std::fs::read(path).unwrap();
            let mut dec = match args.get(3) {
                Some(dict) => zstd::bulk::Decompressor::with_dictionary(&std::fs::read(dict).unwrap()),
                None => zstd::bulk::Decompressor::new(),
            }
            .unwrap();
            let (n, out, hash) = frames(&data, &mut dec, true);
            let times = time(|| {
                black_box(frames(&data, &mut dec, false));
            });
            (times, data.len() as u64, out, format!("\"{n} frames {hash:016x}\""))
        }
        _ => panic!("tiles, frame or frames"),
    };
    let name = path.rsplit('/').next().unwrap();
    println!(
        "{{\"mode\":\"{mode}\",\"file\":\"{name}\",\"reps\":{},\"median_ms\":{:.3},\"min_ms\":{:.3},\"in\":{bytes_in},\"out\":{bytes_out},\"check\":{check}}}",
        times.len(),
        times[times.len() / 2],
        times[0]
    );
}
