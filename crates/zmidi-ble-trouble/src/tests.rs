//! Host tests of the adapter: the attribute table, the advertising data, the link parameters
//! and the packet paths of the Receiver and the Sender, with the fault paths of T-019 (a failed
//! GATT reply, a full packet pool). The rest of the async connection loop needs a BLE controller
//! and is tested on hardware (T-016, docs/outstanding-tests.md).

extern crate std;

use std::vec::Vec;

use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use trouble_host::prelude::{AttributeTable, Uuid};
use zmidi_ble::zmidi_ump::Ump;

use super::*;

/// A table with the MIDI Service only.
fn table() -> (AttributeTable<'static, NoopRawMutex, 8>, MidiService) {
    let mut table = AttributeTable::new();
    let service = MidiService::add(&mut table);
    (table, service)
}

/// BLE-MIDI v1.0 section 3: the service, the characteristic declaration with the properties
/// 0x1E, the value and its Client Characteristic Configuration descriptor (for notify).
#[test]
fn service_attributes_of_section_3() {
    let (table, s) = table();
    assert_eq!(table.len(), 4);
    let mut buf = [0u8; 32];

    // Primary Service declaration (0x2800) with the MIDI Service UUID.
    assert_eq!(table.uuid(s.service), Some(Uuid::new_short(0x2800)));
    let n = table.read(s.service, 0, &mut buf).unwrap();
    assert_eq!(&buf[..n], &gatt::SERVICE_UUID_LE);

    // Characteristic declaration (0x2803): properties, value handle, UUID.
    let decl = s.service + 1;
    assert_eq!(table.uuid(decl), Some(Uuid::new_short(0x2803)));
    let n = table.read(decl, 0, &mut buf).unwrap();
    let mut expected = std::vec![0x1E];
    expected.extend_from_slice(&s.value_handle().to_le_bytes());
    expected.extend_from_slice(&gatt::CHARACTERISTIC_UUID_LE);
    assert_eq!(&buf[..n], &expected[..]);

    assert_eq!(s.value_handle(), s.service + 2);
    assert_eq!(s.properties(), gatt::CHARACTERISTIC_PROPERTIES);
    assert_eq!(
        table.uuid(s.value_handle()),
        Some(Uuid::from(gatt::CHARACTERISTIC_UUID))
    );
    // CCCD (0x2902) after the value.
    assert_eq!(s.cccd_handle(), Some(s.service + 3));
    assert_eq!(table.uuid(s.service + 3), Some(Uuid::new_short(0x2902)));
}

/// Section 5: a read of the characteristic gives a packet with no payload.
#[test]
fn read_gives_no_payload() {
    let (table, s) = table();
    let mut buf = [0xAAu8; 8];
    assert_eq!(table.read(s.value_handle(), 0, &mut buf), Ok(0));
}

/// The service goes after the services that the firmware adds first (for example GAP).
#[test]
fn service_after_other_attributes() {
    let mut table: AttributeTable<'static, NoopRawMutex, 8> = AttributeTable::new();
    table.add_service(Service::new(0x1800u16)).build();
    let s = MidiService::add(&mut table);
    assert_eq!(s.service, 2);
    assert_eq!(s.value_handle(), 4);
    assert_eq!(table.len(), 5);
}

#[test]
fn advertising_data_has_the_service_uuid() {
    let mut buf = [0u8; 31];
    let n = advertising_data(&mut buf).unwrap();
    let mut expected = std::vec![0x02, 0x01, 0x06, 0x11, 0x07];
    expected.extend_from_slice(&gatt::SERVICE_UUID_LE);
    assert_eq!(&buf[..n], &expected[..]);
    assert_eq!(n, 21);
}

#[test]
fn scan_response_has_the_name() {
    let mut buf = [0u8; 31];
    let n = scan_response_data("zetaohm-midi", &mut buf).unwrap();
    assert_eq!(buf[..2], [13, 0x09]);
    assert_eq!(&buf[2..n], b"zetaohm-midi");
    // A buffer that is too small gives an error, no panic.
    assert!(scan_response_data("zetaohm-midi", &mut [0u8; 4]).is_err());
    assert!(advertising_data(&mut [0u8; 10]).is_err());
}

/// Section 4: an interval of 15 ms or less; the parameters are valid for trouble-host.
#[test]
fn connection_params_of_section_4() {
    assert!(CONNECTION_PARAMS.is_valid());
    assert_eq!(
        CONNECTION_PARAMS.max_connection_interval.as_micros(),
        15_000
    );
    assert_eq!(CONNECTION_PARAMS.min_connection_interval.as_micros(), 7_500);
    assert_eq!(CONNECTION_PARAMS.max_latency, 0);
    assert_eq!(DATA_LENGTH, 251);
    assert_eq!(DATA_LENGTH_TIME_US, (251 + 14) * 8);
}

