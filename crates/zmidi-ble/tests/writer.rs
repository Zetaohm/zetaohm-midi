//! Packet writer tests: the packet forms of BLE-MIDI v1.0 sections 7 to 9 and the readings of
//! DEC-BLE-001.

use zmidi_ble::{PacketWriter, TimedUmp, WriteError, MIN_ATT_MTU};

const NOTE_ON: [u32; 1] = [0x2090_3C64];
const NOTE_ON_2: [u32; 1] = [0x2090_4064];
const NOTE_OFF: [u32; 1] = [0x2080_3C00];
const CLOCK: [u32; 1] = [0x10F8_0000];
const SONG_SELECT: [u32; 1] = [0x10F3_0100];

/// All packets for `umps` (time, words).
fn packets(w: &mut PacketWriter, umps: &[(u64, &[u32])]) -> Vec<Vec<u8>> {
    let mut it = umps
        .iter()
        .map(|&(millis, words)| TimedUmp { millis, words })
        .peekable();
    let mut out = Vec::new();
    let mut buf = vec![0u8; w.packet_len()];
    loop {
        let n = w.write_packet(&mut it, &mut buf).unwrap();
        if n == 0 {
            break;
        }
        assert!(n <= w.packet_len());
        out.push(buf[..n].to_vec());
    }
    assert!(it.next().is_none());
    out
}

fn hdr(t: u16) -> u8 {
    0x80 | (t >> 7) as u8
}
fn ts(t: u16) -> u8 {
    0x80 | (t & 0x7F) as u8
}

#[test]
fn one_and_two_full_messages() {
    let mut w = PacketWriter::default();
    assert_eq!(
        packets(&mut w, &[(0x0A55, &NOTE_ON)]),
        vec![vec![hdr(0x0A55), ts(0x0A55), 0x90, 0x3C, 0x64]]
    );
    assert_eq!(
        packets(&mut w, &[(0x0A55, &NOTE_ON), (0x0A57, &NOTE_OFF)]),
        vec![vec![
            hdr(0x0A55),
            ts(0x0A55),
            0x90,
            0x3C,
            0x64,
            ts(0x0A57),
            0x80,
            0x3C,
            0x00
        ]]
    );
}

#[test]
fn running_status_with_and_without_timestamp() {
    let mut w = PacketWriter::default();
    // Same time: no status byte and no timestamp byte.
    assert_eq!(
        packets(&mut w, &[(5, &NOTE_ON), (5, &NOTE_ON_2)]),
        vec![vec![hdr(5), ts(5), 0x90, 0x3C, 0x64, 0x40, 0x64]]
    );
    // Later time: a timestamp byte and no status byte.
    assert_eq!(
        packets(&mut w, &[(5, &NOTE_ON), (6, &NOTE_ON_2)]),
        vec![vec![hdr(5), ts(5), 0x90, 0x3C, 0x64, ts(6), 0x40, 0x64]]
    );
    // Off: full messages only.
    w.set_running_status(false);
    assert_eq!(
        packets(&mut w, &[(7, &NOTE_ON), (7, &NOTE_ON_2)]),
        vec![vec![
            hdr(7),
            ts(7),
            0x90,
            0x3C,
            0x64,
            ts(7),
            0x90,
            0x40,
            0x64
        ]]
    );
}

#[test]
fn no_running_status_across_a_system_message_or_a_packet() {
    let mut w = PacketWriter::default();
    assert_eq!(
        packets(
            &mut w,
            &[
                (1, &NOTE_ON),
                (1, &SONG_SELECT),
                (1, &NOTE_ON_2),
                (1, &CLOCK),
                (1, &NOTE_ON)
            ]
        ),
        vec![vec![
            hdr(1),
            ts(1),
            0x90,
            0x3C,
            0x64,
            ts(1),
            0xF3,
            0x01,
            ts(1),
            0x90,
            0x40,
            0x64,
            ts(1),
            0xF8,
            ts(1),
            0x90,
            0x3C,
            0x64
        ]]
    );
    // Five Note On messages: the fifth does not fit in 20 bytes and starts the next packet
    // with its status byte.
    let five: Vec<(u64, &[u32])> = vec![(1, &NOTE_ON); 10];
    let p = packets(&mut w, &five);
    assert_eq!(p.len(), 2);
    assert_eq!(p[0].len(), 19, "header, timestamp, 3 + 7 * 2 bytes");
    assert_eq!(&p[1][..5], &[hdr(1), ts(1), 0x90, 0x3C, 0x64]);
}

