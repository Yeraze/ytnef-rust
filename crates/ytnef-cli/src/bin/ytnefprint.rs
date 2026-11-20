//! `ytnefprint`: dump a TNEF stream in the format of the C tool.
//!
//! Output (stdout, exit status) matches C `ytnefprint` byte for byte,
//! including the messages the C library prints while parsing, except:
//! `-h` prints the help and `-s <n>` sets the attribute size limit in MB
//! (C ytnefprint ignores both and parses the `-s` value as a file name), and
//! `-v` given more than once does not reproduce C's internal parser trace.

use std::io::{self, BufWriter, Write};
use std::process::ExitCode;

use ytnef_cli::parse_reported;
use ytnef_core::Limits;
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

fn print_file(out: &mut impl Write, path: &str, verbose: u32, limits: &Limits) -> io::Result<()> {
    if verbose >= 1 {
        writeln!(out, "Attempting to parse {path}...")?;
    }
    let Ok(buf) = std::fs::read(path) else {
        return out.write_all(b"ERROR processing file\n");
    };
    match parse_reported(out, &buf, verbose, limits)? {
        Ok(t) => print_tnef(out, &t, verbose),
        Err(_) => out.write_all(b"ERROR processing file\n"),
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
