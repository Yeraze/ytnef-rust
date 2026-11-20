# Packaging notes

ytnef-rust replaces the C `ytnef` package: same binaries, man pages, library SONAME,
headers and pkg-config name. Package it as a replacement, not alongside.

## Install layout

| File | From |
|---|---|
| `/usr/bin/ytnef`, `/usr/bin/ytnefprint` | `cargo build --release --locked -p ytnef-cli` |
| `/usr/bin/ytnefprocess` | `contrib/ytnefprocess/ytnefprocess` (Perl; needs MIME-tools, MailTools) |
| `/usr/share/man/man1/ytnef.1`, `ytnefprint.1` | `crates/ytnef-cli/man/` |
| `/usr/share/man/man1/ytnefprocess.1` | `contrib/ytnefprocess/` |
| `/usr/lib*/libytnef.so.0*`, `libytnef.so`, `libytnef.a` | `cargo cinstall -p ytnef-capi` |
| `/usr/lib*/pkgconfig/libytnef.pc` | same (`Version: 2.1.4`) |
| `/usr/include/{ytnef,tnef-types,tnef-errors,mapi,mapidefs,mapitags}.h` | same |

Library build and stage:

```sh
cargo install cargo-c   # or the distro's cargo-c package
cargo cinstall -p ytnef-capi --release --locked \
    --prefix /usr --libdir /usr/lib64 --destdir "$DESTDIR"
```

Offline builds: `cargo vendor` the workspace; the only external crates are
`thiserror` and `libc`.

## Split packages

Mirror the C split so dependents keep working:

- `ytnef`: the three tools and their man pages.
- `libytnef0` / `libytnef`: `libytnef.so.0*`.
- `libytnef0-dev` / `libytnef-devel`: `libytnef.so`, headers, `libytnef.pc`
  (and `libytnef.a` if the distro ships static libs).

Each should `Provides`/`Replaces`/`Conflicts` (Debian) or `Provides`/`Obsoletes`
(RPM) the matching C package at version 2.1.4.

## RPM

[`rpm/ytnef-rust.spec`](rpm/ytnef-rust.spec) builds all three from the GitHub release
tarball. Fedora packagers will want `%cargo_*` macros and vendored or system crates
instead of the network build it uses.

## Debian

- Source package `ytnef-rust`; binary packages `ytnef`, `libytnef0`, `libytnef0-dev`
  with `Breaks`/`Replaces`/`Provides` on the C ones.
- Build with `dh-cargo` for the tools; call `cargo cinstall` from
  `override_dh_auto_install` for the library (`--libdir /usr/lib/$(DEB_HOST_MULTIARCH)`).
- `libytnef0.symbols`: start from `crates/ytnef-capi/libytnef0.symbols`; the CI checks
  the exported symbol list against it.
- The library is built with `overflow-checks = true` in release; keep it.

## Static binaries

Release tarballs carry `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl`
static binaries. To build one yourself:

```sh
rustup target add x86_64-unknown-linux-musl
cargo build --release --locked -p ytnef-cli --target x86_64-unknown-linux-musl
```
