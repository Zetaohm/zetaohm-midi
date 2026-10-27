//! Integration tests of the MIDI-CI Device (M2-101-UM v1.2.1): two Devices exchange messages
//! through caller buffers, as an application does. One Device is the Initiator, so these tests need
//! the feature `initiator`; `tests/responder_only.rs` tests the build without it (DEC-CI-001).
#![cfg(feature = "initiator")]
// DEC-017: some imports are used only by the tests of a category whose feature is on.
#![cfg_attr(not(feature = "full"), allow(unused_imports))]

use zmidi_ci::management::{AckStatus, Details, ManufacturerId, Nak, NakStatus};
use zmidi_ci::process::{
    ChannelBits, DataControl, MessageReportInquiry, MessageReportReply, PiCapabilities, Protocol,
    ReportSource,
};
use zmidi_ci::profile::{
    ChannelCounts, ProfileAddress, ProfileEntry, ProfileId, ProfileLevel, ProfileReport,
};
use zmidi_ci::property::{ChunkEnd, Direction, PeKind};
use zmidi_ci::{
    Categories, CiDevice, DeviceConfig, DeviceError, DeviceId, DeviceIdentity, Event, Framing,
    Header, Muid, ReportKind, Version,
};
use zmidi_ump::{Group, SysEx7Event, SysEx7Packets, SysEx7Reassembler, UmpWords};

type Dev = CiDevice<'static, 4, 4, 2>;

fn identity(m: u8) -> DeviceIdentity {
    DeviceIdentity {
        manufacturer: ManufacturerId::OneByte(m),
        family: 1,
        model: 2,
        revision: [0, 0, 0, 1],
    }
}

fn config(m: u8) -> DeviceConfig<'static> {
    DeviceConfig {
        identity: identity(m),
        categories: Categories {
            profile_configuration: true,
            property_exchange: true,
            process_inquiry: true,
            ..Default::default()
        },
        max_sysex_size: 512,
        function_block: Some(1),
        first_group: 0,
        group_count: 2,
        channels_in_use: 0x0003,
        product_instance_id: Some("SN-42"),
        framing: Framing::Bare,
        process_inquiry: PiCapabilities {
            midi_message_report: true,
        },
    }
}

/// A random closure that gives `values` in turn.
fn rng(values: &'static [u32]) -> impl FnMut() -> u32 {
    let mut i = 0;
    move || {
        let v = values[i % values.len()];
        i += 1;
        v
    }
}

fn dev(m: u8, seed: &'static [u32]) -> Dev {
    CiDevice::new(config(m), &mut rng(seed)).unwrap()
}

/// Delivers `msg` on `group` and collects the event and the replies.
fn deliver<'m>(d: &mut Dev, group: u8, msg: &'m [u8]) -> (Option<Event<'m>>, Vec<(u8, Vec<u8>)>) {
    let mut r = d.receive(group, msg, 0, &mut rng(&[0x0123_4567, 0x0234_5678]));
    let ev = r.event();
    let mut replies = Vec::new();
    let mut buf = [0u8; 512];
    while let Some(x) = r.write_next(&mut buf) {
        let (g, n) = x.unwrap();
        replies.push((g, buf[..n].to_vec()));
    }
    (ev, replies)
}

fn header(msg: &[u8]) -> Header {
    Header::parse(msg).unwrap().0
}

fn sub(msg: &[u8]) -> u8 {
    header(msg).sub_id2
}

fn nak_of(msg: &[u8]) -> Nak<'_> {
    let (h, body) = Header::parse(msg).unwrap();
    assert_eq!(h.sub_id2, 0x7F, "not a NAK");
    Nak::read(&h, body).unwrap()
}

/// A discovers B; gives both with B known to A and A known to B.
fn discovered() -> (Dev, Dev) {
    let mut a = dev(0x41, &[0x0111_1111]);
    let mut b = dev(0x42, &[0x0222_2222]);
    let mut buf = [0u8; 512];
    let n = a.discovery(0, 0, &mut buf).unwrap();
    let (_, replies) = deliver(&mut b, 0, &buf[..n]);
    let (ev, _) = deliver(&mut a, 0, &replies[0].1);
    assert!(matches!(ev, Some(Event::Discovered { .. })));
    (a, b)
}

#[test]
fn discovery_and_reply() {
    // 5.5, 5.6, Tables 6 and 8; REQ-CI-DISC-001 to -003, -013, -014, -016, -017.
    let mut a = dev(0x41, &[0x0111_1111]);
    let mut b = dev(0x42, &[0x0222_2222]);
    assert_ne!(a.muid(), b.muid());
    let mut buf = [0u8; 512];
    let n = a.discovery(5, 0, &mut buf).unwrap();
    let h = header(&buf[..n]);
    assert_eq!(
        (h.sub_id2, h.source, h.destination),
        (0x70, a.muid(), Muid::BROADCAST)
    );
    let (ev, replies) = deliver(&mut b, 0, &buf[..n]);
    assert!(ev.is_none());
    assert_eq!(replies.len(), 1);
    let r = &replies[0].1;
    let h = header(r);
    assert_eq!(
        (h.sub_id2, h.source, h.destination),
        (0x71, b.muid(), a.muid())
    );
    // Output Path ID of the Discovery, then the Function Block number.
    assert_eq!(&r[r.len() - 2..], &[5, 1]);
    assert!(b.peer(a.muid()).is_some());
    let (ev, _) = deliver(&mut a, 0, r);
    match ev {
        Some(Event::Discovered { muid, reply }) => {
            assert_eq!(muid, b.muid());
            assert_eq!(reply.function_block, Some(1));
            assert_eq!(reply.identity.manufacturer, ManufacturerId::OneByte(0x42));
        }
        e => panic!("{e:?}"),
    }
    assert_eq!(a.peer(b.muid()).unwrap().max_sysex_size, 512);
}

#[test]
fn version1_discovery_gets_path_zero() {
    // REQ-CI-DISC-013: Output Path ID 0 for a version 1 Discovery.
    let mut a = dev(0x41, &[0x0111_1111]);
    let mut b = dev(0x42, &[0x0222_2222]);
    let mut buf = [0u8; 512];
    let n = a.discovery(7, 0, &mut buf).unwrap();
    buf[4] = 0x01;
    let (_, replies) = deliver(&mut b, 0, &buf[..n - 1]);
    let r = &replies[0].1;
    assert_eq!(&r[r.len() - 2..], &[0, 1]);
    assert_eq!(header(r).version, Version::CURRENT, "REQ-CI-MSG-007, -008");
}

#[test]
fn discovery_window_three_seconds() {
    // REQ-CI-DISC-011.
    let mut a = dev(0x41, &[0x0111_1111]);
    let mut buf = [0u8; 512];
    a.discovery(0, 1_000_000, &mut buf).unwrap();
    assert_eq!(a.poll(3_999_999), None);
    assert_eq!(a.poll(4_000_000), Some(Event::DiscoveryEnded));
    assert_eq!(a.poll(9_000_000), None);
}

