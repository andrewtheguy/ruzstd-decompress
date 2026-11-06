//! The decode benchmark: `build.sh` makes it a crate of its own per variant, depending on that variant's ruzstd-decompress.
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
use ruzstd_decompress::decoding::{
    BlockDecodingStrategy, Dictionary, FrameDecoder, StreamingDecoder,
};
use std::hint::black_box;
use std::io::{self, Read, Write};
use std::time::{Duration, Instant};

const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];
const STILL_WINDOW_MOST: u64 = 8 << 20;

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

/// What `WlshareTiles` keeps from one tile to the next, for the whole session: one
/// decoder for every planar tile and one for the still stream, and the bitmap's
/// buffer. Made once for all the passes, as zDesk makes it once for a session, so
/// that a pass does not grow their buffers from empty again.
struct Session {
    bitmap: Vec<u8>,
    planar: FrameDecoder,
    still: FrameDecoder,
}

impl Session {
    fn new() -> Session {
        let mut still = FrameDecoder::new();
        still.set_max_window_size(STILL_WINDOW_MOST);
        Session { bitmap: Vec::new(), planar: FrameDecoder::new(), still }
    }
}

/// The tiles decoded as `WlshareTiles::planar` and `::still` decode them, by the
/// session's decoders, each reset for a tile or a new stream rather than made
/// anew, the bitmap left in its buffer. Returns the bytes out and, with `check`,
/// a hash over every bitmap.
fn replay(data: &[u8], tiles: &[Tile], session: &mut Session, check: bool) -> (u64, u64) {
    let Session { bitmap, planar, still } = session;
    let mut in_still = false;
    let (mut out, mut hash) = (0u64, 0xcbf29ce484222325u64);
    for t in tiles {
        let body = &data[t.at..t.at + t.len];
        let most = planar_most(t.w, t.h);
        if t.kind == 0 {
            bitmap.clear();
            let mut source = body;
            let frame = StreamingDecoder::new_with_decoder(&mut source, &mut *planar).unwrap();
            let mut capped = Read::take(frame, most as u64 + 1);
            capped.read_to_end(bitmap).unwrap();
            assert!(bitmap.len() <= most);
            drop(capped);
            assert!(source.is_empty());
        } else {
            let mut source = body;
            if body.starts_with(&ZSTD_MAGIC) {
                still.init(&mut source).unwrap();
                in_still = true;
            }
            assert!(in_still, "a body that continues no stream");
            let frame = &mut *still;
            while !source.is_empty() {
                let header = u32::from_le_bytes([source[0], source[1], source[2], 0]);
                assert!(header & 1 == 0, "a last block");
                let content = if (header >> 1) & 3 == 1 { 1 } else { (header >> 3) as usize };
                let block = &source[..3 + content];
                let (read, _) = frame.decode_from_to(block, &mut []).unwrap();
                assert_eq!(read, block.len());
                source = &source[block.len()..];
                assert!(frame.can_collect() <= most);
            }
            // The body's blocks' bytes, all collected as soon as they are decoded.
            bitmap.clear();
            frame.collect_to_writer(&mut *bitmap).unwrap();
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

/// One whole frame, a megabyte of blocks at a time, collected to a sink.
fn frame(data: &[u8]) -> (u64, bool) {
    let mut source = data;
    let mut dec = FrameDecoder::new();
    dec.reset(&mut source).unwrap();
    let mut sink = Count(0);
    loop {
        dec.decode_blocks(&mut source, BlockDecodingStrategy::UptoBytes(1 << 20)).unwrap();
        dec.collect_to_writer(&mut sink).unwrap();
        if dec.is_finished() && dec.can_collect() == 0 {
            break;
        }
    }
    let ok = match (dec.get_checksum_from_data(), dec.get_calculated_checksum()) {
        (Some(a), Some(b)) => a == b,
        (None, _) => true,
        _ => false,
    };
    (sink.0, ok)
}

/// Independent frames back to back, as small records are kept, each compressed
/// alone (with the dictionary, if the decoder has one): each read whole into one
/// reused buffer through a `StreamingDecoder` over one `FrameDecoder`, reset for
/// each. Returns the frames, the bytes out and, with `check`, a hash over every
/// frame's content.
fn frames(data: &[u8], dec: &mut FrameDecoder, check: bool) -> (u64, u64, u64) {
    let mut record: Vec<u8> = Vec::new();
    let mut source = data;
    let (mut n, mut out, mut hash) = (0u64, 0u64, 0xcbf29ce484222325u64);
    while !source.is_empty() {
        record.clear();
        let mut frame = StreamingDecoder::new_with_decoder(&mut source, &mut *dec).unwrap();
        frame.read_to_end(&mut record).unwrap();
        drop(frame);
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
            // Again by the same, reused, decoders, as the timed passes are: the same bitmaps.
            let again = replay(&data, &tiles, &mut session, true);
            assert_eq!(again, (out, hash), "a reused session decoded differently");
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
            let mut dec = FrameDecoder::new();
            if let Some(dict) = args.get(3) {
                let dict = Dictionary::decode_dict(&std::fs::read(dict).unwrap()).unwrap();
                dec.add_dict(dict).unwrap();
            }
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
