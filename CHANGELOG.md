# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- Round 248: P4 (binary PBM) decode → `MonoBlack` rewritten as a per-row
  memcpy. The Netpbm wire format for P4 (1 bit per pixel, MSB-first
  packed, rows padded to a byte boundary, `1 = black` per `pbm(5)`) is
  byte-for-byte identical to the crate's `MonoBlack` plane convention,
  so the body is a straight per-row `copy_from_slice` with at most one
  trailing-bit mask on the last byte of each row when `w % 8 != 0`. The
  pre-r248 decode path ran two scalar bit loops — `decode_binary`
  allocated a `Vec<u16>` sized at `width * height` and walked each bit
  of the body into a one-per-pixel `u16` sample, then
  `samples_to_plane`'s `MonoBlack` arm walked every pixel again
  (`s.samples[y * w + x] != 0 → data[y * stride + x / 8] |= 1 << (7 -
  (x % 8))`) re-packing the bits into the destination plane. Both loops
  plus the `Vec<u16>` intermediate are gone for the P4 case; a
  dedicated `decode_p4_monoblack` fast path in `src/decoder.rs`
  dispatches inside `decode_pbm` right alongside the existing PFM
  dispatch and funnels through the round-229 `copy_p4_row_msb` row
  helper (the same kernel the P4 *encoder* uses, now driving both
  directions). Apple-silicon numbers at 640×480:
  - decode `P4` 1.077 ms → ~2.07 µs (≈ 520× faster, ~17.3 GiB/s up from
    ~34 MiB/s).
  P1 (ASCII bitmap) and P7 `BLACKANDWHITE` (which inverts the bit sense
  per `pam(5)` `TUPLE TYPE`) keep going through the generic
  `decode_binary` / `samples_to_plane` path; the fast path triggers
  only when the magic is `P4BinaryBitmap`. Adds five decoder-level
  unit tests covering byte-aligned widths (pure memcpy),
  unaligned-width trailing-pad masking, full-suite agreement with the
  pre-r248 generic re-pack for every `width % 8` case 1..=129 across
  multi-row inputs, the body-truncation rejection (mirrors the P4
  `binary_huge_dimension_does_not_oom` regression on the generic path),
  and an upfront OOM rejection on a multi-billion-height header.
  `decode_binary` itself is unchanged — it remains the path for P5 /
  P6 / P7 — so the round 171 OOM hardening + the round 210 / 217 /
  222 16-bit row-level helpers continue to apply unmodified. The
  existing four `MonoBlack` round-trip and PBM regression tests
  (including the round 229 `encode_p4` symmetry tests) continue to
  pass unchanged.

### Added

- Round 236: typed comment-iteration accessor
  `iter_pnm_header_comments(input: &[u8]) -> PnmHeaderComments<'_>`
  on the header surface. The man pages (`pbm(5)` / `pgm(5)` / `ppm(5)`
  / `pnm(5)` / `pam(5)`) permit `# … LF` comment lines anywhere in
  the PNM/PAM header, and the decoder already tolerates them silently
  — this accessor surfaces them as a non-allocating `Iterator<Item =
  &[u8]>` so a caller (image-tool round-trip, container-to-container
  metadata forwarding, "Created by …" provenance preservation) can
  read them without re-walking the header bytes. The iterator stops at
  the start of the pixel data (so a `#` byte that occurs as a valid
  P5 / P6 / P7 sample is never misread as a comment) and yields each
  comment body trimmed of surrounding ASCII whitespace. Portable
  FloatMap inputs (`Pf` / `PF`) yield zero items by spec — the
  Debevec reference forbids comments in the three-line PFM header.
  Unrecognised magics also yield zero items rather than erroring, so
  callers can treat the accessor as best-effort. New public types:
  `PnmHeaderComments<'_>` (the iterator) and `iter_pnm_header_comments`
  (the constructor); both are re-exported from the crate root. Adds
  six header-level unit tests covering single-comment P4 headers,
  multi-comment + inline-tail P3 headers, PAM-block-interleaved
  comments under P7, the PFM forbidden-comment + valid-no-comment
  pair, the unrecognised-magic empty path, and the
  pixel-data-with-`#`-byte boundary stop for binary P5 — confirming
  the iterator does not bleed past `data_offset` into pixel bytes.

### Changed

