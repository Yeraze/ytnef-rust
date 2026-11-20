//! iCalendar for `IPM.Task` (VTODO) and meeting / appointment (VEVENT)
//! messages, as C `ytnef`'s `SaveVTask`, `SaveVCalendar` and `PrintRrule`
//! write them.

use ytnef_core::mapi::pt;
use ytnef_core::{rtf, Limits, RtfError, Tnef};

use crate::print::{cstr, subject};
use crate::rtf_text::rtf_text;
use crate::{dword, find, stamp, text};

const PRODID: &[u8] = b"PRODID:-//The Gauntlet//ytnef 2.1.4//EN\n";
const PR_CONVERSATION_TOPIC: u16 = 0x0070;
const PR_DISPLAY_TO: u16 = 0x0E04;
const PR_SENDER_SEARCH_KEY: u16 = 0x0C1D;
const PR_SENT_REPRESENTING_EMAIL_ADDRESS: u16 = 0x0065;
const PR_RTF_COMPRESSED: u16 = 0x1009;
const PR_CREATION_TIME: u16 = 0x3007;
const PR_LAST_MODIFICATION_TIME: u16 = 0x3008;
/// C `TNEFStruct.messageID` holds this many bytes.
const MESSAGE_ID_MAX: usize = 50;

/// C `Cstylefprint`: up to the first NUL, LF as `\n`, `;` and `,`
/// escaped, CR dropped. C does not escape `\`; neither does this.
fn escaped(out: &mut Vec<u8>, v: &[u8]) {
    for &c in cstr(v) {
        match c {
            b'\n' => out.extend_from_slice(b"\\n"),
            b'\r' => {}
            b';' => out.extend_from_slice(b"\\;"),
            b',' => out.extend_from_slice(b"\\,"),
            _ => out.push(c),
        }
    }
}

/// A `;`-separated list as C walks it: each entry with leading spaces
/// dropped, empty entries included.
fn entries(list: &[u8]) -> impl Iterator<Item = &[u8]> {
    cstr(list)
        .split(|&c| c == b';')
        .map(|e| &e[e.iter().take_while(|&&c| c == b' ').count()..])
}

/// `PR_CONVERSATION_TOPIC`, which names the task file (`<topic>.vcf`)
/// before sanitising. `None` when C writes no task.
pub fn vtask_name(t: &Tnef) -> Option<Vec<u8>> {
    let topic = find(t, false, pt::STRING8, PR_CONVERSATION_TOPIC).filter(|v| !v.is_empty())?;
    Some(cstr(topic).to_vec())
}

/// The task C `SaveVTask` writes for `t`.
pub fn vtask(t: &Tnef) -> Vec<u8> {
    let mut o = b"BEGIN:VCALENDAR\n".to_vec();
    o.extend_from_slice(PRODID);
    o.extend_from_slice(b"VERSION:2.0\nMETHOD:PUBLISH\nBEGIN:VTODO\n");
    // C reads up to 50 bytes and, without a NUL, on past the field
    let id = cstr(t.message_id.unwrap_or_default());
    let id = &id[..id.len().min(MESSAGE_ID_MAX)];
    if !id.is_empty() {
        line(&mut o, "UID:", id);
    }
    if let Some(v) = find(t, true, pt::STRING8, 0x8122) {
        line(&mut o, "ORGANIZER:", cstr(v));
    }
    // attendees come from 0x811f, read only when PR_DISPLAY_TO is set
    if find(t, false, pt::STRING8, PR_DISPLAY_TO).is_some() {
        if let Some(list) = find(t, true, pt::STRING8, 0x811f).filter(|v| v.len() > 1) {
            for a in entries(list) {
                o.extend_from_slice(b"ATTENDEE;CN=");
                o.extend_from_slice(a);
                o.extend_from_slice(b";ROLE=REQ-PARTICIPANT:");
                o.extend_from_slice(a);
                o.push(b'\n');
            }
        }
    }
    if let Some(s) = subject(t).filter(|s| !s.is_empty()) {
        o.extend_from_slice(b"SUMMARY:");
        escaped(&mut o, &s);
        o.push(b'\n');
    }
    if let Some(b) = t.body.filter(|b| !b.is_empty()) {
        o.extend_from_slice(b"DESCRIPTION:");
        escaped(&mut o, b);
        o.push(b'\n');
    }
    if let Some(v) = find(t, false, pt::SYSTIME, PR_CREATION_TIME) {
        stamp(&mut o, "DTSTAMP:", v);
    }
    if let Some(v) = find(t, true, pt::SYSTIME, 0x8517) {
        stamp(&mut o, "DUE:", v);
    }
    if let Some(v) = find(t, false, pt::SYSTIME, PR_LAST_MODIFICATION_TIME) {
        stamp(&mut o, "LAST-MODIFIED:", v);
    }
    class(&mut o, t);
    o.extend_from_slice(b"END:VTODO\nEND:VCALENDAR\n");
    o
}

