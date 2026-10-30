//! Descriptor primitives that every writer and reader of zmidi-usb shares (USB 2.0 9.5,
//! USB-MIDI v1.0 6.1.2.1, USB-MIDI v2.0 5.2 and 5.3; REQ-USB-MS1-006).
//!
//! - [`DescWriter`] fills a caller buffer; multi-byte fields are little-endian (USB 2.0 8.1,
//!   AMB-USB-MIDI-1-31, AMB-USB-MIDI-2-25). A failed write leaves the buffer content after the
//!   cursor unspecified; callers that must write nothing check the length first with
//!   [`DescWriter::remaining`].
//! - [`Descriptors`] walks a caller slice by bLength. A bLength below 2 or past the end gives
//!   one error item and ends the walk; nothing panics on received bytes (U-24).
//! - [`interfaces`] splits a configuration into groups that start at a standard interface
//!   descriptor (one group for each Alternate Setting).
//!
//! No alloc (DEC-002), no I/O (DEC-004, DEC-014).

use crate::constants::INTERFACE;

/// Error of the descriptor primitives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum WalkError {
    /// The caller buffer is too small.
    #[error("buffer too small")]
    Overflow,
    /// A bLength below 2: the walk cannot move forward.
    #[error("bLength below 2")]
    LengthTooSmall,
    /// A bLength that goes past the end of the slice.
    #[error("descriptor goes past the end of the slice")]
    Truncated,
}

/// A cursor that writes descriptor bytes into a caller buffer.
#[derive(Debug)]
pub struct DescWriter<'b> {
    buf: &'b mut [u8],
    pos: usize,
}

impl<'b> DescWriter<'b> {
    /// A writer at the start of `buf`.
    pub fn new(buf: &'b mut [u8]) -> Self {
        DescWriter { buf, pos: 0 }
    }

    /// The number of bytes written.
    pub fn len(&self) -> usize {
        self.pos
    }

    /// True when nothing is written.
    pub fn is_empty(&self) -> bool {
        self.pos == 0
    }

    /// The free bytes after the cursor.
    pub fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    /// Writes one byte.
    pub fn u8(&mut self, b: u8) -> Result<(), WalkError> {
        let slot = self.buf.get_mut(self.pos).ok_or(WalkError::Overflow)?;
        *slot = b;
        self.pos += 1;
        Ok(())
    }

    /// Writes a 16-bit value, least significant byte first.
    pub fn u16_le(&mut self, v: u16) -> Result<(), WalkError> {
        self.bytes(&v.to_le_bytes())
    }

    /// Writes bytes.
    pub fn bytes(&mut self, b: &[u8]) -> Result<(), WalkError> {
        let end = self.pos.checked_add(b.len()).ok_or(WalkError::Overflow)?;
        let dst = self.buf.get_mut(self.pos..end).ok_or(WalkError::Overflow)?;
        dst.copy_from_slice(b);
        self.pos = end;
        Ok(())
    }

    /// Reserves a 16-bit field and gives its position for [`Self::patch_u16_le`].
    pub fn reserve_u16(&mut self) -> Result<usize, WalkError> {
        let at = self.pos;
        self.u16_le(0)?;
        Ok(at)
    }

    /// Writes a 16-bit value at a position that [`Self::reserve_u16`] gave.
    pub fn patch_u16_le(&mut self, at: usize, v: u16) -> Result<(), WalkError> {
        let dst = self
            .buf
            .get_mut(at..at.saturating_add(2))
            .filter(|d| d.len() == 2 && at + 2 <= self.pos)
            .ok_or(WalkError::Overflow)?;
        dst.copy_from_slice(&v.to_le_bytes());
        Ok(())
    }

    /// The bytes written.
    pub fn written(&self) -> &[u8] {
        &self.buf[..self.pos]
    }
}

/// One descriptor of a slice: its bytes from bLength on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawDesc<'a> {
    bytes: &'a [u8],
}

impl<'a> RawDesc<'a> {
    /// All bytes of the descriptor (bLength of them).
    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    /// bLength.
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Never true: a descriptor has at least 2 bytes.
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// bDescriptorType (offset 1).
    pub fn dtype(&self) -> u8 {
        self.byte(1).unwrap_or(0)
    }

    /// bDescriptorSubtype (offset 2), when the descriptor has it.
    pub fn subtype(&self) -> Option<u8> {
        self.byte(2)
    }

    /// The byte at `offset`, when the descriptor has it.
    pub fn byte(&self, offset: usize) -> Option<u8> {
        self.bytes.get(offset).copied()
    }

