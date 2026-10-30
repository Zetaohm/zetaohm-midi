//! The device-side configuration of one USB MIDI function and the writer of its descriptors
//! (USB-MIDI v2.0 sections 3.1.1, 4 and Appendix B; USB-MIDI v1.0 Appendix B; DEC-014).
//!
//! - [`FunctionConfig`] holds the AudioControl interface number, the MIDI Streaming interface
//!   number (the USB stack assigns both, so the caller gives them), the [`BusSpeed`], the
//!   USB-MIDI 1.0 Alternate Setting 0 ([`Midi1Setting`]) and the USB MIDI 2.0 Alternate
//!   Settings 1 and higher ([`Midi2Setting`]). The const generics are the capacities of the
//!   Jacks (`J`), of the USB MIDI 2.0 Alternate Settings (`S`), of the endpoints of one setting
//!   (`E`) and of the Group Terminal Blocks of one setting (`B`) (DEC-002).
//! - The USB-MIDI 1.0 setting has the cable-count form [`CableCounts`] or the full Jack list
//!   form ([`Ms1Topology`]) (DEC-USB-001 choice U-26). The cable-count form gives the
//!   Appendix B topology and the Jack IDs 1, 2, 3 and higher in descriptor order
//!   (REQ-USB-MS1-027, REQ-USB-MS1-028). The checks of a full Jack list are in the Ms1 module.
//! - [`write_function_to`] gives the same descriptors one at a time to a caller
//!   [`DescriptorSink`]: the fields of each standard interface and endpoint descriptor, and the
//!   type and the bytes of each class-specific descriptor, for a USB stack that writes the
//!   standard descriptors itself (REQ-USB-GEN-013). [`FunctionConfig::endpoints`] gives the
//!   endpoints of each Alternate Setting, so that the stack can allocate them first.
//! - [`write_function`] writes the AudioControl interface, then the MIDI Streaming interface
//!   with all Alternate Settings in order: Alternate Setting 0 as USB-MIDI 1.0
//!   (REQ-USB-GEN-001), then the USB MIDI 2.0 settings with the numbers 1, 2 and higher
//!   (REQ-USB-GEN-004, REQ-USB-GEN-005), all with the same bInterfaceNumber (REQ-USB-GEN-007).
//!   [`function_len`] gives the same length before the write, for wTotalLength of the
//!   Configuration descriptor (REQ-USB-MS1-026). A configuration with no USB-MIDI 1.0 setting
//!   is an error, and nothing is written (REQ-USB-GEN-001). A configuration with no USB MIDI
//!   2.0 setting is a plain USB-MIDI 1.0 function (choice U-02).
//! - The Group Terminal Block set of a USB MIDI 2.0 setting has its own buffer: [`gtb_set`]
//!   selects it by the Alternate Setting number, for the Request module. The function writer
//!   never puts the set inline (REQ-USB-GTB-015, choice U-12).
//! - The writer does not write an Interface Association Descriptor: the USB stack owns it
//!   (choice U-28). The string indexes default to 0 (choice U-27).
//!
//! The writers check the whole configuration and the buffer length first; on an error they
//! return [`DeviceError`] and write nothing. Nothing panics. No alloc (DEC-002), synchronous
//! and sans-IO, no clock and no USB stack dependency (DEC-004, DEC-014).

use crate::ac::{audio_control_len, write_audio_control, AcError, AcHeader, StandardAcInterface};
use crate::constants::{EndpointDirection, JackType, ENDPOINT, INTERFACE};
use crate::gtb::{GtbError, GtbList};
use crate::ms1::{
    Jack, MidiInJack, MidiOutJack, Ms1Endpoint, Ms1Error, Ms1Topology, MsHeader, Pin,
    HEADER_LEN as MS1_HEADER_LEN, IN_JACK_LEN, MAX_ENDPOINT_JACKS,
};
use crate::ms2::{
    check_max_packet, BusSpeed, EndpointConfig, FixedList, Ms2Error, Ms2Header, Ms2Setting,
    MsInterface, TransferType, INTERFACE_LEN,
};
use crate::walk::{Descriptors, RawDesc};

/// bLength of a MIDI OUT Jack with one Input Pin: 7 + 2 x 1.
const OUT_JACK_1_PIN_LEN: usize = 9;
/// The standard endpoint descriptor (9 bytes) and the MS_GENERAL header (4 bytes) of
/// Alternate Setting 0, without the n Jack IDs.
const MS1_ENDPOINT_FIXED_LEN: usize = 9 + 4;

/// Error of the Device writers. Each variant names the rule that the configuration or the
/// buffer broke. On every error, nothing is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DeviceError {
    /// The caller buffer is too small (REQ-USB-MS1-026, REQ-USB-GTB-015).
    #[error("buffer too small")]
    Overflow,
    /// The configuration has no USB-MIDI 1.0 Alternate Setting 0 (REQ-USB-GEN-001).
    #[error("no USB-MIDI 1.0 Alternate Setting 0")]
    NoMidi1Setting,
    /// The AudioControl and the MIDI Streaming interface have the same bInterfaceNumber.
    #[error("AudioControl and MIDI Streaming interface number {0} are the same")]
    SameInterfaceNumber(u8),
    /// Cable counts above 16, or both 0 in a function with no USB MIDI 2.0 setting
    /// (REQ-USB-MS1-027).
    #[error(
        "cable counts n_out {n_out} and n_in {n_in}: each 0 to 16, not both 0 with no USB MIDI 2.0 setting (REQ-USB-MS1-027)"
    )]
    CableCount {
        /// The cable count of the MIDI OUT endpoint.
        n_out: u8,
        /// The cable count of the MIDI IN endpoint.
        n_in: u8,
    },
    /// A wMaxPacketSize of a bulk endpoint of Alternate Setting 0 that USB 2.0 does not permit
    /// at the bus speed (REQ-USB-MS2-016, choice U-25).
    #[error("Alternate Setting 0: wMaxPacketSize {0} not permitted at this bus speed")]
    Midi1PacketSize(u16),
    /// More USB MIDI 2.0 Alternate Settings than the capacity `S`, or than bAlternateSetting
    /// can number (255) (REQ-USB-GEN-005).
    #[error("too many USB MIDI 2.0 Alternate Settings (REQ-USB-GEN-005)")]
    TooManySettings,
    /// The Alternate Setting number is 0 or names no USB MIDI 2.0 setting (REQ-USB-GTB-015).
    #[error("no USB MIDI 2.0 Alternate Setting {0}")]
    NoSuchSetting(u8),
    /// A Jack or an endpoint on Alternate Setting 0 in a build without the feature `alt0`, which
    /// has no Alternate Setting 0 data path (REQ-USB-GEN-014).
    #[error(
        "Alternate Setting 0 has Jacks or endpoints, but the feature alt0 is off (REQ-USB-GEN-014)"
    )]
    Alt0Disabled,
    /// The descriptors are longer than wTotalLength of the Configuration descriptor can give.
    #[error("descriptors longer than 0xFFFF bytes")]
    TooLong,
    /// A rule of the AudioControl interface (REQ-USB-AC-001 to REQ-USB-AC-004).
    #[error("AudioControl interface: {0}")]
    Ac(AcError),
    /// A rule of the USB-MIDI 1.0 setting: the Jack topology or an endpoint (REQ-USB-MS1-*,
    /// REQ-USB-TOPO-*).
    #[error("Alternate Setting 0: {0}")]
    Midi1(Ms1Error),
    /// A rule of a USB MIDI 2.0 setting (REQ-USB-MS2-*, REQ-USB-GEN-008).
    #[error("Alternate Setting {alt}: {error}")]
    Midi2 {
        /// bAlternateSetting.
        alt: u8,
        /// The broken rule.
        error: Ms2Error,
    },
    /// A rule of the Group Terminal Blocks of a USB MIDI 2.0 setting (REQ-USB-GTB-*,
    /// REQ-USB-GEN-010, REQ-USB-GEN-011).
    #[error("Alternate Setting {alt}: Group Terminal Blocks: {error}")]
    Gtb {
        /// bAlternateSetting.
        alt: u8,
        /// The broken rule.
        error: GtbError,
    },
}

impl From<AcError> for DeviceError {
    fn from(e: AcError) -> Self {
        match e {
            AcError::Overflow => DeviceError::Overflow,
            other => DeviceError::Ac(other),
        }
    }
}

impl From<Ms1Error> for DeviceError {
    fn from(e: Ms1Error) -> Self {
        match e {
            Ms1Error::Overflow => DeviceError::Overflow,
            other => DeviceError::Midi1(other),
        }
    }
}

fn midi2_error(alt: u8) -> impl Fn(Ms2Error) -> DeviceError {
    move |e| match e {
        Ms2Error::Overflow => DeviceError::Overflow,
        error => DeviceError::Midi2 { alt, error },
    }
}

fn gtb_error(alt: u8) -> impl Fn(GtbError) -> DeviceError {
    move |e| match e {
        GtbError::Overflow => DeviceError::Overflow,
        error => DeviceError::Gtb { alt, error },
    }
}

/// The cable-count form of the USB-MIDI 1.0 setting (choice U-26, REQ-USB-MS1-027,
/// REQ-USB-MS1-028).
///
/// The writer builds, for each k from 0 to max(n_out, n_in) - 1: if k < n_out an Embedded
/// MIDI IN Jack; if k < n_in an External MIDI IN Jack and an Embedded MIDI OUT Jack from it;
/// if k < n_out an External MIDI OUT Jack from the Embedded MIDI IN Jack of k. The Jack IDs
/// are 1, 2, 3 and higher in descriptor order (4k+1 to 4k+4 when n_out = n_in), baSourcePin
/// is 1 and iJack is 0 (choice U-27; a caller that needs Jack strings gives the full Jack
/// list). The MIDI OUT endpoint is written before the MIDI IN endpoint; an endpoint with a
/// cable count of 0 is not written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CableCounts {
    /// The cable count of the MIDI OUT endpoint (host to device), 0 to 16.
    pub n_out: u8,
    /// The cable count of the MIDI IN endpoint (device to host), 0 to 16.
    pub n_in: u8,
    /// The endpoint number of the MIDI OUT endpoint, 1 to 15.
    pub out_endpoint: u8,
    /// The endpoint number of the MIDI IN endpoint, 1 to 15.
    pub in_endpoint: u8,
    /// wMaxPacketSize of both bulk endpoints.
    pub max_packet: u16,
}

impl CableCounts {
    /// `n_out` and `n_in` cables on bulk endpoints OUT 1 and IN 1, with the default
    /// wMaxPacketSize of `speed`: 64 at full speed, 512 at high speed (choices U-06, U-25).
    /// The writer checks the counts.
    pub const fn new(n_out: u8, n_in: u8, speed: BusSpeed) -> Self {
        CableCounts {
            n_out,
            n_in,
            out_endpoint: 1,
            in_endpoint: 1,
            max_packet: speed.default_max_packet(),
        }
    }

    /// One cable in each direction (the default, REQ-USB-MS1-027).
    pub const fn default_for(speed: BusSpeed) -> Self {
        Self::new(1, 1, speed)
    }

    /// The number of MIDI endpoints: an endpoint with a count of 0 is not written
    /// (REQ-USB-MS1-028).
    pub const fn num_endpoints(&self) -> u8 {
        (self.n_out > 0) as u8 + (self.n_in > 0) as u8
    }

