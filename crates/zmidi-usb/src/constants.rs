//! Constants of USB-MIDI v2.0 Appendix A and USB-MIDI v1.0 Appendix A (REQ-USB-MS2-015,
//! REQ-USB-GTB-021).
//!
//! - Plain constants for the descriptor types, subtypes and class codes that the writers and
//!   readers of the crate use.
//! - Three value sets as enums with `from_bits` / `into_bits`: [`JackType`], [`GtbType`] and
//!   [`MidiProtocol`]. A value outside a set is kept as `Unknown(raw)`, so a lenient reader can
//!   give it to the caller (DEC-USB-001 choices U-15, U-24). The writers refuse `Unknown`.
//! - `MIDI_IN_JACK`, `MIDI_OUT_JACK`, `ELEMENT`, `MS_GENERAL` and the jack types apply to the
//!   USB-MIDI 1.0 Alternate Setting only.

/// Interface class AUDIO (USB Audio 1.0, used by USB-MIDI v1.0 and v2.0).
pub const AUDIO: u8 = 0x01;
/// Interface subclass AUDIOCONTROL.
pub const AUDIOCONTROL: u8 = 0x01;
/// Interface subclass MIDISTREAMING.
pub const MIDISTREAMING: u8 = 0x03;

/// Standard descriptor type INTERFACE (USB 2.0 Table 9-5).
pub const INTERFACE: u8 = 0x04;
/// Standard descriptor type ENDPOINT (USB 2.0 Table 9-5).
pub const ENDPOINT: u8 = 0x05;
/// Standard request GET_DESCRIPTOR (USB 2.0 Table 9-4).
pub const GET_DESCRIPTOR: u8 = 0x06;

/// Class-specific descriptor type CS_UNDEFINED.
pub const CS_UNDEFINED: u8 = 0x20;
/// Class-specific descriptor type CS_DEVICE.
pub const CS_DEVICE: u8 = 0x21;
/// Class-specific descriptor type CS_CONFIGURATION.
pub const CS_CONFIGURATION: u8 = 0x22;
/// Class-specific descriptor type CS_STRING.
pub const CS_STRING: u8 = 0x23;
/// Class-specific descriptor type CS_INTERFACE.
pub const CS_INTERFACE: u8 = 0x24;
/// Class-specific descriptor type CS_ENDPOINT.
pub const CS_ENDPOINT: u8 = 0x25;
/// Class-specific descriptor type CS_GR_TRM_BLOCK (USB-MIDI v2.0 A.1).
pub const CS_GR_TRM_BLOCK: u8 = 0x26;

/// AudioControl class-specific interface subtype HEADER (USB Audio 1.0, USB-MIDI v1.0 B.3.2).
pub const AC_HEADER: u8 = 0x01;

/// MS interface descriptor subtype MS_DESCRIPTOR_UNDEFINED.
pub const MS_DESCRIPTOR_UNDEFINED: u8 = 0x00;
/// MS interface descriptor subtype MS_HEADER.
pub const MS_HEADER: u8 = 0x01;
/// MS interface descriptor subtype MIDI_IN_JACK (USB-MIDI 1.0 only).
pub const MIDI_IN_JACK: u8 = 0x02;
/// MS interface descriptor subtype MIDI_OUT_JACK (USB-MIDI 1.0 only).
pub const MIDI_OUT_JACK: u8 = 0x03;
/// MS interface descriptor subtype ELEMENT (USB-MIDI 1.0 only).
pub const ELEMENT: u8 = 0x04;

/// MS endpoint descriptor subtype DESCRIPTOR_UNDEFINED.
pub const DESCRIPTOR_UNDEFINED: u8 = 0x00;
/// MS endpoint descriptor subtype MS_GENERAL (USB-MIDI 1.0 only).
pub const MS_GENERAL: u8 = 0x01;
/// MS endpoint descriptor subtype MS_GENERAL_2_0.
pub const MS_GENERAL_2_0: u8 = 0x02;

