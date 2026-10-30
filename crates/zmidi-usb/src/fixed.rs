//! A USB MIDI function whose descriptors are made when the firmware is built (OPT-016,
//! DEC-USB-002).
//!
//! - [`write_rust`] runs in a build script (or any host program). It takes a
//!   [`FunctionConfig`], runs every check of [`write_function`], and writes the Rust source of
//!   one [`FixedFunction`] constant: the descriptor bytes of [`write_function`] (USB-MIDI v2.0
//!   Appendix B order), the endpoints of [`FunctionConfig::endpoints`], the Group mask of the MIDI
//!   OUT endpoint of each Alternate Setting, and the Group Terminal Block descriptor set of each
//!   USB MIDI 2.0 setting ([`gtb_set`]).
//! - The firmware links the constant and [`FixedFunction::replay`], which gives the same
//!   descriptors to a [`DescriptorSink`] with no check and no build step. It patches the values
//!   that the USB stack gives when the device starts: the two interface numbers (also
//!   baInterfaceNr of the class-specific AudioControl header, USB-MIDI v1.0 Table B-3) and the
//!   string indexes (iInterface).
//! - [`FixedFunction::gtb_setup`] answers the Group Terminal Block descriptors request
//!   (USB-MIDI v2.0 6) from the stored sets, with the rules of [`handle_gtb_setup`].
//!
//! [`FunctionConfig`] stays for any topology that the firmware makes when it runs; this module
//! is beside it (DEC-USB-002). No alloc (DEC-002): [`write_rust`] writes into a
//! [`core::fmt::Write`].
//!
//! [`write_function`]: crate::device::write_function
//! [`gtb_set`]: crate::device::gtb_set
//! [`handle_gtb_setup`]: crate::request::handle_gtb_setup

use core::fmt::{self, Write};

use crate::constants::EndpointDirection;
use crate::device::{
    function_len, gtb_set, write_function_to, DescriptorSink, DeviceError, EmitError,
    EndpointFields, FunctionConfig, InterfaceFields, SettingEndpoint,
};
use crate::gtb::GroupMask;
use crate::ms2::TransferType;
use crate::request::{handle_gtb_setup, GtbResponse, RequestError, SetupPacket};

/// The interface numbers that the stored descriptors use; [`FixedFunction::replay`] replaces
/// them with the ones of the USB stack.
const AC_TEMPLATE: u8 = 0;
const MS_TEMPLATE: u8 = 1;
/// bDescriptorType of a standard interface and endpoint descriptor (USB 2.0 Table 9-5).
const INTERFACE: u8 = 0x04;
const ENDPOINT: u8 = 0x05;
/// CS_INTERFACE and the HEADER subtype of the class-specific AudioControl interface descriptor.
const CS_INTERFACE: u8 = 0x24;
const AC_HEADER: u8 = 0x01;
/// bInterfaceSubClass AUDIOCONTROL.
const AUDIOCONTROL: u8 = 0x01;
/// The largest class-specific descriptor: bLength is one byte.
const MAX_DESCRIPTOR: usize = 255;

/// A USB MIDI function made by [`write_rust`] when the firmware is built. The only constructor
/// is the `const fn` [`FixedFunction::new`], which checks the endpoint rules of the adapters; the
/// source of [`write_rust`] calls it in a constant, so a broken rule stops the build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixedFunction {
    /// The bytes of `write_function` for interface numbers 0 (AudioControl) and 1 (MIDI
    /// Streaming) and no strings (all iInterface 0).
    descriptors: &'static [u8],
    /// The endpoints, in the order of `FunctionConfig::endpoints`.
    endpoints: &'static [SettingEndpoint],
    /// The Groups of the MIDI OUT endpoint of each Alternate Setting, index = bAlternateSetting;
    /// all Groups for Alternate Setting 0. The length is the number of Alternate Settings.
    group_masks: &'static [GroupMask],
    /// The Group Terminal Block descriptor set of each USB MIDI 2.0 setting, index =
    /// bAlternateSetting - 1.
    gtb_sets: &'static [&'static [u8]],
}

