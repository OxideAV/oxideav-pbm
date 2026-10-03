//! The `IMAGE_CRATE_API` contract, exercised through the crate root
//! exactly as an external consumer sees it: the one-screen example,
//! every root item's shape, the lossless `decode(encode(img)) == img`
//! pin per layout, the `encode_rgb8` → P6 / `encode_rgba8` → P7
//! `RGB_ALPHA` pins (never a thresholded bitmap), the deprecated
//! wrappers, and — with the `registry` feature — the framework adapter
//! delegating to the same functions.

use oxideav_pbm::{
    decode, decode_all, decode_from, decode_rgb8, decode_rgba8, decode_with, encode, encode_rgb8,
    encode_rgba8, encode_to, info, probe, ColorInfo, ColorRange, DecodeOptions, EncodeOptions,
    Error, Frame, ImageInfo, Metadata, PbmImage, PixelFormat, Plane, RgbImage, RgbaImage,
};

fn checker(w: u32, h: u32) -> Vec<u8> {
    let mut data = Vec::with_capacity((w * h * 3) as usize);
    for y in 0..h {
        for x in 0..w {
            let q = ((x & 1) + 2 * (y & 1)) as usize;
            data.extend_from_slice(&[[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 255]][q]);
        }
    }
    data
}

#[test]
fn one_screen_example() -> Result<(), Error> {
    let bytes = encode_rgb8(4, 2, &checker(4, 2), &EncodeOptions::default())?;
    assert!(probe(&bytes));
    let info: ImageInfo = info(&bytes)?;
    assert_eq!((info.width, info.height, info.frames), (4, 2, 1));
    assert_eq!(info.format, PixelFormat::Rgb24);
    let img: PbmImage = decode(&bytes)?;
    let rgba: Vec<u8> = img.to_rgba8();
    assert_eq!(rgba.len(), 4 * 2 * 4);
    let (w, h) = (img.width(), img.height());
    let opts = EncodeOptions::default().with_ascii(true);
    let out = encode(&img, &opts)?;
    assert!(out.starts_with(b"P3\n4 2\n255\n"));
    let again = encode_rgba8(w, h, &rgba, &EncodeOptions::default())?;
    assert!(again.starts_with(b"P7\n"));
    Ok(())
}

#[test]
fn root_types_have_the_contract_shape() {
    let plane = Plane::new(6, vec![1, 2, 3, 4, 5, 6]);
    let img = PbmImage::new(2, 1, PixelFormat::Rgb24, vec![plane])
        .unwrap()
        .with_color(ColorInfo::srgb())
        .with_metadata(Metadata::new().with_gamma(0.45455));
    assert_eq!(img.width, 2);
    assert_eq!(img.height, 1);
    assert_eq!(img.format, PixelFormat::Rgb24);
    assert_eq!(img.planes.len(), 1);
    assert_eq!(img.planes[0].stride, 6);
    assert_eq!(img.color.range, ColorRange::Full);
    assert_eq!(img.color.primaries, 1);
    assert_eq!(img.color.transfer, 13);
    assert_eq!(img.color.matrix, 0);
    assert_eq!(img.metadata.gamma, Some(0.45455));
    assert!(
        img.metadata.icc.is_none() && img.metadata.exif.is_none() && img.metadata.xmp.is_none()
    );
    assert_eq!(img.as_bytes(), Some(&[1u8, 2, 3, 4, 5, 6][..]));
    assert_eq!(img.to_rgb8(), vec![1, 2, 3, 4, 5, 6]);
    assert_eq!(img.to_rgba8(), vec![1, 2, 3, 255, 4, 5, 6, 255]);
    assert_eq!(img.clone().into_raw(), vec![1, 2, 3, 4, 5, 6]);

    let rgb: RgbImage = RgbImage::new(1, 1, vec![1, 2, 3]);
    assert_eq!(rgb.as_bytes(), &[1, 2, 3]);
    assert_eq!(rgb.into_raw(), vec![1, 2, 3]);
    let rgba: RgbaImage = RgbaImage::new(1, 1, vec![1, 2, 3, 4]);
    assert_eq!(rgba.data.len(), 4);

    let d = DecodeOptions::default();
    assert_eq!(d.max_width, None);
    assert_eq!(d.max_height, None);
    assert_eq!(d.max_pixels, None);
    assert_eq!(d.max_bytes, Some(1 << 30));
    assert!(!d.strict);

    let e = EncodeOptions::default();
    assert!(!e.ascii && !e.pam && e.maxval.is_none() && e.tupltype.is_none());
    assert!(e.pfm_little_endian);
    assert_eq!(e.pfm_scale, 1.0);

    // Error: the four contract variants, Io carries std::io::Error.
    let io: Error = std::io::Error::other("x").into();
    assert!(matches!(io, Error::Io(_)));
    assert!(matches!(Error::invalid("a"), Error::InvalidData(_)));
    assert!(matches!(Error::unsupported("a"), Error::Unsupported(_)));
    assert!(matches!(Error::limit("a"), Error::LimitExceeded(_)));
    fn assert_error<E: std::error::Error + Send + Sync + 'static>() {}
    assert_error::<Error>();
}

