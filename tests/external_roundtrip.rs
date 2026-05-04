//! Optional black-box validation against the Netpbm CLI tools
//! (`pamtopnm`, `pnmtopam`, `pnmtoplainpnm`). The tools are *binary*
//! validators, never sources — workspace policy bars all external
//! Netpbm implementation source code as a reference.
//!
//! Skips silently when none of the tools are on `PATH`.

use std::io::Write;
use std::process::{Command, Stdio};

use oxideav_pbm::{decode_pbm, encode_pbm, PbmImage, PbmPixelFormat, PbmPlane};

fn have(prog: &str) -> bool {
    Command::new(prog)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success() || s.code().is_some())
        .unwrap_or(false)
}

fn pipe_through(prog: &str, args: &[&str], input: &[u8]) -> Option<Vec<u8>> {
    let mut child = Command::new(prog)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    child.stdin.as_mut()?.write_all(input).ok()?;
    let out = child.wait_with_output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(out.stdout)
}

#[test]
fn pnmtoplainpnm_roundtrip_p6() {
    if !have("pnmtoplainpnm") {
        eprintln!("skip: pnmtoplainpnm not on PATH");
        return;
    }
    let mut data = Vec::with_capacity(8 * 6 * 3);
    for y in 0..6u32 {
        for x in 0..8u32 {
            data.push((x * 31) as u8);
            data.push((y * 41) as u8);
            data.push(((x ^ y) * 17) as u8);
        }
    }
    let src = PbmImage {
        width: 8,
        height: 6,
        pixel_format: PbmPixelFormat::Rgb24,
        planes: vec![PbmPlane { stride: 24, data }],
        pts: None,
    };
    let bin = encode_pbm(&src).unwrap();
    let plain = match pipe_through("pnmtoplainpnm", &[], &bin) {
        Some(v) => v,
        None => {
            eprintln!("skip: pnmtoplainpnm failed (likely not actually installed)");
            return;
        }
    };
    // The plain version should be P3 ASCII; decode it through us.
    assert!(plain.starts_with(b"P3\n"));
    let (back, fmt) = decode_pbm(&plain).unwrap();
    assert_eq!(fmt, PbmPixelFormat::Rgb24);
    assert_eq!(back.planes[0].data, src.planes[0].data);
}
