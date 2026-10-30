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
    deliver_at(d, group, msg, 0)
}

/// [`deliver`] at the time `now_us`.
fn deliver_at<'m>(
    d: &mut Dev,
    group: u8,
    msg: &'m [u8],
    now_us: u64,
) -> (Option<Event<'m>>, Vec<(u8, Vec<u8>)>) {
    let mut r = d.receive(group, msg, now_us, &mut rng(&[0x0123_4567, 0x0234_5678]));
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
    // REQ-CI-DISC-003: an Inquiry to the discovered Responder has its MUID as Destination.
    let n = a.endpoint_info_inquiry(b.muid(), &mut buf).unwrap();
    let h = header(&buf[..n]);
    assert_eq!(
        (h.sub_id2, h.source, h.destination),
        (0x72, a.muid(), b.muid())
    );
    // REQ-CI-DISC-017: one connection: Output Path ID 0, the last byte of Discovery.
    let n = a.discovery(0, 0, &mut buf).unwrap();
    assert_eq!((n, buf[n - 1]), (30, 0));
    // REQ-CI-DISC-014: no Function Block: 0x7F in the Reply to Discovery.
    let mut c = config(0x43);
    c.function_block = None;
    let mut d: Dev = CiDevice::new(c, &mut rng(&[0x0444_4444])).unwrap();
    let (_, r) = deliver(&mut d, 0, &buf[..n]);
    let r = &r[0].1;
    assert_eq!((sub(r), &r[r.len() - 2..]), (0x71, &[0u8, 0x7F][..]));
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
    // A Reply at t + 2.999999 s is in the window of its Discovery: it is accepted, and the same
    // reply again in that window is a duplicate (REQ-CI-MUID-015). At t + 3 s the window is
    // closed: the same reply is a new Discovered event.
    let mut b = dev(0x42, &[0x0222_2222]);
    for (late, dup) in [(2_999_999u64, true), (3_000_000, false)] {
        let mut a = dev(0x41, &[0x0111_1111]);
        let n = a.discovery(0, 1_000_000, &mut buf).unwrap();
        let r = deliver(&mut b, 0, &buf[..n]).1[0].1.clone();
        let (ev, _) = deliver_at(&mut a, 0, &r, 1_000_000 + late);
        assert!(
            matches!(ev, Some(Event::Discovered { muid, .. }) if muid == b.muid()),
            "{late}"
        );
        let (ev, inv) = deliver_at(&mut a, 0, &r, 1_000_000 + late);
        if dup {
            assert_eq!(ev, Some(Event::PeerInvalidated { muid: b.muid() }));
            assert_eq!(sub(&inv[0].1), 0x7E);
        } else {
            assert!(matches!(ev, Some(Event::Discovered { .. })) && inv.is_empty());
        }
    }
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
    // The new MUID is the random number that the caller gives (deliver: 0x0123_4567).
    assert_eq!(a.muid(), Muid::new(0x0123_4567).unwrap());
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
    assert_eq!(a.muid(), Muid::new(0x0123_4567).unwrap());
    let r = &replies[0].1;
    assert_eq!(sub(r), 0x7E);
    assert_eq!(&r[13..17], &old.to_bytes());
}

