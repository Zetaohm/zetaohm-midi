// MIDI 1.0 typed messages and receiver states (REQ-TS-MIDI1-*): M1 v4.2.1, RP-015, RP-018, RP-024.
import { beforeAll, describe, expect, test } from "vitest";

import {
  ActiveSensingMonitor,
  ActiveSensingSender,
  ControllerState,
  GmReceiver,
  InvalidValueError,
  Midi1Reader,
  Midi1Writer,
  MPE_LAYOUT_DEFAULT,
  ModeState,
  MpeNoteAllocator,
  MpeReceiver,
  PITCH_UNITS_PER_SEMITONE,
  ParameterState,
  SyncReceiver,
  applyParameter,
  bankProgramMessages,
  changeMessage,
  controller14Messages,
  gm2ControllerDefault,
  init,
  mcmMessages,
  midi1Message,
  modeMessage,
  modulationDepthRange,
  mpeClass,
  mpeLayout,
  mpeLayoutWithMcm,
  mpeLowestMember,
  mpeRole,
  noteMessages,
  pitchBendValue,
  pitchOffset,
  resetValue,
  rhythmIgnoresController,
  rhythmIgnoresMessage,
  rhythmIgnoresParameter,
  selectMessages,
  sensitivityMessages,
  sysexMessage,
  valueMessages,
  zoneSensitivityMessages,
} from "./index.js";
import type { Midi1Message, MpeEvent } from "./index.js";

beforeAll(async () => {
  await init();
});

const b = (...x: number[]): Uint8Array => new Uint8Array(x);
const cc = (c: number, v: number) => midi1Message(0xb0, c, v);

