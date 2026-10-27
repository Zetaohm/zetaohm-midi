// MIDI Clip Files (REQ-TS-FILE-001): M2-116.
import { beforeAll, describe, expect, test } from "vitest";

import { ClipWriter, InvalidValueError, MalformedError, RejectedError, init, readClip } from "./index.js";

beforeAll(async () => {
  await init();
});

function words(data: Uint8Array): number[] {
  const v = new DataView(data.buffer, data.byteOffset, data.byteLength);
  const out: number[] = [];
  for (let at = 8; at < data.length; at += 4) {
    out.push(v.getUint32(at, false));
  }
  return out;
}

describe("file", () => {
  test("minimal clip", () => {
    const w = new ClipWriter({ ticksPerQuarterNote: 96 });
    w.start(0);
    const data = w.finish(0);
    expect(new TextDecoder().decode(data.slice(0, 8))).toBe("SMF2CLIP");
    expect(words(data)).toEqual([0x00400000, 0x00300060, 0x00400000, 0xf0200000, 0, 0, 0, 0x00400000, 0xf0210000, 0, 0, 0]);
    expect(() => w.event(0, [0x20903c64])).toThrow(RejectedError);
  });

  test("write and read back", () => {
    const noteOn = [0x40903c00, 0xffff0000];
    const noteOff = [0x40803c00, 0];
    const w = new ClipWriter({
      ticksPerQuarterNote: 96,
      tempo10ns: 50_000_000,
      timeSignature: [4, 2, 8],
      profiles: [{ profileId: new Uint8Array([0x7e, 0, 1, 2, 3]), destination: "group", channels: 5 }],
    });
    w.start(0);
    w.event(0, noteOn);
    w.event(96, noteOff);
    expect(() => w.event(10, noteOn)).toThrow(RejectedError);
    expect(() => w.event(96, noteOn.slice(0, 1))).toThrow(InvalidValueError);
    const notes = readClip(w.finish(192))
      .filter((e) => e.section === "sequence" && (e.words[0] === noteOn[0] || e.words[0] === noteOff[0]))
      .map((e) => [e.ticks, e.words]);
    expect(notes).toEqual([
      [0, noteOn],
      [96, noteOff],
    ]);
  });

  test("a long clip grows its buffer", () => {
    const w = new ClipWriter();
    w.start(0);
    for (let t = 0; t < 2000; t++) {
      w.event(t, [0x20903c64]);
    }
    const n = readClip(w.finish(2000)).filter((e) => e.section === "sequence" && e.words[0] === 0x20903c64).length;
    expect(n).toBe(2000);
  });

  test("bad clip input", () => {
    expect(() => readClip(new TextEncoder().encode("SMF1CLIP"))).toThrow(MalformedError);
    expect(() => new ClipWriter({ ticksPerQuarterNote: 0 })).toThrow(InvalidValueError);
  });
});
