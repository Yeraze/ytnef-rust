use std::path::Path;

fn fixture(name: &str) -> Vec<u8> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures").join(name);
    std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

#[test]
fn corpus_has_tnef_signature() {
    for name in [
        "one-file.tnef",
        "long-filename.tnef",
        "missing-filenames.tnef",
        "multi-value-attribute.tnef",
        "rtf.tnef",
        "winmail.dat",
    ] {
        assert!(ytnef_core::has_signature(&fixture(name)), "{name}");
    }
}
