//! Shared by the `ytnef` and `ytnefprint` binaries: replaying the messages the
//! C library prints while it parses, and `ytnef` file processing.

pub mod process;

use std::io::{self, Write};

use ytnef_core::attr::id;
use ytnef_core::{Error, HandlerError, Limits, Tnef};

/// C `PREALLOCCHECK` failure message; `line` is where it sits in C `ytnef.c`.
fn prealloc(line: u32, value: impl std::fmt::Display, max: usize) -> String {
    format!(
        "ERROR: invalid alloc size {value} at ytnef.c : {line}, suspected corruption (exceeded {max} bytes)\n"
    )
}

/// C `SIZECHECK` failure message.
fn sizecheck(line: u32) -> String {
    format!("Corrupted file detected at ytnef.c : {line}\n")
}

/// What C prints inside a failing attribute handler.
fn handler_message(attr: u32, reason: &HandlerError, size: usize, verbose: u32) -> String {
    let mapi = attr == id::MAPI_PROPS || attr == id::ATTACHMENT;
    let mut s = match (attr, reason) {
        (id::TNEF_VERSION, _) => "Incorrect size of version field, suspected corruption\n".into(),
        (_, HandlerError::TooLarge { len, max }) => {
            let line = match attr {
                id::BODY => 299,
                id::ORIGINAL_MESSAGE_CLASS => 310,
                id::FROM => 325,
                id::SUBJECT => 339,
                id::ATTACH_META_FILE => 392,
                id::ATTACH_TITLE => 828,
                _ => 565, // a MAPI value
            };
            prealloc(line, len, *max)
        }
        (
            _,
            HandlerError::OutOfRange {
                what, value, max, ..
            },
        ) => {
            let line = match *what {
                "property count" => 472,
                "name count" => 497,
                "name length" => 505,
                "value count" => 535,
                _ => 650, // subject length
            };
            prealloc(line, value, *max)
        }
        (_, HandlerError::BadPropType { .. }) => "Bad file\n".into(),
        (_, HandlerError::TrailingBytes { len }) if verbose >= 1 => format!(
            "ERROR DURING MAPI READ\nRead {} bytes, Expected {size} bytes\n{len} bytes missing\n",
            size - len
        ),
        (_, HandlerError::Overrun { extra }) if verbose >= 1 => format!(
            "ERROR DURING MAPI READ\nRead {} bytes, Expected {size} bytes\n{extra} bytes extra\n",
            size + extra
        ),
        (id::RECIP_TABLE, HandlerError::Truncated { .. }) => sizecheck(408),
        (_, HandlerError::MapiTruncated { what, .. }) if mapi => sizecheck(match *what {
            "property count" => 468,
            "property tag" => 480,
            "name guid" => 489,
            "name count" => 493,
            "name length" => 502,
            "name" => 509,
            "name id" => 518,
            "value count" => 530,
            "value length" => 557,
            "PT_I2 value" => 594,
            "32-bit value" => 609,
            "64-bit value" => 621,
            "PT_CLSID value" => 631,
            _ => 564, // a variable-length value
        }),
        // a short date, and anything else C rejects without a message
        _ => String::new(),
    };
    if attr == id::MAPI_PROPS {
        s += "ERROR Parsing MAPI block\n";
    }
    s
}

/// What the C library prints when `TNEFParse` fails with `e`.
fn error_message(e: &Error, buf: &[u8], verbose: u32) -> String {
    let v = verbose >= 1;
    match e {
        Error::NotTnefStream => "ERROR: Signature does not match. Not TNEF.\n".into(),
        Error::NoSignature => "ERROR: Error reading signature\n".into(),
        Error::NoKey if v => "Error reading Key\nERROR: Unable to retrieve key.\n".into(),
        Error::NoKey => "ERROR: Unable to retrieve key.\n".into(),
        Error::EmptyAttribute { .. } => "ERROR: Field with size of 0\n".into(),
        Error::Truncated { .. } if v => {
            "ERROR: Error reading data\nERROR: Unable to read data.\n".into()
        }
        Error::Truncated { .. } => "ERROR: Unable to read data.\n".into(),
        Error::NoChecksum { .. } if v => {
            "ERROR: Error reading data\nERROR: Unable to read checksum.\n".into()
        }
        Error::NoChecksum { .. } => "ERROR: Unable to read checksum.\n".into(),
        Error::BadChecksum { .. } => "ERROR: Checksum mismatch. Data corruption?:\n".into(),
        Error::InHandler {
            attr: id::SENT_FOR,
            offset,
            ..
        } => {
            let size = attr_size(buf, *offset);
            let data = buf.get(offset + 9..).unwrap_or_default();
            sent_for_failure(&data[..size.min(data.len())], verbose)
        }
        Error::InHandler {
            attr,
            offset,
            reason,
        } => {
            let size = attr_size(buf, *offset);
            handler_message(*attr, reason, size, verbose)
        }
        Error::LimitExceeded {
            what: "attribute size",
            value,
            max,
        } => prealloc(1240, value, *max),
        // Rust-only limits: C has no equivalent message
        e => format!("ERROR: {e}\n"),
    }
}