fn received(
    r: &mut Receiver<20>,
    packet: &[u8],
) -> Result<Vec<Result<Received, ReadError>>, usize> {
    let len = r.take(packet)?;
    let mut out = Vec::new();
    r.decode(len, |x| out.push(x));
    Ok(out)
}

/// The receiver gives what the zmidi-ble reader gives for the same packet.
#[test]
fn receiver_gives_the_reader_output() {
    let (_t, s) = table();
    let mut r = Receiver::<20>::new(s);
    // Section 7: a Note On with a header byte and a timestamp byte.
    let packet = [0x80, 0x80, 0x90, 0x3C, 0x7F];
    let got = received(&mut r, &packet).unwrap();
    let mut expected = Vec::new();
    PacketReader::default().read(&packet, |x| expected.push(x));
    assert_eq!(got, expected);
    assert_eq!(got.len(), 1);
    let ump = got[0].unwrap().ump;
    let mut words = [0u32; 4];
    assert_eq!(ump.write(&mut words), Ok(1));
    assert_eq!(words[0], 0x2090_3C7F);
    let _: Ump = ump;
}

/// A packet longer than the buffer is dropped whole and gives its length.
#[test]
fn receiver_drops_a_packet_longer_than_the_buffer() {
    let (_t, s) = table();
    let mut r = Receiver::<20>::new(s);
    assert_eq!(received(&mut r, &[0x80; 21]), Err(21));
    // The state does not change: the next packet is read.
    assert_eq!(received(&mut r, &[0x80, 0x80, 0xF8]).unwrap().len(), 1);
}

/// A SysEx message open at the end of a packet continues in the next one; reset forgets it.
#[test]
fn receiver_keeps_and_resets_an_open_sysex() {
    let (_t, s) = table();
    let mut r = Receiver::<20>::new(s);
    assert!(received(&mut r, &[0x80, 0x80, 0xF0, 0x01, 0x02])
        .unwrap()
        .is_empty());
    assert!(r.reader().in_sysex());
    r.reset();
    assert!(!r.reader().in_sysex());
}

fn note(i: u32) -> [u32; 1] {
    [0x2090_0040 | (i & 0x7F) << 8]
}

/// The sender gives the packets of the zmidi-ble writer, at most ATT_MTU - 3 and `N` bytes.
#[test]
fn sender_gives_the_writer_packets() {
    let (_t, s) = table();
    let words: Vec<[u32; 1]> = (0..40).map(note).collect();
    let umps = || {
        words
            .iter()
            .map(|w| TimedUmp {
                millis: 5,
                words: &w[..],
            })
            .peekable()
    };
    for att_mtu in [23u16, 30, 247, 512] {
        let mut sender = Sender::<64>::new(s);
        let mut a = umps();
        let mut got = Vec::new();
        loop {
            let p = sender.next_packet(att_mtu, &mut a).unwrap();
            if p.is_empty() {
                break;
            }
            assert!(p.len() <= usize::from(att_mtu) - 3 && p.len() <= 64);
            got.push(p.to_vec());
        }
        let mut writer = PacketWriter::new(Sender::<64>::att_mtu_for(att_mtu)).unwrap();
        let mut b = umps();
        let mut expected = Vec::new();
        let mut buf = [0u8; 64];
        loop {
            let n = writer.write_packet(&mut b, &mut buf).unwrap();
            if n == 0 {
                break;
            }
            expected.push(buf[..n].to_vec());
        }
        assert_eq!(got, expected, "ATT_MTU {att_mtu}");
        assert!(got.len() > 1);
    }
}

#[test]
fn sender_att_mtu_is_limited_by_the_buffer() {
    assert_eq!(Sender::<20>::att_mtu_for(247), 23);
    assert_eq!(Sender::<244>::att_mtu_for(247), 247);
    assert_eq!(Sender::<244>::att_mtu_for(23), 23);
    assert_eq!(Sender::<512>::att_mtu_for(u16::MAX), 515);
}

/// An ATT_MTU smaller than 23 is refused; nothing is taken from the UMPs.
#[test]
fn sender_refuses_a_small_att_mtu() {
    let (_t, s) = table();
    let mut sender = Sender::<20>::new(s);
    let w = note(1);
    let mut umps = [TimedUmp {
        millis: 0,
        words: &w[..],
    }]
    .into_iter()
    .peekable();
    assert_eq!(
        sender.next_packet(22, &mut umps),
        Err(Error::Write(WriteError::MtuTooSmall))
    );
    assert!(umps.peek().is_some());
    assert!(!sender.has_held());
}

