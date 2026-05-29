//! End-to-end encode → decode roundtrip across every supported
//! Netpbm output format.

use oxideav_pbm::{
    decode_pbm, encode_pbm, encode_pbm_ascii, encode_pbm_with_format, PbmEncodeFormat, PbmImage,
    PbmPixelFormat, PbmPlane,
};

/// Build a deterministic test pattern with `bytes_per_pixel` samples
/// per pixel.
fn pattern(w: u32, h: u32, bpp: usize, format: PbmPixelFormat) -> PbmImage {
    let mut data = Vec::with_capacity((w * h) as usize * bpp);
    for y in 0..h {
        for x in 0..w {
            for c in 0..bpp {
                data.push(((x.wrapping_mul(7) + y.wrapping_mul(13) + c as u32) & 0xFF) as u8);
            }
        }
    }
    PbmImage {
        width: w,
        height: h,
        pixel_format: format,
        planes: vec![PbmPlane {
            stride: w as usize * bpp,
            data,
        }],
        pts: None,
    }
}

#[test]
fn roundtrip_p4_monoblack() {
    // MonoBlack is 1bpp packed MSB-first; build it directly.
    let w = 17u32; // not byte-aligned, exercises padding
    let h = 5u32;
    let row_bytes = (w as usize).div_ceil(8);
    let mut data = vec![0u8; row_bytes * h as usize];
    for y in 0..h as usize {
        for x in 0..w as usize {
            if (x + y) % 3 == 0 {
                data[y * row_bytes + x / 8] |= 1 << (7 - (x % 8));
            }
        }
    }
    let src = PbmImage {
        width: w,
        height: h,
        pixel_format: PbmPixelFormat::MonoBlack,
        planes: vec![PbmPlane {
            stride: row_bytes,
            data: data.clone(),
        }],
        pts: None,
    };
    let bytes = encode_pbm(&src).unwrap();
    assert!(bytes.starts_with(b"P4\n"));
    let (back, fmt) = decode_pbm(&bytes).unwrap();
    assert_eq!(fmt, PbmPixelFormat::MonoBlack);
    // Check just the populated bits — padding bits in the last byte of
    // each row are unspecified (the encoder zeros them, but the
    // decoder might not — tolerate either by comparing per-pixel).
    for y in 0..h as usize {
        for x in 0..w as usize {
            let exp = (data[y * row_bytes + x / 8] >> (7 - (x % 8))) & 1;
            let got = (back.planes[0].data[y * back.planes[0].stride + x / 8] >> (7 - (x % 8))) & 1;
            assert_eq!(exp, got, "bit at ({x},{y}) differs");
        }
    }
}

#[test]
fn roundtrip_p5_gray8() {
    let src = pattern(20, 13, 1, PbmPixelFormat::Gray8);
    let bytes = encode_pbm(&src).unwrap();
    assert!(bytes.starts_with(b"P5\n"));
    let (back, fmt) = decode_pbm(&bytes).unwrap();
    assert_eq!(fmt, PbmPixelFormat::Gray8);
    assert_eq!(back.planes[0].data, src.planes[0].data);
}

#[test]
fn roundtrip_p5_gray16() {
    // 16-bit gray: build directly so we can pick non-trivial values.
    let w = 8u32;
    let h = 4u32;
    let mut data = Vec::with_capacity((w * h) as usize * 2);
    for y in 0..h as usize {
        for x in 0..w as usize {
            let v = ((x * 7919 + y * 31337) & 0xFFFF) as u16;
            data.extend_from_slice(&v.to_le_bytes());
        }
    }
    let src = PbmImage {
        width: w,
        height: h,
        pixel_format: PbmPixelFormat::Gray16Le,
        planes: vec![PbmPlane {
            stride: w as usize * 2,
            data,
        }],
        pts: None,
    };
    let bytes = encode_pbm(&src).unwrap();
    assert!(bytes.starts_with(b"P5\n"));
    assert!(bytes.windows(5).any(|w| w == b"65535"));
    let (back, fmt) = decode_pbm(&bytes).unwrap();
    assert_eq!(fmt, PbmPixelFormat::Gray16Le);
    assert_eq!(back.planes[0].data, src.planes[0].data);
}

#[test]
fn roundtrip_p6_rgb8() {
    let src = pattern(16, 12, 3, PbmPixelFormat::Rgb24);
    let bytes = encode_pbm(&src).unwrap();
    assert!(bytes.starts_with(b"P6\n"));
    let (back, fmt) = decode_pbm(&bytes).unwrap();
    assert_eq!(fmt, PbmPixelFormat::Rgb24);
    assert_eq!(back.planes[0].data, src.planes[0].data);
}

