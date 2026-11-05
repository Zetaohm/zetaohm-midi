// MIDI 1.0 typed messages and receiver states (REQ-DART-MIDI1-*): M1 v4.2.1, RP-015, RP-018.
import 'package:test/test.dart';
import 'package:zetaohm_midi/zetaohm_midi.dart';

Midi1Message cc(int c, int v) => Midi1Message(0xB0, c, v);

void main() {
  setUpAll(initWeb);

  test('the reader gives typed messages and whole System Exclusive messages', () {
    // M1 p. 5 running status; p. 30 a Timing Clock inside a System Exclusive message.
    final r = Midi1Reader();
    final ms = r.feed([
      0x90, 0x3C, 0x40, 0x3E, 0x40, 0xF0, 0x7E, 0x7F, 0xF8, 0x06, 0x01, 0xF7, //
    ]);
    expect(ms.map((m) => m.kind), [
      Midi1Kind.channelVoice,
      Midi1Kind.channelVoice,
      Midi1Kind.system,
      Midi1Kind.sysex,
    ]);
    expect(ms[1], Midi1Message(0x90, 0x3E, 0x40));
    expect(ms[2], Midi1Message(0xF8));
    expect(ms[3].data, [0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7]);
    // A System Exclusive message longer than the buffer gives its length only.
    final small = Midi1Reader(sysexCapacity: 4);
    final long = small.feed([0xF0, 1, 2, 3, 4, 0xF7]).single;
    expect(
      [long.kind, long.length, long.data.length],
      [Midi1Kind.sysexTooLong, 6, 0],
    );
    // AMB-M1-13: the reset of the program clears running status.
    r.push(0x90);
    r.reset();
    expect(r.feed([0x3C, 0x40]), isEmpty);
    r.free();
  });

  test('the writer with running status and System Exclusive', () {
    final w = Midi1Writer(runningStatus: true);
    expect(w.write(Midi1Message(0x90, 0x3C, 0x40)), [0x90, 0x3C, 0x40]);
    expect(w.write(Midi1Message(0x90, 0x3E, 0x40)), [0x3E, 0x40]);
    w.refreshStatus();
    expect(w.write(Midi1Message(0x90, 0x3E, 0x40)), [0x90, 0x3E, 0x40]);
    const sysex = [0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7];
    expect(w.write(Midi1Message.sysex(sysex)), sysex);
    expect(w.write(Midi1Message(0xF2, 0x10, 0x02)), [0xF2, 0x10, 0x02]);
    w.free();
  });

  test('the mode state: modes and actions', () {
    // M1 pp. 20 to 26: Omni Off selects Mode 3; All Notes Off acts on the instrument.
    final s = ModeState();
    expect([s.mode, s.local], [1, true]);
    final a = s.receive(cc(0x7C, 0));
    expect(
      [a.kind, a.changed, a.alternate],
      [ModeActionKind.modeChange, true, false],
    );
    expect(s.mode, 3);
    final off = s.receive(cc(0x7B, 0));
    expect([off.kind, off.channel], [ModeActionKind.allNotesOff, null]);
    expect(
      s.receive(Midi1Message(0x91, 0x3C, 0x40)).kind,
      ModeActionKind.ignore,
    );
    expect(
      s.receive(Midi1Message(0x90, 0x3C, 0x40)).kind,
      ModeActionKind.voice,
    );
    // Mode 4 with M = 4: a voice number for each channel from the Basic Channel.
    s.setMono(4);
    expect([s.mode, s.m], [4, 4]);
    expect(s.receive(Midi1Message(0x92, 0x3C, 0x40)).voice, 2);
    s.systemReset();
    expect([s.mode, s.local, s.m], [1, true, 4]);
    s
      ..basicChannel = 5
      ..local = false;
    expect([s.basicChannel, s.local], [5, false]);
    // A mode that is not implemented gives the alternate mode (p. 23).
    final t = ModeState(implemented: [1, 3]);
    t.receive(cc(0x7C, 0));
    final b = t.receive(cc(0x7E, 0));
    expect(
      [t.requestedMode, t.mode, t.isAlternate, b.alternate],
      [4, 1, true, true],
    );
    s.free();
    t.free();
  });

  test('the controller state: 14-bit pairs, Portamento, bank and RP-015', () {
    final s = ControllerState();
    final e = s.receive(cc(0x01, 0x12))!;
    expect(
      [e.kind, e.controller, e.value],
      [ControllerEventKind.controller14, 1, 0x12 << 7],
    );
    expect(s.receive(cc(0x21, 0x34))!.value, 0x12 << 7 | 0x34);
    // p. 17, Example 2: B0 54 3C, 90 40 40 glides from note 60; a later Note On has no source.
    expect(s.receive(cc(0x54, 0x3C)), isNull);
    final p = s.receive(Midi1Message(0x90, 0x40, 0x40))!;
    expect([p.kind, p.source], [ControllerEventKind.portamento, 0x3C]);
    expect(s.receive(Midi1Message(0x90, 0x40, 0x40)), isNull);
    // pp. 13, 14: the bank and program writer and the bank state.
    final ms = bankProgramMessages(0, 0x0102, 0x11);
    expect(ms.map((m) => [m.status, m.data1, m.data2]), [
      [0xB0, 0, 2],
      [0xB0, 0x20, 2],
      [0xC0, 0x11, 0],
    ]);
    final events = ms.map(s.receive).toList();
    expect(events.take(2), [null, null]);
    expect([events[2]!.program, events[2]!.bank], [0x11, 0x0102]);
    // RP-015: Reset All Controllers keeps the bank; Expression 127, Balance kept (AMB-RP-015-04).
    expect(
      s.receive(cc(0x79, 0))!.kind,
      ControllerEventKind.resetAllControllers,
    );
    expect(s.bank, 0x0102);
    expect(
      [resetValue(11), resetValue(64), resetValue(8), resetValue(7)],
      [127, 0, null, null],
    );
    expect(
      controller14Messages(
        5,
        1,
        0x1234,
      ).map((m) => [m.status, m.data1, m.data2]),
      [
        [0xB5, 1, 0x24],
        [0xB5, 0x21, 0x34],
      ],
    );
    s.free();
  });

  test(
    'the parameter receiver: the RP-018 example and Tuning Program Select',
    () {
      // RP-018 example: B0 65 00, B0 64 00, B0 60 00, B0 60 00, B0 65 7F, B0 64 7F.
      final s = ParameterState();
      expect([s.rpnReception, s.nrpnReception], [true, false]);
      var pbs = 2 << 7;
      for (final (c, v) in [
        (0x65, 0),
        (0x64, 0),
        (0x60, 0),
        (0x60, 0),
        (0x65, 0x7F),
        (0x64, 0x7F),
      ]) {
        final e = s.receive(cc(c, v));
        if (e != null) {
          expect(
            [e.change, e.parameter, e.value],
            [ParameterChange.increment, Parameter(0, 0), null],
          );
          pbs = e.apply(pbs);
        }
      }
      expect(pbs, 2 << 7 | 2);
      expect(s.selection, isNull);
      // p. 50: Bn 64 03 65 00 06 tt selects Tuning Program tt.
      expect(s.receive(cc(0x64, 3)), isNull);
      expect(s.receive(cc(0x65, 0)), isNull);
      final t = s.receive(cc(0x06, 5))!;
      expect(
        [t.change, t.value, t.parameter],
        [ParameterChange.dataEntryMsb, 5, Parameter(0, 3)],
      );
      // RP-015: Reset All Controllers selects no parameter.
      expect(s.receive(cc(0x79, 0)), isNull);
      expect(s.selection, isNull);
      s.nrpnReception = true;
      expect([s.rpnReception, s.nrpnReception], [true, true]);
      final ms = valueMessages(3, Parameter(0, 0), 2 << 7 | 50);
      expect(ms.map((m) => [m.data1, m.data2]), [
        [0x65, 0],
        [0x64, 0],
        [0x06, 2],
        [0x26, 50],
      ]);
      expect(
        selectMessages(0, Parameter(0x7F, 0x7F)).map((m) => [m.data1, m.data2]),
        [
          [0x65, 0x7F],
          [0x64, 0x7F],
        ],
      );
      expect(
        changeMessage(0, ParameterChange.decrement),
        Midi1Message(0xB0, 0x61, 0),
      );
      s.free();
    },
  );

  test('MIDI 1.0 bad values', () {
    final invalid = throwsA(isA<InvalidValueException>());
    expect(() => Midi1Message(0x90, 0x80), invalid);
    expect(() => Midi1Message(0xF0), invalid);
    expect(() => Midi1Message.sysex([0xF0, 0x80, 0xF7]), invalid);
    expect(() => ModeState(basicChannel: 16), invalid);
    expect(() => ModeState(implemented: [5]), invalid);
    expect(() => ControllerState().receive(Midi1Message(0xF8)), invalid);
    expect(() => controller14Messages(0, 32, 0), invalid);
    expect(() => Parameter(0x80, 0), invalid);
    final tooLong = Midi1Reader(sysexCapacity: 0).feed([0xF0, 0xF7]).single;
    expect(() => Midi1Writer().write(tooLong), invalid);
  });
}