#[test]
fn constructors_are_fallible() {
    assert!(matches!(
        PbmImage::from_rgb8(2, 2, vec![0; 11]),
        Err(Error::InvalidData(_))
    ));
    assert!(matches!(
        PbmImage::from_rgba8(2, 2, vec![0; 15]),
        Err(Error::InvalidData(_))
    ));
    assert!(matches!(
        PbmImage::new(0, 1, PixelFormat::Gray8, vec![Plane::new(0, vec![])]),
        Err(Error::InvalidData(_))
    ));
    assert!(matches!(
        PbmImage::packed(2, 1, PixelFormat::Gray8, 1, vec![0, 0]),
        Err(Error::InvalidData(_))
    ));
    assert!(PbmImage::from_rgb8(2, 2, vec![0; 12]).is_ok());
}

#[test]
fn raw_paths_pin_their_magic() {
    // r463 lesson: a PPM must never come out as a thresholded bitmap.
    let p6 = encode_rgb8(
        3,
        1,
        &[0, 0, 0, 128, 128, 128, 255, 255, 255],
        &EncodeOptions::default(),
    )
    .unwrap();
    assert!(p6.starts_with(b"P6\n3 1\n255\n"));
    assert_eq!(&p6[11..], &[0, 0, 0, 128, 128, 128, 255, 255, 255]);
    assert_eq!(decode(&p6).unwrap().format, PixelFormat::Rgb24);
    let p7 = encode_rgba8(
        1,
        2,
        &[0, 0, 0, 0, 255, 255, 255, 255],
        &EncodeOptions::default(),
    )
    .unwrap();
    assert!(
        p7.starts_with(b"P7\nWIDTH 1\nHEIGHT 2\nDEPTH 4\nMAXVAL 255\nTUPLTYPE RGB_ALPHA\nENDHDR\n")
    );
    assert_eq!(decode(&p7).unwrap().format, PixelFormat::Rgba);
    assert_eq!(
        decode_rgba8(&p7).unwrap().data,
        vec![0, 0, 0, 0, 255, 255, 255, 255]
    );
    assert_eq!(decode_rgb8(&p7).unwrap().data, vec![0, 0, 0, 255, 255, 255]);
}

fn sample_image(f: PixelFormat, w: u32, h: u32) -> PbmImage {
    let row = f.row_bytes(w).unwrap();
    let mut data: Vec<u8> = (0..row * h as usize).map(|i| (i * 53 + 7) as u8).collect();
    if f.is_bilevel() {
        // Padding bits past the last pixel are not content.
        let pad = row * 8 - w as usize;
        let mask = (0xFFu16 << pad) as u8;
        for r in data.chunks_exact_mut(row) {
            r[row - 1] &= mask;
        }
    }
    PbmImage::packed(w, h, f, row, data).unwrap()
}

#[test]
fn lossless_round_trip_every_layout_and_flavour() {
    for f in PixelFormat::ALL {
        let img = sample_image(f, 7, 3);
        let bytes = encode(&img, &EncodeOptions::default()).unwrap();
        let back = decode(&bytes).unwrap();
        // The decoder reports the wire layout; `MonoWhite` / `Bgra` are
        // lossless re-representations.
        let expect_fmt = match f {
            PixelFormat::MonoWhite => PixelFormat::MonoBlack,
            PixelFormat::Bgra => PixelFormat::Rgba,
            other => other,
        };
        assert_eq!(back.format, expect_fmt, "{f:?}");
        assert_eq!(back.to_rgba8(), img.to_rgba8(), "{f:?}");
        if expect_fmt == f {
            assert_eq!(back, img, "{f:?}");
        }
        // The same through the writer variant and the reader variant.
        let mut sink = Vec::new();
        encode_to(&img, &EncodeOptions::default(), &mut sink).unwrap();
        assert_eq!(sink, bytes);
        assert_eq!(decode_from(&bytes[..]).unwrap(), back);
        // Every integer layout also survives the PAM container.
        if !f.is_float() {
            let pam = encode(&img, &EncodeOptions::new().with_pam(true)).unwrap();
            assert!(pam.starts_with(b"P7\n"));
            assert_eq!(
                decode(&pam).unwrap().to_rgba8(),
                img.to_rgba8(),
                "{f:?} pam"
            );
            // ... and a 16-bit MAXVAL re-quantisation of an 8-bit layout
            // is exact in the 8-bit sense.
            if f.bits_per_channel() == 8 {
                let wide = encode(&img, &EncodeOptions::new().with_maxval(65535)).unwrap();
                assert_eq!(
                    decode(&wide).unwrap().to_rgba8(),
                    img.to_rgba8(),
                    "{f:?} wide"
                );
            }
        }
        // Plain text for the layouts that have it.
        if !f.has_alpha() && !f.is_float() {
            let plain = encode(&img, &EncodeOptions::new().with_ascii(true)).unwrap();
            assert!(plain[1].is_ascii_digit() && plain[1] <= b'3', "{f:?}");
            assert_eq!(
                decode(&plain).unwrap().to_rgba8(),
                img.to_rgba8(),
                "{f:?} ascii"
            );
        } else {
            assert!(matches!(
                encode(&img, &EncodeOptions::new().with_ascii(true)),
                Err(Error::Unsupported(_))
            ));
        }
        assert_eq!(info(&bytes).unwrap().format, expect_fmt);
    }
}

