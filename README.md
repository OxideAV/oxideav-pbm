# oxideav-pbm

Pure-Rust Netpbm (PBM/PGM/PPM/PNM/PAM) image codec and container for
the [`oxideav`](https://github.com/OxideAV/oxideav) framework. Covers
all eight Netpbm magic numbers in one self-contained crate. Spec
sources: the Netpbm man pages (`pbm(5)`, `pgm(5)`, `ppm(5)`, `pnm(5)`,
`pam(5)`) — no external implementation source was consulted.

## Decode

| Magic | Name | Encoding | Channels | Bit depth | `PixelFormat` out |
|-------|------|----------|----------|-----------|-------------------|
| P1    | PBM  | ASCII    | 1 (1-bit)  | 1         | `MonoBlack` |
| P2    | PGM  | ASCII    | 1          | 8 / 16    | `Gray8` / `Gray16Le` |
| P3    | PPM  | ASCII    | 3 (RGB)    | 8 / 16    | `Rgb24` / `Rgb48Le` |
| P4    | PBM  | Binary   | 1 (1-bit)  | 1         | `MonoBlack` |
| P5    | PGM  | Binary   | 1          | 8 / 16    | `Gray8` / `Gray16Le` |
| P6    | PPM  | Binary   | 3 (RGB)    | 8 / 16    | `Rgb24` / `Rgb48Le` |
| P7    | PAM  | Binary   | 1-4, any `TUPLTYPE` (6 standard + arbitrary) | 1-16 | `MonoBlack` / `Gray*` / `Rgb*` / `Ya8` / `Rgba` / `Rgba64Le` |
| `Pf`  | PFM  | Binary   | 1 (gray)   | 32 float  | `GrayF32` |
| `PF`  | PFM  | Binary   | 3 (RGB)    | 32 float  | `RgbF32` |

Comments (`# … LF`) are tolerated everywhere the integer PNM/PAM spec
permits them — both in headers and (for ASCII variants) in the body
between samples. Any ASCII whitespace separates header tokens and ASCII
samples. P1 accepts both canonical token style and whitespace-free digit
runs. The Portable FloatMap header is the strict exception (see below).

## Encode

Picks the closest binary form for the input `PixelFormat`:

| Input          | Output |
|----------------|--------|
| `MonoBlack`    | P4     |
| `Gray8`        | P5 (maxval 255)  |
| `Gray16Le`     | P5 (maxval 65535, on-disk samples big-endian) |
| `Rgb24`        | P6 (maxval 255)  |
| `Rgb48Le`      | P6 (maxval 65535) |
| `Rgba`/`Bgra`  | P7 RGB_ALPHA (maxval 255) |
| `Rgba64Le`     | P7 RGB_ALPHA (maxval 65535) |
| `Ya8`          | P7 GRAYSCALE_ALPHA (maxval 255) |
| `GrayF32`      | `Pf` Portable FloatMap (little-endian, scale -1.0) |
| `RgbF32`       | `PF` Portable FloatMap (little-endian, scale -1.0) |

Plain ASCII output (P1/P2/P3) is available via
[`encoder::encode_pbm_ascii`] — the binary path is always preferred
for size.

For callers that need to pin the on-disk magic explicitly,
[`encoder::encode_pbm_with_format`] takes a [`encoder::PbmEncodeFormat`]
selector covering every magic individually (`Pnm1` / `Pnm2` / `Pnm3` /
`Pnm4` / `Pnm5` / `Pnm6` / `Pam7` / `Pfm`) plus the convenience
`AutoBinary` / `AutoAscii` variants. P7 PAM accepts every supported
`PbmPixelFormat` (including the GRAYSCALE-as-PAM case that `encode_pbm`
would otherwise route to P5).

## Portable FloatMap (`Pf` / `PF`)

The Portable FloatMap is the floating-point member of the family: a
strict three-line ASCII header followed by raw IEEE-754 binary32 samples
(one per pixel for `Pf` grayscale, three interleaved R/G/B for `PF`
colour). Reference: `docs/image/netpbm/pfm-portable-floatmap.md` (Debevec
PFM reference).

- **Header** is exactly three LF-terminated lines — magic, `width
  height`, and a scale line — with **no comments** and **no CRLF**. Any
  `#`, carriage return, or missing LF is rejected.
- **Byte order** is carried by the *sign* of the scale line: negative ⇒
  little-endian, positive ⇒ big-endian. Its *absolute value* is an
  application-defined scale factor, preserved as metadata (not applied to
  the pixels).
- **Row order on disk is bottom-to-top**; the decoder flips rows so the
  in-memory plane is the conventional top-to-bottom layout. In memory the
  float samples are always little-endian (`GrayF32` = 4 B/px, `RgbF32` =
  12 B/px).

Dedicated entry points [`decode_pfm`] / [`encode_pfm`] expose the byte
order and scale explicitly; [`decode_pbm`] / [`encode_pbm`] also handle
`Pf` / `PF` automatically (encoding defaults to little-endian with a unit
scale). The two float formats have no `oxideav_core::PixelFormat`
counterpart, so the framework codec/container path advertises no pixel
format for them — they are reachable through the standalone API and the
crate-local `PbmImage` model.

## PAM tuple-type handling

The six standard `TUPLTYPE` names (`BLACKANDWHITE`, `GRAYSCALE`, `RGB`,
`BLACKANDWHITE_ALPHA`, `GRAYSCALE_ALPHA`, `RGB_ALPHA`) pin a fixed
channel layout; `pam(5)` also permits arbitrary user-defined names so
producers can carry depth maps, RGBE light probes, normal maps, opacity
masks, or scientific multi-channel volumes. The parser round-trips any
non-standard name through a `Tupltype::Custom(String)` variant and
routes the pixels through the same depth-based fallback used when
`TUPLTYPE` is omitted entirely — channels reach the caller as
`Gray8` / `Gray16Le` / `Ya8` / `Rgb24` / `Rgb48Le` / `Rgba` /
`Rgba64Le` based on `DEPTH` (1..=4) and `MAXVAL`.

## Round-1 deferrals

* 16-bit `GRAYSCALE_ALPHA` is widened to `Rgba` on decode (no `Ya16`
  variant in `oxideav-core` yet).

## Fuzzing

A `fuzz/` cargo-fuzz workspace exercises four independent entry
points:

* `decode` — full pipeline (`parse_header` → ASCII/binary body decoder
  → `samples_to_plane`).
* `header` — header parser in isolation (PNM tokenizer + PAM
  key/value block).
* `encode_roundtrip` — synthetic `PbmImage` → every `PbmEncodeFormat`
  × `PbmPixelFormat` pair, including the `Unsupported` rejection paths.
* `pfm` — Portable FloatMap decoder in isolation (`decode_pfm`):
  the strict three-line header (no comments, no CRLF, single-LF
  terminator, sign-of-scale endianness selector), the raster overflow
  guards on `width * height * channels * 4`, the body-truncation
  check, and the big-endian byte-swap kernel. The PFM parser is
  disjoint from the PNM/PAM tokenizer the `decode` / `header`
  harnesses cover, so the round-199 addition closes a coverage gap.

The harness uncovered one pre-allocation OOM during round 171 (a
header claiming `width * height` in the billions triggered an
unchecked `vec![0u16; total_samples]` before the body-length check) —
both ASCII and binary decoders now validate dimensions against the
available body length before allocating. A daily CI run
(`.github/workflows/fuzz.yml`, 30 min budget split across the four
targets) keeps the contract enforced.

## Benchmarks

Three Criterion bench binaries cover the codec hot paths
(`benches/{decode,encode,roundtrip}.rs`). Inputs are synthesised
in-bench from a deterministic xorshift seed — no fixture files are
committed. Run:

```
cargo bench -p oxideav-pbm --bench decode
cargo bench -p oxideav-pbm --bench encode
cargo bench -p oxideav-pbm --bench roundtrip
```

The matrix covers every binary magic (P4/P5/P6/P7) at 8 and 16-bit
plus the three ASCII magics (P1/P2/P3) and — as of round 199 — the
two Portable FloatMap magics (`Pf` / `PF`) in both byte orders, so
future optimisation rounds can A/B-compare SIMD byte-swap (P5/P6
16-bit, `Pf`/`PF` BE), branch-free bit packers (P4), or lookup-table
ASCII writers (P2/P3) against a stable baseline. Indicative
apple-silicon numbers on the binary path: ~1.7 GiB/s P6 8-bit
decode, ~6.9 GiB/s P7 16-bit RGBA decode, ~26 GiB/s P7 8-bit
GRAYSCALE_ALPHA encode. Round 199 PFM baselines at 256×256:
`Pf` LE decode ~32 GiB/s, `Pf` BE decode ~30 GiB/s, `PF` LE decode
~27 GiB/s, `PF` BE decode ~21 GiB/s; encode `Pf` LE ~42 GiB/s vs
`Pf` BE ~1.86 GiB/s, `PF` LE ~45 GiB/s vs `PF` BE ~1.86 GiB/s —
the BE encode is bottlenecked by the per-sample 4-byte swap loop
and is the obvious target for a future SIMD pass. Round 189 rewrote
the ASCII hot path (direct digit writers + u32 accumulator, no
`to_string`/`parse` round-trips): 320×240 P1 encode 7 MiB/s →
~140 MiB/s, P2 encode 60 MiB/s → ~320 MiB/s, P3 encode 58 MiB/s →
~295 MiB/s, P2/P3 decode both up ≈30-40 %.

## Registration

```rust
let mut ctx = oxideav_core::RuntimeContext::new();
oxideav_pbm::register(&mut ctx);
```
