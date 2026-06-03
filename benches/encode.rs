//! Criterion benchmarks for the Netpbm encoder hot paths.
//!
//! Round 176 (depth-mode benchmarks): the encoder owns the per-magic
//! header builder, the byte-swap for 16-bit Gray/RGB (Netpbm stores
//! BE on disk; `oxideav-core` uses LE-tagged variants), the P4
//! bit-repack from `MonoBlack` row bytes, the P7 PAM header
//! generation, and the plain-ASCII writers for P1/P2/P3. These
//! benches make each magic's cost measurable so a future "Lever
//! N+1" optimisation round (SIMD byte-swap for P5/P6 16-bit,
//! lookup-table ASCII writer for P2/P3, branch-free P4 bit packer)
//! can A/B-compare against the r176 baseline.
//!
//! Scenarios (all freshly synthesised, no committed fixtures):
//!
//!   - **encode_p4_mono_640x480**: 640×480 binary PBM.
//!   - **encode_p5_gray8_640x480**: 640×480 binary PGM 8-bit.
//!   - **encode_p5_gray16_640x480**: 640×480 binary PGM 16-bit
//!     (LE→BE byte swap on every sample).
//!   - **encode_p6_rgb24_640x480**: 640×480 binary PPM 8-bit.
//!   - **encode_p6_rgb48_320x240**: 320×240 binary PPM 16-bit.
//!   - **encode_p7_rgba_320x240**: 320×240 PAM `RGB_ALPHA` 8-bit.
//!   - **encode_p7_rgba64_320x240**: 320×240 PAM `RGB_ALPHA` 16-bit.
//!   - **encode_p7_gray16_320x240**: 320×240 PAM `GRAYSCALE` 16-bit —
//!     the 1-channel LE→BE swap path on `Gray16Le` (only reachable via
//!     the explicit `Pam7` selector; default routing for `Gray16Le`
//!     goes to P5). Closes the r222 symmetry gap that left this path
//!     on the slow per-sample `out.push` pattern.
//!   - **encode_p7_ya8_320x240**: 320×240 PAM `GRAYSCALE_ALPHA` 8-bit
//!     — the only path that exercises the 2-channel non-mono encoder.
//!   - **encode_p1_mono_320x240**: 320×240 plain-ASCII PBM — write
//!     hot path for the ASCII bit writer.
//!   - **encode_p2_gray8_320x240**: 320×240 plain-ASCII PGM —
//!     itoa-style sample writer.
//!   - **encode_p3_rgb24_320x240**: 320×240 plain-ASCII PPM — densest
//!     ASCII writer (3 samples per pixel).
//!   - **encode_pf_gray_le_256x256** / **encode_pf_gray_be_256x256**:
//!     256×256 single-channel Portable FloatMap. LE is the row-flip-
//!     only fast path; BE adds the per-sample 4-byte swap.
//!   - **encode_pf_rgb_le_256x256** / **encode_pf_rgb_be_256x256**:
//!     256×256 3-channel Portable FloatMap (12 B/px). Widest PFM
//!     encode paths.
//!
//! Run with:
//!     cargo bench -p oxideav-pbm --bench encode

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

