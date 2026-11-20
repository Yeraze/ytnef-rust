//! MAPI property blocks (`attMAPIProps`, `attAttachment`): the C `TNEFFillMapi`.
//!
//! Values stay raw little-endian bytes borrowed from the input; the property
//! type says how to read them. Strings are not decoded (`PT_UNICODE` stays
//! UTF-16LE; C converts it to UTF-8).

use crate::error::HandlerError;
use crate::limits::Limits;
use crate::reader::{Reader, Truncated};

/// Multi-value bit in a property type.
pub const MV_FLAG: u16 = 0x1000;

/// Property types (`PT_*` from `mapi.h`) the parser accepts. Anything else
/// fails, as in C.
pub mod pt {
    pub const I2: u16 = 0x0002;
    pub const LONG: u16 = 0x0003;
    pub const R4: u16 = 0x0004;
    pub const DOUBLE: u16 = 0x0005;
    /// 8 bytes per MS-OXTNEF, but C reads 4; so do we.
    pub const CURRENCY: u16 = 0x0006;
    /// 8 bytes per MS-OXTNEF, but C reads 4; so do we.
    pub const APPTIME: u16 = 0x0007;
    pub const ERROR: u16 = 0x000A;
    pub const BOOLEAN: u16 = 0x000B;
    pub const OBJECT: u16 = 0x000D;
    pub const I8: u16 = 0x0014;
    pub const STRING8: u16 = 0x001E;
    pub const UNICODE: u16 = 0x001F;
    pub const SYSTIME: u16 = 0x0040;
    pub const CLSID: u16 = 0x0048;
    pub const BINARY: u16 = 0x0102;
}

/// Property IDs that C copies into the message subject when it has none.
const SUBJECT_IDS: [u16; 5] = [0x0037, 0x0038, 0x0049, 0x0070, 0x0E1D];

/// One property.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prop<'a> {
    /// Tag as stored: ID in the high 16 bits, type (with [`MV_FLAG`]) in the low.
    pub tag: u32,
    /// Present when the stored ID is 0x8000 or above.
    pub name: Option<Name<'a>>,
    /// One entry for a single-valued property. Variable-length values exclude
    /// the padding; I2 values are the 2 meaningful bytes.
    pub values: Vec<&'a [u8]>,
}

/// Named-property identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Name<'a> {
    pub guid: [u8; 16],
    pub kind: NameKind<'a>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameKind<'a> {
    /// Kind 0: numeric ID.
    Id(u32),
    /// Kind n > 0: C reads n length-prefixed UTF-16LE names (real files have
    /// one). Stream order; C stores them reversed.
    Strings(Vec<&'a [u8]>),
}

impl Prop<'_> {
    /// Type without [`MV_FLAG`].
    pub fn prop_type(&self) -> u16 {
        (self.tag as u16) & !MV_FLAG
    }

    pub fn is_multi(&self) -> bool {
        (self.tag as u16) & MV_FLAG != 0
    }

    /// The ID C works with: a numeric name replaces the stored ID (low 16
    /// bits, as `PROP_TAG` truncates it).
    pub fn id(&self) -> u16 {
        match &self.name {
            Some(Name {
                kind: NameKind::Id(n),
                ..
            }) => *n as u16,
            _ => (self.tag >> 16) as u16,
        }
    }

    /// The tag C stores and looks up: [`Prop::id`] with the type, minus [`MV_FLAG`].
    fn c_tag(&self) -> u32 {
        (u32::from(self.id()) << 16) | u32::from(self.prop_type())
    }
}

/// A parsed property block, in stream order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MapiProps<'a>(pub Vec<Prop<'a>>);

impl<'a> std::ops::Deref for MapiProps<'a> {
    type Target = Vec<Prop<'a>>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'a> MapiProps<'a> {
    /// First non-named property with this tag (`PROP_TAG(type, id)`, no
    /// [`MV_FLAG`]): C's `MAPIFindProperty`.
    pub fn get(&self, tag: u32) -> Option<&Prop<'a>> {
        self.iter().find(|p| p.name.is_none() && p.c_tag() == tag)
    }

    /// First named property with this tag, where the ID is the numeric name
    /// (or the stored ID for string names): C's `MAPIFindUserProp`.
    pub fn get_named(&self, tag: u32) -> Option<&Prop<'a>> {
        self.iter().find(|p| p.name.is_some() && p.c_tag() == tag)
    }
}

