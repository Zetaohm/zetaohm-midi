//! Host tests of the embassy adapter with a fake `embassy-usb-driver` Driver (DEC-015
//! verification).

extern crate std;

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::vec;
use std::vec::Vec;

use embassy_futures::block_on;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_usb::control::{Recipient, RequestType};
use embassy_usb::driver::{Bus, ControlPipe, EndpointAllocError, EndpointInfo, Event, Unsupported};
use embassy_usb::Config;
use zmidi_usb::device::{gtb_set, write_function, Midi2Setting};
use zmidi_usb::EndpointConfig;

use super::*;

#[derive(Default)]
struct St {
    /// (address, type, wMaxPacketSize, interval) of each allocated endpoint.
    allocated: Vec<(u8, EndpointType, u16, u8)>,
    enabled: Vec<u8>,
    out_packets: HashMap<u8, VecDeque<Vec<u8>>>,
    written: Vec<(u8, Vec<u8>)>,
}

type Shared0 = Rc<RefCell<St>>;

struct FakeDriver(Shared0);
struct FakeOut(EndpointInfo, Shared0);
struct FakeIn(EndpointInfo, Shared0);
struct FakeBus;
struct FakePipe;

impl FakeDriver {
    fn alloc(
        &mut self,
        dir: Direction,
        ep_type: EndpointType,
        addr: Option<EndpointAddress>,
        max_packet_size: u16,
        interval_ms: u8,
    ) -> Result<EndpointInfo, EndpointAllocError> {
        let addr = addr.ok_or(EndpointAllocError)?;
        assert_eq!(addr.direction(), dir);
        let mut st = self.0.borrow_mut();
        if st.allocated.iter().any(|a| a.0 == u8::from(addr)) {
            return Err(EndpointAllocError);
        }
        st.allocated
            .push((u8::from(addr), ep_type, max_packet_size, interval_ms));
        Ok(EndpointInfo {
            addr,
            ep_type,
            max_packet_size,
            interval_ms,
        })
    }
}

impl<'a> Driver<'a> for FakeDriver {
    type EndpointOut = FakeOut;
    type EndpointIn = FakeIn;
    type ControlPipe = FakePipe;
    type Bus = FakeBus;

    fn alloc_endpoint_out(
        &mut self,
        ep_type: EndpointType,
        ep_addr: Option<EndpointAddress>,
        max_packet_size: u16,
        interval_ms: u8,
    ) -> Result<FakeOut, EndpointAllocError> {
        let info = self.alloc(
            Direction::Out,
            ep_type,
            ep_addr,
            max_packet_size,
            interval_ms,
        )?;
        Ok(FakeOut(info, self.0.clone()))
    }

    fn alloc_endpoint_in(
        &mut self,
        ep_type: EndpointType,
        ep_addr: Option<EndpointAddress>,
        max_packet_size: u16,
        interval_ms: u8,
    ) -> Result<FakeIn, EndpointAllocError> {
        let info = self.alloc(
            Direction::In,
            ep_type,
            ep_addr,
            max_packet_size,
            interval_ms,
        )?;
        Ok(FakeIn(info, self.0.clone()))
    }

    fn start(self, _control_max_packet_size: u16) -> (FakeBus, FakePipe) {
        (FakeBus, FakePipe)
    }
}

impl Endpoint for FakeOut {
    fn info(&self) -> &EndpointInfo {
        &self.0
    }
    async fn wait_enabled(&mut self) {
        assert!(self.1.borrow().enabled.contains(&u8::from(self.0.addr)));
    }
}

impl EndpointOut for FakeOut {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, EndpointError> {
        let mut st = self.1.borrow_mut();
        let a = u8::from(self.0.addr);
        if !st.enabled.contains(&a) {
            return Err(EndpointError::Disabled);
        }
        let p = st
            .out_packets
            .get_mut(&a)
            .and_then(|q| q.pop_front())
            .ok_or(EndpointError::Disabled)?;
        buf[..p.len()].copy_from_slice(&p);
        Ok(p.len())
    }
}

