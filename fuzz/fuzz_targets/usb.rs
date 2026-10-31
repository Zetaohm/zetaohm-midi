//! USB data from a host or a device: the bulk data of `MidiLink` at each Alternate Setting, and
//! the configuration descriptor reader of the host side.
#![no_main]

use libfuzzer_sys::fuzz_target;
use zmidi_usb::{read_configuration, LinkConfig, LinkEvent, MidiLink, SettingNumber};

fuzz_target!(|data: &[u8]| {
    let Some((&mode, rest)) = data.split_first() else {
        return;
    };
    let mut l = MidiLink::<2>::new(LinkConfig::default()).unwrap();
    l.report(LinkEvent::AlternateSetting(SettingNumber(mode % 3)));
    for transfer in zmidi_fuzz::parts(rest) {
        assert!(l.read(transfer).count() <= transfer.len() + 1);
    }
    let f = read_configuration(rest);
    let _ = (
        f.problems().count(),
        f.audio_control().count(),
        f.ms_interface_numbers().count(),
    );
    for s in f.settings() {
        let _ = (
            s.problems().count(),
            s.jacks().count(),
            s.elements().count(),
        );
        let _ = (s.ms1_endpoints().count(), s.transfer_endpoints().count());
        let _ = (s.ms1_problems().count(), s.block_ids().count());
        let _ = s.ms2_endpoints().count();
        let _ = s.ms2_setting::<16>();
    }
});
