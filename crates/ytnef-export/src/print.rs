//! Text dump of a parsed message in the format of C `ytnefprint`
//! (`PrintTNEF`, `MAPIPrint`, `TNEFPrintDate`), byte for byte.
//!
//! C prints most fields with `%s`, so a value ends at its first NUL; the
//! helpers below reproduce that.

use std::io::{self, Write};

use ytnef_core::mapi::{self, pt, MapiProps, NameKind, Prop};
use ytnef_core::{rtf, Dtr, Limits, RtfError, Tnef};

/// `%s` of a byte string: everything before the first NUL.
fn cstr(b: &[u8]) -> &[u8] {
    &b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())]
}

/// C `char[cap]` field filled by `memcpy(min(len, copy))`, read with `%s`.
fn fixed(b: &[u8], copy: usize) -> &[u8] {
    cstr(&b[..b.len().min(copy)])
}

/// `isprint` in the C locale, else `.`.
fn printable(out: &mut impl Write, b: &[u8]) -> io::Result<()> {
    let s: Vec<u8> = b
        .iter()
        .map(|&c| if (0x20..0x7f).contains(&c) { c } else { b'.' })
        .collect();
    out.write_all(&s)
}

/// C `TNEFPrintDate`.
pub fn print_date(out: &mut impl Write, d: &Dtr) -> io::Result<()> {
    const DAYS: [&str; 7] = [
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
    ];
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    if let Some(day) = DAYS.get(usize::from(d.day_of_week)) {
        write!(out, "{day} ")?;
    }
    if let Some(month) = usize::from(d.month)
        .checked_sub(1)
        .and_then(|m| MONTHS.get(m))
    {
        write!(out, "{month} ")?;
    }
    write!(out, "{}, {} ", d.day, d.year)?;
    let (hour, ampm) = match d.hour {
        h if h > 12 => (h - 12, "pm"),
        12 => (12, "pm"),
        h => (h, "am"),
    };
    write!(out, "{hour}:{:02}:{:02} {ampm}", d.minute, d.second)
}

fn type_label(t: u16) -> String {
    match t {
        0x0000 => "  NONE   ",
        0x0001 => "  NULL   ",
        pt::I2 => "   I2    ",
        pt::LONG => "  LONG   ",
        pt::R4 => "   R4    ",
        pt::DOUBLE => " DOUBLE  ",
        pt::CURRENCY => "CURRENCY ",
        pt::APPTIME => "APP TIME ",
        pt::ERROR => "  ERROR  ",
        pt::BOOLEAN => " BOOLEAN ",
        pt::OBJECT => " OBJECT  ",
        pt::I8 => "   I8    ",
        pt::STRING8 => " STRING8 ",
        pt::UNICODE => " UNICODE ",
        pt::SYSTIME => "SYS TIME ",
        pt::CLSID => "OLE GUID ",
        pt::BINARY => " BINARY  ",
        _ => return format!("<{t:x}>"),
    }
    .to_owned()
}

/// What C `DecompressRTF` prints when it gives up.
pub fn rtf_error(e: &RtfError) -> String {
    match e {
        RtfError::TooShort { .. } => "File too small\n".to_owned(),
        // C compares against the size minus the size field itself
        RtfError::SizeMismatch { stored, len } => {
            format!(" Size Mismatch: {stored} != {}\n", *len as i64 - 4)
        }
        RtfError::BadRawSize { .. } => "Corrupted file\n".to_owned(),
        RtfError::Truncated { .. } => "Corrupted stream\n".to_owned(),
        RtfError::UnknownMagic { magic } => {
            format!("Unknown compression type (magic number {magic:x})\n")
        }
        // C has no output cap; a Rust-only limit
        e => format!("{e}\n"),
    }
}

