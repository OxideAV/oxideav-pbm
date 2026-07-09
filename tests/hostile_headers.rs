//! Hostile-header regression suite.
//!
//! The `decode`/`header` fuzz targets enforce panic-freedom on random
//! bytes, but that contract only runs in the daily fuzz workflow. These
//! CI-visible tests pin the *typed* rejection of specific malformed
//! headers so a future refactor can't silently turn a clean `Err` into a
//! panic, an accept, or a silent clamp. Every case asserts
//! `PbmError::InvalidData` (never a panic, never `Ok`).
//!
//! Cases here target header fields whose bounds checks previously had no
//! dedicated test: the `MAXVAL` range guard (`1..=65535`) for both the
//! PNM and PAM headers, the PAM `DEPTH` lower bound, and a handful of
//! truncation / missing-separator shapes.

use oxideav_pbm::{decode_pbm, parse_header, PbmError};

/// Assert `decode_pbm(buf)` fails with `InvalidData` whose message
/// mentions `needle` — never panics, never returns `Ok`.
fn assert_invalid(buf: &[u8], needle: &str) {
    match decode_pbm(buf) {
        Ok((img, fmt)) => panic!(
            "expected InvalidData for {buf:?}, got Ok({}x{} {fmt:?})",
            img.width, img.height
        ),
        Err(PbmError::InvalidData(msg)) => assert!(
            msg.contains(needle),
            "message {msg:?} does not mention {needle:?} for {buf:?}"
        ),
        Err(other) => panic!("expected InvalidData for {buf:?}, got {other:?}"),
    }
}

// --- MAXVAL range guard: PNM (P2/P3/P5/P6) -------------------------------

#[test]
fn pnm_maxval_zero_is_rejected() {
    // maxval 0 is below the 1..=65535 range; a "graymap" with no
    // representable levels is malformed.
    assert_invalid(b"P2\n1 1\n0\n0\n", "out of range");
    assert_invalid(b"P5\n1 1\n0\n\x00", "out of range");
}

#[test]
fn pnm_maxval_above_65535_is_rejected() {
    // 65536 needs 17 bits — outside the 16-bit sample ceiling the family
    // permits. Must be rejected at the header, before any body read.
    assert_invalid(b"P2\n1 1\n65536\n0\n", "out of range");
    assert_invalid(b"P6\n1 1\n70000\n\x00\x00\x00\x00\x00\x00", "out of range");
}

#[test]
fn pnm_maxval_far_above_range_is_rejected_not_wrapped() {
    // A value that would overflow a naive u16 cast (e.g. 65535 + 65536)
    // must not wrap into a valid-looking maxval.
    assert_invalid(b"P5\n1 1\n131071\n\x00", "out of range");
}

// --- MAXVAL range guard: PAM (P7) ----------------------------------------

#[test]
fn pam_maxval_zero_is_rejected() {
    let buf = b"P7\nWIDTH 1\nHEIGHT 1\nDEPTH 1\nMAXVAL 0\nTUPLTYPE GRAYSCALE\nENDHDR\n\x00";
    assert_invalid(buf, "out of range");
}

#[test]
fn pam_maxval_above_65535_is_rejected() {
    let buf = b"P7\nWIDTH 1\nHEIGHT 1\nDEPTH 1\nMAXVAL 65536\nTUPLTYPE GRAYSCALE\nENDHDR\n\x00\x00";
    assert_invalid(buf, "out of range");
}

// --- PAM DEPTH lower bound ------------------------------------------------

#[test]
fn pam_depth_zero_is_rejected() {
    // DEPTH 0 means "no channels" — degenerate. (DEPTH > 4 already has a
    // test in encode_roundtrip.rs; this pins the lower bound.)
    let buf = b"P7\nWIDTH 1\nHEIGHT 1\nDEPTH 0\nMAXVAL 255\nTUPLTYPE GRAYSCALE\nENDHDR\n";
    assert!(
        decode_pbm(buf).is_err(),
        "DEPTH 0 must be rejected, not accepted"
    );
}

// --- Truncation / missing separator --------------------------------------

#[test]
fn magic_only_is_rejected_not_panicked() {
    // Just the two magic bytes, no dimensions. Header parse must fail
    // cleanly for every magic rather than index past the buffer.
    for m in [
        &b"P1"[..],
        b"P2",
        b"P3",
        b"P4",
        b"P5",
        b"P6",
        b"P7",
        b"Pf",
        b"PF",
    ] {
        assert!(
            parse_header(m).is_err(),
            "magic-only {m:?} must be a clean Err"
        );
        assert!(
            decode_pbm(m).is_err(),
            "magic-only {m:?} must be a clean Err"
        );
    }
}

#[test]
fn header_present_but_body_empty_is_truncation_error() {
    // Complete header, natural maxval, but zero body bytes: the binary
    // decoder must report truncation rather than allocate + read past the
    // end.
    assert_invalid(b"P5\n2 2\n255\n", "truncated");
    assert_invalid(b"P6\n2 2\n255\n", "truncated");
    assert_invalid(b"P4\n8 2\n", "truncated");
}

#[test]
fn pam_header_without_endhdr_is_rejected() {
    // A PAM key/value block that runs off the end before ENDHDR must not
    // spin or panic — it is a truncated header.
    let buf = b"P7\nWIDTH 4\nHEIGHT 4\nDEPTH 3\nMAXVAL 255\nTUPLTYPE RGB\n";
    assert!(decode_pbm(buf).is_err(), "missing ENDHDR must be an Err");
}

#[test]
fn non_numeric_dimension_token_is_rejected() {
    // A width token that is not a decimal integer.
    assert_invalid(b"P5\nxx 2\n255\n\x00\x00\x00\x00", "integer");
}

#[test]
fn zero_width_or_height_is_rejected() {
    // A zero dimension yields no pixels; the header parser rejects it for
    // every magic rather than emit an empty image the encoder can't
    // round-trip.
    //
    // Regression: the `recode` fuzz target found a `P1` with `height 0`
    // that `decode_pbm` used to *accept* (the ASCII path had no zero-dim
    // guard) — but the re-encoded binary `P4` was then rejected with
    // "zero dimension", breaking the decode→encode→decode fixed point.
    // The guard now lives in the header parser so every magic agrees.
    for buf in [
        // ASCII bitmap/graymap/pixmap (the fuzz-found case + siblings).
        &b"P1\n1 0\n1\n"[..],
        b"P1\n0 1\n1\n",
        b"P2\n0 4\n255\n0\n",
        b"P3\n4 0\n255\n0 0 0\n",
        // Binary.
        b"P5\n0 4\n255\n",
        b"P5\n4 0\n255\n",
        // PAM.
        b"P7\nWIDTH 0\nHEIGHT 1\nDEPTH 1\nMAXVAL 255\nTUPLTYPE GRAYSCALE\nENDHDR\n",
        b"P7\nWIDTH 1\nHEIGHT 0\nDEPTH 1\nMAXVAL 255\nTUPLTYPE GRAYSCALE\nENDHDR\n",
    ] {
        assert_invalid(buf, "zero");
    }
}

#[test]
fn zero_dimension_ascii_p1_recode_regression() {
    // The exact byte shape the `recode` fuzz target flagged: `P1`,
    // width 1, height 0, with form-feed / CR whitespace runs. Must fail
    // at decode, not sneak through and blow up on re-encode.
    let crash = b"P1\x0c\x0c1\x0c\x0c\x0c0\x0c\x0c\x0c\x0c\x0c\r\r\r\r";
    assert_invalid(crash, "zero");
}
