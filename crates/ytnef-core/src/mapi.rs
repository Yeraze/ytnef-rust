//! MAPI property blocks (`attMAPIProps`, `attAttachment`): the C `TNEFFillMapi`.
//!
//! Values stay raw little-endian bytes borrowed from the input; the property
//! type says how to read them. Strings are not decoded (`PT_UNICODE` stays
//! UTF-16LE; C converts it to UTF-8).

use crate::error::HandlerError;
use crate::limits::Limits;
use crate::reader::Reader;

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
}

/// Parse a whole property block; every byte must be consumed.
pub(crate) fn parse<'a>(d: &'a [u8], l: &Limits) -> Result<Vec<Prop<'a>>, HandlerError> {
    let mut r = Reader::new(d);
    let n = count(&mut r, l.max_mapi_count)?;
    let mut props = Vec::new();
    for _ in 0..n {
        props.push(prop(&mut r, l)?);
    }
    if !r.is_empty() {
        return Err(HandlerError::TrailingBytes { len: r.remaining() });
    }
    Ok(props)
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
fn count(r: &mut Reader, max: usize) -> Result<usize, HandlerError> {
    let offset = r.pos();
    let v = r.u32()?;
    usize::try_from(v)
        .ok()
        .filter(|n| (1..=max).contains(n))
        .ok_or(HandlerError::OutOfRange {
            offset,
            value: v,
            max,
        })
}

/// Bytes after `len` that bring it to a multiple of 4.
fn pad(len: usize) -> usize {
    len.wrapping_neg() & 3
}

fn prop<'a>(r: &mut Reader<'a>, l: &Limits) -> Result<Prop<'a>, HandlerError> {
    let tag = r.u32()?;
    let name = if tag >> 16 >= 0x8000 {
        Some(name(r, l)?)
    } else {
        None
    };
    let ty = tag as u16;
    let multi = ty & MV_FLAG != 0;
    let n = if multi {
        count(r, l.max_mapi_count)?
    } else {
        1
    };
    let mut values = Vec::new();
    for _ in 0..n {
        values.push(value(r, ty & !MV_FLAG, multi, l)?);
    }
    Ok(Prop { tag, name, values })
}

fn name<'a>(r: &mut Reader<'a>, l: &Limits) -> Result<Name<'a>, HandlerError> {
    let guid = r.array::<16>()?;
    let offset = r.pos();
    let kind = r.u32()?;
    if kind == 0 {
        return Ok(Name {
            guid,
            kind: NameKind::Id(r.u32()?),
        });
    }
    if usize::try_from(kind).map_or(true, |k| k > l.max_mapi_count) {
        return Err(HandlerError::OutOfRange {
            offset,
            value: kind,
            max: l.max_mapi_count,
        });
    }
    let mut names = Vec::new();
    for _ in 0..kind {
        let len = count(r, l.max_mapi_name)?;
        names.push(r.take(len)?);
        r.take(pad(len))?;
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
) -> Result<&'a [u8], HandlerError> {
    let offset = r.pos();
    let v = match ty {
        pt::BINARY | pt::OBJECT | pt::STRING8 | pt::UNICODE => {
            if !multi {
                // value count; C assumes 1 and skips it
                r.u32()?;
            }
            let len = usize::try_from(r.u32()?).unwrap_or(usize::MAX);
            let v = r.take(len)?;
            if len > l.max_mapi_value {
                return Err(HandlerError::TooLarge {
                    len,
                    max: l.max_mapi_value,
                });
            }
            r.take(pad(len))?;
            v
        }
        pt::I2 => {
            let v = r.take(2)?;
            r.take(2)?;
            v
        }
        pt::BOOLEAN | pt::LONG | pt::R4 | pt::CURRENCY | pt::APPTIME | pt::ERROR => r.take(4)?,
        pt::DOUBLE | pt::I8 | pt::SYSTIME => r.take(8)?,
        pt::CLSID => r.take(16)?,
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
        Ok(v.remove(0))
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
            assert!(matches!(one(&d), Err(HandlerError::Truncated { .. })));
        }
    }

    #[test]
    fn i2_reads_two_skips_four() {
        let d = block(&[le(&[tag(1, pt::I2), 0xFFFF_0007])]);
        assert_eq!(one(&d).unwrap().values, vec![&[7, 0][..]]);
        // padding must be there
        let mut short = d.clone();
        short.truncate(d.len() - 2);
        assert!(matches!(one(&short), Err(HandlerError::Truncated { .. })));
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
            Err(HandlerError::Truncated { .. })
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
            Err(HandlerError::Truncated { .. })
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
            Err(HandlerError::Truncated { .. })
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
}