const fn transfer_eq(a: TransferType, b: TransferType) -> bool {
    match (a, b) {
        (TransferType::Bulk, TransferType::Bulk)
        | (TransferType::Interrupt, TransferType::Interrupt) => true,
        (TransferType::Other(x), TransferType::Other(y)) => x == y,
        _ => false,
    }
}

const fn direction_index(d: EndpointDirection) -> usize {
    match d {
        EndpointDirection::MidiOut => 0,
        EndpointDirection::MidiIn => 1,
    }
}

/// The values that the USB stack gives when the device starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FixedIds<'s> {
    /// bInterfaceNumber of the AudioControl interface.
    pub ac_interface: u8,
    /// bInterfaceNumber of the MIDI Streaming interface.
    pub ms_interface: u8,
    /// iInterface of the AudioControl interface; 0 is no string.
    pub ac_string: u8,
    /// iInterface of each Alternate Setting of the MIDI Streaming interface, index =
    /// bAlternateSetting; a missing entry or 0 is no string.
    pub streaming_strings: &'s [u8],
}

impl FixedFunction {
    /// A function from the tables of [`write_rust`]. Checks the endpoint rules of the adapters:
    /// at most one endpoint in each direction in each Alternate Setting, and one transfer type,
    /// wMaxPacketSize and bInterval for each address. In a constant, an error stops the build.
    pub const fn new(
        descriptors: &'static [u8],
        endpoints: &'static [SettingEndpoint],
        group_masks: &'static [GroupMask],
        gtb_sets: &'static [&'static [u8]],
    ) -> Result<Self, FixedError> {
        let mut i = 0;
        while i < endpoints.len() {
            let a = endpoints[i];
            let mut j = 0;
            while j < i {
                let b = endpoints[j];
                let same_slot = a.alternate_setting == b.alternate_setting
                    && direction_index(a.direction) == direction_index(b.direction);
                let mismatch = a.address() == b.address()
                    && (!transfer_eq(a.transfer, b.transfer)
                        || a.max_packet != b.max_packet
                        || a.interval != b.interval);
                if same_slot || mismatch {
                    return Err(FixedError::Endpoints {
                        address: a.address(),
                    });
                }
                j += 1;
            }
            i += 1;
        }
        Ok(FixedFunction {
            descriptors,
            endpoints,
            group_masks,
            gtb_sets,
        })
    }

    /// The descriptor bytes (interface numbers 0 and 1, no strings).
    pub const fn descriptors(&self) -> &'static [u8] {
        self.descriptors
    }

    /// The endpoints, in the order of `FunctionConfig::endpoints`.
    pub const fn endpoints(&self) -> &'static [SettingEndpoint] {
        self.endpoints
    }

    /// The Groups of the MIDI OUT endpoint of each Alternate Setting.
    pub const fn group_masks(&self) -> &'static [GroupMask] {
        self.group_masks
    }

    /// The Group Terminal Block descriptor set of each USB MIDI 2.0 setting.
    pub const fn gtb_sets(&self) -> &'static [&'static [u8]] {
        self.gtb_sets
    }

    /// The number of Alternate Settings: 1 + the USB MIDI 2.0 ones.
    pub const fn alternate_settings(&self) -> usize {
        self.group_masks.len()
    }

    /// The number of descriptor bytes of the function (wTotalLength part, as `function_len`).
    pub const fn len(&self) -> usize {
        self.descriptors.len()
    }

    /// True when the function has no descriptor bytes.
    pub const fn is_empty(&self) -> bool {
        self.descriptors.is_empty()
    }

    /// The Groups of the MIDI OUT endpoint of Alternate Setting `alt`; no Group for a number
    /// with no setting.
    pub fn group_mask(&self, alt: u8) -> GroupMask {
        self.group_masks
            .get(usize::from(alt))
            .copied()
            .unwrap_or(GroupMask::EMPTY)
    }

    /// Gives the descriptors to `sink`, one call for each descriptor, as `write_function_to`
    /// does for the configuration that [`write_rust`] read, with the interface numbers and
    /// string indexes of `ids`. A descriptor with a bLength under 2 or past the end stops the
    /// walk (the bytes of [`write_rust`] have none). Never panics.
    pub fn replay<K: DescriptorSink>(
        &self,
        ids: &FixedIds<'_>,
        sink: &mut K,
    ) -> Result<(), K::Error> {
        let mut rest = self.descriptors;
        // The interface of the current descriptors: (bInterfaceNumber, subclass).
        let mut in_ac = false;
        while let [len, dtype, ..] = *rest {
            let len = usize::from(len);
            let Some((d, tail)) = rest.split_at_checked(len).filter(|_| len >= 2) else {
                break;
            };
            rest = tail;
            match dtype {
                INTERFACE if d.len() >= 9 => {
                    in_ac = d[6] == AUDIOCONTROL && d[2] == AC_TEMPLATE;
                    let alternate_setting = d[3];
                    let (interface_number, string_index) = if in_ac {
                        (ids.ac_interface, ids.ac_string)
                    } else {
                        let s = ids.streaming_strings.get(usize::from(alternate_setting));
                        (ids.ms_interface, s.copied().unwrap_or(0))
                    };
                    sink.interface(&InterfaceFields {
                        interface_number,
                        alternate_setting,
                        num_endpoints: d[4],
                        class: d[5],
                        subclass: d[6],
                        protocol: d[7],
                        string_index,
                    })?;
                }
                ENDPOINT if d.len() >= 7 => {
                    let fields = EndpointFields {
                        address: d[2],
                        attributes: d[3],
                        max_packet: u16::from_le_bytes([d[4], d[5]]),
                        interval: d[6],
                    };
                    sink.endpoint(&fields, &d[7..])?;
                }
                CS_INTERFACE if in_ac && d.get(2) == Some(&AC_HEADER) && d.len() >= 8 => {
                    // baInterfaceNr from byte 8 on: the MIDI Streaming interface number.
                    let mut payload = [0u8; MAX_DESCRIPTOR];
                    let body = &d[2..];
                    payload[..body.len()].copy_from_slice(body);
                    for b in payload[6..body.len()].iter_mut() {
                        if *b == MS_TEMPLATE {
                            *b = ids.ms_interface;
                        }
                    }
                    sink.class_descriptor(dtype, &payload[..body.len()])?;
                }
                _ => sink.class_descriptor(dtype, &d[2..])?,
            }
        }
        Ok(())
    }

    /// Answers the Group Terminal Block descriptors request (USB-MIDI v2.0 6) from the stored
    /// sets, with the rules of `handle_gtb_setup`: `NotHandled` for another request, `Stall`
    /// for another interface than `ms_interface` or an Alternate Setting with no USB MIDI 2.0
    /// setting, else the first wLength bytes of the set (REQ-USB-GTB-017 to REQ-USB-GTB-019).
    pub fn gtb_setup(
        &self,
        setup: &SetupPacket,
        ms_interface: u8,
        out: &mut [u8],
    ) -> Result<GtbResponse, RequestError> {
        let Some(alt) = setup.gtb_alternate_setting() else {
            return Ok(GtbResponse::NotHandled);
        };
        if setup.index != u16::from(ms_interface) {
            return Ok(GtbResponse::Stall);
        }
        let set = usize::from(alt)
            .checked_sub(1)
            .and_then(|i| self.gtb_sets.get(i));
        let Some(set) = set else {
            return Ok(GtbResponse::Stall);
        };
        let n = usize::from(setup.length).min(set.len());
        let dst = out.get_mut(..n).ok_or(RequestError::Overflow)?;
        dst.copy_from_slice(&set[..n]);
        Ok(GtbResponse::Answered(n))
    }
}