/// C `TNEFSentFor` on a payload it rejects: the debug prints up to the
/// failing `SIZECHECK`, then its message.
fn sent_for_failure(d: &[u8], verbose: u32) -> String {
    let mut s = String::new();
    let mut pos = 0;
    let lossy = |b: &[u8]| String::from_utf8_lossy(upto_nul(b)).into_owned();
    while pos < d.len() {
        for (len_line, data_line) in [(703, 706), (711, 714)] {
            let Some(n) = d.get(pos..pos + 2) else {
                return s + &sizecheck(len_line);
            };
            let n = usize::from(u16::from_le_bytes([n[0], n[1]]));
            pos += 2;
            let Some(v) = d.get(pos..pos + n) else {
                return s + &sizecheck(data_line);
            };
            pos += n;
            if verbose >= 1 {
                s += &if len_line == 703 {
                    format!("Sent For : {}", lossy(v))
                } else {
                    format!("<{}>\n", lossy(v))
                };
            }
        }
    }
    s
}

/// Payload size from the attribute header at `offset`.
fn attr_size(buf: &[u8], offset: usize) -> usize {
    buf.get(offset + 5..)
        .and_then(|b| b.first_chunk::<4>())
        .map_or(0, |b| u32::from_le_bytes(*b) as usize)
}

/// Offset of the attribute header an error points at. Read errors and an
/// oversized attribute name no offset: the failing attribute is the first
/// whose payload and checksum run past the end or whose size is over the cap.
fn error_attr_offset(e: &Error, buf: &[u8], limits: &Limits) -> Option<usize> {
    match e {
        Error::EmptyAttribute { offset, .. }
        | Error::BadChecksum { offset, .. }
        | Error::InHandler { offset, .. } => Some(*offset),
        Error::Truncated { .. }
        | Error::NoChecksum { .. }
        | Error::LimitExceeded {
            what: "attribute size",
            ..
        } => {
            let mut at = 6;
            loop {
                let size = attr_size(buf, at);
                let next = at + 9 + size + 2;
                if size > limits.max_attr_size || next > buf.len() {
                    return Some(at);
                }
                at = next;
            }
        }
        _ => None,
    }
}

/// `%.*s`: at most `n` bytes, stopping at a NUL.
fn upto_nul(b: &[u8]) -> &[u8] {
    &b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())]
}

/// What the C library prints at debug level 1 while it walks the stream.
fn trace(out: &mut impl Write, t: &Tnef, buf: &[u8]) -> io::Result<()> {
    for a in &t.attributes {
        let name = match a.id {
            id::NULL => "Null",
            id::MESSAGE_STATUS => "Message Status",
            id::CONVERSATION_ID => "Conversation ID",
            id::ATTACH_TRANSPORT_FILENAME => "Attachment Transport name",
            id::OWNER => "Owner",
            id::DELEGATE => "Delegate",
            id::AID_OWNER => "Aid Owner",
            id::REQUEST_RES => "Request Response",
            id::SENT_FOR => {
                let mut d = a.data;
                while let Some((name, rest)) = counted(d) {
                    let Some((addr, rest)) = counted(rest) else {
                        break;
                    };
                    out.write_all(b"Sent For : ")?;
                    out.write_all(upto_nul(name))?;
                    out.write_all(b"<")?;
                    out.write_all(upto_nul(addr))?;
                    out.write_all(b">\n")?;
                    d = rest;
                }
                continue;
            }
            _ => continue,
        };
        write!(out, "{name}: [{}] ", a.data.len())?;
        out.write_all(upto_nul(a.data))?;
        out.write_all(b"\n")?;
    }
    // C reads the 1-byte level, then fails on the 4-byte type or size
    let used = t
        .attributes
        .last()
        .map_or(6, |a| a.offset + 9 + a.data.len() + 2);
    match buf.len().saturating_sub(used) {
        1..=4 => out.write_all(b"ERROR: Error reading type\n"),
        5..=8 => out.write_all(b"ERROR: Error reading size\n"),
        _ => Ok(()),
    }
}

/// A u16-length-prefixed field, as in `attSentFor`.
fn counted(d: &[u8]) -> Option<(&[u8], &[u8])> {
    let n = usize::from(u16::from_le_bytes(*d.first_chunk::<2>()?));
    let rest = d.get(2..)?;
    Some((rest.get(..n)?, rest.get(n..)?))
}

/// Parse `buf`, writing what the C library prints at `verbose` while parsing:
/// the debug-level trace and, on failure, its error message.
pub fn parse_reported<'a>(
    out: &mut impl Write,
    buf: &'a [u8],
    verbose: u32,
    limits: &Limits,
) -> io::Result<Result<Tnef<'a>, Error>> {
    let res = ytnef_core::parse(buf, limits);
    match &res {
        Ok(t) => {
            if verbose >= 1 {
                trace(out, t, buf)?;
            }
        }
        Err(e) => {
            if verbose >= 1 {
                // replay what parsed cleanly before the failing attribute
                let end = error_attr_offset(e, buf, limits).unwrap_or(0);
                if let Some(t) = buf
                    .get(..end)
                    .and_then(|b| ytnef_core::parse(b, limits).ok())
                {
                    trace(out, &t, &[])?;
                }
            }
            out.write_all(error_message(e, buf, verbose).as_bytes())?;
        }
    }
    Ok(res)
}