use oxideav_pbm::{
    encode_pbm, encode_pbm_ascii, encode_pbm_with_format, encode_pfm, PbmEncodeFormat, PbmImage,
    PbmPixelFormat, PbmPlane,
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

/// Build a finite-valued float image (no NaN / inf samples) so the PFM
/// encoder sees representative HDR-pipeline floats rather than the
/// NaN trash random bytes would produce.
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

fn bench_encode_p4_mono_640x480(c: &mut Criterion) {
    let image = build_filled(640, 480, PbmPixelFormat::MonoBlack, 0x1111_2222);
    let mut g = c.benchmark_group("encode_p4_mono_640x480");
    g.throughput(Throughput::Bytes((640 * 480 / 8) as u64));
    g.bench_function(BenchmarkId::from_parameter("p4/640x480"), |b| {
        b.iter(|| encode_pbm(criterion::black_box(&image)).expect("encode_pbm"));
    });
    g.finish();
}

fn bench_encode_p5_gray8_640x480(c: &mut Criterion) {
    let image = build_filled(640, 480, PbmPixelFormat::Gray8, 0x2222_3333);
    let mut g = c.benchmark_group("encode_p5_gray8_640x480");
    g.throughput(Throughput::Bytes((640 * 480) as u64));
    g.bench_function(BenchmarkId::from_parameter("p5/640x480"), |b| {
        b.iter(|| encode_pbm(criterion::black_box(&image)).expect("encode_pbm"));
    });
    g.finish();
}

fn bench_encode_p5_gray16_640x480(c: &mut Criterion) {
    let image = build_filled(640, 480, PbmPixelFormat::Gray16Le, 0x3333_4444);
    let mut g = c.benchmark_group("encode_p5_gray16_640x480");
    g.throughput(Throughput::Bytes((640 * 480 * 2) as u64));
    g.bench_function(BenchmarkId::from_parameter("p5/16/640x480"), |b| {
        b.iter(|| encode_pbm(criterion::black_box(&image)).expect("encode_pbm"));
    });
    g.finish();
}

fn bench_encode_p6_rgb24_640x480(c: &mut Criterion) {
    let image = build_filled(640, 480, PbmPixelFormat::Rgb24, 0x4444_5555);
    let mut g = c.benchmark_group("encode_p6_rgb24_640x480");
    g.throughput(Throughput::Bytes((640 * 480 * 3) as u64));
    g.bench_function(BenchmarkId::from_parameter("p6/640x480"), |b| {
        b.iter(|| encode_pbm(criterion::black_box(&image)).expect("encode_pbm"));
    });
    g.finish();
}

fn bench_encode_p6_rgb48_320x240(c: &mut Criterion) {
    let image = build_filled(320, 240, PbmPixelFormat::Rgb48Le, 0x5555_6666);
    let mut g = c.benchmark_group("encode_p6_rgb48_320x240");
    g.throughput(Throughput::Bytes((320 * 240 * 6) as u64));
    g.bench_function(BenchmarkId::from_parameter("p6/16/320x240"), |b| {
        b.iter(|| encode_pbm(criterion::black_box(&image)).expect("encode_pbm"));
    });
    g.finish();
}

fn bench_encode_p7_rgba_320x240(c: &mut Criterion) {
    let image = build_filled(320, 240, PbmPixelFormat::Rgba, 0x6666_7777);
    let mut g = c.benchmark_group("encode_p7_rgba_320x240");
    g.throughput(Throughput::Bytes((320 * 240 * 4) as u64));
    g.bench_function(BenchmarkId::from_parameter("p7/rgba/320x240"), |b| {
        b.iter(|| encode_pbm(criterion::black_box(&image)).expect("encode_pbm"));
    });
    g.finish();
}

fn bench_encode_p7_rgba64_320x240(c: &mut Criterion) {
    let image = build_filled(320, 240, PbmPixelFormat::Rgba64Le, 0x7777_8888);
    let mut g = c.benchmark_group("encode_p7_rgba64_320x240");
    g.throughput(Throughput::Bytes((320 * 240 * 8) as u64));
    g.bench_function(BenchmarkId::from_parameter("p7/rgba64/320x240"), |b| {
        b.iter(|| encode_pbm(criterion::black_box(&image)).expect("encode_pbm"));
    });
    g.finish();
}

fn bench_encode_p7_gray16_320x240(c: &mut Criterion) {
    // P7 GRAYSCALE 16-bit — single-channel LE→BE swap reached via the
    // explicit `Pam7` selector with `Gray16Le`. Round 222 routed this
    // path through the shared `swap_bytes_u16_row` helper that
    // P5 / P6 / P7 RGB / RGBA already use; this bench makes the
    // resulting speedup measurable against the round-217 baseline.
    let image = build_filled(320, 240, PbmPixelFormat::Gray16Le, 0x8765_4321);
    let mut g = c.benchmark_group("encode_p7_gray16_320x240");
    g.throughput(Throughput::Bytes((320 * 240 * 2) as u64));
    g.bench_function(BenchmarkId::from_parameter("p7/gray16/320x240"), |b| {
        b.iter(|| {
            encode_pbm_with_format(criterion::black_box(&image), PbmEncodeFormat::Pam7)
                .expect("encode P7 gray16")
        });
    });
    g.finish();
}

fn bench_encode_p7_ya8_320x240(c: &mut Criterion) {
    let image = build_filled(320, 240, PbmPixelFormat::Ya8, 0x8888_9999);
    let mut g = c.benchmark_group("encode_p7_ya8_320x240");
    g.throughput(Throughput::Bytes((320 * 240 * 2) as u64));
    g.bench_function(BenchmarkId::from_parameter("p7/ya8/320x240"), |b| {
        b.iter(|| encode_pbm(criterion::black_box(&image)).expect("encode_pbm"));
    });
    g.finish();
}

fn bench_encode_p1_mono_320x240(c: &mut Criterion) {
    let image = build_filled(320, 240, PbmPixelFormat::MonoBlack, 0x9999_aaaa);
    let mut g = c.benchmark_group("encode_p1_mono_320x240");
    g.throughput(Throughput::Bytes((320 * 240 / 8) as u64));
    g.sample_size(20);
    g.bench_function(BenchmarkId::from_parameter("p1/320x240"), |b| {
        b.iter(|| {
            encode_pbm_with_format(criterion::black_box(&image), PbmEncodeFormat::Pnm1)
                .expect("encode P1")
        });
    });
    g.finish();
}

fn bench_encode_p2_gray8_320x240(c: &mut Criterion) {
    let image = build_filled(320, 240, PbmPixelFormat::Gray8, 0xaaaa_bbbb);
    let mut g = c.benchmark_group("encode_p2_gray8_320x240");
    g.throughput(Throughput::Bytes((320 * 240) as u64));
    g.sample_size(20);
    g.bench_function(BenchmarkId::from_parameter("p2/320x240"), |b| {
        b.iter(|| encode_pbm_ascii(criterion::black_box(&image)).expect("encode P2"));
    });
    g.finish();
}

fn bench_encode_p3_rgb24_320x240(c: &mut Criterion) {
    let image = build_filled(320, 240, PbmPixelFormat::Rgb24, 0xbbbb_cccc);
    let mut g = c.benchmark_group("encode_p3_rgb24_320x240");
    g.throughput(Throughput::Bytes((320 * 240 * 3) as u64));
    g.sample_size(10);
    g.bench_function(BenchmarkId::from_parameter("p3/320x240"), |b| {
        b.iter(|| encode_pbm_ascii(criterion::black_box(&image)).expect("encode P3"));
    });
    g.finish();
}

fn bench_encode_pf_gray_le_256x256(c: &mut Criterion) {
    // `Pf` LE — bottom-to-top row flip with no byte swap.
    let image = build_float_image(256, 256, 1, 0x1357_2468);
    let mut g = c.benchmark_group("encode_pf_gray_le_256x256");
    g.throughput(Throughput::Bytes((256 * 256 * 4) as u64));
    g.bench_function(BenchmarkId::from_parameter("pf/le/256x256"), |b| {
        b.iter(|| encode_pfm(criterion::black_box(&image), true, 1.0).expect("encode_pfm Pf LE"));
    });
    g.finish();
}

fn bench_encode_pf_gray_be_256x256(c: &mut Criterion) {
    // `Pf` BE — bottom-to-top row flip plus the per-sample 4-byte swap.
    let image = build_float_image(256, 256, 1, 0x2468_1357);
    let mut g = c.benchmark_group("encode_pf_gray_be_256x256");
    g.throughput(Throughput::Bytes((256 * 256 * 4) as u64));
    g.bench_function(BenchmarkId::from_parameter("pf/be/256x256"), |b| {
        b.iter(|| encode_pfm(criterion::black_box(&image), false, 1.0).expect("encode_pfm Pf BE"));
    });
    g.finish();
}

fn bench_encode_pf_rgb_le_256x256(c: &mut Criterion) {
    // `PF` LE — widest LE float path (12 B/px).
    let image = build_float_image(256, 256, 3, 0x9bdf_eca8);
    let mut g = c.benchmark_group("encode_pf_rgb_le_256x256");
    g.throughput(Throughput::Bytes((256 * 256 * 12) as u64));
    g.bench_function(BenchmarkId::from_parameter("pf/rgb/le/256x256"), |b| {
        b.iter(|| encode_pfm(criterion::black_box(&image), true, 1.0).expect("encode_pfm PF LE"));
    });
    g.finish();
}

fn bench_encode_pf_rgb_be_256x256(c: &mut Criterion) {
    // `PF` BE — widest BE float path: 3× the byte-swap traffic of `Pf` BE.
    let image = build_float_image(256, 256, 3, 0xeca8_9bdf);
    let mut g = c.benchmark_group("encode_pf_rgb_be_256x256");
    g.throughput(Throughput::Bytes((256 * 256 * 12) as u64));
    g.bench_function(BenchmarkId::from_parameter("pf/rgb/be/256x256"), |b| {
        b.iter(|| encode_pfm(criterion::black_box(&image), false, 1.0).expect("encode_pfm PF BE"));
    });
    g.finish();
}

criterion_group!(
    benches,
    bench_encode_p4_mono_640x480,
    bench_encode_p5_gray8_640x480,
    bench_encode_p5_gray16_640x480,
    bench_encode_p6_rgb24_640x480,
    bench_encode_p6_rgb48_320x240,
    bench_encode_p7_rgba_320x240,
    bench_encode_p7_rgba64_320x240,
    bench_encode_p7_gray16_320x240,
    bench_encode_p7_ya8_320x240,
    bench_encode_p1_mono_320x240,
    bench_encode_p2_gray8_320x240,
    bench_encode_p3_rgb24_320x240,
    bench_encode_pf_gray_le_256x256,
    bench_encode_pf_gray_be_256x256,
    bench_encode_pf_rgb_le_256x256,
    bench_encode_pf_rgb_be_256x256,
);
criterion_main!(benches);