/// The most interface names of a function: the AudioControl interface and 7 Alternate Settings
/// of the MIDI Streaming interface.
pub const MAX_NAMES: usize = 8;

/// The interface numbers and string indexes that a USB stack adapter gets when the device starts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FunctionIds {
    /// bInterfaceNumber of the AudioControl interface.
    pub ac_interface: u8,
    /// bInterfaceNumber of the MIDI Streaming interface.
    pub ms_interface: u8,
    /// iInterface of the AudioControl interface; 0 is no string.
    pub ac_string: u8,
    /// iInterface of each Alternate Setting, index = bAlternateSetting; 0 is no string.
    pub streaming_strings: [u8; MAX_NAMES],
}

/// The descriptors and the endpoint table of a USB MIDI function, as a USB stack adapter uses
/// them (DEC-USB-002): a [`FunctionConfig`] made when the firmware runs, or a [`Fixed`]
/// function made when it is built. Both adapters (`usb-device`, `embassy-usb`) take it.
pub trait Function {
    /// True when the build checked the function (a [`Fixed`] of [`write_rust`]): the
    /// adapters then skip their run-time checks of the endpoints (DEC-USB-002 addendum of
    /// 2026-10-26, OPT-027).
    const CHECKED: bool = false;
    /// Takes the interface numbers and string indexes and checks the function. An adapter calls
    /// it one time, before any other method.
    fn prepare(&mut self, ids: &FunctionIds) -> Result<(), DeviceError>;
    /// The number of Alternate Settings.
    fn alternate_settings(&self) -> usize;
    /// The endpoints, in descriptor order.
    fn endpoints(&self) -> impl Iterator<Item = SettingEndpoint> + '_;
    /// The Groups of the MIDI OUT endpoint of Alternate Setting `alt`: all for setting 0, none
    /// for a setting that does not exist.
    fn group_mask(&self, alt: u8) -> GroupMask;
    /// Gives the descriptors of the function to `sink`.
    fn write<K: DescriptorSink>(&self, sink: &mut K) -> Result<(), EmitError<K::Error>>;
    /// Answers the Group Terminal Block descriptors request (USB-MIDI v2.0 6).
    fn gtb_setup(&self, setup: &SetupPacket, out: &mut [u8]) -> Result<GtbResponse, RequestError>;
}

