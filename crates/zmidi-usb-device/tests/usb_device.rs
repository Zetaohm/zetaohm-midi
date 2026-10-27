//! Host tests of the usb-device adapter with a fake `UsbBus` (DEC-015 verification).
//! They use Alternate Setting 0 with ports (feature `alt0`).
#![cfg(feature = "alt0")]

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};

use usb_device::bus::PollResult;
use usb_device::class_prelude::*;
use usb_device::device::{StringDescriptors, UsbDevice, UsbDeviceBuilder, UsbVidPid};
use usb_device::endpoint::{EndpointAddress, EndpointType};
use usb_device::{UsbDirection, UsbError};
use zmidi_usb::device::{gtb_set, write_function, FunctionConfig, Midi2Setting};
use zmidi_usb::{BusSpeed, EndpointConfig, EndpointDirection, LinkConfig};
use zmidi_usb_device::{Error, Fixed, Function, InterfaceNames, MidiClass};

/// USB-MIDI v1.0 Appendix B, Tables B-3 to B-14: 92 bytes (the same vector as zmidi-usb).
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

#[derive(Default)]
struct State {
    allocated: Vec<u8>,
    /// EP0 OUT: (is a SETUP packet, bytes).
    ep0_out: VecDeque<(bool, Vec<u8>)>,
    out_packets: HashMap<u8, VecDeque<Vec<u8>>>,
    written: Vec<(u8, Vec<u8>)>,
    in_complete: u16,
    /// When true, an IN packet of a data endpoint does not complete by itself.
    hold_in: bool,
    stalled: Vec<u8>,
}

/// The fake bus; the test keeps a second handle to the same state.
#[derive(Default, Clone)]
struct FakeBus {
    st: Arc<Mutex<State>>,
}

impl UsbBus for FakeBus {
    fn alloc_ep(
        &mut self,
        dir: UsbDirection,
        addr: Option<EndpointAddress>,
        _ep_type: EndpointType,
        _max_packet_size: u16,
        _interval: u8,
    ) -> usb_device::Result<EndpointAddress> {
        let mut st = self.st.lock().unwrap();
        let addr = addr.unwrap_or_else(|| EndpointAddress::from_parts(0, dir));
        let a = u8::from(addr);
        if st.allocated.contains(&a) {
            return Err(UsbError::InvalidEndpoint);
        }
        st.allocated.push(a);
        Ok(addr)
    }
    fn enable(&mut self) {}
    fn reset(&self) {}
    fn set_device_address(&self, _addr: u8) {}
    fn write(&self, ep: EndpointAddress, buf: &[u8]) -> usb_device::Result<usize> {
        let mut st = self.st.lock().unwrap();
        let a = u8::from(ep);
        st.written.push((a, buf.to_vec()));
        if ep.index() == 0 || !st.hold_in {
            st.in_complete |= 1 << ep.index();
        }
        Ok(buf.len())
    }
    fn read(&self, ep: EndpointAddress, buf: &mut [u8]) -> usb_device::Result<usize> {
        let mut st = self.st.lock().unwrap();
        let data = if ep.index() == 0 {
            st.ep0_out.pop_front().map(|(_, d)| d)
        } else {
            st.out_packets
                .get_mut(&u8::from(ep))
                .and_then(|q| q.pop_front())
        };
        let data = data.ok_or(UsbError::WouldBlock)?;
        buf[..data.len()].copy_from_slice(&data);
        Ok(data.len())
    }
    fn set_stalled(&self, ep: EndpointAddress, stalled: bool) {
        let mut st = self.st.lock().unwrap();
        let a = u8::from(ep);
        st.stalled.retain(|x| *x != a);
        if stalled {
            st.stalled.push(a);
        }
    }
    fn is_stalled(&self, ep: EndpointAddress) -> bool {
        self.st.lock().unwrap().stalled.contains(&u8::from(ep))
    }
    fn suspend(&self) {}
    fn resume(&self) {}
    fn poll(&self) -> PollResult {
        let mut st = self.st.lock().unwrap();
        let mut ep_setup = 0;
        let mut ep_out = 0;
        match st.ep0_out.front() {
            Some((true, _)) => ep_setup |= 1,
            Some((false, _)) => ep_out |= 1,
            None => {}
        }
        for (a, q) in &st.out_packets {
            if !q.is_empty() {
                ep_out |= 1 << (a & 0x0F);
            }
        }
        let ep_in_complete = core::mem::take(&mut st.in_complete);
        if ep_setup | ep_out | ep_in_complete == 0 {
            return PollResult::None;
        }
        PollResult::Data {
            ep_out,
            ep_in_complete,
            ep_setup,
        }
    }
}