#[test]
fn roundtrip_p6_rgb16() {
    let w = 6u32;
    let h = 5u32;
    let mut data = Vec::with_capacity((w * h) as usize * 6);
    for y in 0..h as usize {
        for x in 0..w as usize {
            for c in 0..3 {
                let v = ((x * 11 + y * 23 + c * 41) & 0xFFFF) as u16;
                data.extend_from_slice(&v.to_le_bytes());
            }
        }
    }
    let src = PbmImage {
        width: w,
        height: h,
        pixel_format: PbmPixelFormat::Rgb48Le,
        planes: vec![PbmPlane {
            stride: w as usize * 6,
            data,
        }],
        pts: None,
    };
    let bytes = encode_pbm(&src).unwrap();
    assert!(bytes.starts_with(b"P6\n"));
    let (back, fmt) = decode_pbm(&bytes).unwrap();
    assert_eq!(fmt, PbmPixelFormat::Rgb48Le);
    assert_eq!(back.planes[0].data, src.planes[0].data);
}

#[test]
fn roundtrip_p7_rgba() {
    let src = pattern(7, 9, 4, PbmPixelFormat::Rgba);
    let bytes = encode_pbm(&src).unwrap();
    assert!(bytes.starts_with(b"P7\n"));
    let header_end = bytes
        .windows(8)
        .position(|w| w == b"ENDHDR\n\x00")
        .unwrap_or(0);
    let _ = header_end;
    let (back, fmt) = decode_pbm(&bytes).unwrap();
    assert_eq!(fmt, PbmPixelFormat::Rgba);
    assert_eq!(back.planes[0].data, src.planes[0].data);
}

#[test]
fn roundtrip_p7_rgba16() {
    let w = 4u32;
    let h = 3u32;
    let mut data = Vec::with_capacity((w * h) as usize * 8);
    for y in 0..h as usize {
        for x in 0..w as usize {
            for c in 0..4 {
                let v = ((x * 1009 + y * 9973 + c * 19) & 0xFFFF) as u16;
                data.extend_from_slice(&v.to_le_bytes());
            }
        }
    }
    let src = PbmImage {
        width: w,
        height: h,
        pixel_format: PbmPixelFormat::Rgba64Le,
        planes: vec![PbmPlane {
            stride: w as usize * 8,
            data,
        }],
        pts: None,
    };
    let bytes = encode_pbm(&src).unwrap();
    assert!(bytes.starts_with(b"P7\n"));
    let (back, fmt) = decode_pbm(&bytes).unwrap();
    assert_eq!(fmt, PbmPixelFormat::Rgba64Le);
    assert_eq!(back.planes[0].data, src.planes[0].data);
}

#[test]
fn roundtrip_p7_ya8() {
    let src = pattern(10, 6, 2, PbmPixelFormat::Ya8);
    let bytes = encode_pbm(&src).unwrap();
    assert!(bytes.starts_with(b"P7\n"));
    let (back, fmt) = decode_pbm(&bytes).unwrap();
    assert_eq!(fmt, PbmPixelFormat::Ya8);
    assert_eq!(back.planes[0].data, src.planes[0].data);
}

#[test]
fn ascii_p3_round_trip() {
    let src = pattern(5, 4, 3, PbmPixelFormat::Rgb24);
    let bytes = encode_pbm_ascii(&src).unwrap();
    assert!(bytes.starts_with(b"P3\n"));
    let (back, fmt) = decode_pbm(&bytes).unwrap();
    assert_eq!(fmt, PbmPixelFormat::Rgb24);
    assert_eq!(back.planes[0].data, src.planes[0].data);
}

#[test]
fn ascii_p2_round_trip() {
    let src = pattern(7, 3, 1, PbmPixelFormat::Gray8);
    let bytes = encode_pbm_ascii(&src).unwrap();
    assert!(bytes.starts_with(b"P2\n"));
    let (back, fmt) = decode_pbm(&bytes).unwrap();
    assert_eq!(fmt, PbmPixelFormat::Gray8);
    assert_eq!(back.planes[0].data, src.planes[0].data);
}

#[test]
fn header_comments_are_tolerated() {
    let buf = b"P3\n# created by GIMP\n2 1\n# maxval next\n255\n255 0 0 0 255 0\n";
    let (image, fmt) = decode_pbm(buf).unwrap();
    assert_eq!(fmt, PbmPixelFormat::Rgb24);
    assert_eq!(image.planes[0].data, [255, 0, 0, 0, 255, 0]);
}