/// Group Terminal Block descriptor subtype GR_TRM_BLOCK_UNDEFINED.
pub const GR_TRM_BLOCK_UNDEFINED: u8 = 0x00;
/// Group Terminal Block descriptor subtype GR_TRM_BLOCK_HEADER.
pub const GR_TRM_BLOCK_HEADER: u8 = 0x01;
/// Group Terminal Block descriptor subtype GR_TRM_BLOCK.
pub const GR_TRM_BLOCK: u8 = 0x02;

/// bcdMSC of a USB-MIDI 1.0 Alternate Setting.
pub const MS_MIDI_1_0: u16 = 0x0100;
/// bcdMSC of a USB MIDI 2.0 Alternate Setting.
pub const MS_MIDI_2_0: u16 = 0x0200;

/// Jack type JACK_TYPE_UNDEFINED.
pub const JACK_TYPE_UNDEFINED: u8 = 0x00;
/// Jack type EMBEDDED.
pub const EMBEDDED: u8 = 0x01;
/// Jack type EXTERNAL.
pub const EXTERNAL: u8 = 0x02;

/// bGrpTrmBlkType BIDIRECTIONAL.
pub const BIDIRECTIONAL: u8 = 0x00;
/// bGrpTrmBlkType INPUT_ONLY (IN Group Terminals, host to device).
pub const INPUT_ONLY: u8 = 0x01;
/// bGrpTrmBlkType OUTPUT_ONLY (OUT Group Terminals, device to host).
pub const OUTPUT_ONLY: u8 = 0x02;

/// bMIDIProtocol USE_MIDI_CI (unknown).
pub const USE_MIDI_CI: u8 = 0x00;
/// bMIDIProtocol MIDI 1.0, UMPs up to 64 bits.
pub const MIDI_1_0_UP_TO_64_BITS: u8 = 0x01;
/// bMIDIProtocol MIDI 1.0, UMPs up to 64 bits, Jitter Reduction Timestamps.
pub const MIDI_1_0_UP_TO_64_BITS_AND_JRTS: u8 = 0x02;
/// bMIDIProtocol MIDI 1.0, UMPs up to 128 bits.
pub const MIDI_1_0_UP_TO_128_BITS: u8 = 0x03;
/// bMIDIProtocol MIDI 1.0, UMPs up to 128 bits, Jitter Reduction Timestamps.
pub const MIDI_1_0_UP_TO_128_BITS_AND_JRTS: u8 = 0x04;
/// bMIDIProtocol MIDI 2.0.
pub const MIDI_2_0: u8 = 0x11;
/// bMIDIProtocol MIDI 2.0, Jitter Reduction Timestamps.
pub const MIDI_2_0_AND_JRTS: u8 = 0x12;

/// Group Terminal Number of Group 1 (GROUP_n is n - 1).
pub const GROUP_1: u8 = 0x00;
/// Group Terminal Number of Group 2.
pub const GROUP_2: u8 = 0x01;
/// Group Terminal Number of Group 3.
pub const GROUP_3: u8 = 0x02;
/// Group Terminal Number of Group 4.
pub const GROUP_4: u8 = 0x03;
/// Group Terminal Number of Group 5.
pub const GROUP_5: u8 = 0x04;
/// Group Terminal Number of Group 6.
pub const GROUP_6: u8 = 0x05;
/// Group Terminal Number of Group 7.
pub const GROUP_7: u8 = 0x06;
/// Group Terminal Number of Group 8.
pub const GROUP_8: u8 = 0x07;
/// Group Terminal Number of Group 9.
pub const GROUP_9: u8 = 0x08;
/// Group Terminal Number of Group 10.
pub const GROUP_10: u8 = 0x09;
/// Group Terminal Number of Group 11.
pub const GROUP_11: u8 = 0x0A;
/// Group Terminal Number of Group 12.
pub const GROUP_12: u8 = 0x0B;
/// Group Terminal Number of Group 13.
pub const GROUP_13: u8 = 0x0C;
/// Group Terminal Number of Group 14.
pub const GROUP_14: u8 = 0x0D;
/// Group Terminal Number of Group 15.
pub const GROUP_15: u8 = 0x0E;
/// Group Terminal Number of Group 16.
pub const GROUP_16: u8 = 0x0F;