- Round 229: P4 (binary PBM) encode bit packer rewritten as a per-row
  memcpy. The crate's `MonoBlack` plane convention (`1 = black`,
  MSB-first packed, row stride `w.div_ceil(8)`) is byte-for-byte
  identical to the P4 wire format, so the body is a straight copy with
  a single trailing-bit mask on the last byte of each row when
  `w % 8 != 0`. The pre-r229 path had two scalar bit loops — an
  unpack-to-bytes pass over the input plane followed by a re-pack pass
  through `encode_p4_body`'s per-bit `row[x / 8] |= 1 << (7 - (x % 8))`
  OR — plus a `w * h`-byte intermediate `Vec<u8>` allocation. Both
  loops and the intermediate allocation are gone; the per-row work is
  now `copy_from_slice` + at most one `&= mask`. New row-level helper
  `binary::copy_p4_row_msb(&[u8], &mut [u8], width)` encapsulates the
  copy + trailing-pad mask. Apple-silicon numbers against the
  round-222 baseline:
  - encode `P4` 640×480 1.02 ms → 1.72 µs (≈ 590× faster,
    ~20.7 GiB/s up from ~36 MiB/s).
  Adds three new helper-level unit tests (byte-aligned width =
  pure memcpy, unaligned width = trailing-pad mask, full-suite
  agreement with the legacy `encode_p4_body` path for every
  `width % 8` case 1..=33) plus three encoder-level regressions
  asserting the byte-aligned, unaligned-padded, and strided-plane
  inputs all produce the same on-disk bytes the previous loop did.
  `encode_p4_body` stays public for callers that hold an unpacked
  bit plane (one byte per pixel); only `encode_p4` (the
  `MonoBlack`-plane fast path) bypasses it. The four `MonoBlack`
  round-trip and PBM regression tests continue to pass unchanged.

- Round 222: PAM `GRAYSCALE` 16-bit encode (`encode_p7_gray16`) now
  shares the round-217 `swap_bytes_u16_row` row-level helper instead
  of the per-sample `out.push(chunk[1]); out.push(chunk[0])` loop the
  path retained when its P5 / P6 / P7 RGB / RGBA siblings moved to the
  helper. The path is only reachable via the explicit `Pam7` encode
  selector with `Gray16Le` (the auto routing for `Gray16Le` goes to
  P5), so it was the lone 16-bit encode hot path still on the scalar
  pattern. Added a dedicated `encode_p7_gray16_320x240` criterion
  bench and a regression unit test asserting byte-equivalence against
  the canonical P5 16-bit body.
- Round 217: 16-bit encode LE→BE row swap factored through a dedicated
  `swap_bytes_u16_row(&[u8], &mut [u8])` helper in `binary.rs`,
  mirroring the round-205 PFM 32-bit helper's shape. The encode hot
  paths for `Gray16Le` / `Rgb48Le` / `Rgba64Le` planes hold an LE byte
  plane directly and need to write BE bytes without ever materialising
  a `Vec<u16>`, so they could not reuse round 210's
  `write_be16_row(&[u16], &mut [u8])` (which assumes `&[u16]` input).
  Pre-r217 the four encode paths (`encode_p5_gray16`,
  `encode_p6_rgb16`, `encode_p7_rgb16`, `encode_p7_rgba16`) ran
  `for chunk in row.chunks_exact(2) { out.push(chunk[1]);
  out.push(chunk[0]); }` — the per-sample `Vec::push` calls
  forced a scalar loop. The new helper walks `chunks_exact(2)`
  zipped with `chunks_exact_mut(2)` over a pre-resized destination,
  letting LLVM lower the swap to a vector lane (`REV16.16B` on
  aarch64; `pshufb` / `vpshufb` on x86). Apple-silicon numbers against
  the round-210 baseline:
  - encode `P5` 16-bit 640×480 208.5 µs → ~11.8 µs (≈18× faster,
    ~48 GiB/s).
  - encode `P6` 16-bit 320×240 154.6 µs → ~8.6 µs (≈18× faster,
    ~50 GiB/s).
  - encode `P7` `RGBA64` 320×240 207.4 µs → ~11.7 µs (≈18× faster,
    ~49 GiB/s).
  (`encode_p7_rgb16` shares the same kernel and gains the same
  speedup; it has no dedicated bench but is exercised by the P7 RGB
  16-bit roundtrip suite.) Adds three unit tests covering
  `swap_bytes_u16_row` (the swap kernel, a self-inverse property, and
  byte-for-byte agreement with the scalar
  `u16::from_le_bytes(…).to_be_bytes()` reference path). The existing
  P5 / P6 / P7 16-bit roundtrip suites already exercise the row layout
  end-to-end (`encode_p5_gray16_swaps_to_be`,
  `explicit_format_pam7_rgb16_be_swap`, etc.).

