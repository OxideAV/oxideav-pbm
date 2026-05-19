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
| P7    | PAM  | Binary   | 1-4 + 6 standard `TUPLTYPE`s | 1-16 | `MonoBlack` / `Gray*` / `Rgb*` / `Ya8` / `Rgba` / `Rgba64Le` |

Comments (`# … LF`) are tolerated everywhere the spec permits — both in
headers and (for ASCII variants) in the body between samples. Any ASCII
whitespace separates header tokens and ASCII samples. P1 accepts both
canonical token style and whitespace-free digit runs.

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

Plain ASCII output (P1/P2/P3) is available via
[`encoder::encode_pbm_ascii`] — the binary path is always preferred
for size.

For callers that need to pin the on-disk magic explicitly,
[`encoder::encode_pbm_with_format`] takes a [`encoder::PbmEncodeFormat`]
selector covering every magic individually (`Pnm1` / `Pnm2` / `Pnm3` /
`Pnm4` / `Pnm5` / `Pnm6` / `Pam7`) plus the convenience `AutoBinary` /
`AutoAscii` variants. P7 PAM accepts every supported `PbmPixelFormat`
(including the GRAYSCALE-as-PAM case that `encode_pbm` would otherwise
route to P5).

## Round 1 deferrals

* User-defined `TUPLTYPE` strings (round 1 supports the six standard
  names `BLACKANDWHITE`, `GRAYSCALE`, `RGB`, `BLACKANDWHITE_ALPHA`,
  `GRAYSCALE_ALPHA`, `RGB_ALPHA` only).
* 16-bit `GRAYSCALE_ALPHA` is widened to `Rgba` on decode (no `Ya16`
  variant in `oxideav-core` yet).

## Registration

```rust
let mut ctx = oxideav_core::RuntimeContext::new();
oxideav_pbm::register(&mut ctx);
```
