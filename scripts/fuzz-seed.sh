#!/bin/sh
# Seed fuzz/corpus/<target> with the TNEF fixtures (parse, cli_process).
# cli_process inputs start with a flags byte; 0x1f turns every option on.
set -eu
ROOT=$(cd "$(dirname "$0")/.." && pwd)
for t in "$@"; do
    case $t in parse | cli_process) ;; *) continue ;; esac
    mkdir -p "$ROOT/fuzz/corpus/$t"
    find "$ROOT/tests/fixtures" -type f ! -name '*.md' ! -name '*.sh' ! -path '*/results/*' |
        while read -r f; do
            out=$ROOT/fuzz/corpus/$t/seed-$(sha1sum "$f" | cut -c1-16)
            if [ "$t" = cli_process ]; then
                { printf '\037'; cat "$f"; } >"$out"
            else
                cp "$f" "$out"
            fi
        done
done
