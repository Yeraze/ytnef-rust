//! vCard 2.1 for `IPM.Contact` messages, as C `ytnef`'s `SaveVCard` writes
//! it.

use ytnef_core::mapi::pt;
use ytnef_core::Tnef;

use crate::print::{cstr, subject};
use crate::{date, find};

const PR_BODY: u16 = 0x1000;
const PR_DISPLAY_NAME: u16 = 0x3001;
const PR_COMPANY_NAME: u16 = 0x3A16;
const PR_DEPARTMENT_NAME: u16 = 0x3A18;
const PR_BIRTHDAY: u16 = 0x3A42;
const PR_WEDDING_ANNIVERSARY: u16 = 0x3A41;

/// `PT_STRING8` properties C prints with a fixed format, in its order,
/// before the organisation.
const PERSONAL: [(u16, &str); 4] = [
    (0x3A4F, "NICKNAME"),
    (0x3A48, "X-EVOLUTION-SPOUSE"),
    (0x3A4E, "X-EVOLUTION-MANAGER"),
    (0x3A30, "X-EVOLUTION-ASSISTANT"),
];

/// Between the organisation and the notes.
const JOB: [(u16, &str); 3] = [
    (0x3A19, "X-EVOLUTION-OFFICE"),
    (0x3A17, "TITLE"),
    (0x3A46, "ROLE"),
];

const PHONES: [(u16, &str); 19] = [
    (0x3A02, "TEL;X-EVOLUTION-CALLBACK"),
    (0x3A1A, "TEL;PREF"),
    (0x3A1C, "TEL;CELL"),
    (0x3A1D, "TEL;X-EVOLUTION-RADIO"),
    (0x3A1E, "TEL;CAR"),
    (0x3A1F, "TEL;VOICE"),
    (0x3A21, "TEL;PAGER"),
    (0x3A2C, "TEL;X-EVOLUTION-TELEX"),
    (0x3A2D, "TEL;ISDN"),
    (0x3A2F, "TEL;HOME"),
    (0x3A4B, "TEL;X-EVOLUTION-TTYTDD"),
    (0x3A09, "TEL;HOME;VOICE"),
    (0x3A2E, "TEL;X-EVOLUTION-ASSISTANT"),
    (0x3A57, "TEL;WORK"),
    (0x3A08, "TEL;WORK"),
    (0x3A1B, "TEL;WORK;VOICE"),
    (0x3A23, "TEL;PREF;FAX"),
    (0x3A24, "TEL;WORK;FAX"),
    (0x3A25, "TEL;HOME;FAX"),
];

/// Address properties: post office box, street, city, state, postal code,
/// country; the `ADR` type; the named label property, if any. C labels the
/// home address `WORK` too; kept for parity.
const ADDRESSES: [([u16; 6], &str, Option<u16>); 3] = [
    (
        [0x3A2B, 0x3A29, 0x3A27, 0x3A28, 0x3A2A, 0x3A26],
        "WORK",
        Some(0x801b),
    ),
    (
        [0x3A5E, 0x3A5D, 0x3A59, 0x3A5C, 0x3A5B, 0x3A5A],
        "HOME",
        Some(0x801a),
    ),
    (
        [0x3A64, 0x3A63, 0x3A5F, 0x3A62, 0x3A61, 0x3A60],
        "OTHER",
        None,
    ),
];

/// E-mail addresses: each named property, or its fallback.
const EMAILS: [(u16, u16); 3] = [(0x8083, 0x8084), (0x8093, 0x8094), (0x80a3, 0x80a4)];

/// A `PT_STRING8` property C treats as set: present with data.
fn set<'a>(t: &Tnef<'a>, id: u16) -> Option<&'a [u8]> {
    find(t, false, pt::STRING8, id).filter(|v| !v.is_empty())
}

/// The display name, else the company name: the `FN` and the file name.
fn full_name<'a>(t: &Tnef<'a>) -> Option<&'a [u8]> {
    set(t, PR_DISPLAY_NAME).or_else(|| set(t, PR_COMPANY_NAME))
}

/// The file name C gives the card (`<name>.vcard`), before sanitising: the
/// full name, else the subject, else `unknown`.
pub fn vcard_name(t: &Tnef) -> Vec<u8> {
    match full_name(t) {
        Some(n) => cstr(n).to_vec(),
        None => match subject(t) {
            Some(s) if !s.is_empty() => cstr(&s).to_vec(),
            _ => b"unknown".to_vec(),
        },
    }
}

