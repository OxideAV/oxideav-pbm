#![no_main]

//! Fuzz: arbitrary bytes → `oxideav_pbm::decode_pfm`.
//!
//! Drives the Portable FloatMap decoder in isolation from the unified
//! PNM/PAM path. PFM has its own strict three-line header (magic,
//! `width height`, scale) — no comments, no CRLF, single-LF
//! terminator — plus a 4-byte-per-sample raster with a sign-driven
//! byte-order selector and bottom-to-top row order. Mutations against
//! the header (an extra LF, a stray `#`, a NaN scale, a non-`PF`/`Pf`
//! first line) and against the raw float body (truncated, oversized,
//! misaligned) all need to surface as `Err(PbmError::…)`.
//!
//! Contract: `decode_pfm` must never panic. Specifically:
//!   - the strict header reader rejects anything other than three
//!     LF-terminated lines with no `\r` and no `#`;
//!   - the dimension overflow guards (row_bytes / raster_bytes) catch
//!     attacker-claimed widths and heights before allocation;
//!   - the body-length check rejects truncated streams;
//!   - the big-endian-to-little-endian byte swap walks
//!     `chunks_exact(4)` so a body whose length isn't a multiple of
//!     four cannot underflow (the truncation check happens first).
//!
//! The 256 KiB input cap matches the `decode` harness so libFuzzer
//! reaches deeper coverage in the daily 30-minute budget instead of
//! burning cycles on huge inputs.

use libfuzzer_sys::fuzz_target;
use oxideav_pbm::decode_pfm;

fuzz_target!(|data: &[u8]| {
    if data.len() > 256 * 1024 {
        return;
    }
    let _ = decode_pfm(data);
});
