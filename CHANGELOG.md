# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