#[test]
fn echo_is_ignored_and_collision_option_a() {
    // DEC-011 D-5, D-6; REQ-CI-MUID-012, REQ-CI-MUID-013, REQ-CI-MUID-011.
    let mut a = dev(0x41, &[0x0111_1111]);
    let mut buf = [0u8; 512];
    let n = a.discovery(0, 0, &mut buf).unwrap();
    let echo = buf[..n].to_vec();
    let (ev, replies) = deliver(&mut a, 0, &echo);
    assert!(
        ev.is_none() && replies.is_empty(),
        "own Discovery is an echo"
    );
    // Another Device with the same MUID and another identity.
    let mut other = echo.clone();
    other[13] = 0x55;
    let old = a.muid();
    let (ev, replies) = deliver(&mut a, 0, &other);
    assert_eq!(ev, Some(Event::MuidChanged { old, new: a.muid() }));
    assert_ne!(a.muid(), old);
    assert_eq!(sub(&replies[0].1), 0x71);
    assert_eq!(header(&replies[0].1).source, a.muid());
}

#[test]
fn collision_option_b_after_use() {
    // REQ-CI-MUID-014: the MUID was used in a Transaction (one of every feature set).
    let (mut a, b) = discovered();
    let mut buf = [0u8; 512];
    a.endpoint_info_inquiry(b.muid(), &mut buf).unwrap();
    let n = a.discovery(0, 0, &mut buf).unwrap();
    let mut other = buf[..n].to_vec();
    other[13] = 0x55;
    let old = a.muid();
    let (ev, replies) = deliver(&mut a, 0, &other);
    assert!(matches!(ev, Some(Event::MuidChanged { .. })));
    let r = &replies[0].1;
    assert_eq!(sub(r), 0x7E);
    assert_eq!(&r[13..17], &old.to_bytes());
}

#[test]
fn duplicate_reply_in_one_window_invalidates() {
    // REQ-CI-MUID-015.
    let mut a = dev(0x41, &[0x0111_1111]);
    let mut b = dev(0x42, &[0x0222_2222]);
    let mut buf = [0u8; 512];
    let n = a.discovery(0, 0, &mut buf).unwrap();
    let (_, replies) = deliver(&mut b, 0, &buf[..n]);
    let (ev, _) = deliver(&mut a, 0, &replies[0].1);
    assert!(matches!(ev, Some(Event::Discovered { .. })));
    let (ev, inv) = deliver(&mut a, 0, &replies[0].1);
    assert_eq!(ev, Some(Event::PeerInvalidated { muid: b.muid() }));
    assert_eq!(sub(&inv[0].1), 0x7E);
    assert_eq!(&inv[0].1[13..17], &b.muid().to_bytes());
    assert!(a.peer(b.muid()).is_none());
}

#[test]
fn destination_filter() {
    // REQ-CI-ADDR-003; REQ-CI-MUID-007 (Broadcast reaches every MUID).
    let (mut a, b) = discovered();
    let mut buf = [0u8; 512];
    let n = a.endpoint_info_inquiry(b.muid(), &mut buf).unwrap();
    let mut msg = buf[..n].to_vec();
    msg[9..13].copy_from_slice(&Muid::new(0x0333_3333).unwrap().to_bytes());
    let mut b = b;
    let (ev, replies) = deliver(&mut b, 0, &msg);
    assert!(ev.is_none() && replies.is_empty());
}

#[cfg(feature = "property-exchange")]
#[test]
fn naks_for_bad_messages() {
    // REQ-CI-NAK-006, -008, -009, REQ-CI-MSG-015, REQ-CI-GEN-004, REQ-CI-ROUTE-001, REQ-CI-PEF-002.
    let (mut a, mut b) = discovered();
    let mut buf = [0u8; 512];
    let n = a.pe_capabilities_inquiry(b.muid(), &mut buf).unwrap();
    let good = buf[..n].to_vec();

    let mut m = good.clone();
    m[3] = 0x2A; // reserved Sub-ID#2
    let (_, r) = deliver(&mut b, 0, &m);
    assert_eq!(
        (nak_of(&r[0].1).status, nak_of(&r[0].1).original_sub_id2),
        (NakStatus::NotSupported, 0x2A)
    );

    let mut m = good.clone();
    m[4] = 0x22; // reserved version bits
    let (_, r) = deliver(&mut b, 0, &m);
    assert_eq!(nak_of(&r[0].1).status, NakStatus::VersionNotSupported);
    assert_eq!(header(&r[0].1).version, Version::CURRENT);

    let m = &good[..13]; // no data
    let (ev, r) = deliver(&mut b, 0, m);
    assert!(matches!(ev, Some(Event::Rejected { .. })));
    assert_eq!(nak_of(&r[0].1).status, NakStatus::Malformed);

    let mut m = good.clone();
    m[1] = 0x7E; // PE Capabilities to a Group
    let (_, r) = deliver(&mut b, 0, &m);
    assert_eq!(nak_of(&r[0].1).status, NakStatus::NotInUse);

    // PE data messages accept Device ID 0x0F (DEC-011 D-10).
    let (_, r) = deliver(&mut a, 0, &{
        let (_, r) = deliver(&mut b, 0, &good);
        r[0].1.clone()
    });
    assert!(r.is_empty());
    let (_, mut w) = a
        .pe_request(b.muid(), PeKind::GetInquiry, b"{}", Some(0), None)
        .unwrap();
    let n = w.write_next(&[], &mut buf).unwrap();
    let mut m = buf[..n].to_vec();
    m[1] = 0x0F;
    let (ev, r) = deliver(&mut b, 0, &m);
    assert!(matches!(ev, Some(Event::PeChunk { .. })) && r.is_empty());
    m[1] = 0x05;
    let (_, r) = deliver(&mut b, 0, &m);
    assert_eq!(nak_of(&r[0].1).status, NakStatus::NotInUse);

    // A Category that the config does not support gives NAK 0x01.
    let mut c = config(0x43);
    c.categories.process_inquiry = false;
    let mut d: Dev = CiDevice::new(c, &mut rng(&[0x0444_4444])).unwrap();
    let mut m = good.clone();
    m[3] = 0x40;
    m[9..13].copy_from_slice(&d.muid().to_bytes());
    let (_, r) = deliver(&mut d, 0, &m[..13]);
    assert_eq!(nak_of(&r[0].1).status, NakStatus::NotSupported);
    // A NAK is never answered with a NAK.
    let malformed_nak = deliver(&mut b, 0, &good[..13]).1[0].1.clone();
    let (ev, r) = deliver(&mut a, 0, &malformed_nak);
    assert!(matches!(ev, Some(Event::Nak { .. })) && r.is_empty());
}