    /// The bytes after the standard interface descriptor: the header, 15 bytes of Jacks for
    /// each cable, and each written endpoint (9 + 4 + n).
    pub const fn body_len(&self) -> usize {
        let (o, i) = (self.n_out as usize, self.n_in as usize);
        let mut len = MS1_HEADER_LEN + (IN_JACK_LEN + OUT_JACK_1_PIN_LEN) * (o + i);
        if o > 0 {
            len += MS1_ENDPOINT_FIXED_LEN + o;
        }
        if i > 0 {
            len += MS1_ENDPOINT_FIXED_LEN + i;
        }
        len
    }

    /// `midi2`: the function has a USB MIDI 2.0 setting. Only then may both counts be 0: an
    /// Alternate Setting 0 with no Jack and no endpoint, which a USB MIDI 1.0 host shows with no
    /// port (OPT-050 option 3, form B; REQ-USB-MS1-027).
    fn check(&self, speed: BusSpeed, midi2: bool) -> Result<(), DeviceError> {
        // The largest cable count of one endpoint is the number of Embedded Jacks of one
        // endpoint, 16 (USB-MIDI v1.0 3.2.1, REQ-USB-MS1-027; `event_packet::MAX_CABLES`).
        let max = MAX_ENDPOINT_JACKS;
        if usize::from(self.n_out) > max
            || usize::from(self.n_in) > max
            || ((self.n_out | self.n_in) == 0 && !midi2)
        {
            return Err(DeviceError::CableCount {
                n_out: self.n_out,
                n_in: self.n_in,
            });
        }
        for (n, number) in [
            (self.n_out, self.out_endpoint),
            (self.n_in, self.in_endpoint),
        ] {
            if n > 0 && !(1..=15).contains(&number) {
                return Err(Ms1Error::EndpointNumber(number).into());
            }
        }
        check_midi1_packet(speed, self.max_packet)
    }

    /// Gives the body (header, Jacks, endpoints) to the emitter, one descriptor piece at a
    /// time. The caller checked the counts.
    fn emit_body<S: DescriptorSink>(
        &self,
        em: &mut Emitter<'_, S>,
    ) -> Result<(), EmitError<S::Error>> {
        // `total` is at most 7 + 15 x 32 + 2 x 29 bytes.
        let total = u16::try_from(self.body_len()).map_err(|_| DeviceError::TooLong)?;
        em.piece(|b| Ok(MsHeader::new(total).write(b)?))?;
        let mut emb_in = [0u8; MAX_ENDPOINT_JACKS];
        let mut emb_out = [0u8; MAX_ENDPOINT_JACKS];
        let mut next_id: u8 = 1;
        let mut take_id = || {
            let id = next_id;
            next_id = next_id.wrapping_add(1);
            id
        };
        let cables = usize::from(self.n_out.max(self.n_in)).min(MAX_ENDPOINT_JACKS);
        for k in 0..cables {
            let mut jacks: [Option<Jack<'_>>; 4] = [None; 4];
            let pins_out: [Pin; 1];
            let pins_ext: [Pin; 1];
            if k < usize::from(self.n_out) {
                let id = take_id();
                emb_in[k] = id;
                jacks[0] = Some(Jack::In(MidiInJack::new(JackType::Embedded, id)));
            }
            if k < usize::from(self.n_in) {
                let ext = take_id();
                let id = take_id();
                emb_out[k] = id;
                pins_out = [Pin::new(ext, 1)];
                jacks[1] = Some(Jack::In(MidiInJack::new(JackType::External, ext)));
                jacks[2] = Some(Jack::Out(MidiOutJack::new(
                    JackType::Embedded,
                    id,
                    &pins_out,
                )));
            }
            if k < usize::from(self.n_out) {
                let id = take_id();
                pins_ext = [Pin::new(emb_in[k], 1)];
                jacks[3] = Some(Jack::Out(MidiOutJack::new(
                    JackType::External,
                    id,
                    &pins_ext,
                )));
            }
            for j in jacks.iter().flatten() {
                em.piece(|b| Ok(j.write(b)?))?;
            }
        }
        let endpoints = [
            (
                EndpointDirection::MidiOut,
                self.out_endpoint,
                emb_in.get(..usize::from(self.n_out)),
            ),
            (
                EndpointDirection::MidiIn,
                self.in_endpoint,
                emb_out.get(..usize::from(self.n_in)),
            ),
        ];
        for (dir, number, jacks) in endpoints {
            let jacks = jacks.unwrap_or(&[]);
            if jacks.is_empty() {
                continue;
            }
            let ep = Ms1Endpoint::new(dir, number, self.max_packet, jacks);
            em.piece(|b| Ok(ep.write(b)?))?;
        }
        Ok(())
    }
}

fn check_midi1_packet(speed: BusSpeed, max_packet: u16) -> Result<(), DeviceError> {
    check_max_packet(TransferType::Bulk, speed, max_packet)
        .map_err(|_| DeviceError::Midi1PacketSize(max_packet))
}

/// The two forms of the Jacks of the USB-MIDI 1.0 setting (choice U-26).
#[derive(Debug, Clone)]
pub enum Midi1Form<'a, const J: usize, const E: usize> {
    /// The cable-count form (REQ-USB-MS1-027, REQ-USB-MS1-028).
    Cables(CableCounts),
    /// The full Jack list with its endpoints; [`Ms1Topology::check`] gives its rules.
    Jacks(Ms1Topology<'a, J, E>),
}

/// The USB-MIDI 1.0 Alternate Setting 0 of the function (REQ-USB-GEN-001).
#[derive(Debug, Clone)]
pub struct Midi1Setting<'a, const J: usize, const E: usize> {
    /// The Jacks and the endpoints.
    pub form: Midi1Form<'a, J, E>,
    /// iInterface of the standard MIDI Streaming interface descriptor; 0 is no string
    /// (choice U-27).
    pub string_index: u8,
}

impl<'a, const J: usize, const E: usize> Midi1Setting<'a, J, E> {
    /// The cable-count form with no string.
    pub const fn cables(counts: CableCounts) -> Self {
        Midi1Setting {
            form: Midi1Form::Cables(counts),
            string_index: 0,
        }
    }

    /// The full Jack list form with no string.
    pub const fn jacks(topology: Ms1Topology<'a, J, E>) -> Self {
        Midi1Setting {
            form: Midi1Form::Jacks(topology),
            string_index: 0,
        }
    }

    /// bNumEndpoints of the standard interface descriptor.
    pub fn num_endpoints(&self) -> u8 {
        match &self.form {
            Midi1Form::Cables(c) => c.num_endpoints(),
            Midi1Form::Jacks(t) => t.num_endpoints(),
        }
    }

    /// The bytes of the setting: the standard interface descriptor and the body.
    pub fn len(&self) -> usize {
        INTERFACE_LEN
            + match &self.form {
                Midi1Form::Cables(c) => c.body_len(),
                Midi1Form::Jacks(t) => t.len(),
            }
    }

    /// Never true.
    pub fn is_empty(&self) -> bool {
        false
    }

    /// True when the setting has a Jack or an endpoint.
    fn has_port(&self) -> bool {
        match &self.form {
            Midi1Form::Cables(c) => (c.n_out | c.n_in) != 0,
            Midi1Form::Jacks(t) => t.jacks().next().is_some() || t.num_endpoints() > 0,
        }
    }

    fn check(&self, speed: BusSpeed, midi2: bool) -> Result<(), DeviceError> {
        if !cfg!(feature = "alt0") && self.has_port() {
            return Err(DeviceError::Alt0Disabled);
        }
        match &self.form {
            Midi1Form::Cables(c) => c.check(speed, midi2),
            Midi1Form::Jacks(t) => {
                t.check()?;
                t.endpoints()
                    .try_for_each(|e| check_midi1_packet(speed, e.max_packet_size))
            }
        }
    }

    /// Gives the setting to the emitter: the standard interface descriptor, then the body.
    /// The caller checked the setting.
    fn emit<S: DescriptorSink>(
        &self,
        ms_interface: u8,
        em: &mut Emitter<'_, S>,
    ) -> Result<(), EmitError<S::Error>> {
        let interface = MsInterface {
            interface_number: ms_interface,
            alternate_setting: 0,
            num_endpoints: self.num_endpoints(),
            string_index: self.string_index,
        };
        em.piece(|b| interface.write(b).map_err(midi2_error(0)))?;
        match &self.form {
            Midi1Form::Cables(c) => c.emit_body(em),
            Midi1Form::Jacks(t) => {
                // The check limits the length to 0xFFFF (REQ-USB-MS1-004).
                let total = u16::try_from(t.len()).map_err(|_| DeviceError::TooLong)?;
                em.piece(|b| Ok(MsHeader::new(total).write(b)?))?;
                for jack in t.jacks_in_write_order() {
                    let jack = jack.map_err(DeviceError::from)?;
                    em.piece(|b| Ok(jack.write(b)?))?;
                }
                for ep in t.endpoints() {
                    em.piece(|b| Ok(ep.write(b)?))?;
                }
                Ok(())
            }
        }
    }
}

/// One USB MIDI 2.0 Alternate Setting of the function: its endpoints and its Group Terminal
/// Blocks (REQ-USB-GEN-004, REQ-USB-GEN-005, REQ-USB-GTB-015). The function gives the
/// interface number, the Alternate Setting number and the bus speed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Midi2Setting<const E: usize, const B: usize> {
    /// iInterface; 0 is no string (choice U-27).
    pub string_index: u8,
    /// The endpoints, at least one (REQ-USB-GEN-008).
    pub endpoints: FixedList<EndpointConfig, E>,
    /// The Group Terminal Blocks of this setting; the default list is the block of choice
    /// U-09.
    pub blocks: GtbList<B>,
}

impl<const E: usize, const B: usize> Default for Midi2Setting<E, B> {
    /// [`Midi2Setting::empty`].
    fn default() -> Self {
        Self::empty()
    }
}

impl<const E: usize, const B: usize> Midi2Setting<E, B> {
    /// A setting with no endpoint (add them with `endpoints.push`) and the default Group
    /// Terminal Block list (choice U-09).
    pub fn empty() -> Self {
        Midi2Setting {
            string_index: 0,
            endpoints: FixedList::new(),
            blocks: GtbList::default_list(),
        }
    }

    /// The default setting (choices U-06, U-09): one bulk MIDI OUT endpoint 1 and one bulk
    /// MIDI IN endpoint 1 with the default wMaxPacketSize of `speed`, both on the default
    /// Group Terminal Block. [`DeviceError::Midi2`] with [`Ms2Error::TooManyEndpoints`] if
    /// `E` is below 2.
    pub fn new(speed: BusSpeed) -> Result<Self, DeviceError> {
        let mut s = Self::empty();
        for dir in [EndpointDirection::MidiOut, EndpointDirection::MidiIn] {
            s.endpoints
                .push(EndpointConfig::bulk(dir, 1, speed))
                .map_err(|_| DeviceError::Midi2 {
                    alt: 1,
                    error: Ms2Error::TooManyEndpoints,
                })?;
        }
        Ok(s)
    }

    /// The Ms2 writer of this setting with its function values.
    fn ms2(&self, ms_interface: u8, alt: u8, speed: BusSpeed) -> Ms2Setting<E> {
        Ms2Setting {
            interface_number: ms_interface,
            alternate_setting: alt,
            string_index: self.string_index,
            speed,
            endpoints: self.endpoints,
        }
    }
}

/// The device-side configuration of one USB MIDI function (USB-MIDI v2.0 3.1.1 and 4).
///
/// `J` is the Jack capacity of a full Jack list, `S` the capacity of USB MIDI 2.0 Alternate
/// Settings (default 1, REQ-USB-GEN-005), `E` the endpoint capacity of one setting and `B`
/// the Group Terminal Block capacity of one setting (DEC-002). The lifetime is the one of the
/// caller slices of a full Jack list.
#[derive(Debug, Clone)]
pub struct FunctionConfig<
    'a,
    const J: usize = 4,
    const S: usize = 1,
    const E: usize = 2,
    const B: usize = 1,
