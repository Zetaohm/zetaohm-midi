//! Group Terminal Blocks of a USB MIDI 2.0 function (USB-MIDI v2.0 sections 3.3, 4.1, 4.2, 5.4
//! and Appendix A, B).
//!
//! - [`GroupTerminalBlock`]: the model of one Group Terminal Block descriptor (Table 5-6,
//!   REQ-USB-GTB-003). The value sets [`GtbType`] and [`MidiProtocol`] come from
//!   [`crate::constants`].
//! - [`GtbList`]: the blocks of one USB MIDI 2.0 Alternate Setting in a fixed capacity `B`
//!   (REQ-USB-GTB-001). [`GtbList::default_list`] is the default list of choice U-09.
//! - [`GtbList::write`]: the writer of the Group Terminal Block descriptor set of one Alternate
//!   Setting into a caller buffer that is separate from the Configuration descriptor set
//!   (REQ-USB-GTB-002, REQ-USB-GTB-015, choice U-12). It checks every rule first and writes
//!   nothing when a rule is broken or the buffer is too small.
//! - [`GtbReader`] and [`read_set`]: the lenient readers (REQ-USB-GTB-016, choices U-16, U-24):
//!   each break of a rule is a value in the result, and the descriptors are kept.
//! - [`EndpointBlocks`]: the view of an Ms2 endpoint (direction and block IDs) that the checks
//!   between endpoints and blocks use (REQ-USB-GTB-013, REQ-USB-GTB-014, REQ-USB-GEN-010).
//! - [`GroupMask`]: the Group Terminals of one endpoint direction as a 16-bit mask, for the UMP
//!   reader (REQ-USB-UMP-013, choice U-05).
//! - [`ProtocolHint`]: bMIDIProtocol converted to the Protocol and the JR Timestamps flag of the
//!   M2-104 Stream Configuration messages (REQ-USB-GTB-011).
//!
//! An IN Group Terminal is host to device (data from a MIDI OUT endpoint). An OUT Group
//! Terminal is device to host (data to a MIDI IN endpoint) (REQ-USB-GTB-005). The Group field
//! value n of a UMP is Group Terminal Number n, shown to users as Group Terminal n + 1
//! (REQ-USB-UMP-013). Every 2-byte field is little-endian (AMB-USB-MIDI-2-25).
//!
//! No alloc (DEC-002), no I/O, no clock and no USB stack (DEC-004, DEC-014). Nothing panics on
//! received bytes.

use core::fmt;
use core::ops::Deref;

use zmidi_ump::{Group, StreamProtocol};

use crate::constants::{
    EndpointDirection, GtbType, MidiProtocol, CS_GR_TRM_BLOCK, GR_TRM_BLOCK, GR_TRM_BLOCK_HEADER,
};
use crate::ms2::{EndpointConfig, FixedList, Ms2Endpoint};
use crate::walk::{DescWriter, Descriptors, RawDesc, WalkError};

/// bLength of the Group Terminal Block Header descriptor (Table 5-5).
pub const HEADER_LEN: usize = 5;
/// bLength of the Group Terminal Block descriptor (Table 5-6).
pub const BLOCK_LEN: usize = 13;
/// The number of Group Terminal Numbers (0x0 to 0xF).
pub const GROUP_TERMINALS: u8 = 16;
/// Bytes per second of one bandwidth unit (5.4.2, AMB-USB-MIDI-2-21).
pub const BANDWIDTH_UNIT: u32 = 4000;
/// The largest number of problems that [`GtbSetInfo`] keeps.
pub const MAX_PROBLEMS: usize = 16;

/// Error of the Group Terminal Block writer, reader and checks. Each variant names the broken
/// rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum GtbError {
    /// The caller buffer is too small; nothing is written (REQ-USB-GTB-015).
    #[error("buffer too small")]
    Overflow,
    /// More blocks than the capacity of the list, or a set longer than 65535 bytes.
    #[error("too many Group Terminal Blocks")]
    TooManyBlocks,
    /// An Alternate Setting with no Group Terminal Block (REQ-USB-GTB-001).
    #[error("no Group Terminal Block")]
    NoBlock,
    /// A bGrpTrmBlkID of 0x00 (REQ-USB-GTB-004).
    #[error("bGrpTrmBlkID 0x00")]
    BlockIdZero,
    /// A second block with the same bGrpTrmBlkID; the reader keeps the first one
    /// (REQ-USB-GTB-004).
    #[error("bGrpTrmBlkID {0} used twice")]
    DuplicateBlockId(u8),
    /// A bGrpTrmBlkType that is not 0x00, 0x01 or 0x02; only the writer refuses it
    /// (REQ-USB-GTB-005, choice U-15).
    #[error("block {id}: bGrpTrmBlkType {raw:#04x} not defined")]
    UnknownType {
        /// bGrpTrmBlkID.
        id: u8,
        /// The raw type value.
        raw: u8,
    },
    /// A bMIDIProtocol that is not defined; only the writer refuses it (REQ-USB-GTB-009,
    /// choice U-15).
    #[error("block {id}: bMIDIProtocol {raw:#04x} not defined")]
    UnknownProtocol {
        /// bGrpTrmBlkID.
        id: u8,
        /// The raw protocol value.
        raw: u8,
    },
    /// nGroupTrm above 0x0F, nNumGroupTrm 0, or nGroupTrm + nNumGroupTrm above 16
    /// (REQ-USB-GTB-006).
    #[error("block {0}: Group Terminal range not in 0x0 to 0xF")]
    GroupRange(u8),
    /// A Group Terminal Number in more than one block in one direction (REQ-USB-GEN-011,
    /// choice U-03). `id` is the later block.
    #[error("block {id}: {direction} Group Terminal {} is in an other block", .group + 1)]
    Overlap {
        /// bGrpTrmBlkID of the later block.
        id: u8,
        /// The lowest Group Terminal Number in both blocks.
        group: u8,
        /// The direction in which the blocks overlap.
        direction: TerminalDirection,
    },
    /// A single bidirectional block with all 16 Groups and a bMIDIProtocol with no 128-bit
    /// UMPs (0x00, 0x01, 0x02) (REQ-USB-001, REQ-USB-GTB-010, choice U-11).
    #[error("block {0}: all 16 Groups need a protocol with 128-bit UMPs")]
    Needs128BitProtocol(u8),
    /// An endpoint names a block ID that has no Group Terminal Block descriptor
    /// (REQ-USB-GTB-013).
    #[error("endpoint names block {0}, which has no descriptor")]
    UnknownBlockId(u8),
    /// A MIDI IN endpoint names a block of type 0x01, or a MIDI OUT endpoint names a block of
    /// type 0x02 (REQ-USB-GEN-010, choice U-07).
    #[error("{endpoint:?} endpoint names block {id} of the wrong direction")]
    WrongDirection {
        /// bGrpTrmBlkID.
        id: u8,
        /// The direction of the endpoint.
        endpoint: EndpointDirection,
    },
    /// No endpoint of the needed direction names the block: a MIDI OUT endpoint for IN Group
    /// Terminals, a MIDI IN endpoint for OUT Group Terminals (REQ-USB-GEN-010, choice U-07).
    #[error("no {endpoint:?} endpoint names block {id}")]
    NotNamed {
        /// bGrpTrmBlkID.
        id: u8,
        /// The direction of the missing endpoint.
        endpoint: EndpointDirection,
    },
    /// More than one endpoint of one direction names the block (REQ-USB-GEN-010).
    #[error("more than one {endpoint:?} endpoint names block {id}")]
    NamedTwice {
        /// bGrpTrmBlkID.
        id: u8,
        /// The direction of the endpoints.
        endpoint: EndpointDirection,
    },
    /// An endpoint whose blocks give 0 different Group Terminal Numbers in its direction
    /// (REQ-USB-GTB-014). `endpoint` is the position of the endpoint in the caller list.
    #[error("endpoint {endpoint} has no Group Terminal")]
    EndpointWithoutGroupTerminals {
        /// The position of the endpoint in the list that the caller gave.
        endpoint: usize,
    },
    /// The set does not start with a Group Terminal Block Header descriptor (REQ-USB-GTB-016).
    #[error("no Group Terminal Block Header first")]
    NoHeader,
    /// A header with bLength below 5, or a wTotalLength below the header (REQ-USB-GTB-016).
    #[error("Group Terminal Block Header too short")]
    HeaderTooShort,
    /// A block descriptor with bLength below 13; the reader skips it (REQ-USB-GTB-016).
    #[error("Group Terminal Block descriptor bLength below 13")]
    BlockTooShort,
    /// The descriptor walk stopped: `WalkError::Truncated` when the slice or wTotalLength
    /// cuts a descriptor, `WalkError::LengthTooSmall` for a bLength below 2. The blocks before
    /// it are kept (REQ-USB-GTB-016, REQ-USB-MS1-006, choice U-16). Never
    /// `WalkError::Overflow`: that is [`GtbError::Overflow`].
    #[error("descriptor walk: {0}")]
    Walk(WalkError),
}

impl From<WalkError> for GtbError {
    fn from(e: WalkError) -> Self {
        match e {
            WalkError::Overflow => GtbError::Overflow,
            other => GtbError::Walk(other),
        }
    }
}

/// The direction of a Group Terminal (REQ-USB-GTB-005).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TerminalDirection {
    /// IN Group Terminal: host to device, data from a MIDI OUT endpoint.
    In,
    /// OUT Group Terminal: device to host, data to a MIDI IN endpoint.
    Out,
}

impl TerminalDirection {
    /// The Group Terminal direction that an endpoint of direction `ep` carries.
    pub const fn of_endpoint(ep: EndpointDirection) -> Self {
        match ep {
            EndpointDirection::MidiOut => TerminalDirection::In,
            EndpointDirection::MidiIn => TerminalDirection::Out,
        }
    }

    /// The endpoint direction that carries this Group Terminal direction.
    pub const fn endpoint(self) -> EndpointDirection {
        match self {
            TerminalDirection::In => EndpointDirection::MidiOut,
            TerminalDirection::Out => EndpointDirection::MidiIn,
        }
    }
}

