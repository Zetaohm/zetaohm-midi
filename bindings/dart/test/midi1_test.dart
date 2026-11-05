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

  test('the sync receiver with the M1 examples', () {
    // M1 p. 28: SPP 4 (24 clocks), Continue and three Timing Clocks give the 27th clock.
    final s = SyncReceiver();
    final l = s.receive(Midi1Message(0xF2, 4, 0))!;
    expect([l.kind, l.position], [SyncEventKind.locate, 24]);
    expect(s.receive(Midi1Message(0xFB))!.kind, SyncEventKind.resume);
    expect(s.run, SyncRun.waiting);
    final clocks = [for (var i = 0; i < 3; i++) s.receive(Midi1Message(0xF8))!];
    expect(clocks.map((c) => [c.kind, c.position]), [
      [SyncEventKind.clock, 24],
      [SyncEventKind.clock, 25],
      [SyncEventKind.clock, 26],
    ]);
    expect([s.position, s.run], [27, SyncRun.playing]);
    // p. 32: Start while playing is ignored; the front panel has the same transitions.
    expect(s.command(SyncCommand.start), isNull);
    expect(s.command(SyncCommand.stop)!.kind, SyncEventKind.stop);
    expect(s.receive(Midi1Message(0xF8)), isNull);
    expect(s.command(SyncCommand.resume)!.kind, SyncEventKind.resume);
    // p. 29: Song Select out of MIDI sync mode is ignored; the System Reset action stops at 0.
    s.syncMode = false;
    expect(s.receive(Midi1Message(0xF3, 5)), isNull);
    s.systemReset();
    expect(
      [s.position, s.run, s.song, s.syncMode],
      [0, SyncRun.stopped, 0, false],
    );
    expect(
      () => s.receive(Midi1Message(0x90, 60, 64)),
      throwsA(isA<InvalidValueException>()),
    );
    s.free();
  });

  test('the Active Sensing monitor and sender', () {
    // M1 p. 32: no timeout before FEH; over 330 ms with no byte gives one timeout; send FEH after
    // 270 ms with no byte.
    final m = ActiveSensingMonitor();
    expect([m.timeoutUs, m.poll(10000000)], [activeSensingTimeoutUs, false]);
    m.receiveByte(0xFE, 0);
    m.receiveByte(0x90, 300000);
    expect([m.isArmed, m.poll(630000)], [true, false]);
    expect([m.poll(630001), m.isArmed], [true, false]);
    expect(m.poll(2000000), isFalse);
    m.receive(true, 3000000);
    expect(m.isArmed, isTrue);
    final s = ActiveSensingSender();
    expect(s.intervalUs, activeSensingIntervalUs);
    expect([s.poll(269999), s.poll(270000)], [false, true]);
    s.sent(400000);
    expect([s.poll(669999), s.poll(670000)], [false, true]);
    expect([activeSensingTimeoutUs, activeSensingIntervalUs], [330000, 270000]);
    m.free();
    s.free();
  });

  test('the MPE receiver with the M1-100 examples', () {
    // Appendix B.5 with the corrected data byte 0x0B (AMB-M1-100-04): Upper 5 to 15, Lower keeps
    // 2 to 4; the reset mask is the Upper Zone after the MCM.
    final params = [for (var i = 0; i < 16; i++) ParameterState()];
    final r = MpeReceiver();
    List<MpeEvent> feed(List<List<int>> msgs) => [
      for (final b in msgs)
        () {
          final m = Midi1Message(b[0], b[1], b.length > 2 ? b[2] : 0);
          return r.receive(m, params[b[0] & 0x0F].receive(m));
        }(),
    ];
    MpeEvent mcm(int ch, int mm) => feed([
      [0xB0 | ch, 0x65, 0],
      [0xB0 | ch, 0x64, 6],
      [0xB0 | ch, 0x06, mm],
    ])[2];
    expect(
      [mcm(0, 7).kind, mcm(15, 0x0B).reset],
      [MpeEventKind.layout, 0xFFF0],
    );
    expect(r.layout, MpeLayout(3, 11));
    expect(r.layout.role(4), (MpeRole.member, MpeZone.upper));
    expect(
      [
        r.sensitivity(MpeZone.upper, true),
        r.sensitivity(MpeZone.upper, false),
        r.mode(MpeZone.upper),
      ],
      [const MpeSensitivity(2), const MpeSensitivity(48), 3],
    );
    // 2.2.5: RPN 0 on one Member Channel sets the Member sensitivity of the Zone.
    final e = feed([
      [0xB5, 0x65, 0],
      [0xB5, 0x64, 0],
      [0xB5, 0x06, 24],
    ])[2];
    expect(
      [e.kind, e.zone, e.manager],
      [MpeEventKind.sensitivity, MpeZone.upper, false],
    );
    // 0x41 << 7 | 0x2B at 24 semitones is a quarter tone (AMB-M1-100-08).
    expect(
      feed([
        [0xE6, 0x2B, 0x41],
      ]).single.kind,
      MpeEventKind.zone,
    );
    expect([r.pitchBend(6), r.pressure(6), r.brightness(6)], [8363, 0, 64]);
    expect((r.pitchOffset(6)! - 5000).abs(), lessThanOrEqualTo(50));
    // Appendix E: Polyphonic Key Pressure on a Member Channel is ignored; All Sound Off on the
    // Manager Channel is given (AMB-M1-100-12).
    expect(
      feed([
        [0xA6, 60, 64],
      ]).single.kind,
      MpeEventKind.ignored,
    );
    final aso = feed([
      [0xBF, 0x78, 0],
    ]).single;
    expect([aso.kind, aso.manager], [MpeEventKind.zone, true]);
    expect(
      mpeClass(MpeMessageKind.allSoundOff, true, false),
      MpeClass.prohibited,
    );
    expect(
      mpeClass(MpeMessageKind.programChange, false, true, true),
      MpeClass.optional,
    );
    // B.6: the Upper Zone with 14 Member Channels leaves the Lower Zone none.
    mcm(15, 0x0E);
    expect(
      [r.layout, r.layout.role(0)],
      [MpeLayout(0, 14), (MpeRole.noZone, null)],
    );
    expect(r.setLayout(MpeLayout.defaultLayout), 0xFFFF);
    expect(r.pitchOffset(0), 0);
    r.free();
    for (final p in params) {
      p.free();
    }
  });

  test(
    'the MPE sender helpers with Appendix C, Table 4 and the note allocator',
    () {
      List<List<int>> bytes(List<Midi1Message> ms) => [
        for (final m in ms) [m.status, m.data1, m.data2],
      ];
      // C.4 and C.6: +7 semitones at 48 is 9387; 16383 at 2 is 2 semitones.
      expect(pitchUnitsPerSemitone, 10000);
      expect(
        [
          pitchBendValue(70000, 48),
          pitchBendValue(20000, 2),
          pitchBendValue(-500000, 48),
        ],
        [9387, 16383, 0],
      );
      expect(pitchOffset(16383, 2), 20000);
      expect((pitchOffset(9387, 48) - 70000).abs(), lessThanOrEqualTo(100));
      // Table 4 with the corrected Pitch Bend (AMB-M1-100-08): a quarter tone up at 48 semitones.
      expect(
        bytes(noteMessages(2, 0x3C, 0x38, pitchBendValue(5000, 48), 0x40, 0)),
        [
          [0xE2, 0x55, 0x40],
          [0xB2, 0x4A, 0x40],
          [0xD2, 0x00, 0],
          [0x92, 0x3C, 0x38],
        ],
      );
      // Appendix B.1, the Mode message with M = 7, RPN 0 of a Zone.
      expect(bytes(mcmMessages(MpeZone.lower, 15)), [
        [0xB0, 0x65, 0],
        [0xB0, 0x64, 6],
        [0xB0, 0x06, 0x0F],
      ]);
      final two = MpeLayout(7, 7);
      expect(bytes([modeMessage(two, MpeZone.upper, true)]), [
        [0xB8, 0x7E, 7],
      ]);
      final zs = zoneSensitivityMessages(two, MpeZone.upper, 2, 48);
      expect(
        [zs.length, zs[2], zs[5]],
        [24, Midi1Message(0xBF, 0x06, 2), Midi1Message(0xB8, 0x06, 48)],
      );
      expect(sensitivityMessages(3, 12)[2], Midi1Message(0xB3, 0x06, 12));
      expect(two.withMcm(MpeZone.upper, 14), MpeLayout(0, 14));
      expect(
        [
          two.lowestMember(MpeZone.upper),
          MpeLayout(7).lowestMember(MpeZone.upper),
        ],
        [8, null],
      );
      // A.3: an unoccupied channel first, then the oldest last Note Off; the same-note option.
      final small = MpeLayout(3);
      final a = MpeNoteAllocator();
      expect(
        [
          for (final n in [60, 62, 64]) a.noteOn(small, MpeZone.lower, n),
        ],
        [1, 2, 3],
      );
      a.noteOff(2);
      a.noteOff(1);
      expect(
        [
          a.noteOn(small, MpeZone.lower, 70),
          a.activeNotes(2),
          a.noteOn(small, MpeZone.upper, 70),
        ],
        [2, 1, null],
      );
      final s = MpeNoteAllocator(sameNote: true);
      s.noteOn(small, MpeZone.lower, 60);
      s.noteOn(small, MpeZone.lower, 62);
      s.noteOff(1);
      s.noteOff(2);
      expect(s.noteOn(small, MpeZone.lower, 60), 1);
      s.reset();
      expect([s.sameNote, s.activeNotes(1)], [true, 0]);
      final invalid = throwsA(isA<InvalidValueException>());
      expect(() => MpeLayout(7, 8), invalid);
      expect(() => mcmMessages(MpeZone.lower, 16), invalid);
      expect(() => noteMessages(2, 60, 0), invalid);
      expect(() => modeMessage(small, MpeZone.upper, true), invalid);
      expect(() => pitchOffset(16384, 48), invalid);
      a.free();
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
