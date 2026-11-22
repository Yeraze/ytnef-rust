# Fixture manifest

Origin and license of every file under `tests/fixtures/`. Add a row for each new fixture.

`ytnef@b976ad9` = [Yeraze/ytnef](https://github.com/Yeraze/ytnef) `test-data/` at commit
`b976ad9f6c542634dd4e1d979817cdeed5ea9732`, GPL-2.0-or-later (same project and license).

Files in `poc/` are crash reproducers attached to Yeraze/ytnef issues (or linked from
them). Their authors stated no license; they are kept unmodified, used only as test
input, and several are fuzzer-mutated copies of the ytnef `test-data/` samples.

| File | Origin | License |
|---|---|---|
| `long-filename.tnef` | ytnef@b976ad9 | GPL-2.0-or-later |
| `missing-filenames.tnef` | ytnef@b976ad9 | GPL-2.0-or-later |
| `multi-value-attribute.tnef` | ytnef@b976ad9 | GPL-2.0-or-later |
| `one-file.tnef` | ytnef@b976ad9 | GPL-2.0-or-later |
| `rtf.tnef` | ytnef@b976ad9 | GPL-2.0-or-later |
| `winmail.dat` | ytnef@b976ad9 | GPL-2.0-or-later |
| `ytnef-oob-TNEFVersion-SwapWord` | ytnef@b976ad9; reported in [ytnef#33](https://github.com/Yeraze/ytnef/issues/33) | GPL-2.0-or-later |
| `regress/*.tnef` (8 files) | ytnef@b976ad9 `test-data/regress/` | GPL-2.0-or-later |
| `results/*` | ytnef@b976ad9 `test-data/results/` (expected `ytnef -f` output) | GPL-2.0-or-later |
| `test.sh` | ytnef@b976ad9 `test-data/test.sh` | GPL-2.0-or-later |
| `export/*.tnef` (16 files) | synthetic, written by `scripts/gen-export-fixtures.py` | GPL-2.0-or-later |
| `fuzz/*.tnef` (11 files) | cargo-fuzz `parse` corpus (mutated fixtures above) and one hand-built block, each a fixed or allowlisted C divergence (#8) | GPL-2.0-or-later |
| `poc/issue34-DecompressRTF` | [ytnef#34](https://github.com/Yeraze/ytnef/issues/34) attachment `ytnef-DecompressRTF.zip` | none stated |
| `poc/issue37-MAPIPrint-nullptr` | [ytnef#37](https://github.com/Yeraze/ytnef/issues/37), [asarubbo/poc 00241](https://github.com/asarubbo/poc/blob/master/00241-ytnef-nullptr-MAPIPrint) | none stated |
| `poc/issue38-PrintTNEF-heapoverflow` | [ytnef#38](https://github.com/Yeraze/ytnef/issues/38), asarubbo/poc 00242 | none stated |
| `poc/issue39-SwapWord-heapoverflow` | [ytnef#39](https://github.com/Yeraze/ytnef/issues/39), asarubbo/poc 00243 | none stated |
| `poc/issue40-DecompressRTF-heapoverflow` | [ytnef#40](https://github.com/Yeraze/ytnef/issues/40), asarubbo/poc 00244 | none stated |
| `poc/issue41-SwapDWord-heapoverflow` | [ytnef#41](https://github.com/Yeraze/ytnef/issues/41), asarubbo/poc 00245 | none stated |
| `poc/issue42-TNEFFillMapi-alloc` | [ytnef#42](https://github.com/Yeraze/ytnef/issues/42), asarubbo/poc 00246 | none stated |
| `poc/issue45-SIZECHECK-overflow` | [ytnef#45](https://github.com/Yeraze/ytnef/issues/45), [bingosxs/fuzzdata](https://github.com/bingosxs/fuzzdata/tree/master/ytnef-1.9) `TNEFFreeMapiProps-Invalid-read.tnef` | none stated |
| `poc/issue46-TNEFDateHandler-overrun` | [ytnef#46](https://github.com/Yeraze/ytnef/issues/46), bingosxs/fuzzdata `TNEFDateHandler_overrun.tnef` | none stated |
| `poc/issue47-TNEFFillMapi-overwrite` | [ytnef#47](https://github.com/Yeraze/ytnef/issues/47), bingosxs/fuzzdata `TNEFFillMapi-over-write.tnef` | none stated |
| `poc/issue48-TNEFDefaultHandler-overread` | [ytnef#48](https://github.com/Yeraze/ytnef/issues/48), bingosxs/fuzzdata `18-TNEFDefaultHandler.tnef` | none stated |
| `poc/issue49-SwapDWord-invalid-read` | [ytnef#49](https://github.com/Yeraze/ytnef/issues/49), [bestshow/p0cs](https://github.com/bestshow/p0cs) | none stated |
| `poc/issue50-TNEFFillMapi-overflow` | [ytnef#50](https://github.com/Yeraze/ytnef/issues/50), bestshow/p0cs | none stated |
| `poc/issue51-TNEFFillMapi-alloc` | [ytnef#51](https://github.com/Yeraze/ytnef/issues/51), bestshow/p0cs | none stated |
| `poc/issue52-PrintTNEF-overflow` | [ytnef#52](https://github.com/Yeraze/ytnef/issues/52), bestshow/p0cs | none stated |
| `poc/issue65-TNEFFreeMapiProps-segv` | [ytnef#65](https://github.com/Yeraze/ytnef/issues/65) `crash0.zip` | none stated |
| `poc/issue66-TNEFFreeMapiProps-segv` | [ytnef#66](https://github.com/Yeraze/ytnef/issues/66) `crash1.zip` | none stated |
| `poc/issue67-MAPIFindUserProp-segv` | [ytnef#67](https://github.com/Yeraze/ytnef/issues/67) `crash1_1.zip` | none stated |
| `poc/issue68-TNEFFreeMapiProps-abrt` | [ytnef#68](https://github.com/Yeraze/ytnef/issues/68) `crash2.zip` | none stated |
| `poc/issue69-TNEFFreeMapiProps-oom` | [ytnef#69](https://github.com/Yeraze/ytnef/issues/69) `crash3.zip` | none stated |
| `poc/issue70-TNEFFreeMapiProps-segv` | [ytnef#70](https://github.com/Yeraze/ytnef/issues/70) `crash4.zip` | none stated |
| `poc/issue85-TNEFSubjectHandler-doublefree` | [ytnef#85](https://github.com/Yeraze/ytnef/issues/85) `doublefree.zip` (`doublefree.dat`) | none stated |
| `poc/issue85-TNEFSubjectHandler-doublefree-min` | ytnef#85 `doublefree.zip` (`doublefree_min.dat`) | none stated |
| `poc/issue86-TNEFVersion-overflow` | [ytnef#86](https://github.com/Yeraze/ytnef/issues/86) `heap.zip` (`heap_min.dat`) | none stated |
| `poc/issue90-double-free` | [ytnef#90](https://github.com/Yeraze/ytnef/issues/90) `ytnef-double_free1.zip` | none stated |
| `poc/issue92-ProcessTNEF-oob` | [ytnef#92](https://github.com/Yeraze/ytnef/issues/92) `oob_poc.zip` | none stated |

Not collected: `winmail-2010-2016.zip` (from [ytnef#44](https://github.com/Yeraze/ytnef/issues/44)) is real mail with no
stated license; ask the owner before adding.
