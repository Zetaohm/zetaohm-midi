//! The C ABI of the zmidi-usb `MidiLink` with one cable (DEC-019): the USB-MIDI 1.0 Event Packets of
//! Alternate Setting 0 and the UMP words of the USB MIDI 2.0 settings, for C USB stacks (for example
//! TinyUSB). The program moves the USB transfers; the link converts them. No callbacks.

use core::mem::{align_of, size_of};

use zmidi_usb::device::gtb_set_len;
use zmidi_usb::fixed::FixedFunction;
use zmidi_usb::gtb::GtbReader;
use zmidi_usb::ms2::{GtbIds, TransferType};
use zmidi_usb::request::{answer_total_length, write_gtb_request, SetupPacket};
use zmidi_usb::zmidi_ump::StreamProtocol;
use zmidi_usb::{
    function_len, gtb_set, handle_gtb_request, write_function, BusSpeed, CableCounts, DeviceError,
    EndpointConfig, EndpointDirection, FunctionConfig, GroupMask, GroupTerminalBlock, GtbList,
    GtbResponse, GtbType, LinkConfig, LinkError, LinkEvent, Midi1Setting, Midi2Setting, MidiLink,
    MidiProtocol, RequestError, SettingNumber,
};

use crate::status::ZmidiStatus;

/// One cable with one translator, so the Event Packet reader can give MIDI 2.0 Protocol UMPs.
type Link = MidiLink<1, 1>;

/// Storage words of a [`ZmidiUsbLink`] on a 64-bit target.
#[cfg(target_pointer_width = "64")]
pub const ZMIDI_USB_LINK_WORDS: usize = 45;
/// Storage words of a [`ZmidiUsbLink`] on a 32-bit or 16-bit target.
#[cfg(not(target_pointer_width = "64"))]
pub const ZMIDI_USB_LINK_WORDS: usize = 38;

/// Storage of one link. The program gives it; [`zmidi_usb_link_init`] sets it up.
#[repr(C)]
pub struct ZmidiUsbLink {
    /// Opaque content.
    pub _opaque: [u64; ZMIDI_USB_LINK_WORDS],
}

const _: () = assert!(size_of::<Link>() <= size_of::<ZmidiUsbLink>());
const _: () = assert!(align_of::<Link>() <= align_of::<ZmidiUsbLink>());

/// The configuration of a link.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ZmidiUsbLinkConfig {
    /// True on a host (the received endpoint is a MIDI IN endpoint); false on a device (MIDI OUT).
    pub host_side: bool,
    /// The Group Terminals of the received endpoint: bit n is Group n.
    pub group_mask: u16,
    /// wMaxPacketSize of the sent endpoint, 4 or more.
    pub max_packet: u16,
    /// The protocol of the UMPs of the Event Packet reader: 1 MIDI 1.0 Protocol, 2 MIDI 2.0
    /// Protocol.
    pub protocol: u32,
}

/// One UMP of a received transfer.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZmidiUsbUmp {
    /// Words of the UMP, 1 to 4.
    pub word_count: u8,
    /// The words; words after `word_count` are 0.
    pub words: [u32; 4],
}

/// Gives the link of `link`, or `None` for a null pointer.
///
/// # Safety
/// `link` is null or was set up by [`zmidi_usb_link_init`].
unsafe fn link_mut<'a>(link: *mut ZmidiUsbLink) -> Option<&'a mut Link> {
    // SAFETY: caller contract above.
    unsafe { link.cast::<Link>().as_mut() }
}

/// Sets up `link` from `config`, at Alternate Setting 0.
///
/// # Safety
/// `link` points to writable storage of a [`ZmidiUsbLink`]; `config` is valid.
#[no_mangle]
pub unsafe extern "C" fn zmidi_usb_link_init(
    link: *mut ZmidiUsbLink,
    config: *const ZmidiUsbLinkConfig,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let (false, Some(c)) = (link.is_null(), unsafe { config.as_ref() }) else {
        return ZmidiStatus::InvalidArgument;
    };
    let protocol = match c.protocol {
        1 => StreamProtocol::Midi1,
        2 => StreamProtocol::Midi2,
        _ => return ZmidiStatus::InvalidArgument,
    };
    let config = LinkConfig {
        receive: if c.host_side {
            EndpointDirection::MidiIn
        } else {
            EndpointDirection::MidiOut
        },
        mask: GroupMask::from_bits(c.group_mask),
        max_packet: c.max_packet,
        protocol,
        ..LinkConfig::default()
    };
    match Link::new(config) {
        // SAFETY: the storage is large and aligned enough (asserts above) and writable (caller).
        Ok(l) => unsafe {
            link.cast::<Link>().write(l);
            ZmidiStatus::Ok
        },
        Err(_) => ZmidiStatus::InvalidArgument,
    }
}