impl<const J: usize, const S: usize, const E: usize, const B: usize> Function
    for FunctionConfig<'_, J, S, E, B>
{
    fn prepare(&mut self, ids: &FunctionIds) -> Result<(), DeviceError> {
        self.ac_interface = ids.ac_interface;
        self.ms_interface = ids.ms_interface;
        self.ac_string = ids.ac_string;
        if let Some(m) = self.midi1.as_mut() {
            m.string_index = ids.streaming_strings[0];
        }
        for (s, index) in self
            .midi2
            .as_mut_slice()
            .iter_mut()
            .zip(&ids.streaming_strings[1..])
        {
            s.string_index = *index;
        }
        function_len(self)?;
        Ok(())
    }

    fn alternate_settings(&self) -> usize {
        FunctionConfig::alternate_settings(self)
    }

    fn endpoints(&self) -> impl Iterator<Item = SettingEndpoint> + '_ {
        FunctionConfig::endpoints(self)
    }

    fn group_mask(&self, alt: u8) -> GroupMask {
        match (alt, self.midi2_setting(alt)) {
            (0, _) => GroupMask::ALL,
            (_, Some(s)) => s
                .endpoints
                .iter()
                .find(|e| e.direction == EndpointDirection::MidiOut)
                .map_or(GroupMask::EMPTY, |e| {
                    GroupMask::from_blocks(s.blocks.as_slice(), e)
                }),
            // As FixedFunction::group_mask: no Group for a setting that does not exist.
            (_, None) => GroupMask::EMPTY,
        }
    }

    fn write<K: DescriptorSink>(&self, sink: &mut K) -> Result<(), EmitError<K::Error>> {
        write_function_to(self, sink)
    }

    fn gtb_setup(&self, setup: &SetupPacket, out: &mut [u8]) -> Result<GtbResponse, RequestError> {
        handle_gtb_setup(setup, self, out)
    }
}

/// A function made when the firmware is built: a [`FixedFunction`] constant of [`write_rust`]
/// with the ids of the USB stack (DEC-USB-002). The firmware links no descriptor builder and
/// no check.
#[derive(Debug, Clone, Copy)]
pub struct Fixed {
    function: &'static FixedFunction,
    ids: FunctionIds,
}

impl Fixed {
    /// The function of `function`.
    pub const fn new(function: &'static FixedFunction) -> Self {
        Fixed {
            function,
            ids: FunctionIds {
                ac_interface: 0,
                ms_interface: 0,
                ac_string: 0,
                streaming_strings: [0; MAX_NAMES],
            },
        }
    }
}

impl Function for Fixed {
    const CHECKED: bool = true;

