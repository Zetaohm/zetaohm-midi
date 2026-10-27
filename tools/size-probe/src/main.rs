//! Size probe: one FLXS1 MIDI 2.0 feature per cargo feature, with the FLXS1 release profile.
//! The cost of a feature = this binary with the feature minus the binary with no feature.
//! INPUT is read volatile, so the compiler cannot fold the code away.

#![no_std]
#![no_main]

use core::hint::black_box;
use core::ptr::{addr_of, read_volatile};

// The critical-section implementation (feature critical-section-single-core) for embassy-sync.
use cortex_m as _;

#[cfg(any(feature = "usb1", feature = "usb2_any", feature = "usb_core"))]
mod bus;
#[cfg(feature = "emb_core")]
mod edriver;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

#[no_mangle]
static mut INPUT: [u32; 64] = [0; 64];

fn input() -> [u32; 64] {
    unsafe { read_volatile(addr_of!(INPUT)) }
}

#[allow(dead_code)]
fn input_bytes() -> &'static [u8] {
    let n = (input()[63] & 0xFF) as usize;
    unsafe { core::slice::from_raw_parts(addr_of!(INPUT) as *const u8, n.min(252)) }
}

#[allow(dead_code)]
fn rand() -> u32 {
    input()[62]
}

#[cortex_m_rt::entry]
fn main() -> ! {
    #[cfg(any(feature = "usb1", feature = "usb2_any", feature = "usb_core"))]
    usb::init();
    #[cfg(feature = "emb_core")]
    emb::init();
    #[cfg(feature = "stream")]
    stream::init();
    #[cfg(feature = "ci")]
    ci::init();
    loop {
        let w = input();
        #[cfg(any(feature = "usb1", feature = "usb2_any", feature = "usb_core"))]
        usb::run(&w);
        #[cfg(feature = "emb_core")]
        emb::run(&w);
        #[cfg(any(feature = "rx_cv2", feature = "rx_cv2_tier"))]
        rx_cv2::run(&w);
        #[cfg(feature = "tx_cv2")]
        tx_cv2::run(&w);
        #[cfg(feature = "translate")]
        translate::run(&w);
        #[cfg(feature = "bytes")]
        bytes::run(&w);
        #[cfg(feature = "flex")]
        flex::run(&w);
        #[cfg(feature = "stream")]
        stream::run(&w);
        #[cfg(feature = "ci")]
        ci::run(&w);
        #[cfg(feature = "sysex8")]
        sysex8::run(&w);
        #[cfg(feature = "remote")]
        remote::run(&w);
        #[cfg(feature = "inject")]
        inject::run(&w);
        #[cfg(any(feature = "rx_lean", feature = "down", feature = "up", feature = "bytes1", feature = "bytes_out", feature = "rx_assign", feature = "down_words", feature = "bytes_core", feature = "bytes_feed", feature = "bytes_out_each", feature = "up_lean", feature = "up_words"))]
        lean::run(&w);
        #[cfg(any(feature = "ble_rx", feature = "ble_rx_words", feature = "ble_tx", feature = "net_client", feature = "net_host", feature = "smf_read", feature = "clip_read", feature = "clip_write"))]
        transport::run(&w);
        black_box(&w);
    }
}

// ---------------------------------------------------------------- USB class (usbd-midi or zmidi-usb-device)
#[cfg(any(feature = "usb1", feature = "usb2_any", feature = "usb_core"))]
mod usb {
    use super::*;
    use crate::bus::StubBus as UsbBus;
    use usb_device::bus::UsbBusAllocator;
    use usb_device::device::{StringDescriptors, UsbDevice, UsbDeviceBuilder, UsbVidPid};

    #[cfg(feature = "usb1")]
    type Class = usbd_midi::UsbMidiClass<'static, UsbBus>;
    #[cfg(all(feature = "usb2_any", not(feature = "usb2_fixed_any")))]
    type Class = zmidi_usb_device::MidiClass<'static, UsbBus>;
    #[cfg(feature = "usb2_fixed_any")]
    type Class = zmidi_usb_device::MidiClass<'static, UsbBus, zmidi_usb_device::Fixed>;
    #[cfg(feature = "usb2_fixed_any")]
    include!(concat!(env!("OUT_DIR"), "/midi_fn.rs"));

