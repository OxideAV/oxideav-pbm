//! Criterion benchmarks for the Netpbm decoder hot paths.
//!
//! Round 176 (depth-mode benchmarks): `oxideav-pbm` is saturated at
//! ~95% decode + encode per the workspace README — all eight Netpbm
//! magics (P1-P7) at 1/8/16-bit, the six standard PAM `TUPLTYPE`s,
//! every legal comment / whitespace placement, and a cargo-fuzz
//! harness for the parser + encoder all shipped through rounds
//! 1..r171. Per the workspace "saturated → fuzz/bench/profile" memo,
//! this round adds criterion benches mirroring the png / flac shape
//! so future optimisation rounds (SIMD pack/unpack for P4, faster
//! ASCII tokenizer for P1/P2/P3, byte-swap kernel for 16-bit P5/P6)
//! can A/B-test changes against a stable r176 baseline.
//!
//! This file covers the **decoder**; sibling files cover `encode` and
//! `roundtrip`.
//!
//! Each scenario synthesises a fresh Netpbm file on the fly via the
//! public `encode_pbm` / `encode_pbm_ascii` API and iterates
//! `decode_pbm` on the encoded bytes. No fixture files are committed;
//! every input is reproducible from the bench source.
//!
//!   - **decode_p4_mono_640x480**: 640×480 binary PBM — exercises the
//!     bit-unpack hot path on a VGA bitmap (8 px per source byte).
//!   - **decode_p5_gray8_640x480**: 640×480 binary PGM 8-bit — single
//!     `Vec::extend_from_slice` per row, the cheap baseline.
//!   - **decode_p5_gray16_640x480**: 640×480 binary PGM 16-bit —
//!     exercises the big-endian-to-little-endian sample swap.
//!   - **decode_p6_rgb24_640x480**: 640×480 binary PPM 8-bit — the
//!     natural-image baseline at 3 bytes/pixel.
//!   - **decode_p6_rgb48_320x240**: 320×240 binary PPM 16-bit — wider
//!     sample swap at 6 bytes/pixel.
//!   - **decode_p7_rgba_320x240**: 320×240 PAM `RGB_ALPHA` 8-bit —
//!     header key/value parsing + 4-byte/pixel copy.
//!   - **decode_p7_rgba64_320x240**: 320×240 PAM `RGB_ALPHA` 16-bit —
//!     widest pixel format (8 bytes/pixel) plus byte swap.
//!   - **decode_p1_mono_320x240**: 320×240 plain-ASCII PBM — the
//!     tokenizer hot path with every digit on its own column.
//!   - **decode_p2_gray8_320x240**: 320×240 plain-ASCII PGM — tests
//!     ASCII `Gray8` decode (number parser + whitespace skipper).
//!   - **decode_p3_rgb24_320x240**: 320×240 plain-ASCII PPM — three
//!     numbers per pixel, the densest ASCII path.
//!
//! Run with:
//!     cargo bench -p oxideav-pbm --bench decode

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

use oxideav_pbm::{
    decode_pbm, encode_pbm, encode_pbm_ascii, encode_pbm_with_format, PbmEncodeFormat, PbmImage,
    PbmPixelFormat, PbmPlane,
};

/// xorshift32 — keeps the bench input from being trivially compressible
/// so the decoder's byte-walk actually does work proportional to image
/// size instead of being CPU-cache-trivial.
fn xorshift_byte(state: &mut u32) -> u8 {
    *state ^= *state << 13;
    *state ^= *state >> 17;
    *state ^= *state << 5;
    (*state & 0xff) as u8
}

fn build_mono(width: u32, height: u32) -> PbmImage {
    let w = width as usize;
    let h = height as usize;
    let row_bytes = w.div_ceil(8);
    let mut data = vec![0u8; row_bytes * h];
    let mut state: u32 = 0x1234_5678;
    for byte in data.iter_mut() {
        *byte = xorshift_byte(&mut state);
    }
    PbmImage {
        width,
        height,
        pixel_format: PbmPixelFormat::MonoBlack,
        planes: vec![PbmPlane {
            stride: row_bytes,
            data,
        }],
        pts: None,
    }
}

fn build_gray8(width: u32, height: u32) -> PbmImage {
    let w = width as usize;
    let h = height as usize;
    let mut data = vec![0u8; w * h];
    let mut state: u32 = 0x2345_6789;
    for byte in data.iter_mut() {
        *byte = xorshift_byte(&mut state);
    }
    PbmImage {
        width,
        height,
        pixel_format: PbmPixelFormat::Gray8,
        planes: vec![PbmPlane { stride: w, data }],
        pts: None,
    }
}

