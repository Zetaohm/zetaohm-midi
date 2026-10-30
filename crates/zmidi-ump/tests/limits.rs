//! Values at the limits of their ranges, and the names of field values (M2-104-UM, M2-115-U).
//! Each case is one that a changed comparison or a lost match arm in the crate would fail.

use zmidi_ump::flex_data::{
    ChordAlteration, ChordName, FlexAddress, FlexDestination, FlexTextEvent, FlexTextPackets,
    FlexTextReassembler, FlexTextStatus, MetadataTextStatus, SetChordName, SetMetronome, SetTempo,
};
use zmidi_ump::scale::decode_step;
use zmidi_ump::system::MidiTimeCode;
use zmidi_ump::translate::{to_midi1, Midi1ToMidi2Group};
use zmidi_ump::ump_stream::{
    DeviceIdentityNotification, FunctionBlockDirection, FunctionBlockDiscovery, FunctionBlockInfo,
    FunctionBlockInfoNotification, FunctionBlockMidi1, StreamConfigurationNotification,
    StreamConfigurationRequest, StreamTextKind, UiHint, UmpStreamMessage,
};
use zmidi_ump::utility::{DeltaClockstamp, DeltaClockstampTpq, Noop, UtilityMessage};
use zmidi_ump::{
    Channel, Group, Midi2NoteOn, Midi2ProgramChange, NoteAttribute, SysEx7Event, SysEx7Reassembler,
    SysEx8Event, SysEx8Reassembler, Ump, UmpError, UmpWords,
};

fn g(n: u8) -> Group {
    Group::new(n).unwrap()
}

fn words<M: UmpWords>(m: &M) -> [u32; 4] {
    let mut out = [0u32; 4];
    m.to_words(&mut out).unwrap();
    out
}

#[test]
fn reassemblers_take_exactly_their_capacity() {
    // SysEx7 Complete with 6 bytes.
    let p = [0x3006_0102u32, 0x0304_0506];
    let mut r = SysEx7Reassembler::<6>::new(g(0));
    assert_eq!(r.push(&p), Some(SysEx7Event::Complete(&[1, 2, 3, 4, 5, 6])));
    let mut r = SysEx7Reassembler::<5>::new(g(0));
    assert!(matches!(r.push(&p), Some(SysEx7Event::Overflow { .. })));
    // SysEx8 Complete, Stream ID 0, 3 data bytes (count 4 with the Stream ID).
    let p = [0x5004_00AAu32, 0xBBCC_0000, 0, 0];
    let mut r = SysEx8Reassembler::<3>::new(g(0), 0);
    assert_eq!(r.push(&p), Some(SysEx8Event::Complete(&[0xAA, 0xBB, 0xCC])));
    let mut r = SysEx8Reassembler::<2>::new(g(0), 0);
    assert!(matches!(r.push(&p), Some(SysEx8Event::Overflow { .. })));
    // A Metadata Text Complete with 4 bytes.
    let p = [0xD010_0101u32, 0x6162_6364, 0, 0];
    let mut r = FlexTextReassembler::<4>::new(g(0));
    assert!(matches!(r.push(&p), Some(FlexTextEvent::Complete(t)) if t.as_str() == Some("abcd")));
    let mut r = FlexTextReassembler::<3>::new(g(0));
    assert!(matches!(r.push(&p), Some(FlexTextEvent::Overflow { .. })));
}

#[test]
fn melisma_only_in_a_lyrics_or_ruby_continue_with_0x00() {
    let full = 0x6162_6364;
    let text = |bank_status: u32, cont: [u32; 3]| {
        let mut r = FlexTextReassembler::<64>::new(g(0));
        let s = 0xD010_0000 | bank_status;
        assert_eq!(r.push(&[s | 0x0040_0000, full, full, full]), None);
        assert_eq!(r.push(&[s | 0x0080_0000, cont[0], cont[1], cont[2]]), None);
        match r.push(&[s | 0x00C0_0000, 0x2000_0000, 0, 0]) {
            Some(FlexTextEvent::Complete(t)) => t.has_melisma(),
            e => panic!("{e:?}"),
        }
    };
    // Lyrics: a Continue with all bytes is no melisma; with 0x00 it is.
    assert!(!text(0x0201, [full, full, full]));
    assert!(text(0x0201, [full, 0, 0]));
    // Metadata Text (Bank 0x01): 0x00 in a Continue is no melisma.
    assert!(!text(0x0101, [full, 0, 0]));
}

