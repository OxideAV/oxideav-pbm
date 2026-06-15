# oxideav-pbm

Pure-Rust Netpbm (PBM/PGM/PPM/PNM/PAM/PFM) image codec and container for
the [`oxideav`](https://github.com/OxideAV/oxideav) framework. Covers
all nine Netpbm magic numbers in one self-contained crate. Implemented
from the Netpbm man pages (`pbm(5)`, `pgm(5)`, `ppm(5)`, `pnm(5)`,
`pam(5)`) plus the Debevec PFM reference, with no external
implementation source consulted.

## Decode

| Magic | Name | Encoding | Channels | Bit depth | `PixelFormat` out |
|-------|------|----------|----------|-----------|-------------------|
| P1    | PBM  | ASCII    | 1 (1-bit)  | 1         | `MonoBlack` |
| P2    | PGM  | ASCII    | 1          | 8 / 16    | `Gray8` / `Gray16Le` |
| P3    | PPM  | ASCII    | 3 (RGB)    | 8 / 16    | `Rgb24` / `Rgb48Le` |
| P4    | PBM  | Binary   | 1 (1-bit)  | 1         | `MonoBlack` |
| P5    | PGM  | Binary   | 1          | 8 / 16    | `Gray8` / `Gray16Le` |
| P6    | PPM  | Binary   | 3 (RGB)    | 8 / 16    | `Rgb24` / `Rgb48Le` |
| P7    | PAM  | Binary   | 1-4, any `TUPLTYPE` | 1-16 | `MonoBlack` / `Gray*` / `Rgb*` / `Ya8` / `Ya16Le` / `Rgba` / `Rgba64Le` |
| `Pf`  | PFM  | Binary   | 1 (gray)   | 32 float  | `GrayF32` |
| `PF`  | PFM  | Binary   | 3 (RGB)    | 32 float  | `RgbF32` |

Comments (`# … LF`) are tolerated everywhere the integer PNM/PAM spec
permits them — in headers and (for ASCII variants) between samples in
the body. Any ASCII whitespace separates header tokens and ASCII
samples. P1 accepts both canonical token style and whitespace-free digit
runs. The Portable FloatMap header is the strict exception (see below).

For consumers that want to forward producer-stashed provenance text, the
crate exposes a non-allocating, typed comment iterator that borrows into
the input and stops at the start of the pixel data:

```rust
let buf = b"P3\n# created by an editor\n# tool: v2.10\n2 1\n255\n0 0 0 1 1 1\n";
let comments: Vec<&[u8]> = oxideav_pbm::iter_pnm_header_comments(buf).collect();
assert_eq!(comments, vec![&b"created by an editor"[..], &b"tool: v2.10"[..]]);
```

## Encode

`encode_pbm` picks the closest binary form for the input `PixelFormat`:

| Input          | Output |
|----------------|--------|
| `MonoBlack`    | P4     |
| `Gray8`        | P5 (maxval 255)  |
| `Gray16Le`     | P5 (maxval 65535, samples big-endian on disk) |
| `Rgb24`        | P6 (maxval 255)  |
| `Rgb48Le`      | P6 (maxval 65535) |
| `Rgba`/`Bgra`  | P7 RGB_ALPHA (maxval 255) |
| `Rgba64Le`     | P7 RGB_ALPHA (maxval 65535) |
| `Ya8`          | P7 GRAYSCALE_ALPHA (maxval 255) |
| `Ya16Le`       | P7 GRAYSCALE_ALPHA (maxval 65535) |
| `GrayF32`      | `Pf` Portable FloatMap (little-endian, scale -1.0) |
| `RgbF32`       | `PF` Portable FloatMap (little-endian, scale -1.0) |

Plain ASCII output (P1/P2/P3) is available via
[`encoder::encode_pbm_ascii`]; the binary path is preferred for size.
[`encoder::encode_pbm_with_format`] takes a [`encoder::PbmEncodeFormat`]
selector to pin the on-disk magic explicitly (`Pnm1`…`Pnm6` / `Pam7` /
`Pfm` plus the convenience `AutoBinary` / `AutoAscii` variants).

## Portable FloatMap (`Pf` / `PF`)

The floating-point member of the family: a strict three-line ASCII
header followed by raw IEEE-754 binary32 samples (one per pixel for `Pf`
grayscale, three interleaved R/G/B for `PF` colour).

- **Header** is exactly three LF-terminated lines — magic, `width
  height`, and a scale line — with **no comments** and **no CRLF**. Any
  `#`, carriage return, or missing LF is rejected.
- **Byte order** is carried by the *sign* of the scale line: negative ⇒
  little-endian, positive ⇒ big-endian. Its absolute value is an
  application-defined scale factor, preserved as metadata (not applied
  to the pixels). Degenerate scale lines (`NaN`, `±0.0`, `±inf`) fail.
- **Row order on disk is bottom-to-top**; the decoder flips rows so the
  in-memory plane is top-to-bottom. In memory the float samples are
  always little-endian (`GrayF32` = 4 B/px, `RgbF32` = 12 B/px).

Dedicated [`decode_pfm`] / [`encode_pfm`] entry points expose byte order
and scale explicitly; [`decode_pbm`] / [`encode_pbm`] also handle
`Pf` / `PF` automatically. [`decode_pfm_consumed`] is the length-aware
variant — it returns the exact on-disk byte count (header plus the
`width × height × channels × 4` raster) alongside the image and
[`PfmHeaderInfo`], so a caller can walk a stream of concatenated PFM
images while keeping each image's byte order and scale (which the
integer-flavoured [`decode_pbm_consumed`] does not carry). The scale factor is advisory and never
applied automatically; opt-in helpers [`apply_pfm_scale`] and
[`decode_pfm_scaled`] fold it into the samples on request. The two float
formats have no `oxideav_core::PixelFormat` counterpart, so the
framework codec/container path advertises no pixel format for them —
they are reachable through the standalone API and the crate-local
`PbmImage` model.