/// The direction of a MIDI Streaming Data Endpoint (bEndpointAddress D7; USB-MIDI v2.0 5.3.1,
/// USB-MIDI v1.0 6.2.1). Both the USB-MIDI 1.0 Alternate Setting and the USB MIDI 2.0 settings
/// use it.
///
/// It is not the Group Terminal direction: an IN Group Terminal is on a MIDI OUT endpoint
/// (see `gtb::TerminalDirection`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum EndpointDirection {
    /// MIDI OUT endpoint: a MIDI data sink, host to device (D7 = 0). On Alternate Setting 0 it
    /// lists Embedded MIDI IN Jacks.
    #[default]
    MidiOut,
    /// MIDI IN endpoint: a MIDI data source, device to host (D7 = 1). On Alternate Setting 0 it
    /// lists Embedded MIDI OUT Jacks.
    MidiIn,
}

impl EndpointDirection {
    /// The direction of a bEndpointAddress (D7).
    pub const fn from_address(address: u8) -> Self {
        if address & 0x80 != 0 {
            EndpointDirection::MidiIn
        } else {
            EndpointDirection::MidiOut
        }
    }

    /// bEndpointAddress of endpoint `number` (the low 4 bits) in this direction.
    pub const fn address(self, number: u8) -> u8 {
        self.address_bit() | (number & 0x0F)
    }

    /// The D7 bit of bEndpointAddress.
    pub const fn address_bit(self) -> u8 {
        match self {
            EndpointDirection::MidiOut => 0x00,
            EndpointDirection::MidiIn => 0x80,
        }
    }

    /// True for a host to device endpoint.
    pub const fn is_out(self) -> bool {
        matches!(self, EndpointDirection::MidiOut)
    }

    /// True for a device to host endpoint.
    pub const fn is_in(self) -> bool {
        matches!(self, EndpointDirection::MidiIn)
    }
}

/// bJackType of a MIDI IN or MIDI OUT Jack (USB-MIDI v1.0 A.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JackType {
    /// JACK_TYPE_UNDEFINED 0x00 (the writer never writes it).
    Undefined,
    /// EMBEDDED 0x01.
    Embedded,
    /// EXTERNAL 0x02.
    External,
    /// Any other value, kept raw by the reader.
    Unknown(u8),
}

impl JackType {
    /// The value set of a raw byte.
    pub const fn from_bits(b: u8) -> Self {
        match b {
            JACK_TYPE_UNDEFINED => JackType::Undefined,
            EMBEDDED => JackType::Embedded,
            EXTERNAL => JackType::External,
            other => JackType::Unknown(other),
        }
    }

    /// The raw byte.
    pub const fn into_bits(self) -> u8 {
        match self {
            JackType::Undefined => JACK_TYPE_UNDEFINED,
            JackType::Embedded => EMBEDDED,
            JackType::External => EXTERNAL,
            JackType::Unknown(b) => b,
        }
    }
}

/// bGrpTrmBlkType of a Group Terminal Block (USB-MIDI v2.0 A.6, REQ-USB-GTB-005).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GtbType {
    /// 0x00: each member Group Terminal Number has an IN and an OUT Group Terminal.
    Bidirectional,
    /// 0x01: IN Group Terminals only (host to device).
    InputOnly,
    /// 0x02: OUT Group Terminals only (device to host).
    OutputOnly,
    /// Any other value, kept raw by the reader (U-15).
    Unknown(u8),
}