#[test]
fn recording_date_limits() {
    // 7.5.9.2: hours 24, minutes 59, seconds 60 and zone minutes 59 are the largest.
    let date = |t: &str| {
        FlexTextPackets::try_new(
            g(0),
            FlexDestination::Group,
            FlexTextStatus::Metadata(MetadataTextStatus::RecordingDate),
            t,
        )
        .is_ok()
    };
    assert!(date("2026-10-30T24:00"));
    assert!(date("2026-10-30T23:59:60"));
    assert!(!date("2026-10-30T23:59:61"));
    assert!(!date("2026-10-30T25:00"));
    assert!(!date("2026-10-30T23:60"));
    assert!(date("2026-10-30T12:00+05:59"));
    assert!(!date("2026-10-30T12:00+05:60"));
}

#[test]
fn flex_values_at_their_limits() {
    assert_eq!(FlexAddress::Channel.into_bits(), 0);
    assert_eq!(FlexAddress::Group.into_bits(), 1);
    // A Set Tempo that is not Format Complete is not read.
    assert_eq!(
        SetTempo::from_words(&[0xD050_0000, 50_000_000, 0, 0]).map(|_| ()),
        Err(UmpError::UnexpectedMessage)
    );
    // Set Metronome: each field in its place (Figure 72).
    let m = SetMetronome::try_new(g(0), 24, [2, 3, 1], [4, 5], 6).unwrap();
    assert_eq!(words(&m)[1], 0x1802_0301);
    assert_eq!(words(&m)[2], 0x0405_0000);
    // Chord Name: the Bass Sharps/Flats range, and each Bass Alteration.
    let ok = ChordName {
        tonic_sharps_flats: 0,
        chord_tonic: 1,
        chord_type: 1,
        alterations: [ChordAlteration::default(); 4],
        bass_sharps_flats: 0,
        bass_note: 2,
        bass_chord_type: 1,
        bass_alterations: [ChordAlteration::default(); 2],
    };
    assert!(SetChordName::try_new(g(0), FlexDestination::Group, ok).is_ok());
    let bad_bass = ChordName {
        bass_sharps_flats: 3,
        ..ok
    };
    assert_eq!(
        SetChordName::try_new(g(0), FlexDestination::Group, bad_bass).map(|_| ()),
        Err(UmpError::OutOfRange)
    );
    let mut bad_alt = ok;
    bad_alt.bass_alterations[0] = ChordAlteration { kind: 5, degree: 1 };
    assert_eq!(
        SetChordName::try_new(g(0), FlexDestination::Group, bad_alt).map(|_| ()),
        Err(UmpError::OutOfRange)
    );
}

#[test]
fn stream_values_at_their_limits() {
    // 7-bit and 14-bit fields: 0x7F and 0x3FFF are the largest.
    assert!(DeviceIdentityNotification::try_new([0x7F; 3], 0x3FFF, 0x3FFF, [0x7F; 4]).is_ok());
    let one = DeviceIdentityNotification::try_new([0x41, 0, 0], 0, 0, [0; 4]).unwrap();
    assert_eq!(one.device_manufacturer(), [0x41, 0, 0]);
    // The readers clear the reserved bits and keep the header.
    let r = StreamConfigurationRequest::from_words(&[0xF005_0203, !0, !0, !0]).unwrap();
    assert_eq!(words(&r), [0xF005_0203, 0, 0, 0]);
    let n = StreamConfigurationNotification::from_words(&[0xF006_0203, !0, !0, !0]).unwrap();
    assert_eq!(words(&n), [0xF006_0203, 0, 0, 0]);
    let d = FunctionBlockDiscovery::from_words(&[0xF010_FF03, !0, !0, !0]).unwrap();
    assert_eq!(words(&d), [0xF010_FF03, 0, 0, 0]);
    assert_eq!(UiHint::from_bits(0), UiHint::Unknown);
    // Function Block: number 31, 16 Groups from Group 0; an inactive Input block.
    let info = FunctionBlockInfo {
        active: false,
        number: 31,
        direction: FunctionBlockDirection::Input,
        midi1: FunctionBlockMidi1::NotMidi1,
        ui_hint: UiHint::Receiver,
        first_group: g(0),
        groups_spanned: 16,
        zmidi_ci_version: 0,
        max_sysex8_streams: 0,
    };
    let m = FunctionBlockInfoNotification::try_new(info).unwrap();
    assert_eq!(m.info(), Some(info));
    // First Group 15 is a Group; 0x10 is not.
    let m = FunctionBlockInfoNotification::from_words(&[0xF011_8511, 0x0F01_0000, 0, 0]).unwrap();
    assert_eq!(m.info().map(|i| i.first_group), Some(g(15)));
    let m = FunctionBlockInfoNotification::from_words(&[0xF011_8511, 0x1001_0000, 0, 0]).unwrap();
    assert_eq!(m.info(), None);
    // A Product Instance Id Notification is a text of that kind.
    match UmpStreamMessage::parse(&[0xF004_6162, 0, 0, 0]) {
        Some(UmpStreamMessage::Text(t)) => assert_eq!(t.kind(), StreamTextKind::ProductInstanceId),
        e => panic!("{e:?}"),
    }
}

