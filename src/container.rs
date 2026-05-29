//! Netpbm container: one single-image file becomes one [`Packet`] on
//! stream `0`. Mirrors the same shape as `oxideav-bmp` /
//! `oxideav-png` (non-APNG path) — Netpbm has no animation or
//! multi-frame layout to worry about.
//!
//! Lives behind the `registry` feature: the container types are all
//! defined by `oxideav-core`, so a standalone build (no framework dep)
//! has nothing meaningful to expose here.

use std::io::{Read, SeekFrom, Write};

use oxideav_core::{
    CodecId, CodecParameters, CodecResolver, Error, MediaType, Packet, PixelFormat, Result,
    StreamInfo, TimeBase,
};
use oxideav_core::{
    ContainerRegistry, Demuxer, Muxer, ProbeData, ProbeScore, ReadSeek, WriteSeek, MAX_PROBE_SCORE,
};

use crate::header::{parse_header, Magic, Tupltype};

pub fn register(reg: &mut ContainerRegistry) {
    reg.register_demuxer("pbm", open_demuxer);
    reg.register_muxer("pbm", open_muxer);
    // All five common Netpbm extensions map to the same demuxer/muxer
    // — the magic at byte 0 unambiguously says which sub-format we have.
    reg.register_extension("pbm", "pbm");
    reg.register_extension("pgm", "pbm");
    reg.register_extension("ppm", "pbm");
    reg.register_extension("pnm", "pbm");
    reg.register_extension("pam", "pbm");
    reg.register_probe("pbm", probe);
}

fn probe(data: &ProbeData) -> ProbeScore {
    if data.buf.len() >= 2 && data.buf[0] == b'P' {
        // Magic byte 1 must be one of '1'..'7' AND byte 2 must be ASCII
        // whitespace — that pair is rare enough in random data to claim
        // a max-score probe (the man pages mandate the whitespace).
        let m = data.buf[1];
        let ws_ok = data.buf.len() < 3 || matches!(data.buf[2], b'\n' | b' ' | b'\t' | b'\r');
        if matches!(m, b'1'..=b'7') && ws_ok {
            return MAX_PROBE_SCORE;
        }
    }
    if matches!(
        data.ext,
        Some("pbm") | Some("pgm") | Some("ppm") | Some("pnm") | Some("pam")
    ) {
        oxideav_core::PROBE_SCORE_EXTENSION
    } else {
        0
    }
}

pub fn open_demuxer(
    mut input: Box<dyn ReadSeek>,
    _codecs: &dyn CodecResolver,
) -> Result<Box<dyn Demuxer>> {
    input.seek(SeekFrom::Start(0))?;
    let mut buf = Vec::new();
    input.read_to_end(&mut buf)?;
    let header = parse_header(&buf)?;
    let pixel_format = pick_advertised_format(&header);
    let mut params = CodecParameters::video(CodecId::new(crate::CODEC_ID_STR));
    params.width = Some(header.width);
    params.height = Some(header.height);
    params.pixel_format = Some(pixel_format);
    let stream = StreamInfo {
        index: 0,
        params,
        time_base: TimeBase::new(1, 1),
        start_time: Some(0),
        duration: None,
    };
    Ok(Box::new(PbmDemuxer {
        streams: vec![stream],
        data: Some(buf),
    }))
}

