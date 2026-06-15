//! Multi-image (concatenated) Netpbm/PFM stream decoding.
//!
//! The PNM/PAM/PFM family permits a single file to carry a sequence of
//! self-describing images packed back-to-back. `decode_pbm_multi` walks
//! every image; `decode_pbm` returns just the first. These tests build
//! streams by concatenating the bytes that `encode_pbm` / `encode_pfm`
//! emit for individual images and assert that the round-trip recovers
//! every image's pixels exactly, regardless of the per-image magic.

use oxideav_pbm::{
    decode_pbm, decode_pbm_consumed, decode_pbm_header_consumed, decode_pbm_multi,
    decode_pbm_multi_with_headers, encode_pbm, encode_pbm_ascii, encode_pfm, Magic, PbmImage,
    PbmPixelFormat, PbmPlane, Tupltype,
};

fn gray8(w: u32, h: u32, seed: u8) -> PbmImage {
    let mut data = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            data.push(((x.wrapping_mul(3) + y.wrapping_mul(5)) as u8).wrapping_add(seed));
        }
    }
    PbmImage {
        width: w,
        height: h,
        pixel_format: PbmPixelFormat::Gray8,
        planes: vec![PbmPlane {
            stride: w as usize,
            data,
        }],
        pts: None,
    }
}

fn rgb24(w: u32, h: u32, seed: u8) -> PbmImage {
    let mut data = Vec::with_capacity((w * h * 3) as usize);
    for y in 0..h {
        for x in 0..w {
            data.push((x as u8).wrapping_add(seed));
            data.push((y as u8).wrapping_add(seed));
            data.push((x as u8 ^ y as u8).wrapping_add(seed));
        }
    }
    PbmImage {
        width: w,
        height: h,
        pixel_format: PbmPixelFormat::Rgb24,
        planes: vec![PbmPlane {
            stride: w as usize * 3,
            data,
        }],
        pts: None,
    }
}

fn gray16(w: u32, h: u32, seed: u16) -> PbmImage {
    let mut data = Vec::with_capacity((w * h * 2) as usize);
    for y in 0..h {
        for x in 0..w {
            let v = ((x as u16) << 8 | (y as u16)).wrapping_add(seed);
            data.extend_from_slice(&v.to_le_bytes());
        }
    }
    PbmImage {
        width: w,
        height: h,
        pixel_format: PbmPixelFormat::Gray16Le,
        planes: vec![PbmPlane {
            stride: w as usize * 2,
            data,
        }],
        pts: None,
    }
}

fn grayf32(w: u32, h: u32, seed: f32) -> PbmImage {
    let mut data = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let v = x as f32 * 0.25 + y as f32 + seed;
            data.extend_from_slice(&v.to_le_bytes());
        }
    }
    PbmImage {
        width: w,
        height: h,
        pixel_format: PbmPixelFormat::GrayF32,
        planes: vec![PbmPlane {
            stride: w as usize * 4,
            data,
        }],
        pts: None,
    }
}

#[test]
fn single_image_yields_one() {
    let src = rgb24(8, 6, 0);
    let bytes = encode_pbm(&src).unwrap();
    let imgs = decode_pbm_multi(&bytes).unwrap();
    assert_eq!(imgs.len(), 1);
    assert_eq!(imgs[0].1, PbmPixelFormat::Rgb24);
    assert_eq!(imgs[0].0.planes[0].data, src.planes[0].data);
}

#[test]
fn two_binary_p6_images() {
    let a = rgb24(8, 6, 1);
    let b = rgb24(5, 9, 200);
    let mut stream = encode_pbm(&a).unwrap();
    stream.extend_from_slice(&encode_pbm(&b).unwrap());

    let imgs = decode_pbm_multi(&stream).unwrap();
    assert_eq!(imgs.len(), 2);
    assert_eq!((imgs[0].0.width, imgs[0].0.height), (8, 6));
    assert_eq!(imgs[0].0.planes[0].data, a.planes[0].data);
    assert_eq!((imgs[1].0.width, imgs[1].0.height), (5, 9));
    assert_eq!(imgs[1].0.planes[0].data, b.planes[0].data);
}