#[test]
fn channel_voice_and_system_values_at_their_limits() {
    // Attribute Types 1 and 2 keep their data (Table 8).
    let m = Midi2NoteOn::from_words(&[0x4090_3C01, 0x8000_1234]).unwrap();
    assert_eq!(m.attribute(), NoteAttribute::ManufacturerSpecific(0x1234));
    let m = Midi2NoteOn::from_words(&[0x4090_3C02, 0x8000_1234]).unwrap();
    assert_eq!(m.attribute(), NoteAttribute::ProfileSpecific(0x1234));
    // Bank 0x7F/0x7F is a Bank.
    assert!(
        Midi2ProgramChange::try_new(g(0), Channel::new(0).unwrap(), 5, Some((0x7F, 0x7F))).is_ok()
    );
    // MIDI Time Code: type 7 and value 15 are the largest.
    assert!(MidiTimeCode::try_new(g(0), 7, 15).is_ok());
    assert_eq!(
        MidiTimeCode::try_new(g(0), 8, 0).map(|_| ()),
        Err(UmpError::OutOfRange)
    );
    assert_eq!(
        MidiTimeCode::try_new(g(0), 0, 16).map(|_| ()),
        Err(UmpError::OutOfRange)
    );
    // M2-115 4.3: step_count up to 2^bits - 1.
    assert!(decode_step(0, 127, 7).is_ok());
    assert_eq!(decode_step(0, 128, 7), Err(UmpError::OutOfRange));
}

// Ump::parse decodes Message Type 0x2 only with the feature channel-voice1 (DEC-017).
#[cfg(feature = "channel-voice1")]
#[test]
fn translators() {
    // One MIDI 2.0 Note On gives one MIDI 1.0 Note On.
    let on = Ump::parse(&[0x4090_3C00, 0x8000_0000]).unwrap().0;
    let t = to_midi1(&on);
    assert_eq!((t.len(), t.is_empty()), (1, false));
    // reset clears a held RPN: no pending Controller after it.
    let mut x = Midi1ToMidi2Group::new();
    for w in [0x20B0_6500u32, 0x20B0_6400, 0x20B0_0602] {
        let _ = x.translate(&Ump::parse(&[w]).unwrap().0);
    }
    assert!(x.flush(g(0), Channel::new(0).unwrap()).is_some());
    for w in [0x20B0_6500u32, 0x20B0_6400, 0x20B0_0602] {
        let _ = x.translate(&Ump::parse(&[w]).unwrap().0);
    }
    x.reset();
    assert!(x.flush(g(0), Channel::new(0).unwrap()).is_none());
}

#[test]
fn utility_words() {
    // The words of Utility messages through Ump.
    for (u, w) in [
        (UtilityMessage::Noop(Noop::new()), 0x0000_0000u32),
        (
            UtilityMessage::DeltaClockstamp(DeltaClockstamp::try_new(5).unwrap()),
            0x0040_0005,
        ),
        (
            UtilityMessage::DeltaClockstampTpq(DeltaClockstampTpq::try_new(96).unwrap()),
            0x0030_0060,
        ),
    ] {
        let mut out = [0u32; 4];
        assert_eq!(Ump::Utility(u).write(&mut out), Ok(1));
        assert_eq!(out[0], w);
    }
}