#[test]
fn p2_maxval_1_round_trips() {
    // P2 with MAXVAL 1 is effectively a PBM-as-PGM — a degenerate but
    // legal form. The decoder picks `Gray8` (since maxval <= 255) and
    // the bit-as-byte sample at 0 / 1 is scaled to 0 / 0xFF.
    let buf = b"P2\n3 2\n1\n0 1 0\n1 0 1\n";
    let (image, fmt) = decode_pbm(buf).unwrap();
    assert_eq!(fmt, PbmPixelFormat::Gray8);
    assert_eq!(image.planes[0].data, [0, 0xFF, 0, 0xFF, 0, 0xFF]);
}

#[test]
fn p5_maxval_1_binary_round_trips() {
    // P5 binary with MAXVAL 1 — one byte per sample on disk, each byte
    // is 0 or 1, scaled to 0 / 0xFF.
    let mut buf = Vec::from(b"P5\n4 1\n1\n".as_slice());
    buf.extend_from_slice(&[0, 1, 1, 0]);
    let (image, fmt) = decode_pbm(&buf).unwrap();
    assert_eq!(fmt, PbmPixelFormat::Gray8);
    assert_eq!(image.planes[0].data, [0, 0xFF, 0xFF, 0]);
}

#[test]
fn p3_extra_whitespace_runs_tolerated() {
    // ASCII formats allow ANY ASCII whitespace separator between
    // samples — multiple spaces, tabs, CR, LF, VT, FF. We feed all of
    // them in sequence.
    let body: &[u8] = b"\t  255  \r\n  0\t\t0\n\x0B0\x0C255 0  ";
    let mut buf = Vec::from(b"P3\n2 1\n255\n".as_slice());
    buf.extend_from_slice(body);
    let (image, fmt) = decode_pbm(&buf).unwrap();
    assert_eq!(fmt, PbmPixelFormat::Rgb24);
    assert_eq!(image.planes[0].data, [255, 0, 0, 0, 255, 0]);
}

#[test]
fn p2_comment_between_every_sample() {
    // Most aggressive comment placement: every sample preceded by a
    // comment line.
    let buf = b"P2\n2 2\n255\n# first\n10\n# second\n20\n# third\n30\n# fourth\n40\n";
    let (image, fmt) = decode_pbm(buf).unwrap();
    assert_eq!(fmt, PbmPixelFormat::Gray8);
    assert_eq!(image.planes[0].data, [10, 20, 30, 40]);
}

#[test]
fn p4_blank_padding_after_header_does_not_eat_pixel_byte() {
    // After P4 + width + height, exactly one whitespace byte separates
    // the header from the binary raster. The encoder always emits LF;
    // a CR or space is also legal per the spec. Verify we read the
    // first raster byte correctly regardless of separator.
    for sep in [b'\n', b'\r', b' ', b'\t'] {
        let buf: &[u8] = &[
            b'P', b'4', b'\n', b'8', b' ', b'1', sep, // separator
            0xAB,
        ];
        let (image, fmt) = decode_pbm(buf).unwrap();
        assert_eq!(fmt, PbmPixelFormat::MonoBlack);
        assert_eq!(image.planes[0].data[0], 0xAB);
    }
}

#[test]
fn p7_pam_blank_lines_and_comment_lines_in_header() {
    // PAM header tolerates blank lines and comment lines anywhere
    // before ENDHDR.
    let mut buf = Vec::from(
        b"P7\n\n# generated by something\nWIDTH 1\n\nHEIGHT 1\n# spec test\nDEPTH 4\nMAXVAL 255\nTUPLTYPE RGB_ALPHA\n\n\nENDHDR\n"
            .as_slice(),
    );
    buf.extend_from_slice(&[1, 2, 3, 4]);
    let (image, fmt) = decode_pbm(&buf).unwrap();
    assert_eq!(fmt, PbmPixelFormat::Rgba);
    assert_eq!(image.planes[0].data, [1, 2, 3, 4]);
}

#[test]
fn p7_pam_unknown_header_keys_are_ignored() {
    // The man page says implementations should ignore unknown keys.
    let mut buf = Vec::from(
        b"P7\nWIDTH 1\nHEIGHT 1\nDEPTH 1\nMAXVAL 255\nTUPLTYPE GRAYSCALE\nVENDOR_KEY whatever\nENDHDR\n"
            .as_slice(),
    );
    buf.push(0x7F);
    let (image, fmt) = decode_pbm(&buf).unwrap();
    assert_eq!(fmt, PbmPixelFormat::Gray8);
    assert_eq!(image.planes[0].data, [0x7F]);
}

