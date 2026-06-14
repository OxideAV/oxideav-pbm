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
| P7    | PAM  | Binary   | 1-4, any `TUPLTYPE` (6 standard + arbitrary) | 1-16 | `MonoBlack` / `Gray*` / `Rgb*` / `Ya8` / `Ya16Le` / `Rgba` / `Rgba64Le` |
| `Pf`  | PFM  | Binary   | 1 (gray)   | 32 float  | `GrayF32` |
| `PF`  | PFM  | Binary   | 3 (RGB)    | 32 float  | `RgbF32` |

Comments (`# … LF`) are tolerated everywhere the integer PNM/PAM spec
permits them — both in headers and (for ASCII variants) in the body
between samples. Any ASCII whitespace separates header tokens and ASCII
samples. P1 accepts both canonical token style and whitespace-free digit
runs. The Portable FloatMap header is the strict exception (see below).

Producers often stash provenance into the header comment block (e.g.
`# created by …`, `# tool: …`, `# resolution note`). For consumers
that want to forward that text into a different container or surface
it in a tool, the crate exposes a typed comment-iteration accessor:

```rust
let buf = b"P3\n# created by GIMP\n# tool: v2.10\n2 1\n255\n0 0 0 1 1 1\n";
let comments: Vec<&[u8]> = oxideav_pbm::iter_pnm_header_comments(buf).collect();
assert_eq!(comments, vec![&b"created by GIMP"[..], &b"tool: v2.10"[..]]);
```

The iterator borrows into `buf` (non-allocating), stops at the start
of the pixel data (so a `#` byte that happens to be a valid binary
sample is never misread as a comment), and yields each line's text
trimmed of surrounding ASCII whitespace. PFM (`Pf` / `PF`) and
unrecognised inputs yield zero items.

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
| `Ya16Le`       | P7 GRAYSCALE_ALPHA (maxval 65535) |
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
  the pixels). A degenerate scale line is rejected on decode so the
  parser accepts exactly the set the encoder can write: `NaN` (no usable
  sign for byte order), `±0.0` (zero is not a scale factor and a positive
  zero's sign is not a reliable endianness selector), and `±inf` (not a
  finite scale factor) all fail.
- **Row order on disk is bottom-to-top**; the decoder flips rows so the
  in-memory plane is the conventional top-to-bottom layout. In memory the
  float samples are always little-endian (`GrayF32` = 4 B/px, `RgbF32` =
  12 B/px).

Dedicated entry points [`decode_pfm`] / [`encode_pfm`] expose the byte
order and scale explicitly; [`decode_pbm`] / [`encode_pbm`] also handle
`Pf` / `PF` automatically (encoding defaults to little-endian with a unit
scale). The Debevec reference describes the scale-line magnitude as "a
scale factor … that an application may use to scale sample values", so it
is advisory: the decoders never apply it automatically. Callers that *do*
want the scaled linear-light values use the opt-in helpers
[`apply_pfm_scale`] (multiply an existing `GrayF32` / `RgbF32` image's
samples in place) or [`decode_pfm_scaled`] (decode and fold the header's
factor into the samples in one call, still reporting the original factor
in [`PfmHeaderInfo`]). Re-encoding a scaled image with a unit scale
reproduces the same linear values. The two float formats have no `oxideav_core::PixelFormat`
counterpart, so the framework codec/container path advertises no pixel
format for them — they are reachable through the standalone API and the
crate-local `PbmImage` model.

## Multi-image streams

A single Netpbm/PAM/PFM file may carry a **sequence of concatenated
images** — each a self-describing magic + header + body, packed
back-to-back. [`decode_pbm`] returns only the first image (trailing
images are ignored); [`decode_pbm_multi`] walks every image in stream
order:

```rust
let imgs = oxideav_pbm::decode_pbm_multi(&stream)?;   // Vec<(PbmImage, PbmPixelFormat)>
for (img, fmt) in &imgs { /* ... */ }
```