> {
    /// bInterfaceNumber of the AudioControl interface, from the USB stack.
    pub ac_interface: u8,
    /// bInterfaceNumber of the MIDI Streaming interface, from the USB stack. All Alternate
    /// Settings use it (REQ-USB-GEN-007).
    pub ms_interface: u8,
    /// iInterface of the AudioControl interface; 0 is no string (choice U-27).
    pub ac_string: u8,
    /// The bus speed: it gives the default and the permitted wMaxPacketSize values.
    pub speed: BusSpeed,
    /// Alternate Setting 0. `None` is an error of the writer (REQ-USB-GEN-001).
    pub midi1: Option<Midi1Setting<'a, J, E>>,
    /// Alternate Settings 1, 2 and higher, in this order (REQ-USB-GEN-004, REQ-USB-GEN-005).
    /// Empty: a plain USB-MIDI 1.0 function (choice U-02).
    pub midi2: FixedList<Midi2Setting<E, B>, S>,
}

impl<'a, const J: usize, const S: usize, const E: usize, const B: usize>
    FunctionConfig<'a, J, S, E, B>
{
    /// The default configuration: Alternate Setting 0 with one cable in each direction on bulk
    /// endpoints OUT 1 and IN 1, and, when `S` is 1 or more, one USB MIDI 2.0 Alternate
    /// Setting 1 with bulk endpoints OUT 1 and IN 1 and the default Group Terminal Block
    /// (choices U-06, U-09, U-26). wMaxPacketSize is 64 at full speed and 512 at high speed.
    ///
    /// Without the feature `alt0`, Alternate Setting 0 has no Jack and no endpoint
    /// (REQ-USB-GEN-014, REQ-USB-MS1-027).
    pub fn new(ac_interface: u8, ms_interface: u8, speed: BusSpeed) -> Result<Self, DeviceError> {
        let mut cfg = Self::midi1_only(ac_interface, ms_interface, speed);
        if !cfg!(feature = "alt0") {
            cfg.midi1 = Some(Midi1Setting::cables(CableCounts::new(0, 0, speed)));
        }
        if S > 0 {
            cfg.add_midi2_setting(Midi2Setting::new(speed)?)?;
        }
        Ok(cfg)
    }

    /// A plain USB-MIDI 1.0 function (choice U-02): Alternate Setting 0 with one cable in
    /// each direction and no USB MIDI 2.0 setting.
    pub fn midi1_only(ac_interface: u8, ms_interface: u8, speed: BusSpeed) -> Self {
        FunctionConfig {
            ac_interface,
            ms_interface,
            ac_string: 0,
            speed,
            midi1: Some(Midi1Setting::cables(CableCounts::default_for(speed))),
            midi2: FixedList::new(),
        }
    }

    /// Adds a USB MIDI 2.0 Alternate Setting at the end and gives its number: 1 for the first,
    /// then 2 and higher (REQ-USB-GEN-004, REQ-USB-GEN-005).
    pub fn add_midi2_setting(&mut self, setting: Midi2Setting<E, B>) -> Result<u8, DeviceError> {
        let alt = u8::try_from(self.midi2.len() + 1).map_err(|_| DeviceError::TooManySettings)?;
        self.midi2
            .push(setting)
            .map_err(|_| DeviceError::TooManySettings)?;
        Ok(alt)
    }

    /// The USB MIDI 2.0 setting with bAlternateSetting `alt` (1 or more).
    pub fn midi2_setting(&self, alt: u8) -> Option<&Midi2Setting<E, B>> {
        self.midi2.get(usize::from(alt).checked_sub(1)?)
    }

    /// The number of Alternate Settings that the writer writes: 1 + the USB MIDI 2.0 ones.
    pub fn alternate_settings(&self) -> usize {
        1 + self.midi2.len()
    }

    fn ac_header(&self) -> Result<AcHeader<1>, DeviceError> {
        Ok(AcHeader::single(self.ms_interface)?)
    }

    /// The endpoints that the writer writes, in descriptor order: those of Alternate Setting 0,
    /// then those of each USB MIDI 2.0 setting (REQ-USB-GEN-013). A USB stack allocates them
    /// before it writes the descriptors. One endpoint address can come in more than one
    /// Alternate Setting (Appendix B: OUT 1 and IN 1 in settings 0 and 1); the stack allocates
    /// it one time. The list is the one of a configuration that [`function_len`] accepts.
    pub fn endpoints(&self) -> impl Iterator<Item = SettingEndpoint> + '_ {
        let alt0 = |direction, number, max_packet| SettingEndpoint {
            alternate_setting: 0,
            direction,
            number,
            transfer: TransferType::Bulk,
            max_packet,
            interval: 0,
        };
        let (cables, jacks) = match self.midi1.as_ref().map(|m| &m.form) {
            Some(Midi1Form::Cables(c)) => (Some(c), None),
            Some(Midi1Form::Jacks(t)) => (None, Some(t)),
            None => (None, None),
        };
        let from_cables = cables.into_iter().flat_map(move |c| {
            [
                (c.n_out, EndpointDirection::MidiOut, c.out_endpoint),
                (c.n_in, EndpointDirection::MidiIn, c.in_endpoint),
            ]
            .into_iter()
            .filter(|(n, _, _)| *n > 0)
            .map(move |(_, direction, number)| alt0(direction, number, c.max_packet))
        });
        let from_jacks = jacks.into_iter().flat_map(move |t| {
            t.endpoints()
                .map(move |e| alt0(e.direction, e.number, e.max_packet_size))
        });
        let from_midi2 = (1..=u8::MAX).zip(self.midi2.iter()).flat_map(|(alt, s)| {
            s.endpoints.iter().map(move |e| SettingEndpoint {
                alternate_setting: alt,
                direction: e.direction,
                number: e.number,
                transfer: e.transfer,
                max_packet: e.max_packet,
                // The writer writes 0 for a bulk endpoint.
                interval: match e.transfer {
                    TransferType::Interrupt => e.interval,
                    _ => 0,
                },
            })
        });
        from_cables.chain(from_jacks).chain(from_midi2)
    }

    /// Checks every writer rule and gives the total length.
    fn checked_len(&self) -> Result<usize, DeviceError> {
        let midi1 = self.midi1.as_ref().ok_or(DeviceError::NoMidi1Setting)?;
        if self.ac_interface == self.ms_interface {
            return Err(DeviceError::SameInterfaceNumber(self.ms_interface));
        }
        let mut total = audio_control_len(&self.ac_header()?);
        midi1.check(self.speed, !self.midi2.is_empty())?;
        total += midi1.len();
        if self.midi2.len() > usize::from(u8::MAX) {
            return Err(DeviceError::TooManySettings);
        }
        for (i, s) in self.midi2.iter().enumerate() {
            // At most 255 settings: the number fits in a byte.
            let alt = (i + 1) as u8;
            let ms2 = s.ms2(self.ms_interface, alt, self.speed);
            ms2.validate().map_err(midi2_error(alt))?;
            s.blocks
                .validate_endpoints(&s.endpoints)
                .map_err(gtb_error(alt))?;
            total += ms2.len();
        }
        if total > usize::from(u16::MAX) {
            return Err(DeviceError::TooLong);
        }
        Ok(total)
    }
}

/// The number of bytes that [`write_function`] writes, given before the write so that the USB
/// stack can add it to wTotalLength of the Configuration descriptor (REQ-USB-MS1-026). It
/// checks the same rules as the writer and gives the same error.
///
/// The length is the one of the function only, at most 0xFFFF ([`DeviceError::TooLong`]).
/// The stack adds the Configuration descriptor (9 bytes), the Interface Association
/// Descriptor (8 bytes, choice U-28) and its other interfaces, and must check that the sum
/// fits in wTotalLength (16 bits) (review robustness L6).
pub fn function_len<const J: usize, const S: usize, const E: usize, const B: usize>(
    cfg: &FunctionConfig<'_, J, S, E, B>,
) -> Result<usize, DeviceError> {
    cfg.checked_len()
}

/// Writes the descriptors of the function into `out` and gives the number of bytes
/// (REQ-USB-MS1-026): the AudioControl interface one time, before the MIDI Streaming
/// interface (REQ-USB-AC-001); Alternate Setting 0 as USB-MIDI 1.0 (REQ-USB-GEN-001); then
/// the USB MIDI 2.0 Alternate Settings 1, 2 and higher in the order of the configuration
/// (REQ-USB-GEN-004, REQ-USB-GEN-005). Every Alternate Setting has the bInterfaceNumber of
/// the caller (REQ-USB-GEN-007). No Group Terminal Block descriptor (choice U-12) and no
/// Interface Association Descriptor (choice U-28) is written.
///
/// A broken rule or a buffer shorter than [`function_len`] gives an error, and nothing is
/// written.
pub fn write_function<const J: usize, const S: usize, const E: usize, const B: usize>(
    cfg: &FunctionConfig<'_, J, S, E, B>,
    out: &mut [u8],
) -> Result<usize, DeviceError> {
    let total = cfg.checked_len()?;
    if out.len() < total {
        return Err(DeviceError::Overflow);
    }
    let mut sink = SliceSink { out, at: 0 };
    match write_function_to(cfg, &mut sink) {
        Ok(()) => Ok(sink.at),
        Err(EmitError::Device(e) | EmitError::Sink(e)) => Err(e),
    }
}

/// The fields of a standard interface descriptor (USB 2.0 Table 9-12), as
/// [`DescriptorSink::interface`] gets them. bLength is 9 and bDescriptorType is 0x04.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InterfaceFields {
    /// bInterfaceNumber. 8 bits.
    pub interface_number: u8,
    /// bAlternateSetting. 8 bits.
    pub alternate_setting: u8,
    /// bNumEndpoints. 8 bits.
    pub num_endpoints: u8,
    /// bInterfaceClass: 0x01 (AUDIO). 8 bits.
    pub class: u8,
    /// bInterfaceSubClass: 0x01 (AUDIOCONTROL) or 0x03 (MIDISTREAMING). 8 bits.
    pub subclass: u8,
    /// bInterfaceProtocol: 0x00. 8 bits.
    pub protocol: u8,
    /// iInterface; 0 is no string (choice U-27). 8 bits.
    pub string_index: u8,
}

/// The fields of a standard endpoint descriptor (USB 2.0 Table 9-13), as
/// [`DescriptorSink::endpoint`] gets them. bDescriptorType is 0x05.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EndpointFields {
    /// bEndpointAddress: D7 direction (1 = IN), D3..0 number. 8 bits.
    pub address: u8,
    /// bmAttributes: D1..0 the transfer type (0x02 bulk, 0x03 interrupt). 8 bits.
    pub attributes: u8,
    /// wMaxPacketSize. 16 bits.
    pub max_packet: u16,
    /// bInterval. 8 bits.
    pub interval: u8,
}

/// A receiver of the descriptors of the function, one call for each descriptor in the order
/// of [`write_function`] (REQ-USB-GEN-013, DEC-004, DEC-014). A USB stack adapter implements
/// it with the calls of its stack: the stack writes the standard interface and endpoint
/// descriptors from the fields, and the class-specific descriptors from their bytes.
pub trait DescriptorSink {
    /// The error of the sink; [`write_function_to`] stops at the first one.
    type Error;