impl GtbType {
    /// The value set of a raw byte.
    pub const fn from_bits(b: u8) -> Self {
        match b {
            BIDIRECTIONAL => GtbType::Bidirectional,
            INPUT_ONLY => GtbType::InputOnly,
            OUTPUT_ONLY => GtbType::OutputOnly,
            other => GtbType::Unknown(other),
        }
    }

    /// The raw byte.
    pub const fn into_bits(self) -> u8 {
        match self {
            GtbType::Bidirectional => BIDIRECTIONAL,
            GtbType::InputOnly => INPUT_ONLY,
            GtbType::OutputOnly => OUTPUT_ONLY,
            GtbType::Unknown(b) => b,
        }
    }

    /// True when the block has IN Group Terminals (host to device).
    pub const fn has_in(self) -> bool {
        matches!(self, GtbType::Bidirectional | GtbType::InputOnly)
    }

    /// True when the block has OUT Group Terminals (device to host).
    pub const fn has_out(self) -> bool {
        matches!(self, GtbType::Bidirectional | GtbType::OutputOnly)
    }
}

/// bMIDIProtocol of a Group Terminal Block (USB-MIDI v2.0 A.7, REQ-USB-GTB-009).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MidiProtocol {
    /// 0x00: unknown (use MIDI-CI).
    UseMidiCi,
    /// 0x01: MIDI 1.0, UMPs up to 64 bits.
    Midi1UpTo64,
    /// 0x02: MIDI 1.0, UMPs up to 64 bits, Jitter Reduction Timestamps.
    Midi1UpTo64Jr,
    /// 0x03: MIDI 1.0, UMPs up to 128 bits.
    Midi1UpTo128,
    /// 0x04: MIDI 1.0, UMPs up to 128 bits, Jitter Reduction Timestamps.
    Midi1UpTo128Jr,
    /// 0x11: MIDI 2.0.
    Midi2,
    /// 0x12: MIDI 2.0, Jitter Reduction Timestamps.
    Midi2Jr,
    /// Any other value, kept raw by the reader and read as unknown (U-15).
    Unknown(u8),
}

impl MidiProtocol {
    /// The value set of a raw byte.
    pub const fn from_bits(b: u8) -> Self {
        match b {
            USE_MIDI_CI => MidiProtocol::UseMidiCi,
            MIDI_1_0_UP_TO_64_BITS => MidiProtocol::Midi1UpTo64,
            MIDI_1_0_UP_TO_64_BITS_AND_JRTS => MidiProtocol::Midi1UpTo64Jr,
            MIDI_1_0_UP_TO_128_BITS => MidiProtocol::Midi1UpTo128,
            MIDI_1_0_UP_TO_128_BITS_AND_JRTS => MidiProtocol::Midi1UpTo128Jr,
            MIDI_2_0 => MidiProtocol::Midi2,
            MIDI_2_0_AND_JRTS => MidiProtocol::Midi2Jr,
            other => MidiProtocol::Unknown(other),
        }
    }

