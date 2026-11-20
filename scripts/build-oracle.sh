#!/bin/sh
# Build the C ytnef reference ("oracle") into target/oracle:
#   target/oracle/tnefdump    attribute-level dump (tests/oracle/tnefdump.c)
#   target/oracle/ytnef       C CLI
#   target/oracle/ytnefprint  C CLI
#
# YTNEF_ORACLE_REPO  git URL or local path (default: GitHub upstream)
# YTNEF_ORACLE_REV   commit to build (default: the commit tests/fixtures came from)
# YTNEF_ORACLE_ASAN  set to 1 for an ASan+UBSan build in target/oracle-asan
#                    (differential runs over fuzz corpora, where C may crash)
#
# Needs: git, a C compiler, autoconf, automake, libtool, make.
set -eu

REPO=${YTNEF_ORACLE_REPO:-https://github.com/Yeraze/ytnef}
REV=${YTNEF_ORACLE_REV:-b976ad9f6c542634dd4e1d979817cdeed5ea9732}
ROOT=$(cd "$(dirname "$0")/.." && pwd)
OUT=$ROOT/target/oracle
SAN=
if [ "${YTNEF_ORACLE_ASAN:-}" = 1 ]; then
    OUT=$ROOT/target/oracle-asan
    SAN="-fsanitize=address,undefined -fno-omit-frame-pointer -g"
fi
SRC=$OUT/src

rm -rf "$SRC"
mkdir -p "$OUT"
git init -q "$SRC"
git -C "$SRC" fetch -q --depth 1 "$REPO" "$REV"
git -C "$SRC" -c advice.detachedHead=false checkout -q FETCH_HEAD

cd "$SRC"
{
    autoreconf -fi &&
        if [ -n "$SAN" ]; then
            ./configure --disable-shared CFLAGS="-O1 $SAN" LDFLAGS="$SAN"
        else
            ./configure --disable-shared
        fi &&
        make -j"$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 2)"
} >"$OUT/build.log" 2>&1 || {
    cat "$OUT/build.log"
    exit 1
}

${CC:-cc} -O1 $SAN -Wall -I lib -o "$OUT/tnefdump" "$ROOT/tests/oracle/tnefdump.c" lib/.libs/libytnef.a
cp ytnef/ytnef ytnefprint/ytnefprint "$OUT/"
echo "oracle built from $REV: $OUT"