#[test]
fn decode_all_walks_concatenated_images() {
    let a = encode(
        &sample_image(PixelFormat::Gray8, 3, 2),
        &EncodeOptions::default(),
    )
    .unwrap();
    let b = encode(
        &sample_image(PixelFormat::Rgb24, 2, 2),
        &EncodeOptions::new().with_ascii(true),
    )
    .unwrap();
    let c = encode(
        &sample_image(PixelFormat::RgbF32Le, 1, 1),
        &EncodeOptions::default(),
    )
    .unwrap();
    let mut stream = a.clone();
    stream.extend_from_slice(b"\n");
    stream.extend_from_slice(&b);
    stream.extend_from_slice(&c);
    assert_eq!(info(&stream).unwrap().frames, 3);
    let frames: Vec<Frame> = decode_all(&stream).unwrap();
    assert_eq!(frames.len(), 3);
    assert_eq!(frames[0].image.format, PixelFormat::Gray8);
    assert_eq!(frames[1].image.format, PixelFormat::Rgb24);
    assert_eq!(frames[2].image.format, PixelFormat::RgbF32Le);
    assert!(frames.iter().all(|f| f.delay.is_none()));
    assert_eq!(frames[1].header.maxval, 255);
    assert!(frames[2].header.pfm.is_some());
    // `decode` is the first image only.
    assert_eq!(decode(&stream).unwrap(), frames[0].image);
}

#[test]
fn limits_and_strictness() {
    let big = b"P6\n50000 50000\n255\n";
    assert!(matches!(decode(big), Err(Error::LimitExceeded(_))));
    assert!(matches!(
        decode_with(big, &DecodeOptions::new().unlimited()),
        Err(Error::InvalidData(_))
    ));
    let over = b"P3\n1 1\n10\n11 0 0\n";
    assert_eq!(decode(over).unwrap().to_rgb8(), vec![255, 0, 0]);
    assert!(matches!(
        decode_with(over, &DecodeOptions::new().with_strict(true)),
        Err(Error::InvalidData(_))
    ));
}

#[test]
fn colour_is_the_documented_default() {
    let i = decode(b"P5\n1 1\n255\n\x00").unwrap();
    assert_eq!(i.color, ColorInfo::netpbm_default());
    assert_eq!(i.color, ColorInfo::new(ColorRange::Full, 1, 1, 0));
    let f = decode(b"Pf\n1 1\n1.0\n\x3f\x80\x00\x00").unwrap();
    assert_eq!(f.color, ColorInfo::pfm_default());
    assert_eq!(f.color.transfer, 8);
    assert!(i.metadata.is_empty() && f.metadata.is_empty());
    assert_eq!(f.to_rgb8(), vec![255, 255, 255]);
}

#[test]
#[allow(deprecated)]
fn deprecated_wrappers_still_work() {
    use oxideav_pbm::{
        decode_pbm, decode_pbm_consumed, decode_pbm_multi, encode_pbm, encode_pbm_ascii,
        encode_pbm_with_format, probe_is_netpbm, PbmEncodeFormat, PbmPixelFormat, PbmPlane,
    };
    let plane: PbmPlane = Plane::new(3, vec![1, 2, 3]);
    let img = PbmImage::new(1, 1, PbmPixelFormat::Rgb24, vec![plane]).unwrap();
    let bytes = encode_pbm(&img).unwrap();
    assert_eq!(bytes, encode(&img, &EncodeOptions::default()).unwrap());
    assert!(probe_is_netpbm(&bytes));
    let (back, fmt) = decode_pbm(&bytes).unwrap();
    assert_eq!(fmt, PbmPixelFormat::Rgb24);
    assert_eq!(back, img);
    let (_, _, consumed) = decode_pbm_consumed(&bytes).unwrap();
    assert_eq!(consumed, bytes.len());
    assert_eq!(decode_pbm_multi(&bytes).unwrap().len(), 1);
    assert_eq!(
        encode_pbm_ascii(&img).unwrap(),
        encode(&img, &EncodeOptions::new().with_ascii(true)).unwrap()
    );
    assert_eq!(
        encode_pbm_with_format(&img, PbmEncodeFormat::Pam7).unwrap(),
        encode(&img, &EncodeOptions::new().with_pam(true)).unwrap()
    );
    assert_eq!(PbmPixelFormat::GrayF32, PbmPixelFormat::GrayF32Le);
}