type Class<'a> = MidiClass<'a, FakeBus>;

/// The allocator and a handle to the state of its bus.
struct Bus {
    alloc: UsbBusAllocator<FakeBus>,
    st: Arc<Mutex<State>>,
}

fn alloc() -> Bus {
    let bus = FakeBus::default();
    let st = bus.st.clone();
    Bus {
        alloc: UsbBusAllocator::new(bus),
        st,
    }
}

fn device(a: &Bus, iads: bool) -> UsbDevice<'_, FakeBus> {
    let b = UsbDeviceBuilder::new(&a.alloc, UsbVidPid(0x1209, 0x0001))
        .strings(&[StringDescriptors::default().product("test")])
        .unwrap()
        .max_packet_size_0(64)
        .unwrap();
    if iads { b.composite_with_iads() } else { b }.build()
}

fn bus(a: &Bus) -> MutexGuard<'_, State> {
    a.st.lock().unwrap()
}

/// Runs one control transfer and gives the data of the IN data stage, or `None` on a stall.
fn control<F: Function>(
    a: &Bus,
    dev: &mut UsbDevice<'_, FakeBus>,
    class: &mut MidiClass<'_, FakeBus, F>,
    setup: [u8; 8],
) -> Option<Vec<u8>> {
    {
        let mut st = bus(a);
        st.written.clear();
        st.stalled.clear();
        st.ep0_out.push_back((true, setup.to_vec()));
    }
    for _ in 0..64 {
        dev.poll(&mut [class]);
    }
    let device_to_host = setup[0] & 0x80 != 0;
    let (data, stalled) = {
        let st = bus(a);
        let data: Vec<u8> = st
            .written
            .iter()
            .filter(|(e, _)| *e == 0x80)
            .flat_map(|(_, d)| d.clone())
            .collect();
        (
            data,
            st.stalled.contains(&0x80) || st.stalled.contains(&0x00),
        )
    };
    if device_to_host && !stalled {
        // Status stage: a zero-length OUT packet from the host.
        bus(a).ep0_out.push_back((false, Vec::new()));
        for _ in 0..8 {
            dev.poll(&mut [class]);
        }
    }
    (!stalled).then_some(data)
}

fn get_configuration<F: Function>(
    a: &Bus,
    dev: &mut UsbDevice<'_, FakeBus>,
    class: &mut MidiClass<'_, FakeBus, F>,
) -> Vec<u8> {
    control(
        a,
        dev,
        class,
        [0x80, 0x06, 0x00, 0x02, 0x00, 0x00, 0xFF, 0x03],
    )
    .unwrap()
}

fn written_on(a: &Bus, ep: u8) -> Vec<Vec<u8>> {
    bus(a)
        .written
        .iter()
        .filter(|(e, _)| *e == ep)
        .map(|(_, d)| d.clone())
        .collect()
}

/// The function with the interface numbers that the allocator gives (AudioControl 0, MIDI
/// Streaming 1).
fn expected<const S: usize>(cfg: &FunctionConfig<'_, 4, S, 2, 1>) -> Vec<u8> {
    let mut buf = [0u8; 1024];
    let n = write_function(cfg, &mut buf).unwrap();
    buf[..n].to_vec()
}

#[test]
fn midi1_only_function_is_appendix_b_v1() {
    let a = alloc();
    let cfg = FunctionConfig::midi1_only(0, 0, BusSpeed::Full);
    let mut class: Class = MidiClass::new(
        &a.alloc,
        cfg,
        InterfaceNames::default(),
        LinkConfig::default(),
    )
    .unwrap();
    let mut dev = device(&a, false);
    let bytes = get_configuration(&a, &mut dev, &mut class);
    assert_eq!(bytes[..4], [0x09, 0x02, 101, 0x00], "wTotalLength = 9 + 92");
    assert_eq!(bytes[4], 2, "bNumInterfaces");
    assert_eq!(bytes[9..], V1_FUNCTION);
}

