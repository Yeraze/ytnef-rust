#![no_main]

//! `ytnef -f <tempdir>` on one input: every body, export and attachment
//! write, recursion into embedded streams, and the safe file writer.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use libfuzzer_sys::fuzz_target;
use ytnef_cli::parse_reported;
use ytnef_cli::process::Opts;
use ytnef_core::Limits;

static RUN: AtomicU64 = AtomicU64::new(0);

fuzz_target!(|data: &[u8]| {
    let Some((&flags, buf)) = data.split_first() else {
        return;
    };
    let dir: PathBuf = std::env::temp_dir().join(format!(
        "ytnef-fuzz-{}-{}",
        std::process::id(),
        RUN.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let o = Opts {
        verbose: i64::from(flags & 3),
        save_files: true,
        save_rtf: flags & 4 != 0,
        save_html: flags & 8 != 0,
        save_intermediate: flags & 16 != 0,
        dir: Some(dir.clone().into_os_string()),
        limits: Limits {
            max_rtf_out: 1 << 20,
            ..Limits::default()
        },
    };
    let mut out = Vec::new();
    if let Ok(t) = parse_reported(&mut out, buf, o.verbose(), &o.limits).unwrap() {
        o.process(&mut out, &t, 0).unwrap();
    }
    // every file stays inside the output directory
    for e in std::fs::read_dir(&dir).unwrap() {
        let e = e.unwrap();
        assert!(e.file_type().unwrap().is_file());
    }
    std::fs::remove_dir_all(&dir).unwrap();
});