impl Endpoint for FakeIn {
    fn info(&self) -> &EndpointInfo {
        &self.0
    }
    async fn wait_enabled(&mut self) {
        assert!(self.1.borrow().enabled.contains(&u8::from(self.0.addr)));
    }
}

impl EndpointIn for FakeIn {
    async fn write(&mut self, buf: &[u8]) -> Result<(), EndpointError> {
        let mut st = self.1.borrow_mut();
        let a = u8::from(self.0.addr);
        if !st.enabled.contains(&a) {
            return Err(EndpointError::Disabled);
        }
        st.written.push((a, buf.to_vec()));
        Ok(())
    }
}

impl Bus for FakeBus {
    async fn enable(&mut self) {}
    async fn disable(&mut self) {}
    async fn poll(&mut self) -> Event {
        core::future::pending().await
    }
    fn endpoint_set_enabled(&mut self, _ep_addr: EndpointAddress, _enabled: bool) {}
    fn endpoint_set_stalled(&mut self, _ep_addr: EndpointAddress, _stalled: bool) {}
    fn endpoint_is_stalled(&mut self, _ep_addr: EndpointAddress) -> bool {
        false
    }
    async fn remote_wakeup(&mut self) -> Result<(), Unsupported> {
        Err(Unsupported)
    }
}

impl ControlPipe for FakePipe {
    fn max_packet_size(&self) -> usize {
        64
    }
    async fn setup(&mut self) -> [u8; 8] {
        core::future::pending().await
    }
    async fn data_out(&mut self, _: &mut [u8], _: bool, _: bool) -> Result<usize, EndpointError> {
        Err(EndpointError::Disabled)
    }
    async fn data_in(&mut self, _: &[u8], _: bool, _: bool) -> Result<(), EndpointError> {
        Err(EndpointError::Disabled)
    }
    async fn accept(&mut self) {}
    async fn reject(&mut self) {}
    async fn accept_set_address(&mut self, _addr: u8) {}
}

type TestClass<'d> = MidiClass<'d, FakeDriver, NoopRawMutex>;

/// Builds a device with one MIDI function and gives the Configuration descriptor and the
/// result of `MidiClass::new`.
fn build(
    cfg: FunctionConfig<'static>,
    names: InterfaceNames<'static>,
    iads: bool,
) -> (Vec<u8>, Result<(), Error>, Shared0) {
    let st: Shared0 = Rc::default();
    let mut config_buf = [0u8; 512];
    let mut bos_buf = [0u8; 64];
    let mut msos_buf = [0u8; 0];
    let mut control_buf = [0u8; 64];
    let mut state = State::<NoopRawMutex>::new();
    let mut config = Config::new(0x1209, 0x0001);
    if !iads {
        config.composite_with_iads = false;
        config.device_class = 0;
        config.device_sub_class = 0;
        config.device_protocol = 0;
    }
    let result = {
        let mut builder = Builder::new(
            FakeDriver(st.clone()),
            config,
            &mut config_buf,
            &mut bos_buf,
            &mut msos_buf,
            &mut control_buf,
        );
        let r: Result<TestClass, Error> =
            MidiClass::new(&mut builder, &mut state, cfg, names, LinkConfig::default());
        let r = r.map(drop);
        drop(builder.build());
        r
    };
    let total = usize::from(u16::from_le_bytes([config_buf[2], config_buf[3]]));
    (config_buf[..total].to_vec(), result, st)
}

