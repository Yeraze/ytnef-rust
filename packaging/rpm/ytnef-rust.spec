# Builds ytnef-rust from the GitHub tag tarball. Network build: the %build
# step fetches crates. For distro builds, vendor them (cargo vendor) instead.
%global capi_ver 2.1.4

Name:           ytnef-rust
Version:        0.1.0
Release:        1%{?dist}
Summary:        TNEF (winmail.dat) stream reader, Rust port of ytnef
License:        GPL-2.0-or-later
URL:            https://github.com/Yeraze/ytnef-rust
Source0:        %{url}/archive/v%{version}/%{name}-%{version}.tar.gz

BuildRequires:  cargo >= 1.85
BuildRequires:  rust >= 1.85
BuildRequires:  cargo-c
BuildRequires:  gcc

Provides:       ytnef = %{capi_ver}
Obsoletes:      ytnef < %{capi_ver}-100
Requires:       libytnef%{?_isa} = %{version}-%{release}
# ytnefprocess
Requires:       perl(MIME::Parser)
Requires:       perl(Mail::Mailer)

%description
Tools to unpack Microsoft TNEF streams (winmail.dat, application/ms-tnef):
ytnef extracts attachments and bodies, ytnefprint dumps their contents.
A memory-safe Rust port of ytnef with the same flags and output.

%package -n libytnef
Summary:        Drop-in libytnef.so.0 backed by a safe Rust TNEF parser
Provides:       libytnef = %{capi_ver}
Obsoletes:      libytnef < %{capi_ver}-100

%description -n libytnef
Binary-compatible replacement for C libytnef (same SONAME, symbols and
struct layouts), built from the Rust ytnef-core parser.

%package -n libytnef-devel
Summary:        Headers and pkg-config file for libytnef
Requires:       libytnef%{?_isa} = %{version}-%{release}
Provides:       libytnef-devel = %{capi_ver}
Obsoletes:      libytnef-devel < %{capi_ver}-100

%description -n libytnef-devel
C headers, libytnef.so and libytnef.pc for building against libytnef.

%prep
%autosetup

%build
cargo build --release --locked -p ytnef-cli

%install
install -Dm755 target/release/ytnef      %{buildroot}%{_bindir}/ytnef
install -Dm755 target/release/ytnefprint %{buildroot}%{_bindir}/ytnefprint
install -Dm755 contrib/ytnefprocess/ytnefprocess %{buildroot}%{_bindir}/ytnefprocess
install -Dm644 -t %{buildroot}%{_mandir}/man1 \
    crates/ytnef-cli/man/ytnef.1 crates/ytnef-cli/man/ytnefprint.1 \
    contrib/ytnefprocess/ytnefprocess.1
cargo cinstall -p ytnef-capi --release --locked \
    --prefix %{_prefix} --libdir %{_libdir} --includedir %{_includedir} \
    --destdir %{buildroot}
rm -f %{buildroot}%{_libdir}/libytnef.a

%check
cargo test --workspace --release --locked

%files
%license LICENSE
%doc README.md CHANGELOG.md contrib/ytnefprocess/README.ytnefprocess
%{_bindir}/ytnef
%{_bindir}/ytnefprint
%{_bindir}/ytnefprocess
%{_mandir}/man1/ytnef.1*
%{_mandir}/man1/ytnefprint.1*
%{_mandir}/man1/ytnefprocess.1*

%files -n libytnef
%license LICENSE
%{_libdir}/libytnef.so.0*

%files -n libytnef-devel
%{_includedir}/*.h
%{_libdir}/libytnef.so
%{_libdir}/pkgconfig/libytnef.pc

%changelog
* Fri Nov 20 2026 Randall Hand - 0.1.0-1
- First release
