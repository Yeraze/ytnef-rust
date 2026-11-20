//! Bounded little-endian cursor. Every read checks length and returns `Result`.

/// A read ran past the end of its buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Truncated {
    /// Offset of the failed read, relative to the reader's buffer.
    pub offset: usize,
    /// Bytes the read asked for.
    pub needed: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(buf: &'a [u8]) -> Self {
        Reader { buf, pos: 0 }
    }

    pub(crate) fn pos(&self) -> usize {
        self.pos
    }

    pub(crate) fn remaining(&self) -> usize {
        self.buf.len().saturating_sub(self.pos)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    pub(crate) fn take(&mut self, n: usize) -> Result<&'a [u8], Truncated> {
        let short = Truncated {
            offset: self.pos,
            needed: n,
        };
        let end = self.pos.checked_add(n).ok_or(short)?;
        let s = self.buf.get(self.pos..end).ok_or(short)?;
        self.pos = end;
        Ok(s)
    }

    pub(crate) fn array<const N: usize>(&mut self) -> Result<[u8; N], Truncated> {
        let s = self.take(N)?;
        s.first_chunk::<N>().copied().ok_or(Truncated {
            offset: self.pos,
            needed: N,
        })
    }

    pub(crate) fn u8(&mut self) -> Result<u8, Truncated> {
        self.array::<1>().map(|[b]| b)
    }

    pub(crate) fn u16(&mut self) -> Result<u16, Truncated> {
        self.array().map(u16::from_le_bytes)
    }

    pub(crate) fn u32(&mut self) -> Result<u32, Truncated> {
        self.array().map(u32::from_le_bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_le_and_stops_at_end() {
        let mut r = Reader::new(&[1, 0, 2, 0, 0, 0, 9]);
        assert_eq!(r.u16(), Ok(1));
        assert_eq!(r.u32(), Ok(2));
        assert_eq!(r.remaining(), 1);
        assert_eq!(
            r.u16(),
            Err(Truncated {
                offset: 6,
                needed: 2
            })
        );
        // a failed read does not move the cursor
        assert_eq!(r.u8(), Ok(9));
        assert!(r.is_empty());
        assert!(r.take(usize::MAX).is_err());
    }
}
