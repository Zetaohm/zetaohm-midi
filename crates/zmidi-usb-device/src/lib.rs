//! `usb-device` 0.3 adapter for the zmidi-usb USB-MIDI 2.0 and 1.0 codecs (DEC-014, DEC-015).
//!
//! [`MidiClass`] is a [`UsbClass`] for one USB MIDI function. It adds no protocol logic
//! (DEC-004): the descriptors are those of [`zmidi_usb::device::write_function_to`], the Group
//! Terminal Block request goes to [`zmidi_usb::handle_gtb_setup`], and the data goes through a
//! [`MidiLink`].
//!
//! The function is a [`Function`]: a [`FunctionConfig`] that the firmware makes when it runs
//! ([`MidiClass::new`]), or a [`zmidi_usb::fixed::FixedFunction`] that a build script makes with
//! [`zmidi_usb::fixed::write_rust`] ([`MidiClass::fixed`], DEC-USB-002 of zmidi-usb). The fixed
//! function links no descriptor builder and no check: about 10 KB less flash.
//!
//! - The function has Alternate Setting 0 (USB-MIDI 1.0) and the USB MIDI 2.0 settings of the
//!   [`FunctionConfig`] (USB-MIDI v2.0 3.1.1). Cargo feature `alt0` (on by default) turns on
//!   `zmidi-usb/alt0`; with it off, Alternate Setting 0 has no Jack, no endpoint and no data path
//!   (DEC-014 addendum, REQ-USB-GEN-014 of zmidi-usb): about 3,300 bytes of flash less, and a host
//!   with only a USB-MIDI 1.0 driver shows no port.
//! - One endpoint address can be in more than one Alternate Setting (USB-MIDI v2.0 Appendix B)
//!   when its transfer type, wMaxPacketSize and bInterval are the same in each setting: one
//!   hardware endpoint has one type. Each setting has at most one MIDI OUT and one MIDI IN
//!   endpoint, because a [`MidiLink`] is for one endpoint pair.
//! - The adapter reports the USB reset, SET_CONFIGURATION and SET_INTERFACE to the link
//!   (REQ-USB-UMP-008 of zmidi-usb).
//! - The Interface Association Descriptor follows `UsbDeviceBuilder::composite_with_iads`.
//! - The crate turns on the `usb-device` feature `control-buffer-256`: the Configuration
//!   descriptor with Alternate Settings 0 and 1 is 141 bytes, and the default control buffer is
//!   128 bytes. A configuration descriptor longer than 256 bytes (for example a large Jack list)
//!   does not fit; `usb-device` then gives no answer to the request.
//!
//! Use it with `UsbDevice::poll`, then call [`MidiClass::read`] and [`MidiClass::write`].

#![no_std]

use core::iter::Peekable;

use usb_device::class_prelude::*;
use usb_device::control::{Recipient, Request, RequestType};
use usb_device::endpoint::{Endpoint, EndpointAddress, EndpointType};
use usb_device::{UsbDirection, UsbError};
use zmidi_usb::device::{
    DescriptorSink, DeviceError, EmitError, EndpointFields, FunctionConfig, InterfaceFields,
    SettingEndpoint,
};
use zmidi_usb::fixed::FixedFunction;
use zmidi_usb::ms2::TransferType;
use zmidi_usb::request::{GtbResponse, SetupPacket};
use zmidi_usb::{
    EndpointDirection, LinkConfig, LinkError, LinkEvent, MidiLink, Received, SettingNumber,
};

/// The most interface names: the AudioControl interface and 7 Alternate Settings of the MIDI
/// Streaming interface.
pub const MAX_NAMES: usize = zmidi_usb::fixed::MAX_NAMES;

/// The largest Group Terminal Block answer: the header (5 bytes) and 16 blocks of 13 bytes
/// (USB-MIDI v2.0 section 6).
const GTB_ANSWER_MAX: usize = 5 + 13 * 16;

/// Audio interface class, AudioControl subclass and no protocol: the function of the Interface
/// Association Descriptor (USB-MIDI v1.0 4.3, as the AudioControl interface).
const IAD_FUNCTION: (u8, u8, u8) = (0x01, 0x01, 0x00);

