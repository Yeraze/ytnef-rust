#![no_main]

use libfuzzer_sys::fuzz_target;
use ytnef_core::{parse_with, Checksum, Limits, Options};
use ytnef_export::{print, vcal, vcard};

fuzz_target!(|data: &[u8]| {
    let limits = Limits {
        max_rtf_out: 1 << 20,
        ..Limits::default()
    };
    for checksum in [Checksum::Strict, Checksum::Lenient] {
        let Ok(t) = parse_with(data, &Options { limits, checksum }) else {
            continue;
        };
        assert!(t.attributes.len() <= limits.max_attributes);
        assert!(t.attachments.len() <= limits.max_attachments);
        assert!(t.mapi.len() <= limits.max_mapi_count);
        for a in &t.attributes {
            assert!(a.data.len() <= limits.max_attr_size);
        }
        let mut sink = Vec::new();
        for verbose in 0..3 {
            sink.clear();
            print::print_tnef(&mut sink, &t, verbose).unwrap();
        }
        let _ = vcard::vcard(&t);
        let _ = vcal::vtask(&t);
        let (_, _) = vcal::vcalendar(&t, b"IPM.Appointment", &limits);
    }
});