    /// The little-endian 16-bit field at `offset`, when the descriptor has it.
    pub fn u16_le(&self, offset: usize) -> Option<u16> {
        let lo = self.byte(offset)?;
        let hi = self.byte(offset + 1)?;
        Some(u16::from_le_bytes([lo, hi]))
    }

    /// True for a standard interface descriptor.
    pub fn is_interface(&self) -> bool {
        self.dtype() == INTERFACE
    }
}

/// An iterator over the descriptors of a slice, by bLength (REQ-USB-MS1-006).
///
/// Each item is a descriptor or one final error: a bLength below 2 or a bLength past the end
/// ends the walk. The iterator does not look into a descriptor; the reader of the caller
/// identifies it by type and subtype and skips what it does not know.
#[derive(Debug, Clone)]
pub struct Descriptors<'a> {
    rest: &'a [u8],
    done: bool,
}

impl<'a> Descriptors<'a> {
    /// A walk over `bytes`.
    pub fn new(bytes: &'a [u8]) -> Self {
        Descriptors {
            rest: bytes,
            done: false,
        }
    }
}

impl<'a> Iterator for Descriptors<'a> {
    type Item = Result<RawDesc<'a>, WalkError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done || self.rest.is_empty() {
            return None;
        }
        let len = usize::from(self.rest[0]);
        if len < 2 {
            self.done = true;
            return Some(Err(WalkError::LengthTooSmall));
        }
        if len > self.rest.len() {
            self.done = true;
            return Some(Err(WalkError::Truncated));
        }
        let (d, rest) = self.rest.split_at(len);
        self.rest = rest;
        Some(Ok(RawDesc { bytes: d }))
    }
}

/// The descriptors of a configuration in groups: each group starts at a standard interface
/// descriptor and ends before the next one, or at the end of the slice. Descriptors before the
/// first interface descriptor (for example the Configuration descriptor) are not in a group.
pub fn interfaces(bytes: &[u8]) -> Interfaces<'_> {
    Interfaces {
        walk: Descriptors::new(bytes),
        bytes,
        pending: None,
        error: None,
    }
}

/// One group of [`interfaces`]: the interface descriptor and the slice of the descriptors
/// after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InterfaceGroup<'a> {
    /// The standard interface descriptor.
    pub interface: RawDesc<'a>,
    /// The descriptors after it, up to the next interface descriptor.
    pub body: &'a [u8],
}

/// Iterator of [`interfaces`]. A walk error ends it; [`Interfaces::error`] gives the error.
#[derive(Debug, Clone)]
pub struct Interfaces<'a> {
    walk: Descriptors<'a>,
    bytes: &'a [u8],
    pending: Option<RawDesc<'a>>,
    error: Option<WalkError>,
}

impl<'a> Interfaces<'a> {
    /// The walk error that ended the iteration, if any.
    pub fn error(&self) -> Option<WalkError> {
        self.error
    }

    fn offset_of(&self, d: &RawDesc<'a>) -> usize {
        d.bytes.as_ptr() as usize - self.bytes.as_ptr() as usize
    }
}

impl<'a> Iterator for Interfaces<'a> {
    type Item = InterfaceGroup<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let head = match self.pending.take() {
            Some(d) => d,
            None => loop {
                match self.walk.next()? {
                    Ok(d) if d.is_interface() => break d,
                    Ok(_) => {}
                    Err(e) => {
                        self.error = Some(e);
                        return None;
                    }
                }
            },
        };
        let start = self.offset_of(&head) + head.len();
        let mut end = start;
        loop {
            match self.walk.next() {
                None => break,
                Some(Ok(d)) if d.is_interface() => {
                    self.pending = Some(d);
                    break;
                }
                Some(Ok(d)) => end = self.offset_of(&d) + d.len(),
                Some(Err(e)) => {
                    self.error = Some(e);
                    break;
                }
            }
        }
        Some(InterfaceGroup {
            interface: head,
            body: &self.bytes[start..end],
        })
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;