#[test]
fn discovery_after_a_collision_that_keeps_this_muid() {
    // REQ-CI-DISC-006: B resolves a collision with another Device and changes its MUID; the MUID
    // of the initiator A does not change, and A starts a new Discovery that finds the new MUID.
    let (mut a, mut b) = discovered();
    let mut buf = [0u8; 512];
    let n = b.discovery(0, 0, &mut buf).unwrap();
    let mut other = buf[..n].to_vec();
    other[13] = 0x55;
    let (old_a, old_b) = (a.muid(), b.muid());
    let (ev, _) = deliver(&mut b, 0, &other);
    assert!(matches!(ev, Some(Event::MuidChanged { old, .. }) if old == old_b));
    assert_eq!(a.muid(), old_a);
    let n = a.discovery(0, 10_000_000, &mut buf).unwrap();
    assert_eq!(header(&buf[..n]).source, old_a);
    let r = deliver(&mut b, 0, &buf[..n]).1[0].1.clone();
    let (ev, _) = deliver_at(&mut a, 0, &r, 10_000_000);
    assert!(
        matches!(ev, Some(Event::Discovered { muid, .. }) if muid == b.muid() && muid != old_b)
    );
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
    // REQ-CI-GEN-004: the NAK only; no Reply, and the capabilities are not kept.
    assert_eq!(r.len(), 1);
    assert!(b.peer(a.muid()).unwrap().pe_capabilities.is_none());

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
    let (a0, b0) = (a.muid(), b.muid());
    // A peer MUID: B forgets A and ends its Requests.
    let n = b.pe_capabilities_inquiry(a.muid(), &mut buf).unwrap();
    let (_, r) = deliver(&mut a, 0, &buf[..n]);
    let _ = deliver(&mut b, 0, &r[0].1);
    let (id, _) = b
        .pe_request(a.muid(), PeKind::GetInquiry, b"{}", Some(0), None)
        .unwrap();
    // REQ-CI-MUID-008: the MUIDs stay the same over the Transactions.
    assert_eq!((a.muid(), b.muid()), (a0, b0));
    let old_a = a.muid();
    let n = a.invalidate(&mut rng(&[0x0666_6666]), &mut buf).unwrap();
    assert_eq!(a.muid(), Muid::new(0x0666_6666).unwrap());
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
    assert_eq!(b.muid(), Muid::new(0x0123_4567).unwrap());
    assert!(r.is_empty());
    // The Function Block moved: Invalidate MUID, new MUID, then a new Discovery.
    let before = a.muid();
    let n = a
        .function_block_changed(4, 1, &mut rng(&[0x0777_7777]), &mut buf)
        .unwrap();
    assert_eq!(&buf[13..17], &before.to_bytes());
    assert_eq!(header(&buf[..n]).destination, Muid::BROADCAST);
    assert_eq!(a.muid(), Muid::new(0x0777_7777).unwrap());
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

/// The fields of a Profile report event: MUID, Device ID, Group, kind, Profile ID, count.
#[cfg(feature = "profiles")]
fn report_of(ev: Option<Event<'_>>) -> (Muid, DeviceId, u8, ReportKind, ProfileId, u16) {
    match ev {
        Some(Event::ProfileReport {
            muid,
            device_id,
            group,
            kind,
            profile,
            channels,
        }) => (muid, device_id, group, kind, profile, channels),
        e => panic!("{e:?}"),
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
    // REQ-CI-PRFR-010: the Responder MUID, the Device ID address and the Profile ID.
    let ch0 = DeviceId::Channel(0);
    assert_eq!(
        report_of(ev),
        (b.muid(), ch0, 0, ReportKind::Added, gm(), 0)
    );

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
    // REQ-CI-PRFR-012.
    assert_eq!(
        report_of(ev),
        (b.muid(), ch0, 0, ReportKind::Enabled, gm(), 1)
    );
    let n = a
        .set_profile_off(b.muid(), DeviceId::Channel(0), gm(), &mut buf)
        .unwrap();
    let (ev, _) = deliver(&mut b, 0, &buf[..n]);
    assert_eq!(
        ev,
        Some(Event::SetProfile {
            muid: a.muid(),
            address,
            profile: gm(),
            on: false,
            channels: 0
        })
    );
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
        (k.status, k.original_sub_id2, k.details),
        (
            NakStatus::ProfileNotSupported,
            0x22,
            Details::Profile(other.to_bytes())
        )
    );
    assert_eq!(r.len(), 1);
    // REQ-CI-PRFR-027: Set Profile Off for an unknown Profile: NAK 0x04, and no event.
    let n = a
        .set_profile_off(b.muid(), DeviceId::Channel(0), other, &mut buf)
        .unwrap();
    let (ev, r) = deliver(&mut b, 0, &buf[..n]);
    assert!(ev.is_none() && r.len() == 1);
    let k = nak_of(&r[0].1);
    assert_eq!(
        (k.status, k.original_sub_id2, k.details),
        (
            NakStatus::ProfileNotSupported,
            0x23,
            Details::Profile(other.to_bytes())
        )
    );
    assert_eq!(header(&r[0].1).destination, a.muid());
    // A known Profile on another Channel is not supported there either.
    let n = a
        .set_profile_off(b.muid(), DeviceId::Channel(1), gm(), &mut buf)
        .unwrap();
    let (_, r) = deliver(&mut b, 0, &buf[..n]);
    assert_eq!(nak_of(&r[0].1).status, NakStatus::ProfileNotSupported);
    // A local action disables it: Disabled Report (REQ-CI-PROF-028, REQ-CI-PRFR-013).
    let report = b.set_profile_state(address, gm(), false, 1).unwrap();
    let (_, n) = b.profile_report(&report, &mut buf).unwrap();
    let (ev, _) = deliver(&mut a, 0, &buf[..n]);
    assert_eq!(
        report_of(ev),
        (b.muid(), ch0, 0, ReportKind::Disabled, gm(), 1)
    );
    let reports: Vec<_> = b.remove_profile(address, gm()).unwrap().collect();
    assert_eq!(reports.len(), 1);
    let (_, n) = b.profile_report(&reports[0], &mut buf).unwrap();
    let (ev, _) = deliver(&mut a, 0, &buf[..n]);
    // REQ-CI-PRFR-011.
    assert_eq!(
        report_of(ev),
        (b.muid(), ch0, 0, ReportKind::Removed, gm(), 0)
    );
    // A report on Group 1 to a Group Profile: the Group and the Device ID of the event.
    let group_entry = ProfileEntry {
        address: ProfileAddress {
            device_id: DeviceId::Group,
            group: 1,
        },
        ..entry(true)
    };
    let reports: Vec<_> = b.add_profile(group_entry).unwrap().collect();
    let (g, n) = b.profile_report(&reports[1], &mut buf).unwrap();
    assert_eq!(g, 1);
    let (ev, _) = deliver(&mut a, g, &buf[..n]);
    assert_eq!(
        report_of(ev),
        (b.muid(), DeviceId::Group, 1, ReportKind::Enabled, gm(), 0)
    );
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

#[cfg(feature = "profiles")]
#[test]
fn profile_details_targets() {
    // 7.6, M2-102 2.5; REQ-CI-PRFR-014, REQ-CI-PRFR-020, REQ-CI-PROF-023, REQ-CI-PROF-024.
    let (mut a, mut b) = discovered();
    let mut buf = [0u8; 512];
    let _ = b.add_profile(entry(false)).unwrap().count();
    let ch0 = DeviceId::Channel(0);
    // No Profile Details Inquiry: no Reply to Profile Details Inquiry (REQ-CI-PRFR-020).
    let n = a
        .profile_inquiry(b.muid(), DeviceId::FunctionBlock, &mut buf)
        .unwrap();
    let (_, r) = deliver(&mut b, 0, &buf[..n]);
    assert!(
        r.iter().all(|(_, m)| sub(m) == 0x21),
        "only Replies to Profile Inquiry"
    );
    // A target other than 0x00: an event with all fields, and no automatic reply.
    let n = a
        .profile_details_inquiry(b.muid(), ch0, gm(), 0x40, &mut buf)
        .unwrap();
    let (ev, r) = deliver(&mut b, 0, &buf[..n]);
    assert!(r.is_empty(), "REQ-CI-PRFR-020: no reply with target 0x00");
    let address = ProfileAddress {
        device_id: ch0,
        group: 0,
    };
    assert_eq!(
        ev,
        Some(Event::ProfileDetailsInquiry {
            muid: a.muid(),
            address,
            profile: gm(),
            target: 0x40
        })
    );
    // The application gives the Inquiry Target Data of that target (REQ-CI-PRFR-014), with the
    // Inquiry Target of the inquiry (REQ-CI-PROF-024).
    let data = [0x01, 0x02, 0x7F];
    let reply = zmidi_ci::ProfileDetailsReply {
        profile: gm(),
        target: 0x40,
        data: &data,
    };
    let n = b
        .profile_details_reply(a.muid(), ch0, &reply, &mut buf)
        .unwrap();
    let h = header(&buf[..n]);
    assert_eq!(
        (h.device_id, h.sub_id2, h.source, h.destination),
        (ch0, 0x29, b.muid(), a.muid())
    );
    assert_eq!(&buf[18..n], &[0x40, 0x03, 0x00, 0x01, 0x02, 0x7F]);
    match deliver(&mut a, 0, &buf[..n]).0 {
        Some(Event::ProfileDetailsReply {
            muid,
            device_id,
            reply,
            notification,
            ..
        }) => {
            assert_eq!((muid, device_id, notification), (b.muid(), ch0, false));
            assert_eq!(
                (reply.profile, reply.target, reply.data),
                (gm(), 0x40, &data[..])
            );
        }
        e => panic!("{e:?}"),
    }
    // A target that the application does not support: it sends NAK 0x04 with the Profile ID.
    let n = a
        .profile_details_inquiry(b.muid(), ch0, gm(), 0x41, &mut buf)
        .unwrap();
    let (ev, r) = deliver(&mut b, 0, &buf[..n]);
    assert!(matches!(ev, Some(Event::ProfileDetailsInquiry { target: 0x41, .. })) && r.is_empty());
    let k = Nak {
        original_sub_id2: 0x28,
        status: NakStatus::ProfileNotSupported,
        details: Details::Profile(gm().to_bytes()),
        text: &[],
    };
    let n = b.nak(a.muid(), ch0, &k, "", &mut buf).unwrap();
    let got = nak_of(&buf[..n]);
    assert_eq!(
        (got.status, got.original_sub_id2, got.details),
        (k.status, 0x28, k.details)
    );
    // Target 0x00, disabled and enabled (REQ-CI-PROF-023): the reply has target 0x00 and the
    // counts.
    for (on, in_use) in [(false, 0), (true, 2)] {
        if on {
            let _ = b.set_profile_state(address, gm(), true, 2).unwrap();
        }
        let n = a
            .profile_details_inquiry(b.muid(), ch0, gm(), 0x00, &mut buf)
            .unwrap();
        let (ev, r) = deliver(&mut b, 0, &buf[..n]);
        assert!(ev.is_none());
        assert_eq!(r.len(), 1);
        let m = &r[0].1;
        assert_eq!(
            (sub(m), header(m).device_id, header(m).destination),
            (0x29, ch0, a.muid())
        );
        // Profile ID (Level of the table entry), target 0x00, length 4, in use, maximum 4.
        assert_eq!(
            &m[13..],
            &[0x7E, 0x21, 0x00, 0x01, 0x01, 0x00, 0x04, 0x00, in_use, 0x00, 0x04, 0x00]
        );
    }
}

#[cfg(feature = "profiles")]
#[test]
fn multi_channel_profile_uses_the_manager_channel() {
    // REQ-CI-PRFR-022: a Multi-Channel Profile on Manager Channel 3 (of 4 Channels): every
    // message about it has Device ID 0x03.
    let (mut a, mut b) = discovered();
    let mut buf = [0u8; 512];
    let mpe = ProfileId::Standard {
        bank: 0x31,
        number: 0x01,
        version: 0x01,
        level: ProfileLevel::Minimum,
    };
    let manager = ProfileAddress {
        device_id: DeviceId::Channel(3),
        group: 0,
    };
    let _ = b
        .add_profile(ProfileEntry {
            address: manager,
            profile: mpe,
            enabled: false,
            channels: 0,
            max_channels: 16,
        })
        .unwrap()
        .count();
    let n = a
        .profile_inquiry(b.muid(), DeviceId::FunctionBlock, &mut buf)
        .unwrap();
    let (_, r) = deliver(&mut b, 0, &buf[..n]);
    assert_eq!(r.len(), 2);
    assert_eq!(header(&r[0].1).device_id, DeviceId::Channel(3));
    // The initiator to the desired Manager Channel, with the count.
    let n = a
        .set_profile_on(b.muid(), DeviceId::Channel(3), mpe, 4, Some(16), &mut buf)
        .unwrap();
    assert_eq!(
        (buf[1], buf[3], &buf[n - 2..n]),
        (0x03, 0x22, &[4u8, 0][..])
    );
    let (ev, r) = deliver(&mut b, 0, &buf[..n]);
    assert!(r.is_empty());
    assert_eq!(
        ev,
        Some(Event::SetProfile {
            muid: a.muid(),
            address: manager,
            // Set Profile On carries Level 0x7F (REQ-CI-PRFR-002).
            profile: ProfileId::Standard {
                bank: 0x31,
                number: 0x01,
                version: 0x01,
                level: ProfileLevel::Highest,
            },
            on: true,
            channels: 4
        })
    );
    // The responder to the requested Manager Channel.
    let report = b.set_profile_state(manager, mpe, true, 4).unwrap();
    let (_, n) = b.profile_report(&report, &mut buf).unwrap();
    assert_eq!(
        (buf[1], buf[3], &buf[n - 2..n]),
        (0x03, 0x24, &[4u8, 0][..])
    );
    let (ev, _) = deliver(&mut a, 0, &buf[..n]);
    assert_eq!(
        report_of(ev),
        (
            b.muid(),
            DeviceId::Channel(3),
            0,
            ReportKind::Enabled,
            mpe,
            4
        )
    );
    let n = a
        .profile_details_inquiry(b.muid(), DeviceId::Channel(3), mpe, 0, &mut buf)
        .unwrap();
    let (_, r) = deliver(&mut b, 0, &buf[..n]);
    let m = &r[0].1;
    assert_eq!((m[1], m[3]), (0x03, 0x29));
    assert_eq!(&m[m.len() - 4..], &[4, 0, 16, 0]);
    let n = a
        .set_profile_off(b.muid(), DeviceId::Channel(3), mpe, &mut buf)
        .unwrap();
    assert_eq!(buf[1], 0x03);
    let (ev, _) = deliver(&mut b, 0, &buf[..n]);
    assert!(matches!(ev, Some(Event::SetProfile { address, on: false, .. }) if address == manager));
}

#[cfg(feature = "profiles")]
#[test]
fn profile_inquiry_to_each_address() {
    // REQ-CI-PRFR-006: a Profile Inquiry to a Channel, to the Group or to the Function Block gets
    // a Reply to Profile Inquiry with that Device ID.
    let (mut a, mut b) = discovered();
    let mut buf = [0u8; 512];
    let group = ProfileEntry {
        address: ProfileAddress {
            device_id: DeviceId::Group,
            group: 0,
        },
        ..entry(true)
    };
    let _ = b.add_profile(group).unwrap().count();
    let _ = b.add_profile(entry(false)).unwrap().count();
    let gm_bytes = gm().to_bytes();
    for (d, want) in [
        (DeviceId::Channel(0), [0u8, 0, 1, 0]),
        (DeviceId::Group, [1, 0, 0, 0]),
        (DeviceId::Channel(7), [0, 0, 0, 0]),
    ] {
        let n = a.profile_inquiry(b.muid(), d, &mut buf).unwrap();
        let (ev, r) = deliver(&mut b, 0, &buf[..n]);
        assert!(ev.is_none());
        assert_eq!(r.len(), 1, "{d:?}");
        let m = &r[0].1;
        let h = header(m);
        assert_eq!((h.device_id, h.sub_id2, h.destination), (d, 0x21, a.muid()));
        // Counts and Profile IDs: enabled list, then disabled list.
        let mut body = vec![want[0], want[1]];
        if want[0] == 1 {
            body.extend(gm_bytes);
        }
        body.extend([want[2], want[3]]);
        if want[2] == 1 {
            body.extend(gm_bytes);
        }
        assert_eq!(&m[13..], &body[..], "{d:?}");
    }
    // The Function Block: the Channel reply, the Group reply, then the 0x7F reply.
    let n = a
        .profile_inquiry(b.muid(), DeviceId::FunctionBlock, &mut buf)
        .unwrap();
    let (_, r) = deliver(&mut b, 0, &buf[..n]);
    let ids: Vec<_> = r.iter().map(|(_, m)| header(m).device_id).collect();
    assert_eq!(
        ids,
        [
            DeviceId::Channel(0),
            DeviceId::Group,
            DeviceId::FunctionBlock
        ]
    );
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
    // REQ-CI-PEF-006: the Device writes PE version 0x00 0x00 in its Inquiry and its Reply.
    let mut want = head(0x7F, 0x30, a.muid(), b.muid());
    want.extend([2, 0, 0]);
    assert_eq!(&buf[..n], &want[..]);
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
    let inquiry = buf[..n].to_vec();
    let (_, r) = deliver(&mut b, 0, &inquiry);
    let mut want = head(0x7F, 0x41, b.muid(), a.muid());
    want.push(0x01);
    assert_eq!(r[0].1, want, "Table 41: D0 MIDI Message Report");
    let (ev, _) = deliver(&mut a, 0, &r[0].1);
    assert!(
        matches!(ev, Some(Event::PiCapabilities { muid, capabilities }) if muid == b.muid() && capabilities.midi_message_report)
    );
    // REQ-CI-PI-001: any other Device ID gets NAK 0x03, and no reply.
    for d in [0x7Eu8, 0x00, 0x0F] {
        let mut m = inquiry.clone();
        m[1] = d;
        let (ev, r) = deliver(&mut b, 0, &m);
        assert!(ev.is_none() && r.len() == 1);
        let k = nak_of(&r[0].1);
        assert_eq!(
            (k.status, k.original_sub_id2),
            (NakStatus::NotInUse, 0x40),
            "{d:#04x}"
        );
    }

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
    // The application cannot enable both: it disables A, then enables B; the Device gives the
    // two reports in that order and keeps the state.
    let mut seen = Vec::new();
    for (p, on) in [(gm(), false), (other, true)] {
        let r = b.set_profile_state(address, p, on, 1).unwrap();
        let (_, n) = b.profile_report(&r, &mut buf).unwrap();
        seen.push(report_of(deliver(&mut a, 0, &buf[..n]).0));
    }
    let ch0 = DeviceId::Channel(0);
    assert_eq!(
        seen,
        [
            (b.muid(), ch0, 0, ReportKind::Disabled, gm(), 1),
            (b.muid(), ch0, 0, ReportKind::Enabled, other, 1),
        ]
    );
    assert!(!b.profiles().find(address, gm()).unwrap().enabled);
    assert!(b.profiles().find(address, other).unwrap().enabled);
}

#[test]
fn second_discovery_round_is_not_a_duplicate() {
    // Code review finding 1; REQ-CI-MUID-015.
    let mut a = dev(0x41, &[0x0111_1111]);
    let mut b = dev(0x42, &[0x0222_2222]);
    let mut buf = [0u8; 512];
    for t in [0u64, 1_000_000, 20_000_000] {
        let n = a.discovery(0, t, &mut buf).unwrap();
        let r = deliver(&mut b, 0, &buf[..n]).1[0].1.clone();
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
    let reply = zmidi_ci::ProfileDetailsReply {
        profile: gm(),
        target: 0,
        data: &[0, 0, 1, 0],
    };
    assert_eq!(
        d.profile_details_reply(
            Muid::new(5).unwrap(),
            DeviceId::Channel(0),
            &reply,
            &mut buf
        ),
        Err(DeviceError::NotSupported)
    );
    assert!(d.profiles().entries().next().is_none());
    // Its Profile messages get NAK 0x01 and nothing else.
    let mut a = dev(0x41, &[0x0111_1111]);
    let n = a.discovery(0, 0, &mut buf).unwrap();
    let r = deliver(&mut d, 0, &buf[..n]).1[0].1.clone();
    assert_eq!(buf_categories(&r) & 0x04, 0, "no Profile bit");
    let _ = deliver(&mut a, 0, &r);
    let n = a
        .profile_inquiry(d.muid(), DeviceId::FunctionBlock, &mut buf)
        .unwrap();
    let msgs = [
        buf[..n].to_vec(),
        {
            let n = a
                .set_profile_on(d.muid(), DeviceId::Channel(0), gm(), 1, None, &mut buf)
                .unwrap();
            buf[..n].to_vec()
        },
        {
            let n = a
                .profile_details_inquiry(d.muid(), DeviceId::Channel(0), gm(), 0, &mut buf)
                .unwrap();
            buf[..n].to_vec()
        },
    ];
    for m in &msgs {
        let (ev, r) = deliver(&mut d, 0, m);
        assert!(ev.is_none());
        assert_eq!(r.len(), 1, "{:#04x}", m[3]);
        let k = nak_of(&r[0].1);
        assert_eq!(
            (k.status, k.original_sub_id2),
            (NakStatus::NotSupported, m[3])
        );
    }
}

/// Capability Inquiry Category Supported byte of a Reply to Discovery (Table 8).
#[cfg_attr(not(feature = "full"), allow(dead_code))]
fn buf_categories(reply: &[u8]) -> u8 {
    let (h, body) = Header::parse(reply).unwrap();
    zmidi_ci::management::DiscoveryReply::read(&h, body)
        .unwrap()
        .categories
        .to_byte()
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
    off(
        profiles,
        a.set_profile_on(p, DeviceId::Channel(0), gm(), 1, None, &mut buf),
    );
    off(
        profiles,
        a.set_profile_off(p, DeviceId::Channel(0), gm(), &mut buf),
    );
    let reply = zmidi_ci::ProfileDetailsReply {
        profile: gm(),
        target: 0,
        data: &[0, 0, 1, 0],
    };
    off(
        profiles,
        a.profile_details_reply(p, DeviceId::Channel(0), &reply, &mut buf),
    );
    if !profiles {
        assert_eq!(
            a.add_profile(entry(true)).err(),
            Some(DeviceError::NotSupported)
        );
        assert!(a
            .set_profile_state(entry(true).address, gm(), true, 1)
            .is_none());
        assert!(a.remove_profile(entry(true).address, gm()).is_none());
        let report = ProfileReport::Added(entry(false));
        assert_eq!(
            a.profile_report(&report, &mut buf),
            Err(DeviceError::NotSupported)
        );
    }
    let pe = cfg!(feature = "property-exchange");
    off(pe, a.pe_capabilities_inquiry(p, &mut buf));
    if !pe {
        assert_eq!(
            a.pe_request(p, PeKind::GetInquiry, b"{}", Some(0), None)
                .err(),
            Some(DeviceError::NotSupported)
        );
        assert_eq!(
            a.pe_reply(p, PeKind::GetReply, 0, b"{}", Some(0), None)
                .err(),
            Some(DeviceError::NotSupported)
        );
    }
    let pi = cfg!(feature = "process-inquiry");
    off(pi, a.pi_capabilities_inquiry(p, &mut buf));
    let q = MessageReportInquiry {
        data_control: DataControl::Full,
        system: Default::default(),
        channel: Default::default(),
        note: Default::default(),
    };
    off(
        pi,
        a.message_report_inquiry(p, DeviceId::FunctionBlock, &q, &mut buf),
    );
    off(
        pi,
        a.message_report_end(p, DeviceId::FunctionBlock, &mut buf),
    );
    off(
        pi,
        a.abort_report(p, DeviceId::FunctionBlock, false, &mut buf),
    );
    if !pi {
        assert!(!a.report_open(p));
        struct NoValues;
        impl ReportSource for NoValues {}
        assert_eq!(
            a.message_report(
                p,
                DeviceId::FunctionBlock,
                0,
                &q,
                &MessageReportReply::default(),
                Protocol::Midi2,
                &NoValues,
                &mut buf,
            )
            .err(),
            Some(DeviceError::NotSupported)
        );
    }
}

/// `a` sends Inquiry: PE Capabilities to `b` and reads the reply.
#[cfg(feature = "property-exchange")]
fn pe_inquire(a: &mut Dev, b: &mut Dev) {
    let mut buf = [0u8; 64];
    let n = a.pe_capabilities_inquiry(b.muid(), &mut buf).unwrap();
    let r = deliver(b, 0, &buf[..n]).1[0].1.clone();
    let (ev, _) = deliver(a, 0, &r);
    assert!(matches!(ev, Some(Event::PeCapabilities { .. })));
}

/// The 13 header bytes of a message from `src` to `dst` (Table 5).
#[cfg_attr(not(feature = "full"), allow(dead_code))]
fn head(device_id: u8, sub_id2: u8, src: Muid, dst: Muid) -> Vec<u8> {
    let mut v = vec![0x7E, device_id, 0x0D, sub_id2, 0x02];
    v.extend(src.to_bytes());
    v.extend(dst.to_bytes());
    v
}

#[cfg(feature = "property-exchange")]
#[test]
fn set_and_subscription_replies() {
    // 8.9 to 8.12, Tables 35 to 38; REQ-CI-PEF-037, REQ-CI-PEF-041: the crate frames the reply,
    // the application gives the Header Data and Property Data.
    let (mut a, mut b) = discovered();
    let mut buf = [0u8; 512];
    pe_inquire(&mut a, &mut b);
    let status = br#"{"status":200}"#;
    for (inquiry, reply, sub) in [
        (PeKind::SetInquiry, PeKind::SetReply, 0x37u8),
        (PeKind::Subscription, PeKind::SubscriptionReply, 0x39),
    ] {
        let header_json = br#"{"resource":"X-Mode"}"#;
        let (id, mut w) = a
            .pe_request(b.muid(), inquiry, header_json, Some(3), None)
            .unwrap();
        let n = w.write_next(b"abc", &mut buf).unwrap();
        let (ev, r) = deliver(&mut b, 0, &buf[..n]);
        assert!(r.is_empty(), "the reply is the application's");
        let chunk = zmidi_ci::PeChunk {
            request_id: id,
            header: header_json,
            total: 1,
            number: 1,
            data: b"abc",
        };
        assert_eq!(
            ev,
            Some(Event::PeChunk {
                muid: a.muid(),
                kind: inquiry,
                chunk
            })
        );
        let mut w = b
            .pe_reply(a.muid(), reply, id, status, Some(0), None)
            .unwrap();
        let n = w.write_next(&[], &mut buf).unwrap();
        let mut want = head(0x7F, sub, b.muid(), a.muid());
        want.extend([id, 14, 0]);
        want.extend_from_slice(status);
        want.extend([1, 0, 1, 0, 0, 0]);
        assert_eq!(&buf[..n], &want[..]);
        let (ev, r) = deliver(&mut a, 0, &buf[..n]);
        assert!(r.is_empty());
        assert_eq!(
            ev,
            Some(Event::PeChunk {
                muid: b.muid(),
                kind: reply,
                chunk: zmidi_ci::PeChunk {
                    request_id: id,
                    header: status,
                    total: 1,
                    number: 1,
                    data: &[],
                }
            })
        );
        b.pe_end(a.muid(), id, Direction::Responder);
        a.pe_end(b.muid(), id, Direction::Initiator);
    }
    // A Reply to Set Property Data has no Property Data (Table 36).
    let mut w = b
        .pe_reply(a.muid(), PeKind::SetReply, 1, status, Some(1), None)
        .unwrap();
    assert_eq!(
        w.write_next(b"x", &mut buf).err(),
        Some(zmidi_ci::WriteError::InvalidValue)
    );
}

#[cfg(feature = "property-exchange")]
#[test]
fn interleaved_chunks_of_two_peers() {
    // 8.4; REQ-CI-PEF-028: two Set inquiries with the same Request ID from two peers, their
    // Chunks interleaved. Each event has the Source MUID and the Request ID of its message, and the
    // Device opens both Requests.
    let (mut a, mut b) = discovered();
    let mut c = dev(0x43, &[0x0333_3333]);
    let mut buf = [0u8; 512];
    let n = c.discovery(0, 0, &mut buf).unwrap();
    let r = deliver(&mut b, 0, &buf[..n]).1[0].1.clone();
    let _ = deliver(&mut c, 0, &r);
    pe_inquire(&mut a, &mut b);
    pe_inquire(&mut c, &mut b);
    // Chunk size 30: 6 bytes in Chunk 1 (with "{}"), 4 bytes in Chunk 2.
    let data_a = [1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10];
    let data_c = [11u8, 12, 13, 14, 15, 16, 17, 18, 19, 20];
    let (ia, mut wa) = a
        .pe_request(b.muid(), PeKind::SetInquiry, b"{}", Some(10), Some(30))
        .unwrap();
    let (ic, mut wc) = c
        .pe_request(b.muid(), PeKind::SetInquiry, b"{}", Some(10), Some(30))
        .unwrap();
    assert_eq!(ia, ic, "the same Request ID between two pairs of MUIDs");
    let mut msgs = Vec::new();
    for (from_a, at) in [(true, 0..6), (false, 0..6), (true, 6..10), (false, 6..10)] {
        let n = if from_a {
            wa.write_next(&data_a[at], &mut buf)
        } else {
            wc.write_next(&data_c[at], &mut buf)
        }
        .unwrap();
        msgs.push(buf[..n].to_vec());
    }
    let mut got = Vec::new();
    for m in &msgs {
        let (ev, r) = deliver(&mut b, 0, m);
        assert!(r.is_empty(), "no NAK");
        let Some(Event::PeChunk {
            muid,
            kind: PeKind::SetInquiry,
            chunk,
        }) = ev
        else {
            panic!("{ev:?}")
        };
        got.push((
            muid,
            chunk.request_id,
            chunk.total,
            chunk.number,
            chunk.data.to_vec(),
        ));
    }
    assert_eq!(
        got,
        [
            (a.muid(), ia, 2, 1, data_a[..6].to_vec()),
            (c.muid(), ia, 2, 1, data_c[..6].to_vec()),
            (a.muid(), ia, 2, 2, data_a[6..].to_vec()),
            (c.muid(), ia, 2, 2, data_c[6..].to_vec()),
        ]
    );
}

#[cfg(feature = "property-exchange")]
#[test]
fn responder_keeps_pe_capabilities_and_sends_subscriptions() {
    // REQ-CI-PEF-044, REQ-CI-PEF-045 with the Initiator role on: B gets the PE Capabilities of A
    // from the Inquiry of A, keeps them until A's MUID is invalidated, and sends A Subscription
    // messages.
    let (mut a, mut b) = discovered();
    let mut buf = [0u8; 512];
    let sub_header = br#"{"command":"full","subscribeId":"1"}"#;
    assert_eq!(
        b.pe_request(a.muid(), PeKind::Subscription, sub_header, Some(1), None)
            .err(),
        Some(DeviceError::NotInquired)
    );
    let caps = zmidi_ci::PeCapabilities {
        simultaneous: 3,
        major: Some(1),
        minor: Some(2),
    };
    let n = caps
        .write_inquiry(a.muid(), b.muid(), Framing::Bare, &mut buf)
        .unwrap();
    let (ev, r) = deliver(&mut b, 0, &buf[..n]);
    assert!(ev.is_none());
    let mut want = head(0x7F, 0x31, b.muid(), a.muid());
    want.extend([2, 0, 0]);
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].1, want, "the Reply with B's own values");
    assert_eq!(b.peer(a.muid()).unwrap().pe_capabilities, Some(caps));
    // Kept over a new Discovery of A.
    let n = a.discovery(0, 0, &mut buf).unwrap();
    let _ = deliver(&mut b, 0, &buf[..n]);
    assert_eq!(b.peer(a.muid()).unwrap().pe_capabilities, Some(caps));
    // B, the Responder, sends a Subscription message to A.
    let (id, mut w) = b
        .pe_request(a.muid(), PeKind::Subscription, sub_header, Some(1), None)
        .unwrap();
    let n = w.write_next(b"x", &mut buf).unwrap();
    let h = header(&buf[..n]);
    assert_eq!(
        (h.sub_id2, h.source, h.destination),
        (0x38, b.muid(), a.muid())
    );
    let (ev, _) = deliver(&mut a, 0, &buf[..n]);
    assert!(matches!(
        ev,
        Some(Event::PeChunk { muid, kind: PeKind::Subscription, chunk })
            if muid == b.muid() && chunk.request_id == id && chunk.data == b"x"
    ));
    // Until the MUID of A is invalidated.
    let old = a.muid();
    let n = a.invalidate(&mut rng(&[0x0666_6666]), &mut buf).unwrap();
    let _ = deliver(&mut b, 0, &buf[..n]);
    assert!(b.peer(old).is_none());
    let n = a.discovery(0, 0, &mut buf).unwrap();
    let _ = deliver(&mut b, 0, &buf[..n]);
    assert_eq!(b.peer(a.muid()).unwrap().pe_capabilities, None);
    assert_eq!(
        b.pe_request(a.muid(), PeKind::Subscription, sub_header, Some(1), None)
            .err(),
        Some(DeviceError::NotInquired)
    );
}

#[cfg(all(feature = "property-exchange", feature = "process-inquiry"))]
#[test]
fn invalidate_ends_the_transactions_of_the_peer() {
    // 5.9; REQ-CI-INV-006: an Invalidate MUID of a peer ends its PE Requests (inquiry and reply)
    // and its MIDI Message Report.
    let (mut a, mut b) = discovered();
    let mut buf = [0u8; 512];
    pe_inquire(&mut a, &mut b);
    pe_inquire(&mut b, &mut a);
    // B's two Request slots: one Request of B (inquiry), one of A (B replies).
    let _ = b
        .pe_request(a.muid(), PeKind::GetInquiry, b"{}", Some(0), None)
        .unwrap();
    let (_, mut w) = a
        .pe_request(b.muid(), PeKind::GetInquiry, b"{}", Some(0), None)
        .unwrap();
    let n = w.write_next(&[], &mut buf).unwrap();
    let (ev, r) = deliver(&mut b, 0, &buf[..n]);
    assert!(matches!(ev, Some(Event::PeChunk { .. })) && r.is_empty());
    assert_eq!(
        b.pe_request(a.muid(), PeKind::GetInquiry, b"{}", Some(0), None)
            .err(),
        Some(DeviceError::Busy)
    );
    // Full: one more inquiry of A gets NAK 0x40 with its Request ID.
    let (id2, mut w) = a
        .pe_request(b.muid(), PeKind::GetInquiry, b"{}", Some(0), None)
        .unwrap();
    let n = w.write_next(&[], &mut buf).unwrap();
    let (_, r) = deliver(&mut b, 0, &buf[..n]);
    let k = nak_of(&r[0].1);
    assert_eq!(
        (k.status, k.details),
        (
            NakStatus::Error,
            Details::PropertyExchange {
                request_id: id2,
                chunk: 1
            }
        )
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
    b.message_report_inquiry(a.muid(), DeviceId::Group, &q, &mut buf)
        .unwrap();
    assert!(b.report_open(a.muid()));
    let old_a = a.muid();
    let n = a.invalidate(&mut rng(&[0x0666_6666]), &mut buf).unwrap();
    let (ev, r) = deliver(&mut b, 0, &buf[..n]);
    assert_eq!(ev, Some(Event::PeerInvalidated { muid: old_a }));
    assert!(r.is_empty());
    assert!(!b.report_open(old_a));
    // A with its new MUID: both slots are free again.
    let n = a.discovery(0, 0, &mut buf).unwrap();
    let r = deliver(&mut b, 0, &buf[..n]).1[0].1.clone();
    let _ = deliver(&mut a, 0, &r);
    pe_inquire(&mut b, &mut a);
    for _ in 0..2 {
        assert!(b
            .pe_request(a.muid(), PeKind::GetInquiry, b"{}", Some(0), None)
            .is_ok());
    }
}

#[cfg(all(feature = "property-exchange", feature = "process-inquiry"))]
#[test]
fn invalidate_of_the_local_muid_ends_its_transactions() {
    // 5.9; REQ-CI-INV-005: an Invalidate MUID with B's MUID as Target ends B's PE Requests and
    // its MIDI Message Report, and B takes a new MUID from the caller's random number.
    let (mut a, mut b) = discovered();
    let mut buf = [0u8; 512];
    pe_inquire(&mut a, &mut b);
    pe_inquire(&mut b, &mut a);
    let _ = b
        .pe_request(a.muid(), PeKind::GetInquiry, b"{}", Some(0), None)
        .unwrap();
    let (_, mut w) = a
        .pe_request(b.muid(), PeKind::GetInquiry, b"{}", Some(0), None)
        .unwrap();
    let n = w.write_next(&[], &mut buf).unwrap();
    let _ = deliver(&mut b, 0, &buf[..n]);
    assert_eq!(
        b.pe_request(a.muid(), PeKind::GetInquiry, b"{}", Some(0), None)
            .err(),
        Some(DeviceError::Busy)
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
    b.message_report_inquiry(a.muid(), DeviceId::Group, &q, &mut buf)
        .unwrap();
    let old_b = b.muid();
    let n = zmidi_ci::InvalidateMuid { target: old_b }
        .write(a.muid(), Framing::Bare, &mut buf)
        .unwrap();
    let msg = buf[..n].to_vec();
    let mut r = b.receive(0, &msg, 0, &mut rng(&[0x0BBB_BBBB]));
    assert_eq!(
        r.event(),
        Some(Event::MuidChanged {
            old: old_b,
            new: Muid::new(0x0BBB_BBBB).unwrap()
        })
    );
    assert!(r.write_next(&mut buf).is_none());
    assert_eq!(b.muid().value(), 0x0BBB_BBBB);
    assert!(!b.report_open(a.muid()));
    // The PE Capabilities are inquired again (REQ-CI-INV-007), then both slots are free.
    assert_eq!(
        b.pe_request(a.muid(), PeKind::GetInquiry, b"{}", Some(0), None)
            .err(),
        Some(DeviceError::NotInquired)
    );
    pe_inquire(&mut b, &mut a);
    for _ in 0..2 {
        assert!(b
            .pe_request(a.muid(), PeKind::GetInquiry, b"{}", Some(0), None)
            .is_ok());
    }
}

#[cfg(feature = "property-exchange")]
#[test]
fn receives_128_and_512_byte_messages() {
    // REQ-CI-LEN-001, REQ-CI-LEN-002: messages of 128 and 512 bytes, collected from UMP SysEx7
    // packets in a 512-byte buffer, reach the Device whole.
    let (a, mut b) = discovered();
    let g = Group::new(0).unwrap();
    for size in [128usize, 512] {
        let data = vec![0x33u8; size - 13 - 9 - 2];
        let c = zmidi_ci::PeChunk {
            request_id: 1,
            header: b"{}",
            total: 1,
            number: 1,
            data: &data,
        };
        let mut buf = [0u8; 512];
        let n = c
            .write(
                PeKind::GetReply,
                a.muid(),
                b.muid(),
                Framing::Bare,
                &mut buf,
            )
            .unwrap();
        assert_eq!(n, size);
        let mut rx = SysEx7Reassembler::<512>::new(g);
        let mut done = None;
        for p in SysEx7Packets::try_new(g, &buf[..n]).unwrap() {
            let mut words = [0u32; 2];
            p.to_words(&mut words).unwrap();
            if let Some(SysEx7Event::Complete(m)) = rx.push(&words) {
                done = Some(m.to_vec());
            }
        }
        let msg = done.unwrap();
        assert_eq!(msg.len(), size);
        let (ev, r) = deliver(&mut b, 0, &msg);
        assert!(r.is_empty());
        match ev {
            Some(Event::PeChunk {
                muid,
                kind: PeKind::GetReply,
                chunk,
            }) => assert_eq!((muid, chunk.data), (a.muid(), &data[..])),
            e => panic!("{size}: {e:?}"),
        }
    }
}

#[test]
fn ack_and_nak_fit_the_peer_size() {
    // REQ-CI-ACK-005, REQ-CI-NAK-005: the size that the peer reported in Discovery, or 128.
    let mut a = dev(0x41, &[0x0111_1111]);
    let mut c = config(0x46);
    c.categories = Categories::default();
    c.max_sysex_size = 200;
    let mut small: Dev = CiDevice::new(c, &mut rng(&[0x0555_5555])).unwrap();
    let mut buf = [0u8; 512];
    let n = small.discovery(0, 0, &mut buf).unwrap();
    let _ = deliver(&mut a, 0, &buf[..n]);
    assert_eq!(a.peer(small.muid()).unwrap().max_sysex_size, 200);
    let ack = zmidi_ci::Ack {
        original_sub_id2: 0x34,
        status: AckStatus::Ack,
        details: Details::Reserved,
        text: &[],
    };
    let k = Nak {
        original_sub_id2: 0x34,
        status: NakStatus::Nak,
        details: Details::Reserved,
        text: &[],
    };
    let long = "x".repeat(300);
    let fb = DeviceId::FunctionBlock;
    let unknown = Muid::new(5).unwrap();
    for (peer, size) in [(small.muid(), 200), (unknown, 128)] {
        let n = a.ack(peer, fb, &ack, &long, &mut buf).unwrap();
        assert_eq!(n, size);
        let n = a.nak(peer, fb, &k, &long, &mut buf).unwrap();
        assert_eq!(n, size);
        let got = nak_of(&buf[..n]);
        assert_eq!(got.text.len(), size - 23);
        // Shortened at a character boundary: whole 6-byte escapes.
        let n = a.nak(peer, fb, &k, &"é".repeat(40), &mut buf).unwrap();
        assert_eq!(n, 23 + (size - 23) / 6 * 6);
    }
    // A text that fits is not shortened.
    let n = a.ack(small.muid(), fb, &ack, "ok", &mut buf).unwrap();
    assert_eq!(n, 25);
}

/// Checks the Destination MUID of a message that a Device wrote (REQ-CI-MUID-010): the Broadcast
/// MUID only in Discovery, Invalidate MUID and the four Profile reports, else `peer`.
#[cfg(feature = "full")]
fn check_destination(msg: &[u8], peer: Muid) {
    let h = header(msg);
    if matches!(h.sub_id2, 0x70 | 0x7E | 0x24..=0x27) {
        assert_eq!(h.destination, Muid::BROADCAST, "{:#04x}", h.sub_id2);
    } else {
        assert_eq!(h.destination, peer, "{:#04x}", h.sub_id2);
    }
}

#[cfg(feature = "full")]
#[test]
fn broadcast_destination_only_where_allowed() {
    // REQ-CI-MUID-010: each message type that a Device writes, with its Destination MUID.
    let mut a = dev(0x41, &[0x0111_1111]);
    let mut b = dev(0x42, &[0x0222_2222]);
    let mut buf = [0u8; 512];
    let mut sent = Vec::new();
    // Each message from A goes to B (and each reply of B to A).
    macro_rules! a_to_b {
        ($n:expr) => {{
            let n = $n;
            let m = buf[..n].to_vec();
            check_destination(&m, b.muid());
            sent.push(m[3]);
            let (ev, r) = deliver(&mut b, 0, &m);
            for (_, x) in &r {
                check_destination(x, a.muid());
                sent.push(x[3]);
                let _ = deliver(&mut a, 0, x);
            }
            ev.map(|e| format!("{e:?}"))
        }};
    }
    macro_rules! b_to_a {
        ($n:expr) => {{
            let n = $n;
            let m = buf[..n].to_vec();
            check_destination(&m, a.muid());
            sent.push(m[3]);
            let _ = deliver(&mut a, 0, &m);
        }};
    }
    a_to_b!(a.discovery(0, 0, &mut buf).unwrap());
    a_to_b!(a.endpoint_info_inquiry(b.muid(), &mut buf).unwrap());
    let reports: Vec<_> = b.add_profile(entry(false)).unwrap().collect();
    b_to_a!(b.profile_report(&reports[0], &mut buf).unwrap().1);
    a_to_b!(a
        .profile_inquiry(b.muid(), DeviceId::FunctionBlock, &mut buf)
        .unwrap());
    a_to_b!(a
        .set_profile_on(b.muid(), DeviceId::Channel(0), gm(), 1, None, &mut buf)
        .unwrap());
    let address = entry(false).address;
    let r = b.set_profile_state(address, gm(), true, 1).unwrap();
    b_to_a!(b.profile_report(&r, &mut buf).unwrap().1);
    a_to_b!(a
        .set_profile_off(b.muid(), DeviceId::Channel(0), gm(), &mut buf)
        .unwrap());
    let r = b.set_profile_state(address, gm(), false, 1).unwrap();
    b_to_a!(b.profile_report(&r, &mut buf).unwrap().1);
    a_to_b!(a
        .profile_details_inquiry(b.muid(), DeviceId::Channel(0), gm(), 0, &mut buf)
        .unwrap());
    let reply = zmidi_ci::ProfileDetailsReply {
        profile: gm(),
        target: 0x40,
        data: &[],
    };
    b_to_a!(b
        .profile_details_reply(a.muid(), DeviceId::Channel(0), &reply, &mut buf)
        .unwrap());
    a_to_b!(a.pe_capabilities_inquiry(b.muid(), &mut buf).unwrap());
    let (id, mut w) = a
        .pe_request(b.muid(), PeKind::GetInquiry, b"{}", Some(0), None)
        .unwrap();
    a_to_b!(w.write_next(&[], &mut buf).unwrap());
    let mut w = b
        .pe_reply(a.muid(), PeKind::GetReply, id, b"{}", Some(1), None)
        .unwrap();
    b_to_a!(w.write_next(b"x", &mut buf).unwrap());
    a_to_b!(a.pi_capabilities_inquiry(b.muid(), &mut buf).unwrap());
    let q = MessageReportInquiry {
        data_control: DataControl::Full,
        system: Default::default(),
        channel: ChannelBits {
            pitch_bend: true,
            ..Default::default()
        },
        note: Default::default(),
    };
    a_to_b!(a
        .message_report_inquiry(b.muid(), DeviceId::Group, &q, &mut buf)
        .unwrap());
    let supported = MessageReportReply::default();
    b_to_a!(
        b.message_report(
            a.muid(),
            DeviceId::Group,
            0,
            &q,
            &supported,
            Protocol::Midi1,
            &Values,
            &mut buf
        )
        .unwrap()
        .0
    );
    b_to_a!(b
        .message_report_end(a.muid(), DeviceId::Group, &mut buf)
        .unwrap());
    b_to_a!(b
        .abort_report(a.muid(), DeviceId::Group, false, &mut buf)
        .unwrap());
    let ack = zmidi_ci::Ack {
        original_sub_id2: 0x34,
        status: AckStatus::Ack,
        details: Details::Reserved,
        text: &[],
    };
    b_to_a!(b
        .ack(a.muid(), DeviceId::FunctionBlock, &ack, "", &mut buf)
        .unwrap());
    // An automatic NAK: an unknown Sub-ID#2.
    let mut m = head(0x7F, 0x2A, a.muid(), b.muid());
    m.push(0);
    let n = m.len();
    buf[..n].copy_from_slice(&m);
    let ev = a_to_b!(n);
    assert!(ev.is_none());
    let reports: Vec<_> = b.remove_profile(address, gm()).unwrap().collect();
    b_to_a!(b.profile_report(&reports[0], &mut buf).unwrap().1);
    a_to_b!(a.invalidate(&mut rng(&[0x0666_6666]), &mut buf).unwrap());
    let mut kinds = sent.clone();
    kinds.sort_unstable();
    kinds.dedup();
    // Every message type of a Device (0x2A: the message of the test that got the NAK).
    assert_eq!(
        kinds,
        [
            0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2A, 0x30, 0x31, 0x34,
            0x35, 0x40, 0x41, 0x42, 0x43, 0x44, 0x70, 0x71, 0x72, 0x73, 0x7D, 0x7E, 0x7F
        ]
    );
}

#[test]
fn device_rules_at_their_limits() {
    let mut buf = [0u8; 512];
    // The Group of a reply record: the first Group of the Function Block plus the offset.
    let mut a = dev(0x41, &[0x0111_1111]);
    let mut c = config(0x42);
    c.first_group = 3;
    c.group_count = 2;
    let mut b: Dev = CiDevice::new(c, &mut rng(&[0x0222_2222])).unwrap();
    let n = a.discovery(0, 0, &mut buf).unwrap();
    let (_, r) = deliver(&mut b, 4, &buf[..n]);
    assert_eq!(r[0].0, 4);
    // A random source that gives only reserved values: the MUID steps from 0 to 1.
    let d: Dev = CiDevice::new(config(0x43), &mut || 0x0FFF_FFFF).unwrap();
    assert_eq!(d.muid(), Muid::new(1).unwrap());
    // Function Block change: Groups 0 to 15 only.
    for (first, count, ok) in [
        (15, 1, true),
        (0, 16, true),
        (5, 4, true),
        (16, 1, false),
        (0, 0, false),
        (15, 2, false),
    ] {
        let mut d = dev(0x44, &[0x0444_4444]);
        let r = d.function_block_changed(first, count, &mut rng(&[0x0555_5555]), &mut buf);
        assert_eq!(r.is_ok(), ok, "{first} {count}");
        if !ok {
            assert_eq!(r, Err(DeviceError::InvalidConfig));
        }
    }
    // A header error other than a reserved Device ID: the message is rejected, with no NAK.
    let (mut a, mut b) = discovered();
    let n = a.endpoint_info_inquiry(b.muid(), &mut buf).unwrap();
    let mut m = buf[..n].to_vec();
    m[4] = 0x80;
    let (ev, r) = deliver(&mut b, 0, &m);
    assert!(matches!(ev, Some(Event::Rejected { .. })) && r.is_empty());
    // A space in a Message Text is written.
    let k = Nak {
        original_sub_id2: 0x72,
        status: NakStatus::Nak,
        details: Details::Reserved,
        text: &[],
    };
    let n = a
        .nak(b.muid(), DeviceId::FunctionBlock, &k, "a b", &mut buf)
        .unwrap();
    assert_eq!(&buf[n - 3..n], b"a b");
    // A Discovery like its own on an Output Path that it did not send: not an echo, a collision.
    let n = a.discovery(0, 0, &mut buf).unwrap();
    let mut other = buf[..n].to_vec();
    assert_eq!(other[29], 0, "Output Path ID");
    other[29] = 5;
    let old = a.muid();
    let (ev, _) = deliver(&mut a, 0, &other);
    assert!(
        matches!(ev, Some(Event::MuidChanged { old: o, .. }) if o == old),
        "{ev:?}"
    );
    // One window for the Discovery of all paths (REQ-CI-DISC-015): a Discovery on another
    // path does not start a new round, so the reply on path 0 again is a duplicate.
    let mut a = dev(0x41, &[0x0111_1111]);
    let mut b = dev(0x42, &[0x0222_2222]);
    let n = a.discovery(0, 0, &mut buf).unwrap();
    let r0 = deliver(&mut b, 0, &buf[..n]).1[0].1.clone();
    assert!(matches!(
        deliver_at(&mut a, 0, &r0, 1000).0,
        Some(Event::Discovered { .. })
    ));
    a.discovery(1, 2000, &mut buf).unwrap();
    assert_eq!(
        deliver_at(&mut a, 0, &r0, 4000).0,
        Some(Event::PeerInvalidated { muid: b.muid() })
    );
    // A peer that reports a size of 0: the default of 128 bytes applies to the messages to it.
    let mut a = dev(0x41, &[0x0111_1111]);
    let mut b = dev(0x42, &[0x0222_2222]);
    let n = a.discovery(0, 0, &mut buf).unwrap();
    let mut r = deliver(&mut b, 0, &buf[..n]).1[0].1.clone();
    r[25..29].copy_from_slice(&[0, 0, 0, 0]);
    assert!(matches!(
        deliver(&mut a, 0, &r).0,
        Some(Event::Discovered { .. })
    ));
    let k = Nak {
        original_sub_id2: 0x72,
        status: NakStatus::Nak,
        details: Details::Reserved,
        text: &[],
    };
    let n = a
        .nak(
            b.muid(),
            DeviceId::FunctionBlock,
            &k,
            &"x".repeat(300),
            &mut buf,
        )
        .unwrap();
    assert_eq!(n, 128);
}

#[cfg(feature = "property-exchange")]
#[test]
fn property_exchange_rules_at_their_limits() {
    let mut buf = [0u8; 512];
    // A Device with no Property Exchange in its configuration: NAK 0x01.
    let mut a = dev(0x41, &[0x0111_1111]);
    let mut c = config(0x42);
    c.categories.property_exchange = false;
    let mut b: Dev = CiDevice::new(c, &mut rng(&[0x0222_2222])).unwrap();
    let n = a.discovery(0, 0, &mut buf).unwrap();
    let r = deliver(&mut b, 0, &buf[..n]).1[0].1.clone();
    deliver(&mut a, 0, &r);
    let n = a.pe_capabilities_inquiry(b.muid(), &mut buf).unwrap();
    let (_, r) = deliver(&mut b, 0, &buf[..n]);
    assert_eq!(nak_of(&r[0].1).status, NakStatus::NotSupported);
    // A reserved Device ID in a message with SysEx framing (F0 ... F7) gets its NAK too.
    let (mut a, mut b) = discovered();
    let n = a.pe_capabilities_inquiry(b.muid(), &mut buf).unwrap();
    let mut m = std::vec![0xF0];
    m.extend_from_slice(&buf[..n]);
    m.push(0xF7);
    m[2] = 0x20;
    let (ev, r) = deliver(&mut b, 0, &m);
    assert!(matches!(ev, Some(Event::Rejected { .. })));
    assert_eq!(nak_of(&r[0].1).status, NakStatus::NotInUse);
    // pe_reply gives no inquiry and no Notify.
    let (mut a, _) = discovered();
    let peer = Muid::new(0x0222_2222).unwrap();
    for kind in [PeKind::GetInquiry, PeKind::SetInquiry, PeKind::Notify] {
        assert_eq!(
            a.pe_reply(peer, kind, 1, b"{}", Some(0), None).err(),
            Some(DeviceError::NotSupported),
            "{kind:?}"
        );
    }
}

#[cfg(feature = "profiles")]
#[test]
fn profile_rules_at_their_limits() {
    let mut buf = [0u8; 512];
    let (mut a, mut b) = discovered();
    let _ = b.add_profile(entry(false)).unwrap().count();
    let n = a
        .profile_inquiry(b.muid(), DeviceId::FunctionBlock, &mut buf)
        .unwrap();
    let (_, r) = deliver(&mut b, 0, &buf[..n]);
    for (_, m) in &r {
        deliver(&mut a, 0, m);
    }
    // A Function Block address (Device ID 0x7F) is Group 0 of the Profile table, also when the
    // Function Block starts at Group 2 and the inquiry comes on Group 3.
    {
        let mut c = config(0x45);
        c.first_group = 2;
        c.group_count = 2;
        let mut fb: Dev = CiDevice::new(c, &mut rng(&[0x0666_6666])).unwrap();
        let mut x = dev(0x46, &[0x0777_7777]);
        let n = x.discovery(0, 0, &mut buf).unwrap();
        let r = deliver(&mut fb, 2, &buf[..n]).1[0].1.clone();
        deliver(&mut x, 0, &r);
        let mut e = entry(false);
        e.address = ProfileAddress {
            device_id: DeviceId::FunctionBlock,
            group: 0,
        };
        let _ = fb.add_profile(e).unwrap().count();
        let n = x
            .profile_inquiry(fb.muid(), DeviceId::FunctionBlock, &mut buf)
            .unwrap();
        let (_, r) = deliver(&mut fb, 3, &buf[..n]);
        assert!(
            r.iter().all(|(g, _)| *g == 3),
            "the replies go on the Group of the inquiry"
        );
        let last = &r.last().unwrap().1;
        match deliver(&mut x, 3, last).0 {
            Some(Event::ProfileInquiryReply {
                device_id, reply, ..
            }) => {
                assert_eq!(device_id, DeviceId::FunctionBlock);
                assert_eq!(reply.disabled().collect::<Vec<_>>(), [gm()]);
            }
            e => panic!("{e:?}"),
        }
        // Set Profile On to Device ID 0x7F on Group 3: the address is Group 0 of the table.
        let n = x
            .set_profile_on(fb.muid(), DeviceId::FunctionBlock, gm(), 0, None, &mut buf)
            .unwrap();
        match deliver(&mut fb, 3, &buf[..n]).0 {
            Some(Event::SetProfile { address, .. }) => assert_eq!(
                address,
                ProfileAddress {
                    device_id: DeviceId::FunctionBlock,
                    group: 0
                }
            ),
            e => panic!("{e:?}"),
        }
    }
    // The count may be the Maximum Number of Channels of the peer.
    assert!(a
        .set_profile_on(b.muid(), DeviceId::Channel(0), gm(), 4, Some(4), &mut buf)
        .is_ok());
    // Profile Specific Data gives its event.
    let n = zmidi_ci::ProfileSpecificData {
        profile: gm(),
        data: &[1, 2, 3],
    }
    .write(
        false,
        DeviceId::Channel(0),
        a.muid(),
        b.muid(),
        Framing::Bare,
        &mut buf,
    )
    .unwrap();
    assert!(matches!(
        deliver(&mut b, 0, &buf[..n]).0,
        Some(Event::ProfileSpecificData { .. })
    ));
    // 4 Profile Details Inquiries wait for a reply; a 5th takes the place of the first.
    for t in 0x40..0x45u8 {
        a.profile_details_inquiry(b.muid(), DeviceId::Channel(0), gm(), t, &mut buf)
            .unwrap();
    }
    for (t, notification) in [(0x40u8, true), (0x44, false), (0x41, false)] {
        let reply = zmidi_ci::ProfileDetailsReply {
            profile: gm(),
            target: t,
            data: &[1],
        };
        let n = b
            .profile_details_reply(a.muid(), DeviceId::Channel(0), &reply, &mut buf)
            .unwrap();
        let (ev, _) = deliver(&mut a, 0, &buf[..n]);
        assert!(
            matches!(ev, Some(Event::ProfileDetailsReply { notification: x, .. }) if x == notification),
            "{t:#04x}: {ev:?}"
        );
    }
}
