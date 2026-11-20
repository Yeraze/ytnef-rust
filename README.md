# ytnef-rust

Rust port of [ytnef](https://github.com/Yeraze/ytnef): a parser for Microsoft TNEF
(`winmail.dat`, `application/ms-tnef`) streams, plus the `ytnef` and `ytnefprint` tools.

**Status: pre-alpha / in progress.** `ytnefprint` and `ytnef` (attachments, RTF/HTML
bodies, vCard/vCalendar/vTask export) match the C tools on the test corpus.
Use the C ytnef for real work.

## Why

The C ytnef has 29 published CVEs; most are memory-safety bugs. A safe-Rust core turns
those into returned errors, and explicit limits cover the rest (path traversal, unbounded
loops, huge allocations). Goal: same output as C ytnef 2.1.x, byte for byte, except for a
short, documented list of bug fixes.

## Relation to C ytnef

- Derived from C ytnef, same license (GPL-2.0-or-later).
- C ytnef stays the reference oracle: we diff outputs against it on a shared corpus.
- `tests/fixtures/` is copied from the C repo's `test-data/` (see its README).
- `crates/ytnef-capi` builds a drop-in `libytnef.so.0`: same SONAME, exported symbols, struct layouts and headers, so C programs (Evolution, Claws Mail, ...) link against it unchanged.

## Layout

| Crate | What |
|---|---|
| `crates/ytnef-core` | Parser library. `#![forbid(unsafe_code)]`, no I/O, no printing |
| `crates/ytnef-export` | vCard / VTODO / VEVENT / MAPI-dump formatters |
| `crates/ytnef-cli` | `ytnef` and `ytnefprint` binaries |
| `crates/ytnef-capi` | C ABI `libytnef.so.0`; the only crate with `unsafe` |

## Build

Needs Rust 1.85 or newer.

```sh
cargo build --release
cargo test
```

Binaries land in `target/release/`.

Install the C library (`libytnef.so.0`, `libytnef.a`, `libytnef.pc`, the
original headers) with [cargo-c](https://github.com/lu-zero/cargo-c):

```sh
cargo cinstall -p ytnef-capi --release --prefix /usr --destdir "$DESTDIR"
```

`scripts/capi-link-test.sh` links the C `ytnef`, `ytnefprint` and
`tnefdump` against it and diffs them with the C library on every fixture.

## License

GPL-2.0-or-later. See [LICENSE](LICENSE).