/// `PROP_TAG(type, id)`.
pub const fn prop_tag(prop_type: u16, id: u16) -> u32 {
    ((id as u32) << 16) | prop_type as u32
}

/// Decode a UTF-16LE value (`PT_UNICODE`, named-property names) up to its
/// first NUL. Unpaired surrogates become U+FFFD, as in C `to_utf8`; an odd
/// trailing byte is ignored.
pub fn utf16(b: &[u8]) -> String {
    let units = b
        .chunks_exact(2)
        .filter_map(|c| c.try_into().ok())
        .map(u16::from_le_bytes)
        .take_while(|&u| u != 0);
    char::decode_utf16(units)
        .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect()
}

/// Convert a `PT_SYSTIME` FILETIME (100 ns ticks since 1601-01-01 UTC, first
/// 8 bytes little-endian) to a date, like C `MAPISysTimetoDTRSize`. `None`
/// if fewer than 8 bytes.
pub fn filetime(b: &[u8]) -> Option<crate::Dtr> {
    let ticks = u64::from_le_bytes(*b.first_chunk::<8>()?);
    let secs = ticks / 10_000_000;
    let days = secs / 86_400;
    let (year, month, day) = civil_from_days(days);
    Some(crate::Dtr {
        year,
        month,
        day,
        hour: (secs / 3600 % 24) as u16,
        minute: (secs / 60 % 60) as u16,
        second: (secs % 60) as u16,
        // 1601-01-01 was a Monday
        day_of_week: (days.wrapping_add(1) % 7) as u16,
    })
}

/// Days since 1601-01-01 to (year, month, day), after Howard Hinnant's
/// `civil_from_days`. A u64 FILETIME reaches year 60056 at most, so every
/// intermediate fits.
#[allow(clippy::arithmetic_side_effects)]
fn civil_from_days(days: u64) -> (u16, u16, u16) {
    // days from 0000-03-01 to 1601-01-01
    let z = days + 584_694;
    let era = z / 146_097;
    let doe = z % 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + u64::from(month <= 2);
    (year as u16, month as u16, day as u16)
}

/// Parse a whole property block; every byte must be consumed.
pub fn parse<'a>(d: &'a [u8], l: &Limits) -> Result<MapiProps<'a>, HandlerError> {
    let mut r = Reader::new(d);
    let n = count(&mut r, l.max_mapi_count, "property count")?;
    let mut props = Vec::new();
    let mut extra = 0;
    for _ in 0..n {
        props.push(prop(&mut r, l, &mut extra)?);
    }
    if extra > 0 {
        return Err(HandlerError::Overrun { extra });
    }
    if !r.is_empty() {
        return Err(HandlerError::TrailingBytes { len: r.remaining() });
    }
    Ok(MapiProps(props))
}

/// Tag a failed read with the field it was for.
fn at(what: &'static str) -> impl Fn(Truncated) -> HandlerError {
    move |t| HandlerError::MapiTruncated {
        what,
        offset: t.offset,
        needed: t.needed,
    }
}

/// Skip `n` bytes C steps over without a bounds check. Past the end, the
/// reader stops there (so the next read fails, as C's next `SIZECHECK`
/// does) and the overshoot is added to `extra`.
fn skip(r: &mut Reader, n: usize, extra: &mut usize) {
    let k = n.min(r.remaining());
    let _ = r.take(k);
    *extra = extra.saturating_add(n.saturating_sub(k));
}

/// C fills an empty subject from the first subject-like value, failing
/// (`PREALLOCCHECK`) when that value is empty or too long.
pub(crate) fn fill_subject<'a>(
    subject: &mut Option<&'a [u8]>,
    props: &[Prop<'a>],
    max: usize,
) -> Result<(), HandlerError> {
    if subject.is_some() {
        return Ok(());
    }
    let first = props
        .iter()
        .filter(|p| SUBJECT_IDS.contains(&p.id()))
        .find_map(|p| p.values.first());
    if let Some(v) = first {
        if v.is_empty() || v.len() > max {
            return Err(HandlerError::OutOfRange {
                what: "subject length",
                offset: 0,
                value: u32::try_from(v.len()).unwrap_or(u32::MAX),
                max,
            });
        }
        *subject = Some(v);
    }
    Ok(())
}