impl fmt::Display for TerminalDirection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            TerminalDirection::In => "IN",
            TerminalDirection::Out => "OUT",
        })
    }
}

/// One Group Terminal: a Group Terminal Number (0x0 to 0xF) and a direction (3.3.3,
/// REQ-USB-UMP-013).
///
/// Group Terminal Number n carries the UMPs with Group field value n, one to one. Users see it
/// as Group Terminal n + 1, which carries Group n + 1 (`Display` gives "IN Group Terminal 3").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GroupTerminal {
    /// The Group Terminal Number, 0x0 to 0xF (the Group field value).
    pub number: u8,
    /// IN (host to device) or OUT (device to host).
    pub direction: TerminalDirection,
}

impl GroupTerminal {
    /// The Group Terminal of the Group field of a UMP in `direction` (REQ-USB-UMP-013).
    pub const fn from_group(group: Group, direction: TerminalDirection) -> Self {
        GroupTerminal {
            number: group.index(),
            direction,
        }
    }

    /// The Group of the UMPs on this Group Terminal (the low 4 bits of the number).
    pub const fn group(self) -> Group {
        Group::from_bits(self.number)
    }

    /// The number that users see: Group Terminal Number + 1 (1 to 16).
    pub const fn user_number(self) -> u8 {
        (self.number & 0x0F) + 1
    }
}

impl fmt::Display for GroupTerminal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} Group Terminal {}",
            self.direction,
            self.user_number()
        )
    }
}

/// bMIDIProtocol as the Protocol and the JR Timestamps flag of the M2-104 Stream
/// Configuration messages (REQ-USB-GTB-011). The crate does not use MIDI-CI Protocol
/// Negotiation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProtocolHint {
    /// MIDI 1.0 (0x01) or MIDI 2.0 (0x02).
    pub protocol: StreamProtocol,
    /// True for the "Jitter Reduction Timestamps" values 0x02, 0x04 and 0x12.
    pub jr_timestamps: bool,
}

impl ProtocolHint {
    /// The hint of a bMIDIProtocol value: 0x01 to 0x04 give MIDI 1.0, 0x11 and 0x12 give
    /// MIDI 2.0, 0x02, 0x04 and 0x12 set the flag; 0x00 and other values give `None`
    /// (REQ-USB-GTB-009, REQ-USB-GTB-011).
    pub const fn from_protocol(p: MidiProtocol) -> Option<Self> {
        let (protocol, jr_timestamps) = match p {
            MidiProtocol::Midi1UpTo64 | MidiProtocol::Midi1UpTo128 => {
                (StreamProtocol::Midi1, false)
            }
            MidiProtocol::Midi1UpTo64Jr | MidiProtocol::Midi1UpTo128Jr => {
                (StreamProtocol::Midi1, true)
            }
            MidiProtocol::Midi2 => (StreamProtocol::Midi2, false),
            MidiProtocol::Midi2Jr => (StreamProtocol::Midi2, true),
            MidiProtocol::UseMidiCi | MidiProtocol::Unknown(_) => return None,
        };
        Some(ProtocolHint {
            protocol,
            jr_timestamps,
        })
    }
}

/// The bandwidth field value (wMaxInputBandwidth, wMaxOutputBandwidth) for a rate in bytes
/// per second: units of 4000 bytes per second, rounded up, at most 0xFFFF; 0 gives 0x0000
/// (unknown) (REQ-USB-GTB-012, AMB-USB-MIDI-2-21). The MIDI 1.0 rate of 31.25 kbit/s (3906
/// bytes per second) gives 0x0001.
pub const fn bandwidth_from_bytes_per_second(bytes_per_second: u32) -> u16 {
    let units = bytes_per_second.div_ceil(BANDWIDTH_UNIT);
    if units > 0xFFFF {
        0xFFFF
    } else {
        units as u16
    }
}

/// The view of a MIDI Streaming Data Endpoint that the block checks need: its direction and
/// the block IDs of its class-specific endpoint descriptor (REQ-USB-GTB-013).
pub trait EndpointBlocks {
    /// MIDI OUT (host to device) or MIDI IN (device to host).
    fn direction(&self) -> EndpointDirection;
    /// baAssoGrpTrmBlkID.
    fn block_ids(&self) -> &[u8];
}

impl EndpointBlocks for EndpointConfig {
    fn direction(&self) -> EndpointDirection {
        EndpointConfig::direction(self)
    }
    fn block_ids(&self) -> &[u8] {
        EndpointConfig::block_ids(self)
    }
}

impl EndpointBlocks for Ms2Endpoint {
    fn direction(&self) -> EndpointDirection {
        Ms2Endpoint::direction(self)
    }
    fn block_ids(&self) -> &[u8] {
        Ms2Endpoint::block_ids(self)
    }
}

impl<T: EndpointBlocks + ?Sized> EndpointBlocks for &T {
    fn direction(&self) -> EndpointDirection {
        (**self).direction()
    }
    fn block_ids(&self) -> &[u8] {
        (**self).block_ids()
    }
}

/// One Group Terminal Block: the fields of Table 5-6 (REQ-USB-GTB-001, REQ-USB-GTB-003).
///
/// The reader keeps every field raw; [`Self::validate`] gives the rules that the writer
/// checks for one block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GroupTerminalBlock {
    /// bGrpTrmBlkID, 1 to 255 (0x00 is refused, REQ-USB-GTB-004).
    pub id: u8,
    /// bGrpTrmBlkType (REQ-USB-GTB-005).
    pub kind: GtbType,
    /// nGroupTrm: the first member Group Terminal Number, 0x0 to 0xF (REQ-USB-GTB-006).
    pub first_group: u8,
    /// nNumGroupTrm: the number of member Group Terminal Numbers, 1 to 16 - `first_group`.
    pub num_groups: u8,
    /// iBlockItem: the string index; 0 = no string (REQ-USB-GEN-012).
    pub string_index: u8,
    /// bMIDIProtocol (REQ-USB-GTB-009, REQ-USB-GTB-010).
    pub protocol: MidiProtocol,
    /// wMaxInputBandwidth for all IN Group Terminals: 0 unknown, else units of 4000 bytes per
    /// second (REQ-USB-GTB-012).
    pub max_input_bandwidth: u16,
    /// wMaxOutputBandwidth for all OUT Group Terminals (REQ-USB-GTB-012).
    pub max_output_bandwidth: u16,
}

