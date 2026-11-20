# Test fixtures

Copied from [Yeraze/ytnef](https://github.com/Yeraze/ytnef) `test-data/` at commit
`b976ad9` (v2.1.4 + parse-error fix), same project and license (GPL-2.0-or-later).

- `*.tnef`, `winmail.dat`: sample TNEF streams used by the C `make check`.
- `regress/`: crafted streams for past crash/security bugs.
- `ytnef-oob-TNEFVersion-SwapWord`: CVE PoC (out-of-bounds read).
- `results/`: files `ytnef -f` must extract, byte-identical.
- `test.sh`: the C test script, kept for reference.

The C unit tests (`test_lib.c`, `test_decompress_rtf.c`) were not copied; their cases
will be ported as Rust tests.