#[test]
fn endpoint_information() {
    // 5.8; REQ-CI-EPI-001, -003, -004, -006, -007; REQ-CI-MSG-010.
    let (mut a, mut b) = discovered();
    let mut buf = [0u8; 512];
    let n = a.endpoint_info_inquiry(b.muid(), &mut buf).unwrap();
    let (_, r) = deliver(&mut b, 0, &buf[..n]);
    let (ev, _) = deliver(&mut a, 0, &r[0].1);
    match ev {
        Some(Event::EndpointInfo { reply, .. }) => {
            assert_eq!(reply.data, b"SN-42");
            assert_eq!(
                reply.status,
                zmidi_ci::EndpointInfoStatus::ProductInstanceId,
                "REQ-CI-EPI-006"
            );
        }
        e => panic!("{e:?}"),
    }
    // No Product Instance ID: NAK 0x00 with Original Sub-ID#2 0x72.
    let mut c = config(0x43);
    c.product_instance_id = None;
    let mut d: Dev = CiDevice::new(c, &mut rng(&[0x0444_4444])).unwrap();
    let mut m = buf[..n].to_vec();
    m[9..13].copy_from_slice(&d.muid().to_bytes());
    let (_, r) = deliver(&mut d, 0, &m);
    let k = nak_of(&r[0].1);
    assert_eq!((k.status, k.original_sub_id2), (NakStatus::Nak, 0x72));
    // A version 1 peer gets no Endpoint Information inquiry.
    let mut v1 = dev(0x44, &[0x0555_5555]);
    let n = v1.discovery(0, 0, &mut buf).unwrap();
    buf[4] = 1;
    let _ = deliver(&mut b, 0, &buf[..n - 1]);
    assert_eq!(
        b.endpoint_info_inquiry(v1.muid(), &mut buf),
        Err(DeviceError::PeerVersion)
    );
    assert_eq!(
        b.endpoint_info_inquiry(Muid::new(5).unwrap(), &mut buf),
        Err(DeviceError::UnknownPeer)
    );
}

#[cfg(feature = "property-exchange")]
#[test]
fn invalidate_muid_flows() {
    // 5.9, 3.3.2; REQ-CI-INV-001 to -007, REQ-CI-MUID-008, -011, REQ-CI-DISC-005.
    let (mut a, mut b) = discovered();
    let mut buf = [0u8; 512];
    // A peer MUID: B forgets A and ends its Requests.
    let n = b.pe_capabilities_inquiry(a.muid(), &mut buf).unwrap();
    let (_, r) = deliver(&mut a, 0, &buf[..n]);
    let _ = deliver(&mut b, 0, &r[0].1);
    let (id, _) = b
        .pe_request(a.muid(), PeKind::GetInquiry, b"{}", Some(0), None)
        .unwrap();
    let old_a = a.muid();
    let n = a.invalidate(&mut rng(&[0x0666_6666]), &mut buf).unwrap();
    assert_ne!(a.muid(), old_a);
    let (ev, r) = deliver(&mut b, 0, &buf[..n]);
    assert_eq!(ev, Some(Event::PeerInvalidated { muid: old_a }));
    assert!(r.is_empty(), "no reply to Invalidate MUID");
    assert!(b.peer(old_a).is_none());
    assert_eq!(
        b.pe_request(old_a, PeKind::GetInquiry, b"{}", Some(0), None)
            .err(),
        Some(DeviceError::UnknownPeer)
    );
    let _ = id;
    // This MUID: B takes a new MUID, different from the old one.
    let old_b = b.muid();
    let n = zmidi_ci::InvalidateMuid { target: old_b }
        .write(a.muid(), Framing::Bare, &mut buf)
        .unwrap();
    let (ev, r) = deliver(&mut b, 0, &buf[..n]);
    assert!(matches!(ev, Some(Event::MuidChanged { old, new }) if old == old_b && new != old_b));
    assert!(r.is_empty());
    // The Function Block moved: Invalidate MUID, new MUID, then a new Discovery.
    let before = a.muid();
    let n = a
        .function_block_changed(4, 1, &mut rng(&[0x0777_7777]), &mut buf)
        .unwrap();
    assert_eq!(&buf[13..17], &before.to_bytes());
    assert_eq!(header(&buf[..n]).destination, Muid::BROADCAST);
    assert_ne!(a.muid(), before);
    assert_eq!(a.config().first_group, 4);
    let n = a.discovery(0, 0, &mut buf).unwrap();
    assert_eq!(header(&buf[..n]).source, a.muid());
}

fn gm() -> ProfileId {
    ProfileId::Standard {
        bank: 0x21,
        number: 0,
        version: 1,
        level: ProfileLevel::Minimum,
    }
}

fn entry(enabled: bool) -> ProfileEntry {
    ProfileEntry {
        address: ProfileAddress {
            device_id: DeviceId::Channel(0),
            group: 0,
        },
        profile: gm(),
        enabled,
        channels: if enabled { 1 } else { 0 },
        max_channels: 4,
    }
}