    static mut ALLOC: Option<UsbBusAllocator<UsbBus>> = None;
    // MaybeUninit, as the crate docs say (OPT-013): an Option of these types has a non-zero None,
    // which would put the static in .data and its image in flash.
    #[cfg(any(feature = "usb1", feature = "usb2_any"))]
    static mut USB: core::mem::MaybeUninit<(UsbDevice<'static, UsbBus>, Class)> = core::mem::MaybeUninit::uninit();
    #[cfg(not(any(feature = "usb1", feature = "usb2_any")))]
    static mut USB: core::mem::MaybeUninit<UsbDevice<'static, UsbBus>> = core::mem::MaybeUninit::uninit();

    #[inline(never)]
    pub fn init() {
        let alloc: &'static UsbBusAllocator<UsbBus> =
            unsafe { (*(&raw mut ALLOC)).insert(UsbBusAllocator::new(UsbBus::new())) };
        #[cfg(feature = "usb1")]
        let midi = usbd_midi::UsbMidiClass::new(alloc, 1, 1).unwrap();
        #[cfg(feature = "usb2_fixed_any")]
        let midi = {
            static STREAMING: [Option<&str>; 2] = [Some("MIDI 1.0"), Some("MIDI 2.0")];
            let names = zmidi_usb_device::InterfaceNames { audio_control: Some("FLXS1"), streaming: &STREAMING };
            zmidi_usb_device::MidiClass::fixed(alloc, &MIDI_FN, names, zmidi_usb::LinkConfig::default()).unwrap()
        };
        #[cfg(all(feature = "usb2_any", not(feature = "usb2_fixed_any")))]
        let midi = {
            use zmidi_usb::device::FunctionConfig;
            use zmidi_usb::{BusSpeed, LinkConfig};
            static STREAMING: [Option<&str>; 2] = [Some("MIDI 1.0"), Some("MIDI 2.0")];
            let cfg = FunctionConfig::new(0, 0, BusSpeed::Full).unwrap();
            let names = zmidi_usb_device::InterfaceNames { audio_control: Some("FLXS1"), streaming: &STREAMING };
            zmidi_usb_device::MidiClass::new(alloc, cfg, names, LinkConfig::default()).unwrap()
        };
        let dev = UsbDeviceBuilder::new(alloc, UsbVidPid(0x16C0, 0x0485))
            .strings(&[StringDescriptors::default().manufacturer("Zetaohm").product("FLXS1").serial_number("0001")])
            .unwrap()
            .max_packet_size_0(64)
            .unwrap()
            .build();
        #[cfg(any(feature = "usb1", feature = "usb2_any"))]
        unsafe {
            (*(&raw mut USB)).write((dev, midi));
        }
        #[cfg(not(any(feature = "usb1", feature = "usb2_any")))]
        unsafe {
            (*(&raw mut USB)).write(dev);
        }
    }

    #[inline(never)]
    pub fn run(w: &[u32; 64]) {
        #[cfg(not(any(feature = "usb1", feature = "usb2_any")))]
        {
            let dev = unsafe { (*(&raw mut USB)).assume_init_mut() };
            black_box(dev.poll(&mut []));
            black_box(w);
        }
        #[cfg(any(feature = "usb1", feature = "usb2_any"))]
        let (dev, midi) = unsafe { (*(&raw mut USB)).assume_init_mut() };
        #[cfg(any(feature = "usb1", feature = "usb2_any"))]
        dev.poll(&mut [&mut *midi]);
        #[cfg(any(feature = "usb1", feature = "usb2_any"))]
        let mut buf = [0u8; 64];
        #[cfg(feature = "usb1")]
        {
            if let Ok(n) = midi.read(&mut buf) {
                black_box(&buf[..n]);
            }
            black_box(midi.send_bytes(w[1].to_le_bytes()).is_ok());
        }
        #[cfg(feature = "usb2_any")]
        {
            while midi
                .read(&mut buf, |r| {
                    if let Ok(r) = r {
                        black_box(r.words());
                    }
                })
                .is_ok()
            {}
            let one: [&[u32]; 1] = [&w[..2]];
            let mut it = one.into_iter().peekable();
            black_box(midi.write(&mut it, &mut buf).is_ok());
        }
    }
}

// ---------------------------------------------------------------- embassy-usb class (zmidi-usb-embassy)
#[cfg(feature = "emb_core")]
mod emb {
    use super::*;
    use crate::edriver::StubDriver;
    use core::future::Future;
    use core::pin::pin;
    use core::task::{Context, Waker};
    #[cfg(feature = "emb2_any")]
    use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex as M;
    use embassy_usb::{Builder, Config, UsbDevice};

    #[cfg(all(feature = "emb2_any", not(feature = "emb2_fixed_any")))]
    type F = zmidi_usb::device::FunctionConfig<'static>;
    #[cfg(feature = "emb2_fixed_any")]
    type F = zmidi_usb_embassy::Fixed;
    #[cfg(feature = "emb2_fixed_any")]
    include!(concat!(env!("OUT_DIR"), "/emb_fn.rs"));
    #[cfg(feature = "emb2_any")]
    type Tx = zmidi_usb_embassy::Sender<'static, StubDriver, M, 1, F, 2, 0>;
    #[cfg(feature = "emb2_any")]
    type Rx = zmidi_usb_embassy::Receiver<'static, StubDriver, M, 1, F, 2, 0>;

    static mut CONFIG_DESC: [u8; 576] = [0; 576];
    static mut BOS_DESC: [u8; 64] = [0; 64];
    static mut CONTROL: [u8; 256] = [0; 256];
    static mut DEV: Option<UsbDevice<'static, StubDriver>> = None;
    #[cfg(feature = "emb2_any")]
    static mut STATE: zmidi_usb_embassy::State<'static, M, 1, F, 0> = zmidi_usb_embassy::State::new();
    #[cfg(feature = "emb2_any")]
    static mut IO: Option<(Tx, Rx)> = None;

    fn poll_once<T>(f: impl Future<Output = T>) {
        let mut f = pin!(f);
        let mut cx = Context::from_waker(Waker::noop());
        black_box(f.as_mut().poll(&mut cx).is_ready());
    }

    #[inline(never)]
    pub fn init() {
        // emb_core adds no class, so the builder does not change.
        #[cfg_attr(not(feature = "emb2_any"), allow(unused_mut))]
        let mut builder = unsafe {
            Builder::new(
                StubDriver,
                Config::new(0x16C0, 0x0485),
                &mut *(&raw mut CONFIG_DESC),
                &mut *(&raw mut BOS_DESC),
                &mut [],
                &mut *(&raw mut CONTROL),
            )
        };
        #[cfg(feature = "emb2_any")]
        {
            static STREAMING: [Option<&str>; 2] = [Some("MIDI 1.0"), Some("MIDI 2.0")];
            let names = zmidi_usb_embassy::InterfaceNames { audio_control: Some("FLXS1"), streaming: &STREAMING };
            let state = unsafe { &mut *(&raw mut STATE) };
            #[cfg(not(feature = "emb2_fixed_any"))]
            let class: zmidi_usb_embassy::MidiClass<'static, StubDriver, M, 1, F, 2, 0> = zmidi_usb_embassy::MidiClass::new(
                &mut builder, state, zmidi_usb_embassy::default_config(zmidi_usb::BusSpeed::Full).unwrap(), names,
                zmidi_usb::LinkConfig::default()).unwrap();
            #[cfg(feature = "emb2_fixed_any")]
            let class: zmidi_usb_embassy::MidiClass<'static, StubDriver, M, 1, F, 2, 0> = zmidi_usb_embassy::MidiClass::fixed(
                &mut builder, state, &EMB_FN, names, zmidi_usb::LinkConfig::default()).unwrap();
            unsafe { *(&raw mut IO) = Some(class.split()) };
        }
        unsafe { *(&raw mut DEV) = Some(builder.build()) };
    }

    #[inline(never)]
    pub fn run(w: &[u32; 64]) {
        let dev = unsafe { (*(&raw mut DEV)).as_mut().unwrap() };
        // The data path first: with the stub bus, `run` never yields, so code after it is dead and
        // the compiler removes it (the jobs emb_class* missed the link read and write before
        // 2026-10-22, OPT-050).
        #[cfg(feature = "emb2_any")]
        {
            let (tx, rx) = unsafe { (*(&raw mut IO)).as_mut().unwrap() };
            let mut buf = [0u8; 64];
            poll_once(rx.read_packet(&mut buf, |r| {
                if let Ok(r) = r {
                    black_box(r.words());
                }
            }));
            let one: [&[u32]; 1] = [&w[..2]];
            let mut it = one.into_iter().peekable();
            let mut out = [0u8; 64];
            poll_once(tx.write_packet(&mut it, &mut out));
        }
        poll_once(dev.run());
        black_box(w);
    }
}