- Round 210: P5 / P6 / P7 16-bit binary body hot paths factored through
  row-level helpers (`read_be16_row` / `write_be16_row`), mirroring
  the shape of the round-205 PFM 32-bit helper. The decode loop now
  walks `chunks_exact(2)` zipped with `out.iter_mut()` and the encode
  loop writes into a pre-sized `vec![0u8; samples.len() * 2]` via
  `chunks_exact_mut(2)` instead of `Vec::extend_from_slice`. The inner
  load / `from_be_bytes` / store sequence lowers to a vectorised
  byte-swap lane (`REV16.16B` on aarch64, `pshufb` / `vpshufb` on x86)
  without any hand-rolled intrinsics. Measured on apple-silicon
  against the round-205 baseline:
  - encode `P5` 16-bit 640×480 217.7 µs → 208.5 µs (≈ -4 %).
  - encode `P6` 16-bit 320×240 157.8 µs → 154.6 µs (≈ -2 %).
  - encode `P7` `RGBA64` 320×240 204.1 µs → 207.4 µs (within noise).
  Decode 16-bit paths reuse the same `read_be16_row` helper and stay
  flat (~213 µs P5 / ~93 µs P6 / ~80 µs P7 at the same sizes). The
  win is largest on the encode side because the original
  `Vec::extend_from_slice` path inhibited the SIMD lowering; the
  pre-sized destination unlocks it. Adds three unit tests covering
  `read_be16_row`, `write_be16_row`, and their round-trip
  self-inverse property; the existing P5/P6/P7 round-trip suites
  already exercise the row layout end-to-end.

- Round 205: Portable FloatMap big-endian byte-swap hot path. The per-sample
  4-byte swap that round 199's PFM benches flagged as the obvious SIMD
  target (encode `Pf` BE ~1.86 GiB/s vs LE ~42 GiB/s, `PF` BE ~1.86 GiB/s
  vs LE ~45 GiB/s) is now routed through a row-level `swap_bytes_u32_row`
  helper. The inner loop walks `chunks_exact(4)` over a pre-resized
  `&mut [u8]` destination instead of pushing individual bytes onto a
  `Vec`, so the compiler can lower it to a vector `swap_bytes` lane
  (`REV32.16B` on aarch64, `pshufb` / `vpshufb` on x86) without any
  hand-rolled intrinsics. Same helper is shared by the decode BE path
  for symmetry. Measured on apple-silicon at 256×256:
  - encode `Pf` BE 1.86 GiB/s → ~27.8 GiB/s (≈15× faster).
  - encode `PF` BE 1.83 GiB/s → ~28.9 GiB/s (≈15.8× faster).
  - decode `Pf` BE 30.5 GiB/s → ~31.2 GiB/s.
  - decode `PF` BE 24.6 GiB/s → ~28.3 GiB/s (≈+15 %).
  LE paths (no swap) are unchanged. Adds two unit tests on the helper
  covering the swap kernel and self-inverse property; the existing PFM
  round-trip suite already exercises the row layout end-to-end.

### Added

- Round 199: Portable FloatMap coverage for the fuzz + bench
  matrices. A dedicated `pfm` cargo-fuzz target drives `decode_pfm`
  directly so the daily 30-minute fuzz budget exercises the strict
  three-line header (no comments, no CRLF, single-LF terminator,
  sign-of-scale endianness selector), the raster-size overflow guards,
  the body-truncation check, and the big-endian byte-swap kernel —
  none of which is reachable through the existing `decode` / `header`
  harnesses (PFM is disjoint from the PNM/PAM tokenizer). Twelve new
  Criterion benches (four each across `benches/{decode,encode,
  roundtrip}.rs`) cover `Pf` / `PF` × LE / BE at 256×256 with
  finite-valued synthetic float input (no NaN / inf), giving a stable
  baseline for a future SIMD byte-swap pass against the current
  per-sample loop. Indicative apple-silicon numbers at 256×256:
  decode `Pf` LE ~32 GiB/s, `Pf` BE ~30 GiB/s, `PF` LE ~27 GiB/s,
  `PF` BE ~21 GiB/s; encode `Pf` / `PF` LE ~42-45 GiB/s vs BE
  ~1.86 GiB/s (the BE encode is the clear bottleneck, dominated by
  the scalar 4-byte swap).

## [0.0.4](https://github.com/OxideAV/oxideav-pbm/compare/v0.0.3...v0.0.4) - 2026-05-29