#[test]
fn realtime_has_its_own_timestamp() {
    let mut w = PacketWriter::default();
    assert_eq!(
        packets(&mut w, &[(9, &NOTE_ON), (9, &[0x10FA_0000])]),
        vec![vec![hdr(9), ts(9), 0x90, 0x3C, 0x64, ts(9), 0xFA]]
    );
}

#[test]
fn sysex_in_one_packet() {
    let mut w = PacketWriter::default();
    assert_eq!(
        packets(&mut w, &[(1, &[0x3003_0102, 0x0300_0000])]),
        vec![vec![hdr(1), ts(1), 0xF0, 1, 2, 3, ts(1), 0xF7]]
    );
}

#[test]
fn sysex_in_continuation_packets() {
    let mut w = PacketWriter::default();
    // 30 data bytes in 5 SysEx7 packets: Start, 3 Continue, End.
    let umps: Vec<[u32; 2]> = (0..5u32)
        .map(|i| {
            let status = match i {
                0 => 1,
                4 => 3,
                _ => 2,
            };
            let d: Vec<u32> = (0..6).map(|k| i * 6 + k).collect();
            [
                0x3000_0000 | status << 20 | 6 << 16 | d[0] << 8 | d[1],
                d[2] << 24 | d[3] << 16 | d[4] << 8 | d[5],
            ]
        })
        .collect();
    let list: Vec<(u64, &[u32])> = umps.iter().map(|u| (100, &u[..])).collect();
    let p = packets(&mut w, &list);
    // Packet 1: header, timestamp, F0 and 17 data bytes; packet 2: header and 13 data bytes,
    // timestamp, F7.
    let mut first = vec![hdr(100), ts(100), 0xF0];
    first.extend(0..17u8);
    let mut second = vec![hdr(100)];
    second.extend(17..30u8);
    second.extend([ts(100), 0xF7]);
    assert_eq!(p, vec![first, second]);
}

#[test]
fn sysex_end_alone_in_the_last_packet() {
    let mut w = PacketWriter::default();
    // F0 and 18 data bytes fill the packet; F7 goes in the next one with a timestamp byte.
    let start = [0x3016_0001, 0x0203_0405];
    let cont = [0x3026_0607, 0x0809_0A0B];
    let end = [0x3036_0C0D, 0x0E0F_1011];
    let p = packets(&mut w, &[(4, &start), (4, &cont), (4, &end)]);
    let mut first = vec![hdr(4), ts(4), 0xF0];
    first.extend(0..17u8);
    assert_eq!(p, vec![first, vec![hdr(4), 17, ts(4), 0xF7]]);
}

#[test]
fn realtime_inside_sysex_then_data_continues() {
    let mut w = PacketWriter::default();
    let p = packets(
        &mut w,
        &[
            (1, &[0x3016_0102, 0x0304_0506]),
            (2, &CLOCK),
            (2, &[0x3032_0708, 0]),
        ],
    );
    assert_eq!(
        p,
        vec![vec![
            hdr(1),
            ts(1),
            0xF0,
            1,
            2,
            3,
            4,
            5,
            6,
            ts(2),
            0xF8,
            7,
            8,
            ts(2),
            0xF7
        ]]
    );
}

#[test]
fn other_message_ends_an_open_sysex_and_its_rest_is_dropped() {
    let mut w = PacketWriter::default();
    let p = packets(
        &mut w,
        &[
            (1, &[0x3016_0102, 0x0304_0506]),
            (1, &NOTE_ON),
            (1, &[0x3032_0708, 0]),
            (1, &CLOCK),
        ],
    );
    assert_eq!(
        p,
        vec![vec![
            hdr(1),
            ts(1),
            0xF0,
            1,
            2,
            3,
            4,
            5,
            6,
            ts(1),
            0xF7,
            ts(1),
            0x90,
            0x3C,
            0x64,
            ts(1),
            0xF8
        ]]
    );
}