#[test]
fn mixed_magics_back_to_back() {
    // P5 (gray8) + P6 (rgb24) + P5 (gray16, 16-bit BE on disk).
    let a = gray8(7, 4, 10);
    let b = rgb24(6, 3, 20);
    let c = gray16(5, 5, 0x1234);
    let mut stream = encode_pbm(&a).unwrap();
    stream.extend_from_slice(&encode_pbm(&b).unwrap());
    stream.extend_from_slice(&encode_pbm(&c).unwrap());

    let imgs = decode_pbm_multi(&stream).unwrap();
    assert_eq!(imgs.len(), 3);
    assert_eq!(imgs[0].1, PbmPixelFormat::Gray8);
    assert_eq!(imgs[0].0.planes[0].data, a.planes[0].data);
    assert_eq!(imgs[1].1, PbmPixelFormat::Rgb24);
    assert_eq!(imgs[1].0.planes[0].data, b.planes[0].data);
    assert_eq!(imgs[2].1, PbmPixelFormat::Gray16Le);
    assert_eq!(imgs[2].0.planes[0].data, c.planes[0].data);
}

#[test]
fn ascii_images_in_stream() {
    // ASCII bodies have no closed-form length; the tokenizer's consumed
    // cursor must land exactly on the next magic. P2 then P3.
    let a = gray8(6, 4, 30);
    let b = rgb24(4, 4, 40);
    let mut stream = encode_pbm_ascii(&a).unwrap();
    assert!(stream.starts_with(b"P2\n"));
    stream.extend_from_slice(&encode_pbm_ascii(&b).unwrap());

    let imgs = decode_pbm_multi(&stream).unwrap();
    assert_eq!(imgs.len(), 2);
    assert_eq!(imgs[0].0.planes[0].data, a.planes[0].data);
    assert_eq!(imgs[1].0.planes[0].data, b.planes[0].data);
}

#[test]
fn ascii_then_binary_then_ascii() {
    // The most adversarial mix for the consumed-byte arithmetic: an
    // ASCII image (cursor-based length) followed by a binary image
    // (deterministic length) followed by another ASCII image.
    let a = gray8(5, 3, 1);
    let b = rgb24(7, 2, 2);
    let c = gray8(3, 6, 3);
    let mut stream = encode_pbm_ascii(&a).unwrap();
    stream.extend_from_slice(&encode_pbm(&b).unwrap());
    stream.extend_from_slice(&encode_pbm_ascii(&c).unwrap());

    let imgs = decode_pbm_multi(&stream).unwrap();
    assert_eq!(imgs.len(), 3);
    assert_eq!(imgs[0].0.planes[0].data, a.planes[0].data);
    assert_eq!(imgs[1].0.planes[0].data, b.planes[0].data);
    assert_eq!(imgs[2].0.planes[0].data, c.planes[0].data);
}

#[test]
fn pfm_images_in_stream() {
    // PFM bodies are width*height*channels*4; two PFM images back to
    // back must decode independently with their bottom-to-top flip.
    let a = grayf32(5, 4, 1.0);
    let b = grayf32(3, 7, 100.0);
    let mut stream = encode_pfm(&a, true, 1.0).unwrap();
    stream.extend_from_slice(&encode_pfm(&b, true, 1.0).unwrap());

    let imgs = decode_pbm_multi(&stream).unwrap();
    assert_eq!(imgs.len(), 2);
    assert_eq!(imgs[0].1, PbmPixelFormat::GrayF32);
    assert_eq!(imgs[0].0.planes[0].data, a.planes[0].data);
    assert_eq!(imgs[1].1, PbmPixelFormat::GrayF32);
    assert_eq!(imgs[1].0.planes[0].data, b.planes[0].data);
}

#[test]
fn trailing_whitespace_after_last_image_ok() {
    let a = gray8(4, 4, 7);
    let mut stream = encode_pbm(&a).unwrap();
    stream.extend_from_slice(b"\n\n  \t\n");
    let imgs = decode_pbm_multi(&stream).unwrap();
    assert_eq!(imgs.len(), 1);
    assert_eq!(imgs[0].0.planes[0].data, a.planes[0].data);
}