/// [`build`] for a fixed function.
fn build_fixed(
    function: &'static zmidi_usb::fixed::FixedFunction,
    names: InterfaceNames<'static>,
    iads: bool,
) -> (Vec<u8>, Result<(), Error>) {
    let st: Shared0 = Rc::default();
    let mut config_buf = [0u8; 512];
    let mut bos_buf = [0u8; 64];
    let mut msos_buf = [0u8; 0];
    let mut control_buf = [0u8; 64];
    let mut state = State::<NoopRawMutex, 1, Fixed>::new();
    let mut config = Config::new(0x1209, 0x0001);
    if !iads {
        config.composite_with_iads = false;
        config.device_class = 0;
        config.device_sub_class = 0;
        config.device_protocol = 0;
    }
    let result = {
        let mut builder = Builder::new(
            FakeDriver(st.clone()),
            config,
            &mut config_buf,
            &mut bos_buf,
            &mut msos_buf,
            &mut control_buf,
        );
        let r: Result<MidiClass<'_, FakeDriver, NoopRawMutex, 1, Fixed>, Error> = MidiClass::fixed(
            &mut builder,
            &mut state,
            function,
            names,
            LinkConfig::default(),
        );
        let r = r.map(drop);
        drop(builder.build());
        r
    };
    let total = usize::from(u16::from_le_bytes([config_buf[2], config_buf[3]]));
    (config_buf[..total].to_vec(), result)
}

mod fixed_default {
    include!("fixtures/default_fn.rs");
}

#[test]
fn fixture_is_the_output_of_write_rust() {
    // REQ-USB-FIX-001 of zmidi-usb, for default_config. To make it again:
    // UPDATE_FIXTURES=1 cargo test -p zmidi-usb-embassy fixture_is_the_output_of_write_rust
    let mut text = std::string::String::new();
    zmidi_usb::fixed::write_rust(&default_config(BusSpeed::Full).unwrap(), "MIDI", &mut text)
        .unwrap();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fixtures/default_fn.rs");
    if std::env::var_os("UPDATE_FIXTURES").is_some() {
        std::fs::write(&path, &text).unwrap();
    }
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
}

#[test]
fn fixed_function_gives_the_configuration_of_the_run_time_function() {
    // DEC-USB-002 of zmidi-usb: the same Configuration descriptor, with and without IADs and
    // strings, from MidiClass::fixed as from MidiClass::new.
    let names = InterfaceNames {
        audio_control: Some("Audio"),
        streaming: &[Some("MIDI 1.0"), Some("MIDI 2.0")],
    };
    for iads in [false, true] {
        for n in [InterfaceNames::default(), names] {
            let (runtime, r, _) = build(default_config(BusSpeed::Full).unwrap(), n, iads);
            let (fixed, f) = build_fixed(&fixed_default::MIDI, n, iads);
            r.unwrap();
            f.unwrap();
            assert!(runtime.len() > 100);
            assert_eq!(fixed, runtime, "iads {iads}");
        }
    }
}

fn function_bytes(cfg: &FunctionConfig<'_>) -> Vec<u8> {
    let mut buf = [0u8; 512];
    let n = write_function(cfg, &mut buf).unwrap();
    buf[..n].to_vec()
}

#[test]
fn default_config_has_distinct_endpoint_pairs() {
    let cfg = default_config(BusSpeed::Full).unwrap();
    let eps: Vec<(u8, u8)> = cfg
        .endpoints()
        .map(|e| (e.alternate_setting, e.address()))
        .collect();
    assert_eq!(eps, [(0, 0x01), (0, 0x81), (1, 0x02), (1, 0x82)]);
}

