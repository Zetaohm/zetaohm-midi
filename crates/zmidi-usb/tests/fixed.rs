//! The fixed function (DEC-USB-002, OPT-016): the source that `write_rust` makes, and the
//! descriptors and Group Terminal Block answers of `FixedFunction`, against the run-time
//! builder of the same configuration.
//!
//! The fixtures are the output of `write_rust` for two configurations. To make them again:
//! `UPDATE_FIXTURES=1 cargo test -p zmidi-usb --test fixed`.
//! The fixtures have an Alternate Setting 0 with ports (feature `alt0`).
#![cfg(feature = "alt0")]

use zmidi_usb::device::SettingEndpoint;
use zmidi_usb::device::{
    gtb_set, write_function, write_function_to, DescriptorSink, EndpointFields, FunctionConfig,
    InterfaceFields, Midi2Setting,
};
use zmidi_usb::fixed::{write_rust, FixedError, FixedFunction, FixedIds};
use zmidi_usb::ms2::TransferType;
use zmidi_usb::EndpointDirection;
use zmidi_usb::{handle_gtb_setup, BusSpeed, GroupMask, GtbResponse, SetupPacket};

mod default_fn {
    include!("fixtures/default_fn.rs");
}
mod two_settings_fn {
    include!("fixtures/two_settings_fn.rs");
}

fn default_cfg() -> FunctionConfig<'static> {
    FunctionConfig::new(0, 1, BusSpeed::Full).unwrap()
}

fn two_settings_cfg() -> FunctionConfig<'static, 4, 2, 2, 1> {
    let mut c = FunctionConfig::<4, 2, 2, 1>::new(0, 1, BusSpeed::High).unwrap();
    c.add_midi2_setting(Midi2Setting::new(BusSpeed::High).unwrap())
        .unwrap();
    c
}

fn source<const J: usize, const S: usize, const E: usize, const B: usize>(
    cfg: &FunctionConfig<'_, J, S, E, B>,
    name: &str,
) -> String {
    let mut s = String::new();
    write_rust(cfg, name, &mut s).unwrap();
    s
}

fn check_fixture(file: &str, text: &str) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(file);
    if std::env::var_os("UPDATE_FIXTURES").is_some() {
        std::fs::write(&path, text).unwrap();
    }
    let stored = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        stored, text,
        "{file} is not the output of write_rust; see the module docs"
    );
}

#[test]
fn write_rust_makes_the_stored_fixtures() {
    // REQ-USB-FIX-001: the source is the same for the same configuration.
    check_fixture("default_fn.rs", &source(&default_cfg(), "MIDI"));
    check_fixture("two_settings_fn.rs", &source(&two_settings_cfg(), "MIDI"));
}

#[test]
fn write_rust_checks_the_rules_of_the_writers() {
    // REQ-USB-FIX-001: a configuration that breaks a rule gives an error, not a source.
    let mut s = String::new();
    // The function writer: no USB-MIDI 1.0 Alternate Setting 0.
    let mut c = default_cfg();
    c.midi1 = None;
    assert_eq!(
        write_rust(&c, "MIDI", &mut s),
        Err(FixedError::Device(
            zmidi_usb::device::DeviceError::NoMidi1Setting
        ))
    );
    // The Group Terminal Block writer: a USB MIDI 2.0 setting with no block.
    let mut c = default_cfg();
    let mut m2 = Midi2Setting::new(BusSpeed::Full).unwrap();
    m2.blocks.clear();
    c.midi2.as_mut_slice()[0] = m2;
    assert!(matches!(
        write_rust(&c, "MIDI", &mut s),
        Err(FixedError::Device(_))
    ));
    // The adapter rule: two endpoints in one direction in one Alternate Setting.
    let mut c = default_cfg();
    let mut m2 = Midi2Setting::new(BusSpeed::Full).unwrap();
    m2.endpoints.as_mut_slice()[1].direction = EndpointDirection::MidiOut;
    m2.endpoints.as_mut_slice()[1].number = 2;
    c.midi2.as_mut_slice()[0] = m2;
    assert!(matches!(
        write_rust(&c, "MIDI", &mut s),
        Err(FixedError::Endpoints { .. })
    ));
}