// ---------------------------------------------------------------- options 1, 2, 3, 6: receive and decode
#[cfg(any(feature = "rx_cv2", feature = "rx_cv2_tier"))]
mod rx_cv2 {
    use super::*;
    use zmidi_ump::{Midi2ChannelVoiceMessage as M, SystemMessage, Ump};

    #[inline(never)]
    pub fn run(w: &[u32; 64]) {
        let mut i = 0;
        while i < 60 {
            match Ump::parse(&w[i..]) {
                Ok((u, n)) => {
                    handle(&u);
                    i += n.max(1);
                }
                Err(_) => i += 1,
            }
        }
    }

    fn handle(u: &Ump) {
        match u {
            Ump::Midi2ChannelVoice(m) => match m {
                M::AssignableController(c) => {
                    black_box((1u8, c.group(), c.channel(), c.bank(), c.index(), c.data()));
                }
                M::RelativeAssignableController(c) => {
                    black_box((2u8, c.group(), c.channel(), c.bank(), c.index(), c.data()));
                }
                M::ProgramChange(p) => {
                    black_box((3u8, p.channel(), p.program(), p.bank_valid()));
                }
                M::ControlChange(c) => {
                    black_box((4u8, c.channel(), c.index(), c.data()));
                }
                M::NoteOn(n) => {
                    black_box((5u8, n.channel(), n.note(), n.velocity(), n.attribute_type(), n.attribute_data()));
                }
                M::NoteOff(n) => {
                    black_box((6u8, n.channel(), n.note()));
                }
                M::PerNotePitchBend(p) => {
                    black_box((7u8, p.channel(), p.note(), p.value()));
                }
                M::PitchBend(p) => {
                    black_box((8u8, p.channel(), p.value()));
                }
                _ => {}
            },
            Ump::System(s) => match s {
                SystemMessage::TimingClock(_) => { black_box(9u8); }
                SystemMessage::Start(_) => { black_box(10u8); }
                SystemMessage::Stop(_) => { black_box(11u8); }
                SystemMessage::Continue(_) => { black_box(12u8); }
                SystemMessage::SongPositionPointer(p) => { black_box(p); }
                _ => {}
            },
            _ => {}
        };
    }
}

// ---------------------------------------------------------------- option 4: sequencer output with attributes
#[cfg(feature = "tx_cv2")]
mod tx_cv2 {
    use super::*;
    use zmidi_ump::{
        Channel, Group, Midi2ChannelVoiceMessage as M, Midi2NoteOff, Midi2NoteOn, NoteAttribute, PerNotePitchBend,
        Pitch7_9, Ump,
    };

    #[inline(never)]
    pub fn run(w: &[u32; 64]) {
        let g = Group::new((w[1] & 15) as u8).unwrap_or_default();
        let c = Channel::new((w[2] & 15) as u8).unwrap_or_default();
        let note = (w[3] & 127) as u8;
        let mut out = [0u32; 4];
        let msg = match w[4] & 3 {
            0 => Midi2NoteOn::try_new(g, c, note, w[5] as u16, NoteAttribute::Pitch(Pitch7_9::from_bits(w[6] as u16)))
                .map(M::NoteOn),
            1 => Midi2NoteOff::try_new(g, c, note, w[5] as u16, NoteAttribute::None).map(M::NoteOff),
            _ => PerNotePitchBend::try_new(g, c, note, w[7]).map(M::PerNotePitchBend),
        };
        if let Ok(m) = msg {
            black_box(Ump::Midi2ChannelVoice(m).write(&mut out).ok());
        }
        black_box(&out);
    }
}

// ---------------------------------------------------------------- MIDI 1.0 <-> MIDI 2.0 Protocol translation
#[cfg(feature = "translate")]
mod translate {
    use super::*;
    use zmidi_ump::{to_midi1, Midi1ToMidi2, Ump};

    static mut UP: Midi1ToMidi2 = Midi1ToMidi2::new();

    #[inline(never)]
    pub fn run(w: &[u32; 64]) {
        if let Ok((u, _)) = Ump::parse(&w[8..]) {
            let up = unsafe { &mut *(&raw mut UP) };
            black_box(up.translate(&u));
            let t = to_midi1(&u);
            black_box(t.as_slice());
        }
    }
}

// ---------------------------------------------------------------- 5-pin DIN bytes <-> UMP
#[cfg(feature = "bytes")]
mod bytes {
    use super::*;
    use zmidi_io::{BytesToUmp, UmpToBytes};
    use zmidi_ump::{Group, StreamProtocol};

    static mut IN: Option<BytesToUmp> = None;
    static mut OUT: Option<UmpToBytes> = None;

    #[inline(never)]
    pub fn run(w: &[u32; 64]) {
        let inp = unsafe { (*(&raw mut IN)).get_or_insert_with(|| BytesToUmp::new(Group::default(), StreamProtocol::Midi2)) };
        for &b in input_bytes() {
            black_box(inp.push(b));
        }
        let out = unsafe { (*(&raw mut OUT)).get_or_insert_with(|| UmpToBytes::new(true)) };
        let mut buf = [0u8; 16];
        black_box(out.write(&w[16..20], &mut buf).ok());
        black_box(&buf);
    }
}

// ---------------------------------------------------------------- option 5: Flex Data tempo, meter, key
#[cfg(feature = "flex")]
mod flex {
    use super::*;
    use zmidi_ump::FlexDataMessage;

