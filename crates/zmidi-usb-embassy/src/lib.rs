//! `embassy-usb` 0.6 adapter for the zmidi-usb USB-MIDI 2.0 and 1.0 codecs (DEC-014, DEC-015).
//!
//! [`MidiClass`] adds one USB MIDI function to an `embassy_usb::Builder`. It adds no protocol
//! logic (DEC-004): the descriptors are those of [`zmidi_usb::device::write_function`], the
//! Group Terminal Block request goes to [`zmidi_usb::handle_gtb_setup`], and the data goes
//! through a [`MidiLink`].
//!
//! The function is a [`Function`] (DEC-USB-002 of zmidi-usb): a [`FunctionConfig`]
//! ([`MidiClass::new`]) or a [`zmidi_usb::fixed::FixedFunction`] that a build script makes
//! ([`MidiClass::fixed`]), with no descriptor builder in the firmware.
//!
//! Cargo feature `alt0` (on by default) turns on `zmidi-usb/alt0`, the USB-MIDI 1.0 data path of
//! Alternate Setting 0. With it off, Alternate Setting 0 has no Jack and no endpoint (DEC-014
//! addendum, REQ-USB-GEN-014 of zmidi-usb): about 3,100 bytes of flash less; a host with only a
//! USB-MIDI 1.0 driver then shows no port.
//!
//! - The function has Alternate Setting 0 (USB-MIDI 1.0) and the USB MIDI 2.0 settings of the
//!   [`FunctionConfig`] (USB-MIDI v2.0 3.1.1).
//! - Each endpoint address is in exactly one Alternate Setting: `embassy-usb` 0.6.0 disables an
//!   endpoint that two settings share (DEC-015). [`default_config`] gives OUT 1 and IN 1 on
//!   Alternate Setting 0, OUT 2 and IN 2 on Alternate Setting 1. Each setting has at most one
//!   MIDI OUT and one MIDI IN endpoint, because a [`MidiLink`] is for one endpoint pair.
//! - A [`State`] holds the link in a blocking mutex. The mutex is held only while the
//!   synchronous codec runs, never across an `.await`. Its handler reports the USB reset, the
//!   configuration change and SET_INTERFACE to the link (REQ-USB-UMP-008 of zmidi-usb) and
//!   answers the Group Terminal Block request and the interface strings.
//! - [`MidiClass::split`] gives a [`Sender`] and a [`Receiver`] for two tasks.
//! - The Interface Association Descriptor follows `embassy_usb::Config::composite_with_iads`.
//! - The endpoint allocation of `embassy-usb` panics when the driver has no free endpoint.

#![no_std]

use core::cell::RefCell;
use core::iter::Peekable;
use core::mem::MaybeUninit;

use embassy_futures::select::{select, Either};
use embassy_sync::blocking_mutex::raw::RawMutex;
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::signal::Signal;
use embassy_usb::control::{InResponse, Recipient, Request};
use embassy_usb::driver::{
    Direction, Driver, Endpoint, EndpointAddress, EndpointError, EndpointIn, EndpointOut,
    EndpointType,
};
use embassy_usb::types::{InterfaceNumber, StringIndex};
use embassy_usb::{Builder, Handler, InterfaceBuilder};
use zmidi_usb::constants::{ENDPOINT, INTERFACE};
use zmidi_usb::device::{
    DescriptorSink, DeviceError, EmitError, EndpointFields, FunctionConfig, InterfaceFields,
};
use zmidi_usb::fixed::FixedFunction;
pub use zmidi_usb::fixed::{Fixed, Function, FunctionIds};
use zmidi_usb::request::{GtbResponse, SetupPacket};
use zmidi_usb::{
    BusSpeed, EndpointDirection, LinkConfig, LinkError, LinkEvent, MidiLink, Received,
    SettingNumber,
};

/// The most interface names: the AudioControl interface and 7 Alternate Settings of the MIDI
/// Streaming interface.
pub const MAX_NAMES: usize = zmidi_usb::fixed::MAX_NAMES;

/// The largest function that [`MidiClass::new`] writes (on the stack, during the build).
pub const FUNCTION_MAX: usize = 512;