impl Default for GroupTerminalBlock {
    /// [`GroupTerminalBlock::DEFAULT`] (choice U-09).
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl GroupTerminalBlock {
    /// The default block of choice U-09: ID 1, bidirectional, Group Terminal 1 only,
    /// iBlockItem 0, bMIDIProtocol 0x11, both bandwidth fields 0x0000 (REQ-USB-GTB-001).
    pub const DEFAULT: GroupTerminalBlock = GroupTerminalBlock {
        id: 1,
        kind: GtbType::Bidirectional,
        first_group: 0,
        num_groups: 1,
        string_index: 0,
        protocol: MidiProtocol::Midi2,
        max_input_bandwidth: 0,
        max_output_bandwidth: 0,
    };

    /// A block with the given members, iBlockItem 0, bMIDIProtocol 0x11 and bandwidth 0x0000
    /// (REQ-USB-GTB-010, REQ-USB-GTB-012). The writer checks the values.
    pub const fn new(id: u8, kind: GtbType, first_group: u8, num_groups: u8) -> Self {
        GroupTerminalBlock {
            id,
            kind,
            first_group,
            num_groups,
            ..Self::DEFAULT
        }
    }

    /// True when nGroupTrm is 0x0 to 0xF, nNumGroupTrm is 1 or more and nGroupTrm +
    /// nNumGroupTrm is 16 or less (REQ-USB-GTB-006).
    pub const fn range_is_valid(&self) -> bool {
        self.first_group <= 0x0F
            && self.num_groups != 0
            && self.first_group as u16 + self.num_groups as u16 <= GROUP_TERMINALS as u16
    }

    /// The member Group Terminal Numbers nGroupTrm to nGroupTrm + nNumGroupTrm - 1 as a mask
    /// (bit n = number n); 0 when the range is not valid (REQ-USB-GTB-006).
    pub const fn member_mask(&self) -> u16 {
        if !self.range_is_valid() {
            return 0;
        }
        (((1u32 << self.num_groups) - 1) << self.first_group) as u16
    }

    /// The IN Group Terminals (host to device) as a mask; 0 for type 0x02, an other type or a
    /// range that is not valid (REQ-USB-GTB-005).
    pub const fn in_mask(&self) -> u16 {
        if self.kind.has_in() {
            self.member_mask()
        } else {
            0
        }
    }

    /// The OUT Group Terminals (device to host) as a mask; 0 for type 0x01, an other type or
    /// a range that is not valid (REQ-USB-GTB-005).
    pub const fn out_mask(&self) -> u16 {
        if self.kind.has_out() {
            self.member_mask()
        } else {
            0
        }
    }

    /// The Group Terminals of one direction as a mask.
    pub const fn mask(&self, direction: TerminalDirection) -> u16 {
        match direction {
            TerminalDirection::In => self.in_mask(),
            TerminalDirection::Out => self.out_mask(),
        }
    }

    /// The number of Group Terminals: nNumGroupTrm for type 0x01 or 0x02, 2 x nNumGroupTrm for
    /// type 0x00, 0 for an other type or a range that is not valid (REQ-USB-GTB-007).
    pub const fn group_terminal_count(&self) -> u8 {
        (self.in_mask().count_ones() + self.out_mask().count_ones()) as u8
    }

    /// The Group Terminals of the block: the IN Group Terminals in number order, then the OUT
    /// Group Terminals (REQ-USB-GTB-007, REQ-USB-GTB-008).
    pub fn group_terminals(&self) -> GroupTerminals {
        GroupTerminals {
            in_mask: self.in_mask(),
            out_mask: self.out_mask(),
        }
    }

    /// True when the block is the M2-104 Appendix I strategy: bidirectional with all 16
    /// Groups (REQ-USB-001).
    pub const fn spans_all_groups(&self) -> bool {
        matches!(self.kind, GtbType::Bidirectional)
            && self.first_group == 0
            && self.num_groups == GROUP_TERMINALS
    }

    /// The bMIDIProtocol that the crate uses: an other value is read as 0x00 (unknown)
    /// (REQ-USB-GTB-009).
    pub const fn effective_protocol(&self) -> MidiProtocol {
        match self.protocol {
            MidiProtocol::Unknown(_) => MidiProtocol::UseMidiCi,
            p => p,
        }
    }

    /// The Stream Configuration Protocol and JR Timestamps flag of bMIDIProtocol
    /// (REQ-USB-GTB-011).
    pub const fn protocol_hint(&self) -> Option<ProtocolHint> {
        ProtocolHint::from_protocol(self.protocol)
    }

    /// Checks the rules of one block that the writer refuses: an other type or protocol value
    /// (choice U-15), ID 0x00 and the Group Terminal range (REQ-USB-GTB-004 to
    /// REQ-USB-GTB-006, REQ-USB-GTB-009).
    pub const fn validate(&self) -> Result<(), GtbError> {
        if let GtbType::Unknown(raw) = self.kind {
            return Err(GtbError::UnknownType { id: self.id, raw });
        }
        if let MidiProtocol::Unknown(raw) = self.protocol {
            return Err(GtbError::UnknownProtocol { id: self.id, raw });
        }
        if self.id == 0 {
            return Err(GtbError::BlockIdZero);
        }
        if !self.range_is_valid() {
            return Err(GtbError::GroupRange(self.id));
        }
        Ok(())
    }

    /// The 13 bytes of the descriptor (Table 5-6), 2-byte fields least significant byte first
    /// (REQ-USB-GTB-003). It does not check the values.
    pub const fn to_bytes(&self) -> [u8; BLOCK_LEN] {
        let i = self.max_input_bandwidth.to_le_bytes();
        let o = self.max_output_bandwidth.to_le_bytes();
        [
            BLOCK_LEN as u8,
            CS_GR_TRM_BLOCK,
            GR_TRM_BLOCK,
            self.id,
            self.kind.into_bits(),
            self.first_group,
            self.num_groups,
            self.string_index,
            self.protocol.into_bits(),
            i[0],
            i[1],
            o[0],
            o[1],
        ]
    }

    /// The block from the first 13 bytes of a descriptor; the rest is ignored. It keeps every
    /// value raw (REQ-USB-GTB-016, choice U-16).
    pub const fn from_bytes(b: &[u8; BLOCK_LEN]) -> Self {
        GroupTerminalBlock {
            id: b[3],
            kind: GtbType::from_bits(b[4]),
            first_group: b[5],
            num_groups: b[6],
            string_index: b[7],
            protocol: MidiProtocol::from_bits(b[8]),
            max_input_bandwidth: u16::from_le_bytes([b[9], b[10]]),
            max_output_bandwidth: u16::from_le_bytes([b[11], b[12]]),
        }
    }

    /// Reads a Group Terminal Block descriptor (REQ-USB-GTB-016). Gives `None` when the
    /// descriptor is not one (type 0x26, subtype 0x02), and [`GtbError::BlockTooShort`] for a
    /// bLength below 13. A longer descriptor is accepted (choice U-16).
    pub fn read(d: &RawDesc<'_>) -> Option<Result<GroupTerminalBlock, GtbError>> {
        if d.dtype() != CS_GR_TRM_BLOCK || d.subtype()? != GR_TRM_BLOCK {
            return None;
        }
        let Some(b) = d
            .bytes()
            .get(..BLOCK_LEN)
            .and_then(|b| <&[u8; BLOCK_LEN]>::try_from(b).ok())
        else {
            return Some(Err(GtbError::BlockTooShort));
        };
        Some(Ok(Self::from_bytes(b)))
    }
}

/// Iterator of the Group Terminals of one block ([`GroupTerminalBlock::group_terminals`]).
#[derive(Debug, Clone)]
pub struct GroupTerminals {
    in_mask: u16,
    out_mask: u16,
}

impl Iterator for GroupTerminals {
    type Item = GroupTerminal;

    fn next(&mut self) -> Option<GroupTerminal> {
        let (mask, direction) = if self.in_mask != 0 {
            (&mut self.in_mask, TerminalDirection::In)
        } else if self.out_mask != 0 {
            (&mut self.out_mask, TerminalDirection::Out)
        } else {
            return None;
        };
        let number = mask.trailing_zeros() as u8;
        *mask &= *mask - 1;
        Some(GroupTerminal { number, direction })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = (self.in_mask.count_ones() + self.out_mask.count_ones()) as usize;
        (n, Some(n))
    }
}

impl ExactSizeIterator for GroupTerminals {}

/// The Group Terminals of one endpoint direction as a mask: bit n is Group Terminal Number n
/// (the Group field value n) (REQ-USB-UMP-013). The UMP reader marks a UMP whose Group is not
/// in the mask as "no Group Terminal" (choice U-05).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct GroupMask(u16);

impl GroupMask {
    /// No Group Terminal.
    pub const EMPTY: GroupMask = GroupMask(0);
    /// All 16 Group Terminals.
    pub const ALL: GroupMask = GroupMask(0xFFFF);

    /// The mask with the raw bits `bits` (bit n = Group Terminal Number n).
    pub const fn from_bits(bits: u16) -> Self {
        GroupMask(bits)
    }

    /// The raw bits.
    pub const fn bits(self) -> u16 {
        self.0
    }

    /// The Group Terminals that `endpoint` carries: for each block ID of the endpoint, the
    /// first block with that ID gives its IN Group Terminals (MIDI OUT endpoint) or its OUT
    /// Group Terminals (MIDI IN endpoint). A block ID with no block, a block of the wrong
    /// direction, an other type or a range that is not valid gives nothing (REQ-USB-GTB-014).
    pub fn from_blocks<E: EndpointBlocks + ?Sized>(
        blocks: &[GroupTerminalBlock],
        endpoint: &E,
    ) -> Self {
        let direction = TerminalDirection::of_endpoint(endpoint.direction());
        let mut bits = 0;
        for id in endpoint.block_ids() {
            if let Some(b) = blocks.iter().find(|b| b.id == *id) {
                bits |= b.mask(direction);
            }
        }
        GroupMask(bits)
    }

    /// True when Group Terminal Number `number` (0x0 to 0xF) is in the mask.
    pub const fn contains(self, number: u8) -> bool {
        number < GROUP_TERMINALS && self.0 & (1 << number) != 0
    }

    /// True when the Group Terminal of `group` is in the mask (REQ-USB-UMP-013).
    pub const fn contains_group(self, group: Group) -> bool {
        self.contains(group.index())
    }

    /// The number of different Group Terminal Numbers in the mask (REQ-USB-GTB-014).
    pub const fn count(self) -> u32 {
        self.0.count_ones()
    }

    /// True when no Group Terminal is in the mask.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

/// The 5 bytes of a Group Terminal Block Header descriptor with wTotalLength `total`
/// (Table 5-5, REQ-USB-GTB-002).
pub const fn header_bytes(total: u16) -> [u8; HEADER_LEN] {
    let t = total.to_le_bytes();
    [
        HEADER_LEN as u8,
        CS_GR_TRM_BLOCK,
        GR_TRM_BLOCK_HEADER,
        t[0],
        t[1],
    ]
}

/// The blocks of one USB MIDI 2.0 Alternate Setting in a fixed capacity `B`, in the order of
/// the caller (REQ-USB-GTB-001, REQ-USB-GTB-002). It dereferences to a slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GtbList<const B: usize> {
    blocks: FixedList<GroupTerminalBlock, B>,
}

impl<const B: usize> Default for GtbList<B> {
    /// [`GtbList::default_list`] (choice U-09).
    fn default() -> Self {
        Self::default_list()
    }
}

impl<const B: usize> Deref for GtbList<B> {
    type Target = [GroupTerminalBlock];
    fn deref(&self) -> &[GroupTerminalBlock] {
        self.blocks.as_slice()
    }
}

impl<const B: usize> GtbList<B> {
    /// An empty list (the writer refuses it, REQ-USB-GTB-001).
    pub fn new() -> Self {
        GtbList {
            blocks: FixedList::new(),
        }
    }

    /// The default list of choice U-09: the one block [`GroupTerminalBlock::DEFAULT`]. With
    /// `B = 0` the list is empty (REQ-USB-GTB-001).
    pub fn default_list() -> Self {
        let mut l = Self::new();
        let _ = l.blocks.push(GroupTerminalBlock::DEFAULT);
        l
    }

    /// A list with the blocks of `blocks`, or `None` if there are more than `B`.
    pub fn from_slice(blocks: &[GroupTerminalBlock]) -> Option<Self> {
        Some(GtbList {
            blocks: FixedList::from_slice(blocks)?,
        })
    }

    /// Adds a block at the end; [`GtbError::TooManyBlocks`] when the list is full.
    pub fn push(&mut self, block: GroupTerminalBlock) -> Result<(), GtbError> {
        self.blocks.push(block).map_err(|_| GtbError::TooManyBlocks)
    }

    /// Removes all blocks.
    pub fn clear(&mut self) {
        self.blocks.clear();
    }

    /// The blocks.
    pub fn as_slice(&self) -> &[GroupTerminalBlock] {
        self.blocks.as_slice()
    }

    /// The blocks, mutable.
    pub fn as_mut_slice(&mut self) -> &mut [GroupTerminalBlock] {
        self.blocks.as_mut_slice()
    }

    /// The first block with bGrpTrmBlkID `id` (REQ-USB-GTB-013).
    pub fn get(&self, id: u8) -> Option<&GroupTerminalBlock> {
        self.iter().find(|b| b.id == id)
    }

    /// The Group Terminals that `endpoint` carries ([`GroupMask::from_blocks`]).
    pub fn group_mask<E: EndpointBlocks + ?Sized>(&self, endpoint: &E) -> GroupMask {
        GroupMask::from_blocks(self, endpoint)
    }

    /// The length of the set: 5 + 13 x number of blocks, the wTotalLength of the header
    /// (REQ-USB-GTB-002).
    pub fn set_len(&self) -> usize {
        HEADER_LEN + BLOCK_LEN * self.len()
    }