#[cfg(feature = "profiles")]
#[test]
fn profile_configuration() {
    // 7; REQ-CI-PROF-015, -018, -025, -026, -028, REQ-CI-PRFR-002, -010 to -013, -021, REQ-CI-GEN-003.
    let (mut a, mut b) = discovered();
    let mut buf = [0u8; 512];
    // No Profiles: the Profile bit of Discovery is 0 (REQ-CI-PROF-018).
    let n = b.discovery(0, 0, &mut buf).unwrap();
    assert_eq!(buf[n - 6] & 0x04, 0);
    let reports: Vec<ProfileReport> = b.add_profile(entry(false)).unwrap().collect();
    let n = b.discovery(0, 0, &mut buf).unwrap();
    assert_eq!(buf[n - 6] & 0x04, 0x04);
    let (g, n) = b.profile_report(&reports[0], &mut buf).unwrap();
    assert_eq!(
        (g, sub(&buf[..n]), header(&buf[..n]).destination),
        (0, 0x26, Muid::BROADCAST)
    );
    let (ev, _) = deliver(&mut a, 0, &buf[..n]);
    assert!(matches!(
        ev,
        Some(Event::ProfileReport {
            kind: ReportKind::Added,
            ..
        })
    ));

    // REQ-CI-GEN-003: Profile Inquiry before Set Profile On.
    assert_eq!(
        a.set_profile_on(b.muid(), DeviceId::Channel(0), gm(), 1, None, &mut buf),
        Err(DeviceError::NotInquired)
    );
    let n = a
        .profile_inquiry(b.muid(), DeviceId::FunctionBlock, &mut buf)
        .unwrap();
    let (_, r) = deliver(&mut b, 0, &buf[..n]);
    assert_eq!(r.len(), 2, "Channel 0 reply, then 0x7F reply");
    let (ev, _) = deliver(&mut a, 0, &r[0].1);
    match ev {
        Some(Event::ProfileInquiryReply {
            device_id, reply, ..
        }) => {
            assert_eq!(device_id, DeviceId::Channel(0));
            assert_eq!(reply.disabled().collect::<Vec<_>>(), [gm()]);
        }
        e => panic!("{e:?}"),
    }
    assert_eq!(
        a.set_profile_on(b.muid(), DeviceId::Channel(0), gm(), 5, Some(4), &mut buf),
        Err(DeviceError::TooManyChannels)
    );
    let n = a
        .set_profile_on(b.muid(), DeviceId::Channel(0), gm(), 1, Some(4), &mut buf)
        .unwrap();
    assert_eq!(buf[17], 0x7F, "Profile Level 0x7F (REQ-CI-PRFR-002)");
    let (ev, r) = deliver(&mut b, 0, &buf[..n]);
    assert!(r.is_empty());
    let Some(Event::SetProfile {
        address,
        profile,
        on: true,
        channels: 1,
        ..
    }) = ev
    else {
        panic!("{ev:?}")
    };
    assert_eq!(profile.to_bytes()[4], 0x7F);
    let report = b.set_profile_state(address, gm(), true, 1).unwrap();
    let (_, n) = b.profile_report(&report, &mut buf).unwrap();
    let (ev, _) = deliver(&mut a, 0, &buf[..n]);
    assert!(matches!(
        ev,
        Some(Event::ProfileReport {
            kind: ReportKind::Enabled,
            channels: 1,
            ..
        })
    ));
    let n = a
        .set_profile_off(b.muid(), DeviceId::Channel(0), gm(), &mut buf)
        .unwrap();
    let (ev, _) = deliver(&mut b, 0, &buf[..n]);
    assert!(matches!(ev, Some(Event::SetProfile { on: false, .. })));
    // Unknown Profile: NAK 0x04 with the Profile ID in the details.
    let other = ProfileId::Standard {
        bank: 0x22,
        number: 1,
        version: 1,
        level: ProfileLevel::Highest,
    };
    let n = a
        .set_profile_on(b.muid(), DeviceId::Channel(0), other, 1, None, &mut buf)
        .unwrap();
    let (_, r) = deliver(&mut b, 0, &buf[..n]);
    let k = nak_of(&r[0].1);
    assert_eq!(
        (k.status, k.details),
        (
            NakStatus::ProfileNotSupported,
            Details::Profile(other.to_bytes())
        )
    );
    // A local action disables it: Disabled Report (REQ-CI-PROF-028).
    let report = b.set_profile_state(address, gm(), false, 1).unwrap();
    let (_, n) = b.profile_report(&report, &mut buf).unwrap();
    let (ev, _) = deliver(&mut a, 0, &buf[..n]);
    assert!(matches!(
        ev,
        Some(Event::ProfileReport {
            kind: ReportKind::Disabled,
            ..
        })
    ));
    let reports: Vec<_> = b.remove_profile(address, gm()).unwrap().collect();
    let (_, n) = b.profile_report(&reports[0], &mut buf).unwrap();
    let (ev, _) = deliver(&mut a, 0, &buf[..n]);
    assert!(matches!(
        ev,
        Some(Event::ProfileReport {
            kind: ReportKind::Removed,
            ..
        })
    ));
}

#[cfg(feature = "profiles")]
#[test]
fn profile_details() {
    // 7.6; REQ-CI-PRFR-014 to -016, -019, -020.
    let (mut a, mut b) = discovered();
    let mut buf = [0u8; 512];
    let _ = b.add_profile(entry(true)).unwrap().count();
    let n = a
        .profile_details_inquiry(b.muid(), DeviceId::Channel(0), gm(), 0, &mut buf)
        .unwrap();
    let (_, r) = deliver(&mut b, 0, &buf[..n]);
    let (ev, _) = deliver(&mut a, 0, &r[0].1);
    match ev {
        Some(Event::ProfileDetailsReply {
            reply,
            notification,
            ..
        }) => {
            assert!(!notification);
            assert_eq!(
                ChannelCounts::read(reply.data),
                Ok(ChannelCounts {
                    in_use: 1,
                    maximum: 4
                })
            );
        }
        e => panic!("{e:?}"),
    }
    // The same reply again is not requested: a Notification.
    let (ev, _) = deliver(&mut a, 0, &r[0].1);
    assert!(matches!(
        ev,
        Some(Event::ProfileDetailsReply {
            notification: true,
            ..
        })
    ));
    // Another target: an event for the application.
    let n = a
        .profile_details_inquiry(b.muid(), DeviceId::Channel(0), gm(), 0x40, &mut buf)
        .unwrap();
    let (ev, r) = deliver(&mut b, 0, &buf[..n]);
    assert!(matches!(ev, Some(Event::ProfileDetailsInquiry { target: 0x40, .. })) && r.is_empty());
    // Unknown Profile: NAK 0x04.
    let other = ProfileId::Standard {
        bank: 0x22,
        number: 1,
        version: 1,
        level: ProfileLevel::Highest,
    };
    let n = a
        .profile_details_inquiry(b.muid(), DeviceId::Channel(0), other, 0, &mut buf)
        .unwrap();
    let (_, r) = deliver(&mut b, 0, &buf[..n]);
    assert_eq!(nak_of(&r[0].1).status, NakStatus::ProfileNotSupported);
}