#[cfg(feature = "registry")]
mod registry {
    use super::*;
    use oxideav_core::{
        CodecId, CodecParameters, Frame as CoreFrame, Packet, PixelFormat as CorePixelFormat,
        RuntimeContext, TimeBase, VideoFrame,
    };

    #[test]
    fn frame_bridge_both_ways() {
        let img = sample_image(PixelFormat::Rgba64Le, 3, 2);
        let frame: VideoFrame = img.clone().into();
        assert_eq!(frame.image_planes().len(), 1);
        assert_eq!(frame.image_planes()[0].data, img.as_bytes().unwrap());
        let mut params = CodecParameters::video(CodecId::new(oxideav_pbm::CODEC_ID_STR));
        params.width = Some(3);
        params.height = Some(2);
        params.pixel_format = Some(CorePixelFormat::Rgba64Le);
        assert_eq!(PbmImage::from_video_frame(&frame, &params).unwrap(), img);
        assert_eq!(PbmImage::try_from((&frame, &params)).unwrap(), img);
        for f in PixelFormat::ALL {
            let core: CorePixelFormat = f.into();
            assert_eq!(PixelFormat::try_from(core).unwrap(), f);
            assert_eq!(oxideav_pbm::to_core_pixel_format(f), core);
        }
    }

    #[test]
    fn registry_decoder_and_encoder_are_thin_adapters() {
        let mut ctx = RuntimeContext::new();
        oxideav_pbm::register(&mut ctx);
        let id = CodecId::new(oxideav_pbm::CODEC_ID_STR);
        assert!(ctx.codecs.has_decoder(&id) && ctx.codecs.has_encoder(&id));

        // Decoder: native layout out, one frame per concatenated image.
        let gray = sample_image(PixelFormat::Gray16Le, 4, 2);
        let rgb = sample_image(PixelFormat::Rgb24, 2, 2);
        let mut stream = encode(&gray, &EncodeOptions::default()).unwrap();
        stream.extend(encode(&rgb, &EncodeOptions::default()).unwrap());
        let params = CodecParameters::video(id.clone());
        let mut dec = oxideav_pbm::make_decoder(&params).unwrap();
        let mut pkt = Packet::new(0, TimeBase::new(1, 1), stream);
        pkt.pts = Some(7);
        dec.send_packet(&pkt).unwrap();
        let CoreFrame::Video(f0) = dec.receive_frame().unwrap() else {
            panic!("video frame expected")
        };
        assert_eq!(f0.pts, Some(7));
        assert_eq!(f0.image_planes()[0].data, gray.as_bytes().unwrap());
        let CoreFrame::Video(f1) = dec.receive_frame().unwrap() else {
            panic!("video frame expected")
        };
        assert_eq!(f1.image_planes()[0].data, rgb.as_bytes().unwrap());
        dec.flush().unwrap();
        assert!(matches!(dec.receive_frame(), Err(oxideav_core::Error::Eof)));

        // Encoder: the same bytes as the standalone `encode`, options
        // through the schema.
        let mut eparams = CodecParameters::video(id.clone());
        eparams.width = Some(2);
        eparams.height = Some(2);
        eparams.pixel_format = Some(CorePixelFormat::Rgb24);
        eparams.options.insert("ascii", "true");
        let mut enc = oxideav_pbm::make_encoder(&eparams).unwrap();
        enc.send_frame(&CoreFrame::Video(rgb.clone().into()))
            .unwrap();
        let out = enc.receive_packet().unwrap();
        assert_eq!(
            out.data,
            encode(&rgb, &EncodeOptions::new().with_ascii(true)).unwrap()
        );
        assert!(out.data.starts_with(b"P3\n"));
        // Unknown option keys are rejected at construction.
        eparams.options.insert("bogus", "1");
        assert!(oxideav_pbm::make_encoder(&eparams).is_err());
        // RGB through the registry is P6, never a bitmap.
        let mut eparams = CodecParameters::video(id);
        eparams.width = Some(2);
        eparams.height = Some(2);
        eparams.pixel_format = Some(CorePixelFormat::Rgb24);
        let mut enc = oxideav_pbm::make_encoder(&eparams).unwrap();
        enc.send_frame(&CoreFrame::Video(rgb.into())).unwrap();
        assert!(enc.receive_packet().unwrap().data.starts_with(b"P6\n"));
    }
}
