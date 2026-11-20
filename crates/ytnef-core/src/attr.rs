//! Attribute stream: header, payload, checksum, and per-attribute handlers
//! (the C `TNEFList` dispatch table).

use crate::error::{Error, HandlerError};
use crate::limits::{Checksum, Limits, Options};
use crate::mapi;
use crate::reader::{Reader, Truncated};
use crate::{Attachment, Dtr, RendData, Tnef};

/// Attribute IDs: `ATT(type, id)` values from `ytnef.h`.
pub mod id {
    pub const NULL: u32 = 0x0000_0000;
    pub const FROM: u32 = 0x0000_8000;
    pub const SUBJECT: u32 = 0x0001_8004;
    pub const DATE_SENT: u32 = 0x0003_8005;
    pub const DATE_RECD: u32 = 0x0003_8006;
    pub const MESSAGE_STATUS: u32 = 0x0006_8007;
    pub const MESSAGE_CLASS: u32 = 0x0007_8008;
    pub const MESSAGE_ID: u32 = 0x0001_8009;
    pub const PARENT_ID: u32 = 0x0001_800A;
    pub const CONVERSATION_ID: u32 = 0x0001_800B;
    pub const BODY: u32 = 0x0002_800C;
    pub const PRIORITY: u32 = 0x0004_800D;
    pub const ATTACH_DATA: u32 = 0x0006_800F;
    pub const ATTACH_TITLE: u32 = 0x0001_8010;
    pub const ATTACH_META_FILE: u32 = 0x0006_8011;
    pub const ATTACH_CREATE_DATE: u32 = 0x0003_8012;
    pub const ATTACH_MODIFY_DATE: u32 = 0x0003_8013;
    pub const DATE_MODIFIED: u32 = 0x0003_8020;
    pub const ATTACH_TRANSPORT_FILENAME: u32 = 0x0006_9001;
    pub const ATTACH_RENDDATA: u32 = 0x0006_9002;
    pub const MAPI_PROPS: u32 = 0x0006_9003;
    pub const RECIP_TABLE: u32 = 0x0006_9004;
    pub const ATTACHMENT: u32 = 0x0006_9005;
    pub const TNEF_VERSION: u32 = 0x0008_9006;
    pub const OEM_CODEPAGE: u32 = 0x0006_9007;
    pub const ORIGINAL_MESSAGE_CLASS: u32 = 0x0007_0006;
    pub const OWNER: u32 = 0x0006_0000;
    pub const SENT_FOR: u32 = 0x0006_0001;
    pub const DELEGATE: u32 = 0x0006_0002;
    pub const DATE_START: u32 = 0x0003_0006;
    pub const DATE_END: u32 = 0x0003_0007;
    pub const AID_OWNER: u32 = 0x0005_0008;
    pub const REQUEST_RES: u32 = 0x0004_0009;
}

/// `level` byte of a message-level attribute.
pub const LVL_MESSAGE: u8 = 1;
/// `level` byte of an attachment-level attribute.
pub const LVL_ATTACHMENT: u8 = 2;

/// One raw attribute as it appears in the stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attribute<'a> {
    /// Offset of the attribute header in the input.
    pub offset: usize,
    /// [`LVL_MESSAGE`] or [`LVL_ATTACHMENT`]; C reads and ignores it.
    pub level: u8,
    /// One of the [`id`] constants, or an unknown value (kept, not handled).
    pub id: u32,
    pub data: &'a [u8],
    /// False only under [`Checksum::Lenient`] when the stored sum is wrong.
    pub checksum_ok: bool,
}

/// level (1) + id (4) + size (4)
const HEADER_LEN: usize = 9;

impl From<Truncated> for Error {
    fn from(t: Truncated) -> Self {
        Error::Truncated {
            offset: t.offset,
            needed: t.needed,
        }
    }
}

/// Read the next attribute. `Ok(None)` at end of stream; like C, fewer than
/// [`HEADER_LEN`] trailing bytes also end the stream quietly.
pub(crate) fn next<'a>(r: &mut Reader<'a>, opts: &Options) -> Result<Option<Attribute<'a>>, Error> {
    if r.remaining() < HEADER_LEN {
        return Ok(None);
    }
    let offset = r.pos();
    let level = r.u8()?;
    let id = r.u32()?;
    let size = r.u32()?;
    if size == 0 {
        return Err(Error::EmptyAttribute { attr: id, offset });
    }
    let max = opts.limits.max_attr_size;
    let size = usize::try_from(size)
        .ok()
        .filter(|s| *s <= max)
        .ok_or(Error::LimitExceeded {
            what: "attribute size",
            value: usize::try_from(size).unwrap_or(usize::MAX),
            max,
        })?;
    let data = r.take(size)?;
    let stored = r.u16()?;
    let computed = data.iter().fold(0u16, |s, &b| s.wrapping_add(u16::from(b)));
    let checksum_ok = stored == computed;
    if !checksum_ok && opts.checksum == Checksum::Strict {
        return Err(Error::BadChecksum {
            attr: id,
            offset,
            stored,
            computed,
        });
    }
    Ok(Some(Attribute {
        offset,
        level,
        id,
        data,
        checksum_ok,
    }))
}

