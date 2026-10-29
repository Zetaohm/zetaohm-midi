//! The responder-only build (feature `initiator` off, DEC-CI-001, REQ-CI-TIER-002): a reply that only an
//! Initiator expects gives no event and no reply; Discovery, Invalidate MUID, ACK, NAK and the inquiries
//! work; every Initiator call gives `NotSupported`. The messages of the other Device come from the
//! message writers.
#![cfg(not(feature = "initiator"))]

use zmidi_ci::{
    Ack, AckStatus, Categories, CiDevice, Details, DeviceConfig, DeviceError, DeviceId,
    DeviceIdentity, Discovery, Framing, InvalidateMuid, ManufacturerId, Muid, PeKind,
    PiCapabilities, ProfileId, ProfileLevel,
};

type Dev = CiDevice<'static, 2, 1, 2>;

const PEER: Muid = match Muid::new(0x0111_1111) {
    Ok(m) => m,
    Err(_) => panic!(),
};

fn identity() -> DeviceIdentity {
    DeviceIdentity {
        manufacturer: ManufacturerId::OneByte(0x41),
        family: 1,
        model: 2,
        revision: [0, 0, 0, 1],
    }
}

fn device() -> Dev {
    let config = DeviceConfig {
        identity: identity(),
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
        channels_in_use: 1,
        product_instance_id: Some("SN-42"),
        framing: Framing::Bare,
        process_inquiry: PiCapabilities {
            midi_message_report: true,
        },
    };
    CiDevice::new(config, &mut || 0x0222_2222).unwrap()
}

/// Gives `msg` to the Device; gives the event and the replies.
fn deliver(d: &mut Dev, msg: &[u8]) -> (Option<String>, Vec<Vec<u8>>) {
    let mut r = d.receive(0, msg, 0, &mut || 0x0333_3333);
    let ev = r.event().map(|e| format!("{e:?}"));
    let mut out = Vec::new();
    let mut buf = [0u8; Dev::MAX_REPLY_LEN];
    while let Some(item) = r.write_next(&mut buf) {
        let (_, n) = item.unwrap();
        out.push(buf[..n].to_vec());
    }
    (ev, out)
}

fn discovery_from(src: Muid) -> Vec<u8> {
    let mut buf = [0u8; 64];
    let d = Discovery {
        identity: identity(),
        categories: Categories::default(),
        max_sysex_size: 256,
        output_path_id: Some(3),
    };
    let n = d.write(src, Framing::Bare, &mut buf).unwrap();
    buf[..n].to_vec()
}

#[test]
fn replies_for_an_initiator_are_ignored() {
    // DEC-CI-001 answer (a): each reply kind, also with a Device ID that the Device would NAK, gives
    // no event and no reply.
    let mut d = device();
    let me = d.muid().to_bytes();
    for sub in [0x71u8, 0x73, 0x21, 0x31, 0x41, 0x43, 0x44] {
        for dev in [0x7Fu8, 0x05] {
            let mut m = vec![0x7E, dev, 0x0D, sub, 0x02];
            m.extend(PEER.to_bytes());
            m.extend(me);
            m.extend([0x00; 20]);
            assert_eq!(deliver(&mut d, &m), (None, vec![]), "sub {sub:#04x}");
        }
    }
}

#[cfg(feature = "profiles")]
#[test]
fn profile_messages_for_an_initiator_are_ignored() {
    // DEC-CI-001 addendum of 2026-10-26: Reply to Profile Details Inquiry (0x29) and the Profile
    // Enabled, Disabled, Added and Removed Reports (0x24 to 0x27) give no event and no reply, to this
    // MUID and to the Broadcast MUID; with the Initiator role they give an event (tests/device.rs).
    let mut d = device();
    for dst in [d.muid(), Muid::BROADCAST] {
        for sub in [0x24u8, 0x25, 0x26, 0x27, 0x29] {
            let mut m = vec![0x7E, 0x7F, 0x0D, sub, 0x02];
            m.extend(PEER.to_bytes());
            m.extend(dst.to_bytes());
            m.extend([0x00; 20]);
            assert_eq!(deliver(&mut d, &m), (None, vec![]), "sub {sub:#04x}");
        }
    }
}

#[test]
fn discovery_and_management_work() {
    let mut d = device();
    // Discovery -> Reply to Discovery with the Output Path ID; the peer is kept (DEC-011 D-11).
    let (_, r) = deliver(&mut d, &discovery_from(PEER));
    assert_eq!((r.len(), r[0][3], r[0][r[0].len() - 2]), (1, 0x71, 3));
    assert_eq!(d.peer(PEER).map(|p| p.max_sysex_size), Some(256));
    // Inquiry: Endpoint Information -> Reply with the Product Instance ID.
    let mut m = vec![0x7E, 0x7F, 0x0D, 0x72, 0x02];
    m.extend(PEER.to_bytes());
    m.extend(d.muid().to_bytes());
    m.push(0x00);
    let (_, r) = deliver(&mut d, &m);
    assert_eq!((r[0][3], &r[0][16..]), (0x73, &b"SN-42"[..]));
    // ACK is read in both roles.
    let mut buf = [0u8; 64];
    let ack = Ack {
        original_sub_id2: 0x34,
        status: AckStatus::Ack,
        details: Details::Reserved,
        text: &[],
    };
    let n = ack
        .write(
            "",
            DeviceId::FunctionBlock,
            PEER,
            d.muid(),
            None,
            Framing::Bare,
            &mut buf,
        )
        .unwrap();
    let (ev, r) = deliver(&mut d, &buf[..n]);
    assert!(ev.is_some_and(|e| e.starts_with("Ack")) && r.is_empty());
    // Invalidate MUID of the peer: the peer is removed.
    let n = InvalidateMuid { target: PEER }
        .write(Muid::new(0x0444_4444).unwrap(), Framing::Bare, &mut buf)
        .unwrap();
    let (ev, _) = deliver(&mut d, &buf[..n]);
    assert!(ev.is_some_and(|e| e.starts_with("PeerInvalidated")));
    assert!(d.peer(PEER).is_none());
}

