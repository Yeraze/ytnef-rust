//! Compressed RTF (`PR_RTF_COMPRESSED`, MS-OXRTFCP): the C `DecompressRTF`.
//!
//! Output matches C byte for byte, including its quirks: the result is always
//! exactly the header's raw size (zero-filled if the stream ends early), and a
//! reference that points before the start of the dictionary copies nothing.

use crate::error::RtfError;
use crate::limits::Limits;

/// Magic of a compressed (`LZFu`) stream.
pub const MAGIC_COMPRESSED: u32 = 0x7546_5A4C;
/// Magic of an uncompressed (`MELA`) stream.
pub const MAGIC_UNCOMPRESSED: u32 = 0x414C_454D;

/// Initial dictionary (MS-OXRTFCP 2.1.2.1), 207 bytes, copied from C: it has
/// `\n\r` where the spec has `\r\n`, which shows in output that references it.
const PREBUF: &[u8] = b"{\\rtf1\\ansi\\mac\\deff0\\deftab720{\\fonttbl;}{\\f0\\fnil \\froman \\fswiss \\fmodern \\fscript \\fdecor MS Sans SerifSymbolArialTimes New RomanCourier{\\colortbl\\red0\\green0\\blue0\n\r\\par \\pard\\plain\\f0\\fs20\\b\\i\\u\\tab\\tx";

const HEADER: usize = 16;

fn le32(d: &[u8], at: usize) -> Option<u32> {
    d.get(at..)?
        .first_chunk::<4>()
        .map(|b| u32::from_le_bytes(*b))
}

/// True if `d` carries one of the two known magics (C `IsCompressedRTF`).
pub fn is_compressed_rtf(d: &[u8]) -> bool {
    matches!(le32(d, 8), Some(MAGIC_COMPRESSED | MAGIC_UNCOMPRESSED))
}

/// True if the header CRC matches: CRC-32 of the bytes after the header for
/// `LZFu`, zero for `MELA`. C never checks it, so [`decompress_rtf`] does not
/// either; callers may warn on a mismatch.
pub fn crc_matches(d: &[u8]) -> bool {
    let (Some(stored), Some(magic), Some(body)) = (le32(d, 12), le32(d, 8), d.get(HEADER..)) else {
        return false;
    };
    match magic {
        MAGIC_UNCOMPRESSED => stored == 0,
        _ => stored == crc32(body),
    }
}

/// MS-OXRTFCP 2.1.3.2: reflected CRC-32 (0xEDB88320), initial 0, no final xor.
fn crc32(d: &[u8]) -> u32 {
    d.iter().fold(0, |mut crc, &b| {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320 & (crc & 1).wrapping_neg());
        }
        crc
    })
}

/// Decompress a `PR_RTF_COMPRESSED` value. Every case where C returns NULL is
/// an `Err`; the output is capped by [`Limits::max_rtf_out`] and by 8 times
/// the input size.
pub fn decompress_rtf(d: &[u8], limits: &Limits) -> Result<Vec<u8>, RtfError> {
    let size = d.len();
    let (Some(stored), Some(raw), Some(magic), Some(body)) =
        (le32(d, 0), le32(d, 4), le32(d, 8), d.get(HEADER..))
    else {
        return Err(RtfError::TooShort { len: size });
    };
    if size < 20 {
        return Err(RtfError::TooShort { len: size });
    }
    if usize::try_from(stored).ok() != size.checked_sub(4) {
        return Err(RtfError::SizeMismatch { stored, len: size });
    }
    let raw_len = usize::try_from(raw).map_err(|_| RtfError::BadRawSize { raw })?;
    // C's checks come first, so a stream C rejects gets C's message; the
    // output cap is Rust-only
    let capped = |len: usize| {
        if len > limits.max_rtf_out {
            Err(RtfError::TooLarge {
                len,
                max: limits.max_rtf_out,
            })
        } else {
            Ok(())
        }
    };
    match magic {
        MAGIC_UNCOMPRESSED => {
            let v = body.get(..raw_len).ok_or(RtfError::BadRawSize { raw })?;
            capped(raw_len)?;
            Ok(v.to_vec())
        }
        // 17 input bytes (a flag byte and 8 references) give at most 136
        // output bytes, so C refuses a raw size above 8 times the input.
        MAGIC_COMPRESSED if raw_len / 8 > size => Err(RtfError::BadRawSize { raw }),
        MAGIC_COMPRESSED => {
            capped(raw_len)?;
            lzfu(body, raw_len)
        }
        _ => Err(RtfError::UnknownMagic { magic }),
    }
}