/// T-019: a failed GATT reply to a write of a packet still decodes the packet, and the event
/// tells that the reply failed; a packet longer than the buffer gives ReplyFailed too.
#[test]
fn receiver_decodes_a_packet_when_the_reply_fails() {
    use trouble_host::prelude::DefaultPacketPool;
    let (_t, s) = table();
    let mut r = Receiver::<20>::new(s);
    let packet = [0x80, 0x80, 0x90, 0x3C, 0x7F];
    let taken = r.take(&packet);
    let mut got = Vec::new();
    let event: Event<'_, '_, DefaultPacketPool> =
        r.finish(taken, Err(trouble_host::Error::OutOfMemory), |x| {
            got.push(x)
        });
    assert!(matches!(
        event,
        Event::ReplyFailed(trouble_host::Error::OutOfMemory)
    ));
    let mut expected = Vec::new();
    PacketReader::default().read(&packet, |x| expected.push(x));
    assert_eq!(got, expected);
    // The same packet with a reply: the event gives the length.
    let taken = r.take(&packet);
    let event: Event<'_, '_, DefaultPacketPool> = r.finish(taken, Ok(()), |_| {});
    assert!(matches!(event, Event::Packet { len: 5 }));
    // Too long and no reply: the reply error is the event, nothing is decoded.
    let taken = r.take(&[0x80; 21]);
    let mut none = 0;
    let event: Event<'_, '_, DefaultPacketPool> =
        r.finish(taken, Err(trouble_host::Error::OutOfMemory), |_| none += 1);
    assert!(matches!(event, Event::ReplyFailed(_)));
    assert_eq!(none, 0);
}

/// T-019: when the packet pool has no free packet, the sender waits and sends the same packet
/// again; the packets are the same as with no failure, and the report counts the waits.
#[test]
fn sender_retries_when_the_pool_is_full() {
    let (_t, s) = table();
    let words: Vec<[u32; 1]> = (0..40).map(note).collect();
    let umps = || {
        words
            .iter()
            .map(|w| TimedUmp {
                millis: 5,
                words: &w[..],
            })
            .peekable()
    };
    let run = |fail_every: usize| {
        let mut sender = Sender::<64>::new(s);
        let mut sent: Vec<Vec<u8>> = Vec::new();
        let mut calls = 0usize;
        let mut a = umps();
        let report = embassy_futures::block_on(sender.send_with(
            || 30,
            &mut a,
            async |packet: &[u8]| {
                calls += 1;
                if fail_every != 0 && !calls.is_multiple_of(fail_every) {
                    return Err(trouble_host::Error::OutOfMemory);
                }
                sent.push(packet.to_vec());
                Ok(())
            },
        ))
        .unwrap();
        assert!(a.peek().is_none());
        (report, sent)
    };
    let (clean, want) = run(0);
    assert_eq!(clean.retries, 0);
    assert_eq!(clean.packets as usize, want.len());
    // Every packet fails twice before it goes out.
    let (busy, got) = run(3);
    assert_eq!(got, want);
    assert_eq!(busy.packets, clean.packets);
    assert_eq!(busy.retries, 2 * clean.packets);
}

/// T-019: another notification error ends the send; the UMPs not yet taken stay in the
/// iterator.
#[test]
fn sender_stops_on_another_error() {
    let (_t, s) = table();
    let words: Vec<[u32; 1]> = (0..40).map(note).collect();
    let mut a = words
        .iter()
        .map(|w| TimedUmp {
            millis: 5,
            words: &w[..],
        })
        .peekable();
    let mut sender = Sender::<20>::new(s);
    let mut calls = 0;
    let r = embassy_futures::block_on(sender.send_with(
        || 23,
        &mut a,
        async |_: &[u8]| {
            calls += 1;
            if calls == 2 {
                Err(trouble_host::Error::Disconnected)
            } else {
                Ok(())
            }
        },
    ));
    assert_eq!(r, Err(Error::Host(trouble_host::Error::Disconnected)));
    assert_eq!(calls, 2);
    assert!(a.peek().is_some());
}

/// The sender holds the rest of a SysEx message that does not fit the packet; reset forgets
/// it, for a new connection.
#[test]
fn sender_holds_a_cut_message_until_reset() {
    let (_t, s) = table();
    let mut sender = Sender::<20>::new(s);
    // SysEx7 Start, Continue, End: 18 data bytes, more than one packet of 20 bytes.
    let words: [[u32; 2]; 3] = [
        [0x3016_0102, 0x0304_0506],
        [0x3026_0708, 0x090A_0B0C],
        [0x3036_0D0E, 0x0F10_1112],
    ];
    let mut umps = words
        .iter()
        .map(|w| TimedUmp {
            millis: 0,
            words: &w[..],
        })
        .peekable();
    let p = sender.next_packet(23, &mut umps).unwrap();
    assert!(!p.is_empty());
    assert!(sender.has_held() || umps.peek().is_some());
    while umps.peek().is_some() {
        sender.next_packet(23, &mut umps).unwrap();
    }
    assert!(sender.has_held());
    sender.reset();
    assert!(!sender.has_held());
}
