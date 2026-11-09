// Universal System Exclusive, MIDI Time Code, MMC and Notation (REQ-DART-SX-*): M1 v4.2.1, RP-024,
// CA-025, MIDI Tuning, RP-004, RP-013.
import 'dart:convert';

import 'package:test/test.dart';
import 'package:zetaohm_midi/zetaohm_midi.dart';

MtcTime t(int h, int m, int s, int f, int rate) => MtcTime(h, m, s, f, rate);

final invalid = throwsA(isA<InvalidValueException>());
final malformed = throwsA(isA<MalformedException>());

void main() {
  setUpAll(initWeb);

  test('the sysex base: M1 Table VII and VIIa', () {
    final s = parseSysex([0xF0, 0x00, 0x20, 0x6B, 0x01, 0x02, 0xF7]);
    expect(
      [s.kind, s.manufacturer, s.data, s.deviceId],
      [
        SysexClass.manufacturer,
        [0, 0x20, 0x6B],
        [1, 2],
        null,
      ],
    );
    final u = parseSysex([0x7E, 0x10, 0x06, 0x01]);
    expect(
      [u.kind, u.deviceId, u.subId1, u.subId2, u.data],
      [SysexClass.nonRealTime, 0x10, 0x06, 0x01, <int>[]],
    );
    expect(parseSysex([0xF0, 0x41, 0x01, 0xF7]).manufacturer, [0x41]);
    expect(() => parseSysex([0xF0, 0xF7]), malformed);
    expect(
      [sysexAddressed(0x7F, 0x10), sysexAddressed(0x11, 0x10)],
      [true, false],
    );
    expect(manufacturerSysex([0, 0x20, 0x6B], [1, 2]), [
      0xF0,
      0x00,
      0x20,
      0x6B,
      0x01,
      0x02,
      0xF7,
    ]);
    expect(() => manufacturerSysex([0x7E], []), invalid);
    expect(() => manufacturerSysex([0, 1], []), invalid);
    expect(universalSysex(false, 0x10, 0x06, 0x01, []), [
      0xF0,
      0x7E,
      0x10,
      0x06,
      0x01,
      0xF7,
    ]);
    // Table VIIa: ACK (7FH) has no Sub-ID#2; its next byte is the packet number.
    expect(universalSysex(false, 0x10, 0x7F, null, [0x05]), [
      0xF0,
      0x7E,
      0x10,
      0x7F,
      0x05,
      0xF7,
    ]);
  });

  test('inquiry, GM System On and Device Control', () {
    // M1 General Information: the responder answers a request to all devices with its own ID.
    final id = Identity([0, 0x20, 0x6B], 0x0102, 3, [0, 0, 0, 1]);
    final reply = respondIdentity(
      [0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7],
      0x10,
      id,
    )!;
    expect(reply, [
      0xF0,
      0x7E,
      0x10,
      0x06,
      0x02,
      0x00,
      0x20,
      0x6B,
      0x02,
      0x02,
      0x03,
      0x00,
      0,
      0,
      0,
      1,
      0xF7, //
    ]);
    final r = readInquiry(reply)!;
    expect([r.isReply, r.deviceId, r.identity], [true, 0x10, id]);
    expect(respondIdentity(reply, 0x10, id), isNull);
    expect(identityRequest(0x7F), [0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7]);
    expect(identityReply(0x10, id), reply);
    expect(() => Identity([0x41], 0, 0, [0, 0, 0]), invalid);
    // RP-024 4.9.1: GM2 System On.
    final on = [0xF0, 0x7E, 0x7F, 0x09, 0x03, 0xF7];
    expect(readGmSystem(on), (deviceId: 0x7F, command: 3));
    expect(gmSystem(0x7F, 3), on);
    expect(readGmSystem(reply), isNull);
    // M1 Device Control, CA-025.
    final vol = [0xF0, 0x7F, 0x7F, 0x04, 0x01, 0x7F, 0x7F, 0xF7];
    final dc = DeviceControl(DeviceControlKind.masterVolume, 0x7F, 0x3FFF);
    expect(readDeviceControl(vol), dc);
    expect(deviceControlBytes(dc), vol);
    expect(
      () => deviceControlBytes(
        DeviceControl(DeviceControlKind.masterCoarseTuning, 0x7F, 0x80),
      ),
      invalid,
    );
    expect(
      [
        fineTuningOffset(0x2000),
        fineTuningOffset(0),
        coarseTuningSemitones(0x40),
      ],
      [0, -8192, 0],
    );
  });

  test('the MIDI Tuning messages', () {
    // M1 MIDI Tuning: the frequency word 3CH 00H 00H is middle C.
    expect(frequencyWord([0x3C, 0, 0]), FrequencyWord(0x3C, 0));
    expect(frequencyWordBytes(0x3C, 0), [0x3C, 0, 0]);
    expect(() => frequencyWord([0x3C, 0]), invalid);
    final name = ascii.encode('zmidi tuning    ');
    final words = [for (var k = 0; k < 128; k++) FrequencyWord(k, k * 64)];
    final dump = readTuning(tuningKeyDump(0x7F, 2, name, words, 1))!;
    expect(
      [dump.kind, dump.bank, dump.program, dump.checksumOk, dump.name],
      [TuningKind.keyDump, 1, 2, true, name],
    );
    expect(
      [dump.notes.length, dump.notes[100]],
      [128, TuningNote(100, 100, 6400)],
    );
    expect(() => tuningKeyDump(0x7F, 2, name, words.sublist(1)), invalid);
    expect(() => tuningKeyDump(0x7F, 2, name.sublist(1), words), invalid);
    final notes = [TuningNote(60, 0x3C, 0), TuningNote(61, 0x3D, 0x2000)];
    final change = readTuning(tuningNoteChange(true, 0x7F, 5, notes))!;
    expect(
      [change.kind, change.realTime, change.bank, change.notes],
      [TuningKind.noteChange, true, null, notes],
    );
    final offsets = [-64, for (var i = 0; i < 10; i++) 0, 63];
    final so = readTuning(
      tuningScaleOctave(false, 0x7F, 0xFFFF, offsets, false),
    )!;
    expect(
      [so.kind, so.channels, so.offsets, so.twoByte],
      [TuningKind.scaleOctave, 0xFFFF, offsets, false],
    );
    expect(
      () => tuningScaleOctave(false, 0x7F, 0xFFFF, [
        -65,
        for (var i = 0; i < 11; i++) 0,
      ], false),
      invalid,
    );
    final request = tuningDumpRequest(0x7F, 2);
    expect(request, [0xF0, 0x7E, 0x7F, 0x08, 0x00, 0x02, 0xF7]);
    expect(readTuning(request)!.kind, TuningKind.dumpRequest);
  });

  test('MIDI Time Code: RP-004 examples', () {
    // RP-004 p. 4: 01:37:52:16 at 30 frames per second.
    final time = t(1, 37, 52, 16, 3);
    final pieces = [0x00, 0x11, 0x24, 0x33, 0x45, 0x52, 0x61, 0x76];
    expect(mtcQuarterFrames(time), pieces);
    // RP-013 p. 10: 00:22:00:00 drop-frame is skipped; 00:22:00:02 is 00:21:58:22 non-drop.
    expect(mtcTimeIsValid(t(0, 22, 0, 0, 2)), false);
    final df = mtcNextValid(t(0, 22, 0, 0, 2));
    expect(df, t(0, 22, 0, 2, 2));
    expect(mtcFromFrameCount(mtcToFrameCount(df), 3), t(0, 21, 58, 22, 3));
    expect(mtcAddFrames(t(0, 0, 0, 0, 3), -1), t(23, 59, 59, 29, 3));
    // The generator gives the pieces; the reader gives the current time, +2 frames forward.
    final g = MtcGenerator(time);
    final r = MtcReader();
    var last = r.receive(g.nextByte()!);
    for (var i = 1; i < 8; i++) {
      last = r.receive(g.nextByte()!);
    }
    expect([last.raw, last.current], [time, t(1, 37, 52, 18, 3)]);
    final info = r.info;
    expect(
      [info.locked, info.running, info.direction, info.time],
      [true, true, MtcDirection.forward, t(1, 37, 52, 18, 3)],
    );
    g.setRunning(false);
    expect(g.nextByte(), isNull);
    // p. 5: the Full Message stops the reader with its time.
    final full = mtcFull(0x7F, time);
    expect(full, [0xF0, 0x7F, 0x7F, 0x01, 0x01, 0x61, 37, 52, 16, 0xF7]);
    r.fullMessage(readMtcFull(full)!.time);
    expect([r.info.running, r.info.time], [false, time]);
    // p. 6: User Bits.
    final ub = mtcUserBits(0x7F, [1, 2, 3, 4, 5, 6, 7, 8], 2);
    final bits = readMtcUserBits(ub)!;
    expect(
      [bits.deviceId, bits.groups, bits.flags],
      [
        0x7F,
        [1, 2, 3, 4, 5, 6, 7, 8],
        2,
      ],
    );
    expect(() => mtcUserBits(0x7F, [1, 2], 2), invalid);
    g.free();
    r.free();
    expect(() => g.time, throwsA(isA<RejectedException>()));
  });

  test('the MMC transport tier: RP-013 examples', () {
    // RP-013 Example 2A (p. 76): 01:02:03:06 at 30 frames, status form.
    final tc = timeCodeFromBytes([0x61, 0x02, 0x03, 0x26, 0x00]);
    expect(
      [tc.time, tc.statusForm, tc.noTimeCode],
      [t(1, 2, 3, 6, 3), true, false],
    );
    expect(timeCodeBytes(tc), [0x61, 0x02, 0x03, 0x26, 0x00]);
    // p. 32 MOVE: {st} to {ff} gives subframes 0; {ff} to {st} gives no time code.
    final ff = timeCodeToSubframesForm(tc);
    expect([ff.statusForm, ff.subframes], [false, 0]);
    final st = timeCodeToStatusForm(ff, true);
    expect([st.statusForm, st.noTimeCode, st.videoField1], [true, true, true]);
    expect(
      [mmcShortName(0x06), mmcFullName(0x26), mmcFullName(0x20)],
      [0x26, 0x06, null],
    );
    // p. 9: a short time code changes the frames of the last full one.
    expect(timeCodeWithShort(ff, timeCodeShortBytes(ff)), ff);
    // STOP and LOCATE [TARGET] 01:02:03:04.05 (p. 28).
    final cmds = [
      0xF0,
      0x7F,
      0x7F,
      0x06,
      0x01,
      0x44,
      0x06,
      0x01,
      0x61,
      0x02,
      0x03,
      0x04,
      0x05,
      0xF7, //
    ];
    final m = readMmc(cmds)!;
    expect(
      [m.response, m.commands.map((c) => c.kind).toList()],
      [
        false,
        [MmcCommandKind.stop, MmcCommandKind.locateTarget],
      ],
    );
    expect(m.commands[1].target!.subframes, 5);
    expect(mmcCommands(0x7F, m.commands), cmds);
    expect(
      () => readMmc([0xF0, 0x7F, 0x7F, 0x06, 0x44, 0x06, 0x01, 0xF7]),
      malformed,
    );
    expect(
      () => mmcCommands(0x7F, [
        MmcCommand(MmcCommandKind.locateField, register: 0x07),
      ]),
      invalid,
    );
    // A response: SELECTED TIME CODE {st} from device 01H, not negative; 7FH is not a source.
    final f = MmcField(MmcFieldKind.timeCode, field: 0x01, code: tc);
    final resp = mmcResponses(0x01, [f]);
    expect(resp, [
      0xF0,
      0x7F,
      0x01,
      0x07,
      0x01,
      0x61,
      0x02,
      0x03,
      0x26,
      0x00,
      0xF7,
    ]);
    final rr = readMmc(resp)!;
    expect(
      [rr.response, rr.fields[0].field, rr.fields[0].data],
      [
        true,
        0x01,
        [0x61, 2, 3, 0x26, 0],
      ],
    );
    expect(() => mmcResponses(0x7F, [f]), invalid);
    final neg = TimeCode(t(1, 0, 0, 0, 3), negative: true, statusForm: true);
    expect(
      () =>
          mmcFields([MmcField(MmcFieldKind.timeCode, field: 0x01, code: neg)]),
      invalid,
    );
    expect(readMmcFields(mmcFields([f])).map((x) => x.field), [0x01]);
  });

  test('Notation Information: M1 examples', () {
    // M1 p. 54: 7FH 3FH is the running state with no known bar number; 01H 40H is the first count-in bar.
    final unknown = [0xF0, 0x7F, 0x7F, 0x03, 0x01, 0x7F, 0x3F, 0xF7];
    expect(readBarMarker(unknown), BarMarker(0x7F, barUnknown));
    expect(barMarkerBytes(BarMarker(0x7F, 8191)), unknown);
    expect(barMarkerBytes(BarMarker(0x7F, -8191)).sublist(5, 7), [0x01, 0x40]);
    expect(barMarkerBytes(BarMarker(0, barNotRunning)).sublist(5, 7), [
      0x00,
      0x40,
    ]);
    expect(() => BarMarker(0x7F, 8192), invalid);
    // M1 p. 55: 3/8 + 2/8 in one bar, Delayed: ln nn dd cc bb nn dd.
    final compound = [
      0xF0, 0x7F, 0x10, 0x03, 0x42, 0x06, 0x03, 0x03, 0x0C, 0x08, 0x02, //
      0x03, 0xF7,
    ];
    final sig = TimeSignature(
      0x10,
      3,
      3,
      12,
      8,
      additional: [2, 3],
      delayed: true,
    );
    expect(readTimeSignature(compound), sig);
    expect(sig.pairs, [(2, 3)]);
    expect(timeSignatureBytes(sig), compound);
    expect(timeSignatureBytes(TimeSignature(0x7F, 4, 2, 24, 8)), [
      0xF0, 0x7F, 0x7F, 0x03, 0x02, 0x04, 0x04, 0x02, 0x18, 0x08, 0xF7, //
    ]);
    expect(readBarMarker(compound), isNull);
    expect(readTimeSignature(unknown), isNull);
    expect(
      () =>
          timeSignatureBytes(TimeSignature(0x7F, 4, 2, 24, 8, additional: [2])),
      invalid,
    );
    expect(
      () => readTimeSignature([
        0xF0,
        0x7F,
        0x7F,
        0x03,
        0x02,
        0x05,
        4,
        2,
        24,
        8,
        2,
        0xF7,
      ]),
      malformed,
    );
  });
}