#[test]
fn descriptors_equal_write_function() {
    // AudioControl interface 0, MIDI Streaming interface 1: the numbers of a builder with no
    // other class.
    let mut expected_cfg = default_config(BusSpeed::Full).unwrap();
    expected_cfg.ac_interface = 0;
    expected_cfg.ms_interface = 1;
    let function = function_bytes(&expected_cfg);
    assert_eq!(function.len(), 132);

    let (bytes, r, st) = build(
        default_config(BusSpeed::Full).unwrap(),
        InterfaceNames::default(),
        false,
    );
    r.unwrap();
    assert_eq!(bytes[1], 0x02);
    assert_eq!(bytes[4], 2, "bNumInterfaces");
    assert_eq!(bytes[9..], function[..]);
    let allocated: Vec<_> = st.borrow().allocated.clone();
    assert_eq!(
        allocated,
        [
            (0x01, EndpointType::Bulk, 64, 0),
            (0x81, EndpointType::Bulk, 64, 0),
            (0x02, EndpointType::Bulk, 64, 0),
            (0x82, EndpointType::Bulk, 64, 0),
        ]
    );

    // With the Interface Association Descriptor of the builder.
    let (bytes, r, _) = build(
        default_config(BusSpeed::Full).unwrap(),
        InterfaceNames::default(),
        true,
    );
    r.unwrap();
    assert_eq!(
        bytes[9..17],
        [0x08, 0x0B, 0x00, 0x02, 0x01, 0x01, 0x00, 0x00]
    );
    assert_eq!(bytes[17..], function[..]);
}

#[test]
fn interrupt_endpoint_and_strings() {
    let mut cfg = FunctionConfig::<'static>::midi1_only(0, 1, BusSpeed::Full);
    let mut s = Midi2Setting::empty();
    s.endpoints
        .push(EndpointConfig::bulk(
            EndpointDirection::MidiOut,
            2,
            BusSpeed::Full,
        ))
        .unwrap();
    s.endpoints
        .push(EndpointConfig::interrupt(
            EndpointDirection::MidiIn,
            2,
            64,
            1,
        ))
        .unwrap();
    cfg.add_midi2_setting(s).unwrap();
    let names = InterfaceNames {
        audio_control: Some("Audio"),
        streaming: &[Some("MIDI 1.0"), Some("MIDI 2.0")],
    };
    let (bytes, r, st) = build(cfg.clone(), names, false);
    r.unwrap();
    // The builder gives string indexes 4, 5 and 6 (the first custom index of embassy-usb).
    cfg.ac_string = 4;
    cfg.midi1.as_mut().unwrap().string_index = 5;
    cfg.midi2.as_mut_slice()[0].string_index = 6;
    assert_eq!(bytes[9..], function_bytes(&cfg)[..]);
    assert!(st
        .borrow()
        .allocated
        .contains(&(0x82, EndpointType::Interrupt, 64, 1)));
}

#[test]
fn shared_endpoint_is_refused() {
    // The zmidi-usb default shares OUT 1 and IN 1 between Alternate Settings 0 and 1.
    let cfg = FunctionConfig::new(0, 1, BusSpeed::Full).unwrap();
    let (_, r, st) = build(cfg, InterfaceNames::default(), false);
    assert_eq!(r, Err(Error::SharedEndpoint { address: 0x01 }));
    assert!(st.borrow().allocated.is_empty());
}

#[test]
fn too_many_names_are_refused() {
    let names = InterfaceNames {
        audio_control: None,
        streaming: &[None, None, Some("no setting 2")],
    };
    let (_, r, _) = build(default_config(BusSpeed::Full).unwrap(), names, false);
    assert_eq!(r, Err(Error::TooManyNames));
}

/// A run-time state with the default configuration, interface 1 as MIDI Streaming.
fn shared() -> &'static Shared<'static, NoopRawMutex, 1, FunctionConfig<'static>, 0> {
    let mut inner = Inner {
        cfg: default_config(BusSpeed::Full).unwrap(),
        link: MidiLink::new(LinkConfig::default()).unwrap(),
    };
    inner.select(0);
    std::boxed::Box::leak(std::boxed::Box::new(Shared {
        inner: Mutex::new(RefCell::new(Slot::Full(inner))),
        rx_changed: Signal::new(),
        tx_changed: Signal::new(),
        _function: core::marker::PhantomData,
    }))
}

