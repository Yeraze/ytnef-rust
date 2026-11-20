//! Resource caps applied while parsing. Defaults match the C library where it
//! has a limit; the rest are new caps sized well above real-world messages.

const MIB: usize = 1024 * 1024;

/// Caps on input size, attribute sizes and counts.
///
/// Exceeding a `max_*` field is an error, never a silent truncation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Whole input stream (new; C has none). Default 1 GiB.
    pub max_input: usize,
    /// One attribute's payload. C: `attachmentSize` MiB, default 50 MiB.
    pub max_attr_size: usize,
    /// Attributes in one stream (new). Default 100 000.
    pub max_attributes: usize,
    /// Attachments in one stream (new). Default 10 000.
    pub max_attachments: usize,
    /// `attFrom` payload. C: 100.
    pub max_from: usize,
    /// `attSubject` payload. C: 1000.
    pub max_subject: usize,
    /// `attBody` payload. C: 100 000.
    pub max_body: usize,
    /// `attOriginalMessageClass` payload. C: 100.
    pub max_original_message_class: usize,
    /// `attAttachTitle` payload. C: 260.
    pub max_title: usize,
    /// `attAttachMetaFile` (icon) payload. C: 10 000.
    pub max_icon: usize,
    /// Decompressed RTF output (new). Default 64 MiB.
    pub max_rtf_out: usize,
    /// Embedded-TNEF recursion depth (new; C recursion is unbounded). Default 8.
    pub max_nesting: u8,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_input: 1024 * MIB,
            max_attr_size: 50 * MIB,
            max_attributes: 100_000,
            max_attachments: 10_000,
            max_from: 100,
            max_subject: 1000,
            max_body: 100_000,
            max_original_message_class: 100,
            max_title: 260,
            max_icon: 10_000,
            max_rtf_out: 64 * MIB,
            max_nesting: 8,
        }
    }
}

/// How to treat an attribute whose stored checksum does not match its payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Checksum {
    /// Fail with [`Error::BadChecksum`](crate::Error::BadChecksum), as C does.
    #[default]
    Strict,
    /// Keep going; the mismatch shows in [`Attribute::checksum_ok`](crate::Attribute::checksum_ok).
    Lenient,
}

/// Everything that tunes [`parse_with`](crate::parse_with).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Options {
    pub limits: Limits,
    pub checksum: Checksum,
}