/// The C loop over a buffer holding [`PREBUF`] then the output, with
/// references resolved against 4096-byte blocks of that buffer.
// Every index and length stays below `total`, at most max_rtf_out + 207.
#[allow(clippy::arithmetic_side_effects)]
fn lzfu(src: &[u8], raw_len: usize) -> Result<Vec<u8>, RtfError> {
    let total = PREBUF.len() + raw_len;
    let mut buf = PREBUF.to_vec();
    let mut i = 0;
    let mut flags = 0u8;
    let mut flag_count = 0u32;
    let truncated = |i: usize| RtfError::Truncated { offset: HEADER + i };
    while buf.len() < total && i < src.len() {
        if flag_count % 8 == 0 {
            flags = src.get(i).copied().ok_or(truncated(i))?;
            i += 1;
        } else {
            flags >>= 1;
        }
        flag_count = flag_count.wrapping_add(1);
        if flags & 1 == 0 {
            buf.push(src.get(i).copied().ok_or(truncated(i))?);
            i += 1;
            continue;
        }
        let (Some(&hi), Some(&lo)) = (src.get(i), src.get(i + 1)) else {
            return Err(truncated(i));
        };
        i += 2;
        let out = buf.len();
        let mut off = out - out % 4096 + (usize::from(hi) << 4 | usize::from(lo >> 4));
        let len = usize::from(lo & 0xF) + 2;
        if off >= out {
            // From the previous block; before the first one, C's unsigned
            // offset wraps past the buffer and nothing is copied.
            match off.checked_sub(4096) {
                Some(o) => off = o,
                None => continue,
            }
        }
        // The source may overlap what this copy writes, so go byte by byte;
        // `off < out` keeps every read in bounds.
        for j in off..off + len {
            if buf.len() >= total {
                break;
            }
            buf.push(buf.get(j).copied().ok_or(truncated(i))?);
        }
    }
    buf.drain(..PREBUF.len());
    buf.resize(raw_len, 0);
    Ok(buf)
}

#[cfg(test)]
#[allow(
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::unwrap_used
)]
mod tests {
    use super::*;

    /// A value: 16-byte header (CRC 0), then `payload`; like C `stream()`.
    fn stream(magic: u32, raw: u32, payload: &[u8]) -> Vec<u8> {
        let size = u32::try_from(12 + payload.len()).unwrap();
        let mut v = [size, raw, magic, 0]
            .iter()
            .flat_map(|x| x.to_le_bytes())
            .collect::<Vec<_>>();
        v.extend_from_slice(payload);
        v
    }

    fn run(d: &[u8]) -> Result<Vec<u8>, RtfError> {
        decompress_rtf(d, &Limits::default())
    }

    /// MS-OXRTFCP 3.1.1.1 and 3.1.1.2.
    const SPEC_SIMPLE: &[u8] = &[
        0x2d, 0x00, 0x00, 0x00, 0x2b, 0x00, 0x00, 0x00, 0x4c, 0x5a, 0x46, 0x75, 0xf1, 0xc5, 0xc7,
        0xa7, 0x03, 0x00, 0x0a, 0x00, 0x72, 0x63, 0x70, 0x67, 0x31, 0x32, 0x35, 0x42, 0x32, 0x0a,
        0xf3, 0x20, 0x68, 0x65, 0x6c, 0x09, 0x00, 0x20, 0x62, 0x77, 0x05, 0xb0, 0x6c, 0x64, 0x7d,
        0x0a, 0x80, 0x0f, 0xa0,
    ];
    const SPEC_RUN: &[u8] = &[
        0x1a, 0x00, 0x00, 0x00, 0x1c, 0x00, 0x00, 0x00, 0x4c, 0x5a, 0x46, 0x75, 0xe2, 0xd4, 0x4b,
        0x51, 0x41, 0x00, 0x04, 0x20, 0x57, 0x58, 0x59, 0x5a, 0x0d, 0x6e, 0x7d, 0x01, 0x0e, 0xb0,
    ];

    #[test]
    fn spec_examples() {
        assert_eq!(PREBUF.len(), 207);
        assert_eq!(
            run(SPEC_SIMPLE).unwrap(),
            // the spec gives "\r\n": C's dictionary swaps them
            b"{\\rtf1\\ansi\\ansicpg1252\\pard hello world}\n\r"
        );
        assert_eq!(run(SPEC_RUN).unwrap(), b"{\\rtf1 WXYZWXYZWXYZWXYZWXYZ}");
    }

    #[test]
    fn crc() {
        assert!(crc_matches(SPEC_SIMPLE));
        assert!(crc_matches(SPEC_RUN));
        let mut bad = SPEC_SIMPLE.to_vec();
        bad[20] ^= 1;
        assert!(!crc_matches(&bad));
        // a mismatch is not an error, as in C
        assert!(run(&bad).is_ok());
        assert!(crc_matches(&stream(MAGIC_UNCOMPRESSED, 5, b"hello")));
        let mut mela = stream(MAGIC_UNCOMPRESSED, 5, b"hello");
        mela[12] = 1;
        assert!(!crc_matches(&mela));
        assert!(!crc_matches(&[0; 15]));
    }

