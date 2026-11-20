//! Differential test against the C ytnef ("oracle").
//!
//! ```sh
//! scripts/build-oracle.sh
//! YTNEF_ORACLE=target/oracle cargo test -p ytnef-cli --test differential -- --nocapture
//! ```
//!
//! Without `YTNEF_ORACLE` the test is skipped. Every fixture under
//! `tests/fixtures/` is run through each check; a mismatch fails unless
//! `tests/differential-allowlist.txt` lists it.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every fixture file, relative to `tests/fixtures`, skipping docs and expected outputs.
fn corpus() -> Vec<String> {
    fn walk(dir: &Path, base: &Path, out: &mut Vec<String>) {
        for e in fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            let rel = p.strip_prefix(base).unwrap().to_string_lossy().into_owned();
            if p.is_dir() {
                if rel != "results" {
                    walk(&p, base, out);
                }
            } else if !rel.ends_with(".md") && !rel.ends_with(".sh") {
                out.push(rel);
            }
        }
    }
    let base = repo().join("tests/fixtures");
    let mut v = Vec::new();
    walk(&base, &base, &mut v);
    v.sort();
    v
}

/// `(check, fixture pattern)` pairs; pattern is a path, a `dir/` prefix, or `*`.
struct Allowlist(Vec<(String, String)>);

impl Allowlist {
    fn load() -> Self {
        let text = fs::read_to_string(repo().join("tests/differential-allowlist.txt")).unwrap();
        Allowlist(
            text.lines()
                .map(|l| l.split('#').next().unwrap())
                .filter_map(|l| {
                    let mut w = l.split_whitespace();
                    Some((w.next()?.to_owned(), w.next()?.to_owned()))
                })
                .collect(),
        )
    }

    fn allows(&self, check: &str, fixture: &str) -> bool {
        self.0.iter().any(|(c, p)| {
            c == check
                && (p == "*"
                    || p == fixture
                    || (p.ends_with('/') && fixture.starts_with(p.as_str())))
        })
    }
}

fn fnv(b: &[u8]) -> u64 {
    b.iter().fold(0xcbf2_9ce4_8422_2325, |h, &x| {
        (h ^ u64::from(x)).wrapping_mul(0x100_0000_01b3)
    })
}

/// Exit code and stdout.
fn run(cmd: &mut Command) -> String {
    let o = cmd.output().unwrap();
    format!(
        "exit={:?}\n{}",
        o.status.code(),
        String::from_utf8_lossy(&o.stdout)
    )
}

/// `ytnef -f <dir>`: exit code, stdout (dir masked) and the written files.
fn extract(bin: &Path, fixture: &Path, out: &Path) -> String {
    let _ = fs::remove_dir_all(out);
    fs::create_dir_all(out).unwrap();
    let mut s = run(Command::new(bin).arg("-f").arg(out).arg(fixture));
    s = s.replace(&*out.to_string_lossy(), "OUT");
    let mut files: Vec<_> = fs::read_dir(out)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    files.sort();
    for f in files {
        let b = fs::read(&f).unwrap_or_default();
        s += &format!(
            "file {} {}:{:016x}\n",
            f.file_name().unwrap().to_string_lossy(),
            b.len(),
            fnv(&b)
        );
    }
    s
}

fn first_diff(a: &str, b: &str) -> String {
    let (mut la, mut lb) = (a.lines(), b.lines());
    for n in 1.. {
        match (la.next(), lb.next()) {
            (None, None) => break,
            (x, y) if x != y => {
                return format!("line {n}: C {x:?} / Rust {y:?}");
            }
            _ => {}
        }
    }
    String::new()
}

#[test]
fn differential() {
    // relative paths are from the repo root, not the crate dir cargo runs tests in
    let Some(oracle) = std::env::var_os("YTNEF_ORACLE").map(|p| repo().join(p)) else {
        eprintln!("YTNEF_ORACLE not set: differential test skipped");
        return;
    };
    let allow = Allowlist::load();
    let tmp = Path::new(env!("CARGO_TARGET_TMPDIR")).join("differential");
    let rust_print = Path::new(env!("CARGO_BIN_EXE_ytnefprint"));
    let rust_ytnef = Path::new(env!("CARGO_BIN_EXE_ytnef"));
    let (mut checks, mut allowed, mut failures) = (0, 0, Vec::new());

    for name in corpus() {
        let path = repo().join("tests/fixtures").join(&name);
        let out = |side: &str| tmp.join(side).join(name.replace('/', "_"));
        let results = [
            (
                "ytnefprint",
                run(Command::new(oracle.join("ytnefprint")).arg(&path)),
                run(Command::new(rust_print).arg(&path)),
            ),
            (
                "ytnef",
                extract(&oracle.join("ytnef"), &path, &out("c")),
                extract(rust_ytnef, &path, &out("rust")),
            ),
        ];
        for (check, c, rust) in results {
            checks += 1;
            let ok = c == rust;
            match (ok, allow.allows(check, &name)) {
                (true, true) => eprintln!("note: {check} {name} matches but is allowlisted"),
                (true, false) => {}
                (false, true) => allowed += 1,
                (false, false) => {
                    failures.push(format!("{check} {name}: {}", first_diff(&c, &rust)))
                }
            }
        }
    }
    eprintln!(
        "differential: {checks} checks, {allowed} allowlisted diffs, {} failures",
        failures.len()
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