/// Reports that the host selected Alternate Setting `setting` (0: USB-MIDI 1.0 Event Packets;
/// other: USB MIDI 2.0 UMP), with the Group Terminal mask and wMaxPacketSize of that setting.
///
/// # Safety
/// `link` is null or was set up by [`zmidi_usb_link_init`].
#[no_mangle]
pub unsafe extern "C" fn zmidi_usb_link_set_alternate_setting(
    link: *mut ZmidiUsbLink,
    setting: u8,
    group_mask: u16,
    max_packet: u16,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let Some(l) = (unsafe { link_mut(link) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    match l.report_setting(
        SettingNumber(setting),
        GroupMask::from_bits(group_mask),
        max_packet,
    ) {
        Ok(()) => ZmidiStatus::Ok,
        Err(_) => ZmidiStatus::InvalidArgument,
    }
}

/// Reports a USB reset: the link goes to Alternate Setting 0.
///
/// # Safety
/// `link` is null or was set up by [`zmidi_usb_link_init`].
#[no_mangle]
pub unsafe extern "C" fn zmidi_usb_link_usb_reset(link: *mut ZmidiUsbLink) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let Some(l) = (unsafe { link_mut(link) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    l.report(LinkEvent::UsbReset);
    ZmidiStatus::Ok
}

/// Reports a configuration change (SET_CONFIGURATION): the link goes to Alternate Setting 0.
///
/// # Safety
/// `link` is null or was set up by [`zmidi_usb_link_init`].
#[no_mangle]
pub unsafe extern "C" fn zmidi_usb_link_configuration_change(
    link: *mut ZmidiUsbLink,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let Some(l) = (unsafe { link_mut(link) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    l.report(LinkEvent::ConfigurationChange);
    ZmidiStatus::Ok
}

/// Gives the selected Alternate Setting, or 0 for a null pointer.
///
/// # Safety
/// `link` is null or was set up by [`zmidi_usb_link_init`].
#[no_mangle]
pub unsafe extern "C" fn zmidi_usb_link_setting(link: *mut ZmidiUsbLink) -> u8 {
    // SAFETY: caller contract above.
    unsafe { link_mut(link) }.map_or(0, |l| l.setting().0)
}

/// Gives true when the link uses UMP words (a USB MIDI 2.0 setting); false for Event Packets.
///
/// # Safety
/// `link` is null or was set up by [`zmidi_usb_link_init`].
#[no_mangle]
pub unsafe extern "C" fn zmidi_usb_link_is_ump(link: *mut ZmidiUsbLink) -> bool {
    // SAFETY: caller contract above.
    unsafe { link_mut(link) }.is_some_and(|l| l.mode() == zmidi_usb::link::LinkMode::Ump)
}

/// Gives the size of the USB packets that the writer writes, or 0 for a null pointer.
///
/// # Safety
/// `link` is null or was set up by [`zmidi_usb_link_init`].
#[no_mangle]
pub unsafe extern "C" fn zmidi_usb_link_packet_size(link: *mut ZmidiUsbLink) -> usize {
    // SAFETY: caller contract above.
    unsafe { link_mut(link) }.map_or(0, |l| l.packet_size())
}

/// Gives true when the writer has bytes for the next packet (a split UMP or a SysEx part).
///
/// # Safety
/// `link` is null or was set up by [`zmidi_usb_link_init`].
#[no_mangle]
pub unsafe extern "C" fn zmidi_usb_link_pending(link: *mut ZmidiUsbLink) -> bool {
    // SAFETY: caller contract above.
    unsafe { link_mut(link) }.is_some_and(|l| l.pending())
}

/// Gives true when the reader holds part of a UMP or of a SysEx message from the last transfer.
///
/// # Safety
/// `link` is null or was set up by [`zmidi_usb_link_init`].
#[no_mangle]
pub unsafe extern "C" fn zmidi_usb_link_holds_part(link: *mut ZmidiUsbLink) -> bool {
    // SAFETY: caller contract above.
    unsafe { link_mut(link) }.is_some_and(|l| l.holds_part())
}

/// Reads the data of one received USB transfer (`len` bytes) and writes each UMP into `out`
/// (`capacity` records), the record count into `count` and the count of errors into `errors`. A
/// `capacity` below `len / 4 + 1` gives [`ZmidiStatus::BufferTooSmall`] before the data is read.
///
/// # Safety
/// `link` was set up by [`zmidi_usb_link_init`]; `data` points to `len` bytes; `out` points to
/// `capacity` writable records; `count` and `errors` are writable.
#[no_mangle]
pub unsafe extern "C" fn zmidi_usb_link_read(
    link: *mut ZmidiUsbLink,
    data: *const u8,
    len: usize,
    out: *mut ZmidiUsbUmp,
    capacity: usize,
    count: *mut usize,
    errors: *mut usize,
) -> ZmidiStatus {
    if data.is_null() || out.is_null() || count.is_null() || errors.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let Some(l) = (unsafe { link_mut(link) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    // Each UMP takes at least 4 bytes; one more record for a UMP that a held part completes.
    if capacity < len / 4 + 1 {
        return ZmidiStatus::BufferTooSmall;
    }
    // SAFETY: caller contract above.
    let (data, out) = unsafe {
        (
            core::slice::from_raw_parts(data, len),
            core::slice::from_raw_parts_mut(out, capacity),
        )
    };
    let (mut n, mut bad) = (0usize, 0usize);
    l.read_with(data, |r| match r {
        Ok(rx) => {
            let (words, wc) = rx.words();
            let mut w = [0u32; 4];
            for (to, from) in w.iter_mut().zip(&words[..wc]) {
                *to = *from;
            }
            if let Some(slot) = out.get_mut(n) {
                *slot = ZmidiUsbUmp {
                    word_count: wc as u8,
                    words: w,
                };
                n += 1;
            }
        }
        Err(LinkError::Tail { .. }) => {}
        Err(_) => bad += 1,
    });
    // SAFETY: caller contract above.
    unsafe {
        count.write(n);
        errors.write(bad);
    }
    ZmidiStatus::Ok
}

/// Writes one USB packet into `out` (`capacity` bytes, at least the packet size) from `umps`
/// (`ump_count` UMPs), and gives its length in `len` (0: nothing to send) and the count of UMPs
/// taken from `umps` in `taken`.
///
/// # Safety
/// `link` was set up by [`zmidi_usb_link_init`]; `umps` points to `ump_count` UMPs (or is null when
/// `ump_count` is 0); `out` points to `capacity` writable bytes; `len` and `taken` are writable.
#[no_mangle]
pub unsafe extern "C" fn zmidi_usb_link_write_packet(
    link: *mut ZmidiUsbLink,
    umps: *const ZmidiUsbUmp,
    ump_count: usize,
    out: *mut u8,
    capacity: usize,
    len: *mut usize,
    taken: *mut usize,
) -> ZmidiStatus {
    if (umps.is_null() && ump_count > 0) || out.is_null() || len.is_null() || taken.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let Some(l) = (unsafe { link_mut(link) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    let (umps, out) = unsafe {
        (
            if ump_count == 0 {
                &[][..]
            } else {
                core::slice::from_raw_parts(umps, ump_count)
            },
            core::slice::from_raw_parts_mut(out, capacity),
        )
    };
    let mut iter = umps
        .iter()
        .map(|u| &u.words[..usize::from(u.word_count).min(4)])
        .peekable();
    match l.write_packet(&mut iter, out) {
        Ok(n) => {
            let left = iter.count();
            // SAFETY: caller contract above.
            unsafe {
                len.write(n);
                taken.write(ump_count - left);
            }
            ZmidiStatus::Ok
        }
        Err(LinkError::Overflow) => ZmidiStatus::BufferTooSmall,
        Err(_) => ZmidiStatus::InvalidArgument,
    }
}

/// Gives the size in bytes of the Rust `MidiLink` (DEC-019 rule 6).
#[no_mangle]
pub extern "C" fn zmidi_usb_link_size() -> usize {
    size_of::<Link>()
}

// The descriptor writer and the Group Terminal Block requests (USB-MIDI v2.0 section 6 and
// Appendix B; REQ-FFI-USB-006 to REQ-FFI-USB-009). The C configuration has fixed capacities
// (DEC-019 third addendum of 2026-10-23) and only the cable-count form of Alternate Setting 0.

/// The most USB MIDI 2.0 Alternate Settings of a [`ZmidiUsbFunction`] (Alternate Settings 1 and 2).
pub const ZMIDI_USB_MAX_SETTINGS: usize = 2;
/// The most endpoints of one USB MIDI 2.0 Alternate Setting.
pub const ZMIDI_USB_MAX_ENDPOINTS: usize = 4;
/// The most Group Terminal Blocks of one USB MIDI 2.0 Alternate Setting: one for each Group.
pub const ZMIDI_USB_MAX_BLOCKS: usize = 16;
/// The most Group Terminal Block IDs of one endpoint (baAssoGrpTrmBlkID, Table 5-4).
pub const ZMIDI_USB_MAX_BLOCK_IDS: usize = 16;
/// The length of a USB setup packet (USB 2.0 section 9.3).
pub const ZMIDI_USB_SETUP_LEN: usize = 8;

const _: () = assert!(ZMIDI_USB_MAX_BLOCK_IDS == zmidi_usb::ms2::MAX_BLOCK_IDS);
const _: () = assert!(ZMIDI_USB_SETUP_LEN == zmidi_usb::request::SETUP_LEN);

/// The function configuration of the C ABI: the capacities above, no full Jack list.
type Function = FunctionConfig<
    'static,
    0,
    ZMIDI_USB_MAX_SETTINGS,
    ZMIDI_USB_MAX_ENDPOINTS,
    ZMIDI_USB_MAX_BLOCKS,
>;

/// Storage words of a [`ZmidiUsbFunction`] on a 64-bit target.
#[cfg(target_pointer_width = "64")]
pub const ZMIDI_USB_FUNCTION_WORDS: usize = 103;
/// Storage words of a [`ZmidiUsbFunction`] on a 32-bit or 16-bit target.
#[cfg(not(target_pointer_width = "64"))]
pub const ZMIDI_USB_FUNCTION_WORDS: usize = 90;

/// Storage of one USB MIDI function configuration. The program gives it;
/// [`zmidi_usb_function_init`] or [`zmidi_usb_function_init_default`] sets it up.
#[repr(C)]
pub struct ZmidiUsbFunction {
    /// Opaque content.
    pub _opaque: [u64; ZMIDI_USB_FUNCTION_WORDS],
}

const _: () = assert!(size_of::<Function>() <= size_of::<ZmidiUsbFunction>());
const _: () = assert!(align_of::<Function>() <= align_of::<ZmidiUsbFunction>());

/// The interfaces, the bus speed and Alternate Setting 0 (USB-MIDI 1.0, cable-count form) of a
/// function.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ZmidiUsbFunctionConfig {
    /// bInterfaceNumber of the AudioControl interface, from the USB stack.
    pub ac_interface: u8,
    /// bInterfaceNumber of the MIDI Streaming interface, from the USB stack.
    pub ms_interface: u8,
    /// True for high speed (480 Mb/s), false for full speed (12 Mb/s).
    pub high_speed: bool,
    /// iInterface of the AudioControl interface; 0 is no string.
    pub ac_string: u8,
    /// iInterface of Alternate Setting 0; 0 is no string.
    pub alt0_string: u8,
    /// The cable count of the MIDI OUT endpoint (host to device) of Alternate Setting 0, 0 to 16.
    pub n_out: u8,
    /// The cable count of the MIDI IN endpoint (device to host) of Alternate Setting 0, 0 to 16.
    pub n_in: u8,
    /// The endpoint number of the MIDI OUT endpoint of Alternate Setting 0, 1 to 15.
    pub out_endpoint: u8,
    /// The endpoint number of the MIDI IN endpoint of Alternate Setting 0, 1 to 15.
    pub in_endpoint: u8,
    /// wMaxPacketSize of both bulk endpoints of Alternate Setting 0; 0 is the default of the bus
    /// speed (64 at full speed, 512 at high speed).
    pub max_packet: u16,
}

/// One endpoint of a USB MIDI 2.0 Alternate Setting (USB-MIDI v2.0 Tables 5-3 and 5-4).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ZmidiUsbEndpoint {
    /// 0: MIDI OUT (host to device); 1: MIDI IN (device to host).
    pub direction: u8,
    /// The endpoint number, 1 to 15.
    pub number: u8,
    /// The transfer type (bmAttributes D1..0): 2 bulk, 3 interrupt.
    pub transfer: u8,
    /// bInterval of an interrupt endpoint (1 or more); the writer writes 0 for bulk.
    pub interval: u8,
    /// wMaxPacketSize: a multiple of 4 that the bus speed permits.
    pub max_packet: u16,
    /// The number of Group Terminal Block IDs in `block_ids`, 1 to 16.
    pub block_count: u8,
    /// baAssoGrpTrmBlkID: the Group Terminal Block IDs, in descriptor order.
    pub block_ids: [u8; ZMIDI_USB_MAX_BLOCK_IDS],
}

/// One Group Terminal Block (USB-MIDI v2.0 Table 5-6). The fields are the raw descriptor values.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ZmidiUsbBlock {
    /// bGrpTrmBlkID, 1 to 255.
    pub id: u8,
    /// bGrpTrmBlkType: 0 bidirectional, 1 input only, 2 output only.
    pub kind: u8,
    /// nGroupTrm: the first member Group Terminal Number, 0x0 to 0xF.
    pub first_group: u8,
    /// nNumGroupTrm: the number of member Group Terminals, 1 to 16 - `first_group`.
    pub num_groups: u8,
    /// iBlockItem; 0 is no string.
    pub string_index: u8,
    /// bMIDIProtocol (for example 0x11 MIDI 2.0, 0x03 MIDI 1.0 with UMPs up to 128 bits).
    pub protocol: u8,
    /// wMaxInputBandwidth: 0 unknown, else units of 4000 bytes per second.
    pub max_input_bandwidth: u16,
    /// wMaxOutputBandwidth: 0 unknown, else units of 4000 bytes per second.
    pub max_output_bandwidth: u16,
}

/// One endpoint as the writer writes it, for the endpoint allocation of the USB stack.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ZmidiUsbSettingEndpoint {
    /// bAlternateSetting: 0 for USB-MIDI 1.0, 1 or more for USB MIDI 2.0.
    pub alternate_setting: u8,
    /// bEndpointAddress: D7 direction (1 IN), D3..0 number.
    pub address: u8,
    /// The transfer type (bmAttributes D1..0): 2 bulk, 3 interrupt.
    pub transfer: u8,
    /// bInterval: 0 for a bulk endpoint.
    pub interval: u8,
    /// wMaxPacketSize.
    pub max_packet: u16,
}

/// The result of [`zmidi_usb_gtb_setup`] (REQ-USB-GTB-017 to REQ-USB-GTB-019).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZmidiUsbGtbResponse {
    /// The handler wrote the data stage of the answer; send it.
    Answered = 0,
    /// Stall the request (USB 2.0 9.2.7). Nothing is written.
    Stall = 1,
    /// Not the Group Terminal Block descriptors request; handle it in another way.
    NotHandled = 2,
}

/// Gives the function of `f`, or `None` for a null pointer.
///
/// # Safety
/// `f` is null or was set up by [`zmidi_usb_function_init`] or [`zmidi_usb_function_init_default`].
unsafe fn function_mut<'a>(f: *mut ZmidiUsbFunction) -> Option<&'a mut Function> {
    // SAFETY: caller contract above.
    unsafe { f.cast::<Function>().as_mut() }
}

/// The status of a writer error: a short buffer, a setting number with no setting, or a broken
/// descriptor rule.
fn device_status(e: DeviceError) -> ZmidiStatus {
    match e {
        DeviceError::Overflow => ZmidiStatus::BufferTooSmall,
        DeviceError::NoSuchSetting(_) => ZmidiStatus::InvalidArgument,
        _ => ZmidiStatus::Rejected,
    }
}

/// Gives `n` to `out` and Ok, or the status of the error.
///
/// # Safety
/// `out` is writable.
unsafe fn give(r: Result<usize, DeviceError>, out: *mut usize) -> ZmidiStatus {
    match r {
        // SAFETY: caller contract above.
        Ok(n) => unsafe {
            out.write(n);
            ZmidiStatus::Ok
        },
        Err(e) => device_status(e),
    }
}

/// The bus speed of `high_speed`.
fn speed(high_speed: bool) -> BusSpeed {
    if high_speed {
        BusSpeed::High
    } else {
        BusSpeed::Full
    }
}

/// Sets up `f` as a function with Alternate Setting 0 only (USB-MIDI 1.0, the cable-count form
/// of `config`). Add the USB MIDI 2.0 settings with [`zmidi_usb_function_add_setting`]. The
/// writers check the rules of the configuration.
///
/// # Safety
/// `f` points to writable storage of a [`ZmidiUsbFunction`]; `config` is null or valid.
#[no_mangle]
pub unsafe extern "C" fn zmidi_usb_function_init(
    f: *mut ZmidiUsbFunction,
    config: *const ZmidiUsbFunctionConfig,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let (false, Some(c)) = (f.is_null(), unsafe { config.as_ref() }) else {
        return ZmidiStatus::InvalidArgument;
    };
    let speed = speed(c.high_speed);
    let mut cfg = Function::midi1_only(c.ac_interface, c.ms_interface, speed);
    cfg.ac_string = c.ac_string;
    let mut cables = CableCounts::new(c.n_out, c.n_in, speed);
    cables.out_endpoint = c.out_endpoint;
    cables.in_endpoint = c.in_endpoint;
    if c.max_packet != 0 {
        cables.max_packet = c.max_packet;
    }
    let mut midi1 = Midi1Setting::cables(cables);
    midi1.string_index = c.alt0_string;
    cfg.midi1 = Some(midi1);
    // SAFETY: the storage is large and aligned enough (asserts above) and writable (caller).
    unsafe { f.cast::<Function>().write(cfg) };
    ZmidiStatus::Ok
}

/// Sets up `f` as the default function (USB-MIDI v2.0 Appendix B): Alternate Setting 0 with one
/// cable in each direction on bulk endpoints OUT 1 and IN 1, and Alternate Setting 1 with bulk
/// endpoints OUT 1 and IN 1 on one bidirectional Group Terminal Block of Group 1. wMaxPacketSize
/// is 64 at full speed and 512 at high speed.
///
/// # Safety
/// `f` points to writable storage of a [`ZmidiUsbFunction`].
#[no_mangle]
pub unsafe extern "C" fn zmidi_usb_function_init_default(
    f: *mut ZmidiUsbFunction,
    ac_interface: u8,
    ms_interface: u8,
    high_speed: bool,
) -> ZmidiStatus {
    if f.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    match Function::new(ac_interface, ms_interface, speed(high_speed)) {
        // SAFETY: the storage is large and aligned enough (asserts above) and writable (caller).
        Ok(cfg) => unsafe {
            f.cast::<Function>().write(cfg);
            ZmidiStatus::Ok
        },
        Err(e) => device_status(e),
    }
}

/// Adds a USB MIDI 2.0 Alternate Setting with no endpoint and the default Group Terminal Block
/// (bidirectional, Group 1, ID 1), and gives its bAlternateSetting (1 or 2) in `alt`. Rejected when
/// the function has [`ZMIDI_USB_MAX_SETTINGS`] settings.
///
/// # Safety
/// `f` is null or was set up; `alt` is writable.
#[no_mangle]
pub unsafe extern "C" fn zmidi_usb_function_add_setting(
    f: *mut ZmidiUsbFunction,
    string_index: u8,
    alt: *mut u8,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let (false, Some(cfg)) = (alt.is_null(), unsafe { function_mut(f) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    let mut setting = Midi2Setting::empty();
    setting.string_index = string_index;
    match cfg.add_midi2_setting(setting) {
        // SAFETY: caller contract above.
        Ok(n) => unsafe {
            alt.write(n);
            ZmidiStatus::Ok
        },
        Err(_) => ZmidiStatus::Rejected,
    }
}

/// Gives the USB MIDI 2.0 setting `alt` of `f`, or `None` for a null pointer or no such setting.
///
/// # Safety
/// As [`function_mut`].
unsafe fn setting_mut<'a>(
    f: *mut ZmidiUsbFunction,
    alt: u8,
) -> Option<&'a mut Midi2Setting<ZMIDI_USB_MAX_ENDPOINTS, ZMIDI_USB_MAX_BLOCKS>> {
    // SAFETY: caller contract above.
    let cfg = unsafe { function_mut(f) }?;
    cfg.midi2
        .as_mut_slice()
        .get_mut(usize::from(alt).checked_sub(1)?)
}

/// Adds an endpoint to USB MIDI 2.0 Alternate Setting `alt`. Invalid argument for a null
/// pointer, no such setting, a direction other than 0 or 1, a transfer other than 2 or 3, or a
/// block count above 16; rejected when the setting has [`ZMIDI_USB_MAX_ENDPOINTS`] endpoints. The
/// writers check the other rules.
///
/// # Safety
/// `f` is null or was set up; `endpoint` is null or valid.
#[no_mangle]
pub unsafe extern "C" fn zmidi_usb_function_add_endpoint(
    f: *mut ZmidiUsbFunction,
    alt: u8,
    endpoint: *const ZmidiUsbEndpoint,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let (Some(e), Some(s)) = (unsafe { endpoint.as_ref() }, unsafe { setting_mut(f, alt) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    let direction = match e.direction {
        0 => EndpointDirection::MidiOut,
        1 => EndpointDirection::MidiIn,
        _ => return ZmidiStatus::InvalidArgument,
    };
    let transfer = match e.transfer {
        2 => TransferType::Bulk,
        3 => TransferType::Interrupt,
        _ => return ZmidiStatus::InvalidArgument,
    };
    let Some(ids) = e.block_ids.get(..usize::from(e.block_count)) else {
        return ZmidiStatus::InvalidArgument;
    };
    let Some(blocks) = GtbIds::from_slice(ids) else {
        return ZmidiStatus::InvalidArgument;
    };
    let config = EndpointConfig {
        direction,
        number: e.number,
        transfer,
        max_packet: e.max_packet,
        interval: e.interval,
        blocks,
    };
    match s.endpoints.push(config) {
        Ok(()) => ZmidiStatus::Ok,
        Err(_) => ZmidiStatus::Rejected,
    }
}

/// Replaces the Group Terminal Blocks of USB MIDI 2.0 Alternate Setting `alt` with the `count`
/// blocks of `blocks`, in this order. Invalid argument for a null pointer or no such setting;
/// rejected for more than [`ZMIDI_USB_MAX_BLOCKS`] blocks. The writers check the block rules.
///
/// # Safety
/// `f` is null or was set up; `blocks` points to `count` blocks (or is null when `count` is 0).
#[no_mangle]
pub unsafe extern "C" fn zmidi_usb_function_set_blocks(
    f: *mut ZmidiUsbFunction,
    alt: u8,
    blocks: *const ZmidiUsbBlock,
    count: usize,
) -> ZmidiStatus {
    if blocks.is_null() && count > 0 {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let Some(s) = (unsafe { setting_mut(f, alt) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    if count > ZMIDI_USB_MAX_BLOCKS {
        return ZmidiStatus::Rejected;
    }
    let src = if count == 0 {
        &[][..]
    } else {
        // SAFETY: caller contract above.
        unsafe { core::slice::from_raw_parts(blocks, count) }
    };
    let mut list = GtbList::new();
    for b in src {
        let block = GroupTerminalBlock {
            id: b.id,
            kind: GtbType::from_bits(b.kind),
            first_group: b.first_group,
            num_groups: b.num_groups,
            string_index: b.string_index,
            protocol: MidiProtocol::from_bits(b.protocol),
            max_input_bandwidth: b.max_input_bandwidth,
            max_output_bandwidth: b.max_output_bandwidth,
        };
        if list.push(block).is_err() {
            return ZmidiStatus::Rejected;
        }
    }
    s.blocks = list;
    ZmidiStatus::Ok
}

/// Gives in `len` the number of bytes of [`zmidi_usb_function_write`], for wTotalLength of the
/// Configuration descriptor. It checks the same rules as the writer: rejected for a broken rule.
///
/// # Safety
/// `f` is null or was set up; `len` is writable.
#[no_mangle]
pub unsafe extern "C" fn zmidi_usb_function_len(
    f: *mut ZmidiUsbFunction,
    len: *mut usize,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let (false, Some(cfg)) = (len.is_null(), unsafe { function_mut(f) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    unsafe { give(function_len(cfg), len) }
}

/// Writes the descriptors of the function into `out` (USB-MIDI v2.0 Appendix B order: the
/// AudioControl interface, then Alternate Setting 0 and the USB MIDI 2.0 settings) and gives the
/// number of bytes in `written`. No Interface Association Descriptor and no Group Terminal Block
/// descriptor. Rejected for a broken rule, buffer too small for a short `out`; nothing is
/// written then.
///
/// # Safety
/// `f` is null or was set up; `out` points to `capacity` writable bytes; `written` is writable.
#[no_mangle]
pub unsafe extern "C" fn zmidi_usb_function_write(
    f: *mut ZmidiUsbFunction,
    out: *mut u8,
    capacity: usize,
    written: *mut usize,
) -> ZmidiStatus {
    if out.is_null() || written.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let Some(cfg) = (unsafe { function_mut(f) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    let out = unsafe { core::slice::from_raw_parts_mut(out, capacity) };
    // SAFETY: caller contract above.
    unsafe { give(write_function(cfg, out), written) }
}

/// Gives the number of endpoints that the writer writes (all Alternate Settings), or 0 for a null
/// pointer. The list is valid for a function that [`zmidi_usb_function_len`] accepts.
///
/// # Safety
/// `f` is null or was set up.
#[no_mangle]
pub unsafe extern "C" fn zmidi_usb_function_endpoint_count(f: *mut ZmidiUsbFunction) -> usize {
    // SAFETY: caller contract above.
    unsafe { function_mut(f) }.map_or(0, |cfg| cfg.endpoints().count())
}

/// Gives endpoint `index` (0 to the count - 1) in descriptor order, so that the USB stack can
/// allocate the endpoints before it writes the descriptors. One endpoint address can come in more
/// than one Alternate Setting. Invalid argument for an index past the end.
///
/// # Safety
/// `f` is null or was set up; `endpoint` is writable.
#[no_mangle]
pub unsafe extern "C" fn zmidi_usb_function_endpoint(
    f: *mut ZmidiUsbFunction,
    index: usize,
    endpoint: *mut ZmidiUsbSettingEndpoint,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let (false, Some(cfg)) = (endpoint.is_null(), unsafe { function_mut(f) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    let Some(e) = cfg.endpoints().nth(index) else {
        return ZmidiStatus::InvalidArgument;
    };
    let out = ZmidiUsbSettingEndpoint {
        alternate_setting: e.alternate_setting,
        address: e.address(),
        transfer: e.transfer.attributes(),
        interval: e.interval,
        max_packet: e.max_packet,
    };
    // SAFETY: caller contract above.
    unsafe { endpoint.write(out) };
    ZmidiStatus::Ok
}

/// Gives the Group Terminals of the endpoints of `direction` (0 MIDI OUT, 1 MIDI IN) of
/// Alternate Setting `alt` as a mask (bit n is Group n), for [`zmidi_usb_link_set_alternate_setting`]:
/// all 16 for Alternate Setting 0, 0 for a setting that does not exist or a null pointer
/// (REQ-USB-GTB-014).
///
/// # Safety
/// `f` is null or was set up.
#[no_mangle]
pub unsafe extern "C" fn zmidi_usb_function_group_mask(
    f: *mut ZmidiUsbFunction,
    alt: u8,
    direction: u8,
) -> u16 {
    // SAFETY: caller contract above.
    let Some(cfg) = (unsafe { function_mut(f) }) else {
        return 0;
    };
    let direction = match direction {
        0 => EndpointDirection::MidiOut,
        1 => EndpointDirection::MidiIn,
        _ => return 0,
    };
    if alt == 0 {
        return GroupMask::ALL.bits();
    }
    cfg.midi2_setting(alt).map_or(0, |s| {
        s.endpoints
            .iter()
            .filter(|e| e.direction == direction)
            .fold(0, |bits, e| {
                bits | GroupMask::from_blocks(&s.blocks, e).bits()
            })
    })
}

/// Gives in `len` the length of the Group Terminal Block descriptor set of USB MIDI 2.0 Alternate
/// Setting `alt` (its wTotalLength). Invalid argument for Alternate Setting 0 or no such setting.
///
/// # Safety
/// `f` is null or was set up; `len` is writable.
#[no_mangle]
pub unsafe extern "C" fn zmidi_usb_gtb_set_len(
    f: *mut ZmidiUsbFunction,
    alt: u8,
    len: *mut usize,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let (false, Some(cfg)) = (len.is_null(), unsafe { function_mut(f) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    unsafe { give(gtb_set_len(cfg, alt), len) }
}

/// Writes the Group Terminal Block descriptor set of USB MIDI 2.0 Alternate Setting `alt` (the
/// header, then the blocks) into `out` and gives its length in `written` (REQ-USB-GTB-015).
/// Invalid argument for Alternate Setting 0 or no such setting, rejected for a broken block rule,
/// buffer too small for a short `out`; nothing is written then.
///
/// # Safety
/// `f` is null or was set up; `out` points to `capacity` writable bytes; `written` is writable.
#[no_mangle]
pub unsafe extern "C" fn zmidi_usb_gtb_set_write(
    f: *mut ZmidiUsbFunction,
    alt: u8,
    out: *mut u8,
    capacity: usize,
    written: *mut usize,
) -> ZmidiStatus {
    if out.is_null() || written.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let Some(cfg) = (unsafe { function_mut(f) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    let out = unsafe { core::slice::from_raw_parts_mut(out, capacity) };
    // SAFETY: caller contract above.
    unsafe { give(gtb_set(cfg, alt, out), written) }
}

/// The device-side handler of the Group Terminal Block descriptors request (USB-MIDI v2.0
/// section 6). `setup` is the 8 bytes of the setup packet. `response` gives what to do:
/// [`ZmidiUsbGtbResponse::Answered`] with the first min(wLength, wTotalLength) bytes of the set in
/// `out` and their number in `written`, [`ZmidiUsbGtbResponse::Stall`] for another interface,
/// Alternate Setting 0 or a setting that does not exist, and [`ZmidiUsbGtbResponse::NotHandled`]
/// for another request (`written` is 0 for both). Buffer too small for a short `out`, rejected for
/// a broken block rule; nothing is written then.
///
/// # Safety
/// `f` is null or was set up; `setup` points to 8 bytes; `out` points to `capacity` writable
/// bytes; `response` and `written` are writable.
#[no_mangle]
pub unsafe extern "C" fn zmidi_usb_gtb_setup(
    f: *mut ZmidiUsbFunction,
    setup: *const u8,
    out: *mut u8,
    capacity: usize,
    response: *mut ZmidiUsbGtbResponse,
    written: *mut usize,
) -> ZmidiStatus {
    if setup.is_null() || out.is_null() || response.is_null() || written.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let Some(cfg) = (unsafe { function_mut(f) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    let (setup, out) = unsafe {
        (
            &*setup.cast::<[u8; ZMIDI_USB_SETUP_LEN]>(),
            core::slice::from_raw_parts_mut(out, capacity),
        )
    };
    let (r, n) = match handle_gtb_request(setup, cfg, out) {
        Ok(GtbResponse::Answered(n)) => (ZmidiUsbGtbResponse::Answered, n),
        Ok(GtbResponse::Stall) => (ZmidiUsbGtbResponse::Stall, 0),
        Ok(GtbResponse::NotHandled) => (ZmidiUsbGtbResponse::NotHandled, 0),
        Err(e) => return request_status(e),
    };
    // SAFETY: caller contract above.
    unsafe {
        response.write(r);
        written.write(n);
    }
    ZmidiStatus::Ok
}

/// The device-side handler of the Group Terminal Block descriptors request for a fixed function
/// (OPT-038): the Group Terminal Block sets are C arrays that a host program wrote before the
/// build (`zmidi_usb_gen.h`), so the firmware links no descriptor writer and no rule check.
/// `sets[i]` is the set of USB MIDI 2.0 Alternate Setting `i + 1` with `lens[i]` bytes, for `count`
/// settings (0 to [`ZMIDI_USB_MAX_SETTINGS`]). The rules and the results are the ones of
/// [`zmidi_usb_gtb_setup`] (zmidi-usb `FixedFunction::gtb_setup`): answered with the first
/// min(wLength, set length) bytes, stall for another interface than `ms_interface` or a setting with
/// no set, not handled for another request. Invalid argument for a null pointer or more than
/// [`ZMIDI_USB_MAX_SETTINGS`] sets; buffer too small for a short `out`, and nothing is written then.
///
/// # Safety
/// `sets` and `lens` point to `count` entries (or are null when `count` is 0), and `sets[i]` points
/// to `lens[i]` bytes; `setup` points to 8 bytes; `out` points to `capacity` writable bytes;
/// `response` and `written` are writable.
#[no_mangle]
pub unsafe extern "C" fn zmidi_usb_fixed_gtb_setup(
    sets: *const *const u8,
    lens: *const usize,
    count: usize,
    ms_interface: u8,
    setup: *const u8,
    out: *mut u8,
    capacity: usize,
    response: *mut ZmidiUsbGtbResponse,
    written: *mut usize,
) -> ZmidiStatus {
    if count > ZMIDI_USB_MAX_SETTINGS
        || (count > 0 && (sets.is_null() || lens.is_null()))
        || setup.is_null()
        || out.is_null()
        || response.is_null()
        || written.is_null()
    {
        return ZmidiStatus::InvalidArgument;
    }
    let mut table: [&[u8]; ZMIDI_USB_MAX_SETTINGS] = [&[]; ZMIDI_USB_MAX_SETTINGS];
    for (i, slot) in table.iter_mut().enumerate().take(count) {
        // SAFETY: caller contract above.
        let (p, n) = unsafe { (*sets.add(i), *lens.add(i)) };
        if p.is_null() && n > 0 {
            return ZmidiStatus::InvalidArgument;
        }
        // SAFETY: caller contract above.
        *slot = unsafe { bytes(p, n) };
    }
    // SAFETY: `FixedFunction` takes 'static tables because a build script makes them constants.
    // Here the function is a local value that does not leave this call, so the borrows of the
    // caller arrays and of `table` are valid for its whole life.
    let gtb_sets: &'static [&'static [u8]] =
        unsafe { core::slice::from_raw_parts(table.as_ptr().cast(), count) };
    // No endpoints: `new` has nothing to refuse.
    let Ok(f) = FixedFunction::new(&[], &[], &[], gtb_sets) else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    let (setup, out) = unsafe {
        (
            &*setup.cast::<[u8; ZMIDI_USB_SETUP_LEN]>(),
            core::slice::from_raw_parts_mut(out, capacity),
        )
    };
    let (r, n) = match f.gtb_setup(&SetupPacket::from_bytes(setup), ms_interface, out) {
        Ok(GtbResponse::Answered(n)) => (ZmidiUsbGtbResponse::Answered, n),
        Ok(GtbResponse::Stall) => (ZmidiUsbGtbResponse::Stall, 0),
        Ok(GtbResponse::NotHandled) => (ZmidiUsbGtbResponse::NotHandled, 0),
        Err(e) => return request_status(e),
    };
    // SAFETY: caller contract above.
    unsafe {
        response.write(r);
        written.write(n);
    }
    ZmidiStatus::Ok
}

/// The status of a request error.
fn request_status(e: RequestError) -> ZmidiStatus {
    match e {
        RequestError::Overflow => ZmidiStatus::BufferTooSmall,
        RequestError::Truncated => ZmidiStatus::InvalidArgument,
        RequestError::Device(e) => device_status(e),
        RequestError::NoTotalLength(_) => ZmidiStatus::Rejected,
    }
}

/// The host side (REQ-USB-GTB-020): writes the 8 setup packet bytes of the Group Terminal Block
/// descriptors request for Alternate Setting `alt`, interface `interface` and wLength `length`
/// into `out`. First send wLength 5 (the header only), then wLength = the wTotalLength that
/// [`zmidi_usb_gtb_total_length`] reads from the answer.
///
/// # Safety
/// `out` is null or points to 8 writable bytes.
#[no_mangle]
pub unsafe extern "C" fn zmidi_usb_gtb_request(
    alt: u8,
    interface: u8,
    length: u16,
    out: *mut u8,
) -> ZmidiStatus {
    if out.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let out = unsafe { core::slice::from_raw_parts_mut(out, ZMIDI_USB_SETUP_LEN) };
    match write_gtb_request(alt, interface, length, out) {
        Ok(_) => ZmidiStatus::Ok,
        Err(e) => request_status(e),
    }
}

/// The host side: gives in `total` wTotalLength of the Group Terminal Block Header at the start of
/// `answer` (the answer can be the 5-byte header only). Rejected when the answer has no readable
/// header or a wTotalLength below 5.
///
/// # Safety
/// `answer` points to `len` bytes (or is null when `len` is 0); `total` is writable.
#[no_mangle]
pub unsafe extern "C" fn zmidi_usb_gtb_total_length(
    answer: *const u8,
    len: usize,
    total: *mut u16,
) -> ZmidiStatus {
    if (answer.is_null() && len > 0) || total.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let answer = unsafe { bytes(answer, len) };
    match answer_total_length(answer) {
        // SAFETY: caller contract above.
        Ok(t) => unsafe {
            total.write(t);
            ZmidiStatus::Ok
        },
        Err(e) => request_status(e),
    }
}

/// The host side: gives in `block` block `index` (0, 1, ...) of the Group Terminal Block
/// descriptor set in `answer`, with the raw field values. The reader is lenient (REQ-USB-GTB-016):
/// it skips a block shorter than 13 bytes and other descriptors, reads at most wTotalLength bytes
/// and accepts a short answer. No pending when the answer has no block `index`. The reader does
/// not check the values.
///
/// # Safety
/// `answer` points to `len` bytes (or is null when `len` is 0); `block` is writable.
#[no_mangle]
pub unsafe extern "C" fn zmidi_usb_gtb_block(
    answer: *const u8,
    len: usize,
    index: usize,
    block: *mut ZmidiUsbBlock,
) -> ZmidiStatus {
    if (answer.is_null() && len > 0) || block.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let answer = unsafe { bytes(answer, len) };
    let Some(b) = GtbReader::new(answer).filter_map(Result::ok).nth(index) else {
        return ZmidiStatus::NoPending;
    };
    let out = ZmidiUsbBlock {
        id: b.id,
        kind: b.kind.into_bits(),
        first_group: b.first_group,
        num_groups: b.num_groups,
        string_index: b.string_index,
        protocol: b.protocol.into_bits(),
        max_input_bandwidth: b.max_input_bandwidth,
        max_output_bandwidth: b.max_output_bandwidth,
    };
    // SAFETY: caller contract above.
    unsafe { block.write(out) };
    ZmidiStatus::Ok
}

/// The slice of `len` bytes at `p`; an empty slice when `len` is 0.
///
/// # Safety
/// `p` points to `len` bytes, or `len` is 0.
unsafe fn bytes<'a>(p: *const u8, len: usize) -> &'a [u8] {
    if len == 0 {
        &[]
    } else {
        // SAFETY: caller contract above.
        unsafe { core::slice::from_raw_parts(p, len) }
    }
}

/// Gives the size in bytes of the Rust function configuration (DEC-019 rule 6).
#[no_mangle]
pub extern "C" fn zmidi_usb_function_size() -> usize {
    size_of::<Function>()
}
