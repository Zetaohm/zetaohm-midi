// Made by zmidi_usb::fixed::write_rust (DEC-USB-002); do not edit.
pub static MIDI: ::zmidi_usb::fixed::FixedFunction = match ::zmidi_usb::fixed::FixedFunction::new(
    &[9, 4, 0, 0, 0, 1, 1, 0, 0, 9, 36, 1, 0, 1, 9, 0, 1, 1, 9, 4, 1, 0, 2, 1, 3, 0, 0, 7, 36, 1, 0, 1, 65, 0, 6, 36, 2, 1, 1, 0, 6, 36, 2, 2, 2, 0, 9, 36, 3, 1, 3, 1, 2, 1, 0, 9, 36, 3, 2, 4, 1, 1, 1, 0, 9, 5, 1, 2, 64, 0, 0, 0, 0, 5, 37, 1, 1, 1, 9, 5, 129, 2, 64, 0, 0, 0, 0, 5, 37, 1, 1, 3, 9, 4, 1, 1, 2, 1, 3, 0, 0, 7, 36, 1, 0, 2, 7, 0, 7, 5, 2, 2, 64, 0, 0, 5, 37, 2, 1, 1, 7, 5, 130, 2, 64, 0, 0, 5, 37, 2, 1, 1],
    &[
        ::zmidi_usb::device::SettingEndpoint { alternate_setting: 0, direction: ::zmidi_usb::EndpointDirection::MidiOut, number: 1, transfer: ::zmidi_usb::ms2::TransferType::Bulk, max_packet: 64, interval: 0 },
        ::zmidi_usb::device::SettingEndpoint { alternate_setting: 0, direction: ::zmidi_usb::EndpointDirection::MidiIn, number: 1, transfer: ::zmidi_usb::ms2::TransferType::Bulk, max_packet: 64, interval: 0 },
        ::zmidi_usb::device::SettingEndpoint { alternate_setting: 1, direction: ::zmidi_usb::EndpointDirection::MidiOut, number: 2, transfer: ::zmidi_usb::ms2::TransferType::Bulk, max_packet: 64, interval: 0 },
        ::zmidi_usb::device::SettingEndpoint { alternate_setting: 1, direction: ::zmidi_usb::EndpointDirection::MidiIn, number: 2, transfer: ::zmidi_usb::ms2::TransferType::Bulk, max_packet: 64, interval: 0 },
    ],
    &[::zmidi_usb::GroupMask::from_bits(0xFFFF), ::zmidi_usb::GroupMask::from_bits(0x0001)],
    &[&[5, 38, 1, 18, 0, 13, 38, 2, 1, 0, 0, 1, 0, 17, 0, 0, 0, 0], ],
) {
    Ok(f) => f,
    Err(_) => panic!("zmidi_usb::fixed: the function breaks an adapter rule"),
};