fn pick_advertised_format(h: &crate::header::Header) -> PixelFormat {
    match h.magic {
        Magic::P1AsciiBitmap | Magic::P4BinaryBitmap => PixelFormat::MonoBlack,
        Magic::P2AsciiGraymap | Magic::P5BinaryGraymap => {
            if h.maxval > 255 {
                PixelFormat::Gray16Le
            } else {
                PixelFormat::Gray8
            }
        }
        Magic::P3AsciiPixmap | Magic::P6BinaryPixmap => {
            if h.maxval > 255 {
                PixelFormat::Rgb48Le
            } else {
                PixelFormat::Rgb24
            }
        }
        Magic::P7Pam => match (&h.tupltype, h.depth, h.maxval > 255) {
            (Some(Tupltype::BlackAndWhite), _, _) => PixelFormat::MonoBlack,
            (Some(Tupltype::Grayscale), _, false) => PixelFormat::Gray8,
            (Some(Tupltype::Grayscale), _, true) => PixelFormat::Gray16Le,
            (Some(Tupltype::Rgb), _, false) => PixelFormat::Rgb24,
            (Some(Tupltype::Rgb), _, true) => PixelFormat::Rgb48Le,
            (Some(Tupltype::GrayscaleAlpha), _, false) => PixelFormat::Ya8,
            (Some(Tupltype::GrayscaleAlpha), _, true) => PixelFormat::Rgba,
            (Some(Tupltype::BlackAndWhiteAlpha), _, _) => PixelFormat::Rgba,
            (Some(Tupltype::RgbAlpha), _, false) => PixelFormat::Rgba,
            (Some(Tupltype::RgbAlpha), _, true) => PixelFormat::Rgba64Le,
            // None and Custom(_) — DEPTH drives the advertised format.
            (None, 1, false) | (Some(Tupltype::Custom(_)), 1, false) => PixelFormat::Gray8,
            (None, 1, true) | (Some(Tupltype::Custom(_)), 1, true) => PixelFormat::Gray16Le,
            (None, 2, false) | (Some(Tupltype::Custom(_)), 2, false) => PixelFormat::Ya8,
            (None, 2, true) | (Some(Tupltype::Custom(_)), 2, true) => PixelFormat::Rgba,
            (None, 3, false) | (Some(Tupltype::Custom(_)), 3, false) => PixelFormat::Rgb24,
            (None, 3, true) | (Some(Tupltype::Custom(_)), 3, true) => PixelFormat::Rgb48Le,
            (None, 4, false) | (Some(Tupltype::Custom(_)), 4, false) => PixelFormat::Rgba,
            (None, 4, true) | (Some(Tupltype::Custom(_)), 4, true) => PixelFormat::Rgba64Le,
            _ => PixelFormat::Rgba,
        },
    }
}

struct PbmDemuxer {
    streams: Vec<StreamInfo>,
    data: Option<Vec<u8>>,
}

impl Demuxer for PbmDemuxer {
    fn format_name(&self) -> &str {
        "pbm"
    }
    fn streams(&self) -> &[StreamInfo] {
        &self.streams
    }
    fn next_packet(&mut self) -> Result<Packet> {
        match self.data.take() {
            Some(bytes) => {
                let mut pkt = Packet::new(0, TimeBase::new(1, 1), bytes);
                pkt.pts = Some(0);
                pkt.dts = Some(0);
                pkt.flags.keyframe = true;
                Ok(pkt)
            }
            None => Err(Error::Eof),
        }
    }
}

pub fn open_muxer(output: Box<dyn WriteSeek>, streams: &[StreamInfo]) -> Result<Box<dyn Muxer>> {
    if streams.len() != 1 {
        return Err(Error::invalid(
            "PBM muxer: expected exactly one video stream",
        ));
    }
    if streams[0].params.media_type != MediaType::Video {
        return Err(Error::invalid("PBM muxer: stream must be video"));
    }
    Ok(Box::new(PbmMuxer { output }))
}

struct PbmMuxer {
    output: Box<dyn WriteSeek>,
}

impl Muxer for PbmMuxer {
    fn format_name(&self) -> &str {
        "pbm"
    }
    fn write_header(&mut self) -> Result<()> {
        Ok(())
    }
    fn write_packet(&mut self, packet: &Packet) -> Result<()> {
        // The encoder produces a complete Netpbm file in a single
        // packet — write it through unchanged.
        self.output.write_all(&packet.data)?;
        Ok(())
    }
    fn write_trailer(&mut self) -> Result<()> {
        Ok(())
    }
}
