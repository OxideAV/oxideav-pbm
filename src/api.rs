//! The contract entry points (`IMAGE_CRATE_API`): `probe`, `info`,
//! `decode*`, `encode*`. Thin, documented fronts over the decoder's
//! `decoder::decode_one` / `decoder::decode_stream`
//! and the encoder's `encoder::encode_image`.

use std::io::{Read, Write};

use crate::decoder::{decode_one, decode_stream, is_ascii_ws, native_format};
use crate::encoder::encode_image;
use crate::error::Result;
use crate::header::{parse_header, probe_is_netpbm, Header};
use crate::image::{ColorInfo, Frame, ImageInfo, PbmImage, RgbImage, RgbaImage};
use crate::options::{DecodeOptions, EncodeOptions};

/// `true` when `bytes` plausibly begins a Netpbm or Portable FloatMap
/// file: byte 0 is `P`, byte 1 is one of `1`…`7` / `f` / `F`, and byte
/// 2 (when present) is ASCII whitespace — the separator every grammar
/// in the family mandates after the magic. Allocation-free, never
/// panics, `false` on input shorter than two bytes. A `true` result
/// does not guarantee the header parses; use [`info`] for that.
pub fn probe(bytes: &[u8]) -> bool {
    probe_is_netpbm(bytes)
}

/// Header-only inspection: dimensions, the native [`crate::PixelFormat`]
/// [`decode`] would return, the number of concatenated images, alpha,
/// the documented colour convention, and the parsed [`Header`] of the
/// first image. Decodes no pixels and allocates no pixel buffer.
///
/// `frames` counts the first image plus every well-formed image that
/// follows it back-to-back (binary and float bodies are skipped by
/// their closed-form length; plain-text bodies by a token scan that
/// stores nothing). Counting stops silently at the first malformed
/// trailer, so `info` succeeds whenever the first header does.
pub fn info(bytes: &[u8]) -> Result<ImageInfo> {
    let header = parse_header(bytes)?;
    let format = native_format(&header)?;
    let frames = count_frames(bytes, &header);
    Ok(ImageInfo {
        width: header.width,
        height: header.height,
        format,
        frames,
        has_alpha: format.has_alpha(),
        color: ColorInfo::default_for(format),
        has_icc: false,
        has_exif: false,
        has_xmp: false,
        header,
    })
}

/// Number of concatenated images at the head of `bytes`, given the
/// already-parsed first header. Walks headers and skips bodies without
/// allocating.
fn count_frames(bytes: &[u8], first: &Header) -> u32 {
    let mut frames: u32 = 1;
    let mut offset = match skip_body(bytes, first, 0) {
        Some(o) => o,
        None => return frames,
    };
    loop {
        while offset < bytes.len() && is_ascii_ws(bytes[offset]) {
            offset += 1;
        }
        if offset >= bytes.len() {
            return frames;
        }
        let Ok(header) = parse_header(&bytes[offset..]) else {
            return frames;
        };
        let Some(next) = skip_body(bytes, &header, offset) else {
            return frames;
        };
        if next <= offset {
            return frames;
        }
        frames = frames.saturating_add(1);
        offset = next;
    }
}

/// Offset just past the image whose header (parsed at `start`) is `h`,
/// or `None` when the body is truncated / malformed.
fn skip_body(bytes: &[u8], h: &Header, start: usize) -> Option<usize> {
    let body_start = start.checked_add(h.data_offset)?;
    let body = bytes.get(body_start..)?;
    if let Ok(Some(len)) = h.body_byte_len() {
        if body.len() < len {
            return None;
        }
        return body_start.checked_add(len);
    }
    // Plain-text body: scan exactly width × height × depth sample tokens.
    let total = (h.width as usize)
        .checked_mul(h.height as usize)?
        .checked_mul(h.depth as usize)?;
    let mut cursor = 0usize;
    if matches!(h.magic, crate::header::Magic::P1AsciiBitmap) {
        for _ in 0..total {
            skip_ws_and_comments(body, &mut cursor);
            match body.get(cursor) {
                Some(b'0') | Some(b'1') => cursor += 1,
                _ => return None,
            }
        }
    } else {
        for _ in 0..total {
            crate::header::next_uint(body, &mut cursor).ok()?;
        }
    }
    body_start.checked_add(cursor)
}

