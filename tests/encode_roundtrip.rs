//! End-to-end encode → decode roundtrip across every supported
//! Netpbm output format.

use oxideav_pbm::{decode_pbm, encode_pbm, encode_pbm_ascii, PbmImage, PbmPixelFormat, PbmPlane};

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
