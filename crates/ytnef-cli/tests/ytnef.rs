//! `ytnef` checks that need no C oracle: the extraction part of C `test.sh`,
//! the safe writer, embedded-TNEF depth, and clean failure on malformed input.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

/// A fresh, empty directory for one test.
fn scratch(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("ytnef")
        .join(name);
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn ytnef(args: &[&std::ffi::OsStr]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ytnef"))
        .args(args)
        .output()
        .unwrap()
}

fn extract(dir: &Path, fixture: &str) -> Output {
    let f = fixtures().join(fixture);
    ytnef(&["-f".as_ref(), dir.as_os_str(), f.as_os_str()])
}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

/// C `test.sh`: winmail.dat and every top-level `*.tnef` into one
/// directory must reproduce `tests/fixtures/results/` byte for byte.
#[test]
fn results_tree() {
    let out = scratch("results");
    let mut inputs = vec!["winmail.dat".to_owned()];
    inputs.extend(
        names(&fixtures())
            .into_iter()
            .filter(|n| n.ends_with(".tnef")),
    );
    for f in &inputs {
        let o = extract(&out, f);
        assert_eq!(o.status.code(), Some(0), "{f}");
    }
    let want = fixtures().join("results");
    assert_eq!(names(&out), names(&want));
    for n in names(&want) {
        assert!(
            fs::read(out.join(&n)).unwrap() == fs::read(want.join(&n)).unwrap(),
            "{n} differs"
        );
    }
}

#[test]
fn path_traversal_stays_inside() {
    let root = scratch("traversal");
    let out = root.join("out");
    fs::create_dir(&out).unwrap();
    let o = extract(&out, "regress/path-traversal.tnef");
    assert_eq!(o.status.code(), Some(0));
    assert_eq!(names(&root), ["out"]);
    // C writes ".._ESC1"; leading dots are stripped here
    assert_eq!(names(&out), ["_ESC1"]);
    let shown = format!("{}/_ESC1\n", out.display());
    assert_eq!(String::from_utf8_lossy(&o.stdout), shown);
}

#[test]
fn nested_tnef_stops_at_depth_limit() {
    let out = scratch("nested");
    let o = extract(&out, "regress/nested-tnef.tnef");
    assert_eq!(o.status.code(), Some(0));
    assert_eq!(names(&out), ["x.dat"]);
    // the fixture nests 20 deep, so the stream at the depth limit is saved
    // as an attachment instead of being unpacked
    let x = fs::read(out.join("x.dat")).unwrap();
    assert!(ytnef_core::has_signature(&x));

    // -a also keeps every intermediate stream, each under a new name
    let out = scratch("nested-a");
    let f = fixtures().join("regress/nested-tnef.tnef");
    let o = ytnef(&["-a".as_ref(), "-f".as_ref(), out.as_os_str(), f.as_os_str()]);
    assert_eq!(o.status.code(), Some(0));
    let limit = usize::from(ytnef_core::Limits::default().max_nesting);
    assert_eq!(names(&out).len(), limit + 1);
}

#[test]
fn never_overwrites() {
    let out = scratch("clobber");
    fs::write(out.join("AUTHORS"), "keep").unwrap();
    let o = extract(&out, "one-file.tnef");
    assert_eq!(o.status.code(), Some(0));
    assert_eq!(fs::read(out.join("AUTHORS")).unwrap(), b"keep");
    assert_eq!(
        fs::read(out.join("AUTHORS1")).unwrap(),
        fs::read(fixtures().join("results/AUTHORS")).unwrap()
    );
    let shown = format!("{}/AUTHORS1\n", out.display());
    assert_eq!(String::from_utf8_lossy(&o.stdout), shown);
}

#[cfg(unix)]
#[test]
fn never_follows_symlinks() {
    let root = scratch("symlink");
    let out = root.join("out");
    fs::create_dir(&out).unwrap();
    let target = root.join("target");
    std::os::unix::fs::symlink(&target, out.join("AUTHORS")).unwrap();
    let o = extract(&out, "one-file.tnef");
    assert_eq!(o.status.code(), Some(0));
    assert!(!target.exists());
    assert!(out.join("AUTHORS1").is_file());
}

#[test]
fn unwritable_dir_fails_cleanly() {
    let out = scratch("missing").join("does-not-exist");
    let o = extract(&out, "one-file.tnef");
    assert_eq!(o.status.code(), Some(255));
    assert!(String::from_utf8_lossy(&o.stderr).starts_with("ERROR: Error writing "));
}

/// Every malformed fixture exits with a C-style error, never a panic.
#[test]
fn malformed_inputs_fail_cleanly() {
    for dir in ["poc", "regress"] {
        for n in names(&fixtures().join(dir)) {
            let out = scratch(&format!("{dir}-{n}"));
            let o = extract(&out, &format!("{dir}/{n}"));
            let stderr = String::from_utf8_lossy(&o.stderr);
            assert!(!stderr.contains("panicked"), "{dir}/{n}: {stderr}");
            assert!(matches!(o.status.code(), Some(0 | 255)), "{dir}/{n}");
            if o.status.code() == Some(255) {
                assert_eq!(stderr, "ERROR processing file\n", "{dir}/{n}");
            }
        }
    }
}

#[test]
fn arguments() {
    let o = ytnef(&[]);
    assert_eq!(o.status.code(), Some(255));
    assert!(o
        .stdout
        .starts_with(b"You must specify files to parse\nYerase TNEF Exporter"));

    let o = ytnef(&["-s".as_ref(), "64".as_ref(), "-h".as_ref()]);
    assert_eq!(o.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&o.stdout).contains("Attachment Size (64 MB)"));

    for (args, msg) in [
        (&["-f"][..], "Option '-f' requires a directory\n"),
        (&["-s"], "Option '-s' requires a size\n"),
        (&["-s", "0"], "Option '-s' must be between 1 and 2047\n"),
        (&["-s", "2048"], "Option '-s' must be between 1 and 2047\n"),
    ] {
        let args: Vec<&std::ffi::OsStr> = args.iter().map(|a| a.as_ref()).collect();
        let o = ytnef(&args);
        assert_eq!(o.status.code(), Some(255));
        assert_eq!(String::from_utf8_lossy(&o.stdout), msg);
    }

    let o = ytnef(&["-x".as_ref()]);
    assert_eq!(o.status.code(), Some(0));
    assert!(o
        .stdout
        .starts_with(b"Unknown option '-x'\nYerase TNEF Exporter"));

    // without -f nothing is saved, and a missing file is one failure
    let f = fixtures().join("winmail.dat");
    let missing = fixtures().join("no-such-file");
    let o = ytnef(&[f.as_os_str(), missing.as_os_str(), missing.as_os_str()]);
    assert_eq!(o.status.code(), Some(254));
    assert!(o.stdout.is_empty());
}