/// Apply one attribute to `t`, as the matching C handler does.
pub(crate) fn apply<'a>(t: &mut Tnef<'a>, a: &Attribute<'a>, l: &Limits) -> Result<(), Error> {
    if a.id == id::ATTACH_RENDDATA {
        if t.attachments.len() >= l.max_attachments {
            return Err(Error::LimitExceeded {
                what: "attachment count",
                value: t.attachments.len(),
                max: l.max_attachments,
            });
        }
        t.attachments.push(Attachment {
            render: RendData::parse(a.data),
            ..Attachment::default()
        });
        return Ok(());
    }
    handle(t, a.id, a.data, l).map_err(|reason| Error::InHandler {
        attr: a.id,
        offset: a.offset,
        reason,
    })
}

fn capped(d: &[u8], max: usize) -> Result<&[u8], HandlerError> {
    if d.len() > max {
        Err(HandlerError::TooLarge { len: d.len(), max })
    } else {
        Ok(d)
    }
}

fn handle<'a>(t: &mut Tnef<'a>, att: u32, d: &'a [u8], l: &Limits) -> Result<(), HandlerError> {
    match att {
        id::FROM => t.from = Some(capped(d, l.max_from)?),
        id::SUBJECT => t.subject = Some(capped(d, l.max_subject)?),
        id::BODY => t.body = Some(capped(d, l.max_body)?),
        id::ORIGINAL_MESSAGE_CLASS => {
            t.original_message_class = Some(capped(d, l.max_original_message_class)?)
        }
        id::MESSAGE_CLASS => t.message_class = Some(d),
        id::MESSAGE_ID => t.message_id = Some(d),
        id::PARENT_ID => t.parent_id = Some(d),
        id::OEM_CODEPAGE => t.oem_codepage = Some(d),
        id::MAPI_PROPS => {
            t.mapi = mapi::parse(d, l)?;
            mapi::fill_subject(&mut t.subject, &t.mapi, l.max_subject)?;
        }
        id::DATE_SENT => t.date_sent = Some(Dtr::parse(d)?),
        id::DATE_RECD => t.date_received = Some(Dtr::parse(d)?),
        id::DATE_MODIFIED => t.date_modified = Some(Dtr::parse(d)?),
        id::DATE_START => t.date_start = Some(Dtr::parse(d)?),
        id::DATE_END => t.date_end = Some(Dtr::parse(d)?),
        id::PRIORITY => {
            // C: SwapDWord(data, size) reads min(size, 4) bytes, zero-extended
            let mut b = [0u8; 4];
            b.iter_mut().zip(d).for_each(|(o, i)| *o = *i);
            t.priority = Some(u32::from_le_bytes(b));
        }
        id::TNEF_VERSION => {
            let mut r = Reader::new(d);
            if d.len() != 4 {
                return Err(HandlerError::BadLength { len: d.len() });
            }
            let minor = r.u16()?;
            let major = r.u16()?;
            t.version = Some((major, minor));
        }
        id::RECIP_TABLE => {
            // C reads the row count and stops: only the first 4 bytes must exist
            Reader::new(d).u32()?;
        }
        id::SENT_FOR => {
            let mut r = Reader::new(d);
            while !r.is_empty() {
                for _ in 0..2 {
                    let n = r.u16()?;
                    r.take(n.into())?;
                }
            }
        }
        id::ATTACH_DATA => t.current_attachment().data = Some(d),
        id::ATTACH_TITLE => t.current_attachment().title = Some(capped(d, l.max_title)?),
        id::ATTACH_META_FILE => t.current_attachment().icon = Some(capped(d, l.max_icon)?),
        id::ATTACH_CREATE_DATE => t.current_attachment().create_date = Some(Dtr::parse(d)?),
        id::ATTACH_MODIFY_DATE => t.current_attachment().modify_date = Some(Dtr::parse(d)?),
        id::ATTACHMENT => {
            let props = mapi::parse(d, l)?;
            mapi::fill_subject(&mut t.subject, &props, l.max_subject)?;
            t.current_attachment().mapi = props;
        }
        // the rest only print in C debug mode; they stay in `Tnef::attributes`
        _ => {}
    }
    Ok(())
}

impl Dtr {
    /// Seven little-endian u16s; C requires at least 14 bytes and ignores the rest.
    fn parse(d: &[u8]) -> Result<Self, HandlerError> {
        let mut r = Reader::new(d);
        if d.len() < 14 {
            return Err(HandlerError::BadLength { len: d.len() });
        }
        Ok(Dtr {
            year: r.u16()?,
            month: r.u16()?,
            day: r.u16()?,
            hour: r.u16()?,
            minute: r.u16()?,
            second: r.u16()?,
            day_of_week: r.u16()?,
        })
    }
}

impl RendData {
    /// [MS-OXTNEF] 2.1.3.3.17 layout, zero-filled when short (C copies what is there).
    fn parse(d: &[u8]) -> Self {
        let mut b = [0u8; 14];
        b.iter_mut().zip(d).for_each(|(o, i)| *o = *i);
        let mut r = Reader::new(&b);
        RendData {
            atyp: r.u16().unwrap_or(0),
            position: r.u32().unwrap_or(0),
            width: r.u16().unwrap_or(0),
            height: r.u16().unwrap_or(0),
            flags: r.u32().unwrap_or(0),
        }
    }
}