    #[inline(never)]
    pub fn run(w: &[u32; 64]) {
        match FlexDataMessage::parse(&w[20..24]) {
            Some(FlexDataMessage::SetTempo(t)) => {
                black_box(t.ten_ns_per_quarter_note());
            }
            Some(FlexDataMessage::SetTimeSignature(t)) => {
                black_box(t);
            }
            Some(FlexDataMessage::SetKeySignature(k)) => {
                black_box(k);
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------- option 7: UMP Endpoint and Function Blocks
#[cfg(feature = "stream")]
mod stream {
    use super::*;
    use zmidi_io::{Endpoint, EndpointDescription, FunctionBlock};
    use zmidi_ump::{DeviceIdentityNotification, FunctionBlockDirection, FunctionBlockMidi1, Group};

    static mut EP: Option<Endpoint<'static, 4>> = None;

    #[inline(never)]
    pub fn init() {
        let names = ["Track 1", "Track 2", "Track 3", "Track 4"];
        let blocks = core::array::from_fn(|i| {
            FunctionBlock::midi1_port(
                i as u8,
                Group::new(i as u8).unwrap(),
                FunctionBlockDirection::Bidirectional,
                FunctionBlockMidi1::NotMidi1,
                names[i],
            )
        });
        let d = EndpointDescription {
            name: "FLXS1",
            product_instance_id: "0001",
            device_identity: DeviceIdentityNotification::try_new([0x7D, 0, 0], 1, 1, [0; 4]).unwrap(),
            midi1: true,
            midi2: true,
            rxjr: false,
            txjr: false,
            static_function_blocks: true,
        };
        unsafe { *(&raw mut EP) = Endpoint::try_new(d, blocks).ok() };
    }

    #[inline(never)]
    pub fn run(w: &[u32; 64]) {
        if let Some(ep) = unsafe { (*(&raw mut EP)).as_mut() } {
            for r in ep.receive(&w[24..28]) {
                black_box(r);
            }
        }
    }
}

// ---------------------------------------------------------------- options 8, 9: MIDI-CI (and PE)
#[cfg(feature = "ci")]
mod ci {
    use super::*;
    use zmidi_ci::{Categories, CiDevice, DeviceConfig, DeviceIdentity, Framing, PiCapabilities};
    use zmidi_ump::{Group, SysEx7Event, SysEx7Packets, SysEx7Reassembler};

    pub type Device = CiDevice<'static, 2, 1, 4>;
    // In `MaybeUninit`: an `Option<Device>` static is in `.data`, and its image costs flash (OPT-013).
    static mut DEV: core::mem::MaybeUninit<Option<Device>> = core::mem::MaybeUninit::uninit();
    static mut RX: SysEx7Reassembler<512> = SysEx7Reassembler::new(Group::from_bits(0));
    // The reply buffer: a responder with no PE needs CiDevice::MAX_REPLY_LEN (60 B here,
    // REQ-CI-LEN-004); a PE reply can use the whole Receivable Maximum SysEx size.
    #[cfg(not(any(feature = "pe", feature = "pe_one_chunk")))]
    static mut TX: [u8; Device::MAX_REPLY_LEN] = [0; Device::MAX_REPLY_LEN];
    #[cfg(any(feature = "pe", feature = "pe_one_chunk"))]
    static mut TX: [u8; 512] = [0; 512];

    #[inline(never)]
    pub fn init() {
        let cfg = DeviceConfig {
            identity: DeviceIdentity { manufacturer: zmidi_ci::ManufacturerId::from_bytes([0x7D, 0, 0]), family: 1, model: 1, revision: [0; 4] },
            categories: Categories {
                protocol_negotiation: false,
                profile_configuration: cfg!(feature = "ci_c"),
                property_exchange: cfg!(any(feature = "pe", feature = "pe_one_chunk")),
                process_inquiry: cfg!(feature = "ci_c"),
            },
            max_sysex_size: 512,
            function_block: None,
            first_group: 0,
            group_count: 1,
            channels_in_use: 4,
            product_instance_id: Some("0001"),
            framing: Framing::Bare,
            process_inquiry: PiCapabilities::default(),
        };
        unsafe { (*(&raw mut DEV)).write(Device::new(cfg, &mut || rand()).ok()) };
        #[cfg(feature = "pe")]
        pe::init();
    }

    #[inline(never)]
    pub fn run(w: &[u32; 64]) {
        let Some(dev) = (unsafe { (*(&raw mut DEV)).assume_init_mut().as_mut() }) else { return };
        let now = w[0] as u64;
        let tx = unsafe { &mut *(&raw mut TX) };
        black_box(dev.poll(now));
        let rx = unsafe { &mut *(&raw mut RX) };
        if let Some(SysEx7Event::Complete(msg)) = rx.push(&w[28..30]) {
            let mut r = dev.receive(0, msg, now, &mut || rand());
            let ev = r.event();
            while let Some(Ok((_g, n))) = r.write_next(tx) {
                send(&tx[..n]);
            }
            #[cfg(feature = "pe")]
            if let Some(ev) = ev {
                pe::on_event(dev, &ev, now);
            }
            // One-chunk PE Responder (DEC-PE-005 of zmidi-ci-pe).
            #[cfg(feature = "pe_one_chunk")]
            if let Some(ev) = ev {
                struct App;
                impl zmidi_ci_pe::one_chunk::Resources for App {
                    fn get(&mut self, r: &[u8], _id: Option<&[u8]>, out: &mut [u8]) -> Result<usize, zmidi_ci_pe::Status> {
                        if r != b"X-State" {
                            return Err(zmidi_ci_pe::Status::NOT_FOUND);
                        }
                        // A constant value, as the get of the C probe (tools/lib-compare/c/m2_ci.c).
                        let v = black_box(&b"{\"v\":1}"[..]);
                        let n = v.len().min(out.len());
                        out[..n].copy_from_slice(&v[..n]);
                        Ok(n)
                    }
                    fn set(&mut self, r: &[u8], _id: Option<&[u8]>, data: &[u8]) -> Result<(), zmidi_ci_pe::Status> {
                        black_box(data);
                        if r == b"X-State" { Ok(()) } else { Err(zmidi_ci_pe::Status::NOT_FOUND) }
                    }
                }
                static LIST: &[u8] = br#"[{"resource":"DeviceInfo"},{"resource":"X-State","canSet":"full"}]"#;
                static mut BODY: [u8; 256] = [0; 256];
                let body = unsafe { &mut *(&raw mut BODY) };
                let out = unsafe { &mut *(&raw mut TX) };
                // pe_one_chunk_flat: the flat Header Data reader (DEC-PE-006).
                #[cfg(not(feature = "pe_one_chunk_flat"))]
                black_box(zmidi_ci_pe::one_chunk::respond(dev, &ev, LIST, &mut App, body, out, send));
                #[cfg(feature = "pe_one_chunk_flat")]
                black_box(zmidi_ci_pe::one_chunk::respond_flat(dev, &ev, LIST, &mut App, body, out, send));
            }
            #[cfg(not(any(feature = "pe", feature = "pe_one_chunk")))]
            black_box(ev);
        }
        #[cfg(feature = "pe")]
        pe::poll(dev, now);
    }

    pub fn send(bytes: &[u8]) {
        if let Ok(p) = SysEx7Packets::try_new(Group::default(), bytes) {
            for pkt in p {
                black_box(pkt);
            }
        }
    }

    #[cfg(feature = "pe")]
    pub mod pe {
        use super::*;
        use zmidi_ci::Event;
        use zmidi_ci_pe::{CanSet, EngineConfig, JsonWriter, PeEngine, PeEvent, ResourceCaps, ResourceEntry};

        #[cfg(not(feature = "pe_nosub"))]
        type Engine = PeEngine<'static, 2, 256, 4, 2>;
        // No subscription table: the S = 0 tier (REQ-PE-SUB-022 of zmidi-ci-pe).
        #[cfg(feature = "pe_nosub")]
        type Engine = PeEngine<'static, 2, 256, 0, 2>;
        // `MaybeUninit`, not `Option`: `None` is not all zero bits, so an `Option` goes to `.data`
        // and its image costs flash (OPT-013). The `PeEngine` docs give this pattern to firmware.
        static mut ENG: core::mem::MaybeUninit<Engine> = core::mem::MaybeUninit::uninit();
        static mut SCRATCH: [u8; 512] = [0; 512];
        static mut BODY: [u8; 512] = [0; 512];

        const CAPS: ResourceCaps = ResourceCaps {
            can_get: true,
            can_set: CanSet::Full,
            can_subscribe: true,
            require_res_id: false,
            fc_on_get: false,
            fc_on_set: false,
            encodings: zmidi_ci_pe::Encodings { ascii: true, mcoded7: true, zlib_mcoded7: cfg!(feature = "zlib") },
            can_paginate: false,
        };
        static RESOURCES: [ResourceEntry<'static>; 4] = [
            ResourceEntry::new("DeviceInfo"),
            ResourceEntry::new("ChannelList"),
            { let mut e = ResourceEntry::new("X-Pattern"); e.caps = CAPS; e },
            { let mut e = ResourceEntry::new("X-Global"); e.caps = CAPS; e },
        ];

        pub fn init() {
            let cfg = EngineConfig { resources: &RESOURCES, zlib_window: if cfg!(feature = "zlib") { 1024 } else { 0 } };
            unsafe { (*(&raw mut ENG)).write(Engine::new(cfg)) };
        }

        #[inline(never)]
        pub fn on_event(dev: &mut Device, ev: &Event<'_>, now: u64) {
            let eng = unsafe { (*(&raw mut ENG)).assume_init_mut() };
            let scratch = unsafe { &mut *(&raw mut SCRATCH) };
            if let Ok(Some(PeEvent::Message(m))) = eng.receive(dev, ev, now, scratch) {
                // The application: one pattern resource as JSON.
                let body = unsafe { &mut *(&raw mut BODY) };
                let mut j = JsonWriter::new(body);
                let _ = j.begin_object();
                for step in 0..16u32 {
                    let _ = j.key("p");
                    let _ = j.number_u32(step ^ rand());
                }
                let _ = j.end_object();
                let n = j.len();
                #[cfg(feature = "zlib")]
                {
                    static mut WINDOW: [u8; 1024] = [0; 1024];
                    static mut INFL: [u8; 256] = [0; 256];
                    let mut d = zmidi_ci_pe::ZlibMcoded7Decoder::new(unsafe { &mut *(&raw mut WINDOW) });
                    black_box(d.decode(m.data, unsafe { &mut *(&raw mut INFL) }).ok());
                    let mut e = zmidi_ci_pe::ZlibMcoded7Encoder::new();
                    let tx = unsafe { &mut *(&raw mut TX) };
                    black_box(e.encode(&body[..n], tx).ok());
                    black_box(e.finish(tx).ok());
                }
                let mut h = zmidi_ci_pe::Header::default();
                h.status = Some(zmidi_ci_pe::Status(200));
                let _ = eng.reply(dev, m.peer, m.request_id, &zmidi_ci_pe::Outgoing::json(h, n), now);
                let tx = unsafe { &mut *(&raw mut TX) };
                if let Ok(k) = eng.send(dev, m.peer, m.request_id, zmidi_ci::Direction::Responder, &body[..n], true, now, tx) {
                    black_box(k);
                }
            }
        }

        #[inline(never)]
        pub fn poll(dev: &mut Device, now: u64) {
            let eng = unsafe { (*(&raw mut ENG)).assume_init_mut() };
            black_box(eng.poll(dev, now));
            let tx = unsafe { &mut *(&raw mut TX) };
            while let Some(Ok(n)) = eng.write_pending(dev, now, tx) {
                send(&tx[..n]);
            }
        }
    }
}

// ---------------------------------------------------------------- option 10: SysEx8 transfer
#[cfg(feature = "sysex8")]
mod sysex8 {
    use super::*;
    use zmidi_ump::{Group, SysEx8Event, SysEx8Packets, SysEx8Reassembler};

    static mut RX: SysEx8Reassembler<256> = SysEx8Reassembler::new(Group::from_bits(0), 0);

    #[inline(never)]
    pub fn run(w: &[u32; 64]) {
        let rx = unsafe { &mut *(&raw mut RX) };
        if let Some(SysEx8Event::Complete(data)) = rx.push(&w[32..36]) {
            black_box(data);
        }
        for pkt in SysEx8Packets::new(Group::default(), 0, input_bytes()) {
            black_box(pkt);
        }
    }
}

// ---------------------------------------------------------------- option 11: remote panel over manufacturer SysEx
#[cfg(feature = "remote")]
mod remote {
    use super::*;
    use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
    use zmidi_ump::{Group, SysEx7Event, SysEx7Packets, SysEx7Reassembler};

    pub static BUTTONS: AtomicU32 = AtomicU32::new(0);
    pub static ENC: AtomicI32 = AtomicI32::new(0);
    pub static CLICK: AtomicBool = AtomicBool::new(false);
    static mut RX: SysEx7Reassembler<16> = SysEx7Reassembler::new(Group::from_bits(0));
    static mut LEDS: [u8; 92] = [0; 92];
    static mut FB: [u8; 6144] = [0; 6144];
    static mut CURSOR: usize = 0;

    #[inline(never)]
    pub fn run(w: &[u32; 64]) {
        let rx = unsafe { &mut *(&raw mut RX) };
        if let Some(SysEx7Event::Complete(m)) = rx.push(&w[36..38]) {
            // 7D (non-commercial id) cmd args
            if m.len() >= 3 && m[0] == 0x7D {
                match m[1] {
                    1 => BUTTONS.store(u32::from(m[2]) | (u32::from(*m.get(3).unwrap_or(&0)) << 7), Ordering::Relaxed),
                    2 => { ENC.fetch_add(i32::from(m[2]) - 64, Ordering::Relaxed); }
                    3 => CLICK.store(m[2] != 0, Ordering::Relaxed),
                    4 => unsafe { CURSOR = 0 },
                    _ => {}
                }
            }
        }
        // Stream the LED frame, then the framebuffer, 7-bit packed, 48 bytes each call.
        let mut out = [0u8; 64];
        let fb = unsafe { &*(&raw const FB) };
        let leds = unsafe { &*(&raw const LEDS) };
        let cur = unsafe { &mut *(&raw mut CURSOR) };
        let src: &[u8] = if *cur < 92 { &leds[*cur..] } else if *cur - 92 < fb.len() { &fb[*cur - 92..] } else { &[] };
        let take = src.len().min(42);
        out[0] = 0x7D;
        out[1] = 0x10;
        let mut n = 2;
        for chunk in src[..take].chunks(7) {
            let mut hi = 0u8;
            for (k, b) in chunk.iter().enumerate() {
                hi |= (b >> 7) << k;
                out[n + 1 + k] = b & 0x7F;
            }
            out[n] = hi;
            n += 1 + chunk.len();
        }
        *cur += take;
        if let Ok(p) = SysEx7Packets::try_new(Group::default(), &out[..n]) {
            for pkt in p {
                black_box(pkt);
            }
        }
        unsafe { FB[(w[37] as usize) % 6144] = w[38] as u8; LEDS[(w[39] as usize) % 92] = w[40] as u8 };
    }
}

// ---------------------------------------------------------------- option 13: UMP injection from the SWD bench
#[cfg(feature = "inject")]
mod inject {
    use super::*;
    use core::sync::atomic::{AtomicU32, Ordering};

    #[no_mangle]
    #[used]
    pub static mut INJECT_UMP: [[u32; 4]; 8] = [[0; 4]; 8];
    #[no_mangle]
    #[used]
    pub static INJECT_HEAD: AtomicU32 = AtomicU32::new(0);
    static TAIL: AtomicU32 = AtomicU32::new(0);

    #[inline(never)]
    pub fn run(_w: &[u32; 64]) {
        let head = INJECT_HEAD.load(Ordering::Acquire);
        let mut tail = TAIL.load(Ordering::Relaxed);
        while tail != head {
            let u = unsafe { read_volatile(addr_of!(INJECT_UMP[(tail & 7) as usize])) };
            black_box(u);
            tail = tail.wrapping_add(1);
        }
        TAIL.store(tail, Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------- lean variants
#[cfg(any(feature = "rx_lean", feature = "down", feature = "up", feature = "bytes1", feature = "bytes_out", feature = "rx_assign", feature = "down_words", feature = "bytes_core", feature = "bytes_feed", feature = "bytes_out_each", feature = "up_lean", feature = "up_words"))]
mod lean {
    use super::*;
    #[allow(unused_imports)]
    use zmidi_ump::{Midi2ChannelVoiceMessage as M, SystemMessage, Ump};

    #[inline(never)]
    #[allow(unused_variables)]
    pub fn run(w: &[u32; 64]) {
        #[cfg(feature = "rx_lean")]
        {
            // Decode only Message Type 0x4 and 0x1, by the first word.
            match w[0] >> 28 {
                0x4 => match M::parse(&w[0..2]) {
                    Some(M::AssignableController(c)) => { black_box((1u8, c.group(), c.channel(), c.bank(), c.index(), c.data())); }
                    Some(M::RelativeAssignableController(c)) => { black_box((2u8, c.group(), c.channel(), c.bank(), c.index(), c.data())); }
                    Some(M::ProgramChange(p)) => { black_box((3u8, p.channel(), p.program(), p.bank_valid())); }
                    Some(M::ControlChange(c)) => { black_box((4u8, c.channel(), c.index(), c.data())); }
                    Some(M::NoteOn(n)) => { black_box((5u8, n.channel(), n.note(), n.velocity(), n.attribute_type(), n.attribute_data())); }
                    Some(M::NoteOff(n)) => { black_box((6u8, n.channel(), n.note())); }
                    Some(M::PerNotePitchBend(p)) => { black_box((7u8, p.channel(), p.note(), p.value())); }
                    Some(M::PitchBend(p)) => { black_box((8u8, p.channel(), p.value())); }
                    _ => {}
                },
                0x1 => match SystemMessage::parse(&w[0..1]) {
                    Some(SystemMessage::TimingClock(_)) => { black_box(9u8); }
                    Some(SystemMessage::Start(_)) => { black_box(10u8); }
                    Some(SystemMessage::Stop(_)) => { black_box(11u8); }
                    Some(SystemMessage::Continue(_)) => { black_box(12u8); }
                    Some(SystemMessage::SongPositionPointer(p)) => { black_box(p); }
                    _ => {}
                },
                _ => {}
            }
        }
        #[cfg(feature = "rx_assign")]
        {
            // Only the controller messages that carry EditAction (options 1, 2).
            if w[0] >> 28 == 0x4 {
                match M::parse(&w[0..2]) {
                    Some(M::AssignableController(c)) => { black_box((1u8, c.group(), c.channel(), c.bank(), c.index(), c.data())); }
                    Some(M::RelativeAssignableController(c)) => { black_box((2u8, c.group(), c.channel(), c.bank(), c.index(), c.data())); }
                    _ => {}
                }
            }
        }
        #[cfg(feature = "down")]
        if let Ok((u, _)) = Ump::parse(&w[8..]) {
            black_box(zmidi_ump::to_midi1(&u).as_slice());
        }
        #[cfg(feature = "up")]
        {
            static mut UP: zmidi_ump::Midi1ToMidi2 = zmidi_ump::Midi1ToMidi2::new();
            if let Ok((u, _)) = Ump::parse(&w[8..]) {
                black_box(unsafe { (*(&raw mut UP)).translate(&u) });
            }
        }
        #[cfg(feature = "bytes1")]
        {
            static mut IN: Option<zmidi_io::BytesToUmp> = None;
            let inp = unsafe { (*(&raw mut IN)).get_or_insert_with(|| zmidi_io::BytesToUmp::midi1(zmidi_ump::Group::default())) };
            for &b in input_bytes() { black_box(inp.push(b)); }
        }
        #[cfg(feature = "down_words")]
        if let Some(m) = zmidi_ump::midi2_to_midi1(&w[8..]) {
            black_box(m.as_slice());
        }
        // The callback forms of the DIN reader and writer (OPT-021, REQ-BS-017).
        #[cfg(feature = "up_lean")]
        {
            static mut UP: zmidi_ump::Midi1ToMidi2Group = zmidi_ump::Midi1ToMidi2Group::new();
            if let Some(m) = zmidi_ump::Midi1ChannelVoiceMessage::parse(&w[8..]) {
                if let Some(u) = unsafe { (*(&raw mut UP)).translate(&Ump::Midi1ChannelVoice(m)) } {
                    black_box(u.to_packet());
                }
            }
        }
        #[cfg(feature = "up_words")]
        {
            static mut UP: zmidi_ump::Midi1ToMidi2Group = zmidi_ump::Midi1ToMidi2Group::new();
            if let Some(w2) = unsafe { (*(&raw mut UP)).translate_words(w[8]) } {
                black_box(w2);
            }
        }
        #[cfg(feature = "bytes_feed")]
        {
            static mut IN: zmidi_io::BytesToUmpCore =
                zmidi_io::BytesToUmpCore::new(zmidi_ump::Group::from_bits(0));
            let inp = unsafe { &mut *(&raw mut IN) };
            for &b in input_bytes() {
                inp.feed(b, |w| {
                    black_box(w[0] ^ w.get(1).copied().unwrap_or(0));
                });
            }
        }
        #[cfg(feature = "bytes_out_each")]
        {
            static mut OUT: zmidi_io::UmpToBytes = zmidi_io::UmpToBytes::new(true);
            let out = unsafe { &mut *(&raw mut OUT) };
            out.write_each(&w[16..20], |b| {
                black_box(b);
            });
        }
        #[cfg(feature = "bytes_core")]
        {
            static mut IN: zmidi_io::BytesToUmpCore =
                zmidi_io::BytesToUmpCore::new(zmidi_ump::Group::from_bits(0));
            let inp = unsafe { &mut *(&raw mut IN) };
            for &b in input_bytes() {
                for p in inp.push_with(b, None) {
                    black_box(p.words());
                }
            }
        }
        #[cfg(feature = "bytes_out")]
        {
            static mut OUT: Option<zmidi_io::UmpToBytes> = None;
            let out = unsafe { (*(&raw mut OUT)).get_or_insert_with(|| zmidi_io::UmpToBytes::new(true)) };
            let mut buf = [0u8; 16];
            black_box(out.write(&w[16..20], &mut buf).ok());
            black_box(&buf);
        }
    }
}

#[cfg(feature = "sz")]
#[no_mangle]
#[used]
pub static SIZES: [u32; 6] = [
    core::mem::size_of::<zmidi_ump::Ump>() as u32,
    core::mem::size_of::<zmidi_io::Pushed>() as u32,
    core::mem::size_of::<zmidi_ump::Translated>() as u32,
    core::mem::size_of::<zmidi_ump::Midi2ChannelVoiceMessage>() as u32,
    core::mem::size_of::<zmidi_ump::Midi1ToMidi2Group>() as u32,
    core::mem::align_of::<zmidi_ump::Ump>() as u32,
];
#[cfg(feature = "sz")]
mod sz {
    const _A: [(); 0] = [(); core::mem::size_of::<zmidi_ump::Ump>()];
    const _B: [(); 0] = [(); core::mem::size_of::<zmidi_io::Pushed>()];
    const _C: [(); 0] = [(); core::mem::size_of::<zmidi_ump::Translated>()];
    const _D: [(); 0] = [(); core::mem::size_of::<zmidi_ump::Midi1ToMidi2Group>()];
    const _E: [(); 0] = [(); core::mem::size_of::<zmidi_ci::CiDevice<'static, 2, 1, 4>>()];
}

// ---------------------------------------------------------------- BLE-MIDI, Network MIDI 2.0, files
#[cfg(any(feature = "ble_rx", feature = "ble_rx_words", feature = "ble_tx", feature = "net_client", feature = "net_host", feature = "smf_read", feature = "clip_read", feature = "clip_write"))]
mod transport {
    use super::*;

    #[inline(never)]
    pub fn run(w: &[u32; 64]) {
        #[cfg(feature = "ble_rx")]
        ble_rx();
        #[cfg(feature = "ble_rx_words")]
        ble_rx_words();
        #[cfg(feature = "ble_tx")]
        ble_tx(w);
        #[cfg(feature = "net_client")]
        net_client(w);
        #[cfg(feature = "net_host")]
        net_host();
        #[cfg(feature = "smf_read")]
        smf_read();
        #[cfg(feature = "clip_read")]
        clip_read();
        #[cfg(feature = "clip_write")]
        clip_write(w);
        black_box(w);
    }

    #[cfg(feature = "ble_rx")]
    fn ble_rx() {
        use zmidi_ble::zmidi_ump::{Group, StreamProtocol, Ump};
        static mut R: core::mem::MaybeUninit<zmidi_ble::PacketReader> = core::mem::MaybeUninit::uninit();
        static mut INIT: bool = false;
        // SAFETY: one thread, no interrupt.
        let r = unsafe {
            if !INIT {
                (*(&raw mut R)).write(zmidi_ble::PacketReader::new(Group::default(), StreamProtocol::Midi2));
                INIT = true;
            }
            (*(&raw mut R)).assume_init_mut()
        };
        r.read(input_bytes(), |x| match x {
            Ok(rx) => {
                if let Ump::Midi2ChannelVoice(m) = rx.ump {
                    black_box(&m);
                }
                black_box(rx.timestamp);
            }
            Err(e) => {
                black_box(e);
            }
        });
    }

    #[cfg(feature = "ble_rx_words")]
    fn ble_rx_words() {
        use zmidi_ble::zmidi_ump::{Group, StreamProtocol};
        static mut R: core::mem::MaybeUninit<zmidi_ble::PacketReader> = core::mem::MaybeUninit::uninit();
        static mut INIT: bool = false;
        // SAFETY: one thread, no interrupt.
        let r = unsafe {
            if !INIT {
                (*(&raw mut R)).write(zmidi_ble::PacketReader::new(Group::default(), StreamProtocol::Midi2));
                INIT = true;
            }
            (*(&raw mut R)).assume_init_mut()
        };
        r.read_words(input_bytes(), |x| match x {
            Ok(rx) => {
                black_box(rx.words);
                black_box(rx.timestamp);
            }
            Err(e) => {
                black_box(e);
            }
        });
    }

    #[cfg(feature = "ble_tx")]
    fn ble_tx(w: &[u32; 64]) {
        static mut W: core::mem::MaybeUninit<zmidi_ble::PacketWriter> = core::mem::MaybeUninit::uninit();
        static mut INIT: bool = false;
        static mut OUT: [u8; 20] = [0; 20];
        // SAFETY: one thread, no interrupt.
        let (wr, out) = unsafe {
            if !INIT {
                if let Ok(x) = zmidi_ble::PacketWriter::new(23) {
                    (*(&raw mut W)).write(x);
                    INIT = true;
                } else {
                    return;
                }
            }
            ((*(&raw mut W)).assume_init_mut(), &mut *(&raw mut OUT))
        };
        let n = (w[61] & 3) as usize + 1;
        let mut umps = w[..60].chunks(n).map(|c| zmidi_ble::TimedUmp { millis: u64::from(w[60]), words: c }).peekable();
        black_box(wr.write_packet(&mut umps, out).ok());
    }

    #[cfg(any(feature = "net_client", feature = "net_host"))]
    fn addr(w: u32) -> core::net::SocketAddr {
        core::net::SocketAddr::new(core::net::IpAddr::V4(core::net::Ipv4Addr::from(w)), 5673)
    }

    #[cfg(feature = "net_client")]
    fn net_client(w: &[u32; 64]) {
        use zmidi_net::{Client, ClientConfig, EventKind, Identity, NoCredentials};
        static mut C: Option<Client<1>> = None;
        static mut BUF: [u8; 512] = [0; 512];
        // SAFETY: one thread, no interrupt.
        let (slot, buf) = unsafe { (&mut *(&raw mut C), &mut *(&raw mut BUF)) };
        let now = u64::from(w[60]);
        let host = addr(w[59]);
        if slot.is_none() {
            let Ok(id) = Identity::new("FLXS1", "0001") else { return };
            let Ok(c) = Client::init(slot, ClientConfig::new(id)) else { return };
            black_box(c.connect(now, host).is_ok());
            return;
        }
        let Some(c) = slot.as_mut() else { return };
        let mut creds = NoCredentials;
        {
            let mut r = c.receive(now, host, input_bytes(), &mut creds);
            for e in r.by_ref() {
                if let EventKind::Data(d) = e.kind {
                    for m in d.messages() {
                        black_box(&m);
                    }
                } else {
                    black_box(&e);
                }
            }
            while let Some(n) = r.write_next(buf) {
                black_box(n);
            }
        }
        for e in c.poll(now) {
            black_box(&e);
        }
        black_box(c.send_ump(host, &w[..(w[61] & 3) as usize + 1]).is_ok());
        while let Some(x) = c.write_pending(now, buf) {
            black_box(x);
        }
    }

    #[cfg(feature = "net_host")]
    fn net_host() {
        use zmidi_net::{EventKind, Host, HostConfig, Identity, NoCredentials};
        static mut H: Option<Host<1>> = None;
        static mut BUF: [u8; 512] = [0; 512];
        // SAFETY: one thread, no interrupt.
        let (slot, buf) = unsafe { (&mut *(&raw mut H), &mut *(&raw mut BUF)) };
        let w = input();
        let now = u64::from(w[60]);
        if slot.is_none() {
            let Ok(id) = Identity::new("FLXS1", "0001") else { return };
            black_box(Host::init(slot, HostConfig::new(id)).is_ok());
            return;
        }
        let Some(h) = slot.as_mut() else { return };
        let mut creds = NoCredentials;
        {
            let mut r = h.receive(now, addr(w[59]), input_bytes(), &mut creds);
            for e in r.by_ref() {
                if let EventKind::Data(d) = e.kind {
                    for m in d.messages() {
                        black_box(&m);
                    }
                } else {
                    black_box(&e);
                }
            }
            while let Some(n) = r.write_next(buf) {
                black_box(n);
            }
        }
        for e in h.poll(now) {
            black_box(&e);
        }
        while let Some(x) = h.write_pending(now, buf) {
            black_box(x);
        }
    }

    #[cfg(feature = "smf_read")]
    fn smf_read() {
        let Ok(f) = zmidi_file::SmfReader::new(input_bytes()) else { return };
        black_box((f.format(), f.division()));
        for t in f.tracks() {
            let Ok(t) = t else { break };
            for e in t {
                let Ok(e) = e else { break };
                black_box(&e);
            }
        }
    }

    #[cfg(feature = "clip_read")]
    fn clip_read() {
        use zmidi_ump::Ump;
        let Ok(r) = zmidi_file::ClipReader::new(input_bytes()) else { return };
        for e in r {
            let Ok(e) = e else { break };
            if let Ump::Midi2ChannelVoice(m) = e.ump {
                black_box(&m);
            }
            black_box(e.ticks);
        }
    }

    #[cfg(feature = "clip_write")]
    fn clip_write(w: &[u32; 64]) {
        use zmidi_file::{ClipConfig, ClipWriter};
        static mut BUF: [u8; 512] = [0; 512];
        // SAFETY: one thread, no interrupt.
        let buf = unsafe { &mut *(&raw mut BUF) };
        let config = ClipConfig {
            ticks_per_quarter_note: (w[61] & 0x7FFF) as u16,
            profiles: &[],
            tempo: None,
            time_signature: None,
            group: zmidi_ump::Group::default(),
        };
        let Ok(mut c) = ClipWriter::new(buf, &config) else { return };
        if c.start(0, None, None).is_err() {
            return;
        }
        for (i, u) in w[..60].chunks(2).enumerate() {
            black_box(c.event(u64::from(w[60]) * i as u64, u).is_ok());
        }
        black_box(c.finish(u64::from(w[62])).ok());
    }
}