/// Audio interface class, AudioControl subclass and no protocol: the function of the Interface
/// Association Descriptor (USB-MIDI v1.0 4.3, as the AudioControl interface).
const FUNCTION: (u8, u8, u8) = (0x01, 0x01, 0x00);

/// Error of the adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The [`FunctionConfig`] breaks a rule of zmidi-usb, or its descriptors are longer than
    /// [`FUNCTION_MAX`] ([`DeviceError::Overflow`]).
    #[error("configuration: {0}")]
    Device(DeviceError),
    /// The link refused the operation (for example a caller buffer shorter than one packet).
    #[error("link: {0}")]
    Link(LinkError),
    /// An error of the endpoint: [`EndpointError::Disabled`] when the host deselects the
    /// Alternate Setting or the configuration; call `wait_connection` again.
    #[error("endpoint: {0:?}")]
    Endpoint(EndpointError),
    /// Alternate Setting `alt` has more than one MIDI OUT or more than one MIDI IN endpoint.
    #[error("Alternate Setting {alt} has more than one endpoint in one direction")]
    SettingEndpoints {
        /// bAlternateSetting.
        alt: u8,
    },
    /// Endpoint `address` is in more than one Alternate Setting (DEC-015).
    #[error("endpoint {address:#04x} is in more than one Alternate Setting")]
    SharedEndpoint {
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
    /// The builder gave interface numbers or Alternate Setting numbers that differ from the
    /// descriptors (another class added interfaces in between).
    #[error("interface numbers of the builder differ from the descriptors")]
    Layout,
    /// The current Alternate Setting has no endpoint in this direction.
    #[error("no endpoint in this direction in the current Alternate Setting")]
    NoEndpoint,
}

impl From<DeviceError> for Error {
    fn from(e: DeviceError) -> Self {
        Error::Device(e)
    }
}

impl From<LinkError> for Error {
    fn from(e: LinkError) -> Self {
        Error::Link(e)
    }
}

impl From<EndpointError> for Error {
    fn from(e: EndpointError) -> Self {
        Error::Endpoint(e)
    }
}

/// The interface names (iInterface strings). The adapter gets a string index from the builder
/// for each name, writes it into the [`FunctionConfig`] and answers the string request.
#[derive(Debug, Clone, Copy, Default)]
pub struct InterfaceNames<'a> {
    /// The name of the AudioControl interface.
    pub audio_control: Option<&'a str>,
    /// The names of the Alternate Settings of the MIDI Streaming interface: index 0 is
    /// Alternate Setting 0 (USB-MIDI 1.0), index 1 is Alternate Setting 1, and so on.
    pub streaming: &'a [Option<&'a str>],
}

/// The default configuration with distinct endpoint pairs (DEC-015): the zmidi-usb default
/// ([`FunctionConfig::new`]) with OUT 2 and IN 2 on Alternate Setting 1. The adapter sets the
/// interface numbers.
pub fn default_config(speed: BusSpeed) -> Result<FunctionConfig<'static>, DeviceError> {
    let mut cfg = FunctionConfig::new(0, 1, speed)?;
    for s in cfg.midi2.as_mut_slice() {
        for e in s.endpoints.as_mut_slice() {
            e.number = 2;
        }
    }
    Ok(cfg)
}

/// The run-time state of the function: the configuration and the link.
struct Inner<const CABLES: usize, F: Function, const X: usize> {
    cfg: F,
    link: MidiLink<CABLES, X>,
}

impl<const CABLES: usize, F: Function, const X: usize> Inner<CABLES, F, X> {
    /// Selects Alternate Setting `alt`: the Group mask and packet size of the link
    /// (`MidiLink::report_setting`).
    fn select(&mut self, alt: u8) {
        let mut max_packet = None;
        for se in self.cfg.endpoints().filter(|e| e.alternate_setting == alt) {
            if se.direction == EndpointDirection::MidiIn || max_packet.is_none() {
                max_packet = Some(se.max_packet);
            }
        }
        let mask = self.cfg.group_mask(alt);
        // A configuration that function_len accepts has wMaxPacketSize 8 or more; if not,
        // the link keeps its packet size and still takes the setting.
        if self
            .link
            .report_setting(SettingNumber(alt), mask, max_packet.unwrap_or(64))
            .is_err()
        {
            self.link
                .report(LinkEvent::AlternateSetting(SettingNumber(alt)));
        }
    }
}

