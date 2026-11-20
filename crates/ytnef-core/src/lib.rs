//! TNEF (`winmail.dat`) parser: safe, bounded, no I/O.
//!
//! [`parse`] walks the attribute stream, verifies checksums and fills a
//! [`Tnef`] that borrows from the input, including parsed MAPI properties
//! ([`Tnef::mapi`], [`Attachment::mapi`]).
#![forbid(unsafe_code)]
#![deny(
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

pub mod attr;
mod error;
mod limits;
pub mod mapi;
mod reader;
pub mod rtf;
pub mod tags;

pub use attr::Attribute;
pub use error::{Error, HandlerError, RtfError};
pub use limits::{Checksum, Limits, Options};

use reader::Reader;

/// Little-endian magic at the start of every TNEF stream.
pub const TNEF_SIGNATURE: u32 = 0x223E_9F78;

/// True if `buf` starts with the TNEF signature.
pub fn has_signature(buf: &[u8]) -> bool {
    buf.first_chunk::<4>()
        .is_some_and(|b| u32::from_le_bytes(*b) == TNEF_SIGNATURE)
}

/// TNEF date-time (`dtr`): fields as stored, not validated.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Dtr {
    pub year: u16,
    pub month: u16,
    pub day: u16,
    pub hour: u16,
    pub minute: u16,
    pub second: u16,
    pub day_of_week: u16,
}

/// `attAttachRenddata` payload.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RendData {
    /// 1 = file, 2 = OLE object, 3 = picture.
    pub atyp: u16,
    pub position: u32,
    pub width: u16,
    pub height: u16,
    /// 1 = MacBinary.
    pub flags: u32,
}

/// One attachment: everything between an `attAttachRenddata` and the next.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Attachment<'a> {
    pub render: RendData,
    pub title: Option<&'a [u8]>,
    pub data: Option<&'a [u8]>,
    pub icon: Option<&'a [u8]>,
    pub create_date: Option<Dtr>,
    pub modify_date: Option<Dtr>,
    /// `attAttachment` MAPI properties; empty when absent.
    pub mapi: mapi::MapiProps<'a>,
}

/// A parsed TNEF stream. Byte strings are raw (no charset decoding) and
/// borrow from the input. For a repeated attribute the last one wins.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tnef<'a> {
    /// The 16-bit key after the signature.
    pub key: u16,
    /// Every attribute in stream order, handled or not.
    pub attributes: Vec<Attribute<'a>>,
    /// `(major, minor)` from `attTnefVersion`.
    pub version: Option<(u16, u16)>,
    pub from: Option<&'a [u8]>,
    pub subject: Option<&'a [u8]>,
    pub body: Option<&'a [u8]>,
    pub message_class: Option<&'a [u8]>,
    pub message_id: Option<&'a [u8]>,
    pub parent_id: Option<&'a [u8]>,
    pub original_message_class: Option<&'a [u8]>,
    /// Raw `attOemCodepage` payload; C stores it and never applies it.
    pub oem_codepage: Option<&'a [u8]>,
    /// `attPriority` value: 1 low, 2 normal, 3 high.
    pub priority: Option<u32>,
    pub date_sent: Option<Dtr>,
    pub date_received: Option<Dtr>,
    pub date_modified: Option<Dtr>,
    pub date_start: Option<Dtr>,
    pub date_end: Option<Dtr>,
    /// `attMAPIProps` properties; empty when absent.
    pub mapi: mapi::MapiProps<'a>,
    /// Attachment-level attributes seen before the first `attAttachRenddata`
    /// (C's `starting_attach`).
    pub head: Attachment<'a>,
    pub attachments: Vec<Attachment<'a>>,
}

impl<'a> Tnef<'a> {
    /// Attachment that attachment-level attributes apply to: the last one
    /// started, or [`Tnef::head`] before any.
    fn current_attachment(&mut self) -> &mut Attachment<'a> {
        match self.attachments.last_mut() {
            Some(a) => a,
            None => &mut self.head,
        }
    }
}

/// Parse a TNEF stream with default [`Options`] except for `limits`.
pub fn parse<'a>(buf: &'a [u8], limits: &Limits) -> Result<Tnef<'a>, Error> {
    parse_with(
        buf,
        &Options {
            limits: *limits,
            ..Options::default()
        },
    )
}

/// Parse a TNEF stream.
pub fn parse_with<'a>(buf: &'a [u8], opts: &Options) -> Result<Tnef<'a>, Error> {
    let l = &opts.limits;
    if buf.len() > l.max_input {
        return Err(Error::LimitExceeded {
            what: "input size",
            value: buf.len(),
            max: l.max_input,
        });
    }
    let mut r = Reader::new(buf);
    let sig = r.u32().map_err(|_| Error::NoSignature)?;
    if sig != TNEF_SIGNATURE {
        return Err(Error::NotTnefStream);
    }
    let key = r.u16().map_err(|_| Error::NoKey)?;
    let mut t = Tnef {
        key,
        ..Tnef::default()
    };
    while let Some(a) = attr::next(&mut r, opts)? {
        if t.attributes.len() >= l.max_attributes {
            return Err(Error::LimitExceeded {
                what: "attribute count",
                value: t.attributes.len(),
                max: l.max_attributes,
            });
        }
        attr::apply(&mut t, &a, l)?;
        t.attributes.push(a);
    }
    Ok(t)
}

#[cfg(test)]
mod tests;
