#![no_main]

use libfuzzer_sys::fuzz_target;
use ytnef_core::{rtf, Limits};

fuzz_target!(|data: &[u8]| {
    let limits = Limits {
        max_rtf_out: 1 << 20,
        ..Limits::default()
    };
    if let Ok(out) = rtf::decompress_rtf(data, &limits) {
        assert!(out.len() <= limits.max_rtf_out);
    }
    let _ = rtf::crc_matches(data);
});