#[test]
fn timestamps_do_not_decrease_and_wrap_at_8192() {
    let mut w = PacketWriter::default();
    assert_eq!(
        packets(&mut w, &[(8191, &CLOCK), (8190, &CLOCK)]),
        vec![vec![0xBF, 0xFF, 0xF8, 0xFF, 0xF8]]
    );
    let mut w = PacketWriter::default();
    // 8192 + 5 is 5 on the air.
    assert_eq!(
        packets(&mut w, &[(8197, &CLOCK)]),
        vec![vec![0x80, 0x85, 0xF8]]
    );
}

#[test]
fn packet_ends_before_a_time_128_ms_after_its_first() {
    let mut w = PacketWriter::default();
    assert_eq!(
        packets(&mut w, &[(10, &CLOCK), (137, &CLOCK), (138, &CLOCK)]),
        vec![
            vec![hdr(10), ts(10), 0xF8, ts(137), 0xF8],
            vec![hdr(138), ts(138), 0xF8]
        ]
    );
    // One wrap of timestampLow inside the span: 120 then 130.
    assert_eq!(
        packets(&mut w, &[(1000 + 120, &CLOCK), (1000 + 130, &CLOCK)]).len(),
        1
    );
}

#[test]
fn header_high_is_the_high_of_the_first_timestamp() {
    let mut w = PacketWriter::default();
    // 18 SysEx data bytes at 127: the continuation packet has the header of 127. A Note On at
    // 128 ends the SysEx; its F7 is the first timestamp byte of the packet and has another
    // timestampHigh, so it starts a new packet.
    let start = [0x3016_0001, 0x0203_0405];
    let cont = [0x3026_0607, 0x0809_0A0B];
    let cont2 = [0x3026_0C0D, 0x0E0F_1011];
    let p = packets(
        &mut w,
        &[(127, &start), (127, &cont), (127, &cont2), (128, &NOTE_ON)],
    );
    assert_eq!(p.len(), 3);
    assert_eq!(p[1], vec![hdr(127), 17]);
    assert_eq!(
        p[2],
        vec![hdr(128), ts(128), 0xF7, ts(128), 0x90, 0x3C, 0x64]
    );
    // With a timestamp byte of the header's timestampHigh first, a later wrap stays in the
    // packet: the receiver sees timestampLow go down.
    let end = [0x3036_0C0D, 0x0E0F_1011];
    let mut w = PacketWriter::default();
    let p = packets(
        &mut w,
        &[(127, &start), (127, &cont), (127, &end), (128, &CLOCK)],
    );
    assert_eq!(p.len(), 2);
    assert_eq!(p[1], vec![hdr(127), 17, ts(127), 0xF7, ts(128), 0xF8]);
}

#[test]
fn umps_with_no_midi1_form_and_the_group() {
    let mut w = PacketWriter::default();
    // Utility NOOP, UMP Stream, and a Note On on Group 5 (the Group is ignored).
    let p = packets(
        &mut w,
        &[
            (1, &[0x0000_0000]),
            (1, &[0xF000_0000, 0, 0, 0]),
            (2, &[0x2590_3C64]),
        ],
    );
    assert_eq!(p, vec![vec![hdr(2), ts(2), 0x90, 0x3C, 0x64]]);
    // The undefined status bytes F4H, F5H, F9H and FDH are never written.
    for status in [0xF4u32, 0xF5, 0xF9, 0xFD] {
        let ump = [0x1000_0000 | status << 16];
        assert!(packets(&mut w, &[(3, &ump)]).is_empty(), "{status:#X}");
    }
    // Nothing to send: 0 and no packet.
    assert!(packets(&mut w, &[]).is_empty());
}

#[test]
fn att_mtu_and_buffer() {
    assert_eq!(PacketWriter::new(22).err(), Some(WriteError::MtuTooSmall));
    let mut w = PacketWriter::new(MIN_ATT_MTU).unwrap();
    assert_eq!(w.packet_len(), 20);
    w.set_att_mtu(185).unwrap();
    assert_eq!(w.packet_len(), 182);
    assert_eq!(w.set_att_mtu(0), Err(WriteError::MtuTooSmall));
    let mut it = [TimedUmp {
        millis: 0,
        words: &CLOCK,
    }]
    .into_iter()
    .peekable();
    let mut small = [0u8; 100];
    assert_eq!(
        w.write_packet(&mut it, &mut small),
        Err(WriteError::BufferTooSmall)
    );
    assert!(it.peek().is_some(), "the UMP stays in the iterator");
}