/// The part of [`State`] that the handler, the [`Sender`] and the [`Receiver`] share.
/// The run-time state of [`Shared`], or none before `MidiClass::new`. Not an `Option`: the
/// `repr(u8)` tag makes the empty value all zero bits, so a `static` [`State`] goes to `.bss`
/// and has no flash image (OPT-013); an `Option` uses a niche with a non-zero `None`.
#[repr(u8)]
enum Slot<const CABLES: usize, F: Function, const X: usize> {
    Empty = 0,
    Full(Inner<CABLES, F, X>),
}

struct Shared<'d, M: RawMutex, const CABLES: usize, F: Function, const X: usize> {
    inner: Mutex<M, RefCell<Slot<CABLES, F, X>>>,
    /// Signaled on each event, for the receiver and for the sender.
    rx_changed: Signal<M, ()>,
    tx_changed: Signal<M, ()>,
    /// The function borrows from the caller for `'d` (a FunctionConfig of Jack lists).
    _function: core::marker::PhantomData<&'d ()>,
}

impl<M: RawMutex, const CABLES: usize, F: Function, const X: usize> Shared<'_, M, CABLES, F, X> {
    /// Runs `f` on the run-time state, with the mutex held (no `.await` in `f`).
    fn with<R>(&self, f: impl FnOnce(&mut Inner<CABLES, F, X>) -> R) -> Option<R> {
        self.inner.lock(|c| match &mut *c.borrow_mut() {
            Slot::Full(inner) => Some(f(inner)),
            Slot::Empty => None,
        })
    }

    /// The selected Alternate Setting.
    fn setting(&self) -> u8 {
        self.with(|i| i.link.setting().0).unwrap_or(0)
    }

    fn changed(&self) {
        self.rx_changed.signal(());
        self.tx_changed.signal(());
    }
}

/// The `embassy-usb` handler of the function.
struct Control<'d, M: RawMutex, const CABLES: usize, F: Function, const X: usize> {
    shared: &'d Shared<'d, M, CABLES, F, X>,
    ms_if: u8,
    names: [Option<(u8, &'d str)>; MAX_NAMES],
}

impl<M: RawMutex, const CABLES: usize, F: Function, const X: usize> Control<'_, M, CABLES, F, X> {
    /// A USB reset or a configuration change: the link goes to Alternate Setting 0.
    fn event(&mut self, event: LinkEvent) {
        self.shared.with(|i| {
            i.link.report(event);
            i.select(0);
        });
        self.shared.changed();
    }
}

impl<M: RawMutex, const CABLES: usize, F: Function, const X: usize> Handler
    for Control<'_, M, CABLES, F, X>
{
    fn reset(&mut self) {
        self.event(LinkEvent::UsbReset);
    }

    fn configured(&mut self, _configured: bool) {
        self.event(LinkEvent::ConfigurationChange);
    }

    fn set_alternate_setting(&mut self, iface: InterfaceNumber, alternate_setting: u8) {
        if u8::from(iface) != self.ms_if {
            return;
        }
        self.shared.with(|i| i.select(alternate_setting));
        self.shared.changed();
    }

    fn control_in<'a>(&'a mut self, req: Request, buf: &'a mut [u8]) -> Option<InResponse<'a>> {
        let setup = setup_packet(&req);
        setup.gtb_alternate_setting()?;
        let r = self.shared.with(|i| i.cfg.gtb_setup(&setup, buf))?;
        match r {
            Ok(GtbResponse::Answered(n)) => Some(InResponse::Accepted(buf.get(..n).unwrap_or(&[]))),
            Ok(GtbResponse::NotHandled) => None,
            Ok(GtbResponse::Stall) | Err(_) => Some(InResponse::Rejected),
        }
    }

    fn get_string(&mut self, index: StringIndex, _lang_id: u16) -> Option<&str> {
        self.names
            .iter()
            .flatten()
            .find(|(i, _)| *i == u8::from(index))
            .map(|(_, name)| *name)
    }
}

