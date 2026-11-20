use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut status = ExitCode::SUCCESS;
    for path in std::env::args_os().skip(1).map(PathBuf::from) {
        match std::fs::read(&path) {
            Ok(buf) if ytnef_core::has_signature(&buf) => {
                println!(
                    "{}: TNEF signature OK (parsing not implemented yet)",
                    path.display()
                );
            }
            Ok(_) => {
                eprintln!("{}: not a TNEF stream", path.display());
                status = ExitCode::FAILURE;
            }
            Err(e) => {
                eprintln!("{}: {e}", path.display());
                status = ExitCode::FAILURE;
            }
        }
    }
    status
}