    #[test]
    fn writer_cursor_and_patch_limits() {
        let mut buf = [0u8; 8];
        let mut w = DescWriter::new(&mut buf);
        assert!(w.is_empty());
        assert_eq!(w.remaining(), 8);
        w.u8(1).unwrap();
        assert!(!w.is_empty());
        assert_eq!(w.remaining(), 7);
        // A patch only over bytes written before: 2 bytes at 0 need 2 written bytes.
        assert_eq!(w.patch_u16_le(0, 0x1234), Err(WalkError::Overflow));
        let at = w.reserve_u16().unwrap();
        assert_eq!(at, 1);
        w.patch_u16_le(at, 0x1234).unwrap();
        assert_eq!(w.written(), &[1, 0x34, 0x12]);
        // A descriptor has bytes.
        let d = Descriptors::new(&[2, 0x24]).next().unwrap().unwrap();
        assert!(!d.is_empty());
    }
    use proptest::prelude::*;
    use std::vec::Vec;

    #[test]
    fn writer_little_endian_and_patch() {
        let mut buf = [0u8; 6];
        let mut w = DescWriter::new(&mut buf);
        w.u8(7).unwrap();
        let at = w.reserve_u16().unwrap();
        w.u16_le(0x0200).unwrap();
        w.patch_u16_le(at, 0x0041).unwrap();
        assert_eq!(w.written(), &[7, 0x41, 0x00, 0x00, 0x02]);
        assert_eq!(w.remaining(), 1);
        assert_eq!(w.u16_le(1), Err(WalkError::Overflow));
        assert_eq!(w.patch_u16_le(4, 1), Err(WalkError::Overflow));
    }

    #[test]
    fn walk_by_blength_in_any_order() {
        // REQ-USB-MS1-006: walk by bLength, identify by type and subtype, unknown and 0x26 are
        // given to the caller, who skips them.
        let bytes = [
            9, 4, 1, 0, 2, 1, 3, 0, 0, // interface
            6, 0x24, 2, 1, 1, 0, // MIDI IN Jack
            3, 0x26, 1, // a CS_GR_TRM_BLOCK descriptor
            2, 0x99, // unknown type, 2 bytes
            7, 0x24, 1, 0, 1, 0x41, 0, // MS header after the jack
        ];
        let d: Vec<_> = Descriptors::new(&bytes).map(|r| r.unwrap()).collect();
        assert_eq!(d.len(), 5);
        assert_eq!((d[1].dtype(), d[1].subtype()), (0x24, Some(2)));
        assert_eq!(d[2].dtype(), 0x26);
        assert_eq!((d[3].dtype(), d[3].subtype()), (0x99, None));
        assert_eq!(d[4].u16_le(5), Some(0x0041));
        assert_eq!(d[4].u16_le(6), None);
    }

    #[test]
    fn walk_stops_at_bad_blength() {
        // REQ-USB-MS1-006: bLength below 2 or past the end: error, stop.
        let r: Vec<_> = Descriptors::new(&[2, 5, 1, 7]).collect();
        assert_eq!(r.len(), 2);
        assert_eq!(r[1], Err(WalkError::LengthTooSmall));
        let r: Vec<_> = Descriptors::new(&[2, 5, 9, 4, 0]).collect();
        assert_eq!(r[1], Err(WalkError::Truncated));
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn interface_groups() {
        // REQ-USB-MS1-006: an Alternate Setting runs to the next interface descriptor.
        let bytes = [
            9, 2, 0, 0, 2, 1, 0, 0x80, 0x32, // configuration
            9, 4, 0, 0, 0, 1, 1, 0, 0, // AC interface
            9, 0x24, 1, 0, 1, 9, 0, 1, 1, // AC header
            9, 4, 1, 0, 2, 1, 3, 0, 0, // MS alt 0
            7, 0x24, 1, 0, 1, 7, 0, // header
            9, 4, 1, 1, 2, 1, 3, 0, 0, // MS alt 1
        ];
        let groups: Vec<_> = interfaces(&bytes).collect();
        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0].body.len(), 9);
        assert_eq!(groups[1].body, &[7, 0x24, 1, 0, 1, 7, 0]);
        assert_eq!(groups[2].interface.byte(3), Some(1));
        assert!(groups[2].body.is_empty());
        let mut it = interfaces(&[9, 4, 1, 0, 2, 1, 3, 0, 0, 1]);
        assert!(it.next().is_some());
        assert_eq!(it.error(), Some(WalkError::LengthTooSmall));
    }

    #[test]
    fn prop_walk_never_panics() {
        proptest!(|(bytes in proptest::collection::vec(any::<u8>(), 0..300))| {
            let mut total = 0;
            for d in Descriptors::new(&bytes).flatten() {
                total += d.len();
                let _ = (d.dtype(), d.subtype(), d.u16_le(3));
            }
            prop_assert!(total <= bytes.len());
            for g in interfaces(&bytes) {
                prop_assert!(g.interface.is_interface());
            }
        });
    }
}
