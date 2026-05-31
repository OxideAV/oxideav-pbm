# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