#[cfg(feature = "property-exchange")]
#[test]
fn property_exchange_framing() {
    // 8; REQ-CI-PEF-031 to -038, -041 to -043.
    let (mut a, mut b) = discovered();
    let mut buf = [0u8; 512];
    assert_eq!(
        a.pe_request(b.muid(), PeKind::GetInquiry, b"{}", Some(0), None)
            .err(),
        Some(DeviceError::NotInquired)
    );
    let n = a.pe_capabilities_inquiry(b.muid(), &mut buf).unwrap();
    let (_, r) = deliver(&mut b, 0, &buf[..n]);
    assert_eq!(
        &r[0].1[13..16],
        &[2, 0, 0],
        "Number of Simultaneous Requests = REQUESTS"
    );
    let (ev, _) = deliver(&mut a, 0, &r[0].1);
    assert!(matches!(ev, Some(Event::PeCapabilities { .. })));
    assert!(a.peer(b.muid()).unwrap().pe_capabilities.is_some());
    assert_eq!(
        a.pe_request(b.muid(), PeKind::Notify, b"", None, None)
            .err(),
        Some(DeviceError::NotSupported)
    );

    let header_json = br#"{"resource":"DeviceInfo"}"#;
    let (id, mut w) = a
        .pe_request(b.muid(), PeKind::GetInquiry, header_json, Some(0), None)
        .unwrap();
    let n = w.write_next(&[], &mut buf).unwrap();
    let (ev, r) = deliver(&mut b, 0, &buf[..n]);
    assert!(r.is_empty());
    let Some(Event::PeChunk {
        kind: PeKind::GetInquiry,
        chunk,
        muid,
    }) = ev
    else {
        panic!("{ev:?}")
    };
    assert_eq!(
        (chunk.request_id, chunk.header, muid),
        (id, &header_json[..], a.muid())
    );

    // B replies in several Chunks sized by A's maximum (512).
    let data: Vec<u8> = (0..1500).map(|i| (i % 100) as u8).collect();
    let mut w = b
        .pe_reply(
            a.muid(),
            PeKind::GetReply,
            id,
            br#"{"status":200}"#,
            Some(data.len()),
            None,
        )
        .unwrap();
    let mut rest = &data[..];
    let mut got = Vec::new();
    loop {
        let take = w.data_room().min(rest.len());
        let n = w.write_next(&rest[..take], &mut buf).unwrap();
        assert!(n <= 512);
        rest = &rest[take..];
        let (ev, _) = deliver(&mut a, 0, &buf[..n]);
        let Some(Event::PeChunk {
            kind: PeKind::GetReply,
            chunk,
            ..
        }) = ev
        else {
            panic!("{ev:?}")
        };
        assert_eq!(chunk.request_id, id);
        got.extend_from_slice(chunk.data);
        if chunk.end() == ChunkEnd::Complete {
            break;
        }
    }
    assert_eq!(got, data);
    b.pe_end(a.muid(), id, Direction::Responder);
    a.pe_end(b.muid(), id, Direction::Initiator);

    // REQUESTS = 2: a third open Request is Busy.
    let _ = a
        .pe_request(b.muid(), PeKind::SetInquiry, b"{}", Some(0), None)
        .unwrap();
    let _ = a
        .pe_request(b.muid(), PeKind::Subscription, b"{}", Some(0), None)
        .unwrap();
    assert_eq!(
        a.pe_request(b.muid(), PeKind::GetInquiry, b"{}", Some(0), None)
            .err(),
        Some(DeviceError::Busy)
    );

    // Notify is read and given to the PE layer (REQ-CI-PEF-043).
    let notify = zmidi_ci::PeChunk {
        request_id: 3,
        header: b"{}",
        total: 1,
        number: 1,
        data: &[],
    };
    let n = notify
        .write(PeKind::Notify, a.muid(), b.muid(), Framing::Bare, &mut buf)
        .unwrap();
    let (ev, _) = deliver(&mut b, 0, &buf[..n]);
    assert!(matches!(
        ev,
        Some(Event::PeChunk {
            kind: PeKind::Notify,
            ..
        })
    ));
    // An ACK instead of Notify (REQ-CI-PEF-042).
    let ack = zmidi_ci::Ack {
        original_sub_id2: 0x34,
        status: AckStatus::Ack,
        details: Details::PropertyExchange {
            request_id: id,
            chunk: 1,
        },
        text: &[],
    };
    let n = b
        .ack(a.muid(), DeviceId::FunctionBlock, &ack, "ok", &mut buf)
        .unwrap();
    let (ev, _) = deliver(&mut a, 0, &buf[..n]);
    assert!(matches!(ev, Some(Event::Ack { .. })));
}

#[cfg(feature = "process-inquiry")]
struct Values;

#[cfg(feature = "process-inquiry")]
impl ReportSource for Values {
    fn pitch_bend(&self, _g: u8, _c: u8) -> Option<u32> {
        Some(0x2000)
    }
}

#[cfg(feature = "process-inquiry")]
#[test]
fn process_inquiry() {
    // 9; REQ-CI-PI-003, REQ-CI-MMR-007, -010, -012, -015, -016, -036, -037.
    let (mut a, mut b) = discovered();
    let mut buf = [0u8; 512];
    let n = a.pi_capabilities_inquiry(b.muid(), &mut buf).unwrap();
    let (_, r) = deliver(&mut b, 0, &buf[..n]);
    let (ev, _) = deliver(&mut a, 0, &r[0].1);
    assert!(
        matches!(ev, Some(Event::PiCapabilities { capabilities, .. }) if capabilities.midi_message_report)
    );

    let q = MessageReportInquiry {
        data_control: DataControl::Full,
        system: Default::default(),
        channel: ChannelBits {
            pitch_bend: true,
            ..Default::default()
        },
        note: Default::default(),
    };
    // Channel 5 is not in use (mask 0x0003): NAK 0x03.
    let n = a
        .message_report_inquiry(b.muid(), DeviceId::Channel(5), &q, &mut buf)
        .unwrap();
    let (_, r) = deliver(&mut b, 0, &buf[..n]);
    assert_eq!(nak_of(&r[0].1).status, NakStatus::NotInUse);
    // A NAK ends the report Transaction of the Initiator.
    assert!(a.report_open(b.muid()));
    let _ = deliver(&mut a, 0, &r[0].1);
    assert!(!a.report_open(b.muid()));
    // Channel 1 is in use: an event, then Reply, report and End.
    let n = a
        .message_report_inquiry(b.muid(), DeviceId::Channel(1), &q, &mut buf)
        .unwrap();
    let (ev, _) = deliver(&mut b, 0, &buf[..n]);
    let Some(Event::MessageReport {
        muid,
        device_id,
        group,
        inquiry,
    }) = ev
    else {
        panic!("{ev:?}")
    };
    let supported = MessageReportReply {
        channel: ChannelBits::from_byte(0x3F),
        ..Default::default()
    };
    let (n, mut w) = b
        .message_report(
            muid,
            device_id,
            group,
            &inquiry,
            &supported,
            Protocol::Midi2,
            &Values,
            &mut buf,
        )
        .unwrap();
    assert_eq!(header(&buf[..n]).device_id, DeviceId::Channel(1));
    let (ev, _) = deliver(&mut a, 0, &buf[..n]);
    assert!(
        matches!(ev, Some(Event::MessageReportReply { reply, .. }) if reply.channel.pitch_bend)
    );
    let p = w.next_packet().unwrap();
    assert_eq!(p.words()[0] >> 28, 4);
    assert!(w.next_packet().is_none());
    let n = b.message_report_end(muid, device_id, &mut buf).unwrap();
    let (ev, _) = deliver(&mut a, 0, &buf[..n]);
    assert_eq!(ev, Some(Event::MessageReportEnd { muid: b.muid() }));
    assert!(!a.report_open(b.muid()));
    // Abort: NAK 0x20 Status Data 0x00 or 0x40.
    let n = b
        .abort_report(a.muid(), DeviceId::Group, false, &mut buf)
        .unwrap();
    assert_eq!(
        nak_of(&buf[..n]).status,
        NakStatus::Terminate {
            by_initiator: false
        }
    );
    let n = b
        .abort_report(a.muid(), DeviceId::Group, true, &mut buf)
        .unwrap();
    assert_eq!(nak_of(&buf[..n]).status, NakStatus::Error);
}

#[test]
fn ump_transport_with_512_byte_buffer() {
    // REQ-CI-LEN-001, -002: the caller collects messages in a 512-byte buffer; UMP SysEx7
    // has no F0/F7 (REQ-CI-MSG-003). A SysEx byte stream has them (REQ-CI-MSG-002).
    let (mut a, mut b) = discovered();
    let mut buf = [0u8; 512];
    let n = a.endpoint_info_inquiry(b.muid(), &mut buf).unwrap();
    let mut rx = SysEx7Reassembler::<512>::new(Group::new(1).unwrap());
    let mut done = None;
    for p in SysEx7Packets::try_new(Group::new(1).unwrap(), &buf[..n]).unwrap() {
        let mut words = [0u32; 2];
        p.to_words(&mut words).unwrap();
        if let Some(SysEx7Event::Complete(m)) = rx.push(&words) {
            done = Some(m.to_vec());
        }
    }
    let msg = done.unwrap();
    let (_, r) = deliver(&mut b, 1, &msg);
    assert_eq!(
        r.last().unwrap().0,
        1,
        "the 0x7F reply is on the Group of the inquiry"
    );

    let mut c = config(0x45);
    c.framing = Framing::SysEx;
    let mut s: Dev = CiDevice::new(c, &mut rng(&[0x0888_8888])).unwrap();
    let n = s.discovery(0, 0, &mut buf).unwrap();
    assert_eq!((buf[0], buf[n - 1]), (0xF0, 0xF7));
    let (_, r) = deliver(&mut b, 0, &buf[..n]);
    assert_eq!(sub(&r[0].1), 0x71);
}