fn print_value(out: &mut impl Write, ty: u16, v: &[u8]) -> io::Result<()> {
    // C keeps a zero-length value as a NULL pointer
    if v.is_empty() {
        return out.write_all(if ty == pt::OBJECT {
            b"\n"
        } else {
            b"    Value: []\n"
        });
    }
    let le = |n: usize| v.get(..n).unwrap_or_default();
    match ty {
        pt::SYSTIME => {
            out.write_all(b"    Value: ")?;
            print_date(out, &mapi::filetime(v).unwrap_or_default())?;
            out.write_all(b" [HEX: ")?;
            for b in le(8) {
                write!(out, " {b:02x}")?;
            }
            let ticks = u64::from_le_bytes(le(8).try_into().unwrap_or_default());
            writeln!(out, "] ({ticks})")
        }
        pt::LONG => {
            let n = i32::from_le_bytes(le(4).try_into().unwrap_or_default());
            writeln!(out, "    Value: {n}")
        }
        pt::I2 => {
            let n = i16::from_le_bytes(le(2).try_into().unwrap_or_default());
            writeln!(out, "    Value: {n}")
        }
        pt::BOOLEAN => writeln!(
            out,
            "    Value: {}",
            if v[0] != 0 { "True" } else { "False" }
        ),
        pt::OBJECT => out.write_all(b"\n"),
        pt::BINARY if rtf::is_compressed_rtf(v) => {
            const RULE: &[u8] = b"-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-\n";
            out.write_all(b"    Detected Compressed RTF. Decompressed text follows\n")?;
            out.write_all(RULE)?;
            match rtf::decompress_rtf(v, &Limits::default()) {
                Ok(text) => {
                    out.write_all(cstr(&text))?;
                    out.write_all(b"\n")?;
                }
                Err(e) => out.write_all(rtf_error(&e).as_bytes())?,
            }
            out.write_all(RULE)
        }
        pt::BINARY => {
            out.write_all(b"    Value: [")?;
            printable(out, v)?;
            out.write_all(b"]\n")
        }
        pt::STRING8 => {
            out.write_all(b"    Value: [")?;
            out.write_all(cstr(v))?;
            out.write_all(b"]\n")?;
            // C stores the value with one extra NUL and compares strlen to size - 1
            if cstr(v).len() + 1 != v.len() {
                out.write_all(b"Detected Hidden data: [")?;
                printable(out, v)?;
                out.write_all(b"]\n")?;
            }
            Ok(())
        }
        pt::CLSID => {
            out.write_all(b"    Value: [HEX: ")?;
            for b in le(16) {
                write!(out, " {b:02x}")?;
            }
            out.write_all(b"]\n")
        }
        // C converts to UTF-8 but keeps the UTF-16 byte count as the size
        pt::UNICODE => {
            let s = mapi::utf16(v);
            out.write_all(b"    Value: [")?;
            out.write_all(&s.as_bytes()[..s.len().min(v.len())])?;
            out.write_all(b"]\n")
        }
        _ => {
            out.write_all(b"    Value: [")?;
            out.write_all(cstr(v))?;
            out.write_all(b"]\n")
        }
    }
}

/// C `MAPIPrint`.
pub fn mapi_print(out: &mut impl Write, props: &MapiProps) -> io::Result<()> {
    for (j, p) in props.iter().enumerate() {
        print_prop(out, j, p)?;
    }
    Ok(())
}

fn print_prop(out: &mut impl Write, j: usize, p: &Prop) -> io::Result<()> {
    let ty = p.prop_type();
    write!(out, "   #{j}: Type: [{}]  Code: [", type_label(ty))?;
    let id = p.id();
    match (&p.name, ytnef_core::tags::tag_name(id)) {
        (Some(_), _) => write!(out, "UD:x{id:04x}")?,
        (None, Some(name)) => out.write_all(name.as_bytes())?,
        (None, None) => write!(out, "0x{id:04x}")?,
    }
    out.write_all(b"]\n")?;
    if let Some(NameKind::Strings(names)) = p.name.as_ref().map(|n| &n.kind) {
        // C keeps the low byte of each UTF-16 unit and stores the names reversed
        for n in names.iter().rev() {
            let low: Vec<u8> = n.chunks_exact(2).map(|c| c[0]).collect();
            out.write_all(b"    Name: ")?;
            out.write_all(cstr(&low))?;
            out.write_all(b"\n")?;
        }
    }
    let count = p.values.len();
    for (i, v) in p.values.iter().enumerate() {
        if count > 1 {
            write!(out, "    [{i}/{count}] ")?;
        } else {
            out.write_all(b"    ")?;
        }
        write!(out, "Size: {}", v.len())?;
        print_value(out, ty, v)?;
    }
    Ok(())
}