    fn prepare(&mut self, ids: &FunctionIds) -> Result<(), DeviceError> {
        self.ids = *ids;
        Ok(())
    }

    fn alternate_settings(&self) -> usize {
        self.function.alternate_settings()
    }

    fn endpoints(&self) -> impl Iterator<Item = SettingEndpoint> + '_ {
        self.function.endpoints.iter().copied()
    }

    fn group_mask(&self, alt: u8) -> GroupMask {
        self.function.group_mask(alt)
    }

    fn write<K: DescriptorSink>(&self, sink: &mut K) -> Result<(), EmitError<K::Error>> {
        let ids = FixedIds {
            ac_interface: self.ids.ac_interface,
            ms_interface: self.ids.ms_interface,
            ac_string: self.ids.ac_string,
            streaming_strings: &self.ids.streaming_strings,
        };
        self.function.replay(&ids, sink).map_err(EmitError::Sink)
    }

    fn gtb_setup(&self, setup: &SetupPacket, out: &mut [u8]) -> Result<GtbResponse, RequestError> {
        self.function.gtb_setup(setup, self.ids.ms_interface, out)
    }
}

/// Error of [`write_rust`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FixedError {
    /// The configuration breaks a rule of the writer.
    #[error("{0}")]
    Device(DeviceError),
    /// The output sink failed.
    #[error("output error")]
    Format,
    /// The endpoints break a rule of the adapters (DEC-USB-002 addendum): more than one endpoint in one
    /// direction in one Alternate Setting, or one address with two types, sizes or intervals.
    #[error("endpoint {address:#04x} breaks an adapter rule")]
    Endpoints {
        /// bEndpointAddress.
        address: u8,
    },
}

/// The endpoint rules that the adapters check when a function is made at run time; a
/// [`Fixed`] function is checked here when the firmware is built, so `Function::CHECKED`.
fn check_adapter_rules<const J: usize, const S: usize, const E: usize, const B: usize>(
    cfg: &FunctionConfig<'_, J, S, E, B>,
) -> Result<(), FixedError> {
    for (i, a) in cfg.endpoints().enumerate() {
        for b in cfg.endpoints().take(i) {
            let same_setting =
                a.alternate_setting == b.alternate_setting && a.direction == b.direction;
            let mismatch = a.address() == b.address()
                && (a.transfer != b.transfer
                    || a.max_packet != b.max_packet
                    || a.interval != b.interval);
            if same_setting || mismatch {
                return Err(FixedError::Endpoints {
                    address: a.address(),
                });
            }
        }
    }
    Ok(())
}

impl From<fmt::Error> for FixedError {
    fn from(_: fmt::Error) -> Self {
        FixedError::Format
    }
}