/// Error of the adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The [`FunctionConfig`] breaks a rule of zmidi-usb.
    #[error("configuration: {0}")]
    Device(DeviceError),
    /// The link refused the operation (for example a caller buffer shorter than one packet).
    #[error("link: {0}")]
    Link(LinkError),
    /// An error of `usb-device`. [`UsbError::WouldBlock`]: no packet to read, or the IN
    /// endpoint is busy.
    #[error("usb-device: {0:?}")]
    Usb(UsbError),
    /// Alternate Setting `alt` has more than one MIDI OUT or more than one MIDI IN endpoint.
    #[error("Alternate Setting {alt} has more than one endpoint in one direction")]
    SettingEndpoints {
        /// bAlternateSetting.
        alt: u8,
    },
    /// Endpoint `address` has another transfer type, wMaxPacketSize or bInterval in another
    /// Alternate Setting.
    #[error("endpoint {address:#04x} differs between Alternate Settings")]
    EndpointMismatch {
        /// bEndpointAddress.
        address: u8,
    },
    /// More different endpoint addresses in one direction than the capacity `EPS`.
    #[error("more endpoints than the capacity")]
    TooManyEndpoints,
    /// More interface names than [`MAX_NAMES`], or a name for an Alternate Setting that the
    /// configuration does not have.
    #[error("too many interface names")]
    TooManyNames,
    /// The current Alternate Setting has no endpoint in this direction.
    #[error("no endpoint in this direction in the current Alternate Setting")]
    NoEndpoint,
}

impl From<UsbError> for Error {
    fn from(e: UsbError) -> Self {
        Error::Usb(e)
    }
}

impl From<LinkError> for Error {
    fn from(e: LinkError) -> Self {
        Error::Link(e)
    }
}

impl From<DeviceError> for Error {
    fn from(e: DeviceError) -> Self {
        Error::Device(e)
    }
}

/// The interface names (iInterface strings). The adapter gets a string index from the
/// allocator for each name, writes it into the [`FunctionConfig`] and answers the string
/// request.
#[derive(Debug, Clone, Copy, Default)]
pub struct InterfaceNames<'a> {
    /// The name of the AudioControl interface.
    pub audio_control: Option<&'a str>,
    /// The names of the Alternate Settings of the MIDI Streaming interface: index 0 is
    /// Alternate Setting 0 (USB-MIDI 1.0), index 1 is Alternate Setting 1, and so on.
    pub streaming: &'a [Option<&'a str>],
}

pub use zmidi_usb::fixed::{Fixed, Function, FunctionIds};

/// The endpoint indexes and values of the current Alternate Setting.
#[derive(Debug, Clone, Copy, Default)]
struct Current {
    /// Index in `outs` of the MIDI OUT endpoint.
    out: Option<usize>,
    /// Index in `ins` of the MIDI IN endpoint.
    inp: Option<usize>,
}

/// A `usb-device` class for one USB MIDI function (DEC-015).
///
/// `CABLES` is the Event Packet cable count of the [`MidiLink`]; `J`, `S`, `E` and `B` are
/// those of [`FunctionConfig`]; `EPS` is the capacity of different endpoint addresses in each
/// direction; `X` is the translator count of the [`MidiLink`]: MIDI 2.0 Protocol output from
/// Event Packets needs `X >= CABLES` (default 0, OPT-011).
pub struct MidiClass<
    'a,
    Bus: UsbBus,
    F: Function = FunctionConfig<'a>,
    const CABLES: usize = 1,
    const EPS: usize = 2,
    const X: usize = 0,