/// The subject as C holds it: copied from a MAPI subject property (with
/// `\`, `/` and NUL replaced by `_`) unless an `attSubject` set it.
pub fn subject(t: &Tnef) -> Option<Vec<u8>> {
    let s = t.subject?;
    let from_attr = t
        .attributes
        .iter()
        .any(|a| a.id == ytnef_core::attr::id::SUBJECT);
    Some(if from_attr {
        cstr(s).to_vec()
    } else {
        s.iter()
            .map(|&c| {
                if matches!(c, b'\\' | b'/' | 0) {
                    b'_'
                } else {
                    c
                }
            })
            .collect()
    })
}

fn line(out: &mut impl Write, label: &str, v: &[u8]) -> io::Result<()> {
    out.write_all(label.as_bytes())?;
    out.write_all(v)?;
    out.write_all(b"\n")
}

fn date_line(out: &mut impl Write, label: &str, d: Option<Dtr>) -> io::Result<()> {
    match d {
        Some(d) if d.year > 0 => {
            out.write_all(label.as_bytes())?;
            print_date(out, &d)?;
            out.write_all(b"\n")
        }
        _ => Ok(()),
    }
}

/// C `ytnefprint`'s `PrintTNEF`. `verbose == 1` adds each attachment's MAPI
/// properties (C prints them at exactly one `-v`).
pub fn print_tnef(out: &mut impl Write, t: &Tnef, verbose: u32) -> io::Result<()> {
    let version = t
        .version
        .map(|(major, minor)| format!("TNEF{major}.{minor}"))
        .unwrap_or_default();
    writeln!(out, "---> In {version} format")?;
    if let Some(from) = t.from {
        line(out, "From: ", cstr(from))?;
    }
    if let Some(s) = subject(t) {
        line(out, "Subject: ", &s)?;
    }
    if let Some(p) = t.priority {
        let p = match p {
            3 => "high",
            2 => "normal",
            1 => "low",
            _ => "N/A",
        };
        writeln!(out, "Message Priority: {p}")?;
    }
    date_line(out, "Date Sent: ", t.date_sent)?;
    date_line(out, "Date Received: ", t.date_received)?;
    let class = fixed(t.message_class.unwrap_or_default(), 49);
    if !class.is_empty() {
        line(out, "Message Class: ", class)?;
        match class {
            b"IPM.Contact" => out.write_all(b"--> Found a contact card\n")?,
            b"IPM.Task" => out.write_all(b"--> Found a Task Entry\n")?,
            _ => {}
        }
    }
    if let Some(c) = t.original_message_class {
        line(out, "Original Message Class: ", cstr(c))?;
    }
    let parent = fixed(t.parent_id.unwrap_or_default(), 50);
    let id = fixed(t.message_id.unwrap_or_default(), 50);
    if !id.is_empty() {
        // a full messageID[50] has no NUL, so %s runs on into parentID
        let tail = if id.len() == 50 { parent } else { b"" };
        line(out, "Message ID: ", &[id, tail].concat())?;
    }
    if !parent.is_empty() {
        line(out, "Parent ID: ", parent)?;
    }
    date_line(out, "Start Date: ", t.date_start)?;
    date_line(out, "End Date: ", t.date_end)?;
    if let Some(body) = t.body {
        writeln!(out, "-- Message Body ({} bytes) --", body.len())?;
        out.write_all(cstr(body))?;
        out.write_all(b"\n-- End Body --\n")?;
    }
    if !t.mapi.is_empty() {
        writeln!(out, "    MAPI Properties: {}", t.mapi.len())?;
        mapi_print(out, &t.mapi)?;
    }
    let class_prop = t
        .mapi
        .get_named(mapi::prop_tag(pt::STRING8, 0x24))
        .and_then(|p| p.values.first());
    if class_prop.is_some_and(|v| cstr(v) == b"IPM.Appointment") {
        out.write_all(b"--> Found an appointment entry\n")?;
    }
    for (n, a) in t.attachments.iter().enumerate() {
        let kind = match a.render.atyp {
            0 => "NULL      ",
            1 => "File      ",
            2 => "OLE Object",
            3 => "Picture   ",
            4 => "Max       ",
            _ => "Unknown   ",
        };
        write!(out, "[{}] [{kind}] ", n + 1)?;
        out.write_all(cstr(a.title.unwrap_or_default()))?;
        out.write_all(b"\n")?;
        if a.render.flags == 1 {
            out.write_all(b"     MAC Binary Encoding\n")?;
        }
        date_line(out, "    Creation Date: ", a.create_date)?;
        date_line(out, "    Modified on: ", a.modify_date)?;
        if !a.mapi.is_empty() {
            writeln!(out, "    MAPI Properties: {}", a.mapi.len())?;
            if verbose == 1 {
                mapi_print(out, &a.mapi)?;
            }
        }
        if let Some(data) = a.data {
            writeln!(out, "    Attachment Size:  {}b", data.len())?;
            let by_tag = |id| {
                a.mapi
                    .get(mapi::prop_tag(pt::STRING8, id))
                    .map(|p| p.values.first().copied().unwrap_or_default())
            };
            let name = by_tag(0x3707).or_else(|| by_tag(0x3001)).or(a.title);
            // glibc prints a NULL %s argument as "(null)"
            let name = match name {
                Some(n) if !n.is_empty() => cstr(n),
                _ => b"(null)",
            };
            let name: Vec<u8> = name
                .iter()
                .take(1023)
                .map(|&c| if c == b' ' { b'_' } else { c })
                .collect();
            line(out, "    File saves as [", &[&name[..], b"]"].concat())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(d: Dtr) -> String {
        let mut v = Vec::new();
        print_date(&mut v, &d).unwrap();
        String::from_utf8(v).unwrap()
    }

    fn value(ty: u16, v: &[u8]) -> String {
        let mut out = Vec::new();
        print_value(&mut out, ty, v).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn dates_like_c() {
        let d = Dtr {
            year: 2003,
            month: 6,
            day: 17,
            hour: 15,
            minute: 23,
            second: 0,
            day_of_week: 2,
        };
        assert_eq!(date(d), "Tuesday June 17, 2003 3:23:00 pm");
        assert_eq!(
            date(Dtr { hour: 12, ..d }),
            "Tuesday June 17, 2003 12:23:00 pm"
        );
        assert_eq!(
            date(Dtr { hour: 0, ..d }),
            "Tuesday June 17, 2003 0:23:00 am"
        );
        // out-of-range weekday and month are left out
        let odd = Dtr {
            month: 13,
            day_of_week: 7,
            ..d
        };
        assert_eq!(date(odd), "17, 2003 3:23:00 pm");
    }

    #[test]
    fn values_like_c() {
        assert_eq!(value(pt::STRING8, b""), "    Value: []\n");
        assert_eq!(value(pt::OBJECT, b""), "\n");
        assert_eq!(value(pt::STRING8, b"ab\0"), "    Value: [ab]\n");
        assert_eq!(
            value(pt::STRING8, b"a\0b\0"),
            "    Value: [a]\nDetected Hidden data: [a.b.]\n"
        );
        assert_eq!(value(pt::LONG, &(-5i32).to_le_bytes()), "    Value: -5\n");
        assert_eq!(value(pt::BOOLEAN, &[0, 1, 0, 0]), "    Value: False\n");
        assert_eq!(value(pt::BINARY, b"a\x01\xffz"), "    Value: [a..z]\n");
        assert_eq!(
            value(pt::UNICODE, b"a\0b\0\xe9\0"),
            "    Value: [ab\u{e9}]\n"
        );
        // three CJK units are 9 UTF-8 bytes; C prints only the 6 the size allows
        assert_eq!(
            value(pt::UNICODE, b"\x2d\x4e\x2d\x4e\x2d\x4e"),
            "    Value: [\u{4e2d}\u{4e2d}]\n"
        );
    }
}
