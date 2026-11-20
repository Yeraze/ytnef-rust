fn main() {
    // C consumers link against libytnef.so.0, as with the autotools build.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        println!("cargo:rustc-cdylib-link-arg=-Wl,-soname,libytnef.so.0");
    }
}