### Other

- ASCII (P1/P2/P3) hot-path: direct digit writers + u32 accumulator
- Portable FloatMap (Pf/PF) decode + encode
- accept user-defined PAM TUPLTYPE names verbatim
- criterion harness for decode / encode / roundtrip hot paths
- cargo-fuzz harness + pre-allocation OOM hardening
- explicit ASCII-vs-binary format selector + parser hardening

### Changed

- Round 189: ASCII (P1 / P2 / P3) encoder and decoder hot paths
  rewritten to remove per-sample heap allocations and `str::parse`
  trips through UTF-8. `encode_ascii_body` now appends digits to the
  output buffer through stack scratch instead of `s.to_string()`; two
  new internal entry points (`encode_ascii_body_u8`,
  `encode_ascii_body_bits`) skip the temporary `Vec<u16>` widen for the
  common P2 `Gray8` / P3 `Rgb24` / P1 `MonoBlack` cases (samples are
  already u8 in the source plane). `next_uint` accumulates digits
  directly into a `u32` with `checked_mul`/`checked_add` overflow guards
  (still rejects malformed input with `InvalidData`). Measured against
  the r176 Criterion baseline on apple-silicon, 320×240 figures:
  - encode P1 7.3 MiB/s → ~139 MiB/s (≈19× faster).
  - encode P2 59.6 MiB/s → ~322 MiB/s (≈5.4× faster).
  - encode P3 58.4 MiB/s → ~295 MiB/s (≈5.1× faster).
  - decode P2 110.7 MiB/s → ~140 MiB/s (≈1.3× faster).
  - decode P3 118.8 MiB/s → ~168 MiB/s (≈1.4× faster).
  Binary paths (P4–P7, PFM) are untouched. Adds four targeted unit
  tests covering the new `write_u8_dec` / `write_u16_dec` digit-width
  branches and the overflow-rejection path on `next_uint`.

### Added

- Round 185: Portable FloatMap (`Pf` / `PF`) decode + encode — the
  floating-point member of the family, storing raw IEEE-754 binary32
  samples (1 channel for `Pf` grayscale, 3 interleaved R/G/B for `PF`
  colour). Reference: `docs/image/netpbm/pfm-portable-floatmap.md`
  (Debevec PFM reference). New `PbmPixelFormat::GrayF32` (4 B/px) and
  `PbmPixelFormat::RgbF32` (12 B/px) variants store float samples
  little-endian in memory. The PFM header is parsed by a dedicated strict
  reader: exactly three LF-terminated lines (magic, `width height`,
  scale) with **no comments** and **no CRLF** — embedded `#`, carriage
  returns, and missing LF terminators are rejected. The scale line's sign
  selects byte order (negative ⇒ little-endian, positive ⇒ big-endian)
  and its absolute value is preserved as an advisory scale factor
  (reported, not applied to the pixels). On-disk rows are bottom-to-top;
  the codec flips them to a conventional top-to-bottom in-memory plane
  and normalises big-endian samples to the little-endian in-memory
  contract. New public API: `decode_pfm` / `encode_pfm` /
  `encode_pfm_plane` plus the `PfmHeaderInfo` (byte order + scale +
  channels) and `header::PfmInfo` types; the unified `decode_pbm` /
  `encode_pbm` entry points route `Pf` / `PF` automatically (encoding
  defaults to little-endian, unit scale), and `PbmEncodeFormat::Pfm`
  pins the float form explicitly. The container probe/extension layer
  recognises the `Pf` / `PF` magics and the `.pfm` extension; because the
  two float formats have no `oxideav_core::PixelFormat` counterpart,
  `pbm_to_pixel_format` now returns `Option<PixelFormat>` (`None` for the
  float maps) and the demuxer advertises no pixel format for PFM streams
  (the decoder is self-describing from the byte stream). Adds header-level
  unit tests (PFM big/little-endian parse, CRLF rejection, comment
  rejection, NaN-scale rejection, zero-dimension rejection), body-level
  unit tests in `src/pfm.rs` (gray/RGB round-trips at both byte orders,
  non-unit scale, bottom-to-top flip, big-endian disk-byte swap,
  truncation/format/scale rejection), and `tests/pfm_roundtrip.rs`
  integration coverage through the public API. The standalone
  (`--no-default-features`) build compiles unchanged.

