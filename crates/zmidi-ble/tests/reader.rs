//! Packet reader tests: the figures of BLE-MIDI v1.0 sections 7 and 8 and the readings of
//! DEC-BLE-001.

use zmidi_ble::zmidi_ump::{Group, StreamProtocol};
use zmidi_ble::{PacketReader, ReadError};

/// The timestamp and words of each UMP, or the error.
type Out = Vec<Result<(u16, Vec<u32>), ReadError>>;

fn read(r: &mut PacketReader, packet: &[u8]) -> Out {
    let mut out = Vec::new();
    r.read(packet, |x| {
        out.push(x.map(|rx| {
            let mut w = [0u32; 4];
            let n = rx.ump.write(&mut w).unwrap();
            (rx.timestamp.millis(), w[..n].to_vec())
        }))
    });
    out
}

fn ok(t: u16, words: &[u32]) -> Result<(u16, Vec<u32>), ReadError> {
    Ok((t, words.to_vec()))
}

/// Header and timestamp bytes of the time `t` (13 bits).
fn hdr(t: u16) -> u8 {
    0x80 | (t >> 7) as u8
}
fn ts(t: u16) -> u8 {
    0x80 | (t & 0x7F) as u8
}

const NOTE_ON: u32 = 0x2090_3C64;
const NOTE_ON_2: u32 = 0x2090_4064;

#[test]
fn figure_one_and_two_messages() {
    let mut r = PacketReader::default();
    let t = 0x0A55;
    assert_eq!(
        read(&mut r, &[hdr(t), ts(t), 0x90, 0x3C, 0x64]),
        vec![ok(t, &[NOTE_ON])]
    );
    assert_eq!(
        read(
            &mut r,
            &[hdr(t), ts(t), 0x90, 0x3C, 0x64, ts(t + 2), 0x80, 0x3C, 0x00]
        ),
        vec![ok(t, &[NOTE_ON]), ok(t + 2, &[0x2080_3C00])]
    );
}

#[test]
fn figure_running_status_with_and_without_timestamp() {
    let mut r = PacketReader::default();
    // "2 MIDI Messages with Running Status": no timestamp byte before the second message.
    assert_eq!(
        read(&mut r, &[hdr(5), ts(5), 0x90, 0x3C, 0x64, 0x40, 0x64]),
        vec![ok(5, &[NOTE_ON]), ok(5, &[NOTE_ON_2])]
    );
    // "Multiple MIDI Messages, mixed type": full, full, running, running with a new
    // timestamp, full.
    let p = [
        hdr(5),
        ts(5),
        0x90,
        0x3C,
        0x64,
        ts(6),
        0xB0,
        0x07,
        0x40,
        0x0A,
        0x10,
        ts(7),
        0x0B,
        0x20,
        ts(8),
        0x90,
        0x3C,
        0x00,
    ];
    assert_eq!(
        read(&mut r, &p),
        vec![
            ok(5, &[NOTE_ON]),
            ok(6, &[0x20B0_0740]),
            ok(6, &[0x20B0_0A10]),
            ok(7, &[0x20B0_0B20]),
            ok(8, &[0x2090_3C00]),
        ]
    );
    // Program Change (2 bytes) with running status.
    assert_eq!(
        read(&mut r, &[hdr(1), ts(1), 0xC3, 0x05, 0x06]),
        vec![ok(1, &[0x20C3_0500]), ok(1, &[0x20C3_0600])]
    );
}

#[test]
fn system_messages_do_not_cancel_running_status() {
    let mut r = PacketReader::default();
    // BLE-MIDI v1.0 section 7, Running Status rule 3 and its figure: a System Common or System
    // Real-Time message between two Running Status messages keeps Running Status. Here: Song
    // Select, then Timing Clock.
    let p = [
        hdr(3),
        ts(3),
        0x90,
        0x3C,
        0x64,
        0x40,
        0x64,
        ts(4),
        0xF3,
        0x01,
        ts(5),
        0x3C,
        0x00,
        ts(6),
        0xF8,
        ts(7),
        0x40,
        0x00,
    ];
    assert_eq!(
        read(&mut r, &p),
        vec![
            ok(3, &[NOTE_ON]),
            ok(3, &[NOTE_ON_2]),
            ok(4, &[0x10F3_0100]),
            ok(5, &[0x2090_3C00]),
            ok(6, &[0x10F8_0000]),
            ok(7, &[0x2090_4000]),
        ]
    );
    // A SysEx message does not cancel it either (AMB-BLE-MIDI-01).
    let p = [
        hdr(1),
        ts(1),
        0x90,
        0x3C,
        0x64,
        ts(1),
        0xF0,
        0x7D,
        ts(1),
        0xF7,
        ts(2),
        0x40,
        0x64,
    ];
    assert_eq!(
        read(&mut r, &p),
        vec![
            ok(1, &[NOTE_ON]),
            ok(1, &[0x3001_7D00, 0]),
            ok(2, &[NOTE_ON_2])
        ]
    );
}