#[test]
fn default_function_equals_write_function_with_iad() {
    let a = alloc();
    let cfg = FunctionConfig::new(0, 0, BusSpeed::Full).unwrap();
    let mut class: Class = MidiClass::new(
        &a.alloc,
        cfg,
        InterfaceNames::default(),
        LinkConfig::default(),
    )
    .unwrap();
    let mut dev = device(&a, true);
    let bytes = get_configuration(&a, &mut dev, &mut class);
    let function = expected(class.config());
    assert_eq!(function.len(), 132);
    assert_eq!(
        bytes[9..17],
        [0x08, 0x0B, 0x00, 0x02, 0x01, 0x01, 0x00, 0x00],
        "IAD"
    );
    assert_eq!(bytes[17..], function[..]);
    assert_eq!(
        usize::from(u16::from_le_bytes([bytes[2], bytes[3]])),
        9 + 8 + 132
    );
}

#[test]
fn appendix_b_v2_example_is_refused() {
    // IN 1 is bulk in Alternate Setting 0 and interrupt in Alternate Setting 1: one usb-device
    // endpoint has one type (DEC-015).
    let a = alloc();
    let mut cfg = FunctionConfig::<'static, 4, 1, 2, 1>::midi1_only(0, 0, BusSpeed::Full);
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
    cfg.add_midi2_setting(s).unwrap();
    let r: Result<Class, _> = MidiClass::new(
        &a.alloc,
        cfg,
        InterfaceNames::default(),
        LinkConfig::default(),
    );
    assert_eq!(r.err(), Some(Error::EndpointMismatch { address: 0x81 }));
}

#[test]
fn gtb_request_is_answered_or_stalled() {
    let a = alloc();
    let cfg = FunctionConfig::new(0, 0, BusSpeed::Full).unwrap();
    let mut class: Class = MidiClass::new(
        &a.alloc,
        cfg,
        InterfaceNames::default(),
        LinkConfig::default(),
    )
    .unwrap();
    let mut dev = device(&a, false);
    let mut set = [0u8; 64];
    let n = gtb_set(class.config(), 1, &mut set).unwrap();
    // GET_DESCRIPTOR, Group Terminal Block (0x26), Alternate Setting 1, interface 1.
    let answer = control(
        &a,
        &mut dev,
        &mut class,
        [0x81, 0x06, 0x01, 0x26, 0x01, 0x00, 0xFF, 0x00],
    );
    assert_eq!(answer.as_deref(), Some(&set[..n]));
    // wLength 5: the header only.
    let answer = control(
        &a,
        &mut dev,
        &mut class,
        [0x81, 0x06, 0x01, 0x26, 0x01, 0x00, 0x05, 0x00],
    );
    assert_eq!(answer.as_deref(), Some(&set[..5]));
    // Alternate Setting 0 and another interface: stall.
    assert_eq!(
        control(
            &a,
            &mut dev,
            &mut class,
            [0x81, 0x06, 0x00, 0x26, 0x01, 0x00, 0xFF, 0x00]
        ),
        None
    );
    assert_eq!(
        control(
            &a,
            &mut dev,
            &mut class,
            [0x81, 0x06, 0x01, 0x26, 0x00, 0x00, 0xFF, 0x00]
        ),
        None
    );
}

#[test]
fn interface_names_get_string_indexes() {
    let a = alloc();
    let cfg = FunctionConfig::new(0, 0, BusSpeed::Full).unwrap();
    let names = InterfaceNames {
        audio_control: Some("Audio"),
        streaming: &[None, Some("MIDI 2.0")],
    };
    let mut class: Class = MidiClass::new(&a.alloc, cfg, names, LinkConfig::default()).unwrap();
    let ac = class.config().ac_string;
    let alt1 = class.config().midi2_setting(1).unwrap().string_index;
    assert!(ac != 0 && alt1 != 0 && ac != alt1);
    assert_eq!(class.config().midi1.as_ref().unwrap().string_index, 0);
    let mut dev = device(&a, false);
    let bytes = get_configuration(&a, &mut dev, &mut class);
    assert_eq!(bytes[9..], expected(class.config())[..]);
    let utf16 = |s: &str| -> Vec<u8> {
        let mut v = vec![(2 + 2 * s.len()) as u8, 0x03];
        v.extend(s.encode_utf16().flat_map(u16::to_le_bytes));
        v
    };
    let get = |dev: &mut UsbDevice<'_, FakeBus>, class: &mut Class<'_>, i: u8| {
        control(
            &a,
            dev,
            class,
            [0x80, 0x06, i, 0x03, 0x09, 0x04, 0xFF, 0x00],
        )
    };
    assert_eq!(get(&mut dev, &mut class, ac), Some(utf16("Audio")));
    assert_eq!(get(&mut dev, &mut class, alt1), Some(utf16("MIDI 2.0")));
}

