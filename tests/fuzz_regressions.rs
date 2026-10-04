//! Regressions from the `multi` fuzz target: `info` and `decode_all`
//! must agree on what is a stream (whenever `decode_all` succeeds,
//! `info` succeeds and counts the same frames).

use oxideav_pbm::{decode, decode_all, info, probe, Error};

/// Fuzz unit `crash-8261079416ab563926cd2db85f329b7e8176f5f1` (Base64
/// `ClA1MQoxCjIO`): a leading newline, then a valid 1×1 P5 (`maxval`
/// 2, one body byte). The stream walker used to skip whitespace before
/// the FIRST image while `probe` / `info` / `decode` require the magic
/// at byte 0; the staged description says a file "is always a two-byte
/// magic number followed by …", so the walker now agrees: leading
/// whitespace is a malformed file.
#[test]
fn leading_whitespace_before_the_first_magic_is_rejected_everywhere() {
    let data: &[u8] = b"\nP51\n1\n2\x0e";
    assert!(!probe(data));
    assert!(matches!(info(data), Err(Error::InvalidData(_))));
    assert!(matches!(decode(data), Err(Error::InvalidData(_))));
    assert!(matches!(decode_all(data), Err(Error::InvalidData(_))));
    // The same bytes without the leading newline are one valid image,
    // and every entry point agrees on that too.
    let ok = &data[1..];
    let frames = decode_all(ok).unwrap();
    assert_eq!(frames.len(), 1);
    assert_eq!((frames[0].image.width, frames[0].image.height), (1, 1));
    assert_eq!(frames[0].header.maxval, 2);
    let i = info(ok).unwrap();
    assert_eq!(i.frames, 1);
    assert_eq!(i.format, frames[0].image.format);
    assert_eq!(decode(ok).unwrap(), frames[0].image);
    // Whitespace BETWEEN images is still a valid separator, and the
    // walkers agree on the count.
    let mut two = ok.to_vec();
    two.extend_from_slice(b"\n\n");
    two.extend_from_slice(ok);
    assert_eq!(decode_all(&two).unwrap().len(), 2);
    assert_eq!(info(&two).unwrap().frames, 2);
    // Every leading whitespace byte the walker used to skip is rejected.
    for ws in [b' ', b'\t', b'\r', b'\n', 0x0B, 0x0C] {
        let mut d = vec![ws];
        d.extend_from_slice(ok);
        assert!(decode_all(&d).is_err(), "leading {ws:#04x}");
        assert!(info(&d).is_err(), "leading {ws:#04x}");
    }
}
