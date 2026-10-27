// UMP functions (REQ-TS-UMP-001, REQ-TS-UMP-002): M2-104 Table 4, 7.4 figures, Appendix D.2,
// M2-115 Tables 5 and 8.
import { beforeAll, describe, expect, test } from "vitest";

import {
  InvalidValueError,
  MalformedError,
  MidiError,
  downscale,
  init,
  midi2ControlChange,
  midi2NoteOn,
  midi2PitchBend,
  midi2ProgramChange,
  midi2ToMidi1,
  parse,
  upscale,
  wordCount,
} from "./index.js";

beforeAll(async () => {
  await init();
});

describe("ump", () => {
  test("word count of Table 4", () => {
    const want = [1, 1, 1, 2, 2, 4, 1, 1, 2, 2, 2, 3, 3, 4, 4, 4];
    expect(want.map((_, mt) => wordCount((mt << 28) >>> 0))).toEqual(want);
  });

  test("MIDI 2.0 writers match the figures", () => {
    expect(midi2NoteOn(0, 0, 60, 0xc000)).toEqual([0x40903c00, 0xc0000000]);
    expect(midi2ControlChange(1, 0, 7, 0xffffffff)).toEqual([0x41b00700, 0xffffffff]);
    expect(midi2ProgramChange(0, 0, 5, [1, 2])).toEqual([0x40c00001, 0x05000102]);
    expect(midi2PitchBend(2, 15, 0x80000000)).toEqual([0x42ef0000, 0x80000000]);
  });

  test("writers reject values out of range", () => {
    expect(() => midi2NoteOn(16, 0, 60, 1)).toThrow(InvalidValueError);
    expect(() => midi2NoteOn(0, 0, 128, 1)).toThrow(InvalidValueError);
    expect(new InvalidValueError("x")).toBeInstanceOf(MidiError);
  });

  test("parse gives the fields", () => {
    const u = parse([0x43923c00, 0xc0000000]);
    expect([u.messageType, u.group, u.status, u.channel, u.decoded]).toEqual([4, 3, 9, 2, true]);
    expect(u.words).toEqual([0x43923c00, 0xc0000000]);
    expect(() => parse([0x43923c00])).toThrow(MalformedError);
  });

  test("MIDI 2.0 to 1.0 of Appendix D.2", () => {
    expect(midi2ToMidi1(midi2NoteOn(0, 0, 60, 0xc000))).toEqual([0x20903c60]);
    expect(midi2ToMidi1(midi2NoteOn(0, 0, 60, 0x0100))).toEqual([0x20903c01]);
    expect(midi2ToMidi1([0x20903c64])).toBeNull();
  });

  test("scaling of M2-115", () => {
    expect([0x00, 0x40, 0x46, 0x60, 0x7f].map((v) => upscale(v, 7, 16))).toEqual([0, 0x8000, 0x8c30, 0xc104, 0xffff]);
    expect([0x1400, 0x8000, 0xaeba, 0xffff].map((v) => downscale(v, 16, 7))).toEqual([0x0a, 0x40, 0x57, 0x7f]);
    expect(() => upscale(0x80, 7, 16)).toThrow(InvalidValueError);
  });
});