/// A u32 that C passes through `PREALLOCCHECK(x, max)`: must be in 1..=max.
fn count(r: &mut Reader, max: usize, what: &'static str) -> Result<usize, HandlerError> {
    let offset = r.pos();
    let v = r.u32().map_err(at(what))?;
    usize::try_from(v)
        .ok()
        .filter(|n| (1..=max).contains(n))
        .ok_or(HandlerError::OutOfRange {
            what,
            offset,
            value: v,
            max,
        })
}

/// Bytes after `len` that bring it to a multiple of 4.
fn pad(len: usize) -> usize {
    len.wrapping_neg() & 3
}

fn prop<'a>(r: &mut Reader<'a>, l: &Limits, extra: &mut usize) -> Result<Prop<'a>, HandlerError> {
    let tag = r.u32().map_err(at("property tag"))?;
    let name = if tag >> 16 >= 0x8000 {
        Some(name(r, l, extra)?)
    } else {
        None
    };
    let ty = tag as u16;
    let multi = ty & MV_FLAG != 0;
    let n = if multi {
        count(r, l.max_mapi_count, "value count")?
    } else {
        1
    };
    let mut values = Vec::new();
    for _ in 0..n {
        values.push(value(r, ty & !MV_FLAG, multi, l, extra)?);
    }
    Ok(Prop { tag, name, values })
}

fn name<'a>(r: &mut Reader<'a>, l: &Limits, extra: &mut usize) -> Result<Name<'a>, HandlerError> {
    let guid = r.array::<16>().map_err(at("name guid"))?;
    let offset = r.pos();
    let kind = r.u32().map_err(at("name count"))?;
    if kind == 0 {
        return Ok(Name {
            guid,
            kind: NameKind::Id(r.u32().map_err(at("name id"))?),
        });
    }
    if usize::try_from(kind).map_or(true, |k| k > l.max_mapi_count) {
        return Err(HandlerError::OutOfRange {
            what: "name count",
            offset,
            value: kind,
            max: l.max_mapi_count,
        });
    }
    let mut names = Vec::new();
    for _ in 0..kind {
        let len = count(r, l.max_mapi_name, "name length")?;
        names.push(r.take(len).map_err(at("name"))?);
        skip(r, pad(len), extra);
    }
    Ok(Name {
        guid,
        kind: NameKind::Strings(names),
    })
}

fn value<'a>(
    r: &mut Reader<'a>,
    ty: u16,
    multi: bool,
    l: &Limits,
    extra: &mut usize,
) -> Result<&'a [u8], HandlerError> {
    let offset = r.pos();
    let v = match ty {
        pt::BINARY | pt::OBJECT | pt::STRING8 | pt::UNICODE => {
            if !multi {
                // value count; C assumes 1 and skips it
                r.u32().map_err(at("value count"))?;
            }
            let len = usize::try_from(r.u32().map_err(at("value length"))?).unwrap_or(usize::MAX);
            let v = r.take(len).map_err(at("value"))?;
            if len > l.max_mapi_value {
                return Err(HandlerError::TooLarge {
                    len,
                    max: l.max_mapi_value,
                });
            }
            skip(r, pad(len), extra);
            v
        }
        pt::I2 => {
            let v = r.take(2).map_err(at("PT_I2 value"))?;
            skip(r, 2, extra);
            v
        }
        pt::BOOLEAN | pt::LONG | pt::R4 | pt::CURRENCY | pt::APPTIME | pt::ERROR => {
            r.take(4).map_err(at("32-bit value"))?
        }
        pt::DOUBLE | pt::I8 | pt::SYSTIME => r.take(8).map_err(at("64-bit value"))?,
        pt::CLSID => r.take(16).map_err(at("PT_CLSID value"))?,
        _ => {
            return Err(HandlerError::BadPropType {
                offset,
                prop_type: ty,
            })
        }
    };
    Ok(v)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
