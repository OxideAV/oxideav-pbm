#![no_main]

//! Fuzz: arbitrary bytes → `oxideav_pbm::parse_header`.
//!
//! Drives the header parser in isolation from the body decoder. The
//! Netpbm header is the rich state machine — a 2-byte magic, then
//! either a PNM line-oriented `<width> <height> [<maxval>]` block with
//! comment tolerance everywhere (`# … LF` may appear anywhere in or
//! between tokens) or a PAM multi-line key/value block terminated by
//! `ENDHDR`. Single-byte mutations against either block exercise the
//! tokenizer, comment skipping, integer parsing, maxval-range
//! validation, PAM unknown-key tolerance, and the
//! TUPLTYPE-vs-DEPTH consistency check.
//!
//! Contract: `parse_header` returns a `Result`. Malformed input yields
//! `Err(PbmError::InvalidData / Unsupported)`. Neither path may panic
//! on out-of-bounds, integer overflow, or non-UTF-8.

use libfuzzer_sys::fuzz_target;
use oxideav_pbm::parse_header;

fuzz_target!(|data: &[u8]| {
    if data.len() > 64 * 1024 {
        // Headers are tiny — 64 KiB is already absurdly large.
        return;
    }
    let _ = parse_header(data);
});