/// The setup packet of zmidi-usb from the parsed request of `embassy-usb`.
fn setup_packet(req: &Request) -> SetupPacket {
    let kind = req.request_type as u8;
    let recipient = match req.recipient {
        Recipient::Device => 0,
        Recipient::Interface => 1,
        Recipient::Endpoint => 2,
        Recipient::Other => 3,
        Recipient::Reserved => 4,
    };
    SetupPacket::from_parts(
        SetupPacket::request_type_from(req.direction == Direction::In, kind, recipient),
        req.request,
        req.value,
        req.index,
        req.length,
    )
}

/// The state of one USB MIDI function, for [`MidiClass::new`]. Put it in a `static` (for
/// example with `static_cell`); `new` is a `const fn`.
pub struct State<
    'd,
    M: RawMutex,
    const CABLES: usize = 1,
    F: Function = FunctionConfig<'d>,
    const X: usize = 0,
> {
    control: MaybeUninit<Control<'d, M, CABLES, F, X>>,
    shared: Shared<'d, M, CABLES, F, X>,
}

impl<M: RawMutex, const CABLES: usize, F: Function, const X: usize> Default
    for State<'_, M, CABLES, F, X>
{
    fn default() -> Self {
        Self::new()
    }
}

impl<M: RawMutex, const CABLES: usize, F: Function, const X: usize> State<'_, M, CABLES, F, X> {
    /// An empty state.
    pub const fn new() -> Self {
        State {
            control: MaybeUninit::uninit(),
            shared: Shared {
                inner: Mutex::new(RefCell::new(Slot::Empty)),
                rx_changed: Signal::new(),
                tx_changed: Signal::new(),
                _function: core::marker::PhantomData,
            },
        }
    }
}

/// One USB MIDI function on `embassy-usb` (DEC-015). [`Self::split`] gives the two halves.
///
/// `CABLES` is the Event Packet cable count of the [`MidiLink`]; `J`, `S`, `E` and `B` are
/// those of [`FunctionConfig`]; `EPS` is the capacity of endpoint addresses in each direction
/// (one for each Alternate Setting with an endpoint in that direction); `X` is the translator
/// count of the [`MidiLink`]: MIDI 2.0 Protocol output from Event Packets needs `X >= CABLES`
/// (default 0, OPT-011). [`State`] has the same `CABLES`, `J`, `S`, `E`, `B` and `X`.
pub struct MidiClass<
    'd,
    D: Driver<'d>,
    M: RawMutex,
    const CABLES: usize = 1,
    F: Function = FunctionConfig<'d>,
    const EPS: usize = 2,
    const X: usize = 0,
> {
    sender: Sender<'d, D, M, CABLES, F, EPS, X>,
    receiver: Receiver<'d, D, M, CABLES, F, EPS, X>,
}

impl<
        'd,
        D: Driver<'d>,
        M: RawMutex,
        const CABLES: usize,
        const J: usize,
        const S: usize,
        const E: usize,
        const B: usize,
        const EPS: usize,
        const X: usize,
    > MidiClass<'d, D, M, CABLES, FunctionConfig<'d, J, S, E, B>, EPS, X>
{
    /// Adds a function made when the firmware runs ([`FunctionConfig`]); see
    /// [`Self::with_function`].
    pub fn new(
        builder: &mut Builder<'d, D>,
        state: &'d mut State<'d, M, CABLES, FunctionConfig<'d, J, S, E, B>, X>,
        cfg: FunctionConfig<'d, J, S, E, B>,
        names: InterfaceNames<'d>,
        link: LinkConfig,
    ) -> Result<Self, Error> {
        Self::with_function(builder, state, cfg, names, link)
    }
}

impl<'d, D: Driver<'d>, M: RawMutex, const CABLES: usize, const EPS: usize, const X: usize>
    MidiClass<'d, D, M, CABLES, Fixed, EPS, X>
{
    /// Adds a function made when the firmware is built (a [`FixedFunction`] of
    /// [`zmidi_usb::fixed::write_rust`], DEC-USB-002 of zmidi-usb): no descriptor builder and no
    /// check in the firmware; see [`Self::with_function`].
    pub fn fixed(
        builder: &mut Builder<'d, D>,
        state: &'d mut State<'d, M, CABLES, Fixed, X>,
        function: &'static FixedFunction,
        names: InterfaceNames<'d>,
        link: LinkConfig,
    ) -> Result<Self, Error> {
        Self::with_function(builder, state, Fixed::new(function), names, link)
    }
}

