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

use crate::header::{parse_header, probe_is_netpbm};

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
    // Portable FloatMap (`Pf` / `PF`) — the floating-point member of the
    // family, same single-image-per-file shape.
    reg.register_extension("pfm", "pbm");
    reg.register_probe("pbm", probe);
}

fn probe(data: &ProbeData) -> ProbeScore {
    // The `magic + whitespace` structural check lives in the always-compiled
    // header module so a standalone consumer can reuse it; delegate here
    // rather than re-implementing the byte rule. That pair is rare enough
    // in random data to claim a max-score probe (the format specs mandate
    // the whitespace / LF separator immediately after the two-byte magic).
    if probe_is_netpbm(data.buf) {
        return MAX_PROBE_SCORE;
    }
    if matches!(
        data.ext,
        Some("pbm") | Some("pgm") | Some("ppm") | Some("pnm") | Some("pam") | Some("pfm")
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
    let mut params = CodecParameters::video(CodecId::new(crate::CODEC_ID_STR));
    params.width = Some(header.width);
    params.height = Some(header.height);
    // Advertise the native layout the decoder will produce — every
    // Netpbm / PFM layout now has a core counterpart. The decoder is
    // self-describing from the byte stream, so this is advisory.
    params.pixel_format = pick_advertised_format(&header);
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

fn pick_advertised_format(h: &crate::header::Header) -> Option<PixelFormat> {
    crate::decoder::native_format(h)
        .ok()
        .map(crate::registry::to_core_pixel_format)
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