Each image's on-disk length is resolved exactly: the binary
(`P4`/`P5`/`P6`/`P7`) and Portable FloatMap (`Pf`/`PF`) bodies are
deterministic from the dimensions, depth, and bits-per-sample, while
the ASCII (`P1`/`P2`/`P3`) bodies report the tokenizer's consumed
cursor (the byte after the final sample token). A stream therefore
decodes correctly even when it interleaves ASCII and binary magics.
[`decode_pbm_consumed`] exposes that per-image byte count directly for
callers that want to drive the walk themselves. Inter-image ASCII
whitespace is skipped before the next magic; because the magic must be
the first two bytes of each image, a `#` comment is *not* a valid
inter-image separator and surfaces a malformed-stream error. Trailing
whitespace after the last image is accepted; trailing non-whitespace
that does not begin a valid header is an error.

## PAM tuple-type handling

The six standard `TUPLTYPE` names (`BLACKANDWHITE`, `GRAYSCALE`, `RGB`,
`BLACKANDWHITE_ALPHA`, `GRAYSCALE_ALPHA`, `RGB_ALPHA`) pin a fixed
channel layout; `pam(5)` also permits arbitrary user-defined names so
producers can carry depth maps, RGBE light probes, normal maps, opacity
masks, or scientific multi-channel volumes. The parser round-trips any
non-standard name through a `Tupltype::Custom(String)` variant and
routes the pixels through the same depth-based fallback used when
`TUPLTYPE` is omitted entirely — channels reach the caller as
`Gray8` / `Gray16Le` / `Ya8` / `Ya16Le` / `Rgb24` / `Rgb48Le` /
`Rgba` / `Rgba64Le` based on `DEPTH` (1..=4) and `MAXVAL`.

16-bit grayscale-with-alpha decodes natively as the crate-local
`Ya16Le` (little-endian `Y, A` u16 pairs, 4 bytes per pixel) with
full 16-bit precision. Like the two Portable FloatMap formats it has
no `oxideav_core::PixelFormat` counterpart yet, so the framework
codec/container path advertises no pixel format for it — the format
is reachable through the standalone API and the crate-local
`PbmImage` model. (`BLACKANDWHITE_ALPHA` still expands to `Rgba` —
its bit-valued first channel needs the gray-triplet expansion
either way.)

## Fuzzing