#[test]
fn end_of_packet_cancels_running_status() {
    let mut r = PacketReader::default();
    read(&mut r, &[hdr(1), ts(1), 0x90, 0x3C, 0x64]);
    assert_eq!(
        read(&mut r, &[hdr(2), ts(2), 0x40, 0x64]),
        vec![
            Err(ReadError::NoRunningStatus),
            Err(ReadError::NoRunningStatus)
        ]
    );
}

#[test]
fn realtime_between_messages_figure() {
    // "MIDI Stream with System Real-Time Message in the middle of another MIDI Message": the
    // BLE form has the real-time message after the whole message.
    let mut r = PacketReader::default();
    assert_eq!(
        read(&mut r, &[hdr(9), ts(9), 0x90, 0x3C, 0x64, ts(9), 0xFA]),
        vec![ok(9, &[NOTE_ON]), ok(9, &[0x10FA_0000])]
    );
}

#[test]
fn sysex_in_one_two_and_three_packets() {
    let mut r = PacketReader::default();
    // Start & End in 1 packet.
    assert_eq!(
        read(
            &mut r,
            &[hdr(1), ts(1), 0xF0, 0x01, 0x02, 0x03, ts(1), 0xF7]
        ),
        vec![ok(1, &[0x3003_0102, 0x0300_0000])]
    );
    // Split across 2 packets: the continuation packet has data right after the header.
    let a = [hdr(1), ts(1), 0xF0, 1, 2, 3, 4, 5, 6, 7];
    let b = [hdr(2), 8, 9, ts(2), 0xF7];
    assert_eq!(read(&mut r, &a), vec![ok(1, &[0x3016_0102, 0x0304_0506])]);
    assert!(r.in_sysex());
    assert_eq!(read(&mut r, &b), vec![ok(2, &[0x3033_0708, 0x0900_0000])]);
    assert!(!r.in_sysex());
    // Split across 3 packets; the UMP that ends in a continuation packet gets the most recent
    // timestamp of the SysEx (AMB-BLE-MIDI-07).
    let a = [hdr(1), ts(1), 0xF0, 1, 2, 3, 4];
    let b = [hdr(3), 5, 6, 7, 8];
    let c = [hdr(4), 9, ts(4), 0xF7];
    assert_eq!(read(&mut r, &a), vec![]);
    assert_eq!(read(&mut r, &b), vec![ok(1, &[0x3016_0102, 0x0304_0506])]);
    assert_eq!(read(&mut r, &c), vec![ok(4, &[0x3033_0708, 0x0900_0000])]);
}

#[test]
fn realtime_inside_sysex_keeps_it_open() {
    let mut r = PacketReader::default();
    let a = [hdr(1), ts(1), 0xF0, 1, 2, ts(2), 0xF8, 3];
    let b = [hdr(3), ts(3), 0xF8, 4, ts(3), 0xF7];
    assert_eq!(read(&mut r, &a), vec![ok(2, &[0x10F8_0000])]);
    assert!(r.in_sysex());
    assert_eq!(
        read(&mut r, &b),
        vec![ok(3, &[0x10F8_0000]), ok(3, &[0x3004_0102, 0x0304_0000])]
    );
}

#[test]
fn other_status_ends_an_open_sysex() {
    let mut r = PacketReader::default();
    assert_eq!(
        read(
            &mut r,
            &[hdr(1), ts(1), 0xF0, 1, 2, ts(1), 0x90, 0x3C, 0x64]
        ),
        vec![ok(1, &[0x3002_0102, 0]), ok(1, &[NOTE_ON])]
    );
    assert!(!r.in_sysex());
}