#[test]
fn config_rules_and_muid_generation() {
    // REQ-CI-LEN-001 to -003, REQ-CI-MUID-004 to -006, REQ-CI-MSG-011.
    let mut c = config(0x41);
    c.max_sysex_size = 256;
    assert_eq!(
        Dev::new(c, &mut rng(&[1])).err(),
        Some(DeviceError::InvalidConfig)
    );
    c.categories = Categories::default();
    assert!(Dev::new(c, &mut rng(&[1])).is_ok());
    c.max_sysex_size = 127;
    assert_eq!(
        Dev::new(c, &mut rng(&[1])).err(),
        Some(DeviceError::InvalidConfig)
    );
    // Reserved values from the random source are skipped.
    let d = Dev::new(
        config(0x41),
        &mut rng(&[0x0FFF_FFFF, 0x0FFF_FF00, 0x0000_0042]),
    )
    .unwrap();
    assert_eq!(d.muid().value(), 0x42);
    let e = Dev::new(config(0x41), &mut rng(&[0x0000_0043])).unwrap();
    assert_ne!(d.muid(), e.muid());
}

#[test]
fn appendix_f_minimum() {
    // Appendix F; REQ-CI-MIN-001, REQ-CI-DISC-010, REQ-CI-GEN-002, REQ-CI-MUID-001, -002, -010.
    let (mut a, mut b) = discovered();
    let mut buf = [0u8; 512];
    // Both roles in one Device: B inquires A, A inquires B.
    let n = b.endpoint_info_inquiry(a.muid(), &mut buf).unwrap();
    let (_, r) = deliver(&mut a, 0, &buf[..n]);
    assert_eq!(sub(&r[0].1), 0x73);
    let n = a.endpoint_info_inquiry(b.muid(), &mut buf).unwrap();
    let (_, r) = deliver(&mut b, 0, &buf[..n]);
    assert_eq!(sub(&r[0].1), 0x73);
    // The Broadcast MUID only in Discovery, Invalidate MUID and Profile reports.
    assert_ne!(header(&r[0].1).destination, Muid::BROADCAST);
    let k = Nak {
        original_sub_id2: 0x34,
        status: NakStatus::Busy(5),
        details: Details::Reserved,
        text: &[],
    };
    let n = a
        .nak(b.muid(), DeviceId::FunctionBlock, &k, "busy", &mut buf)
        .unwrap();
    let (ev, _) = deliver(&mut b, 0, &buf[..n]);
    assert!(matches!(ev, Some(Event::Nak { nak, .. }) if nak.status == NakStatus::Busy(5)));
}

#[test]
fn invalidate_from_the_colliding_device() {
    // 5.9.1 Option B: the other Device sends Invalidate MUID with the duplicated MUID as
    // Source and Target; this Device takes a new MUID (REQ-CI-MUID-014, REQ-CI-INV-005).
    let mut a = dev(0x41, &[0x0111_1111]);
    let m = a.muid();
    let mut buf = [0u8; 64];
    let n = zmidi_ci::InvalidateMuid { target: m }
        .write(m, Framing::Bare, &mut buf)
        .unwrap();
    let (ev, r) = deliver(&mut a, 0, &buf[..n]);
    assert!(matches!(ev, Some(Event::MuidChanged { old, new }) if old == m && new != m));
    assert!(r.is_empty());
    // The own echo has the old MUID as Source: it changes nothing.
    let before = a.muid();
    let n = a.invalidate(&mut rng(&[0x0999_9999]), &mut buf).unwrap();
    let after = a.muid();
    assert_ne!(after, before);
    let (ev, r) = deliver(&mut a, 0, &buf[..n]);
    assert_eq!(ev, Some(Event::PeerInvalidated { muid: before }));
    assert!(r.is_empty());
    assert_eq!(a.muid(), after);
}

#[test]
fn two_connections_are_not_a_duplicate() {
    // REQ-CI-DISC-015, REQ-CI-MUID-015, DEC-011 D-5: one Discovery for each MIDI Out
    // connection; one reply on each is not a duplicate, and each echo is an echo.
    let mut a = dev(0x41, &[0x0111_1111]);
    let mut b = dev(0x42, &[0x0222_2222]);
    let mut buf = [0u8; 512];
    let n0 = a.discovery(0, 0, &mut buf).unwrap();
    let d0 = buf[..n0].to_vec();
    let n1 = a.discovery(1, 10, &mut buf).unwrap();
    let d1 = buf[..n1].to_vec();
    for echo in [&d0, &d1] {
        let (ev, r) = deliver(&mut a, 0, echo);
        assert!(ev.is_none() && r.is_empty(), "echo");
    }
    let r0 = deliver(&mut b, 0, &d0).1[0].1.clone();
    let r1 = deliver(&mut b, 0, &d1).1[0].1.clone();
    assert!(matches!(
        deliver(&mut a, 0, &r0).0,
        Some(Event::Discovered { .. })
    ));
    assert!(matches!(
        deliver(&mut a, 0, &r1).0,
        Some(Event::Discovered { .. })
    ));
    // The same reply again is a duplicate.
    assert!(matches!(
        deliver(&mut a, 0, &r1).0,
        Some(Event::PeerInvalidated { .. })
    ));
}

#[cfg(feature = "property-exchange")]
#[test]
fn reserved_device_id_and_other_destinations() {
    // DEC-011 D-10, REQ-CI-NAK-006, REQ-CI-ADDR-003, REQ-CI-INV-004.
    let (mut a, mut b) = discovered();
    let mut buf = [0u8; 64];
    let n = a.pe_capabilities_inquiry(b.muid(), &mut buf).unwrap();
    let mut m = buf[..n].to_vec();
    m[1] = 0x20;
    let (ev, r) = deliver(&mut b, 0, &m);
    assert!(matches!(ev, Some(Event::Rejected { .. })));
    let k = nak_of(&r[0].1);
    assert_eq!((k.status, k.original_sub_id2), (NakStatus::NotInUse, 0x30));
    // To another MUID: nothing, not even an event.
    m[9..13].copy_from_slice(&Muid::new(0x0333_3333).unwrap().to_bytes());
    let (ev, r) = deliver(&mut b, 0, &m);
    assert!(ev.is_none() && r.is_empty());
    // PE Capabilities accept Device ID 0x0F (8.2: all Property Exchange messages).
    let mut m = buf[..n].to_vec();
    m[1] = 0x0F;
    let (_, r) = deliver(&mut b, 0, &m);
    assert_eq!(sub(&r[0].1), 0x31);
    // Invalidate MUID never gets a NAK.
    let n = zmidi_ci::InvalidateMuid {
        target: Muid::new(7).unwrap(),
    }
    .write(a.muid(), Framing::Bare, &mut buf)
    .unwrap();
    let mut m = buf[..n].to_vec();
    m[1] = 0x7E;
    assert!(deliver(&mut b, 0, &m).1.is_empty());
    m[1] = 0x7F;
    m[4] = 0x22;
    assert!(deliver(&mut b, 0, &m).1.is_empty());
    assert!(deliver(&mut b, 0, &m[..14]).1.is_empty());
}

