# ytnef-rust

Rust port of [ytnef](https://github.com/Yeraze/ytnef): a parser for Microsoft TNEF
(`winmail.dat`, `application/ms-tnef`) streams, the `ytnef` and `ytnefprint` tools,
and a drop-in `libytnef.so.0`.

**Status: beta.** `ytnef`, `ytnefprint` and `libytnef.so.0` match C ytnef 2.1.4 on the
test corpus and on nightly fuzz corpora, except for the differences listed below.

## Why

The C ytnef has 29 published CVEs; most are memory-safety bugs. A safe-Rust core turns
those into returned errors, and explicit limits cover the rest (path traversal, unbounded
loops, huge allocations). Goal: same output as C ytnef 2.1.x, byte for byte, except for a
short, documented list of bug fixes.

## Crates

| Crate | What |
|---|---|
| [`ytnef-core`](crates/ytnef-core) | Parser library. `#![forbid(unsafe_code)]`, no I/O, no printing |
| [`ytnef-export`](crates/ytnef-export) | vCard / VTODO / VEVENT / MAPI-dump formatters |
| [`ytnef-cli`](crates/ytnef-cli) | `ytnef` and `ytnefprint` binaries, man pages |
| [`ytnef-capi`](crates/ytnef-capi) | C ABI `libytnef.so.0`; the only crate with `unsafe` |

`contrib/ytnefprocess` is the C project's procmail filter, unchanged; it calls `ytnef`.

## Install

Needs Rust 1.85 or newer.

**Tools** from crates.io:

```sh
cargo install ytnef-cli
```

or prebuilt binaries (static musl on Linux) from
[GitHub releases](https://github.com/Yeraze/ytnef-rust/releases). Each tarball holds
`ytnef`, `ytnefprint`, their man pages, `ytnefprocess`, `LICENSE` and this README.

**Library** in a Rust project:

```toml
[dependencies]
ytnef-core = "0.1"
```

```rust
let data = std::fs::read("winmail.dat")?;
let tnef = ytnef_core::parse(&data, &ytnef_core::Limits::default())?;
```

**From source:**

```sh
cargo build --release      # binaries in target/release/
cargo test --workspace
install -Dm644 crates/ytnef-cli/man/ytnef.1      /usr/local/share/man/man1/ytnef.1
install -Dm644 crates/ytnef-cli/man/ytnefprint.1 /usr/local/share/man/man1/ytnefprint.1
```

Distro packagers: see [packaging/](packaging/README.md).

## CLI usage

Same flags as the C tools; see `ytnef(1)` and `ytnefprint(1)`.

```sh
ytnef -v winmail.dat          # list contents, save nothing
ytnef -f out/ winmail.dat     # save attachments to out/
ytnef -F -H -f out/ winmail.dat   # also save the body as RTF and HTML
ytnefprint winmail.dat        # dump fields, attachments, MAPI properties
ytnefprint -s 100 big.dat     # raise the attachment size limit to 100 MB
```

## C ABI: drop-in libytnef

`ytnef-capi` builds `libytnef.so.0` with the C library's SONAME, exported symbols,
struct layouts and headers (`ytnef.h`, `tnef-types.h`, `tnef-errors.h`, `mapi*.h`), so
programs built against C libytnef (Evolution, Claws Mail, ...) run against it without a
rebuild. Install it with [cargo-c](https://github.com/lu-zero/cargo-c):

```sh
cargo install cargo-c
cargo cinstall -p ytnef-capi --release --prefix /usr --libdir /usr/lib --destdir "$DESTDIR"
```

This installs `libytnef.so.0`, `libytnef.a`, `libytnef.pc` (`Version: 2.1.4`) and the
headers. Build C programs as before:

```sh
cc -o tool tool.c $(pkg-config --cflags --libs libytnef)
```

`scripts/capi-link-test.sh` links the C `ytnef`, `ytnefprint` and `tnefdump` against
it and diffs them with the C library on every fixture, also under ASan/UBSan in CI.
As in C, release a parsed `TNEFStruct` with `TNEFFree`.

## Differences from C ytnef

C ytnef stays the reference: CI diffs both tools against it on every fixture, and
nightly against fuzz corpora. Each allowed difference is listed, with its reason, in
[`tests/differential-allowlist.txt`](tests/differential-allowlist.txt). In short:

- **Safe file names.** Output names are one path component; control bytes become `_`,
  leading dots are removed, names are cut to 255 bytes. Fixes Yeraze/ytnef#113.
- **No overwrites.** Files are created new; an existing file or symlink is never
  overwritten or followed. A clash adds a counter: `x.dat` → `x1.dat`.
- **Nesting limit.** Embedded TNEF nests at most 8 deep (C: 16).
- **Limits.** `ytnef_core::Limits` bounds input and attribute size (`-s`, default
  50 MB), attribute, attachment and MAPI property counts, decompressed RTF size and
  nesting depth; C trusts the stream.
- **Fuzz-found C bugs kept fixed:** `attRenddata` flags (C misreads them, so "MAC
  Binary" never prints) and RTF-to-text (C drops a stray `H`).
- **`ytnefprint -h` and `-s`** work; C ignores `-h` and reads the `-s` value as a file.
- **`-v -v`** does not reproduce the C library's internal parser trace.

## Relation to C ytnef

- Derived from C ytnef, same license (GPL-2.0-or-later).
- `tests/fixtures/` is copied from the C repo's `test-data/` (see its README).

## License

GPL-2.0-or-later. See [LICENSE](LICENSE).
