//! `ytnefprint` checks that need no C oracle: the `ytnefprint` lines of C
//! `test.sh`, and clean failure on every malformed fixture.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

fn ytnefprint(args: &[&str], file: Option<&str>) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_ytnefprint"));
    c.args(args);
    if let Some(f) = file {
        c.arg(fixture(f));
    }
    c.output().unwrap()
}

fn stdout(file: &str) -> String {
    let o = ytnefprint(&[], Some(file));
    assert_eq!(o.status.code(), Some(0));
    String::from_utf8_lossy(&o.stdout).into_owned()
}

/// `grep -A <after> <key> | grep <want>`
fn grep_after(out: &str, key: &str, after: usize, want: &str) -> bool {
    let lines: Vec<&str> = out.lines().collect();
    lines.iter().enumerate().any(|(i, l)| {
        l.contains(key)
            && lines[i..lines.len().min(i + after + 1)]
                .iter()
                .any(|l| l.contains(want))
    })
}

#[test]
fn c_test_sh_checks() {
    let w = stdout("winmail.dat");
    assert!(grep_after(
        &w,
        "PR_MESSAGE_DELIVERY_TIME",
        1,
        "Tuesday June 17, 2003 3:23:00 pm"
    ));
    assert!(grep_after(
        &w,
        "PR_TNEF_CORRELATION_KEY",
        1,
        "[00000000CFF233DD7623274EB78A4912C247A0E464532700.]"
    ));
    assert!(grep_after(&w, "PR_RTF_SYNC_BODY_CRC", 1, "872404792"));
    assert!(grep_after(&w, "PR_RTF_SYNC_BODY_COUNT", 1, "90"));
    assert!(grep_after(
        &w,
        "PR_RTF_COMPRESSED",
        18,
        r"\pard Casdasdfasdfasd\par"
    ));
    assert!(stdout("ytnef-oob-TNEFVersion-SwapWord").contains("ERROR: Field with size of 0"));
    for f in [
        "regress/empty-string-prop.tnef",
        "regress/duplicate-attribute.tnef",
        "regress/early-attachment-attr.tnef",
        "regress/duplicate-mapi.tnef",
    ] {
        stdout(f);
    }
}

/// Every fixture, PoC crashers included, exits 0 with no panic, as C does.
#[test]
fn every_fixture_exits_cleanly() {
    let dir = fixture("poc");
    for e in std::fs::read_dir(&dir).unwrap() {
        let p = e.unwrap().path();
        for args in [&[][..], &["-v"]] {
            let o = Command::new(env!("CARGO_BIN_EXE_ytnefprint"))
                .args(args)
                .arg(&p)
                .output()
                .unwrap();
            assert_eq!(o.status.code(), Some(0), "{}", p.display());
            assert!(o.stderr.is_empty(), "{}", p.display());
        }
    }
}

#[test]
fn usage() {
    let o = ytnefprint(&[], None);
    assert_eq!(o.status.code(), Some(255));
    assert!(o.stdout.starts_with(b"You must specify files to parse\n"));

    let o = ytnefprint(&["-h"], None);
    assert_eq!(o.status.code(), Some(0));
    assert!(o.stdout.starts_with(b"Yerase TNEF Printer"));

    // C prints this for a file it cannot open
    let o = ytnefprint(&[], Some("does-not-exist"));
    assert_eq!(o.stdout, b"ERROR processing file\n");
}

/// `-s` caps the attribute size, in MB (C `ytnef` semantics).
#[test]
fn size_limit() {
    let o = ytnefprint(&["-s", "0"], Some("winmail.dat"));
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(
        out.starts_with("ERROR: invalid alloc size 4 at ytnef.c : 1240"),
        "{out}"
    );
    assert!(out.ends_with("ERROR processing file\n"));
}