#[test]
fn timestamp_low_wrap_increments_high() {
    let mut r = PacketReader::default();
    // timestampLow 0x7E then 0x02: timestampHigh + 1.
    let p = [0x85, 0xFE, 0xF8, 0x82, 0xF8, 0x83, 0xF8];
    assert_eq!(
        read(&mut r, &p),
        vec![
            ok(5 * 128 + 0x7E, &[0x10F8_0000]),
            ok(6 * 128 + 2, &[0x10F8_0000]),
            ok(6 * 128 + 3, &[0x10F8_0000]),
        ]
    );
    // timestampHigh 63 wraps to 0 (13 bits).
    assert_eq!(
        read(&mut r, &[0xBF, 0xFF, 0xF8, 0x80, 0xF8]),
        vec![ok(8191, &[0x10F8_0000]), ok(0, &[0x10F8_0000])]
    );
}

#[test]
fn header_bit_6_is_ignored() {
    let mut r = PacketReader::default();
    assert_eq!(
        read(&mut r, &[0xC1, 0x82, 0xF8]),
        vec![ok(130, &[0x10F8_0000])]
    );
}

#[test]
fn malformed_parts_are_dropped_and_reported() {
    let mut r = PacketReader::default();
    // No header byte: the packet is dropped whole.
    assert_eq!(
        read(&mut r, &[0x00, 0x80, 0xF8]),
        vec![Err(ReadError::NoHeader)]
    );
    // Empty packet and header only: nothing.
    assert_eq!(read(&mut r, &[]), vec![]);
    assert_eq!(read(&mut r, &[0x80]), vec![]);
    // Data with no running status, then a good message.
    assert_eq!(
        read(&mut r, &[0x80, 0x10, 0x80, 0xF8]),
        vec![Err(ReadError::NoRunningStatus), ok(0, &[0x10F8_0000])]
    );
    // A message cut by a timestamp byte, and one cut by the end of the packet.
    assert_eq!(
        read(
            &mut r,
            &[0x80, 0x80, 0x90, 0x3C, 0x81, 0xF8, 0x82, 0x90, 0x3C]
        ),
        vec![
            Err(ReadError::Incomplete),
            ok(1, &[0x10F8_0000]),
            Err(ReadError::Incomplete)
        ]
    );
    // A timestamp byte at the end.
    assert_eq!(
        read(&mut r, &[0x80, 0x80, 0xF8, 0x81]),
        vec![ok(0, &[0x10F8_0000]), Err(ReadError::LoneTimestamp)]
    );
    // Undefined status bytes go to zmidi-io, which ignores them (AMB-BLE-MIDI-11).
    assert_eq!(read(&mut r, &[0x80, 0x80, 0xF4, 0x80, 0xFD]), vec![]);
}

#[test]
fn group_and_protocol_of_the_caller() {
    let mut r = PacketReader::new(Group::new(3).unwrap(), StreamProtocol::Midi2);
    let got = read(&mut r, &[hdr(1), ts(1), 0x90, 0x3C, 0x64]);
    let Ok((t, w)) = &got[0] else { panic!() };
    assert_eq!(*t, 1);
    assert_eq!(w[0] >> 24, 0x43, "Message Type 0x4 on Group 3");
    let mut r = PacketReader::new(Group::new(3).unwrap(), StreamProtocol::Midi1);
    assert_eq!(
        read(&mut r, &[hdr(1), ts(1), 0x90, 0x3C, 0x64]),
        vec![ok(1, &[0x2390_3C64])]
    );
}

#[test]
fn reset_forgets_an_open_sysex() {
    let mut r = PacketReader::default();
    read(&mut r, &[hdr(1), ts(1), 0xF0, 1, 2]);
    r.reset();
    assert!(!r.in_sysex());
    assert_eq!(
        read(&mut r, &[hdr(2), 3, 4]),
        vec![Err(ReadError::NoRunningStatus); 2]
    );
}

#[test]
fn a_timestamp_byte_0x80_cuts_a_message() {
    // Time 0x100: the timestamp byte is 0x80. It cuts the Note On after one data byte.
    let mut r = PacketReader::default();
    let t = 0x0100;
    assert_eq!(ts(t), 0x80);
    assert_eq!(
        read(&mut r, &[hdr(t), ts(t), 0x90, 0x3C, ts(t), 0xF8]),
        vec![Err(ReadError::Incomplete), ok(t, &[0x10F8_0000])]
    );
}