#[test]
fn p7_pam_crlf_line_endings() {
    // PAM is line-based; CRLF should be tolerated transparently.
    let mut buf = Vec::from(
        b"P7\r\nWIDTH 1\r\nHEIGHT 1\r\nDEPTH 1\r\nMAXVAL 255\r\nTUPLTYPE GRAYSCALE\r\nENDHDR\r\n"
            .as_slice(),
    );
    buf.push(42);
    let (image, fmt) = decode_pbm(&buf).unwrap();
    assert_eq!(fmt, PbmPixelFormat::Gray8);
    assert_eq!(image.planes[0].data, [42]);
}

#[test]
fn explicit_pam7_round_trip_for_rgb24() {
    let src = pattern(5, 3, 3, PbmPixelFormat::Rgb24);
    let bytes = encode_pbm_with_format(&src, PbmEncodeFormat::Pam7).unwrap();
    assert!(bytes.starts_with(b"P7\n"));
    // Quick header sanity: TUPLTYPE RGB, DEPTH 3, MAXVAL 255.
    let endhdr = bytes.windows(7).position(|w| w == b"ENDHDR\n").unwrap();
    let header_str = std::str::from_utf8(&bytes[..endhdr]).unwrap();
    assert!(header_str.contains("DEPTH 3"));
    assert!(header_str.contains("TUPLTYPE RGB"));
    let (back, fmt) = decode_pbm(&bytes).unwrap();
    assert_eq!(fmt, PbmPixelFormat::Rgb24);
    assert_eq!(back.planes[0].data, src.planes[0].data);
}

#[test]
fn explicit_pnm3_ascii_for_rgb24() {
    let src = pattern(3, 2, 3, PbmPixelFormat::Rgb24);
    let bytes = encode_pbm_with_format(&src, PbmEncodeFormat::Pnm3).unwrap();
    assert!(bytes.starts_with(b"P3\n"));
    let (back, fmt) = decode_pbm(&bytes).unwrap();
    assert_eq!(fmt, PbmPixelFormat::Rgb24);
    assert_eq!(back.planes[0].data, src.planes[0].data);
}

#[test]
fn explicit_pnm6_rejects_rgb48() {
    // Pnm6 explicitly forces P6; Rgb48Le is fine for P6 (we accept it).
    // What we want to verify: an unsupported pixel format under Pnm6
    // (e.g. Rgba) produces an Unsupported error rather than silent
    // misencode.
    let img = PbmImage {
        width: 1,
        height: 1,
        pixel_format: PbmPixelFormat::Rgba,
        planes: vec![PbmPlane {
            stride: 4,
            data: vec![1, 2, 3, 4],
        }],
        pts: None,
    };
    assert!(encode_pbm_with_format(&img, PbmEncodeFormat::Pnm6).is_err());
}

#[test]
fn pam_round_trip_blackandwhite() {
    // Encode a P4 file, then surgically rewrite it as a P7 BLACKANDWHITE
    // and confirm the decoder maps the 1-bit data to MonoBlack
    // correctly (handling the PAM "1=white" inversion).
    let w = 8u32;
    let h = 1u32;
    let row_bytes = (w as usize).div_ceil(8);
    let bits = [0b1010_1100u8]; // bits as in P4 (1 = black)
    let mut bw_samples = Vec::new();
    for x in 0..w as usize {
        let bit = (bits[0] >> (7 - x)) & 1;
        // PAM BLACKANDWHITE: 1 = white, so invert.
        bw_samples.push(if bit == 1 { 0u8 } else { 1u8 });
    }
    let mut buf = Vec::from(
        b"P7\nWIDTH 8\nHEIGHT 1\nDEPTH 1\nMAXVAL 1\nTUPLTYPE BLACKANDWHITE\nENDHDR\n".as_slice(),
    );
    buf.extend_from_slice(&bw_samples);
    let _ = (row_bytes, h);

    let (image, fmt) = decode_pbm(&buf).unwrap();
    assert_eq!(fmt, PbmPixelFormat::MonoBlack);
    // After inversion, MonoBlack plane should hold the original P4 bits
    // exactly.
    let plane_byte = image.planes[0].data[0];
    assert_eq!(plane_byte, 0b1010_1100);
}