#[cfg(all(feature = "property-exchange", feature = "profiles"))]
#[test]
fn new_muid_needs_new_inquiries() {
    // REQ-CI-PEF-031, REQ-CI-GEN-003 after a MUID change (REQ-CI-INV-007 on the peer side).
    let (mut a, mut b) = discovered();
    let mut buf = [0u8; 512];
    let n = a.pe_capabilities_inquiry(b.muid(), &mut buf).unwrap();
    let r = deliver(&mut b, 0, &buf[..n]).1[0].1.clone();
    let _ = deliver(&mut a, 0, &r);
    a.profile_inquiry(b.muid(), DeviceId::FunctionBlock, &mut buf)
        .unwrap();
    assert!(a
        .pe_request(b.muid(), PeKind::GetInquiry, b"{}", Some(0), None)
        .is_ok());
    a.invalidate(&mut rng(&[0x0aaa_aaaa]), &mut buf).unwrap();
    assert_eq!(
        a.pe_request(b.muid(), PeKind::GetInquiry, b"{}", Some(0), None)
            .err(),
        Some(DeviceError::NotInquired)
    );
    assert_eq!(
        a.set_profile_on(b.muid(), DeviceId::Channel(0), gm(), 1, None, &mut buf),
        Err(DeviceError::NotInquired)
    );
}

#[test]
fn ack_needs_version_2() {
    // REQ-CI-MSG-010: MIDI-CI ACK is a version 2 message.
    let mut b = dev(0x42, &[0x0222_2222]);
    let mut v1 = dev(0x44, &[0x0555_5555]);
    let mut buf = [0u8; 512];
    let n = v1.discovery(0, 0, &mut buf).unwrap();
    buf[4] = 1;
    let _ = deliver(&mut b, 0, &buf[..n - 1]);
    let ack = zmidi_ci::Ack {
        original_sub_id2: 0x34,
        status: AckStatus::Ack,
        details: Details::Reserved,
        text: &[],
    };
    assert_eq!(
        b.ack(v1.muid(), DeviceId::FunctionBlock, &ack, "", &mut buf),
        Err(DeviceError::PeerVersion)
    );
}

#[cfg(feature = "profiles")]
#[test]
fn two_open_details_inquiries() {
    // REQ-CI-PRFR-016: a requested reply is not a Notification, also with two open inquiries.
    let (mut a, mut b) = discovered();
    let mut buf = [0u8; 512];
    let _ = b.add_profile(entry(true)).unwrap().count();
    let n = a
        .profile_details_inquiry(b.muid(), DeviceId::Channel(0), gm(), 0, &mut buf)
        .unwrap();
    let first = deliver(&mut b, 0, &buf[..n]).1[0].1.clone();
    let n = a
        .profile_details_inquiry(b.muid(), DeviceId::Channel(0), gm(), 0x41, &mut buf)
        .unwrap();
    let _ = deliver(&mut b, 0, &buf[..n]);
    assert!(matches!(
        deliver(&mut a, 0, &first).0,
        Some(Event::ProfileDetailsReply {
            notification: false,
            ..
        })
    ));
}

#[test]
fn discovery_counts_as_use_for_option_b() {
    // DEC-011 D-6 and 4.1: after a Discovery Transaction with a peer, a collision uses Option B.
    let (mut a, _b) = discovered();
    let mut buf = [0u8; 512];
    let n = a.discovery(0, 0, &mut buf).unwrap();
    let mut other = buf[..n].to_vec();
    other[13] = 0x55;
    let (_, r) = deliver(&mut a, 0, &other);
    assert_eq!(sub(&r[0].1), 0x7E, "Option B: Invalidate MUID");
}

#[cfg(feature = "process-inquiry")]
#[test]
fn group_report_has_one_reply_and_end() {
    // REQ-CI-MMR-016: Device ID 0x7E gives one Reply and one End, not one pair per Channel.
    let (mut a, mut b) = discovered();
    let mut buf = [0u8; 512];
    let q = MessageReportInquiry {
        data_control: DataControl::Full,
        system: Default::default(),
        channel: ChannelBits {
            pitch_bend: true,
            ..Default::default()
        },
        note: Default::default(),
    };
    let n = a
        .message_report_inquiry(b.muid(), DeviceId::Group, &q, &mut buf)
        .unwrap();
    let (ev, _) = deliver(&mut b, 0, &buf[..n]);
    let Some(Event::MessageReport {
        muid,
        device_id,
        group,
        inquiry,
    }) = ev
    else {
        panic!("{ev:?}")
    };
    let supported = MessageReportReply {
        channel: ChannelBits::from_byte(0x3F),
        ..Default::default()
    };
    let (n, mut w) = b
        .message_report(
            muid,
            device_id,
            group,
            &inquiry,
            &supported,
            Protocol::Midi1,
            &Values,
            &mut buf,
        )
        .unwrap();
    assert_eq!(header(&buf[..n]).device_id, DeviceId::Group);
    let mut packets = 0;
    while w.next_packet().is_some() {
        packets += 1;
    }
    assert_eq!(packets, 2, "Channels 1 and 2 in use, in one report");
    let n = b.message_report_end(muid, device_id, &mut buf).unwrap();
    assert_eq!(
        (sub(&buf[..n]), header(&buf[..n]).device_id),
        (0x44, DeviceId::Group)
    );
}

#[cfg(feature = "profiles")]
#[test]
fn disabled_report_answers_set_profile_on_for_another_profile() {
    // REQ-CI-PRFR-028, -029: a Disabled Report for A, then an Enabled Report for B.
    let (mut a, mut b) = discovered();
    let mut buf = [0u8; 512];
    let other = ProfileId::Standard {
        bank: 0x22,
        number: 1,
        version: 1,
        level: ProfileLevel::Minimum,
    };
    let _ = b.add_profile(entry(true)).unwrap().count();
    let _ = b
        .add_profile(ProfileEntry {
            profile: other,
            ..entry(false)
        })
        .unwrap()
        .count();
    a.profile_inquiry(b.muid(), DeviceId::Channel(0), &mut buf)
        .unwrap();
    let n = a
        .set_profile_on(b.muid(), DeviceId::Channel(0), other, 1, None, &mut buf)
        .unwrap();
    let Some(Event::SetProfile { address, .. }) = deliver(&mut b, 0, &buf[..n]).0 else {
        panic!()
    };
    for (p, on) in [(gm(), false), (other, true)] {
        let r = b.set_profile_state(address, p, on, 1).unwrap();
        let (_, n) = b.profile_report(&r, &mut buf).unwrap();
        let kind = if on {
            ReportKind::Enabled
        } else {
            ReportKind::Disabled
        };
        assert!(
            matches!(deliver(&mut a, 0, &buf[..n]).0, Some(Event::ProfileReport { kind: k, .. }) if k == kind)
        );
    }
}

