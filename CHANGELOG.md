# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