/// A sink that writes each descriptor again as bytes.
#[derive(Default)]
struct Bytes(Vec<u8>);

impl DescriptorSink for Bytes {
    type Error = ();
    fn interface(&mut self, f: &InterfaceFields) -> Result<(), ()> {
        self.0.extend_from_slice(&[
            9,
            4,
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
    fn endpoint(&mut self, f: &EndpointFields, extra: &[u8]) -> Result<(), ()> {
        let [lo, hi] = f.max_packet.to_le_bytes();
        self.0.extend_from_slice(&[
            7 + extra.len() as u8,
            5,
            f.address,
            f.attributes,
            lo,
            hi,
            f.interval,
        ]);
        self.0.extend_from_slice(extra);
        Ok(())
    }
    fn class_descriptor(&mut self, t: u8, payload: &[u8]) -> Result<(), ()> {
        self.0.extend_from_slice(&[2 + payload.len() as u8, t]);
        self.0.extend_from_slice(payload);
        Ok(())
    }
}

fn check_function<const J: usize, const S: usize, const E: usize, const B: usize>(
    fixed: &FixedFunction,
    cfg: &FunctionConfig<'_, J, S, E, B>,
) {
    // REQ-USB-FIX-002: for other interface numbers and strings, replay gives the bytes of
    // write_function, and the sink gets the same calls as from write_function_to.
    for (ac, ms) in [(0u8, 1u8), (2, 3), (5, 4)] {
        let mut c = cfg.clone();
        c.ac_interface = ac;
        c.ms_interface = ms;
        c.ac_string = 4;
        if let Some(m) = c.midi1.as_mut() {
            m.string_index = 5;
        }
        let mut strings = vec![5u8];
        for (i, s) in c.midi2.as_mut_slice().iter_mut().enumerate() {
            s.string_index = 6 + i as u8;
            strings.push(6 + i as u8);
        }
        let mut expected = vec![0u8; 1024];
        let n = write_function(&c, &mut expected).unwrap();
        let mut via_sink = Bytes::default();
        write_function_to(&c, &mut via_sink).unwrap();
        assert_eq!(via_sink.0, &expected[..n]);
        let ids = FixedIds {
            ac_interface: ac,
            ms_interface: ms,
            ac_string: 4,
            streaming_strings: &strings,
        };
        let mut got = Bytes::default();
        fixed.replay(&ids, &mut got).unwrap();
        assert_eq!(got.0, &expected[..n], "interfaces {ac}, {ms}");
        assert_eq!(fixed.len(), n);
    }
    // The endpoints and the Alternate Settings.
    assert_eq!(
        fixed.endpoints(),
        cfg.endpoints().collect::<Vec<_>>().as_slice()
    );
    assert_eq!(fixed.alternate_settings(), cfg.alternate_settings());
    assert_eq!(fixed.group_mask(0), GroupMask::ALL);
    for alt in 1..cfg.alternate_settings() as u8 {
        let s = cfg.midi2_setting(alt).unwrap();
        let e = s
            .endpoints
            .iter()
            .find(|e| e.direction == zmidi_usb::EndpointDirection::MidiOut);
        let mask = e.map_or(GroupMask::EMPTY, |e| {
            GroupMask::from_blocks(s.blocks.as_slice(), e)
        });
        assert_eq!(fixed.group_mask(alt), mask);
        let mut set = [0u8; 256];
        let k = gtb_set(cfg, alt, &mut set).unwrap();
        assert_eq!(fixed.gtb_sets()[usize::from(alt) - 1], &set[..k]);
    }
    // REQ-USB-FIX-003: the Group Terminal Block request gets the answer of handle_gtb_setup,
    // for each interface, Alternate Setting and wLength.
    for interface in [0u16, 1, 2] {
        for alt in 0..=3u8 {
            for length in [0u16, 1, 5, 6, 12, 18, 255] {
                let setup =
                    SetupPacket::from_parts(0x81, 0x06, u16::from(alt) | 0x2600, interface, length);
                let (mut a, mut b) = ([0u8; 256], [0u8; 256]);
                let ra = handle_gtb_setup(&setup, cfg, &mut a);
                let rb = fixed.gtb_setup(&setup, cfg.ms_interface, &mut b);
                assert_eq!(ra, rb, "interface {interface}, alt {alt}, wLength {length}");
                if let Ok(GtbResponse::Answered(n)) = ra {
                    assert_eq!(a[..n], b[..n]);
                }
            }
        }
    }
    let other = SetupPacket::from_parts(0x80, 0x06, 0x0200, 0, 9);
    assert_eq!(
        fixed.gtb_setup(&other, 1, &mut [0u8; 16]),
        Ok(GtbResponse::NotHandled)
    );
}

#[test]
fn fixed_function_gives_the_run_time_descriptors_and_answers() {
    check_function(&default_fn::MIDI, &default_cfg());
    check_function(&two_settings_fn::MIDI, &two_settings_cfg());
}

#[test]
fn replay_stops_at_a_broken_length_with_no_panic() {
    // A descriptor with bLength 0, 1 or past the end ends the walk (never panics).
    static BAD: [&[u8]; 4] = [&[0, 4, 1], &[1, 4], &[9, 4, 0, 0], &[3, 0x24, 1, 200, 5]];
    for d in BAD {
        let f = FixedFunction::new(d, &[], &[GroupMask::ALL], &[]).unwrap();
        let mut got = Bytes::default();
        f.replay(&FixedIds::default(), &mut got).unwrap();
        assert!(got.0.len() <= d.len());
    }
}

fn ep(alt: u8, direction: EndpointDirection, number: u8, max_packet: u16) -> SettingEndpoint {
    SettingEndpoint {
        alternate_setting: alt,
        direction,
        number,
        transfer: TransferType::Bulk,
        max_packet,
        interval: 0,
    }
}

#[test]
fn new_refuses_a_broken_endpoint_rule() {
    // OPT-027: the checks that the adapters skip for a Fixed function.
    use EndpointDirection::{MidiIn, MidiOut};
    let masks: &'static [GroupMask] = &[GroupMask::ALL, GroupMask::ALL];
    let leak =
        |e: Vec<SettingEndpoint>| -> &'static [SettingEndpoint] { Box::leak(e.into_boxed_slice()) };
    let ok = leak(vec![
        ep(0, MidiOut, 1, 64),
        ep(0, MidiIn, 1, 64),
        ep(1, MidiOut, 1, 64),
    ]);
    assert!(FixedFunction::new(&[], ok, masks, &[]).is_ok());
    // Two MIDI OUT endpoints in one Alternate Setting.
    let two = leak(vec![ep(0, MidiOut, 1, 64), ep(0, MidiOut, 2, 64)]);
    assert_eq!(
        FixedFunction::new(&[], two, masks, &[]),
        Err(FixedError::Endpoints { address: 0x02 })
    );
    // One address with two sizes.
    let size = leak(vec![ep(0, MidiOut, 1, 64), ep(1, MidiOut, 1, 32)]);
    assert_eq!(
        FixedFunction::new(&[], size, masks, &[]),
        Err(FixedError::Endpoints { address: 0x01 })
    );
    // The check also runs in a constant (a broken rule there stops the build).
    const EMPTY: Result<FixedFunction, FixedError> = FixedFunction::new(&[], &[], &[], &[]);
    assert!(EMPTY.is_ok());
}
