#![no_main]

//! Fuzz: arbitrary bytes → `oxideav_pbm::decode_pbm`.
//!
//! Contract: every public decode entry point MUST return a `Result` for
//! malformed input — never panic, never abort, never over-allocate
//! based on attacker-claimed dimensions. The decoder's
//! `samples_to_plane` helper allocates `stride * height` bytes from the
//! parsed header, so a malicious input with a huge width / height /
//! depth field is the easiest panic surface.
//!
//! The harness imposes a 256 KiB input cap so libFuzzer doesn't burn
//! cycles on inputs that the public API would already reject for being
//! larger than any plausible image header. The full decoder pipeline
//! exercised is:
//!
//!     parse_header
//!       → decode_ascii / decode_binary (body decoder)
//!         → samples_to_plane (per-format buffer allocator)
//!
//! Errors from any of the three layers are fine — the contract is that
//! they surface as `Err(PbmError::…)` rather than a panic.

use libfuzzer_sys::fuzz_target;
use oxideav_pbm::decode_pbm;

fuzz_target!(|data: &[u8]| {
    // 256 KiB cap. The Netpbm header is at most ~80 bytes for any
    // legitimate file; the payload can be arbitrarily large but for
    // panic-discovery we only need a few-KiB window. The cap also keeps
    // the per-input runtime low enough that libFuzzer reaches deeper
    // coverage in the available time budget.
    if data.len() > 256 * 1024 {
        return;
    }
    let _ = decode_pbm(data);
});