/// PAM allows arbitrary user-defined `TUPLTYPE` names (the spec is
/// explicit that the standard six are just defaults; producers like
/// scientific/HDR-pipeline tools commonly put custom names there, e.g.
/// `DEPTH_MAP`, `RGBE`, `NORMAL_MAP`, `OPACITY`, multi-channel volumes).
/// We must (a) accept the file rather than reject it, (b) round-trip the
/// name verbatim through the header parser, and (c) route the channels
/// through the depth-fallback layout when the name is non-standard.
#[test]
fn p7_custom_tupltype_depth1_decodes_as_gray() {
    let mut buf = Vec::from(
        b"P7\nWIDTH 4\nHEIGHT 2\nDEPTH 1\nMAXVAL 255\nTUPLTYPE DEPTH_MAP\nENDHDR\n".as_slice(),
    );
    let body: [u8; 8] = [10, 20, 30, 40, 50, 60, 70, 80];
    buf.extend_from_slice(&body);

    let (image, fmt) = decode_pbm(&buf).unwrap();
    assert_eq!(fmt, PbmPixelFormat::Gray8);
    assert_eq!(image.width, 4);
    assert_eq!(image.height, 2);
    assert_eq!(&image.planes[0].data[..8], &body);
}

#[test]
fn p7_custom_tupltype_depth3_decodes_as_rgb() {
    // A 1×1 RGBE-named PAM at depth=3, 8-bit: the channels reach the
    // decoder as plain Rgb24 since `RGBE` isn't one of the six standard
    // names. The producer is signalling "interpret these channels
    // yourself"; our decoder hands them through unchanged.
    let mut buf = Vec::from(
        b"P7\nWIDTH 1\nHEIGHT 1\nDEPTH 3\nMAXVAL 255\nTUPLTYPE RGBE\nENDHDR\n".as_slice(),
    );
    buf.extend_from_slice(&[0xAB, 0xCD, 0xEF]);
    let (image, fmt) = decode_pbm(&buf).unwrap();
    assert_eq!(fmt, PbmPixelFormat::Rgb24);
    assert_eq!(image.planes[0].data[..3], [0xAB, 0xCD, 0xEF]);
}

#[test]
fn p7_custom_tupltype_depth4_decodes_as_rgba() {
    // depth=4 + custom name → falls through to Rgba (the depth-4 entry
    // in the fallback table).
    let mut buf = Vec::from(
        b"P7\nWIDTH 1\nHEIGHT 1\nDEPTH 4\nMAXVAL 255\nTUPLTYPE NORMAL_MAP\nENDHDR\n".as_slice(),
    );
    buf.extend_from_slice(&[0x11, 0x22, 0x33, 0x44]);
    let (image, fmt) = decode_pbm(&buf).unwrap();
    assert_eq!(fmt, PbmPixelFormat::Rgba);
    assert_eq!(image.planes[0].data[..4], [0x11, 0x22, 0x33, 0x44]);
}

#[test]
fn p7_custom_tupltype_depth1_16bit_decodes_as_gray16() {
    // Same as the depth=1 case but maxval > 255 forces 16-bit samples.
    // On-disk samples are big-endian per the PAM spec; the decoder
    // returns Gray16Le in memory.
    let mut buf = Vec::from(
        b"P7\nWIDTH 2\nHEIGHT 1\nDEPTH 1\nMAXVAL 65535\nTUPLTYPE OPACITY\nENDHDR\n".as_slice(),
    );
    // Two big-endian u16 samples: 0x1234, 0xABCD.
    buf.extend_from_slice(&[0x12, 0x34, 0xAB, 0xCD]);
    let (image, fmt) = decode_pbm(&buf).unwrap();
    assert_eq!(fmt, PbmPixelFormat::Gray16Le);
    // In-memory is little-endian.
    assert_eq!(&image.planes[0].data[..4], &[0x34, 0x12, 0xCD, 0xAB]);
}

#[test]
fn p7_custom_tupltype_depth_outside_range_is_rejected() {
    // DEPTH 5 is outside the 1..=4 range the parser accepts. Even with
    // a custom tuple-type name, we must reject — the Custom escape
    // hatch only bypasses the standard-name vs DEPTH consistency check;
    // it doesn't widen the depth range.
    let mut buf = Vec::from(
        b"P7\nWIDTH 1\nHEIGHT 1\nDEPTH 5\nMAXVAL 255\nTUPLTYPE FIVE_CHANNEL\nENDHDR\n".as_slice(),
    );
    buf.extend_from_slice(&[0, 0, 0, 0, 0]);
    assert!(decode_pbm(&buf).is_err());
}