/// C `fprintProperty`: `<label>:<value>` unless the value is empty or a
/// lone NUL.
fn field(out: &mut Vec<u8>, v: Option<&[u8]>, label: &str) {
    if let Some(v) = v.filter(|v| !v.is_empty() && *v != [0]) {
        line(out, label, cstr(v));
    }
}

fn line(out: &mut Vec<u8>, label: &str, v: &[u8]) {
    out.extend_from_slice(label.as_bytes());
    out.push(b':');
    out.extend_from_slice(v);
    out.push(b'\n');
}

/// C `quotedfprint`: every byte but the last (the NUL C expects there),
/// with LF as `=0A` and CR dropped. Other bytes, NUL included, are copied.
fn quoted(out: &mut Vec<u8>, v: &[u8]) {
    for &c in &v[..v.len().saturating_sub(1)] {
        match c {
            b'\n' => out.extend_from_slice(b"=0A"),
            b'\r' => {}
            _ => out.push(c),
        }
    }
}

fn ymd(out: &mut Vec<u8>, label: &str, v: &[u8]) {
    let d = date(v);
    out.extend_from_slice(format!("{label}:{}-{:02}-{:02}\n", d.year, d.month, d.day).as_bytes());
}

/// The card C `SaveVCard` writes for `t`.
pub fn vcard(t: &Tnef) -> Vec<u8> {
    let s8 = |id| find(t, false, pt::STRING8, id);
    let mut o = b"BEGIN:VCARD\nVERSION:2.1\n".to_vec();
    if let Some(n) = full_name(t) {
        line(&mut o, "FN", cstr(n));
    }
    field(&mut o, s8(PERSONAL[0].0), PERSONAL[0].1);
    if let Some(v) = find(t, true, pt::STRING8, 0x8554).filter(|v| !v.is_empty() && *v != [0]) {
        o.extend_from_slice(b"MAILER:Microsoft Outlook ");
        o.extend_from_slice(cstr(v));
        o.push(b'\n');
    }
    for (id, label) in &PERSONAL[1..] {
        field(&mut o, s8(*id), label);
    }

    if let Some(c) = s8(PR_COMPANY_NAME).filter(|v| !v.is_empty() && *v != [0]) {
        o.extend_from_slice(b"ORG:");
        o.extend_from_slice(cstr(c));
        if let Some(d) = s8(PR_DEPARTMENT_NAME) {
            o.push(b';');
            o.extend_from_slice(cstr(d));
        }
        o.push(b'\n');
    }

    for (id, label) in JOB {
        field(&mut o, s8(id), label);
    }
    field(&mut o, s8(PR_BODY), "NOTE");
    if let Some(b) = t.body.filter(|b| !b.is_empty()) {
        o.extend_from_slice(b"NOTE;QUOTED-PRINTABLE:");
        quoted(&mut o, b);
        o.push(b'\n');
    }

    for (ids, kind, label) in ADDRESSES {
        let parts = ids.map(s8);
        if parts.iter().all(Option::is_none) {
            continue;
        }
        o.extend_from_slice(format!("ADR;QUOTED-PRINTABLE;{kind}:").as_bytes());
        for (i, p) in parts.iter().enumerate() {
            o.extend_from_slice(match i {
                0 => b"",
                1 => b";;",
                _ => b";",
            });
            if let Some(p) = p {
                quoted(&mut o, p);
            }
        }
        o.push(b'\n');
        if let Some(l) = label.and_then(|id| find(t, true, pt::STRING8, id)) {
            o.extend_from_slice(b"LABEL;QUOTED-PRINTABLE;WORK:");
            quoted(&mut o, l);
            o.push(b'\n');
        }
    }

    for (id, label) in PHONES {
        field(&mut o, s8(id), label);
    }
    for (id, fallback) in EMAILS {
        let v = find(t, true, pt::STRING8, id).or_else(|| find(t, true, pt::STRING8, fallback));
        if let Some(v) = v.filter(|v| !v.is_empty()) {
            line(&mut o, "EMAIL", cstr(v));
        }
    }
    field(&mut o, s8(0x3A51), "URL");
    field(&mut o, find(t, true, pt::STRING8, 0x80d8), "FBURL");

    if let Some(v) = find(t, false, pt::SYSTIME, PR_BIRTHDAY) {
        ymd(&mut o, "BDAY", v);
    }
    if let Some(v) = find(t, false, pt::SYSTIME, PR_WEDDING_ANNIVERSARY) {
        ymd(&mut o, "X-EVOLUTION-ANNIVERSARY", v);
    }
    o.extend_from_slice(b"END:VCARD\n");
    o
}