impl<
        'd,
        D: Driver<'d>,
        M: RawMutex,
        const CABLES: usize,
        F: Function,
        const EPS: usize,
        const X: usize,
    > MidiClass<'d, D, M, CABLES, F, EPS, X>
{
    /// Adds the function to `builder`: the string indexes of `names`, an Interface
    /// Association Descriptor when the builder writes them, the AudioControl and MIDI
    /// Streaming interfaces with each Alternate Setting of `cfg`, the endpoints, and the
    /// handler. The adapter gives the function its interface numbers and string indexes
    /// ([`Function::prepare`]). `link` gives the Protocol and writer options of the
    /// [`MidiLink`]; the adapter sets its received direction (MIDI OUT), Group mask and packet
    /// size from the current setting.
    pub fn with_function(
        builder: &mut Builder<'d, D>,
        state: &'d mut State<'d, M, CABLES, F, X>,
        mut cfg: F,
        names: InterfaceNames<'d>,
        link: LinkConfig,
    ) -> Result<Self, Error> {
        let mut table: [Option<(u8, &'d str)>; MAX_NAMES] = [None; MAX_NAMES];
        let mut count = 0;
        let mut add = |builder: &mut Builder<'d, D>, name: Option<&'d str>| -> Result<u8, Error> {
            let Some(name) = name else { return Ok(0) };
            let slot = table.get_mut(count).ok_or(Error::TooManyNames)?;
            let index = u8::from(builder.string());
            *slot = Some((index, name));
            count += 1;
            Ok(index)
        };
        let mut ids = FunctionIds {
            ac_interface: 0,
            ms_interface: 1,
            ..FunctionIds::default()
        };
        ids.ac_string = add(builder, names.audio_control)?;
        if names.streaming.len() > cfg.alternate_settings() || names.streaming.len() > MAX_NAMES {
            return Err(Error::TooManyNames);
        }
        for (alt, name) in names.streaming.iter().enumerate() {
            ids.streaming_strings[alt] = add(builder, *name)?;
        }

        // The rules first: a configuration that is refused adds nothing to the builder.
        cfg.prepare(&ids)?;
        check_endpoints::<F, EPS>(&cfg)?;

        let mut outs: [Option<(u8, D::EndpointOut)>; EPS] = core::array::from_fn(|_| None);
        let mut ins: [Option<(u8, D::EndpointIn)>; EPS] = core::array::from_fn(|_| None);
        let (class, subclass, protocol) = FUNCTION;
        let mut func = builder.function(class, subclass, protocol);
        let mut ac = func.interface();
        let ac_if = u8::from(ac.interface_number());
        let ms_if = ac_if.checked_add(1).ok_or(Error::Layout)?;
        ids.ac_interface = ac_if;
        ids.ms_interface = ms_if;
        cfg.prepare(&ids)?;
        let mut sink = ByteSink {
            bytes: [0u8; FUNCTION_MAX],
            len: 0,
        };
        cfg.write(&mut sink).map_err(|e| match e {
            EmitError::Device(e) => Error::Device(e),
            EmitError::Sink(()) => Error::Device(DeviceError::Overflow),
        })?;
        let bytes = sink.bytes.get(..sink.len).unwrap_or(&[]);
        fill(&mut ac, bytes, ac_if, &mut outs, &mut ins)?;
        let mut ms = func.interface();
        if u8::from(ms.interface_number()) != ms_if {
            return Err(Error::Layout);
        }
        fill(&mut ms, bytes, ms_if, &mut outs, &mut ins)?;
        drop(func);

        let mut inner = Inner {
            cfg,
            link: MidiLink::new(LinkConfig {
                receive: EndpointDirection::MidiOut,
                ..link
            })?,
        };
        inner.select(0);
        let State { control, shared } = state;
        shared.inner.lock(|c| *c.borrow_mut() = Slot::Full(inner));
        let shared: &'d Shared<'d, M, CABLES, F, X> = shared;
        let control = control.write(Control {
            shared,
            ms_if,
            names: table,
        });
        builder.handler(control);
        Ok(MidiClass {
            sender: Sender {
                shared,
                eps: ins,
                zlp: None,
            },
            receiver: Receiver { shared, eps: outs },
        })
    }

    /// The two halves: the [`Sender`] (MIDI IN endpoints, device to host) and the
    /// [`Receiver`] (MIDI OUT endpoints, host to device).
    pub fn split(
        self,
    ) -> (
        Sender<'d, D, M, CABLES, F, EPS, X>,
        Receiver<'d, D, M, CABLES, F, EPS, X>,
    ) {
        (self.sender, self.receiver)
    }
}

/// A descriptor sink that writes the descriptors as bytes, for [`fill`].
struct ByteSink {
    bytes: [u8; FUNCTION_MAX],
    len: usize,
}

impl ByteSink {
    fn put(&mut self, bytes: &[u8]) -> Result<(), ()> {
        let dst = self
            .bytes
            .get_mut(self.len..self.len + bytes.len())
            .ok_or(())?;
        dst.copy_from_slice(bytes);
        self.len += bytes.len();
        Ok(())
    }
}

impl DescriptorSink for ByteSink {
    type Error = ();

    fn interface(&mut self, f: &InterfaceFields) -> Result<(), ()> {
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

    fn endpoint(&mut self, f: &EndpointFields, extra: &[u8]) -> Result<(), ()> {
        let [lo, hi] = f.max_packet.to_le_bytes();
        let len = u8::try_from(7 + extra.len()).map_err(|_| ())?;
        self.put(&[len, ENDPOINT, f.address, f.attributes, lo, hi, f.interval])?;
        self.put(extra)
    }

    fn class_descriptor(&mut self, descriptor_type: u8, payload: &[u8]) -> Result<(), ()> {
        let len = u8::try_from(2 + payload.len()).map_err(|_| ())?;
        self.put(&[len, descriptor_type])?;
        self.put(payload)
    }
}

/// Checks the endpoint rules of DEC-015: at most one endpoint in each direction in each
/// Alternate Setting, each address in one setting only, and at most `EPS` addresses in each
/// direction.
fn check_endpoints<F: Function, const EPS: usize>(cfg: &F) -> Result<(), Error> {
    let mut counts = [0usize; 2];
    for (i, se) in cfg.endpoints().enumerate() {
        if let Some(other) = cfg
            .endpoints()
            .take(i)
            .find(|e| e.address() == se.address())
        {
            return Err(if other.alternate_setting == se.alternate_setting {
                Error::SettingEndpoints {
                    alt: se.alternate_setting,
                }
            } else {
                Error::SharedEndpoint {
                    address: se.address(),
                }
            });
        }
        let same_direction = cfg
            .endpoints()
            .take(i)
            .filter(|e| e.alternate_setting == se.alternate_setting && e.direction == se.direction)
            .count();
        if same_direction > 0 {
            return Err(Error::SettingEndpoints {
                alt: se.alternate_setting,
            });
        }
        let d = usize::from(se.direction == EndpointDirection::MidiIn);
        counts[d] += 1;
        if counts[d] > EPS {
            return Err(Error::TooManyEndpoints);
        }
    }
    Ok(())
}

/// Writes the Alternate Settings of interface `number` from `bytes` (the function that zmidi-usb
/// wrote) with the builder calls, and allocates their endpoints.
fn fill<'d, D: Driver<'d>, const EPS: usize>(
    ib: &mut InterfaceBuilder<'_, 'd, D>,
    bytes: &[u8],
    number: u8,
    outs: &mut [Option<(u8, D::EndpointOut)>; EPS],
    ins: &mut [Option<(u8, D::EndpointIn)>; EPS],
) -> Result<(), Error> {
    let desc = |at: usize| -> Result<Option<&[u8]>, Error> {
        let Some(&len) = bytes.get(at) else {
            return Ok(None);
        };
        bytes
            .get(at..at + usize::from(len))
            .filter(|d| d.len() >= 2)
            .map(Some)
            .ok_or(Error::Layout)
    };
    let mut at = 0;
    while let Some(d) = desc(at)? {
        at += d.len();
        // A standard interface descriptor of this interface: bInterfaceNumber, bAlternateSetting,
        // bNumEndpoints, class, subclass, protocol, iInterface.
        let &[_, t, n, alt_number, _, class, subclass, protocol, string, ..] = d else {
            continue;
        };
        if t != INTERFACE || n != number {
            continue;
        }
        let string = (string != 0).then_some(StringIndex(string));
        let mut alt = ib.alt_setting(class, subclass, protocol, string);
        if alt.alt_setting_number() != alt_number {
            return Err(Error::Layout);
        }
        while let Some(e) = desc(at)? {
            if e[1] == INTERFACE {
                break;
            }
            at += e.len();
            if e[1] != ENDPOINT {
                alt.descriptor(e[1], &e[2..]);
                continue;
            }
            let &[_, _, address, attributes, mps_lo, mps_hi, interval, ref extra @ ..] = e else {
                return Err(Error::Layout);
            };
            let ep_type = if attributes & 0x03 == 0x03 {
                EndpointType::Interrupt
            } else {
                EndpointType::Bulk
            };
            let max_packet = u16::from_le_bytes([mps_lo, mps_hi]);
            let addr = EndpointAddress::from(address);
            use embassy_usb::descriptor::{SynchronizationType, UsageType};
            let (sync, usage) = (
                SynchronizationType::NoSynchronization,
                UsageType::DataEndpoint,
            );
            if addr.is_in() {
                let slot = ins
                    .iter_mut()
                    .find(|s| s.is_none())
                    .ok_or(Error::TooManyEndpoints)?;
                let ep = alt.alloc_endpoint_in(ep_type, Some(addr), max_packet, interval);
                alt.endpoint_descriptor(ep.info(), sync, usage, extra);
                *slot = Some((alt_number, ep));
            } else {
                let slot = outs
                    .iter_mut()
                    .find(|s| s.is_none())
                    .ok_or(Error::TooManyEndpoints)?;
                let ep = alt.alloc_endpoint_out(ep_type, Some(addr), max_packet, interval);
                alt.endpoint_descriptor(ep.info(), sync, usage, extra);
                *slot = Some((alt_number, ep));
            }
        }
    }
    Ok(())
}

/// Waits until the endpoint of the selected Alternate Setting in `eps` is enabled.
async fn wait_enabled<E: Endpoint, M: RawMutex>(
    eps: &mut [Option<(u8, E)>],
    changed: &Signal<M, ()>,
    setting: impl Fn() -> u8,
) {
    loop {
        changed.reset();
        let alt = setting();
        match eps.iter_mut().flatten().find(|(a, _)| *a == alt) {
            Some((_, ep)) => {
                if let Either::First(()) = select(ep.wait_enabled(), changed.wait()).await {
                    if setting() == alt {
                        return;
                    }
                }
            }
            None => changed.wait().await,
        }
    }
}

/// The half that sends: the MIDI IN endpoint of the selected Alternate Setting.
pub struct Sender<
    'd,
    D: Driver<'d>,
    M: RawMutex,
    const CABLES: usize = 1,
    F: Function = FunctionConfig<'d>,
    const EPS: usize = 2,
    const X: usize = 0,
> {
    shared: &'d Shared<'d, M, CABLES, F, X>,
    eps: [Option<(u8, D::EndpointIn)>; EPS],
    /// The Alternate Setting of the last packet of exactly wMaxPacketSize: a zero-length
    /// packet ends that transfer.
    zlp: Option<u8>,
}

impl<
        'd,
        D: Driver<'d>,
        M: RawMutex,
        const CABLES: usize,
        F: Function,
        const EPS: usize,
        const X: usize,
    > Sender<'d, D, M, CABLES, F, EPS, X>
{
    /// The selected Alternate Setting.
    pub fn setting(&self) -> SettingNumber {
        SettingNumber(self.shared.setting())
    }

    /// The packet size of the link: the length that the buffer of [`Self::write_packet`] needs.
    pub fn packet_size(&self) -> usize {
        self.shared.with(|i| i.link.packet_size()).unwrap_or(0)
    }

    /// Waits until the MIDI IN endpoint of the selected Alternate Setting is enabled (the host
    /// configured the device and selected the setting).
    pub async fn wait_connection(&mut self) {
        let shared = self.shared;
        wait_enabled(&mut self.eps, &shared.tx_changed, || shared.setting()).await;
    }

    /// Writes one packet to the MIDI IN endpoint of the selected Alternate Setting from `umps`
    /// ([`MidiLink::write_packet`]) and gives its length; `buf` holds the packet and must have
    /// [`Self::packet_size`] bytes. A UMP that does not fit stays in `umps`.
    ///
    /// - After a packet of exactly wMaxPacketSize, a call with nothing to send writes a
    ///   zero-length packet, so that the host sees the end of the bulk transfer.
    /// - 0 with no packet when there is nothing to send.
    /// - [`EndpointError::Disabled`] when the host deselects the setting; the packet is lost,
    ///   as each held part is at a change of setting (REQ-USB-UMP-008 of zmidi-usb).
    pub async fn write_packet<'w, I>(
        &mut self,
        umps: &mut Peekable<I>,
        buf: &mut [u8],
    ) -> Result<usize, Error>
    where
        I: Iterator<Item = &'w [u32]>,
    {
        let alt = self.shared.setting();
        let (_, ep) = self
            .eps
            .iter_mut()
            .flatten()
            .find(|(a, _)| *a == alt)
            .ok_or(Error::NoEndpoint)?;
        let pending = self.shared.with(|i| i.link.pending()).unwrap_or(false);
        if self.zlp == Some(alt) && umps.peek().is_none() && !pending {
            self.zlp = None;
            ep.write(&[]).await?;
            return Ok(0);
        }
        let n = self
            .shared
            .with(|i| {
                if i.link.setting().0 == alt {
                    i.link.write_packet(umps, buf)
                } else {
                    Ok(0)
                }
            })
            .unwrap_or(Ok(0))?;
        if n == 0 {
            return Ok(0);
        }
        self.zlp = (n == usize::from(ep.info().max_packet_size)).then_some(alt);
        ep.write(buf.get(..n).unwrap_or(&[])).await?;
        Ok(n)
    }
}

/// The half that receives: the MIDI OUT endpoint of the selected Alternate Setting.
pub struct Receiver<
    'd,
    D: Driver<'d>,
    M: RawMutex,
    const CABLES: usize = 1,
    F: Function = FunctionConfig<'d>,
    const EPS: usize = 2,
    const X: usize = 0,
> {
    shared: &'d Shared<'d, M, CABLES, F, X>,
    eps: [Option<(u8, D::EndpointOut)>; EPS],
}

impl<
        'd,
        D: Driver<'d>,
        M: RawMutex,
        const CABLES: usize,
        F: Function,
        const EPS: usize,
        const X: usize,
    > Receiver<'d, D, M, CABLES, F, EPS, X>
{
    /// The selected Alternate Setting.
    pub fn setting(&self) -> SettingNumber {
        SettingNumber(self.shared.setting())
    }

    /// Waits until the MIDI OUT endpoint of the selected Alternate Setting is enabled (the
    /// host configured the device and selected the setting).
    pub async fn wait_connection(&mut self) {
        let shared = self.shared;
        wait_enabled(&mut self.eps, &shared.rx_changed, || shared.setting()).await;
    }

    /// Reads one packet from the MIDI OUT endpoint of the selected Alternate Setting into `buf`
    /// and gives each UMP of it to `sink` ([`MidiLink::read_with`]); gives the packet length.
    /// `buf` must hold wMaxPacketSize bytes. `sink` runs with the mutex held: keep it short
    /// (for example a `try_send` into a channel).
    ///
    /// A packet that arrives while the host changes the setting belongs to the old stream and
    /// is not decoded (REQ-USB-UMP-008 of zmidi-usb). [`EndpointError::Disabled`] when the host
    /// deselects the setting: call [`Self::wait_connection`] again.
    pub async fn read_packet(
        &mut self,
        buf: &mut [u8],
        sink: impl FnMut(Result<Received, LinkError>),
    ) -> Result<usize, Error> {
        let alt = self.shared.setting();
        let (_, ep) = self
            .eps
            .iter_mut()
            .flatten()
            .find(|(a, _)| *a == alt)
            .ok_or(Error::NoEndpoint)?;
        let n = ep.read(buf).await?;
        let data = buf.get(..n).unwrap_or(&[]);
        self.shared.with(|i| {
            if i.link.setting().0 == alt {
                i.link.read_with(data, sink);
            }
        });
        Ok(n)
    }
}

// The tests use Alternate Setting 0 with ports (feature `alt0`).
#[cfg(all(test, feature = "alt0"))]
mod tests;