#[test]
fn held_bytes_go_first_in_the_next_packet() {
    let mut w = PacketWriter::default();
    let start = [0x3016_0001, 0x0203_0405];
    let cont = [0x3026_0607, 0x0809_0A0B];
    let end = [0x3036_0C0D, 0x0E0F_1011];
    let umps = [
        TimedUmp {
            millis: 0,
            words: &start,
        },
        TimedUmp {
            millis: 0,
            words: &cont,
        },
        TimedUmp {
            millis: 0,
            words: &end,
        },
    ];
    let mut it = umps.into_iter().peekable();
    let mut buf = [0u8; 20];
    assert_eq!(w.write_packet(&mut it, &mut buf), Ok(20));
    assert!(w.has_held());
    assert!(it.next().is_none(), "all three UMPs are written or held");
    assert_eq!(w.write_packet(&mut it, &mut buf), Ok(4));
    assert!(!w.has_held());
    w.reset();
    assert_eq!(w.write_packet(&mut it, &mut buf), Ok(0));
}

#[test]
fn units_that_fill_the_packet_exactly_stay_in_it() {
    // A packet of 20 bytes (ATT_MTU 23): header, 4 messages of 4 bytes, then 3 bytes.
    const PROGRAM: [u32; 1] = [0x20C0_0500];
    let mut w = PacketWriter::default();
    assert_eq!(w.packet_len(), 20);
    let p = packets(
        &mut w,
        &[
            (0, &NOTE_ON),
            (1, &NOTE_OFF),
            (2, &NOTE_ON),
            (3, &NOTE_OFF),
            (4, &PROGRAM),
        ],
    );
    assert_eq!(p.len(), 1);
    assert_eq!(p[0].len(), 20);
    assert_eq!(&p[0][17..], &[ts(4), 0xC0, 0x05]);
    // The last unit by running status: a timestamp byte and 2 data bytes.
    let mut w = PacketWriter::default();
    let p = packets(
        &mut w,
        &[
            (0, &NOTE_ON),
            (1, &NOTE_OFF),
            (2, &NOTE_ON),
            (3, &NOTE_OFF),
            (4, &NOTE_OFF),
        ],
    );
    assert_eq!(p.len(), 1);
    assert_eq!(&p[0][13..], &[ts(3), 0x80, 0x3C, 0x00, ts(4), 0x3C, 0x00]);
}

#[test]
fn reset_forgets_the_held_bytes_and_the_open_sysex_only() {
    let mut w = PacketWriter::new(30).unwrap();
    w.set_running_status(false);
    // A SysEx longer than one packet: bytes are held for the next packet.
    let start = [0x3016_0102u32, 0x0304_0506];
    let cont = [0x3026_0708u32, 0x090A_0B0C];
    let mut it = [
        TimedUmp {
            millis: 0,
            words: &start,
        },
        TimedUmp {
            millis: 0,
            words: &cont,
        },
        TimedUmp {
            millis: 0,
            words: &cont,
        },
        TimedUmp {
            millis: 0,
            words: &cont,
        },
        TimedUmp {
            millis: 0,
            words: &cont,
        },
    ]
    .into_iter()
    .peekable();
    let mut buf = [0u8; 27];
    assert_eq!(w.write_packet(&mut it, &mut buf), Ok(27));
    assert!(w.has_held());
    w.reset();
    assert!(!w.has_held());
    // The ATT_MTU and the running status setting stay.
    assert_eq!(w.packet_len(), 27);
    assert_eq!(
        packets(&mut w, &[(7, &NOTE_ON), (7, &NOTE_ON_2)]),
        vec![vec![
            hdr(7),
            ts(7),
            0x90,
            0x3C,
            0x64,
            ts(7),
            0x90,
            0x40,
            0x64
        ]]
    );
    // No SysEx is open: Continue data is dropped.
    assert!(packets(&mut w, &[(8, &cont)]).is_empty());
}