- Round 183: user-defined `TUPLTYPE` support. The PAM spec (pam(5))
  explicitly permits arbitrary tuple-type names beyond the six standard
  ones (`BLACKANDWHITE` / `GRAYSCALE` / `RGB` / `BLACKANDWHITE_ALPHA` /
  `GRAYSCALE_ALPHA` / `RGB_ALPHA`) — producers in HDR / depth-map /
  normal-map / scientific-imaging pipelines routinely emit names like
  `DEPTH_MAP`, `RGBE`, `NORMAL_MAP`, `OPACITY`, and arbitrary
  multi-channel volumes. The header parser previously rejected every
  such file with `Unsupported`; it now round-trips the name verbatim
  through a new `Tupltype::Custom(String)` variant and routes the
  channels through the existing depth-based fallback layout (the same
  table used when `TUPLTYPE` is omitted entirely). Standard names
  still pin their channel layout, and the consistency check
  (`TUPLTYPE RGB` with `DEPTH 4` etc.) is preserved for them. Empty
  `TUPLTYPE` values are rejected with `InvalidData` instead of being
  silently coerced into `Custom("")`. Drops `Copy` from `Tupltype`
  (the `Custom` arm holds an owned `String`); the type stays `Clone +
  PartialEq + Eq` and `Tupltype::name()` / `Tupltype::channels()` now
  take `&self`. Adds five header-level unit tests
  (`accepts_user_defined_tupltype`,
  `custom_tupltype_with_any_depth_in_range`,
  `rejects_empty_tupltype_value`,
  `standard_tupltype_channel_check_still_applies`) plus five
  integration tests in `tests/encode_roundtrip.rs` exercising the full
  `decode_pbm` pipeline at depths 1 / 3 / 4 at both 8-bit and 16-bit,
  including the depth-outside-range rejection. Closes the round-1
  deferral listed in the README.

- Round 176 (depth-mode benchmarks): three Criterion bench binaries
  under `benches/{decode,encode,roundtrip}.rs` covering every binary
  magic (P4/P5/P6/P7) at 8 and 16-bit plus the three ASCII magics
  (P1/P2/P3). Inputs are synthesised in-bench from a deterministic
  xorshift seed (no committed fixtures) and pushed through the public
  `encode_pbm` / `encode_pbm_ascii` / `encode_pbm_with_format` /
  `decode_pbm` API. Establishes the r176 throughput baseline so future
  optimisation rounds (SIMD byte-swap for 16-bit P5/P6, lookup-table
  ASCII writer for P2/P3, branch-free P4 bit packer) can A/B-compare.
  Run with `cargo bench -p oxideav-pbm --bench <name>`. `criterion =
  "0.5"` pinned to the same line as the other OxideAV crates with
  benches (png / flac / tiff / cinepak / tta / magicyuv / h264 /
  pixfmt). Standalone (`--no-default-features`) build still compiles
  unchanged — benches are dev-only and the `dev-dependencies` block
  carries no `oxideav-core` dependency.

- `fuzz/` cargo-fuzz harness with three libfuzzer targets covering the
  parser surface end-to-end (`decode`), the header-only state machine
  (`header`), and the encoder × every `PbmEncodeFormat` pair
  (`encode_roundtrip`). Seed corpus committed: 15 hand-curated
  `decode/`, 13 `header/`, and 6 `encode_roundtrip/` seeds covering
  every magic, both 8-bit and 16-bit depths, comment placement, CRLF
  line endings, PAM BLACKANDWHITE inversion, and PAM unknown-key
  tolerance. Local panic-freedom verified across 9.3M decode + 6.8M
  header + 0.4M encoder runs (>=30 s each) with no crashes after the
  OOM fix below. Daily CI workflow at `.github/workflows/fuzz.yml`
  (06:47 UTC, 30 min total budget split across the three targets) via
  the shared `OxideAV/.github` reusable `crate-fuzz.yml` workflow.

### Fixed

- Pre-allocation OOM in `decode_ascii` / `decode_binary`: a header
  claiming a multi-billion `width * height` (e.g.
  `P2\n2 200888808\n50\n…`) triggered an unchecked `Vec::with_capacity`
  / `vec![0u16; …]` and OOMed the process. Both decoders now compute
  the required body length first and reject the input upfront with
  `InvalidData` when the claimed dimensions exceed what the body could
  possibly contain. `samples_to_plane` adds a matching defence-in-depth
  `stride * height` overflow check before allocating the output plane.
  Regression tests committed in `src/ascii.rs` /
  `src/binary.rs` (`ascii_huge_dimension_does_not_oom`,
  `binary_huge_dimension_does_not_oom`); the original libFuzzer artifact
  is preserved as `fuzz/corpus/decode/regression_oom_huge_height.bin`.

