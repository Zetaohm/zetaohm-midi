// Universal System Exclusive, MIDI Time Code and MMC (REQ-TS-SX-*): M1 v4.2.1, RP-024, CA-025, MIDI
// Tuning, RP-004, RP-013.
import { beforeAll, describe, expect, test } from "vitest";

import {
  InvalidValueError,
  MalformedError,
  MtcGenerator,
  MtcReader,
  coarseTuningSemitones,
  deviceControlBytes,
  fineTuningOffset,
  frequencyWord,
  frequencyWordBytes,
  gmSystem,
  identityReply,
  identityRequest,
  init,
  manufacturerSysex,
  mmcCommands,
  mmcFields,
  mmcFullName,
  mmcResponses,
  mmcShortName,
  mtcFromFrameCount,
  mtcFull,
  mtcNextValid,
  mtcQuarterFrames,
  mtcTimeIsValid,
  mtcToFrameCount,
  mtcUserBits,
  parseSysex,
  readDeviceControl,
  readGmSystem,
  readInquiry,
  readMmc,
  readMmcFields,
  readMtcFull,
  readMtcUserBits,
  readTuning,
  respondIdentity,
  sysexAddressed,
  timeCode,
  timeCodeBytes,
  timeCodeFromBytes,
  timeCodeToStatusForm,
  timeCodeToSubframesForm,
  tuningKeyDump,
  tuningNoteChange,
  tuningScaleOctave,
  universalSysex,
} from "./index.js";
import type { Identity, MtcTime } from "./index.js";

beforeAll(async () => {
  await init();
});

const b = (...x: number[]): Uint8Array => new Uint8Array(x);
const t = (hours: number, minutes: number, seconds: number, frames: number, rate: number): MtcTime => ({
  hours,
  minutes,
  seconds,
  frames,
  rate,
});