    /// A standard interface descriptor (9 bytes).
    fn interface(&mut self, fields: &InterfaceFields) -> Result<(), Self::Error>;

    /// A standard endpoint descriptor: the fields of its first 7 bytes, and `extra`, the bytes
    /// after them. `extra` is bRefresh and bSynchAddress (both 0) of the 9-byte form of
    /// Alternate Setting 0 (USB-MIDI v1.0 Table 6-6), and empty for a USB MIDI 2.0 setting
    /// (USB-MIDI v2.0 Table 5-3).
    fn endpoint(&mut self, fields: &EndpointFields, extra: &[u8]) -> Result<(), Self::Error>;

    /// A class-specific (or any other) descriptor: bDescriptorType and the bytes after
    /// bDescriptorType. bLength is 2 + `payload.len()`.
    fn class_descriptor(&mut self, descriptor_type: u8, payload: &[u8]) -> Result<(), Self::Error>;
}

/// Error of [`write_function_to`]: a broken rule of the configuration, or the first error of
/// the sink.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EmitError<E> {
    /// The configuration breaks a rule; the sink got nothing.
    #[error("{0}")]
    Device(DeviceError),
    /// The sink gave this error; the descriptors after it were not given.
    #[error("descriptor sink error")]
    Sink(E),
}

impl<E> From<DeviceError> for EmitError<E> {
    fn from(e: DeviceError) -> Self {
        EmitError::Device(e)
    }
}

/// Gives the descriptors of the function to `sink`, one call for each descriptor, in the order
/// of [`write_function`] (REQ-USB-GEN-013). The bytes that the sink gets are the bytes of
/// [`write_function`]: a sink that writes each descriptor again (bLength, bDescriptorType,
/// then the fields or the payload) makes the same bytes.
///
/// The writer checks the whole configuration first, as [`function_len`] does: on a broken rule
/// it gives [`EmitError::Device`] and the sink gets nothing. An error of the sink stops the
/// writer and comes back as [`EmitError::Sink`].
pub fn write_function_to<K, const J: usize, const S: usize, const E: usize, const B: usize>(
    cfg: &FunctionConfig<'_, J, S, E, B>,
    sink: &mut K,
) -> Result<(), EmitError<K::Error>>
where
    K: DescriptorSink,
{
    cfg.checked_len()?;
    let midi1 = cfg.midi1.as_ref().ok_or(DeviceError::NoMidi1Setting)?;
    let mut em = Emitter { sink };
    let ac = StandardAcInterface::new(cfg.ac_interface).with_string(cfg.ac_string);
    let header = cfg.ac_header()?;
    em.piece(|b| Ok(write_audio_control(&ac, &header, b)?))?;
    midi1.emit(cfg.ms_interface, &mut em)?;
    for (alt, s) in (1..=u8::MAX).zip(cfg.midi2.iter()) {
        let ms2 = s.ms2(cfg.ms_interface, alt, cfg.speed);
        let interface = ms2.interface().map_err(midi2_error(alt))?;
        em.piece(|b| interface.write(b).map_err(midi2_error(alt)))?;
        em.piece(|b| Ms2Header::write(b).map_err(midi2_error(alt)))?;
        for ep in ms2.endpoints.iter() {
            em.piece(|b| ep.write(cfg.speed, b).map_err(midi2_error(alt)))?;
        }
    }
    Ok(())
}

/// The largest piece that the emitter writes at one time: one MIDI OUT Jack (at most 255
/// bytes), one endpoint with its class-specific descriptor (at most 9 + 4 + 16 bytes), or the
/// AudioControl interface with its header (18 bytes).
const PIECE_LEN: usize = 256;

/// Writes each piece of the function into a small buffer and gives its descriptors to the
/// sink.
struct Emitter<'s, S: DescriptorSink> {
    sink: &'s mut S,
}

impl<S: DescriptorSink> Emitter<'_, S> {
    fn piece(
        &mut self,
        write: impl FnOnce(&mut [u8]) -> Result<usize, DeviceError>,
    ) -> Result<(), EmitError<S::Error>> {
        let mut buf = [0u8; PIECE_LEN];
        let n = write(&mut buf)?;
        for d in Descriptors::new(buf.get(..n).unwrap_or(&[])) {
            // The writers of this crate write whole descriptors: a walk error cannot come.
            let d = d.map_err(|_| DeviceError::Overflow)?;
            dispatch(self.sink, &d).map_err(EmitError::Sink)?;
        }
        Ok(())
    }
}

/// Gives one descriptor to the sink by its bDescriptorType.
fn dispatch<S: DescriptorSink>(sink: &mut S, d: &RawDesc<'_>) -> Result<(), S::Error> {
    match (d.dtype(), d.bytes()) {
        (
            INTERFACE,
            &[_, _, interface_number, alternate_setting, num_endpoints, class, subclass, protocol, string_index, ..],
        ) => sink.interface(&InterfaceFields {
            interface_number,
            alternate_setting,
            num_endpoints,
            class,
            subclass,
            protocol,
            string_index,
        }),
        (ENDPOINT, &[_, _, address, attributes, lo, hi, interval, ref extra @ ..]) => sink
            .endpoint(
                &EndpointFields {
                    address,
                    attributes,
                    max_packet: u16::from_le_bytes([lo, hi]),
                    interval,
                },
                extra,
            ),
        (t, b) => sink.class_descriptor(t, b.get(2..).unwrap_or(&[])),
    }
}

/// The sink of [`write_function`]: it writes each descriptor again into the caller buffer.
struct SliceSink<'o> {
    out: &'o mut [u8],
    at: usize,
}

impl SliceSink<'_> {
    fn put(&mut self, parts: &[&[u8]]) -> Result<(), DeviceError> {
        for p in parts {
            let end = self.at + p.len();
            let dst = self
                .out
                .get_mut(self.at..end)
                .ok_or(DeviceError::Overflow)?;
            dst.copy_from_slice(p);
            self.at = end;
        }
        Ok(())
    }
}

impl DescriptorSink for SliceSink<'_> {
    type Error = DeviceError;

    fn interface(&mut self, f: &InterfaceFields) -> Result<(), DeviceError> {
        self.put(&[&[
            INTERFACE_LEN as u8,
            INTERFACE,
            f.interface_number,
            f.alternate_setting,
            f.num_endpoints,
            f.class,
            f.subclass,
            f.protocol,
            f.string_index,
        ]])
    }

    fn endpoint(&mut self, f: &EndpointFields, extra: &[u8]) -> Result<(), DeviceError> {
        let len = u8::try_from(7 + extra.len()).map_err(|_| DeviceError::Overflow)?;
        let [lo, hi] = f.max_packet.to_le_bytes();
        self.put(&[
            &[len, ENDPOINT, f.address, f.attributes, lo, hi, f.interval],
            extra,
        ])
    }

    fn class_descriptor(&mut self, descriptor_type: u8, payload: &[u8]) -> Result<(), DeviceError> {
        let len = u8::try_from(2 + payload.len()).map_err(|_| DeviceError::Overflow)?;
        self.put(&[&[len, descriptor_type], payload])
    }
}

/// One endpoint of one Alternate Setting, as the writer writes it ([`FunctionConfig::endpoints`],
/// REQ-USB-GEN-013).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SettingEndpoint {
    /// bAlternateSetting: 0 for the USB-MIDI 1.0 setting, 1 or more for a USB MIDI 2.0 setting.
    pub alternate_setting: u8,
    /// MidiOut is host to device (an OUT endpoint), MidiIn is device to host.
    pub direction: EndpointDirection,
    /// The endpoint number, 1 to 15.
    pub number: u8,
    /// Bulk or interrupt; always bulk on Alternate Setting 0.
    pub transfer: TransferType,
    /// wMaxPacketSize.
    pub max_packet: u16,
    /// bInterval: 0 for a bulk endpoint.
    pub interval: u8,
}

impl SettingEndpoint {
    /// bEndpointAddress: D7 direction, D3..0 number.
    pub const fn address(&self) -> u8 {
        self.direction.address_bit() | (self.number & 0x0F)
    }
}

/// The length of the Group Terminal Block descriptor set of USB MIDI 2.0 Alternate Setting
/// `alt`: its wTotalLength (REQ-USB-GTB-002). [`DeviceError::NoSuchSetting`] for Alternate
/// Setting 0 or a number with no USB MIDI 2.0 setting.
pub fn gtb_set_len<const J: usize, const S: usize, const E: usize, const B: usize>(
    cfg: &FunctionConfig<'_, J, S, E, B>,
    alt: u8,
) -> Result<usize, DeviceError> {
    let s = cfg
        .midi2_setting(alt)
        .ok_or(DeviceError::NoSuchSetting(alt))?;
    Ok(s.blocks.set_len())
}