### Added

- `encode_pbm_with_format(&PbmImage, PbmEncodeFormat) -> Result<Vec<u8>>`
  public API: callers can now pin the on-disk magic explicitly instead
  of relying on the auto-selected closest-match. The `PbmEncodeFormat`
  enum has one variant per Netpbm magic (`Pnm1` / `Pnm2` / `Pnm3` /
  `Pnm4` / `Pnm5` / `Pnm6` / `Pam7`) plus `AutoBinary` / `AutoAscii`
  convenience modes that match the existing `encode_pbm` /
  `encode_pbm_ascii` behaviour.
- P7 PAM encoder now handles `GRAYSCALE` and `RGB` tuple types (depths
  1 / 3) — previously the P7 encoder only emitted the alpha-bearing
  tuple types (`GRAYSCALE_ALPHA`, `RGB_ALPHA`) because the auto-format
  selector always preferred P5 / P6 for non-alpha pixel formats. The
  new `PbmEncodeFormat::Pam7` selector exercises the new path.

### Fixed

- Lenient parser hardening: added regression tests for MAXVAL=1 on
  ASCII PGM (`P2`) and binary PGM (`P5`) — the spec permits a
  degenerate "PBM-as-PGM" form, which the existing decode path scales
  correctly to `Gray8` (0 / 0xFF) but had no test coverage.
- Regression tests for header tolerances: every ASCII whitespace
  category (space / tab / CR / LF / VT / FF) accepted between samples,
  comments interleaved at every legal position in ASCII P2 bodies,
  blank + comment lines anywhere in the PAM header, CRLF line endings
  in PAM, and unknown PAM header keys silently ignored per the
  "implementations should ignore unknown keys" man-page guidance.
- `Tupltype::channels()` simplified: the awkward nested `match` arm
  that grouped `Rgb` with the 2-channel alpha types and then
  disambiguated inside has been flattened. No behaviour change.

## [0.0.3](https://github.com/OxideAV/oxideav-pbm/compare/v0.0.2...v0.0.3) - 2026-05-06

### Other

- drop stale REGISTRARS / with_all_features intra-doc links
- drop dead `linkme` dep
- re-export __oxideav_entry from registry sub-module
- auto-register via oxideav_core::register! macro (linkme distributed slice)
- unify entry point on register(&mut RuntimeContext) ([#502](https://github.com/OxideAV/oxideav-pbm/pull/502))

## [0.0.2](https://github.com/OxideAV/oxideav-pbm/compare/v0.0.1...v0.0.2) - 2026-05-04

### Fixed

- *(clippy)* replace needless_range_loop + useless_vec

### Other

- Standalone-friendly retrofit: gate oxideav-core behind `registry`

### Changed

- Standalone-friendly retrofit (#360): `oxideav-core` is now an
  optional dep behind a default-on `registry` cargo feature.
  Image-library consumers can depend on `oxideav-pbm` with
  `default-features = false` to get a framework-free build that
  exposes the standalone `decode_pbm` / `encode_pbm` /
  `encode_pbm_ascii` API plus crate-local `PbmImage` /
  `PbmPixelFormat` / `PbmError` types. The `Decoder` / `Encoder`
  trait surface and the container registration stay behind the
  `registry` feature.
- `encode_pbm` / `encode_pbm_ascii` signature simplified to take a
  `&PbmImage` (carrying width, height, pixel format inline). New
  `encode_pbm_plane` / `encode_pbm_ascii_plane` helpers expose the
  underlying plane-based API.

### Added

- Initial release: pure-Rust Netpbm codec + container covering all
  eight magic numbers (P1-P7).
- Decode: P1/P4 (1-bit `MonoBlack`), P2/P5 (`Gray8` / `Gray16Le`),
  P3/P6 (`Rgb24` / `Rgb48Le`), P7 PAM with the six standard
  `TUPLTYPE`s (`BLACKANDWHITE`, `GRAYSCALE`, `RGB`,
  `BLACKANDWHITE_ALPHA`, `GRAYSCALE_ALPHA`, `RGB_ALPHA`) at any
  `MAXVAL` 1..=65535.
- Encode: chooses the closest binary form (P4/P5/P6/P7) for the
  input `PixelFormat`. Plain ASCII output (P1/P2/P3) available via
  the dedicated entry point.
- Tolerates comments (`# … LF`) in headers and in P1/P2/P3 bodies.
- Container + codec registration matching every other image sibling.
