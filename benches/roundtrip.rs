//! Criterion roundtrip benchmarks for `oxideav-pbm`.
//!
//! Round 176 (depth-mode benchmarks): each scenario encodes a freshly
//! synthesised image to one of the seven on-disk Netpbm magics, then
//! decodes it back. Useful as a single-number "end-to-end throughput"
//! signal that catches when either side regresses: a roundtrip
//! slowdown that isn't explained by either the matching `decode` or
//! `encode` bench (or vice versa) is a flag.
//!
//! Scenarios (no committed fixtures — every input is reproducible
//! from the bench source):
//!
//!   - **roundtrip_p4_mono_320x240**: 320×240 binary PBM end-to-end.
//!   - **roundtrip_p5_gray8_320x240**: 320×240 binary PGM 8-bit.
//!   - **roundtrip_p5_gray16_320x240**: 320×240 binary PGM 16-bit
//!     (BE/LE swap on both sides).
//!   - **roundtrip_p6_rgb24_320x240**: 320×240 binary PPM 8-bit.
//!   - **roundtrip_p6_rgb48_256x256**: 256×256 binary PPM 16-bit.
//!   - **roundtrip_p7_rgba_320x240**: 320×240 PAM `RGB_ALPHA` 8-bit.
//!   - **roundtrip_p7_rgba64_256x256**: 256×256 PAM `RGB_ALPHA`
//!     16-bit — widest pixel format.
//!
//! Run with:
//!     cargo bench -p oxideav-pbm --bench roundtrip

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

use oxideav_pbm::{decode_pbm, encode_pbm, PbmImage, PbmPixelFormat, PbmPlane};

fn xorshift_byte(state: &mut u32) -> u8 {
    *state ^= *state << 13;
    *state ^= *state >> 17;
    *state ^= *state << 5;
    (*state & 0xff) as u8
}

fn build_filled(width: u32, height: u32, format: PbmPixelFormat, seed: u32) -> PbmImage {
    let w = width as usize;
    let h = height as usize;
    let (stride, len) = match format {
        PbmPixelFormat::MonoBlack => {
            let rb = w.div_ceil(8);
            (rb, rb * h)
        }
        PbmPixelFormat::Gray8 => (w, w * h),
        PbmPixelFormat::Gray16Le => (w * 2, w * h * 2),
        PbmPixelFormat::Rgb24 => (w * 3, w * h * 3),
        PbmPixelFormat::Rgb48Le => (w * 6, w * h * 6),
        PbmPixelFormat::Rgba | PbmPixelFormat::Bgra => (w * 4, w * h * 4),
        PbmPixelFormat::Rgba64Le => (w * 8, w * h * 8),
        PbmPixelFormat::Ya8 => (w * 2, w * h * 2),
        PbmPixelFormat::GrayF32 => (w * 4, w * h * 4),
        PbmPixelFormat::RgbF32 => (w * 12, w * h * 12),
    };
    let mut data = vec![0u8; len];
    let mut state = seed;
    for byte in data.iter_mut() {
        *byte = xorshift_byte(&mut state);
    }
    PbmImage {
        width,
        height,
        pixel_format: format,
        planes: vec![PbmPlane { stride, data }],
        pts: None,
    }
}

fn rt(image: &PbmImage) {
    let bytes = encode_pbm(criterion::black_box(image)).expect("encode_pbm");
    let (_, _fmt) = decode_pbm(criterion::black_box(&bytes)).expect("decode_pbm");
}

fn bench_roundtrip_p4_mono_320x240(c: &mut Criterion) {
    let image = build_filled(320, 240, PbmPixelFormat::MonoBlack, 0xa1a1_b2b2);
    let mut g = c.benchmark_group("roundtrip_p4_mono_320x240");
    g.throughput(Throughput::Bytes((320 * 240 / 8) as u64));
    g.bench_function(BenchmarkId::from_parameter("p4/320x240"), |b| {
        b.iter(|| rt(&image));
    });
    g.finish();
}

fn bench_roundtrip_p5_gray8_320x240(c: &mut Criterion) {
    let image = build_filled(320, 240, PbmPixelFormat::Gray8, 0xb2b2_c3c3);
    let mut g = c.benchmark_group("roundtrip_p5_gray8_320x240");
    g.throughput(Throughput::Bytes((320 * 240) as u64));
    g.bench_function(BenchmarkId::from_parameter("p5/320x240"), |b| {
        b.iter(|| rt(&image));
    });
    g.finish();
}

fn bench_roundtrip_p5_gray16_320x240(c: &mut Criterion) {
    let image = build_filled(320, 240, PbmPixelFormat::Gray16Le, 0xc3c3_d4d4);
    let mut g = c.benchmark_group("roundtrip_p5_gray16_320x240");
    g.throughput(Throughput::Bytes((320 * 240 * 2) as u64));
    g.bench_function(BenchmarkId::from_parameter("p5/16/320x240"), |b| {
        b.iter(|| rt(&image));
    });
    g.finish();
}

fn bench_roundtrip_p6_rgb24_320x240(c: &mut Criterion) {
    let image = build_filled(320, 240, PbmPixelFormat::Rgb24, 0xd4d4_e5e5);
    let mut g = c.benchmark_group("roundtrip_p6_rgb24_320x240");
    g.throughput(Throughput::Bytes((320 * 240 * 3) as u64));
    g.bench_function(BenchmarkId::from_parameter("p6/320x240"), |b| {
        b.iter(|| rt(&image));
    });
    g.finish();
}

fn bench_roundtrip_p6_rgb48_256x256(c: &mut Criterion) {
    let image = build_filled(256, 256, PbmPixelFormat::Rgb48Le, 0xe5e5_f6f6);
    let mut g = c.benchmark_group("roundtrip_p6_rgb48_256x256");
    g.throughput(Throughput::Bytes((256 * 256 * 6) as u64));
    g.bench_function(BenchmarkId::from_parameter("p6/16/256x256"), |b| {
        b.iter(|| rt(&image));
    });
    g.finish();
}

fn bench_roundtrip_p7_rgba_320x240(c: &mut Criterion) {
    let image = build_filled(320, 240, PbmPixelFormat::Rgba, 0xf6f6_0707);
    let mut g = c.benchmark_group("roundtrip_p7_rgba_320x240");
    g.throughput(Throughput::Bytes((320 * 240 * 4) as u64));
    g.bench_function(BenchmarkId::from_parameter("p7/rgba/320x240"), |b| {
        b.iter(|| rt(&image));
    });
    g.finish();
}

fn bench_roundtrip_p7_rgba64_256x256(c: &mut Criterion) {
    let image = build_filled(256, 256, PbmPixelFormat::Rgba64Le, 0x0707_1818);
    let mut g = c.benchmark_group("roundtrip_p7_rgba64_256x256");
    g.throughput(Throughput::Bytes((256 * 256 * 8) as u64));
    g.bench_function(BenchmarkId::from_parameter("p7/rgba64/256x256"), |b| {
        b.iter(|| rt(&image));
    });
    g.finish();
}

criterion_group!(
    benches,
    bench_roundtrip_p4_mono_320x240,
    bench_roundtrip_p5_gray8_320x240,
    bench_roundtrip_p5_gray16_320x240,
    bench_roundtrip_p6_rgb24_320x240,
    bench_roundtrip_p6_rgb48_256x256,
    bench_roundtrip_p7_rgba_320x240,
    bench_roundtrip_p7_rgba64_256x256,
);
criterion_main!(benches);