describe("midi1", () => {
  test("the reader gives typed messages and whole System Exclusive messages", () => {
    // M1 p. 5 running status; p. 30 a Timing Clock inside a System Exclusive message.
    const r = new Midi1Reader();
    const ms = r.feed(b(0x90, 0x3c, 0x40, 0x3e, 0x40, 0xf0, 0x7e, 0x7f, 0xf8, 0x06, 0x01, 0xf7));
    expect(ms.map((m) => m.kind)).toEqual(["channelVoice", "channelVoice", "system", "sysex"]);
    expect(ms[1]).toEqual(midi1Message(0x90, 0x3e, 0x40));
    expect(ms[2]).toEqual(midi1Message(0xf8));
    expect(ms[3]?.data).toEqual(b(0xf0, 0x7e, 0x7f, 0x06, 0x01, 0xf7));
    // A System Exclusive message longer than the buffer gives its length only.
    const small = new Midi1Reader(4);
    expect(small.feed(b(0xf0, 1, 2, 3, 4, 0xf7))).toEqual([
      { kind: "sysexTooLong", status: 0xf0, data1: 0, data2: 0, data: b(), length: 6 },
    ]);
    // AMB-M1-13: the reset of the program clears running status.
    r.push(0x90);
    r.reset();
    expect(r.feed(b(0x3c, 0x40))).toEqual([]);
    r.free();
  });

  test("the writer with running status and System Exclusive", () => {
    const w = new Midi1Writer(true);
    expect(w.write(midi1Message(0x90, 0x3c, 0x40))).toEqual(b(0x90, 0x3c, 0x40));
    expect(w.write(midi1Message(0x90, 0x3e, 0x40))).toEqual(b(0x3e, 0x40));
    w.refreshStatus();
    expect(w.write(midi1Message(0x90, 0x3e, 0x40))).toEqual(b(0x90, 0x3e, 0x40));
    const sysex = b(0xf0, 0x7e, 0x7f, 0x06, 0x01, 0xf7);
    expect(w.write(sysexMessage(sysex))).toEqual(sysex);
    expect(w.write(midi1Message(0xf2, 0x10, 0x02))).toEqual(b(0xf2, 0x10, 0x02));
  });

  test("the mode state: modes and actions", () => {
    // M1 pp. 20 to 26: Omni Off selects Mode 3; All Notes Off acts on the instrument.
    const s = new ModeState({ basicChannel: 0 });
    expect([s.mode, s.local]).toEqual([1, true]);
    expect(s.receive(cc(0x7c, 0))).toMatchObject({ kind: "modeChange", changed: true, alternate: false });
    expect(s.mode).toBe(3);
    expect(s.receive(cc(0x7b, 0))).toMatchObject({ kind: "allNotesOff", channel: null });
    expect(s.receive(midi1Message(0x91, 0x3c, 0x40)).kind).toBe("ignore");
    expect(s.receive(midi1Message(0x90, 0x3c, 0x40)).kind).toBe("voice");
    // Mode 4 with M = 4: a voice number for each channel from the Basic Channel.
    s.setMono(4);
    expect([s.mode, s.m]).toEqual([4, 4]);
    expect(s.receive(midi1Message(0x92, 0x3c, 0x40)).voice).toBe(2);
    s.systemReset();
    expect([s.mode, s.local, s.m]).toEqual([1, true, 4]);
    s.basicChannel = 5;
    s.local = false;
    expect([s.basicChannel, s.local]).toEqual([5, false]);
    // A mode that is not implemented gives the alternate mode (p. 23).
    const t = new ModeState({ implemented: [1, 3] });
    t.receive(cc(0x7c, 0));
    const a = t.receive(cc(0x7e, 0));
    expect([t.requestedMode, t.mode, t.isAlternate, a.alternate]).toEqual([4, 1, true, true]);
  });

  test("the controller state: 14-bit pairs, Portamento, bank and RP-015", () => {
    const s = new ControllerState();
    expect(s.receive(cc(0x01, 0x12))).toMatchObject({ kind: "controller14", controller: 1, value: 0x12 << 7 });
    expect(s.receive(cc(0x21, 0x34))?.value).toBe((0x12 << 7) | 0x34);
    // p. 17, Example 2: B0 54 3C, 90 40 40 glides from note 60; a later Note On has no source.
    expect(s.receive(cc(0x54, 0x3c))).toBeNull();
    expect(s.receive(midi1Message(0x90, 0x40, 0x40))).toMatchObject({ kind: "portamento", source: 0x3c });
    expect(s.receive(midi1Message(0x90, 0x40, 0x40))).toBeNull();
    // pp. 13, 14: the bank and program writer and the bank state.
    const ms = bankProgramMessages(0, 0x0102, 0x11);
    expect(ms.map((m) => [m.status, m.data1, m.data2])).toEqual([
      [0xb0, 0, 2],
      [0xb0, 0x20, 2],
      [0xc0, 0x11, 0],
    ]);
    const events = ms.map((m) => s.receive(m));
    expect(events.slice(0, 2)).toEqual([null, null]);
    expect(events[2]).toMatchObject({ kind: "program", program: 0x11, bank: 0x0102 });
    // RP-015: Reset All Controllers keeps the bank; Expression 127, Balance kept (AMB-RP-015-04).
    expect(s.receive(cc(0x79, 0))?.kind).toBe("resetAllControllers");
    expect(s.bank).toBe(0x0102);
    expect([resetValue(11), resetValue(64), resetValue(8), resetValue(7)]).toEqual([127, 0, null, null]);
    expect(controller14Messages(5, 1, 0x1234).map((m) => [m.status, m.data1, m.data2])).toEqual([
      [0xb5, 1, 0x24],
      [0xb5, 0x21, 0x34],
    ]);
    s.free();
  });

  test("the parameter receiver: the RP-018 example and Tuning Program Select", () => {
    // RP-018 example: B0 65 00, B0 64 00, B0 60 00, B0 60 00, B0 65 7F, B0 64 7F.
    const s = new ParameterState();
    expect([s.rpnReception, s.nrpnReception]).toEqual([true, false]);
    let pbs = 2 << 7;
    for (const [c, v] of [
      [0x65, 0],
      [0x64, 0],
      [0x60, 0],
      [0x60, 0],
      [0x65, 0x7f],
      [0x64, 0x7f],
    ] as const) {
      const e = s.receive(cc(c, v));
      if (e !== null) {
        expect(e).toEqual({ parameter: { msb: 0, lsb: 0, registered: true }, change: "increment", value: null });
        pbs = applyParameter(e, pbs);
      }
    }
    expect(pbs).toBe((2 << 7) | 2);
    expect(s.selection).toBeNull();
    // p. 50: Bn 64 03 65 00 06 tt selects Tuning Program tt.
    expect(s.receive(cc(0x64, 3))).toBeNull();
    expect(s.receive(cc(0x65, 0))).toBeNull();
    expect(s.receive(cc(0x06, 5))).toEqual({
      parameter: { msb: 0, lsb: 3, registered: true },
      change: "dataEntryMsb",
      value: 5,
    });
    // RP-015: Reset All Controllers selects no parameter.
    expect(s.receive(cc(0x79, 0))).toBeNull();
    expect(s.selection).toBeNull();
    s.nrpnReception = true;
    expect([s.rpnReception, s.nrpnReception]).toEqual([true, true]);
    const ms = valueMessages(3, { msb: 0, lsb: 0, registered: true }, (2 << 7) | 50);
    expect(ms.map((m) => [m.data1, m.data2])).toEqual([
      [0x65, 0],
      [0x64, 0],
      [0x06, 2],
      [0x26, 50],
    ]);
    const nul = selectMessages(0, { msb: 0x7f, lsb: 0x7f, registered: true });
    expect(nul.map((m) => [m.data1, m.data2])).toEqual([
      [0x65, 0x7f],
      [0x64, 0x7f],
    ]);
    expect(changeMessage(0, "decrement")).toEqual(midi1Message(0xb0, 0x61, 0));
    s.free();
  });

  test("bad values", () => {
    expect(() => midi1Message(0x90, 0x80)).toThrow(InvalidValueError);
    expect(() => midi1Message(0xf0)).toThrow(InvalidValueError);
    expect(() => sysexMessage(b(0xf0, 0x80, 0xf7))).toThrow(InvalidValueError);
    expect(() => new ModeState({ basicChannel: 16 })).toThrow(InvalidValueError);
    expect(() => new ModeState({ implemented: [5] })).toThrow(InvalidValueError);
    expect(() => new ControllerState().receive(midi1Message(0xf8))).toThrow(InvalidValueError);
    expect(() => controller14Messages(0, 32, 0)).toThrow(InvalidValueError);
    expect(() => selectMessages(0, { msb: 0x80, lsb: 0, registered: true })).toThrow(InvalidValueError);
    expect(() => changeMessage(0, "jump" as "increment")).toThrow(InvalidValueError);
    expect(() => new Midi1Writer().write(new Midi1Reader(0).feed(b(0xf0, 0xf7))[0]!)).toThrow(InvalidValueError);
  });

  test("the sync receiver: M1 examples", () => {
    // M1 p. 28: SPP 4 (24 clocks), Continue and three Timing Clocks give the 27th clock.
    const s = new SyncReceiver();
    expect(s.receive(midi1Message(0xf2, 4, 0))).toEqual({ kind: "locate", position: 24, song: null });
    expect(s.receive(midi1Message(0xfb))?.kind).toBe("continue");
    expect(s.run).toBe("waiting");
    const clocks = [0, 1, 2].map(() => s.receive(midi1Message(0xf8))?.position);
    expect(clocks).toEqual([24, 25, 26]);
    expect([s.position, s.run]).toEqual([27, "playing"]);
    // p. 32: Start while playing is ignored; the front panel has the same transitions.
    expect(s.command("start")).toBeNull();
    expect(s.command("stop")?.kind).toBe("stop");
    expect(s.receive(midi1Message(0xf8))).toBeNull();
    // p. 29: Song Select out of MIDI sync mode is ignored; the System Reset action stops at 0.
    s.syncMode = false;
    expect(s.receive(midi1Message(0xf3, 5))).toBeNull();
    s.systemReset();
    expect([s.position, s.run, s.song, s.syncMode]).toEqual([0, "stopped", 0, false]);
    expect(() => s.command("pause" as "stop")).toThrow(InvalidValueError);
    expect(() => s.receive(midi1Message(0x90, 60, 64))).toThrow(InvalidValueError);
    s.free();
  });

  test("the Active Sensing monitor and sender", () => {
    // M1 p. 32: no timeout before FEH; over 330 ms with no byte gives one timeout; send FEH after
    // 270 ms with no byte.
    const m = new ActiveSensingMonitor();
    expect(m.timeoutUs).toBe(330_000);
    expect(m.poll(10_000_000)).toBe(false);
    m.receiveByte(0xfe, 0);
    m.receiveByte(0x90, 300_000);
    expect([m.isArmed, m.poll(630_000)]).toEqual([true, false]);
    expect(m.poll(630_001)).toBe(true);
    expect([m.isArmed, m.poll(2_000_000)]).toEqual([false, false]);
    m.receive(true, 3_000_000);
    expect(m.isArmed).toBe(true);
    const s = new ActiveSensingSender();
    expect(s.intervalUs).toBe(270_000);
    expect([s.poll(269_999), s.poll(270_000)]).toEqual([false, true]);
    s.sent(400_000);
    expect([s.poll(669_999), s.poll(670_000)]).toEqual([false, true]);
    m.free();
    s.free();
  });

  test("the MPE receiver: M1-100 examples", () => {
    // Appendix B.5 with the corrected data byte 0x0B (AMB-M1-100-04): Upper 5 to 15, Lower keeps
    // 2 to 4; the reset mask is the Upper Zone after the MCM.
    const params = Array.from({ length: 16 }, () => new ParameterState());
    const r = new MpeReceiver();
    const feed = (...msgs: number[][]): MpeEvent[] =>
      msgs.map(([st = 0, d1 = 0, d2 = 0]) => {
        const m = midi1Message(st, d1, d2);
        return r.receive(m, params[st & 0x0f]?.receive(m) ?? null);
      });
    const mcm = (ch: number, mm: number): MpeEvent | undefined => feed([0xb0 | ch, 0x65, 0], [0xb0 | ch, 0x64, 6], [0xb0 | ch, 0x06, mm])[2];
    expect(mcm(0, 7)).toEqual({ kind: "layout", zone: null, manager: false, reset: 0x00ff });
    expect(mcm(15, 0x0b)).toEqual({ kind: "layout", zone: null, manager: false, reset: 0xfff0 });
    expect(r.layout).toEqual({ lower: 3, upper: 11 });
    expect(mpeRole(r.layout, 4)).toEqual({ role: "member", zone: "upper" });
    expect([r.sensitivity("upper", true), r.sensitivity("upper", false), r.mode("upper")]).toEqual([
      { semitones: 2, cents: 0 },
      { semitones: 48, cents: 0 },
      3,
    ]);
    // 2.2.5: RPN 0 on one Member Channel sets the Member sensitivity of the Zone.
    expect(feed([0xb5, 0x65, 0], [0xb5, 0x64, 0], [0xb5, 0x06, 24])[2]).toEqual({ kind: "sensitivity", zone: "upper", manager: false, reset: 0 });
    // 0x41 << 7 | 0x2B at 24 semitones is a quarter tone (AMB-M1-100-08).
    expect(feed([0xe6, 0x2b, 0x41])[0]?.kind).toBe("zone");
    expect([r.pitchBend(6), r.pressure(6), r.brightness(6)]).toEqual([8363, 0, 64]);
    expect(Math.abs((r.pitchOffset(6) ?? 0) - 5_000)).toBeLessThanOrEqual(50);
    // Appendix E: Polyphonic Key Pressure on a Member Channel is ignored; All Sound Off on the
    // Manager Channel is given (AMB-M1-100-12).
    expect(feed([0xa6, 60, 64])[0]?.kind).toBe("ignored");
    expect(feed([0xbf, 0x78, 0])[0]).toEqual({ kind: "zone", zone: "upper", manager: true, reset: 0 });
    expect(mpeClass("allSoundOff", true, false)).toBe("prohibited");
    expect(mpeClass("programChange", false, true, true)).toBe("optional");
    // B.6: the Upper Zone with 14 Member Channels leaves the Lower Zone none.
    mcm(15, 0x0e);
    expect(r.layout).toEqual({ lower: 0, upper: 14 });
    expect(mpeRole(r.layout, 0)).toEqual({ role: "noZone", zone: null });
    expect(r.setLayout(MPE_LAYOUT_DEFAULT)).toBe(0xffff);
    expect(r.pitchOffset(0)).toBe(0);
    r.free();
  });

  test("the MPE sender helpers: Appendix C, Table 4 and the note allocator", () => {
    const bytes = (ms: Midi1Message[]): number[][] => ms.map((m) => [m.status, m.data1, m.data2]);
    // C.4 and C.6: +7 semitones at 48 is 9387; 16383 at 2 is 2 semitones.
    expect(PITCH_UNITS_PER_SEMITONE).toBe(10_000);
    expect([pitchBendValue(70_000, 48), pitchBendValue(20_000, 2), pitchBendValue(-500_000, 48)]).toEqual([9387, 16383, 0]);
    expect(pitchOffset(16383, 2)).toBe(20_000);
    expect(Math.abs(pitchOffset(9387, 48) - 70_000)).toBeLessThanOrEqual(100);
    // Table 4 with the corrected Pitch Bend (AMB-M1-100-08): a quarter tone up at 48 semitones.
    expect(bytes(noteMessages(2, 0x3c, 0x38, pitchBendValue(5_000, 48), 0x40, 0))).toEqual([
      [0xe2, 0x55, 0x40],
      [0xb2, 0x4a, 0x40],
      [0xd2, 0x00, 0],
      [0x92, 0x3c, 0x38],
    ]);
    // Appendix B.1, the Mode message with M = 7, RPN 0 of a Zone.
    expect(bytes(mcmMessages("lower", 15))).toEqual([
      [0xb0, 0x65, 0],
      [0xb0, 0x64, 6],
      [0xb0, 0x06, 0x0f],
    ]);
    const two = mpeLayout(7, 7);
    expect(bytes([modeMessage(two, "upper", true)])).toEqual([[0xb8, 0x7e, 7]]);
    const zs = zoneSensitivityMessages(two, "upper", 2, 48);
    expect([zs.length, bytes(zs.slice(2, 3)), bytes(zs.slice(5, 6))]).toEqual([24, [[0xbf, 0x06, 2]], [[0xb8, 0x06, 48]]]);
    expect(bytes(sensitivityMessages(3, 12).slice(2))).toEqual([[0xb3, 0x06, 12]]);
    expect(mpeLayoutWithMcm(two, "upper", 14)).toEqual({ lower: 0, upper: 14 });
    expect([mpeLowestMember(two, "upper"), mpeLowestMember(mpeLayout(7, 0), "upper")]).toEqual([8, null]);
    // A.3: an unoccupied channel first, then the oldest last Note Off; the same-note option.
    const small = mpeLayout(3, 0);
    const a = new MpeNoteAllocator();
    expect([60, 62, 64].map((n) => a.noteOn(small, "lower", n))).toEqual([1, 2, 3]);
    a.noteOff(2);
    a.noteOff(1);
    expect([a.noteOn(small, "lower", 70), a.activeNotes(2), a.noteOn(small, "upper", 70)]).toEqual([2, 1, null]);
    const s = new MpeNoteAllocator(true);
    s.noteOn(small, "lower", 60);
    s.noteOn(small, "lower", 62);
    s.noteOff(1);
    s.noteOff(2);
    expect(s.noteOn(small, "lower", 60)).toBe(1);
    s.reset();
    expect([s.sameNote, s.activeNotes(1)]).toEqual([true, 0]);
    for (const bad of [
      () => mpeLayout(7, 8),
      () => mcmMessages("lower", 16),
      () => noteMessages(2, 60, 0),
      () => modeMessage(small, "upper", true),
      () => pitchOffset(16384, 48),
      () => pitchBendValue(0.5, 48),
    ]) {
      expect(bad).toThrow(InvalidValueError);
    }
    a.free();
    s.free();
  });

  test("the GM receiver: RP-024 rules", () => {
    // 2.4: channel 10 is a Rhythm Channel at the start; a bank MSB 78H makes channel 11 one too.
    const gm = new GmReceiver();
    expect([gm.level, gm.rhythmChannels, gm.kind(9), gm.kind(0)]).toEqual(["gm2", 0x0200, "rhythm", "melody"]);
    gm.program(10, 0x78 << 7);
    expect([gm.kind(10), gm.rhythmChannels]).toEqual(["rhythm", 0x0600]);
    // 3.5.6: Mono On with M = 1 gives Mode 4 on a Melody Channel, All Notes Off on a Rhythm Channel.
    expect(gm.receive(midi1Message(0xb0, 0x7e, 0x01)).kind).toBe("modeChange");
    expect(gm.isMono(0)).toBe(true);
    expect(gm.receive(midi1Message(0xb9, 0x7e, 0x01)).kind).toBe("allNotesOff");
    // 4.9.2, 4.9.3: GM1 System On resets the channels; GM System Off gives the level off.
    gm.reset("gm1");
    expect([gm.level, gm.rhythmChannels, gm.isMono(0)]).toEqual(["gm1", 0x0200, false]);
    gm.systemOff();
    expect(gm.level).toBe("off");
    // 3.3: the GM reset of a controller state holds the Rhythm bank on channel 10.
    const s = new ControllerState();
    s.gmReset(9);
    expect(s.bank).toBe(0x78 << 7);
    expect([gm2ControllerDefault(7), gm2ControllerDefault(120)]).toEqual([100, null]);
    expect([rhythmIgnoresController(1), rhythmIgnoresController(7)]).toEqual([true, false]);
    expect(rhythmIgnoresParameter({ msb: 0, lsb: 0, registered: true })).toBe(true);
    expect(rhythmIgnoresMessage(midi1Message(0xe9, 0x00, 0x40))).toBe(true);
    // 3.4.4: the GM2 default 00H 40H is 50 cents.
    expect(modulationDepthRange(0x40)).toBe(5000);
    expect(() => modulationDepthRange(0x4000)).toThrow(InvalidValueError);
    gm.free();
    s.free();
  });
});