#[test]
fn comment_between_images_is_rejected() {
    // The magic must be the first two bytes of each image (the PNM/PAM
    // grammar places the magic before any comment), so a `#` line
    // between images is NOT a valid separator — it leaves the stream
    // pointing at a `#` where a magic is required. The decoder reports
    // a malformed stream rather than silently swallowing the comment.
    let a = gray8(4, 4, 7);
    let b = rgb24(3, 3, 9);
    let mut stream = encode_pbm(&a).unwrap();
    stream.extend_from_slice(b"\n# separator comment\n");
    stream.extend_from_slice(&encode_pbm(&b).unwrap());

    assert!(decode_pbm_multi(&stream).is_err());
}

#[test]
fn decode_pbm_returns_only_first_of_stream() {
    // decode_pbm ignores trailing concatenated images.
    let a = rgb24(8, 6, 1);
    let b = rgb24(5, 9, 200);
    let mut stream = encode_pbm(&a).unwrap();
    stream.extend_from_slice(&encode_pbm(&b).unwrap());

    let (img, fmt) = decode_pbm(&stream).unwrap();
    assert_eq!(fmt, PbmPixelFormat::Rgb24);
    assert_eq!((img.width, img.height), (8, 6));
    assert_eq!(img.planes[0].data, a.planes[0].data);
}

#[test]
fn consumed_length_equals_first_image_size() {
    let a = rgb24(8, 6, 1);
    let first = encode_pbm(&a).unwrap();
    let mut stream = first.clone();
    stream.extend_from_slice(&encode_pbm(&rgb24(5, 9, 2)).unwrap());

    let (_img, _fmt, consumed) = decode_pbm_consumed(&stream).unwrap();
    assert_eq!(consumed, first.len());
}

#[test]
fn garbage_after_image_is_an_error() {
    // Non-whitespace, non-header trailing bytes are a malformed stream.
    let a = gray8(4, 4, 7);
    let mut stream = encode_pbm(&a).unwrap();
    stream.extend_from_slice(b"\nNOTAMAGIC garbage");
    assert!(decode_pbm_multi(&stream).is_err());
}

#[test]
fn empty_input_is_an_error() {
    assert!(decode_pbm_multi(b"").is_err());
    assert!(decode_pbm_multi(b"   \n\t  ").is_err());
}

fn rgba8(w: u32, h: u32, seed: u8) -> PbmImage {
    let mut data = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            data.push((x as u8).wrapping_add(seed));
            data.push((y as u8).wrapping_add(seed));
            data.push((x as u8 ^ y as u8).wrapping_add(seed));
            data.push(if (x + y) & 1 == 0 { 255 } else { 64 });
        }
    }
    PbmImage {
        width: w,
        height: h,
        pixel_format: PbmPixelFormat::Rgba,
        planes: vec![PbmPlane {
            stride: w as usize * 4,
            data,
        }],
        pts: None,
    }
}

#[test]
fn header_consumed_carries_maxval_and_offset() {
    // 16-bit P5 carries maxval 65535; the consumed entry must surface the
    // parsed header (magic, maxval, depth) and the same byte count as
    // decode_pbm_consumed.
    let a = gray16(5, 5, 0x1234);
    let bytes = encode_pbm(&a).unwrap();

    let (img, fmt, header, consumed) = decode_pbm_header_consumed(&bytes).unwrap();
    assert_eq!(fmt, PbmPixelFormat::Gray16Le);
    assert_eq!(header.magic, Magic::P5BinaryGraymap);
    assert_eq!(header.maxval, 65535);
    assert_eq!(header.depth, 1);
    assert!(header.pfm.is_none());
    assert_eq!(img.planes[0].data, a.planes[0].data);

    // The discarding entry must agree on the byte count.
    let (_i, _f, c) = decode_pbm_consumed(&bytes).unwrap();
    assert_eq!(consumed, c);
    assert_eq!(consumed, bytes.len());
    // data_offset points past the header, into the body.
    assert!(header.data_offset > 0 && header.data_offset < bytes.len());
}