fn line(out: &mut Vec<u8>, label: &str, v: &[u8]) {
    out.extend_from_slice(label.as_bytes());
    out.extend_from_slice(v);
    out.push(b'\n');
}

fn class(out: &mut Vec<u8>, t: &Tnef) {
    if let Some(v) = find(t, true, pt::BOOLEAN, 0x8506) {
        out.extend_from_slice(if dword(v) == 1 {
            b"CLASS:PRIVATE\n"
        } else {
            b"CLASS:PUBLIC\n"
        });
    }
}

fn attendees(out: &mut Vec<u8>, list: &[u8], role: &str) {
    if cstr(list).len() <= 1 {
        return;
    }
    for a in entries(list) {
        out.extend_from_slice(b"ATTENDEE;PARTSTAT=NEEDS-ACTION;ROLE=");
        out.extend_from_slice(role.as_bytes());
        out.extend_from_slice(b";RSVP=TRUE;CN=\"");
        out.extend_from_slice(a);
        out.extend_from_slice(b"\":MAILTO:");
        out.extend_from_slice(a);
        out.push(b'\n');
    }
}

/// The calendar C `SaveVCalendar` writes for `t`, and the error from
/// decompressing its RTF description, which C prints to stdout.
pub fn vcalendar(t: &Tnef, class_name: &[u8], limits: &Limits) -> (Vec<u8>, Option<RtfError>) {
    let mut o = b"BEGIN:VCALENDAR\n".to_vec();
    let last = class_name.iter().rposition(|&c| c == b'.').unwrap_or(0);
    o.extend_from_slice(if &class_name[last..] == b".MtgCncl" {
        b"METHOD:CANCEL\n"
    } else {
        b"METHOD:REQUEST\n"
    });
    o.extend_from_slice(PRODID);
    o.extend_from_slice(b"VERSION:2.0\nBEGIN:VEVENT\n");

    let uid = find(t, true, pt::BINARY, 0x3).or_else(|| find(t, true, pt::BINARY, 0x23));
    if let Some(uid) = uid {
        o.extend_from_slice(b"UID:");
        for b in uid {
            o.extend_from_slice(format!("{b:02X}").as_bytes());
        }
        o.push(b'\n');
    }
    if let Some(v) = find(t, true, pt::LONG, 0x8201) {
        o.extend_from_slice(format!("SEQUENCE:{}\n", dword(v) as i32).as_bytes());
    }

    let organizer = find(t, false, pt::BINARY, PR_SENDER_SEARCH_KEY)
        .map(|v| cstr(v).to_vec())
        .or_else(|| text(t, false, PR_SENT_REPRESENTING_EMAIL_ADDRESS));
    if let Some(org) = organizer {
        let addr = match org.iter().position(|&c| c == b':') {
            Some(i) => &org[i + 1..],
            None => &org[..],
        };
        o.extend_from_slice(b"ORGANIZER;CN=\"");
        o.extend_from_slice(addr);
        o.extend_from_slice(b"\":MAILTO:");
        o.extend_from_slice(addr);
        o.push(b'\n');
    }

    if let Some(req) = text(t, true, 0x823b) {
        attendees(&mut o, &req, "REQ-PARTICIPANT");
        if let Some(opt) = text(t, true, 0x823c) {
            attendees(&mut o, &opt, "OPT-PARTICIPANT");
        }
    } else if let Some(req) = text(t, true, 0x8238) {
        attendees(&mut o, &req, "REQ-PARTICIPANT");
    }

    if let Some(s) = text(t, false, PR_CONVERSATION_TOPIC) {
        o.extend_from_slice(b"SUMMARY:");
        escaped(&mut o, &s);
        o.push(b'\n');
    }

    let mut rtf_err = None;
    if let Some(v) = find(t, false, pt::BINARY, PR_RTF_COMPRESSED) {
        match rtf::decompress_rtf(v, limits) {
            Ok(r) => {
                o.extend_from_slice(b"DESCRIPTION:");
                rtf_text(&mut o, &r);
            }
            Err(e) => rtf_err = Some(e),
        }
    }

    let location = find(t, true, pt::STRING8, 0x0002)
        .or_else(|| find(t, true, pt::STRING8, 0x8208))
        .map(|v| cstr(v).to_vec())
        .or_else(|| text(t, true, 0x0002))
        .or_else(|| text(t, true, 0x8208));
    if let Some(l) = location {
        line(&mut o, "LOCATION: ", &l);
    }
    for (label, id, fallback) in [
        ("DTSTART:", 0x820d, 0x8516),
        ("DTEND:", 0x820e, 0x8517),
        ("DTSTAMP:", 0x8202, 0x001a),
    ] {
        let v = find(t, true, pt::SYSTIME, id).or_else(|| find(t, true, pt::SYSTIME, fallback));
        if let Some(v) = v {
            stamp(&mut o, label, v);
        }
    }
    class(&mut o, t);
    if let Some(r) = find(t, true, pt::BINARY, 0x8216) {
        let interval = find(t, true, pt::I2, 0x0011);
        rrule(&mut o, r, interval);
    }
    o.extend_from_slice(b"END:VEVENT\nEND:VCALENDAR\n");
    (o, rtf_err)
}

