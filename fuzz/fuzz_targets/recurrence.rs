#![no_main]

//! `PR_RECURRENCE` blobs (vCalendar RRULE) and the RTF-to-text pass that
//! fills calendar descriptions.

use libfuzzer_sys::fuzz_target;
use ytnef_export::{rtf_text, vcal};

fuzz_target!(|data: &[u8]| {
    let (interval, rule) = match data.split_first() {
        Some((&n, rest)) if n & 1 == 1 && rest.len() >= 4 => {
            let (i, r) = rest.split_at(4);
            (Some(i), r)
        }
        Some((_, rest)) => (None, rest),
        None => (None, data),
    };
    let mut out = Vec::new();
    vcal::rrule(&mut out, rule, interval);
    out.clear();
    rtf_text::rtf_text(&mut out, data);
});
