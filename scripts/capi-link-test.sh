#!/bin/sh
# Link the unchanged C programs (tnefdump, ytnef, ytnefprint) against the
# Rust libytnef.so.0 and diff their output with the C oracle on every fixture.
#
#   scripts/build-oracle.sh                 # C sources + reference binaries
#   cargo build --release -p ytnef-capi
#   scripts/capi-link-test.sh
#
# YTNEF_CAPI_ASAN=1  build the C programs with ASan+UBSan (the Rust library
#                    allocates with libc malloc, so ASan sees every buffer it
#                    hands out and every free() the C side does)
#
# Also checks the SONAME and that the exported symbols match the Debian
# libytnef0.symbols list. Exits non-zero on any difference.
set -eu
export LC_ALL=C

ROOT=$(cd "$(dirname "$0")/.." && pwd)
ORACLE=$ROOT/target/oracle
SRC=$ORACLE/src
LIB=$ROOT/target/release/libytnef.so
OUT=$ROOT/target/capi
SAN=
if [ "${YTNEF_CAPI_ASAN:-}" = 1 ]; then
    SAN="-fsanitize=address,undefined -fno-omit-frame-pointer -g"
    export ASAN_OPTIONS=detect_leaks=1:abort_on_error=1
    export UBSAN_OPTIONS=halt_on_error=1:print_stacktrace=1
fi

[ -x "$ORACLE/ytnef" ] || { echo "run scripts/build-oracle.sh first" >&2; exit 2; }
[ -f "$LIB" ] || { echo "run cargo build --release -p ytnef-capi first" >&2; exit 2; }

rm -rf "$OUT"
mkdir -p "$OUT/lib" "$OUT/bin"
cp "$LIB" "$OUT/lib/libytnef.so.0"
ln -s libytnef.so.0 "$OUT/lib/libytnef.so"

fail=0

# --- SONAME and symbols -----------------------------------------------------
soname=$(readelf -d "$OUT/lib/libytnef.so.0" | sed -n 's/.*Library soname: \[\(.*\)\]/\1/p')
if [ "$soname" != libytnef.so.0 ]; then
    echo "FAIL soname: '$soname'"
    fail=1
fi
nm -D --defined-only "$OUT/lib/libytnef.so.0" | awk '{print $3}' | sort >"$OUT/exported.txt"
sed -n 's/^ \([A-Za-z_0-9]*\)@Base .*/\1/p' "$ROOT/crates/ytnef-capi/libytnef0.symbols" | sort >"$OUT/debian.txt"
missing=$(comm -23 "$OUT/debian.txt" "$OUT/exported.txt")
if [ -n "$missing" ]; then
    echo "FAIL symbols missing from libytnef.so.0:" $missing
    fail=1
fi
extra=$(comm -13 "$OUT/debian.txt" "$OUT/exported.txt" | grep -v '^MAPISysTimetoDTRSize$' || true)
if [ -n "$extra" ]; then
    echo "FAIL unexpected exported symbols:" $extra
    fail=1
fi

# --- build the C programs against the Rust library --------------------------
CC=${CC:-cc}
# shellcheck disable=SC2086
{
    $CC -O1 $SAN -w -I "$SRC" -I "$SRC/lib" -o "$OUT/bin/ytnef" "$SRC/ytnef/main.c" -L "$OUT/lib" -lytnef
    $CC -O1 $SAN -w -I "$SRC" -I "$SRC/lib" -o "$OUT/bin/ytnefprint" "$SRC/ytnefprint/main.c" -L "$OUT/lib" -lytnef
    $CC -O1 $SAN -Wall -I "$ROOT/crates/ytnef-capi/include" -o "$OUT/bin/tnefdump" "$ROOT/tests/oracle/tnefdump.c" -L "$OUT/lib" -lytnef
}
export LD_LIBRARY_PATH="$OUT/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
if ! ldd "$OUT/bin/ytnef" | grep -q "$OUT/lib/libytnef.so.0"; then
    echo "FAIL ytnef does not load the Rust libytnef.so.0"
    fail=1
fi

# --- differential -----------------------------------------------------------
# run <dir> <bin> <args...>: stdout+stderr, exit code, and any files written
run() {
    dir=$1
    shift
    rm -rf "$dir"
    mkdir -p "$dir/files"
    rc=0
    (cd "$dir/files" && "$@") >"$dir/out" 2>&1 || rc=$?
    echo "rc=$rc" >>"$dir/out"
    # diagnostics name the source location that raised them
    sed -i 's/ at [^ ]* : [0-9]*\([,]\{0,1\}\)/ at <src>\1/' "$dir/out"
    (cd "$dir/files" && find . -type f | sort | while read -r f; do
        echo "$f $(cksum <"$f")"
    done) >>"$dir/out"
}

checks=0
cd "$ROOT/tests/fixtures"
for fx in $(find . -type f ! -name '*.md' ! -name '*.sh' ! -path './results/*' | sed 's|^\./||' | sort); do
    in=$ROOT/tests/fixtures/$fx
    for check in tnefdump ytnefprint ytnefprint-v ytnef ytnef-v ytnef-all; do
        case $check in
            tnefdump) prog=tnefdump args= ;;
            ytnefprint) prog=ytnefprint args= ;;
            ytnefprint-v) prog=ytnefprint args=-v ;;
            ytnef) prog=ytnef args="-f ." ;;
            ytnef-v) prog=ytnef args="-v -f ." ;;
            ytnef-all) prog=ytnef args="-a -F -H -f ." ;;
        esac
        # shellcheck disable=SC2086
        run "$OUT/c" "$ORACLE/$prog" $args "$in"
        # shellcheck disable=SC2086
        run "$OUT/r" "$OUT/bin/$prog" $args "$in"
        checks=$((checks + 1))
        if ! cmp -s "$OUT/c/out" "$OUT/r/out"; then
            echo "FAIL $check $fx"
            diff "$OUT/c/out" "$OUT/r/out" | head -10
            fail=1
        fi
    done
done

echo "capi link test: $checks checks, $(wc -l <"$OUT/exported.txt") exported symbols"
exit $fail