fn build_gray16(width: u32, height: u32) -> PbmImage {
    let w = width as usize;
    let h = height as usize;
    let mut data = vec![0u8; w * h * 2];
    let mut state: u32 = 0x3456_789a;
    for byte in data.iter_mut() {
        *byte = xorshift_byte(&mut state);
    }
    PbmImage {
        width,
        height,
        pixel_format: PbmPixelFormat::Gray16Le,
        planes: vec![PbmPlane {
            stride: w * 2,
            data,
        }],
        pts: None,
    }
}

fn build_rgb24(width: u32, height: u32) -> PbmImage {
    let w = width as usize;
    let h = height as usize;
    let mut data = vec![0u8; w * h * 3];
    let mut state: u32 = 0x4567_89ab;
    for byte in data.iter_mut() {
        *byte = xorshift_byte(&mut state);
    }
    PbmImage {
        width,
        height,
        pixel_format: PbmPixelFormat::Rgb24,
        planes: vec![PbmPlane {
            stride: w * 3,
            data,
        }],
        pts: None,
    }
}

fn build_rgb48(width: u32, height: u32) -> PbmImage {
    let w = width as usize;
    let h = height as usize;
    let mut data = vec![0u8; w * h * 6];
    let mut state: u32 = 0x5678_9abc;
    for byte in data.iter_mut() {
        *byte = xorshift_byte(&mut state);
    }
    PbmImage {
        width,
        height,
        pixel_format: PbmPixelFormat::Rgb48Le,
        planes: vec![PbmPlane {
            stride: w * 6,
            data,
        }],
        pts: None,
    }
}

fn build_rgba(width: u32, height: u32) -> PbmImage {
    let w = width as usize;
    let h = height as usize;
    let mut data = vec![0u8; w * h * 4];
    let mut state: u32 = 0x6789_abcd;
    for byte in data.iter_mut() {
        *byte = xorshift_byte(&mut state);
    }
    PbmImage {
        width,
        height,
        pixel_format: PbmPixelFormat::Rgba,
        planes: vec![PbmPlane {
            stride: w * 4,
            data,
        }],
        pts: None,
    }
}

fn build_rgba64(width: u32, height: u32) -> PbmImage {
    let w = width as usize;
    let h = height as usize;
    let mut data = vec![0u8; w * h * 8];
    let mut state: u32 = 0x789a_bcde;
    for byte in data.iter_mut() {
        *byte = xorshift_byte(&mut state);
    }
    PbmImage {
        width,
        height,
        pixel_format: PbmPixelFormat::Rgba64Le,
        planes: vec![PbmPlane {
            stride: w * 8,
            data,
        }],
        pts: None,
    }
}

fn bench_decode_p4_mono_640x480(c: &mut Criterion) {
    let image = build_mono(640, 480);
    let bytes = encode_pbm(&image).expect("encode_pbm P4");
    let mut g = c.benchmark_group("decode_p4_mono_640x480");
    g.throughput(Throughput::Bytes((640 * 480 / 8) as u64));
    g.bench_function(BenchmarkId::from_parameter("p4/640x480"), |b| {
        b.iter(|| decode_pbm(criterion::black_box(&bytes)).expect("decode_pbm"));
    });
    g.finish();
}

fn bench_decode_p5_gray8_640x480(c: &mut Criterion) {
    let image = build_gray8(640, 480);
    let bytes = encode_pbm(&image).expect("encode_pbm P5");
    let mut g = c.benchmark_group("decode_p5_gray8_640x480");
    g.throughput(Throughput::Bytes((640 * 480) as u64));
    g.bench_function(BenchmarkId::from_parameter("p5/640x480"), |b| {
        b.iter(|| decode_pbm(criterion::black_box(&bytes)).expect("decode_pbm"));
    });
    g.finish();
}

fn bench_decode_p5_gray16_640x480(c: &mut Criterion) {
    let image = build_gray16(640, 480);
    let bytes = encode_pbm(&image).expect("encode_pbm P5 16-bit");
    let mut g = c.benchmark_group("decode_p5_gray16_640x480");
    g.throughput(Throughput::Bytes((640 * 480 * 2) as u64));
    g.bench_function(BenchmarkId::from_parameter("p5/16/640x480"), |b| {
        b.iter(|| decode_pbm(criterion::black_box(&bytes)).expect("decode_pbm"));
    });
    g.finish();
}