    /// The raw byte.
    pub const fn into_bits(self) -> u8 {
        match self {
            MidiProtocol::UseMidiCi => USE_MIDI_CI,
            MidiProtocol::Midi1UpTo64 => MIDI_1_0_UP_TO_64_BITS,
            MidiProtocol::Midi1UpTo64Jr => MIDI_1_0_UP_TO_64_BITS_AND_JRTS,
            MidiProtocol::Midi1UpTo128 => MIDI_1_0_UP_TO_128_BITS,
            MidiProtocol::Midi1UpTo128Jr => MIDI_1_0_UP_TO_128_BITS_AND_JRTS,
            MidiProtocol::Midi2 => MIDI_2_0,
            MidiProtocol::Midi2Jr => MIDI_2_0_AND_JRTS,
            MidiProtocol::Unknown(b) => b,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appendix_a_values() {
        // REQ-USB-MS2-015.
        assert_eq!(
            [CS_UNDEFINED, CS_DEVICE, CS_CONFIGURATION, CS_STRING],
            [0x20, 0x21, 0x22, 0x23]
        );
        assert_eq!(
            [CS_INTERFACE, CS_ENDPOINT, CS_GR_TRM_BLOCK],
            [0x24, 0x25, 0x26]
        );
        assert_eq!(
            [
                MS_DESCRIPTOR_UNDEFINED,
                MS_HEADER,
                MIDI_IN_JACK,
                MIDI_OUT_JACK,
                ELEMENT
            ],
            [0, 1, 2, 3, 4]
        );
        assert_eq!(
            [DESCRIPTOR_UNDEFINED, MS_GENERAL, MS_GENERAL_2_0],
            [0, 1, 2]
        );
        assert_eq!((MS_MIDI_1_0, MS_MIDI_2_0), (0x0100, 0x0200));
        assert_eq!([JACK_TYPE_UNDEFINED, EMBEDDED, EXTERNAL], [0, 1, 2]);
        // REQ-USB-GTB-021.
        assert_eq!(
            [GR_TRM_BLOCK_UNDEFINED, GR_TRM_BLOCK_HEADER, GR_TRM_BLOCK],
            [0, 1, 2]
        );
        assert_eq!([BIDIRECTIONAL, INPUT_ONLY, OUTPUT_ONLY], [0, 1, 2]);
        assert_eq!(
            [
                USE_MIDI_CI,
                MIDI_1_0_UP_TO_64_BITS,
                MIDI_1_0_UP_TO_64_BITS_AND_JRTS,
                MIDI_1_0_UP_TO_128_BITS,
                MIDI_1_0_UP_TO_128_BITS_AND_JRTS,
                MIDI_2_0,
                MIDI_2_0_AND_JRTS
            ],
            [0x00, 0x01, 0x02, 0x03, 0x04, 0x11, 0x12]
        );
        assert_eq!(
            [
                GROUP_1, GROUP_2, GROUP_3, GROUP_4, GROUP_5, GROUP_6, GROUP_7, GROUP_8, GROUP_9,
                GROUP_10, GROUP_11, GROUP_12, GROUP_13, GROUP_14, GROUP_15, GROUP_16
            ],
            [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]
        );
    }

    #[test]
    fn endpoint_direction_from_and_to_address() {
        // REQ-USB-MS2-015: bEndpointAddress D7 gives the direction of a MIDI endpoint.
        for a in 0..=255u8 {
            let d = EndpointDirection::from_address(a);
            assert_eq!(d.address_bit(), a & 0x80);
            assert_eq!(d.address(a), a & 0x8F);
            assert_eq!(d.is_in(), a & 0x80 != 0);
            assert_eq!(d.is_out(), a & 0x80 == 0);
        }
        assert_eq!(EndpointDirection::MidiIn.address(1), 0x81);
        assert_eq!(EndpointDirection::MidiOut.address(0x12), 0x02);
        assert_eq!(EndpointDirection::default(), EndpointDirection::MidiOut);
    }

    #[test]
    fn value_sets_round_trip_and_keep_unknown() {
        // U-15: the reader keeps a value outside the set as a raw value.
        for b in 0..=255u8 {
            assert_eq!(JackType::from_bits(b).into_bits(), b);
            assert_eq!(GtbType::from_bits(b).into_bits(), b);
            assert_eq!(MidiProtocol::from_bits(b).into_bits(), b);
        }
        assert_eq!(GtbType::from_bits(3), GtbType::Unknown(3));
        assert_eq!(MidiProtocol::from_bits(0x13), MidiProtocol::Unknown(0x13));
        assert_eq!(JackType::from_bits(9), JackType::Unknown(9));
        assert!(GtbType::Bidirectional.has_in() && GtbType::Bidirectional.has_out());
        assert!(GtbType::InputOnly.has_in() && !GtbType::InputOnly.has_out());
        assert!(!GtbType::Unknown(7).has_in() && !GtbType::Unknown(7).has_out());
    }
}