#[test]
fn too_many_names_are_refused() {
    let a = alloc();
    let cfg = FunctionConfig::new(0, 0, BusSpeed::Full).unwrap();
    let names = InterfaceNames {
        audio_control: None,
        streaming: &[None, None, Some("no setting 2")],
    };
    let r: Result<Class, _> = MidiClass::new(&a.alloc, cfg, names, LinkConfig::default());
    assert_eq!(r.err(), Some(Error::TooManyNames));
}

/// SET_INTERFACE for the MIDI Streaming interface (1).
fn set_interface(
    a: &Bus,
    dev: &mut UsbDevice<'_, FakeBus>,
    class: &mut Class<'_>,
    alt: u8,
) -> bool {
    control(
        a,
        dev,
        class,
        [0x01, 0x0B, alt, 0x00, 0x01, 0x00, 0x00, 0x00],
    )
    .is_some()
}

#[test]
fn alternate_setting_selects_the_transport() {
    let a = alloc();
    let cfg = FunctionConfig::new(0, 0, BusSpeed::Full).unwrap();
    let mut class: Class = MidiClass::new(
        &a.alloc,
        cfg,
        InterfaceNames::default(),
        LinkConfig::default(),
    )
    .unwrap();
    let mut dev = device(&a, false);
    let mut buf = [0u8; 64];

    // Alternate Setting 0: Event Packets. CIN 0x9 Note On, cable 0.
    bus(&a)
        .out_packets
        .entry(0x01)
        .or_default()
        .push_back(vec![0x09, 0x90, 0x3C, 0x7F]);
    let mut got = Vec::new();
    class
        .read(&mut buf, |r| got.push(r.unwrap().words()))
        .unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].0[0], 0x2090_3C7F);

    // GET_INTERFACE gives 0; SET_INTERFACE 1 selects USB MIDI 2.0.
    assert_eq!(
        control(&a, &mut dev, &mut class, [0x81, 0x0A, 0, 0, 0x01, 0, 1, 0]),
        Some(vec![0])
    );
    assert!(set_interface(&a, &mut dev, &mut class, 1));
    assert_eq!(class.link().setting().0, 1);
    assert_eq!(
        control(&a, &mut dev, &mut class, [0x81, 0x0A, 0, 0, 0x01, 0, 1, 0]),
        Some(vec![1])
    );
    // A setting that the function does not have is stalled.
    assert!(!set_interface(&a, &mut dev, &mut class, 2));

    // UMP words, least significant byte first (USB-MIDI v2.0 3.2.2).
    bus(&a)
        .out_packets
        .entry(0x01)
        .or_default()
        .push_back(0x2090_3C7Fu32.to_le_bytes().to_vec());
    let mut got = Vec::new();
    class
        .read(&mut buf, |r| got.push(r.unwrap().words()))
        .unwrap();
    assert_eq!(got, [([0x2090_3C7F, 0, 0, 0], 1)]);

    let words = [0x2080_3C00u32];
    let mut umps = core::iter::once(&words[..]).peekable();
    assert_eq!(class.write(&mut umps, &mut buf), Ok(4));
    assert_eq!(
        written_on(&a, 0x81).last().unwrap(),
        &0x2080_3C00u32.to_le_bytes().to_vec()
    );

    // A USB reset and SET_CONFIGURATION go back to Alternate Setting 0.
    dev.poll(&mut [&mut class]);
    usb_device::class::UsbClass::reset(&mut class);
    assert_eq!(class.link().setting().0, 0);
    assert!(set_interface(&a, &mut dev, &mut class, 1));
    assert!(control(
        &a,
        &mut dev,
        &mut class,
        [0x00, 0x09, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00]
    )
    .is_some());
    assert_eq!(class.link().setting().0, 0);
}

