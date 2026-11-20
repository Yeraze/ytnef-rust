//! TNEF (`winmail.dat`) parser. Pre-alpha: only the stream signature check exists so far.
#![forbid(unsafe_code)]
#![deny(
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

/// Little-endian magic at the start of every TNEF stream.
pub const TNEF_SIGNATURE: u32 = 0x223E_9F78;

/// True if `buf` starts with the TNEF signature.
pub fn has_signature(buf: &[u8]) -> bool {
    buf.first_chunk::<4>()
        .is_some_and(|b| u32::from_le_bytes(*b) == TNEF_SIGNATURE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature() {
        assert!(has_signature(&[0x78, 0x9F, 0x3E, 0x22, 0x00]));
        assert!(!has_signature(&[0x78, 0x9F, 0x3E]));
        assert!(!has_signature(b"PK\x03\x04"));
    }
}