/// C `GetRruleMonthNum`: the month from two bytes of the recurrence blob,
/// 0 when unknown.
fn month(a: u8, b: u8) -> u8 {
    match (a, b) {
        (0x00, 0x00) => 1,
        (0x60, 0xAE) => 2,
        (0xE0, 0x4B) => 3,
        (0x40, 0xFA) => 4,
        (0x00, 0xA3) => 5,
        (0x60, 0x51) => 6,
        (0x20, 0xFA) => 7,
        (0x80, 0xA8) => 8,
        (0xE0, 0x56) => 9,
        (0xA0, 0xFF) => 10,
        (0x00, 0xAE) => 11,
        (0xC0, 0x56) => 12,
        _ => 0,
    }
}

/// C `GetRruleDayname`: the day bit mask as `SU,MO,...`.
fn days(mask: u8) -> String {
    ["SU", "MO", "TU", "WE", "TH", "FR", "SA"]
        .iter()
        .enumerate()
        .filter(|(i, _)| mask & (1 << i) != 0)
        .map(|(_, d)| *d)
        .collect::<Vec<_>>()
        .join(",")
}

/// C `PrintRrule`: an `RRULE` line from the recurrence blob (named
/// property 0x8216), read at C's fixed offsets. `interval` is the named
/// `PT_I2` 0x0011 used for daily rules.
///
/// Divergences, all where C's output is garbage:
/// - C reads up to offset 0x23 of a blob it only checks to be 0x1F bytes;
///   missing bytes read as 0 here.
/// - C's `GetRruleCount` returns an `unsigned char`, so `COUNT` loses its
///   high byte (300 occurrences print as 44); here it is 16 bits.
/// - C reads blob bytes as signed `char`, so values from 0x80 print
///   negative; here they print unsigned. The daily `INTERVAL` is the whole
///   16-bit value; C prints its low byte.
pub fn rrule(out: &mut Vec<u8>, r: &[u8], interval: Option<&[u8]>) {
    if r.len() < 0x1F {
        return;
    }
    let b = |i: usize| r.get(i).copied().unwrap_or(0);
    let count = |hi: usize, lo: usize| (u16::from(b(hi)) << 8) | u16::from(b(lo));
    let ends = |i: usize| matches!(b(i), 0x21 | 0x22);
    let setpos = |v: u8| if v == 0x05 { -1 } else { i32::from(v) };
    let mut s = String::from("RRULE:FREQ=");
    match b(0x04) {
        0x0A => {
            s += "DAILY";
            if matches!(b(0x16), 0x21..=0x23) {
                if let Some(v) = interval {
                    let mut w = [0; 2];
                    w[..v.len().min(2)].copy_from_slice(&v[..v.len().min(2)]);
                    s += &format!(";INTERVAL={}", u16::from_le_bytes(w));
                }
                if ends(0x16) {
                    s += &format!(";COUNT={}", count(0x1B, 0x1A));
                }
            } else if b(0x16) == 0x3E {
                s += ";BYDAY=MO,TU,WE,TH,FR";
                if ends(0x1A) {
                    s += &format!(";COUNT={}", count(0x1F, 0x1E));
                }
            }
        }
        0x0B => {
            s += &format!("WEEKLY;INTERVAL={};BYDAY={}", b(0x0E), days(b(0x16)));
            if ends(0x1A) {
                s += &format!(";COUNT={}", count(0x1F, 0x1E));
            }
        }
        0x0C => {
            s += "MONTHLY";
            match b(0x06) {
                0x02 => {
                    s += &format!(";INTERVAL={};BYMONTHDAY={}", b(0x0E), b(0x16));
                    if ends(0x1A) {
                        s += &format!(";COUNT={}", count(0x1F, 0x1E));
                    }
                }
                0x03 => {
                    s += &format!(
                        ";BYDAY={};BYSETPOS={};INTERVAL={}",
                        days(b(0x16)),
                        setpos(b(0x1A)),
                        b(0x0E)
                    );
                    if ends(0x1E) {
                        s += &format!(";COUNT={}", count(0x23, 0x22));
                    }
                }
                _ => {}
            }
        }
        0x0D => {
            s += &format!("YEARLY;BYMONTH={}", month(b(0x0A), b(0x0B)));
            match b(0x06) {
                0x02 => s += &format!(";BYMONTHDAY={}", b(0x16)),
                0x03 => {
                    s += &format!(";BYDAY={};BYSETPOS={}", days(b(0x16)), setpos(b(0x1A)));
                }
                _ => {}
            }
            if ends(0x1E) {
                s += &format!(";COUNT={}", count(0x23, 0x22));
            }
        }
        _ => {}
    }
    out.extend_from_slice(s.as_bytes());
    out.push(b'\n');
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 0x28-byte blob: `kind` at 0x04, `pattern` at 0x06, then `at`.
    fn blob(kind: u8, pattern: u8, at: &[(usize, u8)]) -> Vec<u8> {
        let mut b = vec![0; 0x28];
        b[0x04] = kind;
        b[0x06] = pattern;
        for &(i, v) in at {
            b[i] = v;
        }
        b
    }

    fn rule(b: &[u8], interval: Option<&[u8]>) -> String {
        let mut v = Vec::new();
        rrule(&mut v, b, interval);
        String::from_utf8(v).unwrap()
    }

    #[test]
    fn daily() {
        let b = blob(0x0A, 0, &[(0x16, 0x22), (0x1A, 7)]);
        assert_eq!(
            rule(&b, Some(&[3, 0])),
            "RRULE:FREQ=DAILY;INTERVAL=3;COUNT=7\n"
        );
        let b = blob(0x0A, 0, &[(0x16, 0x23)]);
        assert_eq!(rule(&b, None), "RRULE:FREQ=DAILY\n");
        let b = blob(0x0A, 0, &[(0x16, 0x3E), (0x1A, 0x21), (0x1E, 20)]);
        assert_eq!(
            rule(&b, None),
            "RRULE:FREQ=DAILY;BYDAY=MO,TU,WE,TH,FR;COUNT=20\n"
        );
    }

    #[test]
    fn weekly() {
        let b = blob(0x0B, 0, &[(0x0E, 1), (0x16, 0x41)]);
        assert_eq!(rule(&b, None), "RRULE:FREQ=WEEKLY;INTERVAL=1;BYDAY=SU,SA\n");
    }

    #[test]
    fn monthly() {
        let b = blob(0x0C, 2, &[(0x0E, 1), (0x16, 15), (0x1A, 0x22), (0x1E, 12)]);
        assert_eq!(
            rule(&b, None),
            "RRULE:FREQ=MONTHLY;INTERVAL=1;BYMONTHDAY=15;COUNT=12\n"
        );
        let b = blob(0x0C, 3, &[(0x0E, 2), (0x16, 0x04), (0x1A, 5)]);
        assert_eq!(
            rule(&b, None),
            "RRULE:FREQ=MONTHLY;BYDAY=TU;BYSETPOS=-1;INTERVAL=2\n"
        );
        // unknown pattern: just the frequency
        assert_eq!(rule(&blob(0x0C, 9, &[]), None), "RRULE:FREQ=MONTHLY\n");
    }

    #[test]
    fn yearly() {
        let b = blob(0x0D, 2, &[(0x0A, 0x60), (0x0B, 0x51), (0x16, 21)]);
        assert_eq!(
            rule(&b, None),
            "RRULE:FREQ=YEARLY;BYMONTH=6;BYMONTHDAY=21\n"
        );
        let b = blob(
            0x0D,
            3,
            &[
                (0x0A, 0xE0),
                (0x0B, 0x56),
                (0x16, 2),
                (0x1A, 1),
                (0x1E, 0x21),
                (0x22, 5),
            ],
        );
        assert_eq!(
            rule(&b, None),
            "RRULE:FREQ=YEARLY;BYMONTH=9;BYDAY=MO;BYSETPOS=1;COUNT=5\n"
        );
        let months: Vec<u8> = [
            (0x00, 0x00),
            (0x60, 0xAE),
            (0xE0, 0x4B),
            (0x40, 0xFA),
            (0x00, 0xA3),
            (0x60, 0x51),
            (0x20, 0xFA),
            (0x80, 0xA8),
            (0xE0, 0x56),
            (0xA0, 0xFF),
            (0x00, 0xAE),
            (0xC0, 0x56),
            (0x12, 0x34),
        ]
        .iter()
        .map(|&(a, b)| month(a, b))
        .collect();
        assert_eq!(months, [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 0]);
    }

    #[test]
    fn unknown_and_short() {
        assert_eq!(rule(&blob(0x01, 0, &[]), None), "RRULE:FREQ=\n");
        assert_eq!(rule(&[0x0B; 0x1E], None), "");
    }

    #[test]
    fn diverges_where_c_is_garbage() {
        // C prints COUNT=44 (the low byte of 300)
        let b = blob(
            0x0B,
            0,
            &[(0x0E, 1), (0x16, 2), (0x1A, 0x22), (0x1E, 0x2C), (0x1F, 1)],
        );
        assert_eq!(
            rule(&b, None),
            "RRULE:FREQ=WEEKLY;INTERVAL=1;BYDAY=MO;COUNT=300\n"
        );
        // C prints INTERVAL=-56
        let b = blob(0x0B, 0, &[(0x0E, 200), (0x16, 2)]);
        assert_eq!(rule(&b, None), "RRULE:FREQ=WEEKLY;INTERVAL=200;BYDAY=MO\n");
        // C prints the low byte (44) of a 16-bit daily interval
        let b = blob(0x0A, 0, &[(0x16, 0x23)]);
        assert_eq!(
            rule(&b, Some(&[0x2C, 0x01])),
            "RRULE:FREQ=DAILY;INTERVAL=300\n"
        );
        // C reads 0x22/0x23 past a 0x1F-byte blob; here they are 0
        let mut b = blob(0x0D, 2, &[(0x1E, 0x22)]);
        b.truncate(0x1F);
        assert_eq!(
            rule(&b, None),
            "RRULE:FREQ=YEARLY;BYMONTH=1;BYMONTHDAY=0;COUNT=0\n"
        );
    }

    #[test]
    fn escapes_and_lists() {
        let mut v = Vec::new();
        escaped(&mut v, b"a;b,c\\d\r\ne\0f");
        assert_eq!(v, b"a\\;b\\,c\\d\\ne");
        let e: Vec<&[u8]> = entries(b"  a;b ;;c\0d").collect();
        assert_eq!(e, [&b"a"[..], b"b ", b"", b"c"]);
    }
}
