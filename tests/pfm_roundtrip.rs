//! End-to-end Portable FloatMap (`Pf` / `PF`) round-trips through the
//! public crate API, including the unified `decode_pbm` dispatch path.

use oxideav_pbm::{
    apply_pfm_scale, decode_pbm, decode_pfm, decode_pfm_scaled, encode_pbm, encode_pfm, PbmImage,
    PbmPixelFormat, PbmPlane,
};

/// Build a float image whose samples encode their coordinates so a row
/// flip or byte swap is observable.
fn float_image(w: u32, h: u32, ch: usize) -> PbmImage {
    let format = if ch == 3 {
        PbmPixelFormat::RgbF32
    } else {
        PbmPixelFormat::GrayF32
    };
    let stride = w as usize * ch * 4;
    let mut data = vec![0u8; stride * h as usize];
    for y in 0..h as usize {
        for x in 0..w as usize {
            for c in 0..ch {
                let v = (y as f32) * 100.0 + (x as f32) * 3.0 + (c as f32) * 0.25 - 7.5;
                let off = y * stride + (x * ch + c) * 4;
                data[off..off + 4].copy_from_slice(&v.to_le_bytes());
            }
        }
    }
    PbmImage {
        width: w,
        height: h,
        pixel_format: format,
        planes: vec![PbmPlane { stride, data }],
        pts: None,
    }
}

#[test]
fn pf_little_endian_roundtrip_via_decode_pbm() {
    let img = float_image(7, 5, 1);
    let bytes = encode_pfm(&img, true, 1.0).unwrap();
    assert!(bytes.starts_with(b"Pf\n7 5\n-1.0\n"));
    // The unified decode entry point dispatches PFM to the float decoder.
    let (back, fmt) = decode_pbm(&bytes).unwrap();
    assert_eq!(fmt, PbmPixelFormat::GrayF32);
    assert_eq!(back.planes[0].data, img.planes[0].data);
}

#[test]
fn pf_big_endian_roundtrip() {
    let img = float_image(4, 6, 1);
    let bytes = encode_pfm(&img, false, 1.0).unwrap();
    assert!(bytes.starts_with(b"Pf\n4 6\n1.0\n"));
    let (back, info) = decode_pfm(&bytes).unwrap();
    assert!(!info.little_endian);
    assert_eq!(back.planes[0].data, img.planes[0].data);
}

#[test]
fn pf_capital_rgb_both_endiannesses_match() {
    let img = float_image(5, 4, 3);
    let le = encode_pfm(&img, true, 1.0).unwrap();
    let be = encode_pfm(&img, false, 1.0).unwrap();
    assert!(le.starts_with(b"PF\n5 4\n-1.0\n"));
    assert!(be.starts_with(b"PF\n5 4\n1.0\n"));
    // Same pixels survive both byte orders identically.
    let (from_le, _) = decode_pfm(&le).unwrap();
    let (from_be, _) = decode_pfm(&be).unwrap();
    assert_eq!(from_le.planes[0].data, img.planes[0].data);
    assert_eq!(from_be.planes[0].data, img.planes[0].data);
}

#[test]
fn scale_factor_roundtrips_as_metadata() {
    let img = float_image(2, 2, 3);
    let bytes = encode_pfm(&img, false, 4.0).unwrap();
    assert!(bytes.starts_with(b"PF\n2 2\n4.0\n"));
    let (_back, info) = decode_pfm(&bytes).unwrap();
    assert_eq!(info.scale, 4.0);
    assert!(!info.little_endian);
}

#[test]
fn encode_pbm_auto_selects_pfm_for_float_formats() {
    let gray = float_image(3, 3, 1);
    let rgb = float_image(3, 3, 3);
    assert!(encode_pbm(&gray).unwrap().starts_with(b"Pf\n"));
    assert!(encode_pbm(&rgb).unwrap().starts_with(b"PF\n"));
}

#[test]
fn decode_pfm_scaled_folds_header_factor_into_samples() {
    // The Debevec reference describes the scale-line magnitude as a
    // factor an application may apply to the samples. The opt-in
    // `decode_pfm_scaled` performs that multiply; plain `decode_pfm`
    // leaves the raw samples and reports the factor as metadata.
    let img = float_image(6, 4, 3);
    let bytes = encode_pfm(&img, true, 5.0).unwrap();

    let (raw, raw_info) = decode_pfm(&bytes).unwrap();
    let (scaled, scaled_info) = decode_pfm_scaled(&bytes).unwrap();

    // Both report the same advisory factor; only `scaled` applied it.
    assert_eq!(raw_info.scale, 5.0);
    assert_eq!(scaled_info.scale, 5.0);

    for (rc, sc) in raw.planes[0]
        .data
        .chunks_exact(4)
        .zip(scaled.planes[0].data.chunks_exact(4))
    {
        let r = f32::from_le_bytes([rc[0], rc[1], rc[2], rc[3]]);
        let s = f32::from_le_bytes([sc[0], sc[1], sc[2], sc[3]]);
        assert_eq!(s, r * 5.0);
    }

    // Applying the factor by hand to the raw decode matches the wrapper.
    let mut by_hand = raw;
    apply_pfm_scale(&mut by_hand, scaled_info.scale).unwrap();
    assert_eq!(by_hand.planes[0].data, scaled.planes[0].data);
}

#[test]
fn decode_pbm_rejects_crlf_pfm_header() {
    let buf = b"PF\r\n2 2\r\n-1.0\r\n";
    assert!(decode_pbm(buf).is_err());
}

#[test]
fn decode_pbm_rejects_commented_pfm_header() {
    let buf = b"Pf\n# nope\n2 2\n-1.0\n";
    assert!(decode_pbm(buf).is_err());
}