> {
    cfg: F,
    ac_if: InterfaceNumber,
    ms_if: InterfaceNumber,
    outs: [Option<EndpointOut<'a, Bus>>; EPS],
    ins: [Option<EndpointIn<'a, Bus>>; EPS],
    names: [Option<(StringIndex, &'a str)>; MAX_NAMES],
    link: MidiLink<CABLES, X>,
    current: Current,
    /// True from a write until `endpoint_in_complete` of the IN endpoint.
    in_busy: bool,
    /// True after a packet of exactly wMaxPacketSize: a zero-length packet ends the transfer.
    zlp: bool,
}

impl<
        'a,
        Bus: UsbBus,
        const J: usize,
        const S: usize,
        const E: usize,
        const B: usize,
        const CABLES: usize,
        const EPS: usize,
        const X: usize,
    > MidiClass<'a, Bus, FunctionConfig<'a, J, S, E, B>, CABLES, EPS, X>
{
    /// Creates the class for a function that the firmware makes when it runs: two interface
    /// numbers (AudioControl, MIDI Streaming), the string indexes of `names` and the endpoints
    /// of `cfg` from `alloc`. The adapter sets `ac_interface`, `ms_interface` and the string
    /// indexes of `cfg`. `link` gives the Protocol and writer options of the [`MidiLink`]; the
    /// adapter sets its received direction (MIDI OUT), Group mask and packet size from the
    /// current Alternate Setting.
    pub fn new(
        alloc: &'a UsbBusAllocator<Bus>,
        cfg: FunctionConfig<'a, J, S, E, B>,
        names: InterfaceNames<'a>,
        link: LinkConfig,
    ) -> Result<Self, Error> {
        Self::with_function(alloc, cfg, names, link)
    }
}

impl<'a, Bus: UsbBus, const CABLES: usize, const EPS: usize, const X: usize>
    MidiClass<'a, Bus, Fixed, CABLES, EPS, X>
{
    /// Creates the class for a function made when the firmware is built (a
    /// [`FixedFunction`] of [`zmidi_usb::fixed::write_rust`], DEC-USB-002), as [`Self::new`]
    /// does for a [`FunctionConfig`]. The firmware links no descriptor builder.
    pub fn fixed(
        alloc: &'a UsbBusAllocator<Bus>,
        function: &'static FixedFunction,
        names: InterfaceNames<'a>,
        link: LinkConfig,
    ) -> Result<Self, Error> {
        Self::with_function(alloc, Fixed::new(function), names, link)
    }
}

impl<'a, Bus: UsbBus, F: Function, const CABLES: usize, const EPS: usize, const X: usize>
    MidiClass<'a, Bus, F, CABLES, EPS, X>
{
    /// Creates the class for any [`Function`]; [`Self::new`] and [`Self::fixed`] use it.
    pub fn with_function(
        alloc: &'a UsbBusAllocator<Bus>,
        mut cfg: F,
        names: InterfaceNames<'a>,
        link: LinkConfig,
    ) -> Result<Self, Error> {
        let ac_if = alloc.interface();
        let ms_if = alloc.interface();
        let mut ids = FunctionIds {
            ac_interface: u8::from(ac_if),
            ms_interface: u8::from(ms_if),
            ..FunctionIds::default()
        };

        let mut table: [Option<(StringIndex, &'a str)>; MAX_NAMES] = [None; MAX_NAMES];
        let mut count = 0;
        let mut add = |name: Option<&'a str>| -> Result<u8, Error> {
            let Some(name) = name else { return Ok(0) };
            let slot = table.get_mut(count).ok_or(Error::TooManyNames)?;
            let index = alloc.string();
            *slot = Some((index, name));
            count += 1;
            Ok(u8::from(index))
        };
        ids.ac_string = add(names.audio_control)?;
        if names.streaming.len() > cfg.alternate_settings() || names.streaming.len() > MAX_NAMES {
            return Err(Error::TooManyNames);
        }
        for (alt, name) in names.streaming.iter().enumerate() {
            ids.streaming_strings[alt] = add(*name)?;
        }

        cfg.prepare(&ids)?;
        if !F::CHECKED {
            check_settings(&cfg)?;
        }

        let mut outs: [Option<EndpointOut<'a, Bus>>; EPS] = core::array::from_fn(|_| None);
        let mut ins: [Option<EndpointIn<'a, Bus>>; EPS] = core::array::from_fn(|_| None);
        for se in cfg.endpoints() {
            match se.direction {
                EndpointDirection::MidiOut => alloc_into::<F, _, _, EPS>(alloc, &mut outs, &se)?,
                EndpointDirection::MidiIn => alloc_into::<F, _, _, EPS>(alloc, &mut ins, &se)?,
            }
        }

        let mut class = MidiClass {
            cfg,
            ac_if,
            ms_if,
            outs,
            ins,
            names: table,
            link: MidiLink::new(LinkConfig {
                receive: EndpointDirection::MidiOut,
                ..link
            })?,
            current: Current::default(),
            in_busy: false,
            zlp: false,
        };
        class.select(0);
        Ok(class)
    }

    /// The function, with the interface numbers and string indexes of the adapter.
    pub fn config(&self) -> &F {
        &self.cfg
    }

    /// The link state (the selected Alternate Setting, [`MidiLink::pending`] and others).
    pub fn link(&self) -> &MidiLink<CABLES, X> {
        &self.link
    }

    /// Reads one packet from the MIDI OUT endpoint of the current Alternate Setting into `buf`
    /// and gives each UMP of it to `sink` ([`MidiLink::read_with`]). Gives the packet length.
    /// `buf` must hold wMaxPacketSize bytes. [`UsbError::WouldBlock`] when no packet is ready.
    pub fn read(
        &mut self,
        buf: &mut [u8],
        sink: impl FnMut(Result<Received, LinkError>),
    ) -> Result<usize, Error> {
        let ep = self
            .current
            .out
            .and_then(|i| self.outs.get(i))
            .and_then(Option::as_ref)
            .ok_or(Error::NoEndpoint)?;
        let n = ep.read(buf)?;
        self.link.read_with(buf.get(..n).unwrap_or(&[]), sink);
        Ok(n)
    }

    /// Writes one packet to the MIDI IN endpoint of the current Alternate Setting from `umps`
    /// ([`MidiLink::write_packet`]) and gives its length; `buf` holds the packet and must have
    /// [`MidiLink::packet_size`] bytes. A UMP that does not fit stays in `umps`.
    ///
    /// - [`UsbError::WouldBlock`] while the last packet is not sent; nothing is taken from
    ///   `umps`.
    /// - After a packet of exactly wMaxPacketSize, a call with nothing to send writes a
    ///   zero-length packet, so that the host sees the end of the bulk transfer.
    /// - 0 with no packet when there is nothing to send.
    ///
    /// An error of the endpoint write after the link wrote the packet loses that packet; the
    /// error comes back to the caller.
    pub fn write<'w, I>(&mut self, umps: &mut Peekable<I>, buf: &mut [u8]) -> Result<usize, Error>
    where
        I: Iterator<Item = &'w [u32]>,
    {
        let ep = self
            .current
            .inp
            .and_then(|i| self.ins.get(i))
            .and_then(Option::as_ref)
            .ok_or(Error::NoEndpoint)?;
        if self.in_busy {
            return Err(Error::Usb(UsbError::WouldBlock));
        }
        if self.zlp && umps.peek().is_none() && !self.link.pending() {
            ep.write(&[])?;
            self.zlp = false;
            self.in_busy = true;
            return Ok(0);
        }
        let n = self.link.write_packet(umps, buf)?;
        if n == 0 {
            return Ok(0);
        }
        ep.write(buf.get(..n).unwrap_or(&[]))?;
        self.in_busy = true;
        self.zlp = n == usize::from(ep.max_packet_size());
        Ok(n)
    }

    /// Selects Alternate Setting `alt`: the endpoint pair, and the Group mask and packet size
    /// of the link (`MidiLink::report_setting`).
    fn select(&mut self, alt: u8) {
        let mut out = None;
        let mut inp = None;
        for se in self.cfg.endpoints().filter(|e| e.alternate_setting == alt) {
            match se.direction {
                EndpointDirection::MidiOut => out = Some(se),
                EndpointDirection::MidiIn => inp = Some(se),
            }
        }
        self.current = Current {
            out: out.and_then(|se| position(&self.outs, se.address())),
            inp: inp.and_then(|se| position(&self.ins, se.address())),
        };
        let mask = self.cfg.group_mask(alt);
        let max_packet = inp.or(out).map_or(64, |se| se.max_packet);
        // A configuration that function_len accepts has wMaxPacketSize 8 or more; if not,
        // the link keeps its packet size and still takes the setting.
        if self
            .link
            .report_setting(SettingNumber(alt), mask, max_packet)
            .is_err()
        {
            self.link
                .report(LinkEvent::AlternateSetting(SettingNumber(alt)));
        }
        self.in_busy = false;
        self.zlp = false;
    }

    /// A USB reset or a configuration change: the link goes to Alternate Setting 0.
    fn event(&mut self, event: LinkEvent) {
        self.link.report(event);
        self.select(0);
    }
}

/// Checks that each Alternate Setting has at most one endpoint in each direction.
fn check_settings<F: Function>(cfg: &F) -> Result<(), Error> {
    for alt in 0..cfg.alternate_settings() {
        let alt = u8::try_from(alt).map_err(|_| Error::Device(DeviceError::TooManySettings))?;
        let mut counts = [0u8; 2];
        for se in cfg.endpoints().filter(|e| e.alternate_setting == alt) {
            let i = usize::from(se.direction == EndpointDirection::MidiIn);
            counts[i] += 1;
        }
        if counts.iter().any(|c| *c > 1) {
            return Err(Error::SettingEndpoints { alt });
        }
    }
    Ok(())
}

/// The `usb-device` endpoint type of a zmidi-usb transfer type; the zmidi-usb writer refuses the
/// other types, so they do not come here.
fn endpoint_type(t: TransferType) -> EndpointType {
    match t {
        TransferType::Interrupt => EndpointType::Interrupt,
        _ => EndpointType::Bulk,
    }
}

/// Allocates the endpoint of `se` into `table`, or checks the one with the same address.
fn alloc_into<
    'a,
    F: Function,
    Bus: UsbBus,
    D: usb_device::endpoint::EndpointDirection,
    const N: usize,
>(
    alloc: &'a UsbBusAllocator<Bus>,
    table: &mut [Option<Endpoint<'a, Bus, D>>; N],
    se: &SettingEndpoint,
) -> Result<(), Error> {
    let ep_type = endpoint_type(se.transfer);
    if let Some(ep) = table
        .iter()
        .flatten()
        .find(|ep| u8::from(ep.address()) == se.address())
    {
        if !F::CHECKED && ep.ep_type() != ep_type
            || ep.max_packet_size() != se.max_packet
            || ep.interval() != se.interval
        {
            return Err(Error::EndpointMismatch {
                address: se.address(),
            });
        }
        return Ok(());
    }
    let slot = table
        .iter_mut()
        .find(|s| s.is_none())
        .ok_or(Error::TooManyEndpoints)?;
    let dir = match se.direction {
        EndpointDirection::MidiOut => UsbDirection::Out,
        EndpointDirection::MidiIn => UsbDirection::In,
    };
    let addr = EndpointAddress::from_parts(usize::from(se.number), dir);
    *slot = Some(alloc.alloc(Some(addr), ep_type, se.max_packet, se.interval)?);
    Ok(())
}

/// The index of the endpoint with address `address`.
fn position<Bus: UsbBus, D: usb_device::endpoint::EndpointDirection>(
    table: &[Option<Endpoint<'_, Bus, D>>],
    address: u8,
) -> Option<usize> {
    table.iter().position(|ep| {
        ep.as_ref()
            .is_some_and(|ep| u8::from(ep.address()) == address)
    })
}

/// Gives the descriptors of zmidi-usb to the `usb-device` descriptor writer.
struct Sink<'w, 'x, 'c, 'a, Bus: UsbBus, const EPS: usize> {
    writer: &'w mut DescriptorWriter<'x>,
    ac_if: InterfaceNumber,
    ms_if: InterfaceNumber,
    names: &'c [Option<(StringIndex, &'a str)>],
    outs: &'c [Option<EndpointOut<'a, Bus>>; EPS],
    ins: &'c [Option<EndpointIn<'a, Bus>>; EPS],
}

impl<Bus: UsbBus, const EPS: usize> DescriptorSink for Sink<'_, '_, '_, '_, Bus, EPS> {
    type Error = UsbError;

    fn interface(&mut self, f: &InterfaceFields) -> Result<(), UsbError> {
        let number = [self.ac_if, self.ms_if]
            .into_iter()
            .find(|n| u8::from(*n) == f.interface_number)
            .ok_or(UsbError::InvalidState)?;
        let string = match f.string_index {
            0 => None,
            i => Some(
                self.names
                    .iter()
                    .flatten()
                    .map(|(s, _)| *s)
                    .find(|s| u8::from(*s) == i)
                    .ok_or(UsbError::InvalidState)?,
            ),
        };
        self.writer.interface_alt(
            number,
            f.alternate_setting,
            f.class,
            f.subclass,
            f.protocol,
            string,
        )
    }

    fn endpoint(&mut self, f: &EndpointFields, extra: &[u8]) -> Result<(), UsbError> {
        let copy = |buf: &mut [u8]| {
            let dst = buf.get_mut(..extra.len()).ok_or(UsbError::BufferOverflow)?;
            dst.copy_from_slice(extra);
            Ok(extra.len())
        };
        if f.address & 0x80 != 0 {
            let ep = self
                .ins
                .iter()
                .flatten()
                .find(|ep| u8::from(ep.address()) == f.address)
                .ok_or(UsbError::InvalidState)?;
            self.writer.endpoint_ex(ep, copy)
        } else {
            let ep = self
                .outs
                .iter()
                .flatten()
                .find(|ep| u8::from(ep.address()) == f.address)
                .ok_or(UsbError::InvalidState)?;
            self.writer.endpoint_ex(ep, copy)
        }
    }

    fn class_descriptor(&mut self, descriptor_type: u8, payload: &[u8]) -> Result<(), UsbError> {
        self.writer.write(descriptor_type, payload)
    }
}

impl<Bus: UsbBus, F: Function, const CABLES: usize, const EPS: usize, const X: usize> UsbClass<Bus>
    for MidiClass<'_, Bus, F, CABLES, EPS, X>
{
    fn get_configuration_descriptors(
        &self,
        writer: &mut DescriptorWriter,
    ) -> usb_device::Result<()> {
        let (class, subclass, protocol) = IAD_FUNCTION;
        writer.iad(self.ac_if, 2, class, subclass, protocol, None)?;
        let mut sink = Sink {
            writer,
            ac_if: self.ac_if,
            ms_if: self.ms_if,
            names: &self.names,
            outs: &self.outs,
            ins: &self.ins,
        };
        self.cfg.write(&mut sink).map_err(|e| match e {
            EmitError::Sink(e) => e,
            EmitError::Device(_) => UsbError::InvalidState,
        })
    }

    fn get_string(&self, index: StringIndex, _lang_id: LangID) -> Option<&str> {
        self.names
            .iter()
            .flatten()
            .find(|(s, _)| *s == index)
            .map(|(_, name)| *name)
    }

    fn reset(&mut self) {
        self.event(LinkEvent::UsbReset);
    }

    fn control_out(&mut self, xfer: ControlOut<Bus>) {
        // SET_CONFIGURATION: seen here, not accepted; UsbDevice answers it.
        let req = xfer.request();
        if req.request_type == RequestType::Standard
            && req.recipient == Recipient::Device
            && req.request == Request::SET_CONFIGURATION
        {
            self.event(LinkEvent::ConfigurationChange);
        }
    }

    fn control_in(&mut self, xfer: ControlIn<Bus>) {
        let req = *xfer.request();
        let setup = SetupPacket::from_parts(
            SetupPacket::request_type_from(
                req.direction == UsbDirection::In,
                req.request_type as u8,
                req.recipient as u8,
            ),
            req.request,
            req.value,
            req.index,
            req.length,
        );
        if setup.gtb_alternate_setting().is_none() {
            return;
        }
        let mut answer = [0u8; GTB_ANSWER_MAX];
        // An error of the stall or of the answer has no other receiver: the host sees the stall
        // or a timeout.
        let _ = match self.cfg.gtb_setup(&setup, &mut answer) {
            Ok(GtbResponse::Answered(n)) => xfer.accept_with(answer.get(..n).unwrap_or(&[])),
            Ok(GtbResponse::NotHandled) => Ok(()),
            Ok(GtbResponse::Stall) | Err(_) => xfer.reject(),
        };
    }

    fn endpoint_in_complete(&mut self, addr: EndpointAddress) {
        let current = self
            .current
            .inp
            .and_then(|i| self.ins.get(i))
            .and_then(Option::as_ref);
        if current.is_some_and(|ep| ep.address() == addr) {
            self.in_busy = false;
        }
    }

    fn get_alt_setting(&mut self, interface: InterfaceNumber) -> Option<u8> {
        (interface == self.ms_if).then(|| self.link.setting().0)
    }

    fn set_alt_setting(&mut self, interface: InterfaceNumber, alternative: u8) -> bool {
        if interface != self.ms_if || usize::from(alternative) >= self.cfg.alternate_settings() {
            return false;
        }
        self.select(alternative);
        true
    }
}