fn bench_decode_p6_rgb24_640x480(c: &mut Criterion) {
    let image = build_rgb24(640, 480);
    let bytes = encode_pbm(&image).expect("encode_pbm P6");
    let mut g = c.benchmark_group("decode_p6_rgb24_640x480");
    g.throughput(Throughput::Bytes((640 * 480 * 3) as u64));
    g.bench_function(BenchmarkId::from_parameter("p6/640x480"), |b| {
        b.iter(|| decode_pbm(criterion::black_box(&bytes)).expect("decode_pbm"));
    });
    g.finish();
}

fn bench_decode_p6_rgb48_320x240(c: &mut Criterion) {
    let image = build_rgb48(320, 240);
    let bytes = encode_pbm(&image).expect("encode_pbm P6 16-bit");
    let mut g = c.benchmark_group("decode_p6_rgb48_320x240");
    g.throughput(Throughput::Bytes((320 * 240 * 6) as u64));
    g.bench_function(BenchmarkId::from_parameter("p6/16/320x240"), |b| {
        b.iter(|| decode_pbm(criterion::black_box(&bytes)).expect("decode_pbm"));
    });
    g.finish();
}

fn bench_decode_p7_rgba_320x240(c: &mut Criterion) {
    let image = build_rgba(320, 240);
    let bytes = encode_pbm(&image).expect("encode_pbm P7 RGBA");
    let mut g = c.benchmark_group("decode_p7_rgba_320x240");
    g.throughput(Throughput::Bytes((320 * 240 * 4) as u64));
    g.bench_function(BenchmarkId::from_parameter("p7/rgba/320x240"), |b| {
        b.iter(|| decode_pbm(criterion::black_box(&bytes)).expect("decode_pbm"));
    });
    g.finish();
}

fn bench_decode_p7_rgba64_320x240(c: &mut Criterion) {
    let image = build_rgba64(320, 240);
    let bytes = encode_pbm(&image).expect("encode_pbm P7 RGBA64");
    let mut g = c.benchmark_group("decode_p7_rgba64_320x240");
    g.throughput(Throughput::Bytes((320 * 240 * 8) as u64));
    g.bench_function(BenchmarkId::from_parameter("p7/rgba64/320x240"), |b| {
        b.iter(|| decode_pbm(criterion::black_box(&bytes)).expect("decode_pbm"));
    });
    g.finish();
}

fn bench_decode_p1_mono_320x240(c: &mut Criterion) {
    let image = build_mono(320, 240);
    let bytes = encode_pbm_with_format(&image, PbmEncodeFormat::Pnm1).expect("encode P1");
    let mut g = c.benchmark_group("decode_p1_mono_320x240");
    g.throughput(Throughput::Bytes((320 * 240 / 8) as u64));
    g.sample_size(20);
    g.bench_function(BenchmarkId::from_parameter("p1/320x240"), |b| {
        b.iter(|| decode_pbm(criterion::black_box(&bytes)).expect("decode_pbm"));
    });
    g.finish();
}

fn bench_decode_p2_gray8_320x240(c: &mut Criterion) {
    let image = build_gray8(320, 240);
    let bytes = encode_pbm_ascii(&image).expect("encode P2");
    let mut g = c.benchmark_group("decode_p2_gray8_320x240");
    g.throughput(Throughput::Bytes((320 * 240) as u64));
    g.sample_size(20);
    g.bench_function(BenchmarkId::from_parameter("p2/320x240"), |b| {
        b.iter(|| decode_pbm(criterion::black_box(&bytes)).expect("decode_pbm"));
    });
    g.finish();
}

fn bench_decode_p3_rgb24_320x240(c: &mut Criterion) {
    let image = build_rgb24(320, 240);
    let bytes = encode_pbm_ascii(&image).expect("encode P3");
    let mut g = c.benchmark_group("decode_p3_rgb24_320x240");
    g.throughput(Throughput::Bytes((320 * 240 * 3) as u64));
    g.sample_size(10);
    g.bench_function(BenchmarkId::from_parameter("p3/320x240"), |b| {
        b.iter(|| decode_pbm(criterion::black_box(&bytes)).expect("decode_pbm"));
    });
    g.finish();
}

criterion_group!(
    benches,
    bench_decode_p4_mono_640x480,
    bench_decode_p5_gray8_640x480,
    bench_decode_p5_gray16_640x480,
    bench_decode_p6_rgb24_640x480,
    bench_decode_p6_rgb48_320x240,
    bench_decode_p7_rgba_320x240,
    bench_decode_p7_rgba64_320x240,
    bench_decode_p1_mono_320x240,
    bench_decode_p2_gray8_320x240,
    bench_decode_p3_rgb24_320x240,
);
criterion_main!(benches);