#[test]
fn own_muid_discovery_is_a_collision() {
    // This Device never sends Discovery, so a Discovery with its MUID is never an echo
    // (REQ-CI-MUID-012, DEC-011 D-5).
    let mut d = device();
    let old = d.muid();
    let (ev, r) = deliver(&mut d, &discovery_from(old));
    assert!(ev.is_some_and(|e| e.starts_with("MuidChanged")));
    assert_ne!(d.muid(), old);
    assert_eq!(r.len(), 1);
}

#[test]
fn initiator_calls_give_not_supported() {
    // DEC-CI-001: every Initiator call gives NotSupported and writes nothing; no Discovery window.
    let mut d = device();
    let _ = deliver(&mut d, &discovery_from(PEER));
    let mut buf = [0u8; 512];
    let gm = ProfileId::Standard {
        bank: 0x21,
        number: 0,
        version: 1,
        level: ProfileLevel::Minimum,
    };
    let ns = Err(DeviceError::NotSupported);
    assert_eq!(d.discovery(0, 0, &mut buf), ns);
    assert_eq!(d.endpoint_info_inquiry(PEER, &mut buf), ns);
    assert_eq!(
        d.profile_inquiry(PEER, DeviceId::FunctionBlock, &mut buf),
        ns
    );
    assert_eq!(
        d.profile_details_inquiry(PEER, DeviceId::Channel(0), gm, 0, &mut buf),
        ns
    );
    assert_eq!(
        d.set_profile_on(PEER, DeviceId::Channel(0), gm, 1, None, &mut buf),
        ns
    );
    assert_eq!(d.pe_capabilities_inquiry(PEER, &mut buf), ns);
    assert_eq!(
        d.pe_request(PEER, PeKind::GetInquiry, b"{}", Some(0), None)
            .err(),
        Some(DeviceError::NotSupported)
    );
    assert_eq!(d.pi_capabilities_inquiry(PEER, &mut buf), ns);
    assert_eq!(d.poll(u64::MAX), None);
}

#[test]
fn categories_follow_their_features() {
    // REQ-CI-TIER-001 in the responder-only build: an inquiry of a category whose feature is off
    // gets NAK 0x01 Not Supported; with the feature on it gets its reply.
    let mut d = device();
    let (src, dst, f) = (PEER, d.muid(), Framing::Bare);
    let mut buf = [0u8; 64];
    let cases = [
        (
            zmidi_ci::ProfileInquiry::write(DeviceId::FunctionBlock, src, dst, f, &mut buf)
                .unwrap(),
            cfg!(feature = "profiles"),
            0x21,
        ),
        (0, cfg!(feature = "property-exchange"), 0x31),
        (0, cfg!(feature = "process-inquiry"), 0x41),
    ];
    for (i, (n, on, reply)) in cases.into_iter().enumerate() {
        let n = match i {
            1 => zmidi_ci::PeCapabilities {
                simultaneous: 1,
                major: None,
                minor: None,
            }
            .write_inquiry(src, dst, f, &mut buf)
            .unwrap(),
            2 => zmidi_ci::PiCapabilitiesInquiry::write(src, dst, f, &mut buf).unwrap(),
            _ => n,
        };
        let msg = buf[..n].to_vec();
        let (_, r) = deliver(&mut d, &msg);
        let want = if on { reply } else { 0x7F };
        // With Profiles on, the replies depend on the Profile table (tests of tests/device.rs).
        if i == 0 && on {
            continue;
        }
        assert_eq!(
            r.first().map(|m| m[3]),
            Some(want),
            "inquiry {:#04x}",
            msg[3]
        );
    }
}

// Property Exchange messages need the category (DEC-017).
#[cfg(feature = "property-exchange")]
#[test]
fn responder_sends_subscription_messages() {
    // REQ-CI-PEF-044, -045 (DEC-CI-001 addendum of 2026-10-29): the Responder keeps the PE
    // Capabilities of the Initiator from its Inquiry and can then send it Subscription messages;
    // a Get or Set inquiry still needs the Initiator role.
    let mut d = device();
    deliver(&mut d, &discovery_from(PEER));
    assert_eq!(
        d.pe_request(PEER, PeKind::Subscription, b"{}", Some(0), None)
            .err(),
        Some(DeviceError::NotInquired)
    );
    let mut buf = [0u8; 64];
    let n = zmidi_ci::PeCapabilities {
        simultaneous: 3,
        major: None,
        minor: None,
    }
    .write_inquiry(PEER, d.muid(), Framing::Bare, &mut buf)
    .unwrap();
    let (_, out) = deliver(&mut d, &buf[..n]);
    assert_eq!(out.len(), 1, "the Reply to Inquiry: PE Capabilities");
    assert_eq!(
        d.peer(PEER)
            .and_then(|p| p.pe_capabilities)
            .map(|c| c.simultaneous),
        Some(3)
    );
    let (id, _) = d
        .pe_request(
            PEER,
            PeKind::Subscription,
            br#"{"command":"notify"}"#,
            Some(0),
            None,
        )
        .unwrap();
    d.pe_end(PEER, id, zmidi_ci::Direction::Initiator);
    assert_eq!(
        d.pe_request(PEER, PeKind::GetInquiry, b"{}", Some(0), None)
            .err(),
        Some(DeviceError::NotSupported)
    );
}