/// Writes the Group Terminal Block descriptor set of USB MIDI 2.0 Alternate Setting `alt` into
/// `out`, a buffer separate from the Configuration descriptor set, and gives its length
/// (REQ-USB-GTB-015, choice U-12). The crate keeps one set for each USB MIDI 2.0 setting and
/// selects it by `alt`: the Request module answers the request of section 6 with it.
///
/// [`DeviceError::NoSuchSetting`] for Alternate Setting 0 or a number with no USB MIDI 2.0
/// setting. A broken block rule gives [`DeviceError::Gtb`], and a buffer shorter than the set
/// gives [`DeviceError::Overflow`]; in each case nothing is written.
pub fn gtb_set<const J: usize, const S: usize, const E: usize, const B: usize>(
    cfg: &FunctionConfig<'_, J, S, E, B>,
    alt: u8,
    out: &mut [u8],
) -> Result<usize, DeviceError> {
    let s = cfg
        .midi2_setting(alt)
        .ok_or(DeviceError::NoSuchSetting(alt))?;
    s.blocks.write(&s.endpoints, out).map_err(gtb_error(alt))
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use crate::constants::{GtbType, MidiProtocol, CS_GR_TRM_BLOCK};
    use crate::gtb::GroupTerminalBlock;
    #[cfg(feature = "alt0")]
    use crate::ms1::{Ms1Desc, Ms1Descriptors};
    use crate::walk::{interfaces, Descriptors};
    use proptest::prelude::*;
    use std::vec::Vec;

    /// USB-MIDI v1.0 Appendix B, Tables B-3 to B-14 (the same bytes are Tables B-3 to B-14 of
    /// USB-MIDI v2.0): 92 bytes.
    #[cfg(feature = "alt0")]
    const V1_FUNCTION: [u8; 92] = [
        0x09, 0x04, 0x00, 0x00, 0x00, 0x01, 0x01, 0x00, 0x00, // B-3 standard AC interface
        0x09, 0x24, 0x01, 0x00, 0x01, 0x09, 0x00, 0x01, 0x01, // B-4 AC header
        0x09, 0x04, 0x01, 0x00, 0x02, 0x01, 0x03, 0x00, 0x00, // B-5 MS interface, alt 0
        0x07, 0x24, 0x01, 0x00, 0x01, 0x41, 0x00, // B-6 MS header, bcdMSC 0x0100
        0x06, 0x24, 0x02, 0x01, 0x01, 0x00, // B-7 Embedded MIDI IN Jack 1
        0x06, 0x24, 0x02, 0x02, 0x02, 0x00, // B-8 External MIDI IN Jack 2
        0x09, 0x24, 0x03, 0x01, 0x03, 0x01, 0x02, 0x01, 0x00, // B-9 Embedded MIDI OUT Jack 3
        0x09, 0x24, 0x03, 0x02, 0x04, 0x01, 0x01, 0x01, 0x00, // B-10 External MIDI OUT Jack 4
        0x09, 0x05, 0x01, 0x02, 0x40, 0x00, 0x00, 0x00, 0x00, // B-11 bulk OUT endpoint 1
        0x05, 0x25, 0x01, 0x01, 0x01, // B-12 MS_GENERAL, Jack 1
        0x09, 0x05, 0x81, 0x02, 0x40, 0x00, 0x00, 0x00, 0x00, // B-13 bulk IN endpoint 1
        0x05, 0x25, 0x01, 0x01, 0x03, // B-14 MS_GENERAL, Jack 3
    ];

    /// USB-MIDI v2.0 Appendix B, Tables B-15 to B-20: Alternate Setting 1 (40 bytes).
    #[cfg(feature = "alt0")]
    const V2_ALT1: [u8; 40] = [
        0x09, 0x04, 0x01, 0x01, 0x02, 0x01, 0x03, 0x00, 0x00, // B-15 MS interface, alt 1
        0x07, 0x24, 0x01, 0x00, 0x02, 0x07, 0x00, // B-16 MS header, bcdMSC 0x0200
        0x07, 0x05, 0x01, 0x02, 0x40, 0x00, 0x00, // B-17 bulk OUT endpoint 1
        0x05, 0x25, 0x02, 0x01, 0x01, // B-18 MS_GENERAL_2_0, block 1
        0x07, 0x05, 0x81, 0x03, 0x40, 0x00, 0x01, // B-19 interrupt IN endpoint 1, bInterval 1
        0x05, 0x25, 0x02, 0x01, 0x01, // B-20 MS_GENERAL_2_0, block 1
    ];

    /// The Appendix B example of USB-MIDI v2.0: B-17 bulk OUT, B-19 interrupt IN.
    #[cfg(feature = "alt0")]
    fn v2_example() -> FunctionConfig<'static> {
        let mut cfg = FunctionConfig::<'static, 4, 1, 2, 1>::midi1_only(0, 1, BusSpeed::Full);
        let mut s = Midi2Setting::empty();
        s.endpoints
            .push(EndpointConfig::bulk(
                EndpointDirection::MidiOut,
                1,
                BusSpeed::Full,
            ))
            .unwrap();
        s.endpoints
            .push(EndpointConfig::interrupt(
                EndpointDirection::MidiIn,
                1,
                64,
                1,
            ))
            .unwrap();
        assert_eq!(cfg.add_midi2_setting(s), Ok(1));
        cfg
    }

    fn write_vec<const J: usize, const S: usize, const E: usize, const B: usize>(
        cfg: &FunctionConfig<'_, J, S, E, B>,
    ) -> Vec<u8> {
        let mut buf = [0u8; 2048];
        let n = write_function(cfg, &mut buf).unwrap();
        assert_eq!(function_len(cfg), Ok(n));
        buf[..n].to_vec()
    }

    /// The interface descriptors (bInterfaceNumber, bAlternateSetting, class, subclass).
    fn interface_list(bytes: &[u8]) -> Vec<(u8, u8, u8, u8)> {
        interfaces(bytes)
            .map(|g| {
                let b = g.interface.bytes();
                (b[2], b[3], b[5], b[6])
            })
            .collect()
    }

    /// The body (after the standard interface descriptor) of Alternate Setting 0.
    #[cfg(feature = "alt0")]
    fn alt0_body(bytes: &[u8]) -> Vec<u8> {
        interfaces(bytes)
            .find(|g| g.interface.bytes()[5..7] == [1, 3] && g.interface.bytes()[3] == 0)
            .unwrap()
            .body
            .to_vec()
    }

    /// (type, id, sources) of each Jack, and (direction, jacks) of each endpoint, in order.
    #[allow(clippy::type_complexity)]
    #[cfg(feature = "alt0")]
    fn read_alt0(
        body: &[u8],
    ) -> (
        Vec<(bool, JackType, u8, Vec<u8>)>,
        Vec<(EndpointDirection, Vec<u8>)>,
    ) {
        let mut jacks = Vec::new();
        let mut eps = Vec::new();
        for item in Ms1Descriptors::new(body) {
            match item.unwrap() {
                Ms1Desc::InJack(j) => jacks.push((false, j.jack_type, j.id, Vec::new())),
                Ms1Desc::OutJack(j) => jacks.push((
                    true,
                    j.jack_type,
                    j.id,
                    j.pins.iter().map(|p| p.source_id).collect(),
                )),
                Ms1Desc::Endpoint(e) => eps.push((e.direction, e.jacks.to_vec())),
                _ => {}
            }
        }
        (jacks, eps)
    }

    #[cfg(feature = "alt0")]
    #[test]
    fn appendix_b_v1_function_is_92_bytes() {
        // REQ-USB-VEC1-013: AC interface 0, MS interface 1, n_out = n_in = 1, endpoints OUT 1
        // and IN 1, 64 bytes, no strings: Tables B-3 to B-14 in table order. Also
        // REQ-USB-MS1-026 (the length before the write) and REQ-USB-GEN-001 (Alternate Setting
        // 0 as USB-MIDI 1.0).
        let cfg = FunctionConfig::<'static, 4, 1, 2, 1>::midi1_only(0, 1, BusSpeed::Full);
        assert_eq!(function_len(&cfg), Ok(92));
        let mut buf = [0xAAu8; 100];
        assert_eq!(write_function(&cfg, &mut buf), Ok(92));
        assert_eq!(buf[..92], V1_FUNCTION);
        assert!(buf[92..].iter().all(|&b| b == 0xAA));
        // REQ-USB-MS1-027: one cable in each direction by default.
        for speed in [BusSpeed::Full, BusSpeed::High] {
            let c = CableCounts::default_for(speed);
            assert_eq!((c.n_out, c.n_in), (1, 1));
        }
    }

    #[cfg(feature = "alt0")]
    #[test]
    fn appendix_b_v2_function_is_132_bytes() {
        // REQ-USB-VEC2-014: Tables B-3 to B-20 in table order, 132 bytes; with the 9-byte
        // Configuration descriptor of the stack the total is 141 = 0x008D.
        let cfg = v2_example();
        let len = function_len(&cfg).unwrap();
        assert_eq!(len, 132);
        assert_eq!(len + 9, 0x008D);
        let bytes = write_vec(&cfg);
        assert_eq!(bytes[..92], V1_FUNCTION);
        assert_eq!(bytes[92..], V2_ALT1);
        // Read back: one AudioControl interface and one MIDI Streaming interface with two
        // Alternate Settings; every descriptor walks by bLength.
        assert!(Descriptors::new(&bytes).all(|d| d.is_ok()));
        assert_eq!(
            interface_list(&bytes),
            [(0, 0, 1, 1), (1, 0, 1, 3), (1, 1, 1, 3)]
        );
        // REQ-USB-GTB-015: no Group Terminal Block descriptor (0x26) in the configuration.
        assert!(Descriptors::new(&bytes)
            .flatten()
            .all(|d| d.dtype() != 0x26));
    }

    #[cfg(feature = "alt0")]
    #[test]
    fn default_config_has_one_midi2_setting_on_bulk_endpoints() {
        // REQ-USB-GEN-004, REQ-USB-GEN-005 (default 1), choices U-06 and U-09.
        let cfg = FunctionConfig::<'static, 4, 1, 2, 1>::new(0, 1, BusSpeed::Full).unwrap();
        assert_eq!(cfg.alternate_settings(), 2);
        assert!(!cfg.midi1.as_ref().unwrap().is_empty());
        let bytes = write_vec(&cfg);
        assert_eq!(bytes.len(), 132);
        assert_eq!(bytes[..92], V1_FUNCTION);
        // Same as Tables B-15 to B-20, but the IN endpoint is bulk with bInterval 0.
        let mut alt1 = V2_ALT1;
        alt1[31] = 0x02;
        alt1[34] = 0x00;
        assert_eq!(bytes[92..], alt1);
        let mut set = [0u8; 18];
        assert_eq!(gtb_set(&cfg, 1, &mut set), Ok(18));
        assert_eq!(set[..5], [0x05, 0x26, 0x01, 0x12, 0x00]);
        assert_eq!(set[5..], GroupTerminalBlock::DEFAULT.to_bytes());
    }

    #[cfg(feature = "alt0")]
    #[test]
    fn high_speed_default_uses_512_byte_packets() {
        // Choices U-06 and U-25: 512 at high speed, on both versions.
        let cfg = FunctionConfig::<'static, 4, 1, 2, 1>::new(0, 1, BusSpeed::High).unwrap();
        let bytes = write_vec(&cfg);
        let eps: Vec<u16> = Descriptors::new(&bytes)
            .map(Result::unwrap)
            .filter(|d| d.dtype() == 0x05)
            .map(|d| d.u16_le(4).unwrap())
            .collect();
        assert_eq!(eps, [512, 512, 512, 512]);
        // 64 is not permitted for bulk at high speed.
        let mut cfg2 = cfg.clone();
        cfg2.midi1 = Some(Midi1Setting::cables(CableCounts::default_for(
            BusSpeed::Full,
        )));
        assert_eq!(function_len(&cfg2), Err(DeviceError::Midi1PacketSize(64)));
    }

    #[test]
    fn no_midi1_setting_is_an_error_and_writes_nothing() {
        // REQ-USB-GEN-001: no USB MIDI 1.0 configuration for Alternate Setting 0.
        let mut cfg = FunctionConfig::<'static, 4, 1, 2, 1>::new(0, 1, BusSpeed::Full).unwrap();
        cfg.midi1 = None;
        let mut buf = [0xAAu8; 256];
        assert_eq!(
            write_function(&cfg, &mut buf),
            Err(DeviceError::NoMidi1Setting)
        );
        assert_eq!(function_len(&cfg), Err(DeviceError::NoMidi1Setting));
        assert!(buf.iter().all(|&b| b == 0xAA));
    }

    #[cfg(feature = "alt0")]
    #[test]
    fn midi1_only_config_writes_only_alternate_setting_0() {
        // Choice U-02 with REQ-USB-GEN-004: no USB MIDI 2.0 setting gives a plain USB-MIDI 1.0
        // function; Alternate Setting 0 is USB-MIDI 1.0 (REQ-USB-GEN-001, bcdMSC 0x0100).
        let cfg = FunctionConfig::<'static, 4, 0, 2, 1>::new(2, 3, BusSpeed::Full).unwrap();
        let bytes = write_vec(&cfg);
        assert_eq!(interface_list(&bytes), [(2, 0, 1, 1), (3, 0, 1, 3)]);
        let body = alt0_body(&bytes);
        assert_eq!(body[..5], [0x07, 0x24, 0x01, 0x00, 0x01]);
        assert_eq!(
            gtb_set(&cfg, 1, &mut [0u8; 32]),
            Err(DeviceError::NoSuchSetting(1))
        );
    }

    #[cfg(feature = "alt0")]
    #[test]
    fn midi2_settings_are_numbered_in_configuration_order() {
        // REQ-USB-GEN-004, REQ-USB-GEN-005: Alternate Settings 1, 2, 3 in order, each a USB
        // MIDI 2.0 setting (bcdMSC 0x0200); a fourth is above the capacity S = 3.
        let mut cfg = FunctionConfig::<'static, 4, 3, 2, 1>::midi1_only(0, 1, BusSpeed::Full);
        for (i, number) in [1u8, 2, 3].into_iter().enumerate() {
            let mut s = Midi2Setting::new(BusSpeed::Full).unwrap();
            s.string_index = 10 + number;
            s.endpoints.as_mut_slice()[0].number = number;
            assert_eq!(cfg.add_midi2_setting(s), Ok(i as u8 + 1));
        }
        assert_eq!(
            cfg.add_midi2_setting(Midi2Setting::new(BusSpeed::Full).unwrap()),
            Err(DeviceError::TooManySettings)
        );
        let bytes = write_vec(&cfg);
        let groups: Vec<_> = interfaces(&bytes).collect();
        assert_eq!(groups.len(), 5);
        for (alt, g) in groups[2..].iter().enumerate() {
            let alt = alt as u8 + 1;
            let b = g.interface.bytes();
            assert_eq!(b[3], alt);
            assert_eq!(b[8], 10 + alt);
            // The header is the USB MIDI 2.0 header.
            assert_eq!(g.body[..7], [0x07, 0x24, 0x01, 0x00, 0x02, 0x07, 0x00]);
            // The OUT endpoint has the number of its setting.
            assert_eq!(g.body[9], alt);
        }
    }

    #[test]
    fn all_alternate_settings_share_the_caller_interface_number() {
        // REQ-USB-GEN-007: one MIDI Streaming interface with the bInterfaceNumber of the
        // caller in every Alternate Setting.
        let mut cfg = FunctionConfig::<'static, 4, 2, 2, 1>::new(4, 7, BusSpeed::Full).unwrap();
        cfg.add_midi2_setting(Midi2Setting::new(BusSpeed::Full).unwrap())
            .unwrap();
        let bytes = write_vec(&cfg);
        assert_eq!(
            interface_list(&bytes),
            [(4, 0, 1, 1), (7, 0, 1, 3), (7, 1, 1, 3), (7, 2, 1, 3)]
        );
        // The same number for both interfaces is refused.
        cfg.ms_interface = 4;
        assert_eq!(function_len(&cfg), Err(DeviceError::SameInterfaceNumber(4)));
    }

    #[cfg(feature = "alt0")]
    #[test]
    fn one_audio_control_interface_before_the_streaming_interface() {
        // REQ-USB-AC-001: one AudioControl interface for the function, before the MIDI
        // Streaming interface, and not one for each Alternate Setting; its header names the
        // MIDI Streaming interface.
        for n2 in 0..=3usize {
            let mut cfg = FunctionConfig::<'static, 4, 3, 2, 1>::midi1_only(0, 1, BusSpeed::Full);
            for _ in 0..n2 {
                cfg.add_midi2_setting(Midi2Setting::new(BusSpeed::Full).unwrap())
                    .unwrap();
            }
            let bytes = write_vec(&cfg);
            let list = interface_list(&bytes);
            assert_eq!(list.len(), 2 + n2);
            assert_eq!(list.iter().filter(|i| (i.2, i.3) == (1, 1)).count(), 1);
            assert_eq!(list[0], (0, 0, 1, 1));
            assert!(list[1..].iter().all(|i| (i.2, i.3) == (1, 3)));
            // The class-specific AC header directly after it: baInterfaceNr = 1.
            assert_eq!(bytes[9..18], V1_FUNCTION[9..18]);
        }
    }

    #[test]
    fn gtb_set_is_selected_by_alternate_setting_number() {
        // REQ-USB-GTB-015: one Group Terminal Block set for each USB MIDI 2.0 Alternate
        // Setting, selected by the setting number, in a separate buffer; never inline.
        let mut cfg = FunctionConfig::<'static, 4, 2, 2, 2>::new(0, 1, BusSpeed::Full).unwrap();
        let mut s2 = Midi2Setting::new(BusSpeed::Full).unwrap();
        let mut b1 = GroupTerminalBlock::new(1, GtbType::Bidirectional, 0, 2);
        b1.max_input_bandwidth = 0x0010;
        let mut b2 = GroupTerminalBlock::new(2, GtbType::Bidirectional, 2, 1);
        b2.protocol = MidiProtocol::Midi1UpTo64;
        s2.blocks = GtbList::from_slice(&[b1, b2]).unwrap();
        for ep in s2.endpoints.as_mut_slice() {
            *ep = ep.with_blocks(&[1, 2]).unwrap();
        }
        assert_eq!(cfg.add_midi2_setting(s2), Ok(2));

        let mut set1 = [0xAAu8; 64];
        assert_eq!(gtb_set_len(&cfg, 1), Ok(18));
        assert_eq!(gtb_set(&cfg, 1, &mut set1), Ok(18));
        assert_eq!(set1[..5], [0x05, 0x26, 0x01, 0x12, 0x00]);
        assert_eq!(set1[5..18], GroupTerminalBlock::DEFAULT.to_bytes());
        assert!(set1[18..].iter().all(|&b| b == 0xAA));

        let mut set2 = [0u8; 64];
        assert_eq!(gtb_set_len(&cfg, 2), Ok(31));
        assert_eq!(gtb_set(&cfg, 2, &mut set2), Ok(31));
        assert_eq!(set2[..5], [0x05, 0x26, 0x01, 0x1F, 0x00]);
        assert_eq!(set2[5..18], b1.to_bytes());
        assert_eq!(set2[18..31], b2.to_bytes());

        // Alternate Setting 0 and an unknown setting have no set.
        assert_eq!(
            gtb_set(&cfg, 0, &mut set2),
            Err(DeviceError::NoSuchSetting(0))
        );
        assert_eq!(
            gtb_set(&cfg, 3, &mut set2),
            Err(DeviceError::NoSuchSetting(3))
        );
        assert_eq!(gtb_set_len(&cfg, 0), Err(DeviceError::NoSuchSetting(0)));

        // A buffer too small: overflow and nothing written.
        let mut small = [0xAAu8; 30];
        assert_eq!(gtb_set(&cfg, 2, &mut small), Err(DeviceError::Overflow));
        assert!(small.iter().all(|&b| b == 0xAA));

        // The configuration descriptors have no Group Terminal Block descriptor (U-12).
        let bytes = write_vec(&cfg);
        assert!(Descriptors::new(&bytes)
            .map(Result::unwrap)
            .all(|d| d.dtype() != CS_GR_TRM_BLOCK));
    }

    #[test]
    fn broken_block_rules_are_refused_with_the_setting_number() {
        // REQ-USB-GTB-015 with choice U-07: the writer refuses a block that no endpoint names
        // the right way; the function writer and gtb_set give the same error.
        let mut cfg = FunctionConfig::<'static, 4, 1, 2, 1>::new(0, 1, BusSpeed::Full).unwrap();
        cfg.midi2.as_mut_slice()[0].blocks =
            GtbList::from_slice(&[GroupTerminalBlock::new(3, GtbType::Bidirectional, 0, 1)])
                .unwrap();
        let e = function_len(&cfg).unwrap_err();
        assert!(matches!(e, DeviceError::Gtb { alt: 1, .. }), "{e:?}");
        let mut buf = [0xAAu8; 256];
        assert_eq!(write_function(&cfg, &mut buf), Err(e));
        assert!(buf.iter().all(|&b| b == 0xAA));
        assert!(matches!(
            gtb_set(&cfg, 1, &mut buf),
            Err(DeviceError::Gtb { alt: 1, .. })
        ));
    }

    #[test]
    fn broken_midi2_setting_is_refused_with_its_number() {
        // REQ-USB-GEN-008 through the Ms2 writer: a setting with no endpoint.
        let mut cfg = FunctionConfig::<'static, 4, 2, 2, 1>::new(0, 1, BusSpeed::Full).unwrap();
        cfg.add_midi2_setting(Midi2Setting::empty()).unwrap();
        assert_eq!(
            function_len(&cfg),
            Err(DeviceError::Midi2 {
                alt: 2,
                error: Ms2Error::NoEndpoint
            })
        );
        // E = 1 cannot hold the default endpoints.
        assert!(matches!(
            FunctionConfig::<'static, 4, 1, 1, 1>::new(0, 1, BusSpeed::Full),
            Err(DeviceError::Midi2 {
                error: Ms2Error::TooManyEndpoints,
                ..
            })
        ));
    }

    #[cfg(feature = "alt0")]
    #[test]
    fn length_is_given_before_and_short_buffers_get_nothing() {
        // REQ-USB-MS1-026: the total length before the write, the number of bytes written, and
        // an overflow error with nothing written for every shorter buffer.
        let cfg = v2_example();
        let total = function_len(&cfg).unwrap();
        for len in 0..total {
            let mut buf = std::vec![0xAAu8; len];
            assert_eq!(write_function(&cfg, &mut buf), Err(DeviceError::Overflow));
            assert!(buf.iter().all(|&b| b == 0xAA), "len {len}");
        }
        let mut buf = std::vec![0u8; total];
        assert_eq!(write_function(&cfg, &mut buf), Ok(total));
    }

    #[cfg(feature = "alt0")]
    #[test]
    fn cable_counts_give_the_jack_order_and_ids() {
        // REQ-USB-MS1-027: n_out = 3, n_in = 2. k = 0: EmbIn 1, ExtIn 2, EmbOut 3 (from 2),
        // ExtOut 4 (from 1); k = 1: EmbIn 5, ExtIn 6, EmbOut 7 (from 6), ExtOut 8 (from 5);
        // k = 2: EmbIn 9, ExtOut 10 (from 9). baSourcePin 1.
        let mut cfg = FunctionConfig::<'static, 4, 1, 2, 1>::midi1_only(0, 1, BusSpeed::Full);
        cfg.midi1 = Some(Midi1Setting::cables(CableCounts::new(3, 2, BusSpeed::Full)));
        let bytes = write_vec(&cfg);
        let body = alt0_body(&bytes);
        let (jacks, eps) = read_alt0(&body);
        use JackType::{Embedded as Emb, External as Ext};
        let expect: Vec<(bool, JackType, u8, Vec<u8>)> = std::vec![
            (false, Emb, 1, std::vec![]),
            (false, Ext, 2, std::vec![]),
            (true, Emb, 3, std::vec![2]),
            (true, Ext, 4, std::vec![1]),
            (false, Emb, 5, std::vec![]),
            (false, Ext, 6, std::vec![]),
            (true, Emb, 7, std::vec![6]),
            (true, Ext, 8, std::vec![5]),
            (false, Emb, 9, std::vec![]),
            (true, Ext, 10, std::vec![9]),
        ];
        assert_eq!(jacks, expect);
        // REQ-USB-MS1-028: OUT endpoint first, Embedded MIDI IN Jacks in the order of k; then
        // the IN endpoint with the Embedded MIDI OUT Jacks in the order of k.
        assert_eq!(
            eps,
            [
                (EndpointDirection::MidiOut, std::vec![1, 5, 9]),
                (EndpointDirection::MidiIn, std::vec![3, 7]),
            ]
        );
        // baSourcePin is 1 in every MIDI OUT Jack (offset 7).
        for d in Descriptors::new(&body).map(Result::unwrap) {
            if d.subtype() == Some(0x03) && d.dtype() == 0x24 {
                assert_eq!(d.byte(7), Some(1));
            }
        }
        // wTotalLength of the header (choice U-10) is the body length.
        assert_eq!(
            usize::from(u16::from_le_bytes([body[5], body[6]])),
            body.len()
        );
    }

    #[cfg(feature = "alt0")]
    #[test]
    fn equal_cable_counts_give_ids_4k_plus_1_to_4k_plus_4() {
        // REQ-USB-MS1-027: with n_out = n_in the IDs of cable k are 4k+1 to 4k+4 (TinyUSB).
        let mut cfg = FunctionConfig::<'static, 4, 1, 2, 1>::midi1_only(0, 1, BusSpeed::Full);
        cfg.midi1 = Some(Midi1Setting::cables(CableCounts::new(
            16,
            16,
            BusSpeed::Full,
        )));
        let bytes = write_vec(&cfg);
        let (jacks, eps) = read_alt0(&alt0_body(&bytes));
        assert_eq!(jacks.len(), 64);
        for (i, j) in jacks.iter().enumerate() {
            assert_eq!(usize::from(j.2), i + 1);
        }
        let k_ids = |off: u8| (0..16u8).map(|k| 4 * k + off).collect::<Vec<_>>();
        assert_eq!(eps[0], (EndpointDirection::MidiOut, k_ids(1)));
        assert_eq!(eps[1], (EndpointDirection::MidiIn, k_ids(3)));
    }

    #[cfg(feature = "alt0")]
    #[test]
    fn endpoint_with_zero_cables_is_not_written() {
        // REQ-USB-MS1-028: n_out = 0 gives only the MIDI IN endpoint, n_in = 0 only the MIDI
        // OUT endpoint; bNumEndpoints follows.
        for (n_out, n_in, dir) in [
            (0u8, 2u8, EndpointDirection::MidiIn),
            (2, 0, EndpointDirection::MidiOut),
        ] {
            let mut cfg = FunctionConfig::<'static, 4, 1, 2, 1>::midi1_only(0, 1, BusSpeed::Full);
            cfg.midi1 = Some(Midi1Setting::cables(CableCounts::new(
                n_out,
                n_in,
                BusSpeed::Full,
            )));
            let bytes = write_vec(&cfg);
            let alt0 = interfaces(&bytes).nth(1).unwrap();
            assert_eq!(alt0.interface.bytes()[4], 1);
            let (jacks, eps) = read_alt0(alt0.body);
            assert_eq!(jacks.len(), 4);
            assert_eq!(eps.len(), 1);
            assert_eq!(eps[0].0, dir);
            assert_eq!(eps[0].1.len(), 2);
        }
    }

    #[cfg(feature = "alt0")]
    #[test]
    fn bad_cable_counts_are_refused() {
        // REQ-USB-MS1-027: each count 0 to 16, not both 0 with no USB MIDI 2.0 setting; endpoint
        // numbers 1 to 15.
        let mut cfg = FunctionConfig::<'static, 4, 1, 2, 1>::midi1_only(0, 1, BusSpeed::Full);
        for (o, i) in [(0u8, 0u8), (17, 1), (1, 17)] {
            cfg.midi1 = Some(Midi1Setting::cables(CableCounts::new(o, i, BusSpeed::Full)));
            assert_eq!(
                function_len(&cfg),
                Err(DeviceError::CableCount { n_out: o, n_in: i })
            );
        }
        let mut c = CableCounts::default_for(BusSpeed::Full);
        c.in_endpoint = 16;
        cfg.midi1 = Some(Midi1Setting::cables(c));
        assert_eq!(
            function_len(&cfg),
            Err(DeviceError::Midi1(Ms1Error::EndpointNumber(16)))
        );
        // An unused endpoint number is not checked.
        let mut c = CableCounts::new(1, 0, BusSpeed::Full);
        c.in_endpoint = 0;
        cfg.midi1 = Some(Midi1Setting::cables(c));
        assert!(function_len(&cfg).is_ok());
    }

    #[test]
    fn empty_alternate_setting_0_needs_a_midi2_setting() {
        // REQ-USB-MS1-027: both counts 0 only with a USB MIDI 2.0 setting. Alternate Setting 0
        // is then the standard interface descriptor with bNumEndpoints 0 and the MS Interface
        // Header alone (bcdMSC 0x0100, wTotalLength 7), so Linux and Windows still find the
        // USB MIDI 2.0 setting on Alternate Setting 1 (OPT-050 option 3, form B).
        let empty = Midi1Setting::cables(CableCounts::new(0, 0, BusSpeed::Full));
        let mut cfg = FunctionConfig::<'static, 4, 1, 2, 1>::new(0, 1, BusSpeed::Full).unwrap();
        cfg.midi1 = Some(empty.clone());
        let bytes = write_vec(&cfg);
        assert_eq!(bytes.len(), function_len(&cfg).unwrap());
        let alt0 = interfaces(&bytes).nth(1).unwrap();
        assert_eq!(alt0.interface.bytes()[3..5], [0, 0]);
        assert_eq!(alt0.body, [0x07, 0x24, 0x01, 0x00, 0x01, 0x07, 0x00]);
        let alt1 = interfaces(&bytes).nth(2).unwrap();
        assert_eq!(alt1.interface.bytes()[3], 1);
        assert_eq!(alt1.body[3..5], [0x00, 0x02]);

        let mut only = FunctionConfig::<'static, 4, 1, 2, 1>::midi1_only(0, 1, BusSpeed::Full);
        only.midi1 = Some(empty);
        assert_eq!(
            function_len(&only),
            Err(DeviceError::CableCount { n_out: 0, n_in: 0 })
        );
    }

    #[test]
    fn without_alt0_the_default_alt0_is_empty_and_ports_are_refused() {
        // REQ-USB-GEN-014: without the feature `alt0`, the default Alternate Setting 0 has no
        // Jack and no endpoint, and a setting with a port is refused; with it, nothing changes.
        let cfg = FunctionConfig::<'static, 4, 1, 2, 1>::new(0, 1, BusSpeed::Full).unwrap();
        let bytes = write_vec(&cfg);
        let alt0 = interfaces(&bytes).nth(1).unwrap();
        let mut ported = cfg.clone();
        ported.midi1 = Some(Midi1Setting::cables(CableCounts::default_for(
            BusSpeed::Full,
        )));
        if cfg!(feature = "alt0") {
            assert_eq!(alt0.interface.bytes()[4], 2);
            assert!(function_len(&ported).is_ok());
        } else {
            assert_eq!(alt0.interface.bytes()[4], 0);
            assert_eq!(alt0.body, [0x07, 0x24, 0x01, 0x00, 0x01, 0x07, 0x00]);
            assert_eq!(function_len(&ported), Err(DeviceError::Alt0Disabled));
        }
    }

    #[cfg(feature = "alt0")]
    #[test]
    fn full_jack_list_form_is_written_and_checked_by_ms1() {
        // Choice U-26: the full Jack list; the Ms1 module checks it (REQ-USB-TOPO-*). Here the
        // Appendix B topology given Jack by Jack gives the same 92 bytes.
        let p3 = [Pin::new(2, 1)];
        let p4 = [Pin::new(1, 1)];
        let mut t = Ms1Topology::<'_, 4, 2>::new();
        t.add_jack(Jack::In(MidiInJack::new(JackType::Embedded, 1)))
            .unwrap();
        t.add_jack(Jack::In(MidiInJack::new(JackType::External, 2)))
            .unwrap();
        t.add_jack(Jack::Out(MidiOutJack::new(JackType::Embedded, 3, &p3)))
            .unwrap();
        t.add_jack(Jack::Out(MidiOutJack::new(JackType::External, 4, &p4)))
            .unwrap();
        t.add_endpoint(Ms1Endpoint::new(EndpointDirection::MidiOut, 1, 64, &[1]))
            .unwrap();
        t.add_endpoint(Ms1Endpoint::new(EndpointDirection::MidiIn, 1, 64, &[3]))
            .unwrap();
        let mut cfg = FunctionConfig::<'_, 4, 1, 2, 1>::midi1_only(0, 1, BusSpeed::Full);
        cfg.midi1 = Some(Midi1Setting::jacks(t.clone()));
        assert_eq!(write_vec(&cfg), V1_FUNCTION);

        // A topology error of Ms1 comes back as Midi1.
        let mut bad = Ms1Topology::<'_, 4, 2>::new();
        bad.add_jack(Jack::In(MidiInJack::new(JackType::Embedded, 1)))
            .unwrap();
        cfg.midi1 = Some(Midi1Setting::jacks(bad));
        assert!(matches!(function_len(&cfg), Err(DeviceError::Midi1(_))));
    }

    #[test]
    fn string_indexes_default_to_zero_and_can_be_set() {
        // Choice U-27: iInterface 0 by default; the caller can give an index.
        let mut cfg = FunctionConfig::<'static, 4, 1, 2, 1>::new(0, 1, BusSpeed::Full).unwrap();
        let bytes = write_vec(&cfg);
        assert!(interfaces(&bytes).all(|g| g.interface.bytes()[8] == 0));
        cfg.ac_string = 5;
        cfg.midi1.as_mut().unwrap().string_index = 6;
        cfg.midi2.as_mut_slice()[0].string_index = 7;
        let bytes = write_vec(&cfg);
        let s: Vec<u8> = interfaces(&bytes).map(|g| g.interface.bytes()[8]).collect();
        assert_eq!(s, [5, 6, 7]);
    }

    /// A sink that writes each descriptor again and keeps a log of the calls.
    #[derive(Default)]
    #[cfg(feature = "alt0")]
    struct Rebuild {
        bytes: Vec<u8>,
        /// (kind, bDescriptorType or 0, extra length): 'i' interface, 'e' endpoint, 'c' class.
        calls: Vec<(char, u8, usize)>,
        /// Stop with an error at this call number.
        fail_at: Option<usize>,
    }

    #[cfg(feature = "alt0")]
    impl Rebuild {
        fn call(&mut self, c: (char, u8, usize)) -> Result<(), &'static str> {
            if self.fail_at == Some(self.calls.len()) {
                return Err("full");
            }
            self.calls.push(c);
            Ok(())
        }
    }

    #[cfg(feature = "alt0")]
    impl DescriptorSink for Rebuild {
        type Error = &'static str;

        fn interface(&mut self, f: &InterfaceFields) -> Result<(), Self::Error> {
            self.call(('i', INTERFACE, 0))?;
            self.bytes.extend_from_slice(&[
                9,
                INTERFACE,
                f.interface_number,
                f.alternate_setting,
                f.num_endpoints,
                f.class,
                f.subclass,
                f.protocol,
                f.string_index,
            ]);
            Ok(())
        }

        fn endpoint(&mut self, f: &EndpointFields, extra: &[u8]) -> Result<(), Self::Error> {
            self.call(('e', ENDPOINT, extra.len()))?;
            let [lo, hi] = f.max_packet.to_le_bytes();
            self.bytes.extend_from_slice(&[
                7 + extra.len() as u8,
                ENDPOINT,
                f.address,
                f.attributes,
                lo,
                hi,
                f.interval,
            ]);
            self.bytes.extend_from_slice(extra);
            Ok(())
        }

        fn class_descriptor(&mut self, t: u8, payload: &[u8]) -> Result<(), Self::Error> {
            self.call(('c', t, 0))?;
            self.bytes.push(2 + payload.len() as u8);
            self.bytes.push(t);
            self.bytes.extend_from_slice(payload);
            Ok(())
        }
    }

    #[cfg(feature = "alt0")]
    fn rebuild<const J: usize, const S: usize, const E: usize, const B: usize>(
        cfg: &FunctionConfig<'_, J, S, E, B>,
    ) -> Rebuild {
        let mut sink = Rebuild::default();
        assert_eq!(write_function_to(cfg, &mut sink), Ok(()));
        sink
    }

    // REQ-USB-GEN-013: a sink that writes each descriptor again from the fields, the extra
    // bytes and the class-specific payloads gives exactly the bytes of write_function, for the
    // 92-byte (USB-MIDI v1.0 Appendix B) and the 132-byte (USB-MIDI v2.0 Appendix B)
    // configurations and for 16 cables in each direction.
    #[cfg(feature = "alt0")]
    #[test]
    fn sink_gets_the_bytes_of_the_buffer_writer() {
        let v1 = FunctionConfig::<'static, 4, 1, 2, 1>::midi1_only(0, 1, BusSpeed::Full);
        let sink = rebuild(&v1);
        assert_eq!(sink.bytes, V1_FUNCTION);
        assert_eq!(sink.bytes, write_vec(&v1));
        // Tables B-3 to B-14: interface, AC header, interface, MS header, 4 Jacks, then each
        // endpoint (9-byte form: bRefresh and bSynchAddress are the 2 extra bytes) with its
        // MS_GENERAL descriptor.
        let cs = |t| ('c', t, 0);
        assert_eq!(
            sink.calls,
            [
                ('i', INTERFACE, 0),
                cs(0x24),
                ('i', INTERFACE, 0),
                cs(0x24),
                cs(0x24),
                cs(0x24),
                cs(0x24),
                cs(0x24),
                ('e', ENDPOINT, 2),
                cs(0x25),
                ('e', ENDPOINT, 2),
                cs(0x25),
            ]
        );

        let v2 = v2_example();
        let sink = rebuild(&v2);
        assert_eq!(sink.bytes[..92], V1_FUNCTION);
        assert_eq!(sink.bytes[92..], V2_ALT1);
        assert_eq!(sink.bytes, write_vec(&v2));
        // Tables B-15 to B-20: the 7-byte endpoints have no extra bytes.
        assert_eq!(
            sink.calls[12..],
            [
                ('i', INTERFACE, 0),
                cs(0x24),
                ('e', ENDPOINT, 0),
                cs(0x25),
                ('e', ENDPOINT, 0),
                cs(0x25),
            ]
        );

        let mut c16 = FunctionConfig::<'static, 4, 1, 2, 1>::new(0, 1, BusSpeed::High).unwrap();
        c16.midi1 = Some(Midi1Setting::cables(CableCounts::new(
            16,
            16,
            BusSpeed::High,
        )));
        let sink = rebuild(&c16);
        assert_eq!(sink.bytes, write_vec(&c16));
        assert_eq!(sink.calls.iter().filter(|c| c.0 == 'e').count(), 4);
        // 64 Jacks: the MIDI OUT Jacks have 9 bytes.
        assert_eq!(sink.bytes.len(), function_len(&c16).unwrap());

        // The full Jack list form gives the same bytes as the cable-count form.
        let p3 = [Pin::new(2, 1)];
        let p4 = [Pin::new(1, 1)];
        let mut t = Ms1Topology::<'_, 4, 2>::new();
        t.add_jack(Jack::In(MidiInJack::new(JackType::Embedded, 1)))
            .unwrap();
        t.add_jack(Jack::In(MidiInJack::new(JackType::External, 2)))
            .unwrap();
        t.add_jack(Jack::Out(MidiOutJack::new(JackType::Embedded, 3, &p3)))
            .unwrap();
        t.add_jack(Jack::Out(MidiOutJack::new(JackType::External, 4, &p4)))
            .unwrap();
        t.add_endpoint(Ms1Endpoint::new(EndpointDirection::MidiOut, 1, 64, &[1]))
            .unwrap();
        t.add_endpoint(Ms1Endpoint::new(EndpointDirection::MidiIn, 1, 64, &[3]))
            .unwrap();
        let mut jl = FunctionConfig::<'_, 4, 1, 2, 1>::midi1_only(0, 1, BusSpeed::Full);
        jl.midi1 = Some(Midi1Setting::jacks(t));
        assert_eq!(rebuild(&jl).bytes, V1_FUNCTION);
    }

    // REQ-USB-GEN-013: a broken rule gives nothing to the sink; an error of the sink stops the
    // writer and comes back as EmitError::Sink.
    #[cfg(feature = "alt0")]
    #[test]
    fn sink_gets_nothing_on_a_broken_rule_and_stops_on_its_error() {
        let mut cfg = v2_example();
        cfg.midi1 = None;
        let mut sink = Rebuild::default();
        assert_eq!(
            write_function_to(&cfg, &mut sink),
            Err(EmitError::Device(DeviceError::NoMidi1Setting))
        );
        assert!(sink.calls.is_empty() && sink.bytes.is_empty());

        let cfg = v2_example();
        let mut sink = Rebuild {
            fail_at: Some(3),
            ..Rebuild::default()
        };
        assert_eq!(
            write_function_to(&cfg, &mut sink),
            Err(EmitError::Sink("full"))
        );
        assert_eq!(sink.calls.len(), 3);
    }

    // REQ-USB-GEN-013: the endpoints of each Alternate Setting, in descriptor order, for the
    // allocation by the USB stack.
    #[cfg(feature = "alt0")]
    #[test]
    fn endpoints_of_each_alternate_setting_are_listed() {
        let cfg = v2_example();
        let eps: Vec<SettingEndpoint> = cfg.endpoints().collect();
        let ep = |alternate_setting, direction, transfer, interval| SettingEndpoint {
            alternate_setting,
            direction,
            number: 1,
            transfer,
            max_packet: 64,
            interval,
        };
        use EndpointDirection::{MidiIn as In, MidiOut as Out};
        use TransferType::{Bulk, Interrupt};
        assert_eq!(
            eps,
            [
                ep(0, Out, Bulk, 0),
                ep(0, In, Bulk, 0),
                ep(1, Out, Bulk, 0),
                ep(1, In, Interrupt, 1),
            ]
        );
        assert_eq!(
            eps.iter().map(SettingEndpoint::address).collect::<Vec<_>>(),
            [0x01, 0x81, 0x01, 0x81]
        );
        // The same as the endpoint descriptors that the writer writes.
        let bytes = write_vec(&cfg);
        let written: Vec<(u8, u8, u16, u8)> = Descriptors::new(&bytes)
            .map(Result::unwrap)
            .filter(|d| d.dtype() == ENDPOINT)
            .map(|d| {
                let b = d.bytes();
                (b[2], b[3], d.u16_le(4).unwrap(), b[6])
            })
            .collect();
        let listed: Vec<(u8, u8, u16, u8)> = eps
            .iter()
            .map(|e| {
                (
                    e.address(),
                    e.transfer.attributes(),
                    e.max_packet,
                    e.interval,
                )
            })
            .collect();
        assert_eq!(written, listed);
        // An endpoint with a cable count of 0 is not listed; the full Jack list form lists
        // its endpoints.
        let mut c = FunctionConfig::<'static, 4, 0, 2, 1>::midi1_only(0, 1, BusSpeed::Full);
        let mut counts = CableCounts::new(0, 2, BusSpeed::Full);
        counts.in_endpoint = 3;
        c.midi1 = Some(Midi1Setting::cables(counts));
        let eps: Vec<SettingEndpoint> = c.endpoints().collect();
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].address(), 0x83);
    }

    // REQ-USB-MS2-017: the function writer refuses a USB MIDI 2.0 setting with two endpoints
    // of the same bEndpointAddress, with the setting number, and writes nothing.
    #[cfg(feature = "alt0")]
    #[test]
    fn midi2_setting_with_two_endpoints_of_one_address_is_refused() {
        let mut cfg = FunctionConfig::<'static, 4, 1, 2, 2>::midi1_only(0, 1, BusSpeed::Full);
        let mut s = Midi2Setting::empty();
        for id in [1u8, 2] {
            let ep = EndpointConfig::bulk(EndpointDirection::MidiOut, 1, BusSpeed::Full)
                .with_blocks(&[id])
                .unwrap();
            s.endpoints.push(ep).unwrap();
        }
        s.blocks = GtbList::from_slice(&[
            GroupTerminalBlock::new(1, GtbType::InputOnly, 0, 1),
            GroupTerminalBlock::new(2, GtbType::InputOnly, 1, 1),
        ])
        .unwrap();
        cfg.add_midi2_setting(s).unwrap();
        let e = DeviceError::Midi2 {
            alt: 1,
            error: Ms2Error::DuplicateEndpoint(0x01),
        };
        assert_eq!(function_len(&cfg), Err(e));
        let mut buf = [0xAAu8; 256];
        assert_eq!(write_function(&cfg, &mut buf), Err(e));
        assert!(buf.iter().all(|&b| b == 0xAA));
    }

    #[cfg(feature = "alt0")]
    #[test]
    fn prop_cable_counts_write_a_valid_topology() {
        // REQ-USB-MS1-026, REQ-USB-MS1-027, REQ-USB-MS1-028: every valid pair of counts gives
        // the length of function_len, a topology that passes the Ms1 checks when read back,
        // and the Embedded Jacks in the order of k.
        proptest!(|(n_out in 0u8..=16, n_in in 0u8..=16)| {
            prop_assume!(n_out | n_in != 0);
            let mut cfg = FunctionConfig::<'static, 4, 0, 2, 1>::midi1_only(0, 1, BusSpeed::Full);
            cfg.midi1 = Some(Midi1Setting::cables(CableCounts::new(n_out, n_in, BusSpeed::Full)));
            let mut buf = [0u8; 2048];
            let n = write_function(&cfg, &mut buf).unwrap();
            prop_assert_eq!(function_len(&cfg), Ok(n));
            let body = alt0_body(&buf[..n]);
            let mut t = Ms1Topology::<'_, 64, 2>::new();
            for item in Ms1Descriptors::new(&body) {
                match item.unwrap() {
                    Ms1Desc::InJack(j) => t.add_jack(Jack::In(j)).unwrap(),
                    Ms1Desc::OutJack(j) => t.add_jack(Jack::Out(j)).unwrap(),
                    Ms1Desc::Endpoint(e) => t.add_endpoint(e).unwrap(),
                    _ => {}
                }
            }
            prop_assert!(t.check().is_ok());
            prop_assert_eq!(t.len(), body.len());
            prop_assert_eq!(usize::from(t.num_endpoints()), usize::from(n_out > 0) + usize::from(n_in > 0));
            let eps: Vec<_> = t.endpoints().copied().collect();
            if n_out > 0 {
                prop_assert_eq!(eps[0].direction, EndpointDirection::MidiOut);
                prop_assert_eq!(eps[0].jacks.len(), usize::from(n_out));
                prop_assert!(eps[0].jacks.windows(2).all(|w| w[0] < w[1]));
            }
            if n_in > 0 {
                let e = eps.last().unwrap();
                prop_assert_eq!(e.direction, EndpointDirection::MidiIn);
                prop_assert_eq!(e.jacks.len(), usize::from(n_in));
                prop_assert!(e.jacks.windows(2).all(|w| w[0] < w[1]));
            }
        });
    }

    #[test]
    fn prop_writer_never_panics_and_writes_nothing_on_error() {
        // REQ-USB-MS1-026: any counts, endpoint numbers, packet sizes and buffer lengths give
        // the bytes or an error with the buffer unchanged.
        proptest!(|(n_out in 0u8..=20, n_in in 0u8..=20, oe in 0u8..=16, ie in 0u8..=16,
                    mp in 0u16..=600, high in any::<bool>(), len in 0usize..=600)| {
            let speed = if high { BusSpeed::High } else { BusSpeed::Full };
            let mut cfg = FunctionConfig::<'static, 4, 1, 2, 1>::new(0, 1, speed).unwrap();
            let mut c = CableCounts::new(n_out, n_in, speed);
            c.out_endpoint = oe;
            c.in_endpoint = ie;
            c.max_packet = mp;
            cfg.midi1 = Some(Midi1Setting::cables(c));
            let mut buf = std::vec![0xAAu8; len];
            match write_function(&cfg, &mut buf) {
                Ok(n) => prop_assert_eq!(function_len(&cfg), Ok(n)),
                Err(_) => prop_assert!(buf.iter().all(|&b| b == 0xAA)),
            }
        });
    }
}