## Multi-image streams

A single file may carry a sequence of concatenated images packed
back-to-back. [`decode_pbm`] returns only the first image;
[`decode_pbm_multi`] walks every image in stream order, and
[`decode_pbm_consumed`] exposes the per-image byte count so callers can
drive the walk themselves:

```rust
let imgs = oxideav_pbm::decode_pbm_multi(&stream)?;   // Vec<(PbmImage, PbmPixelFormat)>
for (img, fmt) in &imgs { /* ... */ }
```

Each image's on-disk length is resolved exactly — deterministic for the
binary/PFM bodies, the tokenizer cursor for ASCII bodies — so a stream
decodes correctly even when it interleaves ASCII and binary magics.

For a stream walker that needs the per-image header metadata — `MAXVAL`,
`DEPTH`, the PAM `TUPLTYPE`, or (for `Pf` / `PF`) the byte order and
scale — the metadata-carrying entries hand the fully parsed `Header`
back alongside each image. [`decode_pbm_header_consumed`] is the
single-image counterpart to [`decode_pbm_consumed`], and
[`decode_pbm_multi_with_headers`] is the counterpart to
[`decode_pbm_multi`]; both close the asymmetry the integer
[`decode_pbm_consumed`] previously had against the PFM-only
[`decode_pfm_consumed`] (which already surfaced byte order and scale via
[`PfmHeaderInfo`]). For PFM inputs the returned header's `pfm` field
carries the same byte order and scale.

```rust
let imgs = oxideav_pbm::decode_pbm_multi_with_headers(&stream)?;
for (img, fmt, header) in &imgs {
    // header.magic / header.maxval / header.depth / header.tupltype,
    // and header.pfm for the Pf / PF magics.
}
```

## PAM tuple-type handling

The six standard `TUPLTYPE` names (`BLACKANDWHITE`, `GRAYSCALE`, `RGB`,
`BLACKANDWHITE_ALPHA`, `GRAYSCALE_ALPHA`, `RGB_ALPHA`) pin a fixed
channel layout. `pam(5)` also permits arbitrary user-defined names; the
parser round-trips any non-standard name through `Tupltype::Custom(String)`
and routes the pixels through the same `DEPTH`/`MAXVAL`-based fallback
used when `TUPLTYPE` is omitted. 16-bit grayscale-with-alpha decodes
natively as the crate-local `Ya16Le`; like the PFM formats it has no
`oxideav_core::PixelFormat` counterpart yet.

## Fuzzing

A `fuzz/` cargo-fuzz workspace exercises five independent entry points:
`decode` (full pipeline), `header` (header parser in isolation),
`encode_roundtrip` (every `PbmEncodeFormat` × `PbmPixelFormat` pair
including rejection paths), `pfm` (Portable FloatMap decoder), and
`multi` (multi-image stream walker). Both ASCII and binary decoders
validate dimensions against the available body length before allocating,
guarding against pre-allocation OOM. A daily CI run
(`.github/workflows/fuzz.yml`) keeps the contract enforced.

## Benchmarks

Three Criterion bench binaries cover the codec hot paths
(`benches/{decode,encode,roundtrip}.rs`). Inputs are synthesised
in-bench from a deterministic seed — no fixture files are committed. The
matrix covers every binary magic (P4/P5/P6/P7) at 8 and 16-bit, the
three ASCII magics, and both Portable FloatMap magics in both byte
orders.

```sh
cargo bench -p oxideav-pbm --bench decode
cargo bench -p oxideav-pbm --bench encode
cargo bench -p oxideav-pbm --bench roundtrip
```

The binary decode/encode hot paths route through row-level copy /
byte-swap helpers that LLVM lowers to vector lane shuffles, and the
bit-packed P4 path is a per-row `copy_from_slice` since the `MonoBlack`
plane layout is byte-identical to the P4 wire format.

## Registration

```rust
let mut ctx = oxideav_core::RuntimeContext::new();
oxideav_pbm::register(&mut ctx);
```