fn request(request_type: u8, request: u8, value: u16, index: u16, length: u16) -> Request {
    Request {
        direction: if request_type & 0x80 != 0 {
            Direction::In
        } else {
            Direction::Out
        },
        request_type: match (request_type >> 5) & 3 {
            0 => RequestType::Standard,
            1 => RequestType::Class,
            2 => RequestType::Vendor,
            _ => RequestType::Reserved,
        },
        recipient: match request_type & 0x1F {
            0 => Recipient::Device,
            1 => Recipient::Interface,
            2 => Recipient::Endpoint,
            3 => Recipient::Other,
            _ => Recipient::Reserved,
        },
        request,
        value,
        index,
        length,
    }
}

#[test]
fn handler_answers_gtb_request_and_strings() {
    let shared = shared();
    let mut control = Control {
        shared,
        ms_if: 1,
        names: [Some((4, "Audio")), None, None, None, None, None, None, None],
    };
    let mut set = [0u8; 64];
    let n = shared
        .with(|i| gtb_set(&i.cfg, 1, &mut set))
        .unwrap()
        .unwrap();
    let mut buf = [0u8; 64];
    match control.control_in(request(0x81, 0x06, 0x2601, 1, 0xFF), &mut buf) {
        Some(InResponse::Accepted(b)) => assert_eq!(b, &set[..n]),
        _ => panic!("no answer"),
    }
    let mut buf = [0u8; 64];
    assert!(matches!(
        control.control_in(request(0x81, 0x06, 0x2600, 1, 0xFF), &mut buf),
        Some(InResponse::Rejected)
    ));
    let mut buf = [0u8; 64];
    assert!(control
        .control_in(request(0x80, 0x06, 0x0200, 0, 0xFF), &mut buf)
        .is_none());
    assert_eq!(control.get_string(StringIndex(4), 0x0409), Some("Audio"));
    assert_eq!(control.get_string(StringIndex(5), 0x0409), None);
}

#[test]
fn handler_reports_events_to_the_link() {
    let shared = shared();
    let mut control = Control {
        shared,
        ms_if: 1,
        names: [None; MAX_NAMES],
    };
    control.set_alternate_setting(InterfaceNumber(1), 1);
    assert_eq!(shared.setting(), 1);
    assert!(shared.rx_changed.signaled() && shared.tx_changed.signaled());
    // Another interface does not change the setting.
    control.set_alternate_setting(InterfaceNumber(0), 0);
    assert_eq!(shared.setting(), 1);
    control.reset();
    assert_eq!(shared.setting(), 0);
    control.set_alternate_setting(InterfaceNumber(1), 1);
    control.configured(true);
    assert_eq!(shared.setting(), 0);
}

/// The endpoints of each Alternate Setting: (bAlternateSetting, endpoint).
type Endpoints<T> = [Option<(u8, T)>; 2];

fn endpoints(st: &Shared0) -> (Endpoints<FakeOut>, Endpoints<FakeIn>) {
    let info = |a: u8| EndpointInfo {
        addr: EndpointAddress::from(a),
        ep_type: EndpointType::Bulk,
        max_packet_size: 64,
        interval_ms: 0,
    };
    (
        [
            Some((0, FakeOut(info(0x01), st.clone()))),
            Some((1, FakeOut(info(0x02), st.clone()))),
        ],
        [
            Some((0, FakeIn(info(0x81), st.clone()))),
            Some((1, FakeIn(info(0x82), st.clone()))),
        ],
    )
}

