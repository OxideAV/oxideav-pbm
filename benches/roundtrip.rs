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
//!   - **roundtrip_pf_gray_le_256x256** /
//!     **roundtrip_pf_gray_be_256x256**: 256×256 single-channel
//!     Portable FloatMap, both byte orders. BE drives the swap kernel
//!     on both encode and decode sides.
//!   - **roundtrip_pf_rgb_le_256x256** /
//!     **roundtrip_pf_rgb_be_256x256**: 256×256 3-channel Portable
//!     FloatMap (12 B/px) — widest PFM roundtrip.
//!
//! Run with:
//!     cargo bench -p oxideav-pbm --bench roundtrip

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

use oxideav_pbm::{
    decode_pbm, decode_pfm, encode_pbm, encode_pfm, PbmImage, PbmPixelFormat, PbmPlane,
};

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

/// Build a finite-valued float image (no NaN / inf samples) so the PFM
/// roundtrip exercises representative HDR data rather than the NaN
/// soup random bytes would produce.
fn build_float_image(width: u32, height: u32, channels: usize, seed: u32) -> PbmImage {
    let w = width as usize;
    let h = height as usize;
    let stride = w * channels * 4;
    let mut data = vec![0u8; stride * h];
    let mut state = seed;
    for y in 0..h {
        for x in 0..w {
            for c in 0..channels {
                let raw =
                    (xorshift_byte(&mut state) as u32) << 8 | xorshift_byte(&mut state) as u32;
                let v = (raw as f32) / 65535.0 * 100.0 - 50.0;
                let off = y * stride + (x * channels + c) * 4;
                data[off..off + 4].copy_from_slice(&v.to_le_bytes());
            }
        }
    }
    let format = if channels == 3 {
        PbmPixelFormat::RgbF32
    } else {
        PbmPixelFormat::GrayF32
    };
    PbmImage {
        width,
        height,
        pixel_format: format,
        planes: vec![PbmPlane { stride, data }],
        pts: None,
    }
}

/// PFM end-to-end roundtrip exercising the chosen byte order on both
/// sides. Drives the dedicated `encode_pfm` / `decode_pfm` entry
/// points rather than the unified `encode_pbm` path so we can pin
/// big-endian on the encode side (the unified path always writes LE).
fn rt_pfm(image: &PbmImage, little_endian: bool) {
    let bytes = encode_pfm(criterion::black_box(image), little_endian, 1.0).expect("encode_pfm");
    let (_, _info) = decode_pfm(criterion::black_box(&bytes)).expect("decode_pfm");
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

fn bench_roundtrip_pf_gray_le_256x256(c: &mut Criterion) {
    let image = build_float_image(256, 256, 1, 0x1818_2929);
    let mut g = c.benchmark_group("roundtrip_pf_gray_le_256x256");
    g.throughput(Throughput::Bytes((256 * 256 * 4) as u64));
    g.bench_function(BenchmarkId::from_parameter("pf/le/256x256"), |b| {
        b.iter(|| rt_pfm(&image, true));
    });
    g.finish();
}

fn bench_roundtrip_pf_gray_be_256x256(c: &mut Criterion) {
    // BE roundtrip — both encoder and decoder traverse the byte-swap
    // kernel; a regression on either side shows up here.
    let image = build_float_image(256, 256, 1, 0x2929_3a3a);
    let mut g = c.benchmark_group("roundtrip_pf_gray_be_256x256");
    g.throughput(Throughput::Bytes((256 * 256 * 4) as u64));
    g.bench_function(BenchmarkId::from_parameter("pf/be/256x256"), |b| {
        b.iter(|| rt_pfm(&image, false));
    });
    g.finish();
}

fn bench_roundtrip_pf_rgb_le_256x256(c: &mut Criterion) {
    let image = build_float_image(256, 256, 3, 0x3a3a_4b4b);
    let mut g = c.benchmark_group("roundtrip_pf_rgb_le_256x256");
    g.throughput(Throughput::Bytes((256 * 256 * 12) as u64));
    g.bench_function(BenchmarkId::from_parameter("pf/rgb/le/256x256"), |b| {
        b.iter(|| rt_pfm(&image, true));
    });
    g.finish();
}

fn bench_roundtrip_pf_rgb_be_256x256(c: &mut Criterion) {
    let image = build_float_image(256, 256, 3, 0x4b4b_5c5c);
    let mut g = c.benchmark_group("roundtrip_pf_rgb_be_256x256");
    g.throughput(Throughput::Bytes((256 * 256 * 12) as u64));
    g.bench_function(BenchmarkId::from_parameter("pf/rgb/be/256x256"), |b| {
        b.iter(|| rt_pfm(&image, false));
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
    bench_roundtrip_pf_gray_le_256x256,
    bench_roundtrip_pf_gray_be_256x256,
    bench_roundtrip_pf_rgb_le_256x256,
    bench_roundtrip_pf_rgb_be_256x256,
);
criterion_main!(benches);