mod tests {
    use super::*;

    /// Little-endian u32s.
    fn le(v: &[u32]) -> Vec<u8> {
        v.iter().flat_map(|x| x.to_le_bytes()).collect()
    }

    fn block(props: &[Vec<u8>]) -> Vec<u8> {
        let mut v = le(&[u32::try_from(props.len()).unwrap()]);
        props.iter().for_each(|p| v.extend(p));
        v
    }

    fn tag(id: u16, ty: u16) -> u32 {
        u32::from(id) << 16 | u32::from(ty)
    }

    fn one(d: &[u8]) -> Result<Prop<'_>, HandlerError> {
        let mut v = parse(d, &Limits::default())?;
        assert_eq!(v.len(), 1);
        Ok(v.0.remove(0))
    }

    #[test]
    fn fixed_size_types() {
        for (ty, size) in [
            (pt::BOOLEAN, 4),
            (pt::LONG, 4),
            (pt::R4, 4),
            (pt::CURRENCY, 4),
            (pt::APPTIME, 4),
            (pt::ERROR, 4),
            (pt::DOUBLE, 8),
            (pt::I8, 8),
            (pt::SYSTIME, 8),
            (pt::CLSID, 16),
        ] {
            let data: Vec<u8> = (1..=size).collect();
            let mut p = le(&[tag(0x1234, ty)]);
            p.extend(&data);
            let d = block(&[p.clone()]);
            let got = one(&d).unwrap();
            assert_eq!(got.prop_type(), ty);
            assert_eq!(got.id(), 0x1234);
            assert!(!got.is_multi());
            assert_eq!(got.values, vec![&data[..]], "type {ty:#x}");
            // one byte short
            let d = block(&[p[..p.len() - 1].to_vec()]);
            assert!(matches!(one(&d), Err(HandlerError::MapiTruncated { .. })));
        }
    }

    #[test]
    fn i2_reads_two_skips_four() {
        let d = block(&[le(&[tag(1, pt::I2), 0xFFFF_0007])]);
        assert_eq!(one(&d).unwrap().values, vec![&[7, 0][..]]);
        // C skips the unused half unchecked, then rejects the overrun
        let mut short = d.clone();
        short.truncate(d.len() - 2);
        assert_eq!(one(&short), Err(HandlerError::Overrun { extra: 2 }));
        // a missing value fails at its own read
        short.truncate(d.len() - 4);
        assert_eq!(
            one(&short),
            Err(HandlerError::MapiTruncated {
                what: "PT_I2 value",
                offset: 8,
                needed: 2
            })
        );
    }

    #[test]
    fn truncation_names_the_field() {
        let what = |d: &[u8]| match parse(d, &Limits::default()) {
            Err(HandlerError::MapiTruncated { what, .. }) => what,
            _ => "not a MAPI truncation",
        };
        assert_eq!(what(&[1, 0]), "property count");
        assert_eq!(what(&le(&[1])), "property tag");
        assert_eq!(what(&le(&[1, tag(1, pt::LONG | MV_FLAG)])), "value count");
        assert_eq!(what(&le(&[1, tag(1, pt::STRING8)])), "value count");
        assert_eq!(what(&le(&[1, tag(1, pt::STRING8), 1])), "value length");
        assert_eq!(what(&le(&[1, tag(1, pt::STRING8), 1, 5])), "value");
        assert_eq!(what(&le(&[1, tag(1, pt::LONG)])), "32-bit value");
        assert_eq!(what(&le(&[1, tag(1, pt::SYSTIME), 0])), "64-bit value");
        assert_eq!(what(&le(&[1, tag(1, pt::CLSID)])), "PT_CLSID value");
        let named = |rest: &[u32]| {
            let mut d = le(&[1, tag(0x8001, pt::LONG), 0, 0, 0, 0]);
            d.extend(le(rest));
            d
        };
        assert_eq!(what(&le(&[1, tag(0x8001, pt::LONG), 0])), "name guid");
        assert_eq!(what(&named(&[])), "name count");
        assert_eq!(what(&named(&[0])), "name id");
        assert_eq!(what(&named(&[1])), "name length");
        assert_eq!(what(&named(&[1, 8, 0])), "name");
    }

    #[test]
    fn padding_overrun_is_not_a_truncation() {
        // a 1-byte string whose padding is missing: C reads past the block
        let d = le(&[1, tag(1, pt::STRING8), 1, 1]);
        let mut d = d;
        d.push(b'x');
        assert_eq!(
            parse(&d, &Limits::default()),
            Err(HandlerError::Overrun { extra: 3 })
        );
        // ... and a following property then fails at its tag
        let mut two = le(&[2, tag(1, pt::STRING8), 1, 1]);
        two.push(b'x');
        assert!(matches!(
            parse(&two, &Limits::default()),
            Err(HandlerError::MapiTruncated {
                what: "property tag",
                ..
            })
        ));
    }

    #[test]
    fn variable_types_pad_to_four() {
        for ty in [pt::BINARY, pt::OBJECT, pt::STRING8, pt::UNICODE] {
            for len in 0..=9u32 {
                let data: Vec<u8> = (0..len as u8).map(|b| b + 0x41).collect();
                let mut p = le(&[tag(0x3001, ty), 1, len]);
                p.extend(&data);
                p.resize(p.len() + pad(data.len()), 0);
                let d = block(&[p]);
                assert_eq!(one(&d).unwrap().values, vec![&data[..]]);
                if len % 4 != 0 {
                    // missing padding on the last value is still an error
                    assert!(one(&d[..d.len() - 1]).is_err());
                }
            }
        }
    }

    #[test]
    fn multi_value() {
        // MV LONG: count then values
        let d = block(&[le(&[tag(0x0E01, pt::LONG | MV_FLAG), 3, 10, 20, 30])]);
        let p = one(&d).unwrap();
        assert!(p.is_multi());
        assert_eq!(p.prop_type(), pt::LONG);
        assert_eq!(p.values, vec![&le(&[10])[..], &le(&[20]), &le(&[30])]);

        // MV strings: count, then (len, data, pad) per value; no per-value count
        let mut s = le(&[tag(0x0E02, pt::STRING8 | MV_FLAG), 2, 3]);
        s.extend(b"ab\0\0");
        s.extend(le(&[5]));
        s.extend(b"cdef\0\0\0\0");
        let d = block(&[s]);
        assert_eq!(one(&d).unwrap().values, vec![&b"ab\0"[..], b"cdef\0"]);

        // zero or excessive counts fail like C's PREALLOCCHECK
        for n in [0, 1001] {
            let d = block(&[le(&[tag(1, pt::LONG | MV_FLAG), n])]);
            assert!(matches!(one(&d), Err(HandlerError::OutOfRange { .. })));
        }
    }

    #[test]
    fn named_by_id() {
        let mut p = le(&[tag(0x8005, pt::LONG)]);
        p.extend([0xAA; 16]);
        p.extend(le(&[0, 0x0001_8102, 42]));
        let d = block(&[p]);
        let got = one(&d).unwrap();
        assert_eq!(
            got.name,
            Some(Name {
                guid: [0xAA; 16],
                kind: NameKind::Id(0x0001_8102)
            })
        );
        assert_eq!(got.id(), 0x8102);
        assert_eq!(got.values, vec![&le(&[42])[..]]);
    }

    #[test]
    fn named_by_string() {
        let name = b"K\0e\0y\0\0\0"; // "Key" UTF-16LE + NUL, 8 bytes
        let odd = b"x\0\0"; // 3 bytes, padded to 4
        let mut p = le(&[tag(0x8001, pt::BOOLEAN)]);
        p.extend([0x11; 16]);
        p.extend(le(&[2, 8]));
        p.extend(name);
        p.extend(le(&[3]));
        p.extend(odd);
        p.push(0);
        p.extend(le(&[1]));
        let d = block(&[p]);
        let got = one(&d).unwrap();
        assert_eq!(
            got.name.as_ref().unwrap().kind,
            NameKind::Strings(vec![&name[..], odd])
        );
        assert_eq!(got.id(), 0x8001);
    }

    #[test]
    fn named_bad_lengths() {
        for (kind, len) in [(1, 0), (1, 1001), (1001, 4)] {
            let mut p = le(&[tag(0x8001, pt::LONG)]);
            p.extend([0; 16]);
            p.extend(le(&[kind, len]));
            assert!(matches!(
                one(&block(&[p])),
                Err(HandlerError::OutOfRange { .. })
            ));
        }
        // truncated GUID
        let mut p = le(&[tag(0x8001, pt::LONG)]);
        p.extend([0; 15]);
        assert!(matches!(
            one(&block(&[p])),
            Err(HandlerError::MapiTruncated {
                what: "name guid",
                ..
            })
        ));
    }

    #[test]
    fn several_props_in_order() {
        let d = block(&[
            le(&[tag(1, pt::LONG), 5]),
            le(&[tag(2, pt::STRING8), 1, 2, 0x0000_6968]),
        ]);
        let v = parse(&d, &Limits::default()).unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!(v[1].values, vec![&b"hi"[..]]);
    }

    #[test]
    fn malformed_blocks() {
        let l = Limits::default();
        // empty, zero count, over count limit
        assert!(matches!(
            parse(&[], &l),
            Err(HandlerError::MapiTruncated { .. })
        ));
        for n in [0, 1001, u32::MAX] {
            assert!(matches!(
                parse(&le(&[n]), &l),
                Err(HandlerError::OutOfRange { .. })
            ));
        }
        // count larger than the data
        assert!(parse(&le(&[2, tag(1, pt::LONG), 5]), &l).is_err());
        // unknown types, including PT_NULL and PT_UNSPECIFIED
        for ty in [0, 1, 0x0008, 0x00FF, 0x0FFF] {
            assert!(matches!(
                parse(&le(&[1, tag(1, ty), 0]), &l),
                Err(HandlerError::BadPropType { prop_type, .. }) if prop_type == ty
            ));
        }
        // trailing bytes
        assert_eq!(
            parse(&le(&[1, tag(1, pt::LONG), 5, 0]), &l),
            Err(HandlerError::TrailingBytes { len: 4 })
        );
        // huge string length: truncation, not an allocation
        assert!(matches!(
            parse(&le(&[1, tag(1, pt::BINARY), 1, u32::MAX]), &l),
            Err(HandlerError::MapiTruncated { what: "value", .. })
        ));
    }

    #[test]
    fn value_size_limit() {
        let l = Limits {
            max_mapi_value: 4,
            ..Limits::default()
        };
        let mut d = le(&[1, tag(1, pt::BINARY), 1, 5]);
        d.extend([0; 8]);
        assert_eq!(
            parse(&d, &l),
            Err(HandlerError::TooLarge { len: 5, max: 4 })
        );
    }

    #[test]
    fn subject_fill() {
        let p = |id, v: &'static [u8]| Prop {
            tag: tag(id, pt::STRING8),
            name: None,
            values: vec![v],
        };
        let props = [p(0x1000, b"no"), p(0x0E1D, b"norm"), p(0x0037, b"subj")];
        let mut s = None;
        fill_subject(&mut s, &props, 1000).unwrap();
        assert_eq!(s, Some(&b"norm"[..]));
        // an existing subject wins
        let mut s = Some(&b"att"[..]);
        fill_subject(&mut s, &props, 1000).unwrap();
        assert_eq!(s, Some(&b"att"[..]));
        // empty or over-long subject value is an error, as in C
        let mut s = None;
        assert!(fill_subject(&mut s, &[p(0x0037, b"")], 1000).is_err());
        assert!(fill_subject(&mut s, &[p(0x0037, b"long")], 3).is_err());
    }

    #[test]
    fn get_and_get_named() {
        let plain = |id: u16, ty: u16, name| Prop {
            tag: tag(id, ty),
            name,
            values: vec![],
        };
        let named = |n| {
            Some(Name {
                guid: [0; 16],
                kind: n,
            })
        };
        let props = MapiProps(vec![
            plain(0x8001, pt::LONG, named(NameKind::Id(0x0037))),
            plain(0x0037, pt::STRING8 | MV_FLAG, None),
            plain(0x8002, pt::UNICODE, named(NameKind::Strings(vec![]))),
        ]);
        let subject = prop_tag(pt::STRING8, 0x0037);
        assert_eq!(props.get(subject).map(|p| p.tag >> 16), Some(0x0037));
        assert!(props.get(prop_tag(pt::LONG, 0x0037)).is_none());
        assert_eq!(
            props
                .get_named(prop_tag(pt::LONG, 0x0037))
                .map(|p| p.tag >> 16),
            Some(0x8001)
        );
        assert!(props.get_named(subject).is_none());
        assert!(props.get_named(prop_tag(pt::UNICODE, 0x8002)).is_some());
    }

    #[test]
    fn utf16_decoding() {
        let enc = |s: &[u16]| -> Vec<u8> { s.iter().flat_map(|u| u.to_le_bytes()).collect() };
        assert_eq!(utf16(&enc(&[0x48, 0x69, 0, 0x41])), "Hi");
        assert_eq!(utf16(&enc(&[0xE9, 0x20AC])), "é€");
        // U+1F600 as a surrogate pair
        assert_eq!(utf16(&enc(&[0xD83D, 0xDE00])), "\u{1F600}");
        // unpaired high and low surrogates
        assert_eq!(utf16(&enc(&[0xD83D, 0x41, 0xDE00])), "\u{FFFD}A\u{FFFD}");
        // odd trailing byte dropped
        assert_eq!(utf16(&[0x41, 0, 0x42]), "A");
        assert_eq!(utf16(&[]), "");
    }

    fn ft(y: u16, mo: u16, d: u16, h: u16, mi: u16, s: u16, dow: u16) -> crate::Dtr {
        crate::Dtr {
            year: y,
            month: mo,
            day: d,
            hour: h,
            minute: mi,
            second: s,
            day_of_week: dow,
        }
    }

    /// FILETIME for a Unix timestamp.
    fn unix(secs: u64) -> [u8; 8] {
        ((secs + 11_644_473_600) * 10_000_000).to_le_bytes()
    }

    #[test]
    fn filetime_conversion() {
        assert_eq!(filetime(&[0; 8]), Some(ft(1601, 1, 1, 0, 0, 0, 1)));
        assert_eq!(filetime(&unix(0)), Some(ft(1970, 1, 1, 0, 0, 0, 4)));
        // first of a month
        assert_eq!(
            filetime(&unix(1_709_251_200)),
            Some(ft(2024, 3, 1, 0, 0, 0, 5))
        );
        assert_eq!(
            filetime(&unix(1_706_745_599)),
            Some(ft(2024, 1, 31, 23, 59, 59, 3))
        );
        assert_eq!(
            filetime(&unix(1_704_067_200)),
            Some(ft(2024, 1, 1, 0, 0, 0, 1))
        );
        // leap days, including the 400-year rule
        assert_eq!(
            filetime(&unix(1_709_164_800)),
            Some(ft(2024, 2, 29, 0, 0, 0, 4))
        );
        assert_eq!(
            filetime(&unix(951_782_400)),
            Some(ft(2000, 2, 29, 0, 0, 0, 2))
        );
        assert_eq!(
            filetime(&unix(4_107_542_400)),
            Some(ft(2100, 3, 1, 0, 0, 0, 1))
        );
        assert_eq!(
            filetime(&unix(1_735_603_200)),
            Some(ft(2024, 12, 31, 0, 0, 0, 2))
        );
        // extra bytes ignored, short input refused, max value does not panic
        assert_eq!(filetime(&[0; 12]), Some(ft(1601, 1, 1, 0, 0, 0, 1)));
        assert_eq!(filetime(&[0; 7]), None);
        assert!(filetime(&[0xff; 8]).is_some_and(|d| d.year > 60_000));
    }

    #[test]
    fn tag_names() {
        assert_eq!(crate::tags::tag_name(0x0037), Some("PR_SUBJECT"));
        assert_eq!(crate::tags::tag_name(0x1009), Some("PR_RTF_COMPRESSED"));
        assert_eq!(crate::tags::tag_name(0x7FFF), None);
    }
}
