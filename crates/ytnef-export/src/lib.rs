//! vCard / VTODO / VEVENT / MAPI-dump formatters.
#![forbid(unsafe_code)]

pub mod print;
pub mod rtf_text;
pub mod vcal;
pub mod vcard;

use ytnef_core::mapi::{self, prop_tag, pt};
use ytnef_core::{Dtr, Tnef};

use crate::print::cstr;

/// First value of `PROP_TAG(ty, id)` in the message properties: C
/// `MAPIFindUserProp` when `named`, else `MAPIFindProperty`. A property
/// without values yields an empty slice, which C would see as no data.
fn find<'a>(t: &Tnef<'a>, named: bool, ty: u16, id: u16) -> Option<&'a [u8]> {
    let tag = prop_tag(ty, id);
    let p = if named {
        t.mapi.get_named(tag)
    } else {
        t.mapi.get(tag)
    }?;
    Some(p.values.first().copied().unwrap_or_default())
}

/// A string property as C reads it with `%s`: `PT_STRING8` up to its NUL,
/// else `PT_UNICODE` converted to UTF-8.
fn text(t: &Tnef, named: bool, id: u16) -> Option<Vec<u8>> {
    find(t, named, pt::STRING8, id)
        .map(|v| cstr(v).to_vec())
        .or_else(|| find(t, named, pt::UNICODE, id).map(|v| mapi::utf16(v).into_bytes()))
}

/// A `PT_SYSTIME` value as C `MAPISysTimetoDTRSize` leaves it: all zero
/// when shorter than 8 bytes.
fn date(v: &[u8]) -> Dtr {
    mapi::filetime(v).unwrap_or_default()
}

/// `%04i%02i%02iT%02i%02i%02iZ`.
fn stamp(out: &mut Vec<u8>, label: &str, v: &[u8]) {
    let d = date(v);
    out.extend_from_slice(
        format!(
            "{label}{:04}{:02}{:02}T{:02}{:02}{:02}Z\n",
            d.year, d.month, d.day, d.hour, d.minute, d.second
        )
        .as_bytes(),
    );
}

/// The little-endian `DWORD` at the start of `v`, zero-padded.
fn dword(v: &[u8]) -> u32 {
    let mut b = [0; 4];
    let n = v.len().min(4);
    b[..n].copy_from_slice(&v[..n]);
    u32::from_le_bytes(b)
}
