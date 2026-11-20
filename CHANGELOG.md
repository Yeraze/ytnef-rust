# Changelog

All notable changes. Versions follow [SemVer](https://semver.org/); the C ABI in
`ytnef-capi` tracks C libytnef 2.1.4 (`libytnef.so.0`) separately.

## [Unreleased]

## [0.1.0] - unreleased

First release.

### Added

- `ytnef-core`: safe, bounded TNEF parser (attributes, attachments, MAPI properties,
  compressed RTF, embedded TNEF) with `Limits` on every allocation and loop.
- `ytnef-export`: vCard, VTODO and VEVENT (with RRULE and RTF text) export, and the
  `ytnefprint` / MAPI dump formatter.
- `ytnef-cli`: `ytnef` and `ytnefprint`, matching the C tools' flags and output, with
  man pages.
- `ytnef-capi`: drop-in `libytnef.so.0` (same SONAME, symbols, struct layouts and
  headers) built with cargo-c; `libytnef.pc` reports `Version: 2.1.4`.
- `contrib/ytnefprocess`: the C project's procmail filter, unchanged.
- Fuzz targets, nightly fuzzing, and differential tests against C ytnef on fixtures
  and fuzz corpora.
- Release workflow: static musl, macOS and Windows binaries on tag push.

### Differences from C ytnef 2.1.4

- Output file names are one safe path component (fixes Yeraze/ytnef#113).
- Existing files and symlinks are never overwritten; clashes get a counter.
- Embedded TNEF nests at most 8 deep (C: 16).
- `attRenddata` flags and RTF-to-text stray-`H` bugs fixed.
- `ytnefprint` honours `-h` and `-s`.

Full list: `tests/differential-allowlist.txt`.

[Unreleased]: https://github.com/Yeraze/ytnef-rust/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/Yeraze/ytnef-rust/releases/tag/v0.1.0