A `fuzz/` cargo-fuzz workspace exercises five independent entry
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
* `multi` — multi-image stream walker (`decode_pbm_multi` /
  `decode_pbm_consumed`). The single-image `decode` harness covers the
  body decoders; this round-299 addition covers the distinct
  byte-accounting layer on top of them — the loop that skips
  inter-image whitespace and advances `offset += consumed` across
  concatenated images, resolving each image's on-disk length
  (deterministic for the binary/PFM magics, ASCII-tokenizer cursor for
  P1/P2/P3) and guarding against a zero-consumed spin. The harness also
  asserts the walker's load-bearing invariant — a successful decode
  never reports `consumed > input.len()`, which would index the next
  `&input[offset..]` re-slice out of bounds. 13.4M runs over a mixed
  ASCII/binary/PFM corpus surfaced no panics; the corpus grew from the
  3.3k single-image seeds to ~6.8k entries, confirming the walk reached
  coverage the `decode` target never did.

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
apple-silicon numbers on the binary path: ~49 GiB/s P6 8-bit
decode, ~50 GiB/s P7 16-bit RGBA decode, ~26 GiB/s P7 8-bit
GRAYSCALE_ALPHA encode. Round 253 closed the last 8-bit binary
encode bottleneck — the `Bgra`→P7 `RGB_ALPHA` per-pixel channel
swap (B ↔ R, with G and A passing through) was the lone path still
walking a per-byte `out.push(px[2]); out.push(px[1]); out.push(px[0]);
out.push(px[3])` loop, while every other 8-bit binary encoder
(P5 / P6 / P7 RGB / P7 RGBA / P7 GRAYSCALE_ALPHA) ran
`extend_from_slice` over a contiguous row. A new
`binary::bgra_to_rgba_row` helper now walks `chunks_exact(4)` zipped
with `chunks_exact_mut(4)` over a pre-resized `&mut [u8]` destination
so LLVM can lower the four-byte permutation to a vector lane shuffle
(`TBL.16B` on aarch64; `pshufb` / `vpshufb` on x86). Apple-silicon
at 320×240: encode `Bgra` ~157 µs → ~7.1 µs (≈ 22× faster,
~40 GiB/s up from ~1.8 GiB/s). The Netpbm wire still declares
`TUPLTYPE RGB_ALPHA` (the format has no `BGR_ALPHA` magic) so files
round-trip through `Rgba` on decode unchanged; the new bench is
`encode_p7_bgra_320x240`. Round 250 closed the last decode bottleneck the
r248 P4 fast path left behind: binary `P5` / `P6` / `P7` 8-bit decode at
`maxval=255` and 16-bit decode at `maxval=65535` (with one of the
standard `GRAYSCALE` / `GRAYSCALE_ALPHA` / `RGB` / `RGB_ALPHA`
tupltypes, plus the depth-routed `Custom` / no-tupltype cases) used to
widen each wire byte into a `Vec<u16>` and then run the per-sample
`scale_to_*` / `to_le_bytes` loop in `samples_to_plane` even though
both transforms collapse to identity (8-bit) or a single byte swap
(16-bit). A new `try_decode_binary_bytewise` helper in `src/decoder.rs`
dispatches inside `decode_pbm` upfront body-length validation followed
by either a single `data.copy_from_slice(&body[..total])` (8-bit) or a
per-row `swap_bytes_u16_row` (16-bit) straight into the destination
plane. PAM combinations that re-arrange channels
(`BLACKANDWHITE` bit-pack, `BLACKANDWHITE_ALPHA` G→RGBA expansion,
16-bit `GRAYSCALE_ALPHA` widened to RGBA because the catalogue has no
`Ya16` variant) and any non-natural maxval still fall through to the
generic widen-then-rescale path unchanged. Apple-silicon numbers
against the round-249 baseline: decode `P5` 8-bit 640×480 ~6.1 µs
(~48 GiB/s, ≈28× faster), `P5` 16-bit 640×480 ~11.6 µs (~45 GiB/s),
`P6` 8-bit 640×480 ~16.5 µs (~49 GiB/s, ≈29× faster), `P6` 16-bit
320×240 ~9.3 µs (~47 GiB/s), `P7` `RGB_ALPHA` 8-bit 320×240 ~6.1 µs
(~48 GiB/s), `P7` `RGB_ALPHA` 16-bit 320×240 ~11.4 µs (~50 GiB/s,
≈7.3× faster). Round 248 closed the matching P4 decode bottleneck: `decode_pbm`
now dispatches a dedicated `decode_p4_monoblack` fast path that walks
the wire body through the same `copy_p4_row_msb` row helper, skipping
both the `Vec<u16>` sample-buffer allocation that `decode_binary`
would make and the per-bit re-pack loop in `samples_to_plane`'s
`MonoBlack` arm. Apple-silicon: decode `P4` 640×480 1.077 ms →
~2.07 µs (≈ 520× faster, ~17.3 GiB/s up from ~34 MiB/s).
P1 (ASCII bitmap) and P7 `BLACKANDWHITE` (which inverts the bit
sense per `pam(5)`) keep going through the generic path. Round 229 closed the last remaining bit-pack
bottleneck flagged by both the encode + decode bench headers:
`encode_p4` (binary PBM, MSB-packed bits) no longer unpacks the input
into a `w * h`-byte intermediate and re-packs it through a per-bit
OR loop. The crate's `MonoBlack` plane convention (`1 = black`,
MSB-first packed, row stride `w.div_ceil(8)`) is byte-for-byte
identical to the P4 wire format, so the body is now a per-row
`copy_from_slice` plus a one-byte trailing-pad mask on widths not a
multiple of 8 — encoded by a dedicated row-level helper
`copy_p4_row_msb`. Apple-silicon: encode `P4` 640×480 1.02 ms →
~1.72 µs (≈ 590× faster, ~20.7 GiB/s). Round 217 closed the remaining 16-bit
encode bottleneck: the LE→BE row swap for `Gray16Le` / `Rgb48Le` /
`Rgba64Le` planes (P5 16-bit, P6 16-bit, P7 16-bit `RGB` /
`RGB_ALPHA`) now funnels through a dedicated row-level
`swap_bytes_u16_row` helper that walks `chunks_exact(2)` over a
pre-resized `&mut [u8]` destination — same shape as the
round-205 PFM 32-bit helper. Round 222 closed the remaining symmetry
gap: `encode_p7_gray16` (PAM `GRAYSCALE` depth-1 16-bit, only
reachable via the explicit `Pam7` selector with `Gray16Le`) was the
last 16-bit encode path still using the per-sample
`out.push(chunk[1]); out.push(chunk[0])` pattern; it now shares the
same row-level helper as P5 / P6 / P7 RGB / RGBA, with a dedicated
benchmark (`encode_p7_gray16_320x240`) and a regression test that
asserts byte-equivalence against the canonical P5 16-bit path.
Round 210's `write_be16_row` helper
took `&[u16]` natively, so the encode paths that hold an LE byte
plane (no `Vec<u16>` materialisation) needed their own variant.
Apple-silicon numbers against the round-210 baseline: encode P5
16-bit 640×480 208.5 µs → ~11.8 µs (≈18× faster, ~48 GiB/s),
encode P6 16-bit 320×240 154.6 µs → ~8.6 µs (≈18× faster,
~50 GiB/s), encode P7 `RGBA64` 320×240 207.4 µs → ~11.7 µs
(≈18× faster, ~49 GiB/s). Round 210 factored the P5 / P6 / P7
16-bit big-endian decode and encode hot paths through row-level
`read_be16_row` / `write_be16_row` helpers (same shape as
round 205's PFM 32-bit helper), letting the inner load /
`from_be_bytes` / `to_be_bytes` sequence lower to a vectorised
byte-swap lane (`REV16.16B` on aarch64; `pshufb` / `vpshufb` on
x86). Round 205 closed the PFM big-endian
byte-swap bottleneck flagged in round 199: the per-sample
`out.push(s[3]); out.push(s[2]); out.push(s[1]); out.push(s[0])`
loop is now a row-level `swap_bytes_u32_row` helper that walks
`chunks_exact(4)` over a pre-resized `&mut [u8]` destination, which
LLVM lowers to vector `swap_bytes` (`REV32.16B` on aarch64,
`pshufb` / `vpshufb` on x86). The PFM baselines at 256×256 are now
`Pf` LE decode ~32 GiB/s, `Pf` BE decode ~31 GiB/s, `PF` LE decode
~27 GiB/s, `PF` BE decode ~28 GiB/s; encode `Pf` LE ~43 GiB/s,
`Pf` BE ~28 GiB/s (was ~1.86 GiB/s — **≈15× faster**), `PF` LE
~45 GiB/s, `PF` BE ~29 GiB/s (was ~1.86 GiB/s — **≈15.8× faster**).
Both decoders also funnel through the same helper, and the swap
itself is covered by two new unit tests. Round 189 rewrote
the ASCII hot path (direct digit writers + u32 accumulator, no
`to_string`/`parse` round-trips): 320×240 P1 encode 7 MiB/s →
~140 MiB/s, P2 encode 60 MiB/s → ~320 MiB/s, P3 encode 58 MiB/s →
~295 MiB/s, P2/P3 decode both up ≈30-40 %.

## Registration

```rust
let mut ctx = oxideav_core::RuntimeContext::new();
oxideav_pbm::register(&mut ctx);
```
