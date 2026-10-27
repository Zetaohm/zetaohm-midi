//! The automatic replies of a Device fit in `CiDevice::MAX_REPLY_LEN` bytes (REQ-CI-LEN-004,
//! OPT-049 step 5): a responder needs a reply buffer of that size, not the Receivable Maximum
//! SysEx Message Size. With one Profile the longest is the Reply to Endpoint Information (Table 11) with a
//! 42-byte Product Instance ID (REQ-CI-EPI-007) and System Exclusive framing: 2 + 13 + 3 + 42 = 60 bytes.

use proptest::prelude::*;
use zmidi_ci::profile::{ProfileAddress, ProfileEntry, ProfileId, ProfileLevel};
use zmidi_ci::{
    Categories, CiDevice, DeviceConfig, DeviceId, DeviceIdentity, Framing, ManufacturerId, Muid,
    PiCapabilities, WriteError,
};

const PID: &str = "~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~"; // 42 bytes, the maximum

fn device_config() -> DeviceConfig<'static> {
    DeviceConfig {
        identity: DeviceIdentity {
            manufacturer: ManufacturerId::ThreeByte([0x00, 0x21, 0x09]),
            family: 0x3FFF,
            model: 0x3FFF,
            revision: [0x7F; 4],
        },
        categories: Categories {
            profile_configuration: true,
            property_exchange: true,
            process_inquiry: true,
            ..Default::default()
        },
        max_sysex_size: 512,
        function_block: Some(1),
        first_group: 0,
        group_count: 1,
        channels_in_use: 0xFFFF,
        product_instance_id: Some(PID),
        framing: Framing::SysEx,
        process_inquiry: PiCapabilities {
            midi_message_report: true,
        },
    }
}

fn device() -> CiDevice<'static, 2, 1, 4> {
    let mut d = CiDevice::new(device_config(), &mut || 0x0222_2222).unwrap();
    let entry = ProfileEntry {
        address: ProfileAddress {
            device_id: DeviceId::Channel(0),
            group: 0,
        },
        profile: ProfileId::Standard {
            bank: 0x21,
            number: 0,
            version: 1,
            level: ProfileLevel::Minimum,
        },
        enabled: true,
        channels: 1,
        max_channels: 16,
    };
    // Without the feature `profiles` the call can fail; the replies are then fewer.
    if let Ok(r) = d.add_profile(entry) {
        let _ = r.count();
    }
    d
}

fn muid_bytes(m: u32) -> [u8; 4] {
    [
        (m & 0x7F) as u8,
        ((m >> 7) & 0x7F) as u8,
        ((m >> 14) & 0x7F) as u8,
        ((m >> 21) & 0x7F) as u8,
    ]
}

#[test]
fn prop_automatic_replies_fit_in_max_reply_len() {
    // Messages to the Device or to the Broadcast MUID, with each Sub-ID#2 and any body.
    proptest!(|(sub in prop_oneof![0x70u8..0x80, 0x20u8..0x30, 0x30u8..0x40, 0x40u8..0x45, any::<u8>()],
                dev in prop_oneof![Just(0x7Fu8), Just(0x7E), 0u8..0x10],
                version in prop_oneof![Just(0x02u8), Just(0x01), 0u8..0x80],
                src in 1u32..0x0FFF_FF00, to_me in any::<bool>(),
                body in proptest::collection::vec(0u8..0x80, 0..48))| {
        let mut d = device();
        let dst = if to_me { d.muid().value() } else { Muid::BROADCAST.value() };
        let mut msg = vec![0x7E, dev, 0x0D, sub & 0x7F, version];
        msg.extend(muid_bytes(src));
        msg.extend(muid_bytes(dst));
        msg.extend(&body);
        let mut r = d.receive(0, &msg, 0, &mut || 7);
        let mut out = [0u8; CiDevice::<2, 1, 4>::MAX_REPLY_LEN];
        while let Some(item) = r.write_next(&mut out) {
            prop_assert_ne!(item.map(|(_, n)| n), Err(WriteError::Overflow), "sub {:#04x}", sub);
        }
    });
}