    /// Checks the rules of the blocks alone: at least one block (REQ-USB-GTB-001), each block
    /// ([`GroupTerminalBlock::validate`]), no repeated ID (REQ-USB-GTB-004), no overlap in one
    /// direction (REQ-USB-GEN-011) and the 128-bit protocol of the 16-Group strategy
    /// (REQ-USB-001, REQ-USB-GTB-010). Gives the first error.
    pub fn validate(&self) -> Result<(), GtbError> {
        if self.is_empty() {
            return Err(GtbError::NoBlock);
        }
        let mut first = None;
        check_blocks(self, true, &mut |e| {
            first.get_or_insert(e);
        });
        first.map_or(Ok(()), Err)
    }

    /// Checks [`Self::validate`] and the rules between the blocks and the endpoints of the
    /// Alternate Setting: each named ID has a block (REQ-USB-GTB-013), each endpoint has 1 or
    /// more Group Terminals (REQ-USB-GTB-014), and the direction rules of REQ-USB-GEN-010
    /// (choice U-07). Gives the first error.
    pub fn validate_endpoints<E: EndpointBlocks>(&self, endpoints: &[E]) -> Result<(), GtbError> {
        self.validate()?;
        let mut first = None;
        check_endpoints(self, endpoints, &mut |e| {
            first.get_or_insert(e);
        });
        first.map_or(Ok(()), Err)
    }

    /// Writes the Group Terminal Block descriptor set of one Alternate Setting into `buf`, a
    /// buffer separate from the Configuration descriptor set, and gives its length
    /// (REQ-USB-GTB-002, REQ-USB-GTB-003, REQ-USB-GTB-015, choice U-12).
    ///
    /// `endpoints` are the endpoints of the same Alternate Setting. The writer first checks
    /// [`Self::validate_endpoints`] and gives its error. When `buf` is shorter than
    /// [`Self::set_len`], it gives [`GtbError::Overflow`]. In both cases it writes nothing.
    pub fn write<E: EndpointBlocks>(
        &self,
        endpoints: &[E],
        buf: &mut [u8],
    ) -> Result<usize, GtbError> {
        self.validate_endpoints(endpoints)?;
        let total = u16::try_from(self.set_len()).map_err(|_| GtbError::TooManyBlocks)?;
        if buf.len() < usize::from(total) {
            return Err(GtbError::Overflow);
        }
        let mut w = DescWriter::new(buf);
        w.bytes(&header_bytes(total))?;
        for b in self.iter() {
            w.bytes(&b.to_bytes())?;
        }
        Ok(w.len())
    }
}

/// The rules of the blocks alone (REQ-USB-GTB-004 to REQ-USB-GTB-006, REQ-USB-GTB-009,
/// REQ-USB-GTB-010, REQ-USB-GEN-011, REQ-USB-001). `strict` (the writer) also reports other
/// type and protocol values (choice U-15); the reader keeps them raw.
fn check_blocks(blocks: &[GroupTerminalBlock], strict: bool, report: &mut dyn FnMut(GtbError)) {
    let mut seen_in = 0u16;
    let mut seen_out = 0u16;
    for (i, b) in blocks.iter().enumerate() {
        if strict {
            if let GtbType::Unknown(raw) = b.kind {
                report(GtbError::UnknownType { id: b.id, raw });
            }
            if let MidiProtocol::Unknown(raw) = b.protocol {
                report(GtbError::UnknownProtocol { id: b.id, raw });
            }
        }
        if b.id == 0 {
            report(GtbError::BlockIdZero);
        }
        if !b.range_is_valid() {
            report(GtbError::GroupRange(b.id));
        }
        if blocks
            .get(..i)
            .is_some_and(|before| before.iter().any(|p| p.id == b.id))
        {
            report(GtbError::DuplicateBlockId(b.id));
        }
        for (mask, seen, direction) in [
            (b.in_mask(), &mut seen_in, TerminalDirection::In),
            (b.out_mask(), &mut seen_out, TerminalDirection::Out),
        ] {
            let both = mask & *seen;
            if both != 0 {
                report(GtbError::Overlap {
                    id: b.id,
                    group: both.trailing_zeros() as u8,
                    direction,
                });
            }
            *seen |= mask;
        }
        if b.spans_all_groups()
            && matches!(
                b.effective_protocol(),
                MidiProtocol::UseMidiCi | MidiProtocol::Midi1UpTo64 | MidiProtocol::Midi1UpTo64Jr
            )
        {
            report(GtbError::Needs128BitProtocol(b.id));
        }
    }
}

/// The rules between the blocks and the endpoints of one Alternate Setting (REQ-USB-GTB-013,
/// REQ-USB-GTB-014, REQ-USB-GEN-010, choice U-07). A repeated block ID is checked once, with
/// its first block.
fn check_endpoints<E: EndpointBlocks>(
    blocks: &[GroupTerminalBlock],
    endpoints: &[E],
    report: &mut dyn FnMut(GtbError),
) {
    for (index, ep) in endpoints.iter().enumerate() {
        let dir = ep.direction();
        for id in ep.block_ids() {
            match blocks.iter().find(|b| b.id == *id) {
                None => report(GtbError::UnknownBlockId(*id)),
                Some(b) => {
                    let wrong = match dir {
                        EndpointDirection::MidiIn => b.kind == GtbType::InputOnly,
                        EndpointDirection::MidiOut => b.kind == GtbType::OutputOnly,
                    };
                    if wrong {
                        report(GtbError::WrongDirection {
                            id: *id,
                            endpoint: dir,
                        });
                    }
                }
            }
        }
        if GroupMask::from_blocks(blocks, ep).is_empty() {
            report(GtbError::EndpointWithoutGroupTerminals { endpoint: index });
        }
    }
    for (i, b) in blocks.iter().enumerate() {
        if blocks
            .get(..i)
            .is_some_and(|before| before.iter().any(|p| p.id == b.id))
        {
            continue;
        }
        for (needed, dir) in [
            (b.kind.has_in(), EndpointDirection::MidiOut),
            (b.kind.has_out(), EndpointDirection::MidiIn),
        ] {
            if !needed {
                continue;
            }
            let named = endpoints
                .iter()
                .filter(|e| e.direction() == dir && e.block_ids().contains(&b.id))
                .count();
            match named {
                0 => report(GtbError::NotNamed {
                    id: b.id,
                    endpoint: dir,
                }),
                1 => {}
                _ => report(GtbError::NamedTwice {
                    id: b.id,
                    endpoint: dir,
                }),
            }
        }
    }
}

/// The lenient iterator over the blocks of a Group Terminal Block descriptor set
/// (REQ-USB-GTB-016, choices U-16, U-24).
///
/// It walks the slice by bLength, needs the header (type 0x26, subtype 0x01) first, and reads
/// at most wTotalLength bytes. It gives each block descriptor with bLength 13 or more (the
/// bytes after 13 are skipped), skips other descriptors, and gives
/// [`GtbError::BlockTooShort`] for a shorter block and continues. A missing header, a cut
/// descriptor or a bLength below 2 gives one error item and ends the walk; the blocks before
/// it are already given. It does not check the values of the blocks: see [`read_set`].
#[derive(Debug, Clone)]
pub struct GtbReader<'a> {
    bytes: &'a [u8],
    walk: Option<Descriptors<'a>>,
    total: Option<u16>,
    done: bool,
}

impl<'a> GtbReader<'a> {
    /// A reader over `bytes`: the answer of the request of section 6.
    pub fn new(bytes: &'a [u8]) -> Self {
        GtbReader {
            bytes,
            walk: None,
            total: None,
            done: false,
        }
    }

    /// wTotalLength of the header, once the header is read; `None` for a wTotalLength below 5.
    pub fn total_length(&self) -> Option<u16> {
        self.total
    }

    /// True when the slice is shorter than wTotalLength (a short answer: wLength below
    /// wTotalLength). The reader gives the complete blocks of the slice.
    pub fn is_short(&self) -> bool {
        self.total
            .is_some_and(|t| usize::from(t) > self.bytes.len())
    }

    fn start(&mut self) -> Result<(), GtbError> {
        let head = match Descriptors::new(self.bytes).next() {
            None => return Err(GtbError::NoHeader),
            Some(Err(e)) => return Err(e.into()),
            Some(Ok(d)) => d,
        };
        if head.dtype() != CS_GR_TRM_BLOCK || head.subtype() != Some(GR_TRM_BLOCK_HEADER) {
            return Err(GtbError::NoHeader);
        }
        let Some(total) = head.u16_le(3).filter(|_| head.len() >= HEADER_LEN) else {
            return Err(GtbError::HeaderTooShort);
        };
        // A wTotalLength below the 5 bytes of the header is no length (review L2): the reader
        // gives no total length for it.
        if usize::from(total) < HEADER_LEN {
            return Err(GtbError::HeaderTooShort);
        }
        self.total = Some(total);
        let total = usize::from(total);
        if total < head.len() {
            return Err(GtbError::Walk(WalkError::Truncated));
        }
        let end = total.min(self.bytes.len());
        let body = self.bytes.get(head.len()..end).unwrap_or(&[]);
        self.walk = Some(Descriptors::new(body));
        Ok(())
    }
}

impl<'a> Iterator for GtbReader<'a> {
    type Item = Result<GroupTerminalBlock, GtbError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        if self.walk.is_none() {
            if let Err(e) = self.start() {
                self.done = true;
                return Some(Err(e));
            }
        }
        let walk = self.walk.as_mut()?;
        loop {
            match walk.next() {
                None => {
                    self.done = true;
                    return None;
                }
                Some(Err(e)) => {
                    self.done = true;
                    return Some(Err(e.into()));
                }
                Some(Ok(d)) => {
                    if let Some(r) = GroupTerminalBlock::read(&d) {
                        return Some(r);
                    }
                }
            }
        }
    }
}

/// What the reader found in the Group Terminal Block descriptor set of one Alternate Setting
/// (REQ-USB-GTB-016, choice U-24). `B` is the capacity of the block list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GtbSetInfo<const B: usize> {
    /// wTotalLength of the header, if the set has a header.
    pub total_length: Option<u16>,
    /// The blocks in descriptor order with their raw values. A second block with an ID that
    /// is already in the list is not kept (REQ-USB-GTB-004).
    pub blocks: GtbList<B>,
    problems: [Option<GtbError>; MAX_PROBLEMS],
    problem_count: usize,
    short: bool,
}