#[test]
fn write_waits_for_the_in_endpoint_and_ends_full_packets() {
    let a = alloc();
    let cfg = FunctionConfig::new(0, 0, BusSpeed::Full).unwrap();
    let mut class: Class = MidiClass::new(
        &a.alloc,
        cfg,
        InterfaceNames::default(),
        LinkConfig::default(),
    )
    .unwrap();
    let mut dev = device(&a, false);
    assert!(set_interface(&a, &mut dev, &mut class, 1));
    bus(&a).hold_in = true;
    let mut buf = [0u8; 64];

    // 16 one-word UMPs fill one 64-byte packet.
    let words: Vec<[u32; 1]> = (0..16).map(|i| [0x2090_3C00 | i]).collect();
    let mut umps = words.iter().map(|w| &w[..]).peekable();
    assert_eq!(class.write(&mut umps, &mut buf), Ok(64));
    // The packet is not sent yet: nothing is taken.
    let more = [0x2080_3C00u32];
    let mut next = core::iter::once(&more[..]).peekable();
    assert_eq!(
        class.write(&mut next, &mut buf),
        Err(Error::Usb(UsbError::WouldBlock))
    );
    assert!(next.peek().is_some());

    // The IN packet completes: a call with nothing to send writes a zero-length packet.
    bus(&a).in_complete |= 1 << 1;
    dev.poll(&mut [&mut class]);
    let mut none = core::iter::empty::<&[u32]>().peekable();
    assert_eq!(class.write(&mut none, &mut buf), Ok(0));
    assert_eq!(written_on(&a, 0x81).last().unwrap(), &Vec::<u8>::new());
    // Then the next UMP.
    bus(&a).in_complete |= 1 << 1;
    dev.poll(&mut [&mut class]);
    assert_eq!(class.write(&mut next, &mut buf), Ok(4));
    // Nothing to send and no full packet: no packet.
    bus(&a).in_complete |= 1 << 1;
    dev.poll(&mut [&mut class]);
    let count = written_on(&a, 0x81).len();
    assert_eq!(class.write(&mut none, &mut buf), Ok(0));
    assert_eq!(written_on(&a, 0x81).len(), count);
}

mod fixed_default {
    include!("../../zmidi-usb/tests/fixtures/default_fn.rs");
}

#[test]
fn fixed_function_answers_as_the_run_time_function() {
    // DEC-USB-002, REQ-USB-FIX-002, REQ-USB-FIX-003 of zmidi-usb: a class made from the fixed
    // function gives the configuration descriptor, the strings and the Group Terminal Block
    // answer of the class made from the same FunctionConfig.
    let names = InterfaceNames {
        audio_control: Some("Audio"),
        streaming: &[Some("MIDI 1.0"), Some("MIDI 2.0")],
    };
    let mut answers = Vec::new();
    for fixed in [false, true] {
        let a = alloc();
        let mut got = Vec::new();
        let gtb = [0x81, 0x06, 0x01, 0x26, 0x01, 0x00, 0xFF, 0x00];
        let string = |i: u8| [0x80, 0x06, i, 0x03, 0x09, 0x04, 0xFF, 0x00];
        if fixed {
            let mut class: MidiClass<'_, FakeBus, Fixed> =
                MidiClass::fixed(&a.alloc, &fixed_default::MIDI, names, LinkConfig::default())
                    .unwrap();
            let mut dev = device(&a, true);
            got.push(get_configuration(&a, &mut dev, &mut class));
            got.push(control(&a, &mut dev, &mut class, gtb).unwrap());
            for i in 1..6 {
                got.push(control(&a, &mut dev, &mut class, string(i)).unwrap_or_default());
            }
        } else {
            let cfg = FunctionConfig::new(0, 0, BusSpeed::Full).unwrap();
            let mut class: Class =
                MidiClass::new(&a.alloc, cfg, names, LinkConfig::default()).unwrap();
            let mut dev = device(&a, true);
            got.push(get_configuration(&a, &mut dev, &mut class));
            got.push(control(&a, &mut dev, &mut class, gtb).unwrap());
            for i in 1..6 {
                got.push(control(&a, &mut dev, &mut class, string(i)).unwrap_or_default());
            }
        }
        answers.push(got);
    }
    assert!(answers[0][0].len() > 100);
    assert_eq!(answers[0], answers[1]);
}