fn skip_ws_and_comments(body: &[u8], cursor: &mut usize) {
    while *cursor < body.len() {
        let c = body[*cursor];
        if is_ascii_ws(c) {
            *cursor += 1;
        } else if c == b'#' {
            while *cursor < body.len() && body[*cursor] != b'\n' {
                *cursor += 1;
            }
        } else {
            break;
        }
    }
}

/// Decode the first image in `bytes` into its native layout with
/// `DecodeOptions::default()`. Trailing bytes (further concatenated
/// images) are ignored; see [`decode_all`].
pub fn decode(bytes: &[u8]) -> Result<PbmImage> {
    decode_with(bytes, &DecodeOptions::default())
}

/// [`decode`] under explicit [`DecodeOptions`]: limits are checked
/// against the header before any pixel buffer is allocated
/// (`Error::LimitExceeded`); `strict` rejects samples above `MAXVAL`
/// instead of clamping them.
pub fn decode_with(bytes: &[u8], opts: &DecodeOptions) -> Result<PbmImage> {
    decode_one(bytes, opts).map(|(image, _header, _consumed)| image)
}

/// One-call raw path: the first image as tightly packed 8-bit RGB
/// ([`PbmImage::to_rgb8`]).
pub fn decode_rgb8(bytes: &[u8]) -> Result<RgbImage> {
    let img = decode(bytes)?;
    Ok(RgbImage::new(img.width, img.height, img.to_rgb8()))
}

/// One-call raw path: the first image as tightly packed 8-bit RGBA
/// ([`PbmImage::to_rgba8`]).
pub fn decode_rgba8(bytes: &[u8]) -> Result<RgbaImage> {
    let img = decode(bytes)?;
    Ok(RgbaImage::new(img.width, img.height, img.to_rgba8()))
}

/// Decode every image of a concatenated Netpbm / PAM / PFM file, in
/// stream order, with `DecodeOptions::default()`. The family lets a
/// file carry a **sequence** of self-describing images back-to-back
/// (optionally separated by ASCII whitespace); a single image yields a
/// one-element `Vec`. Each [`Frame`] carries its own [`Header`]
/// (`MAXVAL`, PAM `TUPLTYPE`, PFM byte order + scale); `delay` is
/// always `None`. Every frame is in its own native layout (a mixed
/// stream yields mixed layouts). Trailing non-whitespace that does not
/// begin a valid header is an error.
pub fn decode_all(bytes: &[u8]) -> Result<Vec<Frame>> {
    decode_all_with(bytes, &DecodeOptions::default())
}

/// [`decode_all`] under explicit [`DecodeOptions`] (applied to every
/// image).
pub fn decode_all_with(bytes: &[u8], opts: &DecodeOptions) -> Result<Vec<Frame>> {
    Ok(decode_stream(bytes, opts)?
        .into_iter()
        .map(|(image, header)| Frame::new(image, header))
        .collect())
}

/// Read `r` to end and [`decode`] it. I/O failures surface as
/// `Error::Io`.
pub fn decode_from<R: Read>(mut r: R) -> Result<PbmImage> {
    let mut buf = Vec::new();
    r.read_to_end(&mut buf)?;
    decode(&buf)
}

/// Write `image` as a complete Netpbm / PAM / PFM file under `opts`.
///
/// With `EncodeOptions::default()` the output is the raw, natural
/// flavour for the layout: `MonoBlack` / `MonoWhite` → P4, `Gray8` /
/// `Gray16Le` → P5, `Rgb24` / `Rgb48Le` → P6, every alpha layout → P7
/// (`GRAYSCALE_ALPHA` / `RGB_ALPHA`), `GrayF32Le` / `RgbF32Le` → `Pf` /
/// `PF`. Every layout is written as given — the only re-representations
/// are lossless (`Bgra` reordered to `RGB_ALPHA`, `MonoWhite` bits
/// inverted to the P4 sense). `Error::Unsupported` for what the family
/// cannot represent (alpha or float in plain text, PAM knobs on a float
/// layout, `ascii` + `pam`); `color` and `metadata` cannot be carried
/// and are ignored.
pub fn encode(image: &PbmImage, opts: &EncodeOptions) -> Result<Vec<u8>> {
    encode_image(image, opts)
}