impl<const B: usize> GtbSetInfo<B> {
    fn report(&mut self, e: GtbError) {
        if let Some(slot) = self.problems.get_mut(self.problem_count) {
            *slot = Some(e);
        }
        self.problem_count += 1;
    }

    /// The problems found, at most [`MAX_PROBLEMS`] (U-24).
    pub fn problems(&self) -> impl Iterator<Item = GtbError> + '_ {
        self.problems.iter().flatten().copied()
    }

    /// The number of problems found, also those above [`MAX_PROBLEMS`].
    pub fn problem_count(&self) -> usize {
        self.problem_count
    }

    /// True when the set has no problem.
    pub fn is_clean(&self) -> bool {
        self.problem_count == 0
    }

    /// True when the bytes were shorter than wTotalLength (a short answer, for example the
    /// header-only answer to the first request of section 6). The set is then not complete:
    /// the reader did not check it against the endpoints and did not report
    /// [`GtbError::NoBlock`].
    pub fn is_short(&self) -> bool {
        self.short
    }

    /// The Group Terminals that `endpoint` carries ([`GroupMask::from_blocks`]).
    pub fn group_mask<E: EndpointBlocks + ?Sized>(&self, endpoint: &E) -> GroupMask {
        self.blocks.group_mask(endpoint)
    }
}

