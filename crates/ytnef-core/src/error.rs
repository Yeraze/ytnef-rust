use crate::reader::Truncated;

/// Why parsing a TNEF stream failed. Variants follow `tnef-errors.h`;
/// [`Error::code`] gives the matching C return value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Input does not start with the TNEF signature `0x223E9F78`.
    #[error("not a TNEF stream")]
    NotTnefStream,
    /// Stream ends inside the signature (C: `YTNEF_ERROR_READING_DATA`).
    #[error("stream too short for a TNEF signature")]
    NoSignature,
    /// Stream ends before the 16-bit key.
    #[error("stream too short for a TNEF key")]
    NoKey,
    /// Stream ends inside an attribute payload or checksum.
    #[error("truncated attribute at offset {offset}: needed {needed} more bytes")]
    Truncated { offset: usize, needed: usize },
    /// Stream ends inside the 2-byte checksum after an attribute payload.
    #[error("attribute checksum truncated at offset {offset}")]
    NoChecksum { offset: usize },
    /// An attribute header declares a zero-length payload.
    #[error("attribute 0x{attr:08x} at offset {offset} has size 0")]
    EmptyAttribute { attr: u32, offset: usize },
    /// Stored attribute checksum differs from the sum of its payload bytes.
    #[error("attribute 0x{attr:08x} at offset {offset}: checksum 0x{stored:04x}, computed 0x{computed:04x}")]
    BadChecksum {
        attr: u32,
        offset: usize,
        stored: u16,
        computed: u16,
    },
    /// An attribute's payload is malformed for its type.
    #[error("attribute 0x{attr:08x} at offset {offset}: {reason}")]
    InHandler {
        attr: u32,
        offset: usize,
        reason: HandlerError,
    },
    /// A stream-level [`Limits`](crate::Limits) cap was hit.
    #[error("{what} {value} exceeds limit {max}")]
    LimitExceeded {
        what: &'static str,
        value: usize,
        max: usize,
    },
}

/// What was wrong with one attribute's payload.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum HandlerError {
    #[error("payload truncated at byte {offset} (needed {needed})")]
    Truncated { offset: usize, needed: usize },
    #[error("payload length {len} invalid for this attribute")]
    BadLength { len: usize },
    #[error("payload length {len} exceeds limit {max}")]
    TooLarge { len: usize, max: usize },
    /// A count or length that C's `PREALLOCCHECK` rejects (0 or above `max`).
    #[error("{what} {value} at byte {offset} outside 1..={max}")]
    OutOfRange {
        /// Which count or length: `"property count"`, `"value count"`,
        /// `"name count"`, `"name length"` or `"subject length"`.
        what: &'static str,
        offset: usize,
        value: u32,
        max: usize,
    },
    #[error("unknown MAPI property type 0x{prop_type:04x} at byte {offset}")]
    BadPropType { offset: usize, prop_type: u16 },
    #[error("{len} bytes left after the last MAPI property")]
    TrailingBytes { len: usize },
}

impl From<Truncated> for HandlerError {
    fn from(t: Truncated) -> Self {
        HandlerError::Truncated {
            offset: t.offset,
            needed: t.needed,
        }
    }
}

impl Error {
    /// The value the C `TNEFParse` returns for the same input.
    pub fn code(&self) -> i32 {
        match self {
            Error::NotTnefStream => -2,
            Error::NoSignature
            | Error::Truncated { .. }
            | Error::NoChecksum { .. }
            | Error::EmptyAttribute { .. } => -3,
            Error::NoKey => -4,
            Error::BadChecksum { .. } => -5,
            Error::InHandler { .. } => -6,
            // C's PREALLOCCHECK returns a bare -1 for an oversized attribute.
            Error::LimitExceeded { .. } => -1,
        }
    }
}

/// Why [`decompress_rtf`](crate::rtf::decompress_rtf) refused a value; each
/// is a case where C `DecompressRTF` returns NULL.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RtfError {
    #[error("{len} bytes is too short for compressed RTF")]
    TooShort { len: usize },
    #[error("header size {stored} does not match {len}-byte value")]
    SizeMismatch { stored: u32, len: usize },
    /// The raw size cannot be produced from the input.
    #[error("raw size {raw} impossible for the input")]
    BadRawSize { raw: u32 },
    #[error("raw size {len} exceeds limit {max}")]
    TooLarge { len: usize, max: usize },
    /// The stream ends inside a literal or reference.
    #[error("stream truncated at byte {offset}")]
    Truncated { offset: usize },
    #[error("unknown compression magic 0x{magic:08x}")]
    UnknownMagic { magic: u32 },
}