    #[test]
    fn uncompressed() {
        assert_eq!(
            run(&stream(MAGIC_UNCOMPRESSED, 5, b"hello")).unwrap(),
            b"hello"
        );
        assert_eq!(
            run(&stream(MAGIC_UNCOMPRESSED, 2, b"hello")).unwrap(),
            b"he"
        );
        assert_eq!(
            run(&stream(MAGIC_UNCOMPRESSED, 0x4000, b"hello")),
            Err(RtfError::BadRawSize { raw: 0x4000 })
        );
    }

    #[test]
    fn compressed_literals_and_references() {
        assert_eq!(
            run(&stream(MAGIC_COMPRESSED, 3, b"\0abcd")).unwrap(),
            b"abc"
        );
        // input ends between flag bytes: the rest is zero-filled, as C does
        assert_eq!(
            run(&stream(MAGIC_COMPRESSED, 6, b"\0abc")).unwrap(),
            b"abc\0\0\0"
        );
        // reference into the initial dictionary
        assert_eq!(
            run(&stream(MAGIC_COMPRESSED, 2, &[1, 0, 0, 0])).unwrap(),
            b"{\\"
        );
        // reference ahead of the write position in the first block: C copies nothing
        assert_eq!(
            run(&stream(MAGIC_COMPRESSED, 1, &[1, 0xff, 0xf0, b'x'])).unwrap(),
            b"x"
        );
    }

    #[test]
    fn malformed() {
        let c = MAGIC_COMPRESSED;
        // C test_decompress_rtf.c cases
        assert!(matches!(
            run(&stream(c, 100, b"\x08abcd")),
            Err(RtfError::Truncated { .. })
        ));
        assert!(matches!(
            run(&stream(c, 100, b"\0abcdefgh\x01")),
            Err(RtfError::Truncated { .. })
        ));
        assert!(matches!(
            run(&stream(c, 100, b"\0abcdefgh\0")),
            Err(RtfError::Truncated { .. })
        ));
        assert!(run(&stream(c, 0xffff_ffff, b"\0abcd")).is_err());
        assert_eq!(
            run(&stream(0x1234_5678, 5, b"hello")),
            Err(RtfError::UnknownMagic { magic: 0x1234_5678 })
        );
        assert_eq!(run(&[0; 4]), Err(RtfError::TooShort { len: 4 }));
        assert_eq!(run(&[]), Err(RtfError::TooShort { len: 0 }));
        assert_eq!(
            run(&stream(c, 3, b"\0ab")),
            Err(RtfError::TooShort { len: 19 })
        );
        let mut v = stream(c, 3, b"\0abcd");
        v[0] = 99;
        assert_eq!(
            run(&v),
            Err(RtfError::SizeMismatch {
                stored: 99,
                len: 21
            })
        );
        // more output than 8x the input could give
        assert_eq!(
            run(&stream(c, 200, b"\0abcd")),
            Err(RtfError::BadRawSize { raw: 200 })
        );
        let tight = Limits {
            max_rtf_out: 2,
            ..Limits::default()
        };
        assert_eq!(
            decompress_rtf(&stream(c, 3, b"\0abcd"), &tight),
            Err(RtfError::TooLarge { len: 3, max: 2 })
        );
        // C's own checks win over the output cap (fuzz-found divergence)
        assert_eq!(
            decompress_rtf(&stream(0x1234_5678, 3, b"abcd"), &tight),
            Err(RtfError::UnknownMagic { magic: 0x1234_5678 })
        );
        assert_eq!(
            decompress_rtf(&stream(c, 200, b"\0abcd"), &tight),
            Err(RtfError::BadRawSize { raw: 200 })
        );
        assert_eq!(
            decompress_rtf(&stream(MAGIC_UNCOMPRESSED, 9, b"abcd"), &tight),
            Err(RtfError::BadRawSize { raw: 9 })
        );
    }

    #[test]
    fn no_panic_on_any_prefix_or_corruption() {
        for src in [SPEC_SIMPLE, SPEC_RUN] {
            for n in 0..=src.len() {
                let _ = run(&src[..n]);
            }
            for i in 0..src.len() {
                for b in [0, 0xff, 0x0f, 0xf0] {
                    let mut v = src.to_vec();
                    v[i] = b;
                    let _ = run(&v);
                }
            }
        }
    }

    #[test]
    fn magic_check() {
        assert!(is_compressed_rtf(&stream(MAGIC_COMPRESSED, 3, b"\0abcd")));
        assert!(is_compressed_rtf(&stream(MAGIC_UNCOMPRESSED, 3, b"abc")));
        assert!(!is_compressed_rtf(&stream(1, 3, b"abc")));
        assert!(!is_compressed_rtf(&[]));
    }
}