/// Reads the Group Terminal Block descriptor set of one Alternate Setting and checks it
/// against the endpoints of that setting (REQ-USB-GTB-004 to REQ-USB-GTB-006,
/// REQ-USB-GTB-013, REQ-USB-GTB-014, REQ-USB-GTB-016, REQ-USB-GEN-010, REQ-USB-GEN-011,
/// REQ-USB-001).
///
/// The reader is lenient (choice U-24): it reports each break of a rule as a problem and
/// keeps the descriptors. Other type and protocol values are kept raw and not reported
/// (REQ-USB-GTB-005, REQ-USB-GTB-009). A block that does not fit in `B` is reported as
/// [`GtbError::TooManyBlocks`].
///
/// Bytes shorter than wTotalLength (a short answer, REQ-USB-GTB-020) give the complete blocks
/// in them and the problems of those blocks, but no [`GtbError::NoBlock`] and no endpoint
/// problems: the missing blocks can be the ones that the endpoints name
/// ([`GtbSetInfo::is_short`]).
pub fn read_set<const B: usize, E: EndpointBlocks>(bytes: &[u8], endpoints: &[E]) -> GtbSetInfo<B> {
    let mut info = GtbSetInfo {
        total_length: None,
        blocks: GtbList::new(),
        problems: [None; MAX_PROBLEMS],
        problem_count: 0,
        short: false,
    };
    let mut reader = GtbReader::new(bytes);
    let mut header_error = false;
    for item in reader.by_ref() {
        match item {
            Ok(b) => {
                if info.blocks.get(b.id).is_some() {
                    info.report(GtbError::DuplicateBlockId(b.id));
                } else if let Err(e) = info.blocks.push(b) {
                    info.report(e);
                }
            }
            Err(e) => {
                header_error = matches!(e, GtbError::NoHeader | GtbError::HeaderTooShort);
                info.report(e);
            }
        }
    }
    info.total_length = reader.total_length();
    info.short = reader.is_short();
    if info.blocks.is_empty() && !header_error && !info.short {
        info.report(GtbError::NoBlock);
    }
    let blocks = info.blocks;
    check_blocks(&blocks, false, &mut |e| info.report(e));
    if !info.short {
        check_endpoints(&blocks, endpoints, &mut |e| info.report(e));
    }
    info
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use crate::ms2::BusSpeed;
    use proptest::prelude::*;
    use std::vec::Vec;

    const OUT: EndpointDirection = EndpointDirection::MidiOut;
    const IN: EndpointDirection = EndpointDirection::MidiIn;

    fn ep(direction: EndpointDirection, ids: &[u8]) -> EndpointConfig {
        EndpointConfig::bulk(direction, 1, BusSpeed::Full)
            .with_blocks(ids)
            .unwrap()
    }

    fn pair(ids: &[u8]) -> [EndpointConfig; 2] {
        [ep(OUT, ids), ep(IN, ids)]
    }

    fn list(blocks: &[GroupTerminalBlock]) -> GtbList<8> {
        GtbList::from_slice(blocks).unwrap()
    }

    fn write_vec<E: EndpointBlocks>(l: &GtbList<8>, eps: &[E]) -> Result<Vec<u8>, GtbError> {
        let mut buf = [0u8; 200];
        let n = l.write(eps, &mut buf)?;
        Ok(buf[..n].to_vec())
    }

    fn problems<const B: usize>(i: &GtbSetInfo<B>) -> Vec<GtbError> {
        i.problems().collect()
    }

    /// Table B-21 and Table B-22 (REQ-USB-VEC2-011).
    const B21_B22: [u8; 18] = [
        0x05, 0x26, 0x01, 0x12, 0x00, // header
        0x0D, 0x26, 0x02, 0x01, 0x00, 0x00, 0x01, 0x04, 0x00, 0x01, 0x00, 0x00, 0x00,
    ];

    fn b22() -> GroupTerminalBlock {
        GroupTerminalBlock {
            string_index: 4,
            protocol: MidiProtocol::UseMidiCi,
            max_input_bandwidth: 0x0001,
            max_output_bandwidth: 0x0000,
            ..GroupTerminalBlock::new(1, GtbType::Bidirectional, 0, 1)
        }
    }

    #[test]
    fn default_list_is_one_bidirectional_block() {
        // REQ-USB-GTB-001, choice U-09.
        let l = GtbList::<4>::default();
        assert_eq!(l.len(), 1);
        let b = l[0];
        assert_eq!(b, GroupTerminalBlock::DEFAULT);
        assert_eq!(b.id, 1);
        assert_eq!(b.kind, GtbType::Bidirectional);
        assert_eq!((b.first_group, b.num_groups, b.string_index), (0, 1, 0));
        assert_eq!(b.protocol.into_bits(), 0x11);
        assert_eq!((b.max_input_bandwidth, b.max_output_bandwidth), (0, 0));
        assert_eq!(GtbList::<0>::default_list().len(), 0);
        // The default list with the default endpoints (block 1) is accepted.
        assert_eq!(l.validate_endpoints(&pair(&[1])), Ok(()));
    }

    #[test]
    fn writer_refuses_no_block_and_full_list() {
        // REQ-USB-GTB-001.
        let l = GtbList::<2>::new();
        let mut buf = [0xAAu8; 32];
        assert_eq!(l.write(&pair(&[1]), &mut buf), Err(GtbError::NoBlock));
        assert!(buf.iter().all(|b| *b == 0xAA));
        let mut l = GtbList::<1>::new();
        l.push(GroupTerminalBlock::DEFAULT).unwrap();
        assert_eq!(
            l.push(GroupTerminalBlock::DEFAULT),
            Err(GtbError::TooManyBlocks)
        );
        assert!(GtbList::<1>::from_slice(&[GroupTerminalBlock::DEFAULT; 2]).is_none());
    }

    #[test]
    fn appendix_b_set_written_and_read_back() {
        // REQ-USB-VEC2-011, REQ-USB-GTB-002, REQ-USB-GTB-003, REQ-USB-GEN-012.
        let l = list(&[b22()]);
        let eps = pair(&[1]);
        assert_eq!(write_vec(&l, &eps).unwrap(), B21_B22);
        let info = read_set::<4, _>(&B21_B22, &eps);
        assert!(info.is_clean(), "{:?}", problems(&info));
        assert_eq!(info.total_length, Some(0x0012));
        assert_eq!(info.blocks.as_slice(), &[b22()]);
    }

    #[test]
    fn header_total_length_and_block_order() {
        // REQ-USB-GTB-002: wTotalLength = 5 + 13 x n, blocks in the order of the caller.
        let blocks = [
            GroupTerminalBlock::new(7, GtbType::InputOnly, 4, 2),
            GroupTerminalBlock::new(3, GtbType::OutputOnly, 0, 1),
            GroupTerminalBlock::new(5, GtbType::Bidirectional, 8, 8),
        ];
        let l = list(&blocks);
        let eps = [ep(OUT, &[7, 5]), ep(IN, &[3, 5])];
        let v = write_vec(&l, &eps).unwrap();
        assert_eq!(v.len(), 5 + 13 * 3);
        assert_eq!(l.set_len(), v.len());
        assert_eq!(&v[..5], &[0x05, 0x26, 0x01, 44, 0x00]);
        assert_eq!([v[5 + 3], v[18 + 3], v[31 + 3]], [7, 3, 5]);
        let info = read_set::<4, _>(&v, &eps);
        assert!(info.is_clean(), "{:?}", problems(&info));
        assert_eq!(info.blocks.as_slice(), &blocks);
    }

    #[test]
    fn block_bytes_at_table_5_6_offsets() {
        // REQ-USB-GTB-003: offsets of Table 5-6, 2-byte fields least significant byte first.
        let b = GroupTerminalBlock {
            id: 0x21,
            kind: GtbType::OutputOnly,
            first_group: 0x03,
            num_groups: 0x04,
            string_index: 0x09,
            protocol: MidiProtocol::Midi2Jr,
            max_input_bandwidth: 0x1234,
            max_output_bandwidth: 0xABCD,
        };
        assert_eq!(
            b.to_bytes(),
            [0x0D, 0x26, 0x02, 0x21, 0x02, 0x03, 0x04, 0x09, 0x12, 0x34, 0x12, 0xCD, 0xAB]
        );
        assert_eq!(GroupTerminalBlock::from_bytes(&b.to_bytes()), b);
        assert_eq!(header_bytes(0x0102), [0x05, 0x26, 0x01, 0x02, 0x01]);
    }

    #[test]
    fn block_id_zero_and_repeated_id() {
        // REQ-USB-GTB-004.
        let zero = GroupTerminalBlock::new(0, GtbType::Bidirectional, 0, 1);
        assert_eq!(list(&[zero]).validate(), Err(GtbError::BlockIdZero));
        let a = GroupTerminalBlock::new(2, GtbType::InputOnly, 0, 1);
        let b = GroupTerminalBlock::new(2, GtbType::OutputOnly, 0, 1);
        assert_eq!(list(&[a, b]).validate(), Err(GtbError::DuplicateBlockId(2)));

        // The reader reports both and keeps the first block with the ID.
        let mut set = std::vec![0x05, 0x26, 0x01, 44, 0x00];
        set.extend_from_slice(&a.to_bytes());
        set.extend_from_slice(&b.to_bytes());
        set.extend_from_slice(&zero.to_bytes());
        let eps = [ep(OUT, &[2]), ep(IN, &[2, 0])];
        let info = read_set::<4, _>(&set, &eps);
        assert_eq!(info.blocks.as_slice(), &[a, zero]);
        let p = problems(&info);
        assert!(p.contains(&GtbError::DuplicateBlockId(2)));
        assert!(p.contains(&GtbError::BlockIdZero));
    }

    #[test]
    fn block_types_and_directions() {
        // REQ-USB-GTB-005: 0x00 both, 0x01 IN only (host to device), 0x02 OUT only.
        let bi = GroupTerminalBlock::new(1, GtbType::Bidirectional, 2, 2);
        let i = GroupTerminalBlock::new(1, GtbType::InputOnly, 2, 2);
        let o = GroupTerminalBlock::new(1, GtbType::OutputOnly, 2, 2);
        assert_eq!((bi.in_mask(), bi.out_mask()), (0b1100, 0b1100));
        assert_eq!((i.in_mask(), i.out_mask()), (0b1100, 0));
        assert_eq!((o.in_mask(), o.out_mask()), (0, 0b1100));
        assert_eq!(TerminalDirection::In.endpoint(), OUT);
        assert_eq!(TerminalDirection::Out.endpoint(), IN);
        assert_eq!(TerminalDirection::of_endpoint(OUT), TerminalDirection::In);
        assert_eq!(TerminalDirection::of_endpoint(IN), TerminalDirection::Out);

        // The writer refuses an other type; the reader keeps it raw, with no Group Terminal.
        let u = GroupTerminalBlock::new(1, GtbType::Unknown(3), 0, 1);
        assert_eq!(
            list(&[u]).validate(),
            Err(GtbError::UnknownType { id: 1, raw: 3 })
        );
        let mut set = header_bytes(18).to_vec();
        set.extend_from_slice(&u.to_bytes());
        let info = read_set::<2, _>(&set, &pair(&[1]));
        assert_eq!(info.blocks[0].kind, GtbType::Unknown(3));
        assert_eq!(info.blocks[0].group_terminal_count(), 0);
        assert!(!problems(&info)
            .iter()
            .any(|e| matches!(e, GtbError::UnknownType { .. })));
        assert!(info.group_mask(&ep(OUT, &[1])).is_empty());
    }

    #[test]
    fn group_terminal_range_rules() {
        // REQ-USB-GTB-006.
        for (first, num, ok) in [
            (0x0F, 1, true),
            (0x00, 16, true),
            (0x08, 8, true),
            (0x10, 1, false),
            (0x00, 0, false),
            (0x08, 9, false),
            (0xFF, 0xFF, false),
        ] {
            let b = GroupTerminalBlock::new(1, GtbType::Bidirectional, first, num);
            assert_eq!(b.range_is_valid(), ok, "{first} {num}");
            let r = list(&[b]).validate();
            if ok {
                assert_eq!(r, Ok(()));
            } else {
                assert_eq!(r, Err(GtbError::GroupRange(1)));
                assert_eq!(b.member_mask(), 0);
            }
        }
        // Members nGroupTrm to nGroupTrm + nNumGroupTrm - 1.
        let b = GroupTerminalBlock::new(1, GtbType::InputOnly, 5, 3);
        assert_eq!(b.member_mask(), 0b1110_0000);
        // The reader reports the error and keeps the raw fields.
        let bad = GroupTerminalBlock::new(1, GtbType::Bidirectional, 0x0C, 5);
        let mut set = header_bytes(18).to_vec();
        set.extend_from_slice(&bad.to_bytes());
        let info = read_set::<2, _>(&set, &pair(&[1]));
        assert_eq!(info.blocks[0], bad);
        assert!(problems(&info).contains(&GtbError::GroupRange(1)));
    }

    #[test]
    fn group_terminal_count_by_type() {
        // REQ-USB-GTB-007.
        assert_eq!(
            GroupTerminalBlock::new(1, GtbType::InputOnly, 0, 5).group_terminal_count(),
            5
        );
        assert_eq!(
            GroupTerminalBlock::new(1, GtbType::OutputOnly, 0, 5).group_terminal_count(),
            5
        );
        assert_eq!(
            GroupTerminalBlock::new(1, GtbType::Bidirectional, 0, 5).group_terminal_count(),
            10
        );
        assert_eq!(
            GroupTerminalBlock::new(1, GtbType::Bidirectional, 0, 16).group_terminal_count(),
            32
        );
    }

    #[test]
    fn examples_of_section_5_4_2() {
        // REQ-USB-GTB-008, AMB-USB-MIDI-2-06.
        let t = |b: GroupTerminalBlock| -> Vec<(TerminalDirection, u8, u8)> {
            b.group_terminals()
                .map(|g| (g.direction, g.user_number(), g.group().index()))
                .collect()
        };
        use TerminalDirection::{In as I, Out as O};
        assert_eq!(
            t(GroupTerminalBlock::new(1, GtbType::InputOnly, 0x00, 3)),
            [(I, 1, 0x0), (I, 2, 0x1), (I, 3, 0x2)]
        );
        assert_eq!(
            t(GroupTerminalBlock::new(1, GtbType::OutputOnly, 0x06, 5)),
            [
                (O, 7, 0x6),
                (O, 8, 0x7),
                (O, 9, 0x8),
                (O, 10, 0x9),
                (O, 11, 0xA)
            ]
        );
        let bi = GroupTerminalBlock::new(1, GtbType::Bidirectional, 0x00, 3);
        assert_eq!(bi.group_terminals().len(), 6);
        assert_eq!(
            t(bi),
            [
                (I, 1, 0x0),
                (I, 2, 0x1),
                (I, 3, 0x2),
                (O, 1, 0x0),
                (O, 2, 0x1),
                (O, 3, 0x2)
            ]
        );
    }

    #[test]
    fn protocol_values_writer_refuses_other_reader_keeps_raw() {
        // REQ-USB-GTB-009.
        for raw in [0x00, 0x01, 0x02, 0x03, 0x04, 0x11, 0x12] {
            let b = GroupTerminalBlock {
                protocol: MidiProtocol::from_bits(raw),
                ..GroupTerminalBlock::DEFAULT
            };
            assert_eq!(list(&[b]).validate(), Ok(()), "{raw}");
            assert_eq!(b.to_bytes()[8], raw);
        }
        let b = GroupTerminalBlock {
            protocol: MidiProtocol::Unknown(0x13),
            ..GroupTerminalBlock::DEFAULT
        };
        assert_eq!(
            list(&[b]).validate(),
            Err(GtbError::UnknownProtocol { id: 1, raw: 0x13 })
        );
        let mut set = header_bytes(18).to_vec();
        set.extend_from_slice(&b.to_bytes());
        let info = read_set::<2, _>(&set, &pair(&[1]));
        assert!(info.is_clean(), "{:?}", problems(&info));
        assert_eq!(info.blocks[0].protocol, MidiProtocol::Unknown(0x13));
        assert_eq!(info.blocks[0].effective_protocol(), MidiProtocol::UseMidiCi);
        assert_eq!(info.blocks[0].protocol_hint(), None);
    }

    #[test]
    fn protocol_default_and_all_16_groups_need_128_bits() {
        // REQ-USB-GTB-010, REQ-USB-001, choice U-11.
        assert_eq!(
            GroupTerminalBlock::new(9, GtbType::InputOnly, 0, 1).protocol,
            MidiProtocol::Midi2
        );
        let all = GroupTerminalBlock::new(1, GtbType::Bidirectional, 0, 16);
        assert!(all.spans_all_groups());
        for raw in [0x00u8, 0x01, 0x02] {
            let b = GroupTerminalBlock {
                protocol: MidiProtocol::from_bits(raw),
                ..all
            };
            assert_eq!(
                list(&[b]).validate(),
                Err(GtbError::Needs128BitProtocol(1)),
                "{raw}"
            );
            // The reader reports it and keeps the block.
            let mut set = header_bytes(18).to_vec();
            set.extend_from_slice(&b.to_bytes());
            let info = read_set::<2, _>(&set, &pair(&[1]));
            assert_eq!(problems(&info), [GtbError::Needs128BitProtocol(1)]);
            assert_eq!(info.blocks.len(), 1);
        }
        for raw in [0x03u8, 0x04, 0x11, 0x12] {
            let b = GroupTerminalBlock {
                protocol: MidiProtocol::from_bits(raw),
                ..all
            };
            assert_eq!(list(&[b]).validate(), Ok(()), "{raw}");
        }
        // 0x01 is accepted when the block does not span 16 Groups (Table B-22 uses 0x00).
        let b = GroupTerminalBlock {
            protocol: MidiProtocol::Midi1UpTo64,
            ..GroupTerminalBlock::new(1, GtbType::Bidirectional, 0, 15)
        };
        assert_eq!(list(&[b]).validate(), Ok(()));
    }

    #[test]
    fn protocol_to_stream_configuration() {
        // REQ-USB-GTB-011.
        for raw in 0..=255u8 {
            let h = ProtocolHint::from_protocol(MidiProtocol::from_bits(raw));
            let want = match raw {
                0x01 | 0x03 => Some((StreamProtocol::Midi1, false)),
                0x02 | 0x04 => Some((StreamProtocol::Midi1, true)),
                0x11 => Some((StreamProtocol::Midi2, false)),
                0x12 => Some((StreamProtocol::Midi2, true)),
                _ => None,
            };
            assert_eq!(h.map(|h| (h.protocol, h.jr_timestamps)), want, "{raw}");
        }
        assert_eq!(StreamProtocol::Midi1.into_bits(), 0x01);
        assert_eq!(StreamProtocol::Midi2.into_bits(), 0x02);
    }

    #[test]
    fn bandwidth_units_of_4000_bytes() {
        // REQ-USB-GTB-012, AMB-USB-MIDI-2-21.
        assert_eq!(bandwidth_from_bytes_per_second(0), 0x0000);
        assert_eq!(bandwidth_from_bytes_per_second(31_250 / 8), 0x0001);
        assert_eq!(bandwidth_from_bytes_per_second(1), 0x0001);
        assert_eq!(bandwidth_from_bytes_per_second(4000), 0x0001);
        assert_eq!(bandwidth_from_bytes_per_second(4001), 0x0002);
        assert_eq!(bandwidth_from_bytes_per_second(4000 * 0xFFFF), 0xFFFF);
        assert_eq!(bandwidth_from_bytes_per_second(u32::MAX), 0xFFFF);
        // The writer writes the caller values; the reader gives them raw.
        let b = GroupTerminalBlock {
            max_input_bandwidth: 0x0001,
            max_output_bandwidth: 0xFFFF,
            ..GroupTerminalBlock::DEFAULT
        };
        let v = write_vec(&list(&[b]), &pair(&[1])).unwrap();
        assert_eq!(&v[14..18], &[0x01, 0x00, 0xFF, 0xFF]);
        let info = read_set::<2, _>(&v, &pair(&[1]));
        assert_eq!(
            (
                info.blocks[0].max_input_bandwidth,
                info.blocks[0].max_output_bandwidth
            ),
            (0x0001, 0xFFFF)
        );
    }

    #[test]
    fn endpoint_names_block_with_no_descriptor() {
        // REQ-USB-GTB-013.
        let l = GtbList::<8>::default_list();
        let eps = [ep(OUT, &[1, 4]), ep(IN, &[1])];
        assert_eq!(l.validate_endpoints(&eps), Err(GtbError::UnknownBlockId(4)));
        let v = write_vec(&l, &pair(&[1])).unwrap();
        let info = read_set::<2, _>(&v, &eps);
        assert_eq!(problems(&info), [GtbError::UnknownBlockId(4)]);
        assert_eq!(info.blocks.len(), 1);
        assert_eq!(info.group_mask(&eps[0]).bits(), 0x0001);
    }

    #[test]
    fn endpoint_with_no_group_terminal_in_its_direction() {
        // REQ-USB-GTB-014: count the different Group Terminal Numbers in the endpoint
        // direction; a block of the other direction gives none.
        let o = GroupTerminalBlock::new(1, GtbType::OutputOnly, 0, 2);
        let l = list(&[o]);
        let eps = [ep(OUT, &[1]), ep(IN, &[1])];
        let mut first = None;
        check_endpoints(&l, &eps, &mut |e| {
            first.get_or_insert(e);
        });
        assert_eq!(
            first,
            Some(GtbError::WrongDirection {
                id: 1,
                endpoint: OUT
            })
        );
        let info = read_set::<2, _>(&write_vec(&l, &[ep(IN, &[1])]).unwrap(), &eps);
        assert!(problems(&info).contains(&GtbError::EndpointWithoutGroupTerminals { endpoint: 0 }));
        // A bidirectional block of 16 numbers gives 16 in each direction.
        let all = GroupTerminalBlock::new(1, GtbType::Bidirectional, 0, 16);
        assert_eq!(GroupMask::from_blocks(&[all], &ep(OUT, &[1])).count(), 16);
        assert_eq!(GroupMask::from_blocks(&[all], &ep(IN, &[1])).count(), 16);
    }

    #[test]
    fn writer_uses_a_separate_buffer_and_writes_nothing_on_overflow() {
        // REQ-USB-GTB-015, choice U-12: the set is the whole content of the buffer (header
        // first), and a too small buffer is left unchanged.
        let l = list(&[b22()]);
        let mut buf = [0xEEu8; 17];
        assert_eq!(l.write(&pair(&[1]), &mut buf), Err(GtbError::Overflow));
        assert!(buf.iter().all(|b| *b == 0xEE));
        let mut buf = [0xEEu8; 18];
        assert_eq!(l.write(&pair(&[1]), &mut buf), Ok(18));
        assert_eq!(buf, B21_B22);
    }

    #[test]
    fn reader_walks_by_blength_and_is_lenient() {
        // REQ-USB-GTB-016, choice U-16.
        // A block with bLength 15 (2 more bytes) and an other descriptor are accepted/skipped.
        let mut set = std::vec![0x05, 0x26, 0x01, 0, 0];
        set.extend_from_slice(&[3, 0x24, 0x01]);
        let mut long = b22().to_bytes().to_vec();
        long[0] = 15;
        long.extend_from_slice(&[0xAA, 0xBB]);
        set.extend_from_slice(&long);
        set.extend_from_slice(&[4, 0x26, 0x02, 9]); // block too short: reported, skipped
        let total = set.len() as u16;
        set[3..5].copy_from_slice(&total.to_le_bytes());
        set.extend_from_slice(&[0xFF; 6]); // after wTotalLength: not read
        let r: Vec<_> = GtbReader::new(&set).collect();
        assert_eq!(r, [Ok(b22()), Err(GtbError::BlockTooShort)]);

        // A header first is needed.
        let r: Vec<_> = GtbReader::new(&B21_B22[5..]).collect();
        assert_eq!(r, [Err(GtbError::NoHeader)]);
        assert_eq!(
            GtbReader::new(&[]).collect::<Vec<_>>(),
            [Err(GtbError::NoHeader)]
        );
        assert_eq!(
            GtbReader::new(&[4, 0x26, 0x01, 18]).collect::<Vec<_>>(),
            [Err(GtbError::HeaderTooShort)]
        );
        assert_eq!(
            GtbReader::new(&[5, 0x26, 0x01, 4, 0]).collect::<Vec<_>>(),
            [Err(GtbError::HeaderTooShort)]
        );

        // Two blocks; the slice cuts the second: the first is given with an error.
        let a = GroupTerminalBlock::new(1, GtbType::InputOnly, 0, 1);
        let b = GroupTerminalBlock::new(2, GtbType::OutputOnly, 0, 1);
        let mut two = header_bytes(31).to_vec();
        two.extend_from_slice(&a.to_bytes());
        two.extend_from_slice(&b.to_bytes());
        let r: Vec<_> = GtbReader::new(&two[..25]).collect();
        assert_eq!(r, [Ok(a), Err(GtbError::Walk(WalkError::Truncated))]);
        let eps = [ep(OUT, &[1]), ep(IN, &[2])];
        let info = read_set::<4, _>(&two[..25], &eps);
        assert_eq!(info.blocks.as_slice(), &[a]);
        assert!(problems(&info).contains(&GtbError::Walk(WalkError::Truncated)));

        // wTotalLength cuts the second block.
        let mut cut = two.clone();
        cut[3] = 25;
        let r: Vec<_> = GtbReader::new(&cut).collect();
        assert_eq!(r, [Ok(a), Err(GtbError::Walk(WalkError::Truncated))]);

        // A short answer at a descriptor boundary gives the complete blocks, no error.
        let mut reader = GtbReader::new(&two[..18]);
        assert_eq!(reader.by_ref().collect::<Vec<_>>(), [Ok(a)]);
        assert_eq!(reader.total_length(), Some(31));
        assert!(reader.is_short());

        // A bLength below 2 ends the walk.
        let mut bad = header_bytes(8).to_vec();
        bad.extend_from_slice(&[1, 0, 0]);
        assert_eq!(
            GtbReader::new(&bad).collect::<Vec<_>>(),
            [Err(GtbError::Walk(WalkError::LengthTooSmall))]
        );
    }

    #[test]
    fn short_answer_gives_no_block_or_endpoint_problems() {
        // REQ-USB-GTB-020 (accept an answer shorter than wLength; review correctness L-2): the
        // header-only answer of Table B-23 gives wTotalLength and no problem; a short answer
        // with the first block gives that block and no endpoint problem for the missing one.
        let eps = [ep(OUT, &[1]), ep(IN, &[1])];
        let info = read_set::<4, _>(&B21_B22[..5], &eps);
        assert_eq!(info.total_length, Some(18));
        assert!(info.is_short());
        assert!(info.blocks.is_empty());
        assert!(info.is_clean(), "{:?}", problems(&info));
        let a = GroupTerminalBlock::new(1, GtbType::InputOnly, 0, 1);
        let b = GroupTerminalBlock::new(2, GtbType::OutputOnly, 0, 1);
        let mut two = header_bytes(31).to_vec();
        two.extend_from_slice(&a.to_bytes());
        two.extend_from_slice(&b.to_bytes());
        let eps = [ep(OUT, &[1]), ep(IN, &[2])];
        let info = read_set::<4, _>(&two[..18], &eps);
        assert!(info.is_short() && info.is_clean(), "{:?}", problems(&info));
        assert_eq!(info.blocks.as_slice(), &[a]);
        // The complete answer is checked against the endpoints.
        let info = read_set::<4, _>(&two, &eps);
        assert!(!info.is_short() && info.is_clean());
        let info = read_set::<4, _>(&two, &[ep(OUT, &[1]), ep(IN, &[3])]);
        assert!(problems(&info).contains(&GtbError::UnknownBlockId(3)));
    }

    #[test]
    fn total_length_below_header_is_no_total_length() {
        // REQ-USB-GTB-016 (review robustness L2): a wTotalLength below 5 is refused and gives
        // no total length, so the host does not ask for fewer bytes than the header.
        for total in 0u16..5 {
            let bytes = header_bytes(total);
            let mut reader = GtbReader::new(&bytes);
            assert_eq!(reader.next(), Some(Err(GtbError::HeaderTooShort)));
            assert_eq!(reader.total_length(), None);
            let info = read_set::<1, EndpointConfig>(&bytes, &[]);
            assert_eq!(info.total_length, None);
        }
    }

    #[test]
    fn endpoint_direction_rules() {
        // REQ-USB-GEN-010, choice U-07.
        let bi = GroupTerminalBlock::new(1, GtbType::Bidirectional, 0, 2);
        let i = GroupTerminalBlock::new(2, GtbType::InputOnly, 2, 1);
        let o = GroupTerminalBlock::new(3, GtbType::OutputOnly, 2, 1);
        let l = list(&[bi, i, o]);
        // One MIDI OUT and one MIDI IN endpoint share the bidirectional block: accepted.
        let good = [ep(OUT, &[1, 2]), ep(IN, &[1, 3])];
        assert_eq!(l.validate_endpoints(&good), Ok(()));

        // A MIDI IN endpoint names a type 0x01 block.
        let wrong = [ep(OUT, &[1, 2]), ep(IN, &[1, 3, 2])];
        assert_eq!(
            l.validate_endpoints(&wrong),
            Err(GtbError::WrongDirection {
                id: 2,
                endpoint: IN
            })
        );
        // A MIDI OUT endpoint names a type 0x02 block.
        let wrong = [ep(OUT, &[1, 2, 3]), ep(IN, &[1, 3])];
        assert_eq!(
            l.validate_endpoints(&wrong),
            Err(GtbError::WrongDirection {
                id: 3,
                endpoint: OUT
            })
        );
        // A bidirectional block named by one endpoint only.
        let half = [ep(OUT, &[1, 2]), ep(IN, &[3])];
        assert_eq!(
            l.validate_endpoints(&half),
            Err(GtbError::NotNamed {
                id: 1,
                endpoint: IN
            })
        );
        // A block named by no endpoint.
        let none = [ep(OUT, &[1]), ep(IN, &[1, 3])];
        assert_eq!(
            l.validate_endpoints(&none),
            Err(GtbError::NotNamed {
                id: 2,
                endpoint: OUT
            })
        );
        // Two MIDI OUT endpoints name the same block.
        let twice = [ep(OUT, &[1, 2]), ep(OUT, &[2]), ep(IN, &[1, 3])];
        assert_eq!(
            l.validate_endpoints(&twice),
            Err(GtbError::NamedTwice {
                id: 2,
                endpoint: OUT
            })
        );
        let mut buf = [0u8; 64];
        assert!(l.write(&twice, &mut buf).is_err());

        // The reader reports each break and keeps the descriptors.
        let v = write_vec(&l, &good).unwrap();
        let info = read_set::<4, _>(&v, &half);
        assert_eq!(info.blocks.len(), 3);
        assert_eq!(
            problems(&info),
            [GtbError::NotNamed {
                id: 1,
                endpoint: IN
            }]
        );
        let info = read_set::<4, _>(&v, &good);
        assert!(info.is_clean());
    }

    #[test]
    fn group_terminal_in_one_block_per_direction() {
        // REQ-USB-GEN-011, choice U-03: a bidirectional block counts in both directions.
        let bi = GroupTerminalBlock::new(1, GtbType::Bidirectional, 0, 4);
        let i = GroupTerminalBlock::new(2, GtbType::InputOnly, 3, 2);
        assert_eq!(
            list(&[bi, i]).validate(),
            Err(GtbError::Overlap {
                id: 2,
                group: 3,
                direction: TerminalDirection::In
            })
        );
        // IN only and OUT only blocks on the same numbers do not overlap.
        let i = GroupTerminalBlock::new(1, GtbType::InputOnly, 0, 4);
        let o = GroupTerminalBlock::new(2, GtbType::OutputOnly, 0, 4);
        assert_eq!(list(&[i, o]).validate(), Ok(()));
        // The overlap is checked in the whole Alternate Setting, not one endpoint.
        let o2 = GroupTerminalBlock::new(3, GtbType::OutputOnly, 3, 1);
        assert_eq!(
            list(&[i, o, o2]).validate(),
            Err(GtbError::Overlap {
                id: 3,
                group: 3,
                direction: TerminalDirection::Out
            })
        );
        // The reader reports it and keeps the descriptors.
        let mut set = header_bytes(5 + 13 * 3).to_vec();
        for b in [i, o, o2] {
            set.extend_from_slice(&b.to_bytes());
        }
        let eps = [ep(OUT, &[1]), ep(IN, &[2]), ep(IN, &[3])];
        let info = read_set::<4, _>(&set, &eps);
        assert_eq!(info.blocks.len(), 3);
        assert_eq!(
            problems(&info),
            [GtbError::Overlap {
                id: 3,
                group: 3,
                direction: TerminalDirection::Out
            }]
        );
    }

    #[test]
    fn string_index_zero_is_no_string() {
        // REQ-USB-GEN-012: default 0, accepted by the writer; a caller index is written.
        assert_eq!(GroupTerminalBlock::DEFAULT.string_index, 0);
        let v = write_vec(&GtbList::default_list(), &pair(&[1])).unwrap();
        assert_eq!(v[5 + 7], 0);
        let b = GroupTerminalBlock {
            string_index: 5,
            ..GroupTerminalBlock::DEFAULT
        };
        let v = write_vec(&list(&[b]), &pair(&[1])).unwrap();
        assert_eq!(v[5 + 7], 5);
        // The host-side reader gives 0 back: no string.
        let v = write_vec(&GtbList::default_list(), &pair(&[1])).unwrap();
        let info = read_set::<8, _>(&v, &pair(&[1]));
        assert!(info.is_clean());
        assert_eq!(info.blocks.as_slice()[0].string_index, 0);
    }

    #[test]
    fn ump_group_is_group_terminal_number() {
        // REQ-USB-UMP-013: Group field n = Group Terminal Number n, shown as n + 1.
        for n in 0..16u8 {
            let g = Group::new(n).unwrap();
            for d in [TerminalDirection::In, TerminalDirection::Out] {
                let t = GroupTerminal::from_group(g, d);
                assert_eq!(t.number, n);
                assert_eq!(t.group(), g);
                assert_eq!(t.user_number(), n + 1);
                assert_eq!(t.user_number(), g.number());
            }
        }
        let t = GroupTerminal::from_group(Group::new(2).unwrap(), TerminalDirection::In);
        assert_eq!(std::format!("{t}"), "IN Group Terminal 3");
        let t = GroupTerminal::from_group(Group::new(15).unwrap(), TerminalDirection::Out);
        assert_eq!(std::format!("{t}"), "OUT Group Terminal 16");
    }

    #[test]
    fn group_mask_of_an_endpoint() {
        // REQ-USB-UMP-013, REQ-USB-GTB-014, choice U-05.
        let blocks = [
            GroupTerminalBlock::new(1, GtbType::Bidirectional, 0, 2),
            GroupTerminalBlock::new(2, GtbType::InputOnly, 4, 2),
            GroupTerminalBlock::new(3, GtbType::OutputOnly, 8, 1),
        ];
        let out = GroupMask::from_blocks(&blocks, &ep(OUT, &[1, 2]));
        assert_eq!(out.bits(), 0b0011_0011);
        assert!(out.contains(4) && !out.contains(2) && !out.contains(16));
        assert!(out.contains_group(Group::new(5).unwrap()));
        let inn = GroupMask::from_blocks(&blocks, &ep(IN, &[1, 3]));
        assert_eq!(inn.bits(), 0b1_0000_0011);
        assert_eq!(inn.count(), 3);
        // The reader endpoint type gives the same mask.
        let mut r = Ms2Endpoint {
            direction: IN,
            ..Ms2Endpoint::default()
        };
        r.blocks = crate::ms2::GtbIds::from_slice(&[1, 3]).unwrap();
        assert_eq!(GroupMask::from_blocks(&blocks, &r), inn);
        assert_eq!(GroupMask::ALL.count(), 16);
        assert!(GroupMask::EMPTY.is_empty());
    }

    fn arb_block() -> impl Strategy<Value = GroupTerminalBlock> {
        (
            1..=255u8,
            0..3u8,
            0..16u8,
            any::<u8>(),
            any::<u8>(),
            prop::sample::select(std::vec![0x03u8, 0x04, 0x11, 0x12]),
            any::<u16>(),
            any::<u16>(),
        )
            .prop_map(|(id, t, first, n, s, p, bi, bo)| GroupTerminalBlock {
                id,
                kind: GtbType::from_bits(t),
                first_group: first,
                num_groups: 1 + n % (16 - first),
                string_index: s,
                protocol: MidiProtocol::from_bits(p),
                max_input_bandwidth: bi,
                max_output_bandwidth: bo,
            })
    }

    #[test]
    fn prop_valid_block_round_trips() {
        // REQ-USB-GTB-003, REQ-USB-GTB-016: write and read back one valid block.
        proptest!(|(b in arb_block())| {
            let l = list(&[b]);
            let mut eps = Vec::new();
            if b.kind.has_in() { eps.push(ep(OUT, &[b.id])); }
            if b.kind.has_out() { eps.push(ep(IN, &[b.id])); }
            let v = write_vec(&l, &eps).unwrap();
            prop_assert_eq!(v.len(), 18);
            let info = read_set::<1, _>(&v, &eps);
            prop_assert!(info.is_clean());
            prop_assert_eq!(info.blocks[0], b);
        });
    }

    #[test]
    fn prop_reader_never_panics() {
        // REQ-USB-GTB-016, choice U-24: any bytes give values, never a panic.
        proptest!(|(bytes in proptest::collection::vec(any::<u8>(), 0..120),
                    ids in proptest::collection::vec(any::<u8>(), 0..4))| {
            let eps = [ep(OUT, &ids), ep(IN, &ids)];
            let info = read_set::<4, _>(&bytes, &eps);
            for b in info.blocks.iter() {
                prop_assert!(b.group_terminal_count() <= 32);
            }
            let _ = info.group_mask(&eps[0]);
            let n = GtbReader::new(&bytes).count();
            prop_assert!(n <= bytes.len() / 2 + 1);
        });
    }

    #[test]
    fn prop_bandwidth_rounds_up() {
        // REQ-USB-GTB-012.
        proptest!(|(bps in 1..(4000u32 * 0xFFFF))| {
            let v = u32::from(bandwidth_from_bytes_per_second(bps));
            prop_assert!(v * 4000 >= bps && (v - 1) * 4000 < bps);
        });
    }
}
