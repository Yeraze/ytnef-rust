//! `ytnef`: extract attachments and message bodies from TNEF streams.
//!
//! Flags, stdout, stderr, exit status and written files match C `ytnef`,
//! except where C was unsafe or unbounded:
//!
//! - Output names are one path component: C's `/`, `\` and space become `_`
//!   as in C, control bytes also become `_`, leading dots are stripped, and
//!   names are cut to 255 bytes. A name left empty falls back to C's
//!   `file_NNN.dat`.
//! - Files are created with `create_new`, so an existing file or symlink is
//!   never overwritten or followed; on a clash the name gets a counter before
//!   its extension (`x.dat` -> `x1.dat`), the scheme C already uses for
//!   sticky-note bodies. C overwrites.
//! - Embedded TNEF recursion stops at `Limits::max_nesting` (default 8; C
//!   stops at 16). An embedded stream that fails to parse is saved as a plain
//!   attachment; C processes whatever it parsed before the error.
//! - vCard, vTask and vCalendar files go through the same writer, so a
//!   write error is reported on stderr and counted as a failure (C prints
//!   `Error writing file to disk!` to stdout and carries on). The file
//!   contents follow C except where `ytnef_export` documents otherwise.
//! - `-v` given more than once does not reproduce C's internal parser trace.

use std::ffi::OsString;
use std::io::{self, BufWriter, Write};
use std::process::ExitCode;

use ytnef_cli::process::{os_bytes, Opts};
use ytnef_core::Limits;

const MIB: usize = 1024 * 1024;

fn help(size_mb: usize) -> String {
    format!(
        "Yerase TNEF Exporter v2.1.4

  usage: ytnef [-+vhf] <filenames>

   -/+v - Enables/Disables verbose output
          Multiple -v's increase the level of output
   -/+f - Enables/Disables saving of attachments
   -/+F - Enables/Disables saving of the message body as RTF
   -/+H - Enables/Disables saving of the message body as HTML
   -/+a - Enables/Disables saving of intermediate files
   -s <n> - Override the default Attachment Size ({size_mb} MB)
   -h   - Displays this help message

Example:
  ytnef -v winmail.dat
     Parse with verbose output, don't save
  ytnef -f . winmail.dat
     Parse and save all attachments to local directory (.)
  ytnef -F -f . winmail.dat
     Parse and save all attachments to local directory (.)
     Including saving the message text to a RTF file.

Send bug reports to randall.hand@gmail.com
"
    )
}

/// C `atoi`: leading whitespace, optional sign, digits; anything else is 0.
fn atoi(s: &[u8]) -> i64 {
    let s = s.trim_ascii_start();
    let (neg, digits) = match s.first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let n = digits
        .iter()
        .take_while(|c| c.is_ascii_digit())
        .fold(0i64, |n, &c| {
            n.saturating_mul(10).saturating_add(i64::from(c - b'0'))
        });
    if neg {
        -n
    } else {
        n
    }
}

/// C returns `-errors` from `main`; the shell sees it modulo 256.
fn exit(errors: u32) -> ExitCode {
    ExitCode::from(errors.wrapping_neg() as u8)
}

fn run(out: &mut impl Write) -> io::Result<ExitCode> {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    if args.is_empty() {
        write!(out, "You must specify files to parse\n{}", help(50))?;
        return Ok(exit(1));
    }
    let mut o = Opts {
        verbose: 0,
        save_files: false,
        save_rtf: false,
        save_html: false,
        save_intermediate: false,
        dir: None,
        limits: Limits::default(),
    };
    let mut size_mb = 50;
    let mut errors = 0u32;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let a = os_bytes(arg);
        let flag = a.get(1).copied();
        match a.first() {
            Some(b'-') => match flag {
                Some(b'a') => o.save_intermediate = true,
                Some(b'v') => o.verbose += 1,
                Some(b'h') => {
                    out.write_all(help(size_mb).as_bytes())?;
                    return Ok(ExitCode::SUCCESS);
                }
                Some(b'f') => {
                    let Some(dir) = args.next() else {
                        out.write_all(b"Option '-f' requires a directory\n")?;
                        return Ok(exit(1));
                    };
                    o.save_files = true;
                    o.dir = Some(dir.clone());
                }
                Some(b's') => {
                    let Some(n) = args.next() else {
                        out.write_all(b"Option '-s' requires a size\n")?;
                        return Ok(exit(1));
                    };
                    let n = atoi(&os_bytes(n));
                    if !(1..=2047).contains(&n) {
                        out.write_all(b"Option '-s' must be between 1 and 2047\n")?;
                        return Ok(exit(1));
                    }
                    size_mb = n as usize;
                    o.limits.max_attr_size = size_mb * MIB;
                }
                Some(b'F') => o.save_rtf = true,
                Some(b'H') => o.save_html = true,
                _ => unknown(out, &a, size_mb)?,
            },
            Some(b'+') => match flag {
                Some(b'a') => o.save_intermediate = false,
                Some(b'v') => o.verbose -= 1,
                Some(b'f') => {
                    o.save_files = false;
                    o.dir = None;
                }
                Some(b'F') => o.save_rtf = false,
                Some(b'H') => o.save_html = false,
                _ => unknown(out, &a, size_mb)?,
            },
            _ => errors = errors.saturating_add(o.file(out, arg)?),
        }
    }
    Ok(exit(errors))
}

fn unknown(out: &mut impl Write, arg: &[u8], size_mb: usize) -> io::Result<()> {
    out.write_all(b"Unknown option '")?;
    out.write_all(arg)?;
    out.write_all(b"'\n")?;
    out.write_all(help(size_mb).as_bytes())
}

fn main() -> ExitCode {
    let mut out = BufWriter::new(io::stdout().lock());
    let code = run(&mut out).and_then(|c| out.flush().map(|()| c));
    code.unwrap_or_else(|e| {
        eprintln!("ytnef: {e}");
        ExitCode::FAILURE
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atoi_matches_c() {
        assert_eq!(atoi(b"12"), 12);
        assert_eq!(atoi(b"  7mb"), 7);
        assert_eq!(atoi(b"-3"), -3);
        assert_eq!(atoi(b"abc"), 0);
        assert_eq!(atoi(b""), 0);
    }
}