/// One-call raw path: `3 × width × height` RGB bytes → P6 (binary PPM,
/// `MAXVAL 255`) with the default options; `opts` selects another
/// flavour (`ascii` → P3, `pam` → P7 `RGB`, `maxval`). Never a bitmap
/// or a graymap.
pub fn encode_rgb8(width: u32, height: u32, rgb: &[u8], opts: &EncodeOptions) -> Result<Vec<u8>> {
    let img = PbmImage::from_rgb8(width, height, rgb.to_vec())?;
    encode(&img, opts)
}

/// One-call raw path: `4 × width × height` RGBA bytes → P7 PAM
/// `RGB_ALPHA` (`MAXVAL 255`); alpha is kept. `ascii` is
/// `Error::Unsupported` (the plain forms have no alpha).
pub fn encode_rgba8(width: u32, height: u32, rgba: &[u8], opts: &EncodeOptions) -> Result<Vec<u8>> {
    let img = PbmImage::from_rgba8(width, height, rgba.to_vec())?;
    encode(&img, opts)
}

/// [`encode`] straight into a writer.
pub fn encode_to<W: Write>(image: &PbmImage, opts: &EncodeOptions, mut w: W) -> Result<()> {
    let bytes = encode(image, opts)?;
    w.write_all(&bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::{PbmPixelFormat, PixelFormat};

    #[test]
    fn probe_is_total_and_cheap() {
        assert!(probe(b"P6\n"));
        assert!(probe(b"P1"));
        assert!(probe(b"PF\n1 1\n-1.0\n"));
        assert!(!probe(b"P"));
        assert!(!probe(b""));
        assert!(!probe(b"P8\n"));
        assert!(!probe(b"P6x"));
        assert!(!probe(b"\x89PNG"));
    }

    #[test]
    fn info_reads_the_header_only() {
        let i = info(b"P6\n3 2\n255\n").unwrap();
        assert_eq!((i.width, i.height), (3, 2));
        assert_eq!(i.format, PixelFormat::Rgb24);
        assert_eq!(i.frames, 1);
        assert!(!i.has_alpha);
        assert_eq!(i.color, ColorInfo::netpbm_default());
        assert!(!i.has_icc && !i.has_exif && !i.has_xmp);
        assert_eq!(i.maxval(), 255);
        assert!(!i.is_ascii());
        let i = info(b"P7\nWIDTH 1\nHEIGHT 1\nDEPTH 4\nMAXVAL 65535\nTUPLTYPE RGB_ALPHA\nENDHDR\n")
            .unwrap();
        assert_eq!(i.format, PixelFormat::Rgba64Le);
        assert!(i.has_alpha);
        let i = info(b"Pf\n2 2\n-1.0\n").unwrap();
        assert_eq!(i.format, PixelFormat::GrayF32Le);
        assert_eq!(i.color, ColorInfo::pfm_default());
        assert!(info(b"P6\n0 2\n255\n").is_err());
        assert!(info(b"nope").is_err());
    }

    #[test]
    fn info_counts_concatenated_frames_without_decoding() {
        let mut s = Vec::new();
        s.extend_from_slice(b"P5\n2 1\n255\nab");
        s.extend_from_slice(b"\nP2\n2 1\n255\n1 2\n");
        s.extend_from_slice(b"P1\n3 1\n101\n");
        s.extend_from_slice(b"Pf\n1 1\n-1.0\n");
        s.extend_from_slice(&1.0f32.to_le_bytes());
        assert_eq!(info(&s).unwrap().frames, 4);
        // Truncated trailer: counting stops, info still succeeds.
        s.extend_from_slice(b"P6\n9 9\n255\nxx");
        assert_eq!(info(&s).unwrap().frames, 4);
        // Truncated first body still parses as info (header only).
        assert_eq!(info(b"P6\n9 9\n255\n").unwrap().frames, 1);
    }

    #[test]
    fn decode_variants_agree() {
        let bytes = b"P6\n2 1\n255\n\x01\x02\x03\x04\x05\x06";
        let img = decode(bytes).unwrap();
        assert_eq!(img.format, PixelFormat::Rgb24);
        assert_eq!(img.as_bytes().unwrap(), &bytes[11..]);
        let rgb = decode_rgb8(bytes).unwrap();
        assert_eq!((rgb.width, rgb.height), (2, 1));
        assert_eq!(rgb.data, vec![1, 2, 3, 4, 5, 6]);
        let rgba = decode_rgba8(bytes).unwrap();
        assert_eq!(rgba.data, vec![1, 2, 3, 255, 4, 5, 6, 255]);
        let from = decode_from(&bytes[..]).unwrap();
        assert_eq!(from, img);
        let all = decode_all(bytes).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].image, img);
        assert!(all[0].delay.is_none());
        assert_eq!(all[0].header.maxval, 255);
    }

    #[test]
    fn decode_limits_fire_before_allocation() {
        let hostile = b"P6\n60000 60000\n255\n";
        let e = decode(hostile).unwrap_err();
        assert!(e.is_limit_exceeded(), "{e}");
        let e = decode_with(hostile, &DecodeOptions::new().unlimited()).unwrap_err();
        assert!(e.is_invalid_data(), "{e}");
        let small = b"P5\n4 4\n255\n0123456789abcdef";
        assert!(decode_with(small, &DecodeOptions::new().with_max_width(3))
            .unwrap_err()
            .is_limit_exceeded());
        assert!(
            decode_with(small, &DecodeOptions::new().with_max_pixels(15u64))
                .unwrap_err()
                .is_limit_exceeded()
        );
        assert!(
            decode_with(small, &DecodeOptions::new().with_max_bytes(15u64))
                .unwrap_err()
                .is_limit_exceeded()
        );
        assert!(decode_with(small, &DecodeOptions::new().with_max_bytes(16u64)).is_ok());
    }

    #[test]
    fn strict_rejects_over_maxval_samples() {
        let ascii = b"P2\n2 1\n100\n50 200\n";
        let lenient = decode(ascii).unwrap();
        // 200 clamps to 100 → 255 after the maxval rescale.
        assert_eq!(lenient.as_bytes().unwrap(), &[128, 255]);
        let e = decode_with(ascii, &DecodeOptions::new().with_strict(true)).unwrap_err();
        assert!(e.is_invalid_data());
        let binary = b"P5\n2 1\n100\n\x32\xc8";
        assert_eq!(decode(binary).unwrap().as_bytes().unwrap(), &[128, 255]);
        assert!(decode_with(binary, &DecodeOptions::new().with_strict(true)).is_err());
        // Natural maxval: nothing to be strict about.
        assert!(decode_with(
            b"P5\n1 1\n255\n\xff",
            &DecodeOptions::new().with_strict(true)
        )
        .is_ok());
    }

    #[test]
    fn decode_from_surfaces_io_errors() {
        struct Broken;
        impl Read for Broken {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("nope"))
            }
        }
        assert!(decode_from(Broken).unwrap_err().is_io());
    }

    #[test]
    fn encode_rgb8_is_p6_and_rgba8_is_pam_rgb_alpha() {
        let o = EncodeOptions::default();
        let p6 = encode_rgb8(2, 1, &[1, 2, 3, 4, 5, 6], &o).unwrap();
        assert!(p6.starts_with(b"P6\n2 1\n255\n"), "{p6:?}");
        assert_eq!(&p6[11..], &[1, 2, 3, 4, 5, 6]);
        let p7 = encode_rgba8(1, 1, &[1, 2, 3, 4], &o).unwrap();
        assert!(p7.starts_with(
            b"P7\nWIDTH 1\nHEIGHT 1\nDEPTH 4\nMAXVAL 255\nTUPLTYPE RGB_ALPHA\nENDHDR\n"
        ));
        assert!(p7.ends_with(&[1, 2, 3, 4]));
        // Round trips keep the layout.
        assert_eq!(decode(&p6).unwrap().format, PixelFormat::Rgb24);
        assert_eq!(decode(&p7).unwrap().format, PixelFormat::Rgba);
        // Length mismatches are errors, not panics.
        assert!(encode_rgb8(2, 1, &[1, 2, 3], &o)
            .unwrap_err()
            .is_invalid_data());
        assert!(encode_rgba8(0, 1, &[], &o).is_err());
        // The plain forms have no alpha.
        assert!(
            encode_rgba8(1, 1, &[1, 2, 3, 4], &EncodeOptions::new().with_ascii(true))
                .unwrap_err()
                .is_unsupported()
        );
        assert!(
            encode_rgb8(1, 1, &[1, 2, 3], &EncodeOptions::new().with_ascii(true))
                .unwrap()
                .starts_with(b"P3\n")
        );
    }

    #[test]
    fn encode_to_writes_the_same_bytes() {
        let img = PbmImage::from_rgb8(1, 1, vec![9, 8, 7]).unwrap();
        let mut sink = Vec::new();
        encode_to(&img, &EncodeOptions::default(), &mut sink).unwrap();
        assert_eq!(sink, encode(&img, &EncodeOptions::default()).unwrap());
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("full"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        assert!(encode_to(&img, &EncodeOptions::default(), Broken)
            .unwrap_err()
            .is_io());
    }

    #[test]
    fn lossless_round_trip_every_integer_and_float_layout() {
        for f in PbmPixelFormat::ALL {
            let (w, h) = (5u32, 3u32);
            let row = f.row_bytes(w).unwrap();
            let mut data: Vec<u8> = (0..row * h as usize).map(|i| (i * 37 + 11) as u8).collect();
            if f.is_bilevel() {
                // Row padding bits are not image content; the P4 writer
                // clears them, so start from a canonical plane.
                for b in data.iter_mut() {
                    *b &= 0b1111_1000;
                }
            }
            let img = PbmImage::packed(w, h, f, row, data).unwrap();
            let bytes = encode(&img, &EncodeOptions::default()).unwrap();
            let back = decode(&bytes).unwrap();
            match f {
                // Lossless re-representations: the decoder reports the
                // wire layout.
                PbmPixelFormat::MonoWhite => {
                    assert_eq!(back.format, PbmPixelFormat::MonoBlack);
                    assert_eq!(back.to_rgb8(), img.to_rgb8());
                }
                PbmPixelFormat::Bgra => {
                    assert_eq!(back.format, PbmPixelFormat::Rgba);
                    assert_eq!(back.to_rgba8(), img.to_rgba8());
                }
                _ => {
                    assert_eq!(back, img, "{f:?}");
                    assert_eq!(info(&bytes).unwrap().format, f);
                }
            }
        }
    }

    #[test]
    fn encode_flavours() {
        let g = PbmImage::packed(2, 1, PixelFormat::Gray8, 2, vec![0, 255]).unwrap();
        // ASCII P2.
        let p2 = encode(&g, &EncodeOptions::new().with_ascii(true)).unwrap();
        assert_eq!(p2, b"P2\n2 1\n255\n0 255\n");
        // PAM GRAYSCALE.
        let p7 = encode(&g, &EncodeOptions::new().with_pam(true)).unwrap();
        assert!(p7.starts_with(
            b"P7\nWIDTH 2\nHEIGHT 1\nDEPTH 1\nMAXVAL 255\nTUPLTYPE GRAYSCALE\nENDHDR\n"
        ));
        assert_eq!(decode(&p7).unwrap(), g);
        // Custom tupltype implies PAM.
        let custom = encode(
            &g,
            &EncodeOptions::new().with_tupltype("MY_GRAY".to_string()),
        )
        .unwrap();
        assert!(custom.starts_with(b"P7\n"));
        assert!(custom.windows(16).any(|w| w == b"TUPLTYPE MY_GRAY"));
        assert_eq!(decode(&custom).unwrap(), g);
        // Non-natural maxval rescales (255 → 15, 0 → 0).
        let mv = encode(&g, &EncodeOptions::new().with_maxval(15)).unwrap();
        assert_eq!(mv, b"P5\n2 1\n15\n\x00\x0f");
        assert_eq!(decode(&mv).unwrap(), g);
        let mv_ascii = encode(&g, &EncodeOptions::new().with_ascii(true).with_maxval(15)).unwrap();
        assert_eq!(mv_ascii, b"P2\n2 1\n15\n0 15\n");
        // Wide maxval widens to two bytes.
        let wide = encode(&g, &EncodeOptions::new().with_maxval(1000)).unwrap();
        assert_eq!(wide, b"P5\n2 1\n1000\n\x00\x00\x03\xe8");
        assert_eq!(decode(&wide).unwrap().format, PixelFormat::Gray16Le);
        // PAM with custom maxval and an RGB layout.
        let rgb = PbmImage::from_rgb8(1, 1, vec![255, 128, 0]).unwrap();
        let pm = encode(&rgb, &EncodeOptions::new().with_pam(true).with_maxval(3)).unwrap();
        assert!(pm.ends_with(b"MAXVAL 3\nTUPLTYPE RGB\nENDHDR\n\x03\x02\x00"));
        // Bilevel → PAM BLACKANDWHITE (1 = white) and back.
        let m = PbmImage::packed(3, 1, PixelFormat::MonoBlack, 1, vec![0b1010_0000]).unwrap();
        let bw = encode(&m, &EncodeOptions::new().with_pam(true)).unwrap();
        assert!(bw.ends_with(b"MAXVAL 1\nTUPLTYPE BLACKANDWHITE\nENDHDR\n\x00\x01\x00"));
        assert_eq!(decode(&bw).unwrap(), m);
        // MonoWhite → P1 inverted / P4 inverted.
        let mw = PbmImage::packed(3, 1, PixelFormat::MonoWhite, 1, vec![0b1010_0000]).unwrap();
        assert_eq!(
            encode(&mw, &EncodeOptions::new().with_ascii(true)).unwrap(),
            b"P1\n3 1\n0 1 0\n"
        );
        assert_eq!(
            encode(&m, &EncodeOptions::new().with_ascii(true)).unwrap(),
            b"P1\n3 1\n1 0 1\n"
        );
        // Float: byte order + scale knobs; PAM / ascii / maxval refused.
        let f = PbmImage::packed(
            1,
            1,
            PixelFormat::GrayF32Le,
            4,
            1.0f32.to_le_bytes().to_vec(),
        )
        .unwrap();
        let be = encode(
            &f,
            &EncodeOptions::new()
                .with_pfm_little_endian(false)
                .with_pfm_scale(2.0),
        )
        .unwrap();
        assert!(be.starts_with(b"Pf\n1 1\n2.0\n"));
        assert_eq!(&be[be.len() - 4..], &[0x3F, 0x80, 0, 0]);
        assert_eq!(decode(&be).unwrap(), f);
        for bad in [
            EncodeOptions::new().with_ascii(true),
            EncodeOptions::new().with_pam(true),
            EncodeOptions::new().with_maxval(255),
        ] {
            assert!(encode(&f, &bad).unwrap_err().is_unsupported());
        }
        // ascii + pam is contradictory.
        assert!(
            encode(&g, &EncodeOptions::new().with_ascii(true).with_pam(true))
                .unwrap_err()
                .is_unsupported()
        );
    }

    #[test]
    fn encode_rejects_a_mutated_inconsistent_image() {
        let mut img = PbmImage::from_rgb8(2, 2, vec![0; 12]).unwrap();
        img.planes[0].data.truncate(5);
        assert!(encode(&img, &EncodeOptions::default())
            .unwrap_err()
            .is_invalid_data());
    }

    #[test]
    fn decode_all_mixed_stream_and_trailing_garbage() {
        let mut s = Vec::new();
        s.extend_from_slice(b"P5\n2 1\n255\nab");
        s.extend_from_slice(b"\n\nP6\n1 1\n255\nxyz");
        let frames = decode_all(&s).unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].image.format, PixelFormat::Gray8);
        assert_eq!(frames[1].image.format, PixelFormat::Rgb24);
        assert_eq!(frames[1].header.magic, crate::header::Magic::P6BinaryPixmap);
        s.extend_from_slice(b"#junk");
        assert!(decode_all(&s).is_err());
        assert!(decode_all(b"  \n").is_err());
        // Limits apply per image.
        assert!(
            decode_all_with(&s[..s.len() - 5], &DecodeOptions::new().with_max_width(1))
                .unwrap_err()
                .is_limit_exceeded()
        );
    }
}
