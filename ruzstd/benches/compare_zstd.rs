//! Compares ruzstd against the `zstd` crate (bindings to the C libzstd).
//!
//! Decompression: both decoders decode the same frame, produced by libzstd at level 3
//! (the level most real-world .zst files are made with).
//! Compression: ruzstd `Fastest` is compared to libzstd level 1, which it aims to match.
//! The compressed sizes are printed once so the ratio can be compared as well.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use ruzstd::decoding::FrameDecoder;
use ruzstd::encoding::{compress_to_vec, CompressionLevel};
use std::path::Path;

/// Concatenation of this crate's own source files: realistic, text-like input.
fn rust_source() -> Vec<u8> {
    fn collect(dir: &Path, out: &mut Vec<u8>) {
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        // Sort so the input is identical between runs.
        entries.sort();
        for path in entries {
            if path.is_dir() {
                collect(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.extend(std::fs::read(&path).unwrap());
            }
        }
    }
    let mut out = Vec::new();
    collect(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut out);
    out
}

fn inputs() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("rust-source", rust_source()),
        (
            "decodecorpus-z000033",
            include_bytes!("../decodecorpus_files/z000033").to_vec(),
        ),
    ]
}

fn decompress(c: &mut Criterion) {
    let mut group = c.benchmark_group("decompress");
    for (name, data) in inputs() {
        let compressed = zstd::bulk::compress(&data, 3).unwrap();
        let mut target = vec![0u8; data.len()];
        group.throughput(Throughput::Bytes(data.len() as u64));

        let mut fr = FrameDecoder::new();
        assert_eq!(fr.decode_all(&compressed, &mut target).unwrap(), data.len());
        assert_eq!(target, data);
        group.bench_with_input(BenchmarkId::new("ruzstd", name), &compressed, |b, src| {
            b.iter(|| fr.decode_all(src, &mut target).unwrap())
        });

        let mut dctx = zstd::bulk::Decompressor::new().unwrap();
        group.bench_with_input(BenchmarkId::new("zstd", name), &compressed, |b, src| {
            b.iter(|| dctx.decompress_to_buffer(src, &mut target).unwrap())
        });
    }
    group.finish();
}

fn compress(c: &mut Criterion) {
    let mut group = c.benchmark_group("compress");
    for (name, data) in inputs() {
        group.throughput(Throughput::Bytes(data.len() as u64));

        let ruzstd_size = compress_to_vec(data.as_slice(), CompressionLevel::Fastest).len();
        let zstd_size = zstd::bulk::compress(&data, 1).unwrap().len();
        println!(
            "{name}: {} bytes -> ruzstd Fastest {ruzstd_size} ({:.2}%), zstd level 1 {zstd_size} ({:.2}%)",
            data.len(),
            100.0 * ruzstd_size as f64 / data.len() as f64,
            100.0 * zstd_size as f64 / data.len() as f64,
        );

        group.bench_with_input(BenchmarkId::new("ruzstd_fastest", name), &data, |b, d| {
            b.iter(|| compress_to_vec(d.as_slice(), CompressionLevel::Fastest))
        });

        let mut cctx = zstd::bulk::Compressor::new(1).unwrap();
        group.bench_with_input(BenchmarkId::new("zstd_level1", name), &data, |b, d| {
            b.iter(|| cctx.compress(d).unwrap())
        });
    }
    group.finish();
}

criterion_group!(benches, decompress, compress);
criterion_main!(benches);