#[test]
fn data_path_follows_the_setting() {
    let shared = shared();
    let st: Shared0 = Rc::default();
    let (outs, ins) = endpoints(&st);
    let mut rx: Receiver<'_, FakeDriver, NoopRawMutex> = Receiver { shared, eps: outs };
    let mut tx: Sender<'_, FakeDriver, NoopRawMutex> = Sender {
        shared,
        eps: ins,
        zlp: None,
    };
    let mut control = Control {
        shared,
        ms_if: 1,
        names: [None; MAX_NAMES],
    };
    st.borrow_mut().enabled = vec![0x01, 0x81];
    block_on(rx.wait_connection());
    block_on(tx.wait_connection());
    let mut buf = [0u8; 64];

    // Alternate Setting 0: an Event Packet on OUT 1 (CIN 0x9 Note On, cable 0).
    st.borrow_mut()
        .out_packets
        .entry(0x01)
        .or_default()
        .push_back(vec![0x09, 0x90, 0x3C, 0x7F]);
    let mut got = Vec::new();
    assert_eq!(
        block_on(rx.read_packet(&mut buf, |r| got.push(r.unwrap().words()))),
        Ok(4)
    );
    assert_eq!(got, [([0x2090_3C7F, 0, 0, 0], 1)]);
    // A UMP goes out on IN 1 as an Event Packet.
    let words = [0x2080_3C00u32];
    let mut umps = core::iter::once(&words[..]).peekable();
    assert_eq!(block_on(tx.write_packet(&mut umps, &mut buf)), Ok(4));
    assert_eq!(
        st.borrow().written.last().unwrap(),
        &(0x81, vec![0x08, 0x80, 0x3C, 0x00])
    );

    // SET_INTERFACE 1: the pair OUT 2 / IN 2 and UMP words, least significant byte first.
    control.set_alternate_setting(InterfaceNumber(1), 1);
    st.borrow_mut().enabled = vec![0x02, 0x82];
    assert_eq!(rx.setting(), SettingNumber(1));
    st.borrow_mut()
        .out_packets
        .entry(0x02)
        .or_default()
        .push_back(0x2090_3C7Fu32.to_le_bytes().to_vec());
    let mut got = Vec::new();
    assert_eq!(
        block_on(rx.read_packet(&mut buf, |r| got.push(r.unwrap().words()))),
        Ok(4)
    );
    assert_eq!(got, [([0x2090_3C7F, 0, 0, 0], 1)]);
    let mut umps = core::iter::once(&words[..]).peekable();
    assert_eq!(block_on(tx.write_packet(&mut umps, &mut buf)), Ok(4));
    assert_eq!(
        st.borrow().written.last().unwrap(),
        &(0x82, 0x2080_3C00u32.to_le_bytes().to_vec())
    );

    // The old pair is disabled: a read on it is not possible.
    control.set_alternate_setting(InterfaceNumber(1), 0);
    assert_eq!(
        block_on(rx.read_packet(&mut buf, |_| {})),
        Err(Error::Endpoint(EndpointError::Disabled))
    );
}

#[test]
fn full_packet_is_followed_by_a_zero_length_packet() {
    let shared = shared();
    let st: Shared0 = Rc::default();
    let (_, ins) = endpoints(&st);
    let mut tx: Sender<'_, FakeDriver, NoopRawMutex> = Sender {
        shared,
        eps: ins,
        zlp: None,
    };
    let mut control = Control {
        shared,
        ms_if: 1,
        names: [None; MAX_NAMES],
    };
    control.set_alternate_setting(InterfaceNumber(1), 1);
    st.borrow_mut().enabled = vec![0x82];
    assert_eq!(tx.packet_size(), 64);
    let mut buf = [0u8; 64];
    let words: Vec<[u32; 1]> = (0..16).map(|i| [0x2090_3C00 | i]).collect();
    let mut umps = words.iter().map(|w| &w[..]).peekable();
    assert_eq!(block_on(tx.write_packet(&mut umps, &mut buf)), Ok(64));
    let mut none = core::iter::empty::<&[u32]>().peekable();
    assert_eq!(block_on(tx.write_packet(&mut none, &mut buf)), Ok(0));
    assert_eq!(st.borrow().written.last().unwrap(), &(0x82, Vec::new()));
    // Only one zero-length packet.
    let count = st.borrow().written.len();
    assert_eq!(block_on(tx.write_packet(&mut none, &mut buf)), Ok(0));
    assert_eq!(st.borrow().written.len(), count);
}