#[test]
fn second_discovery_round_is_not_a_duplicate() {
    // Code review finding 1; REQ-CI-MUID-015, REQ-CI-DISC-011.
    let mut a = dev(0x41, &[0x0111_1111]);
    let mut b = dev(0x42, &[0x0222_2222]);
    let mut buf = [0u8; 512];
    for (t, expired) in [(0u64, false), (1_000_000, false), (20_000_000, true)] {
        let n = a.discovery(0, t, &mut buf).unwrap();
        let r = deliver(&mut b, 0, &buf[..n]).1[0].1.clone();
        let _ = expired;
        let (ev, inv) = deliver(&mut a, 0, &r);
        assert!(matches!(ev, Some(Event::Discovered { .. })), "round at {t}");
        assert!(inv.is_empty());
    }
}

#[cfg(feature = "profiles")]
#[test]
fn echo_after_profile_change_is_still_an_echo() {
    // Code review finding 3; DEC-011 D-5.
    let mut a = dev(0x41, &[0x0111_1111]);
    let mut buf = [0u8; 512];
    let n = a.discovery(0, 0, &mut buf).unwrap();
    let echo = buf[..n].to_vec();
    let _ = a.add_profile(entry(false)).unwrap().count();
    let old = a.muid();
    let (ev, r) = deliver(&mut a, 0, &echo);
    assert!(ev.is_none() && r.is_empty());
    assert_eq!(a.muid(), old);
}

#[test]
fn categories_that_are_off_give_nak_and_no_discovery_bit() {
    // DEC-017, M2-101 5.11, 7.13, AMB-M2-101-26: in every feature set, a category whose Cargo
    // feature is off is not in the Reply to Discovery, and its inquiries get NAK Not Supported,
    // although DeviceConfig turns all categories on.
    let mut a = dev(0x41, &[0x0111_1111]);
    let mut b = dev(0x42, &[0x0222_2222]);
    let mut buf = [0u8; 512];
    let n = a.discovery(0, 0, &mut buf).unwrap();
    let (_, replies) = deliver(&mut b, 0, &buf[..n]);
    let (h, body) = Header::parse(&replies[0].1).unwrap();
    let reply = zmidi_ci::management::DiscoveryReply::read(&h, body).unwrap();
    assert_eq!(
        reply.categories.property_exchange,
        cfg!(feature = "property-exchange")
    );
    assert_eq!(
        reply.categories.process_inquiry,
        cfg!(feature = "process-inquiry")
    );
    // No Profile bit without Profiles in the table (REQ-CI-PROF-018), with the feature or not.
    assert!(!reply.categories.profile_configuration);
    deliver(&mut a, 0, &replies[0].1);

    // The inquiries come from the message writers: the Initiator calls of a category that is off
    // give NotSupported in this build (DEC-017 addendum of 2026-10-22).
    let (src, dst, f) = (a.muid(), b.muid(), Framing::Bare);
    let on = [
        cfg!(feature = "profiles"),
        cfg!(feature = "property-exchange"),
        cfg!(feature = "process-inquiry"),
    ];
    for (i, on) in on.into_iter().enumerate() {
        let n = match i {
            0 => zmidi_ci::ProfileInquiry::write(DeviceId::FunctionBlock, src, dst, f, &mut buf),
            1 => zmidi_ci::PeCapabilities {
                simultaneous: 1,
                major: None,
                minor: None,
            }
            .write_inquiry(src, dst, f, &mut buf),
            _ => zmidi_ci::PiCapabilitiesInquiry::write(src, dst, f, &mut buf),
        }
        .unwrap();
        let msg = buf[..n].to_vec();
        let (_, r) = deliver(&mut b, 0, &msg);
        let answer = header(&r[0].1).sub_id2;
        if on {
            assert_ne!(answer, 0x7F, "inquiry {:#04x} got a NAK", msg[3]);
        } else {
            let nak = nak_of(&r[0].1);
            assert_eq!(
                (nak.status, nak.original_sub_id2),
                (NakStatus::NotSupported, msg[3])
            );
        }
    }
}

#[test]
#[cfg(feature = "profiles")]
fn local_profile_calls_need_the_profile_category_of_the_configuration() {
    // REQ-CI-TIER-004: a Device whose configuration does not declare Profile Configuration keeps
    // no local Profiles and writes no Profile report or reply.
    let mut c = config(1);
    c.categories.profile_configuration = false;
    let mut d: Dev = CiDevice::new(c, &mut rng(&[0x0123_4567])).unwrap();
    let mut buf = [0u8; 512];
    assert_eq!(
        d.add_profile(entry(true)).err(),
        Some(DeviceError::NotSupported)
    );
    assert!(d
        .set_profile_state(entry(true).address, gm(), true, 1)
        .is_none());
    assert!(d.remove_profile(entry(true).address, gm()).is_none());
    let report = ProfileReport::Added(entry(false));
    assert_eq!(
        d.profile_report(&report, &mut buf),
        Err(DeviceError::NotSupported)
    );
}

#[test]
fn calls_of_categories_that_are_off_give_not_supported() {
    // DEC-017 addendum of 2026-10-22: a call of a category that this build does not have gives
    // DeviceError::NotSupported (or None), never a message.
    let (mut a, b) = discovered();
    let mut buf = [0u8; 512];
    let p = b.muid();
    let off = |on: bool, r: Result<usize, DeviceError>| {
        if !on {
            assert_eq!(r, Err(DeviceError::NotSupported));
        }
    };
    let profiles = cfg!(feature = "profiles");
    off(
        profiles,
        a.profile_inquiry(p, DeviceId::FunctionBlock, &mut buf),
    );
    off(
        profiles,
        a.profile_details_inquiry(p, DeviceId::Channel(0), gm(), 0, &mut buf),
    );
    if !profiles {
        assert!(a.add_profile(entry(true)).is_err());
        assert!(a
            .set_profile_state(entry(true).address, gm(), true, 1)
            .is_none());
        assert!(a.remove_profile(entry(true).address, gm()).is_none());
    }
    let pe = cfg!(feature = "property-exchange");
    off(pe, a.pe_capabilities_inquiry(p, &mut buf));
    if !pe {
        assert_eq!(
            a.pe_request(p, PeKind::GetInquiry, b"{}", Some(0), None)
                .err(),
            Some(DeviceError::NotSupported)
        );
    }
    let pi = cfg!(feature = "process-inquiry");
    off(pi, a.pi_capabilities_inquiry(p, &mut buf));
    if !pi {
        assert!(!a.report_open(p));
    }
}
