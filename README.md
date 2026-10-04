# oxideav-pbm

[![CI](https://github.com/OxideAV/oxideav-pbm/actions/workflows/ci.yml/badge.svg)](https://github.com/OxideAV/oxideav-pbm/actions/workflows/ci.yml) [![crates.io](https://img.shields.io/crates/v/oxideav-pbm.svg)](https://crates.io/crates/oxideav-pbm) [![docs.rs](https://docs.rs/oxideav-pbm/badge.svg)](https://docs.rs/oxideav-pbm) [![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

Pure-Rust Netpbm (PBM/PGM/PPM/PNM/PAM/PFM) image codec and container for
the [`oxideav`](https://github.com/OxideAV/oxideav) framework. Covers
all nine Netpbm magic numbers in one self-contained crate. Implemented
from the staged family reference and the Debevec PFM reference, with no
external implementation source consulted.

## Standalone use

`oxideav-pbm` follows the OxideAV image-crate contract
(`IMAGE_CRATE_API`): the same small root vocabulary every
`oxideav-<format>` image crate exposes, usable with
`default-features = false` and no `oxideav-core`, returning pixels as
plain `Vec<u8>`.

```toml
[dependencies]
oxideav-pbm = { version = "0.0", default-features = false }
```

```rust
let bytes = std::fs::read("in.ppm")?;
if oxideav_pbm::probe(&bytes) {
    let info  = oxideav_pbm::info(&bytes)?;         // header only: width, height, format, frames
    let img   = oxideav_pbm::decode(&bytes)?;       // PbmImage, native layout (Rgb24 for a P6)
    let rgba: Vec<u8> = img.to_rgba8();             // tightly packed RGBA, 4 * width bytes per row
    let (w, h) = (img.width(), img.height());

    let opts = oxideav_pbm::EncodeOptions::default();          // raw, natural flavour
    let out: Vec<u8> = oxideav_pbm::encode_rgb8(w, h, &img.to_rgb8(), &opts)?;   // P6
    std::fs::write("out.ppm", out)?;
    let pam: Vec<u8> = oxideav_pbm::encode_rgba8(w, h, &rgba, &opts)?;           // P7 RGB_ALPHA
    std::fs::write("out.pam", pam)?;
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

| Item | Signature |
|---|---|
| `probe` | `fn(&[u8]) -> bool` — `P` + magic char + whitespace sniff, allocation-free |
| `info` | `fn(&[u8]) -> Result<ImageInfo, Error>` — `width`, `height`, `format`, `frames` (concatenated images), `has_alpha`, `color`, `has_icc` / `has_exif` / `has_xmp` (always `false`), plus the parsed `header` (`Magic`, `MAXVAL`, `DEPTH`, `TUPLTYPE`, PFM byte order + scale) |
| `decode` / `decode_with` | `fn(&[u8][, &DecodeOptions]) -> Result<PbmImage, Error>` — first image, native layout |
| `decode_rgb8` / `decode_rgba8` | `-> Result<RgbImage / RgbaImage, Error>` — `{ width, height, data }`, tightly packed, 3 / 4 bytes per pixel |
| `decode_all` / `decode_all_with` | `-> Result<Vec<Frame>, Error>` — `Frame { image, delay: None, header }`; every image of a concatenated file, each in its own native layout |
| `decode_from` | `fn<R: Read>(R) -> Result<PbmImage, Error>` |
| `encode` | `fn(&PbmImage, &EncodeOptions) -> Result<Vec<u8>, Error>` — as given, never a silent conversion |
| `encode_rgb8` / `encode_rgba8` | `fn(w, h, &[u8], &EncodeOptions)` — P6 / P7 `RGB_ALPHA` (never a bitmap or graymap) |
| `encode_all` | `fn(&[Frame], &EncodeOptions) -> Result<Vec<u8>, Error>` — the images back to back as one concatenated stream, each written as `encode` would under `opts`; the mirror of `decode_all` (`Frame::from_image` wraps a caller-built image) |
| `encode_to` | `fn<W: Write>(&PbmImage, &EncodeOptions, W) -> Result<(), Error>` |
| `PbmImage` | `{ width, height, format: PixelFormat, planes: Vec<Plane>, color: ColorInfo, metadata: Metadata }` (no palette: Netpbm has none) with `new` / `packed` / `from_rgb8` / `from_rgba8` (all `Result`, geometry validated), `width()` / `height()` / `format()` / `stride()`, `as_bytes()` / `into_raw()`, `to_rgb8()` / `to_rgba8()`, `validate()` |
| `PixelFormat` | `= PbmPixelFormat`: `MonoBlack`, `MonoWhite`, `Gray8`, `Gray16Le`, `Ya8`, `Ya16Le`, `Rgb24`, `Rgb48Le`, `Rgba`, `Bgra`, `Rgba64Le`, `GrayF32Le`, `RgbF32Le` (names mirror `oxideav_core::PixelFormat`) |
| `Error` | `= PbmError`: `InvalidData`, `Unsupported`, `LimitExceeded`, `Io(std::io::Error)` |

`to_rgba8` is an exact integer kernel per layout: bilevel bits → 0 /
255 (`MonoBlack`: 1 = black), grey replicated to R = G = B, `Bgra`
reordered, 16-bit samples reduced by round-half-up
`(v × 255 + 32767) / 65535`, float samples clamped to `[0, 1]` then
`round(v × 255)` (NaN → 0), alpha 255 where the source has none. No
colour management is applied.

The pre-contract names remain for one release as `#[deprecated]` thin
wrappers over the same functions: `decode_pbm` (returned the format in
a tuple — it is now `PbmImage::format`), `decode_pbm_multi`,
`decode_pbm_multi_with_headers`, `decode_pbm_consumed`,
`decode_pbm_header_consumed`, `encode_pbm`, `encode_pbm_ascii`,
`encode_pbm_plane`, `encode_pbm_ascii_plane`, `encode_pbm_with_format`
+ `PbmEncodeFormat`, `probe_is_netpbm`, `PbmPlane` (→ `Plane`),
`PbmPixelFormat::GrayF32` / `RgbF32` (→ `GrayF32Le` / `RgbF32Le`),
and under `registry` `pixel_format_to_pbm` / `pbm_to_pixel_format`
(→ `from_core_pixel_format` / `to_core_pixel_format`). `PbmImage`
itself changed shape (`pixel_format` → `format`, `pts` dropped,
`color` + `metadata` added, `#[non_exhaustive]`): construct it through
`PbmImage::packed` / `new`.

## Framework use

With the default-on `registry` feature the crate plugs into the
`oxideav-core` registry:

```rust
# let img = oxideav_pbm::PbmImage::from_rgb8(1, 1, vec![255, 0, 0])?;
# let mut params = oxideav_core::CodecParameters::video(oxideav_core::CodecId::new("pbm"));
# params.width = Some(1);
# params.height = Some(1);
# params.pixel_format = Some(oxideav_core::PixelFormat::Rgb24);
let mut ctx = oxideav_core::RuntimeContext::new();
oxideav_pbm::register(&mut ctx);                       // codec "pbm" + the Netpbm container (.pbm .pgm .ppm .pnm .pam .pfm)
let dec = oxideav_pbm::make_decoder(&params)?;         // / make_encoder (options: ascii, pam, maxval, tupltype, pfm_little_endian, pfm_scale)
let frame: oxideav_core::VideoFrame = img.into();      // From<PbmImage>: the one packed plane
let back = oxideav_pbm::PbmImage::from_video_frame(&frame, &params)?;   // also TryFrom<(&VideoFrame, &CodecParameters)>
# let _ = (dec, back);
# Ok::<(), Box<dyn std::error::Error>>(())
```

The trait-side `Decoder` / `Encoder` are thin adapters over the
standalone functions (one implementation): the decoder emits every
image of a concatenated packet as its own frame in the native layout,
the encoder rebuilds a `PbmImage` from the frame and calls `encode`.
Every `PixelFormat` maps 1:1 onto `oxideav_core::PixelFormat`
(`to_core_pixel_format` / `from_core_pixel_format`); the container
advertises the native layout, including `Ya16Le`, `GrayF32Le` and
`RgbF32Le`. Netpbm carries no colour signalling or palette, so no
side-channel is stamped on frames.

## Supported layouts

Decode — magic / `MAXVAL` / `TUPLTYPE` → native `PixelFormat`:

| Magic | Name | Body | `MAXVAL` ≤ 255 | `MAXVAL` > 255 |
|-------|------|------|----------------|----------------|
| P1 / P4 | PBM | ASCII / raw bits | `MonoBlack` (1 = black) | — |
| P2 / P5 | PGM | ASCII / raw | `Gray8` | `Gray16Le` |
| P3 / P6 | PPM | ASCII / raw | `Rgb24` | `Rgb48Le` |
| P7 `BLACKANDWHITE` | PAM | raw | `MonoBlack` (bit sense normalised) | — |
| P7 `GRAYSCALE` / depth 1 | PAM | raw | `Gray8` | `Gray16Le` |
| P7 `GRAYSCALE_ALPHA` / depth 2 | PAM | raw | `Ya8` | `Ya16Le` |
| P7 `RGB` / depth 3 | PAM | raw | `Rgb24` | `Rgb48Le` |
| P7 `RGB_ALPHA` / depth 4 | PAM | raw | `Rgba` | `Rgba64Le` |
| P7 `BLACKANDWHITE_ALPHA` | PAM | raw | `Rgba` (bit expanded to a grey triple) | `Rgba` |
| `Pf` / `PF` | PFM | raw float32 | `GrayF32Le` / `RgbF32Le` (native float; little-endian in memory, rows flipped to top-down) | |

A non-natural `MAXVAL` (e.g. 15 or 1000) is rescaled to the 8- or
16-bit layout by round-half-up; a custom `TUPLTYPE` routes by `DEPTH`.
Comments (`# … LF`) are tolerated in every integer header and in the
P1 / P2 / P3 bodies; the three-line PFM header admits none.

Encode — `PbmImage::format` → natural magic (`EncodeOptions::default()`):

| `PixelFormat` | Output | Other flavours |
|---|---|---|
| `MonoBlack` / `MonoWhite` | P4 (`MonoWhite` bits inverted to the wire sense) | `ascii` → P1; `pam` → P7 `BLACKANDWHITE` |
| `Gray8` / `Gray16Le` | P5, `MAXVAL` 255 / 65535 | `ascii` → P2; `pam` → P7 `GRAYSCALE`; `maxval` |
| `Rgb24` / `Rgb48Le` | P6, `MAXVAL` 255 / 65535 | `ascii` → P3; `pam` → P7 `RGB`; `maxval` |
| `Ya8` / `Ya16Le` | P7 `GRAYSCALE_ALPHA` | `maxval`, `tupltype`; `ascii` → `Unsupported` |
| `Rgba` / `Bgra` / `Rgba64Le` | P7 `RGB_ALPHA` (`Bgra` reordered) | `maxval`, `tupltype`; `ascii` → `Unsupported` |
| `GrayF32Le` / `RgbF32Le` | `Pf` / `PF`, little-endian, scale 1 | `pfm_little_endian`, `pfm_scale`; `ascii` / `pam` / `maxval` → `Unsupported` |

Every layout is written as given; the only re-representations are
lossless (`Bgra` → `RGB_ALPHA` order, `MonoWhite` → P4 bit sense).
`encode` returns `Error::Unsupported` for what the family cannot
represent (alpha or float in plain text, PAM knobs on a float layout,
`ascii` together with `pam`) and never converts silently. `color` and
`metadata` cannot be carried and are ignored.

## Options

`DecodeOptions` (`Default` + `with_*`): `max_width`, `max_height`,
`max_pixels`, `max_bytes` (decoded plane bytes of the native layout;
default 1 GiB, `None` lifts it) — all checked against the header
before any allocation (`Error::LimitExceeded`) — and `strict`
(default `false`): a sample above `MAXVAL` is clamped in lenient mode
(the long-standing behaviour) and rejected with `Error::InvalidData`
in strict mode. Magic, dimensions, `MAXVAL` / `DEPTH` ranges, PAM
`TUPLTYPE`-vs-`DEPTH` consistency, body length and the PFM header
grammar are enforced in both modes.

`EncodeOptions` (`Default` + `with_*`; behaviour variants are fields):
`ascii` (plain-text P1 / P2 / P3), `pam` (force the P7 container),
`maxval: Option<u32>` (1..=65535; samples rescaled, one byte on disk
when ≤ 255 else two big-endian), `tupltype: Option<String>` (custom
PAM token, implies `pam`), `pfm_little_endian` (default `true`),
`pfm_scale` (default `1.0`, advisory metadata; samples are stored
verbatim — use the `pfm` module's `encode_pfm_scaled` /
`decode_pfm_scaled` to fold it in). The same keys are the framework
encoder's option schema (`maxval = 0` and an empty `tupltype` mean
"natural").

## Metadata and colour

Netpbm has no metadata mechanism beyond free-text header comments, so
`PbmImage::metadata` (`icc` / `exif` / `xmp` / `gamma`) is always
empty and the encoder cannot carry it; the comments are readable
through `iter_pnm_header_comments`. The file carries no colour
signalling either, so `PbmImage::color` is the family's documented
convention:

| Magics | `color` |
|---|---|
| P1–P7 | `ColorInfo::netpbm_default()` = full range, primaries 1 (BT.709), transfer 1 (BT.709), matrix 0 — the colour space the PPM format text names; in practice sRGB or linear data is also common and nothing in the file says which |
| `Pf` / `PF` | `ColorInfo::pfm_default()` = full range, primaries unspecified, transfer 8 (linear), matrix 0 — the reference describes PFM samples as linear light |

`decode(encode(img)) == img` for planes, colour and metadata for every
layout the decoder produces (`tests/image_crate_api.rs`,
`tests/recode_identity.rs`, the `recode` fuzz target).

## Limits

Every function returns `Error` on hostile input, never panics (fuzzed:
`probe` / `info` / `decode` / `decode_with` / `decode_rgb8` /
`decode_rgba8` / `decode_all` plus `parse_header`, `decode_pfm`, the
options-driven `encode` and the decode → encode → decode fixed point).
`DecodeOptions` limits fire from the header before the plane is
allocated; both ASCII and binary decoders also validate the declared
geometry against the available body length before allocating. `info`
is allocation-free: it walks the concatenated images by their
closed-form body length (binary / float) or a storing-nothing token
scan (plain text).

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
  always little-endian (`GrayF32Le` = 4 B/px, `RgbF32Le` = 12 B/px).

The depth API keeps its names: `decode_pfm` / `encode_pfm` expose byte
order and scale explicitly; `decode_pfm_consumed` returns the exact
on-disk byte count alongside the image and `PfmHeaderInfo`;
`decode_pfm_multi` walks an all-PFM stream. The scale factor is advisory
and never applied automatically; `apply_pfm_scale` / `decode_pfm_scaled`
fold it into the samples on request, and `apply_inverse_pfm_scale` /
`encode_pfm_scaled` are the encode-side inverses (store `sample / scale`
with the factor in the header so a reader applying it recovers the
linear values within `f32` precision).

## Multi-image streams

A single file may carry a sequence of concatenated images packed
back-to-back, optionally separated by ASCII whitespace. `decode` returns
only the first image; `decode_all` walks every image in stream order,
each `Frame` carrying its own parsed `Header`; `info(..).frames` counts
them without decoding. Each image's on-disk length is resolved exactly
— deterministic for the binary / PFM bodies, the tokenizer cursor for
ASCII bodies — so a stream decodes correctly even when it interleaves
ASCII and binary magics. A `#` between images is not a valid separator
(the magic must be the first two bytes of each image) and surfaces a
malformed-stream error. The framework `Decoder` honours multi-image
streams too: one packet yields one `receive_frame` result per image.

`encode_all(&frames, &opts)` writes such a stream: every frame's image
as `encode` would write it under the same options, back to back (binary
bodies are self-delimiting by length, plain bodies by sample count and
their trailing newline). `decode_all(encode_all(frames)) == frames` for
every layout the decoder produces (`tests/image_crate_api.rs`).

## PAM tuple-type handling

The six standard `TUPLTYPE` names (`BLACKANDWHITE`, `GRAYSCALE`, `RGB`,
`BLACKANDWHITE_ALPHA`, `GRAYSCALE_ALPHA`, `RGB_ALPHA`) pin a fixed
channel layout. The format also permits arbitrary user-defined names;
the parser round-trips any non-standard name through
`Tupltype::Custom(String)` and routes the pixels through the same
`DEPTH` / `MAXVAL`-based fallback used when `TUPLTYPE` is omitted. The
encoder writes a custom token through `EncodeOptions::tupltype`.

## Typed introspection & stream-dispatch helpers

`PbmPixelFormat` carries derived accessors so a caller can reason about
a layout without enumerating the variants by hand: `channels()`,
`bits_per_channel()`, `bytes_per_pixel()` (`None` for the sub-byte 1-bit
layouts), `row_bytes(width)`, `is_float()`, `has_alpha()`, `is_color()`,
`is_bilevel()`, and the `ALL` list. `PbmImage::min_row_bytes()` /
`min_plane_len()` / `validate()` size or sanity-check a
programmatically-built image.

For stream dispatch the always-compiled (framework-free) front door is
`probe` and `peek_magic(input) -> Option<Magic>` (identify a stream from
the two-byte magic alone). `Header::body_byte_len() ->
Result<Option<usize>>` returns the closed-form on-disk body length for
the binary and Portable FloatMap magics — and `None` for the ASCII
magics whose body length is not a closed form (`Header::is_ascii_body()`
distinguishes the two).

## Fuzzing

A `fuzz/` cargo-fuzz workspace exercises six independent entry points:
`decode` (`probe` / `info` / `decode` / `decode_rgb8` / `decode_rgba8`
/ `decode_with` with tight limits), `header` (header parser in
isolation), `encode_roundtrip` (options-driven `encode` of a synthetic
image for every layout × flavour, then `decode` of the output), `pfm`
(Portable FloatMap decoder), `multi` (`decode_all` + `info.frames`
agreement), and `recode` (decode → encode → decode fixed point,
`decode(encode(img)) == img` with an idempotent encoder). A daily CI
run (`.github/workflows/fuzz.yml`) keeps the contract enforced.

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