/// Writes the Rust source of a `pub static <name>: zmidi_usb::fixed::FixedFunction` for `cfg`
/// into `out` (for a build script: a `String`, then a file in `OUT_DIR` that the firmware
/// `include!`s). The interface numbers and strings of `cfg` are not used: the constant stores
/// 0, 1 and no strings, and [`FixedFunction::replay`] patches them. Every rule of
/// `write_function` and `gtb_set` is checked here; a broken rule gives
/// [`FixedError::Device`] and writes nothing useful.
pub fn write_rust<const J: usize, const S: usize, const E: usize, const B: usize>(
    cfg: &FunctionConfig<'_, J, S, E, B>,
    name: &str,
    out: &mut impl Write,
) -> Result<(), FixedError> {
    check_adapter_rules(cfg)?;
    let mut c = cfg.clone();
    c.ac_interface = AC_TEMPLATE;
    c.ms_interface = MS_TEMPLATE;
    c.ac_string = 0;
    if let Some(m) = c.midi1.as_mut() {
        m.string_index = 0;
    }
    for s in c.midi2.as_mut_slice() {
        s.string_index = 0;
    }
    writeln!(
        out,
        "// Made by zmidi_usb::fixed::write_rust (DEC-USB-002); do not edit."
    )?;
    writeln!(
        out,
        "pub static {name}: ::zmidi_usb::fixed::FixedFunction = match ::zmidi_usb::fixed::FixedFunction::new("
    )?;
    write!(out, "    &[")?;
    let mut bytes = ByteList { out, n: 0 };
    write_function_to(&c, &mut bytes).map_err(|e| match e {
        EmitError::Device(d) => FixedError::Device(d),
        EmitError::Sink(_) => FixedError::Format,
    })?;
    let out = bytes.out;
    writeln!(out, "],")?;
    writeln!(out, "    &[")?;
    for e in c.endpoints() {
        let direction = match e.direction {
            EndpointDirection::MidiOut => "MidiOut",
            EndpointDirection::MidiIn => "MidiIn",
        };
        let transfer = match e.transfer {
            TransferType::Bulk => "Bulk",
            TransferType::Interrupt => "Interrupt",
            TransferType::Other(_) => return Err(FixedError::Device(DeviceError::Overflow)),
        };
        writeln!(
            out,
            "        ::zmidi_usb::device::SettingEndpoint {{ alternate_setting: {}, direction: \
             ::zmidi_usb::EndpointDirection::{direction}, number: {}, transfer: \
             ::zmidi_usb::ms2::TransferType::{transfer}, max_packet: {}, interval: {} }},",
            e.alternate_setting, e.number, e.max_packet, e.interval
        )?;
    }
    writeln!(out, "    ],")?;
    write!(
        out,
        "    &[::zmidi_usb::GroupMask::from_bits(0x{:04X})",
        GroupMask::ALL.bits()
    )?;
    for s in c.midi2.iter() {
        let mask = s
            .endpoints
            .iter()
            .find(|e| e.direction == EndpointDirection::MidiOut)
            .map_or(GroupMask::EMPTY, |e| {
                GroupMask::from_blocks(s.blocks.as_slice(), e)
            });
        write!(
            out,
            ", ::zmidi_usb::GroupMask::from_bits(0x{:04X})",
            mask.bits()
        )?;
    }
    writeln!(out, "],")?;
    write!(out, "    &[")?;
    for alt in 1..=c.midi2.len() {
        let alt =
            u8::try_from(alt).map_err(|_| FixedError::Device(DeviceError::TooManySettings))?;
        let mut set = [0u8; 2048];
        let n = gtb_set(&c, alt, &mut set).map_err(FixedError::Device)?;
        write!(out, "&[")?;
        for (i, b) in set[..n].iter().enumerate() {
            write!(out, "{}{b}", if i == 0 { "" } else { ", " })?;
        }
        write!(out, "], ")?;
    }
    writeln!(out, "],")?;
    writeln!(
        out,
        ") {{\n    Ok(f) => f,\n    Err(_) => panic!(\"zmidi_usb::fixed: the function breaks an adapter rule\"),\n}};"
    )?;
    Ok(())
}

/// A [`DescriptorSink`] that writes each descriptor again as decimal bytes.
struct ByteList<'o, W: Write> {
    out: &'o mut W,
    n: usize,
}

impl<W: Write> ByteList<'_, W> {
    fn put(&mut self, bytes: &[u8]) -> fmt::Result {
        for b in bytes {
            let sep = if self.n == 0 { "" } else { ", " };
            write!(self.out, "{sep}{b}")?;
            self.n += 1;
        }
        Ok(())
    }
}

impl<W: Write> DescriptorSink for ByteList<'_, W> {
    type Error = fmt::Error;

    fn interface(&mut self, f: &InterfaceFields) -> fmt::Result {
        self.put(&[
            9,
            INTERFACE,
            f.interface_number,
            f.alternate_setting,
            f.num_endpoints,
            f.class,
            f.subclass,
            f.protocol,
            f.string_index,
        ])
    }

    fn endpoint(&mut self, f: &EndpointFields, extra: &[u8]) -> fmt::Result {
        let [lo, hi] = f.max_packet.to_le_bytes();
        let len = u8::try_from(7 + extra.len()).map_err(|_| fmt::Error)?;
        self.put(&[len, ENDPOINT, f.address, f.attributes, lo, hi, f.interval])?;
        self.put(extra)
    }

    fn class_descriptor(&mut self, descriptor_type: u8, payload: &[u8]) -> fmt::Result {
        let len = u8::try_from(2 + payload.len()).map_err(|_| fmt::Error)?;
        self.put(&[len, descriptor_type])?;
        self.put(payload)
    }
}
