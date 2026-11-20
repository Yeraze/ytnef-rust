//! `ytnefprint`: dump a TNEF stream in the format of the C tool.
//!
//! Output (stdout, exit status) matches C `ytnefprint` byte for byte,
//! including the messages the C library prints while parsing, except:
//! `-h` prints the help and `-s <n>` sets the attribute size limit in MB
//! (C ytnefprint ignores both and parses the `-s` value as a file name), and
//! `-v` given more than once does not reproduce C's internal parser trace.

use std::io::{self, BufWriter, Write};
use std::process::ExitCode;

use ytnef_core::attr::id;
use ytnef_core::{Error, HandlerError, Limits, Tnef};
use ytnef_export::print::print_tnef;

const HELP: &str = "Yerase TNEF Printer v2.1.4

  usage: ytnefprint <options> <filenames>

Options:
   -h   - Displays this help message
   -s <n> - Override the default max attachment size (50 MB)
   -v   - Verbose output (multiple -v's increase\x20
                   the level of output.

Send bug reports to randall.hand@gmail.com
";

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
        (id::RECIP_TABLE, HandlerError::Truncated { .. }) => sizecheck(408),
        // the value itself runs past the block; C checks other reads at
        // other lines, which the Rust error does not identify
        (_, HandlerError::Truncated { .. }) if mapi => sizecheck(564),
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

/// Payload size from the attribute header at `offset`.
fn attr_size(buf: &[u8], offset: usize) -> usize {
    buf.get(offset + 5..)
        .and_then(|b| b.first_chunk::<4>())
        .map_or(0, |b| u32::from_le_bytes(*b) as usize)
}

/// Offset of the attribute header an error points at, if it names one.
fn error_attr_offset(e: &Error) -> Option<usize> {
    match e {
        Error::EmptyAttribute { offset, .. }
        | Error::BadChecksum { offset, .. }
        | Error::InHandler { offset, .. } => Some(*offset),
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

fn print_file(out: &mut impl Write, path: &str, verbose: u32, limits: &Limits) -> io::Result<()> {
    if verbose >= 1 {
        writeln!(out, "Attempting to parse {path}...")?;
    }
    let Ok(buf) = std::fs::read(path) else {
        return out.write_all(b"ERROR processing file\n");
    };
    match ytnef_core::parse(&buf, limits) {
        Ok(t) => {
            if verbose >= 1 {
                trace(out, &t, &buf)?;
            }
            print_tnef(out, &t, verbose)
        }
        Err(e) => {
            if verbose >= 1 {
                // replay what parsed cleanly before the failing attribute
                let end = error_attr_offset(&e).unwrap_or(0);
                if let Ok(t) = ytnef_core::parse(&buf[..end], limits) {
                    trace(out, &t, &[])?;
                }
            }
            out.write_all(error_message(&e, &buf, verbose).as_bytes())?;
            out.write_all(b"ERROR processing file\n")
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut out = BufWriter::new(io::stdout().lock());
    if args.is_empty() {
        let _ = write!(out, "You must specify files to parse\n{HELP}");
        let _ = out.flush();
        return ExitCode::from(255);
    }
    let mut verbose = 0;
    let mut limits = Limits::default();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if let Some(opt) = arg.strip_prefix('-') {
            match opt.chars().next() {
                Some('v') => verbose += 1,
                Some('h') => {
                    let _ = out.write_all(HELP.as_bytes());
                }
                Some('s') => match args.next().and_then(|n| n.parse::<usize>().ok()) {
                    Some(mb) => limits.max_attr_size = mb.saturating_mul(1024 * 1024),
                    None => {
                        eprintln!("ytnefprint: -s needs a size in MB");
                        return ExitCode::FAILURE;
                    }
                },
                _ => {}
            }
            continue;
        }
        if let Err(e) = print_file(&mut out, arg, verbose, &limits) {
            eprintln!("ytnefprint: {e}");
            return ExitCode::FAILURE;
        }
    }
    if let Err(e) = out.flush() {
        eprintln!("ytnefprint: {e}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