#[test]
fn header_consumed_recovers_pam_tupltype() {
    // A P7 RGBA image round-trips its RGB_ALPHA tupltype through the
    // header-carrying decode — the metadata a plain decode discards.
    let a = rgba8(6, 4, 5);
    let bytes = encode_pbm(&a).unwrap();
    assert!(bytes.starts_with(b"P7\n"));

    let (img, fmt, header, _consumed) = decode_pbm_header_consumed(&bytes).unwrap();
    assert_eq!(fmt, PbmPixelFormat::Rgba);
    assert_eq!(header.magic, Magic::P7Pam);
    assert_eq!(header.depth, 4);
    assert_eq!(header.tupltype, Some(Tupltype::RgbAlpha));
    assert_eq!(img.planes[0].data, a.planes[0].data);
}

#[test]
fn header_consumed_carries_pfm_byte_order_and_scale() {
    // The integer header-carrying entry also routes PFM, surfacing the
    // byte order + scale via Header::pfm (mirroring decode_pfm_consumed).
    let a = grayf32(4, 3, 2.0);
    let bytes = encode_pfm(&a, true, 1.0).unwrap();

    let (img, fmt, header, consumed) = decode_pbm_header_consumed(&bytes).unwrap();
    assert_eq!(fmt, PbmPixelFormat::GrayF32);
    assert_eq!(header.magic, Magic::PfPfmGrayFloat);
    let pfm = header
        .pfm
        .expect("PFM header must carry byte order + scale");
    assert!(pfm.little_endian);
    assert_eq!(pfm.scale, 1.0);
    assert_eq!(consumed, bytes.len());
    assert_eq!(img.planes[0].data, a.planes[0].data);
}

#[test]
fn multi_with_headers_keeps_every_header() {
    // A mixed stream: P5 gray8 (maxval 255), P7 RGBA (RGB_ALPHA tupltype),
    // PFM gray (byte order + scale). Each image's header is recovered.
    let a = gray8(7, 4, 10);
    let b = rgba8(6, 3, 20);
    let c = grayf32(5, 5, 1.0);
    let mut stream = encode_pbm(&a).unwrap();
    stream.extend_from_slice(&encode_pbm(&b).unwrap());
    stream.extend_from_slice(&encode_pfm(&c, false, 2.5).unwrap());

    let imgs = decode_pbm_multi_with_headers(&stream).unwrap();
    assert_eq!(imgs.len(), 3);

    assert_eq!(imgs[0].1, PbmPixelFormat::Gray8);
    assert_eq!(imgs[0].2.magic, Magic::P5BinaryGraymap);
    assert_eq!(imgs[0].2.maxval, 255);
    assert_eq!(imgs[0].0.planes[0].data, a.planes[0].data);

    assert_eq!(imgs[1].1, PbmPixelFormat::Rgba);
    assert_eq!(imgs[1].2.magic, Magic::P7Pam);
    assert_eq!(imgs[1].2.tupltype, Some(Tupltype::RgbAlpha));
    assert_eq!(imgs[1].0.planes[0].data, b.planes[0].data);

    assert_eq!(imgs[2].1, PbmPixelFormat::GrayF32);
    assert_eq!(imgs[2].2.magic, Magic::PfPfmGrayFloat);
    let pfm = imgs[2].2.pfm.expect("PFM header");
    assert!(!pfm.little_endian); // encoded big-endian
    assert_eq!(pfm.scale, 2.5);
    assert_eq!(imgs[2].0.planes[0].data, c.planes[0].data);
}

#[test]
fn multi_with_headers_agrees_with_multi_on_pixels() {
    // The header-carrying walker must decode the exact same pixels and
    // image count as the lean walker.
    let a = rgb24(8, 6, 1);
    let b = gray16(5, 9, 0x0203);
    let mut stream = encode_pbm(&a).unwrap();
    stream.extend_from_slice(&encode_pbm(&b).unwrap());

    let lean = decode_pbm_multi(&stream).unwrap();
    let rich = decode_pbm_multi_with_headers(&stream).unwrap();
    assert_eq!(lean.len(), rich.len());
    for (l, r) in lean.iter().zip(rich.iter()) {
        assert_eq!(l.1, r.1);
        assert_eq!(l.0.planes[0].data, r.0.planes[0].data);
    }
}