describe("sysex", () => {
  test("the sysex base: M1 Table VII and VIIa", () => {
    const s = parseSysex(b(0xf0, 0x00, 0x20, 0x6b, 0x01, 0x02, 0xf7));
    expect([s.kind, s.manufacturer, s.data, s.deviceId]).toEqual(["manufacturer", b(0, 0x20, 0x6b), b(1, 2), null]);
    const u = parseSysex(b(0x7e, 0x10, 0x06, 0x01));
    expect([u.kind, u.deviceId, u.subId1, u.subId2, u.data]).toEqual(["nonRealTime", 0x10, 0x06, 0x01, b()]);
    expect(() => parseSysex(b(0xf0, 0xf7))).toThrow(MalformedError);
    expect([sysexAddressed(0x7f, 0x10), sysexAddressed(0x11, 0x10)]).toEqual([true, false]);
    expect(manufacturerSysex(b(0, 0x20, 0x6b), b(1, 2))).toEqual(b(0xf0, 0x00, 0x20, 0x6b, 0x01, 0x02, 0xf7));
    expect(() => manufacturerSysex(b(0x7e), b())).toThrow(InvalidValueError);
    expect(universalSysex(false, 0x10, 0x06, 0x01, b())).toEqual(b(0xf0, 0x7e, 0x10, 0x06, 0x01, 0xf7));
  });

  test("inquiry, GM System On and Device Control", () => {
    // M1 General Information: the responder answers a request to all devices with its own ID.
    const id: Identity = { manufacturer: b(0, 0x20, 0x6b), family: 0x0102, member: 3, revision: b(0, 0, 0, 1) };
    const reply = respondIdentity(b(0xf0, 0x7e, 0x7f, 0x06, 0x01, 0xf7), 0x10, id);
    expect(reply).toEqual(b(0xf0, 0x7e, 0x10, 0x06, 0x02, 0x00, 0x20, 0x6b, 0x02, 0x02, 0x03, 0x00, 0, 0, 0, 1, 0xf7));
    expect(readInquiry(reply ?? b())).toEqual({ isReply: true, deviceId: 0x10, identity: id });
    expect(respondIdentity(reply ?? b(), 0x10, id)).toBeNull();
    expect(identityRequest(0x7f)).toEqual(b(0xf0, 0x7e, 0x7f, 0x06, 0x01, 0xf7));
    expect(identityReply(0x10, id)).toEqual(reply);
    // RP-024 4.9.1: GM2 System On.
    const on = b(0xf0, 0x7e, 0x7f, 0x09, 0x03, 0xf7);
    expect(readGmSystem(on)).toEqual({ deviceId: 0x7f, command: 3 });
    expect(gmSystem(0x7f, 3)).toEqual(on);
    expect(readGmSystem(reply ?? b())).toBeNull();
    // M1 Device Control, CA-025.
    const vol = b(0xf0, 0x7f, 0x7f, 0x04, 0x01, 0x7f, 0x7f, 0xf7);
    expect(readDeviceControl(vol)).toEqual({ kind: "masterVolume", deviceId: 0x7f, value: 0x3fff });
    expect(deviceControlBytes({ kind: "masterVolume", deviceId: 0x7f, value: 0x3fff })).toEqual(vol);
    expect(() => deviceControlBytes({ kind: "masterCoarseTuning", deviceId: 0x7f, value: 0x80 })).toThrow(
      InvalidValueError,
    );
    expect([fineTuningOffset(0x2000), fineTuningOffset(0), coarseTuningSemitones(0x40)]).toEqual([0, -8192, 0]);
  });

  test("the MIDI Tuning messages", () => {
    // M1 MIDI Tuning: the frequency word 3CH 00H 00H is middle C.
    expect(frequencyWord(b(0x3c, 0, 0))).toEqual({ semitone: 0x3c, fraction: 0 });
    expect(frequencyWordBytes(0x3c, 0)).toEqual(b(0x3c, 0, 0));
    const name = new TextEncoder().encode("zmidi tuning    ");
    const words = Array.from({ length: 128 }, (_, k) => ({ semitone: k, fraction: k * 64 }));
    const dump = readTuning(tuningKeyDump(0x7f, 2, name, words, 1));
    expect([dump?.kind, dump?.bank, dump?.program, dump?.checksumOk, dump?.name]).toEqual(["keyDump", 1, 2, true, name]);
    expect([dump?.notes.length, dump?.notes[100]]).toEqual([128, { key: 100, semitone: 100, fraction: 6400 }]);
    const notes = [
      { key: 60, semitone: 0x3c, fraction: 0 },
      { key: 61, semitone: 0x3d, fraction: 0x2000 },
    ];
    const change = readTuning(tuningNoteChange(true, 0x7f, 5, notes));
    expect([change?.kind, change?.realTime, change?.notes]).toEqual(["noteChange", true, notes]);
    const offsets = [-64, ...Array<number>(10).fill(0), 63];
    const so = readTuning(tuningScaleOctave(false, 0x7f, 0xffff, offsets, false));
    expect([so?.kind, so?.channels, so?.offsets, so?.twoByte]).toEqual(["scaleOctave", 0xffff, offsets, false]);
    expect(() => tuningScaleOctave(false, 0x7f, 0xffff, [-65, ...Array<number>(11).fill(0)], false)).toThrow(
      InvalidValueError,
    );
  });

  test("MIDI Time Code: RP-004 examples", () => {
    // RP-004 p. 4: 01:37:52:16 at 30 frames per second.
    const time = t(1, 37, 52, 16, 3);
    const pieces = b(0x00, 0x11, 0x24, 0x33, 0x45, 0x52, 0x61, 0x76);
    expect(mtcQuarterFrames(time)).toEqual(pieces);
    // RP-013 p. 10: 00:22:00:00 drop-frame is skipped; 00:22:00:02 is 00:21:58:22 non-drop.
    expect(mtcTimeIsValid(t(0, 22, 0, 0, 2))).toBe(false);
    const df = mtcNextValid(t(0, 22, 0, 0, 2));
    expect(df).toEqual(t(0, 22, 0, 2, 2));
    expect(mtcFromFrameCount(mtcToFrameCount(df), 3)).toEqual(t(0, 21, 58, 22, 3));
    // The generator gives the pieces; the reader gives the current time, +2 frames forward.
    const g = new MtcGenerator(time);
    const r = new MtcReader();
    let last = r.receive(g.nextByte() ?? 0);
    for (let i = 1; i < 8; i++) {
      last = r.receive(g.nextByte() ?? 0);
    }
    expect([last.raw, last.current]).toEqual([time, t(1, 37, 52, 18, 3)]);
    expect(r.info).toEqual({ locked: true, running: true, direction: "forward", time: t(1, 37, 52, 18, 3) });
    g.setRunning(false);
    expect(g.nextByte()).toBeNull();
    // p. 5: the Full Message stops the reader with its time.
    const full = mtcFull(0x7f, time);
    expect(full).toEqual(b(0xf0, 0x7f, 0x7f, 0x01, 0x01, 0x61, 37, 52, 16, 0xf7));
    r.fullMessage(readMtcFull(full)?.time ?? t(0, 0, 0, 0, 0));
    expect([r.info.running, r.info.time]).toEqual([false, time]);
    // p. 6: User Bits.
    const ub = mtcUserBits(0x7f, b(1, 2, 3, 4, 5, 6, 7, 8), 2);
    expect(readMtcUserBits(ub)).toEqual({ deviceId: 0x7f, groups: b(1, 2, 3, 4, 5, 6, 7, 8), flags: 2 });
    g.free();
    r.free();
  });

  test("the MMC transport tier: RP-013 examples", () => {
    // RP-013 Example 2A (p. 76): 01:02:03:06 at 30 frames, status form.
    const tc = timeCodeFromBytes(b(0x61, 0x02, 0x03, 0x26, 0x00));
    expect([tc.time, tc.statusForm, tc.noTimeCode]).toEqual([t(1, 2, 3, 6, 3), true, false]);
    expect(timeCodeBytes(tc)).toEqual(b(0x61, 0x02, 0x03, 0x26, 0x00));
    // p. 32 MOVE: {st} to {ff} gives subframes 0; {ff} to {st} gives no time code.
    const ff = timeCodeToSubframesForm(tc);
    expect([ff.statusForm, ff.subframes]).toEqual([false, 0]);
    const st = timeCodeToStatusForm(ff, true);
    expect([st.statusForm, st.noTimeCode, st.videoField1]).toEqual([true, true, true]);
    expect([mmcShortName(0x06), mmcFullName(0x26), mmcFullName(0x20)]).toEqual([0x26, 0x06, null]);
    // STOP and LOCATE [TARGET] 01:02:03:04.05 (p. 28).
    const cmds = b(0xf0, 0x7f, 0x7f, 0x06, 0x01, 0x44, 0x06, 0x01, 0x61, 0x02, 0x03, 0x04, 0x05, 0xf7);
    const m = readMmc(cmds);
    expect([m?.response, m?.commands.map((c) => c.kind)]).toEqual([false, ["stop", "locateTarget"]]);
    expect(m?.commands[1]?.target?.subframes).toBe(5);
    expect(mmcCommands(0x7f, m?.commands ?? [])).toEqual(cmds);
    expect(() => readMmc(b(0xf0, 0x7f, 0x7f, 0x06, 0x44, 0x06, 0x01, 0xf7))).toThrow(MalformedError);
    expect(() => mmcCommands(0x7f, [{ kind: "locateField", register: 0x07 }])).toThrow(InvalidValueError);
    // A response: SELECTED TIME CODE {st} from device 01H, not negative; 7FH is not a source.
    const f = { kind: "timeCode", field: 0x01, code: tc } as const;
    const resp = mmcResponses(0x01, [f]);
    expect(resp).toEqual(b(0xf0, 0x7f, 0x01, 0x07, 0x01, 0x61, 0x02, 0x03, 0x26, 0x00, 0xf7));
    const r = readMmc(resp);
    expect([r?.response, r?.fields[0]?.field, r?.fields[0]?.data]).toEqual([true, 0x01, b(0x61, 2, 3, 0x26, 0)]);
    expect(() => mmcResponses(0x7f, [f])).toThrow(InvalidValueError);
    const neg = timeCode(t(1, 0, 0, 0, 3), { negative: true, statusForm: true });
    expect(() => mmcFields([{ kind: "timeCode", field: 0x01, code: neg }])).toThrow(InvalidValueError);
    expect(readMmcFields(mmcFields([f])).map((x) => x.field)).toEqual([0x01]);
  });
});