#[test]
fn longest_automatic_reply_is_60_bytes() {
    // One valid inquiry of each kind that gets an automatic reply (M2-101 Tables 6, 9, 12, 17, 22,
    // 30, 40), with the bodies of those tables.
    let mut d = device();
    let src = muid_bytes(0x0111_1111);
    let me = muid_bytes(d.muid().value());
    let msg = |dev: u8, sub: u8, dst: [u8; 4], body: &[u8]| {
        let mut m = vec![0x7E, dev, 0x0D, sub, 0x02];
        m.extend(src);
        m.extend(dst);
        m.extend(body);
        m
    };
    let discovery_body = [
        0x41, 0, 0, 1, 0, 2, 0, 0, 0, 0, 1, 0x1C, 0x00, 0x04, 0, 0, 0x05,
    ];
    let inquiries = [
        msg(0x7F, 0x70, [0x7F; 4], &discovery_body),
        msg(0x7F, 0x72, me, &[0x00]),
        msg(0x7F, 0x30, me, &[0x04, 0x00, 0x00]),
        msg(0x7F, 0x40, me, &[]),
        msg(0x00, 0x20, me, &[]),
        msg(0x7F, 0x20, me, &[]),
        msg(0x00, 0x28, me, &[0x7E, 0x21, 0x00, 0x01, 0x01, 0x00]),
        msg(0x7F, 0x7E, [0x7F; 4], &[0x7F; 4]),
    ];
    let mut longest = 0;
    let mut replies = 0;
    for m in &inquiries {
        let mut r = d.receive(0, m, 0, &mut || 7);
        let mut out = [0u8; 64];
        while let Some(item) = r.write_next(&mut out) {
            let (_, n) = item.unwrap();
            longest = longest.max(n);
            replies += 1;
        }
    }
    assert!(replies >= 5, "{replies} replies");
    assert_eq!(
        longest, 60,
        "the Reply to Endpoint Information with a 42-byte ID"
    );
}

#[test]
fn max_reply_len_value() {
    assert_eq!(CiDevice::<'static, 2, 1, 4>::MAX_REPLY_LEN, 60);
    assert_eq!(CiDevice::<'static, 2, 16, 4>::MAX_REPLY_LEN, 99);
}

#[cfg(feature = "profiles")]
#[test]
fn max_reply_len_with_many_profiles() {
    // Table 18: 16 Profiles on Channel 1 of Group 1 give one reply of 2 + 13 + 4 + 16 * 5 = 99
    // bytes, the MAX_REPLY_LEN of 16 Profiles.
    let mut d: CiDevice<'static, 2, 16, 4> =
        CiDevice::new(device_config(), &mut || 0x0222_2222).unwrap();
    for i in 0..16u8 {
        let entry = ProfileEntry {
            address: ProfileAddress {
                device_id: DeviceId::Channel(0),
                group: 0,
            },
            profile: ProfileId::Standard {
                bank: 0x21,
                number: i,
                version: 1,
                level: ProfileLevel::Minimum,
            },
            enabled: i % 2 == 0,
            channels: u16::from(i % 2 == 0),
            max_channels: 16,
        };
        let _ = d.add_profile(entry).unwrap().count();
    }
    let mut m = vec![0x7E, 0x00, 0x0D, 0x20, 0x02];
    m.extend(muid_bytes(0x0111_1111));
    m.extend(muid_bytes(d.muid().value()));
    let mut r = d.receive(0, &m, 0, &mut || 7);
    let mut out = [0u8; CiDevice::<'static, 2, 16, 4>::MAX_REPLY_LEN];
    let (_, n) = r.write_next(&mut out).unwrap().unwrap();
    assert_eq!(n, 99);
}
