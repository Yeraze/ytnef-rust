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

fn esc(k: &str, b: &[u8]) -> String {
    let mut s = format!("{k}={}:", b.len());
    for &c in b {
        if (0x20..0x7f).contains(&c) && c != b'\\' {
            s.push(char::from(c));
        } else {
            s += &format!("\\x{c:02x}");
        }
    }
    s + "\n"
}

/// C `char[cap]` field filled by `memcpy(min(len, copy))`, read with `strnlen`.
fn fixed(k: &str, b: Option<&[u8]>, copy: usize) -> String {
    let b = b.unwrap_or_default();
    let b = &b[..b.len().min(copy)];
    let b = &b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())];
    if b.is_empty() {
        String::new()
    } else {
        esc(k, b)
    }
}

fn opt(k: &str, b: Option<&[u8]>) -> String {
    b.map(|b| esc(k, b)).unwrap_or_default()
}

fn blob(k: &str, b: Option<&[u8]>) -> String {
    b.map(|b| format!("{k}={}:{:016x}\n", b.len(), fnv(b)))
        .unwrap_or_default()
}

fn date(k: &str, d: Option<ytnef_core::Dtr>) -> String {
    match d {
        Some(d) if d != ytnef_core::Dtr::default() => format!(
            "{k}={}-{}-{} {}:{}:{} dow{}\n",
            d.year, d.month, d.day, d.hour, d.minute, d.second, d.day_of_week
        ),
        _ => String::new(),
    }
}

/// The Rust parse rendered in `tests/oracle/tnefdump.c`'s format.
fn dump(buf: &[u8]) -> String {
    let t = match ytnef_core::parse(buf, &ytnef_core::Limits::default()) {
        Ok(t) => t,
        Err(e) => return format!("rc={}\n", e.code()),
    };
    let version = t
        .version
        .map(|(major, minor)| format!("TNEF{major}.{minor}"));
    let priority = t.priority.map(|p| match p {
        3 => "high",
        2 => "normal",
        1 => "low",
        _ => "N/A",
    });
    let mut s = String::from("rc=0\n");
    s += &opt("version", version.as_deref().map(str::as_bytes));
    s += &opt("from", t.from);
    s += &opt("subject", t.subject);
    s += &opt("body", t.body);
    s += &fixed("class", t.message_class, 49);
    s += &fixed("id", t.message_id, 50);
    s += &fixed("parent", t.parent_id, 50);
    s += &opt("priority", priority.map(str::as_bytes));
    s += &opt("codepage", t.oem_codepage);
    s += &opt("origclass", t.original_message_class);
    s += &date("sent", t.date_sent);
    s += &date("received", t.date_received);
    s += &date("modified", t.date_modified);
    s += &date("start", t.date_start);
    s += &date("end", t.date_end);
    let rtf_tag = ytnef_core::mapi::prop_tag(ytnef_core::mapi::pt::BINARY, 0x1009);
    if let Some(v) = t.mapi.get(rtf_tag).and_then(|p| p.values.first()) {
        s += &match ytnef_core::rtf::decompress_rtf(v, &ytnef_core::Limits::default()) {
            Ok(r) => format!("rtf={}:{:016x}\n", r.len(), fnv(&r)),
            Err(_) => "rtf=error\n".to_owned(),
        };
    }
    let head = std::iter::once(("head".to_owned(), &t.head));
    let rest = t.attachments.iter().enumerate();
    for (name, a) in head.chain(rest.map(|(i, a)| (format!("attachment {i}"), a))) {
        s += &format!("[{name}]\n");
        if a.render.atyp != 0 {
            s += &format!("atyp={}\n", a.render.atyp);
        }
        s += &opt("title", a.title);
        s += &blob("data", a.data);
        s += &blob("icon", a.icon);
        s += &date("create", a.create_date);
        s += &date("modify", a.modify_date);
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
                "dump",
                run(Command::new(oracle.join("tnefdump")).arg(&path)),
                format!("exit=Some(0)\n{}", dump(&fs::read(&path).unwrap())),
            ),
            (
                "ytnefprint",
                run(Command::new(oracle.join("ytnefprint")).arg(&path)),
                run(Command::new(rust_print).arg(&path)),
            ),
            (
                "ytnefprint-v",
                run(Command::new(oracle.join("ytnefprint")).arg("-v").arg(&path)),
                run(Command::new(rust_print).arg("-v").arg(&path)),
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
