use std::path::{Path, PathBuf};
use ytnef_core::{parse, parse_with, Checksum, Error, Limits, Options, Tnef};

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn fixture(name: &str) -> Vec<u8> {
    let p = dir().join(name);
    std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// Every TNEF input under tests/fixtures (skips docs and expected outputs).
fn corpus() -> Vec<(String, Vec<u8>)> {
    let mut v = Vec::new();
    for sub in ["", "regress", "poc"] {
        for e in std::fs::read_dir(dir().join(sub)).unwrap() {
            let p = e.unwrap().path();
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            if p.is_file() && !name.ends_with(".md") && !name.ends_with(".sh") {
                v.push((format!("{sub}/{name}"), std::fs::read(&p).unwrap()));
            }
        }
    }
    v.sort();
    v
}

fn ok(name: &str) -> Tnef<'static> {
    let buf = fixture(name).leak();
    parse(buf, &Limits::default()).unwrap_or_else(|e| panic!("{name}: {e}"))
}

#[test]
fn samples_parse() {
    for (name, attachments) in [
        ("one-file.tnef", 1),
        ("long-filename.tnef", 1),
        ("missing-filenames.tnef", 4),
        ("multi-value-attribute.tnef", 1),
        ("rtf.tnef", 0),
        ("winmail.dat", 2),
    ] {
        let t = ok(name);
        assert_eq!(t.attachments.len(), attachments, "{name}");
        assert_eq!(t.version, Some((1, 0)), "{name}");
        assert!(t.attributes.iter().all(|a| a.checksum_ok), "{name}");
    }
}

#[test]
fn one_file_contents() {
    let t = ok("one-file.tnef");
    assert_eq!(t.subject, Some(&b"one-file\0"[..]));
    assert_eq!(t.message_class, Some(&b"IPM.Microsoft Mail.Note\0"[..]));
    assert_eq!(t.priority, Some(2));
    let d = t.date_sent.unwrap();
    assert_eq!((d.year, d.month, d.day, d.hour), (1999, 10, 13, 22));
    let a = &t.attachments[0];
    assert_eq!(a.render.atyp, 1);
    assert_eq!(a.title, Some(&b"AUTHORS\0"[..]));
    assert_eq!(a.data, Some(&fixture("results/AUTHORS")[..]));
}

#[test]
fn bad_inputs_match_c_errors() {
    // C return codes from scripts/build-oracle.sh's tnefdump
    for (name, code) in [
        ("ytnef-oob-TNEFVersion-SwapWord", -3),
        ("regress/truncated-field.tnef", -3),
        ("regress/duplicate-attribute.tnef", -3),
        ("regress/duplicate-mapi.tnef", -5),
        ("poc/issue39-SwapWord-heapoverflow", -6),
        ("poc/issue40-DecompressRTF-heapoverflow", -5),
        ("poc/issue46-TNEFDateHandler-overrun", -6),
        ("poc/issue85-TNEFSubjectHandler-doublefree", -3),
        ("poc/issue86-TNEFVersion-overflow", -6),
        ("poc/issue90-double-free", -3),
    ] {
        let e = parse(&fixture(name), &Limits::default()).unwrap_err();
        assert_eq!(e.code(), code, "{name}: {e}");
    }
    assert_eq!(
        parse(&fixture("results/AUTHORS"), &Limits::default()),
        Err(Error::NotTnefStream)
    );
}

/// Every fixture, every prefix of the small ones, never panics.
#[test]
fn corpus_never_panics() {
    let l = Limits::default();
    for (name, buf) in corpus() {
        let _ = parse(&buf, &l);
        if buf.len() <= 8192 {
            for n in 0..buf.len() {
                let _ = parse(&buf[..n], &l);
            }
        }
        eprintln!("{name}: ok");
    }
}

/// Cheap deterministic mutation fuzz over the corpus: byte flips, random
/// u32 overwrites (hits size fields), truncation. Asserts no panic.
#[test]
fn mutated_corpus_never_panics() {
    let mut seed = 0x9E37_79B9_7F4A_7C15_u64;
    let mut rnd = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let l = Limits::default();
    let lenient = Options {
        checksum: Checksum::Lenient,
        ..Options::default()
    };
    for (_, orig) in corpus() {
        for _ in 0..500 {
            let mut buf = orig.clone();
            for _ in 0..1 + rnd() % 4 {
                let i = (rnd() as usize) % buf.len();
                match rnd() % 3 {
                    0 => buf[i] ^= 1 << (rnd() % 8),
                    1 => {
                        let end = (i + 4).min(buf.len());
                        let r = (rnd() as u32).to_le_bytes();
                        buf[i..end].copy_from_slice(&r[..end - i]);
                    }
                    _ => buf.truncate(i.max(6)),
                }
            }
            let _ = parse(&buf, &l);
            let _ = parse_with(&buf, &lenient);
        }
    }
}
