#![no_main]

//! A MAPI property block, wrapped in a message-level `attMAPIProps` and in an
//! attachment's, so it reaches `mapi::parse` and everything reading props.

use libfuzzer_sys::fuzz_target;
use ytnef_core::attr::id;
use ytnef_core::{parse_with, Checksum, Limits, Options};
use ytnef_export::print;

fn attr(out: &mut Vec<u8>, level: u8, id: u32, data: &[u8]) {
    out.push(level);
    out.extend_from_slice(&id.to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(data);
    let sum = data.iter().fold(0u16, |s, &b| s.wrapping_add(b.into()));
    out.extend_from_slice(&sum.to_le_bytes());
}

fuzz_target!(|data: &[u8]| {
    let mut tnef = 0x223E_9F78u32.to_le_bytes().to_vec();
    tnef.extend_from_slice(&[0, 0]);
    attr(&mut tnef, 1, id::MAPI_PROPS, data);
    attr(&mut tnef, 2, id::ATTACH_RENDDATA, &[0; 14]);
    attr(&mut tnef, 2, id::ATTACHMENT, data);
    let opts = Options {
        limits: Limits {
            max_rtf_out: 1 << 20,
            ..Limits::default()
        },
        checksum: Checksum::Lenient,
    };
    if let Ok(t) = parse_with(&tnef, &opts) {
        assert!(t.mapi.len() <= opts.limits.max_mapi_count);
        for p in t.mapi.iter() {
            assert!(p.values.len() <= opts.limits.max_mapi_count);
            for v in &p.values {
                assert!(v.len() <= opts.limits.max_mapi_value);
            }
        }
        let mut sink = Vec::new();
        print::mapi_print(&mut sink, &t.mapi).unwrap();
        print::print_tnef(&mut sink, &t, 2).unwrap();
        let _ = print::subject(&t);
    }
});
