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
- `poc/`: crash reproducers from Yeraze/ytnef issues #34–#92.
- `export/`: small synthetic contact, task and calendar messages (`IPM.Contact`,
  `IPM.Task`, `IPM.Appointment`, meeting request / cancel, one per recurrence pattern)
  for the vCard / VTODO / VEVENT export. All names, addresses and dates are made up.
  Regenerate with `scripts/gen-export-fixtures.py`; edit the script, not the files.

`MANIFEST.md` lists the origin and license of every file.
