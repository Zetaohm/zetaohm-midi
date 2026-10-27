// MIDI 1.0 byte stream to UMP and back (REQ-TS-IO-001): M2-104 Appendix D, MIDI 1.0.
import { beforeAll, describe, expect, test } from "vitest";

import { BytesToUmp, InvalidValueError, UmpToBytes, init } from "./index.js";

beforeAll(async () => {
  await init();
});

const b = (...x: number[]): Uint8Array => new Uint8Array(x);

describe("io", () => {
  test("MIDI 1.0 bytes to UMP with running status", () => {
    const r = new BytesToUmp(3);
    expect(r.feed(b(0x90, 0x3c, 0x64))).toEqual([[0x23903c64]]);
    expect(r.feed(b(0x3e, 0x40))).toEqual([[0x23903e40]]);
    // Timing Clock between data bytes comes out at once.
    expect(r.feed(b(0x90, 0x3c, 0xf8, 0x64))).toEqual([[0x13f80000], [0x23903c64]]);
    r.free();
  });

  test("bytes to MIDI 2.0 UMP and SysEx", () => {
    expect(new BytesToUmp(0, 2).feed(b(0x90, 0x3c, 0x64))).toEqual([[0x40903c00, 0xc9240000]]);
    expect(new BytesToUmp().feed(b(0xf0, 0x7e, 0x7f, 0x06, 0x01, 0xf7))).toEqual([[0x30047e7f, 0x06010000]]);
  });

  test("feed of a long stream gives the UMPs of push", () => {
    // OPT-035: one C call for many bytes, over more than one 4096-byte chunk and full word
    // buffers. Notes with running status and Clocks between data bytes, then a long SysEx.
    const bytes: number[] = [0x90];
    for (let i = 0; i < 3000; i++) {
      bytes.push(i & 0x7f, i % 3 === 0 ? 0xf8 : 0x40, 0x40);
    }
    bytes.push(0xf0, ...Array.from({ length: 5000 }, (_, i) => i & 0x7f), 0xf7);
    const data = new Uint8Array(bytes);
    for (const protocol of [1, 2]) {
      const a = new BytesToUmp(5, protocol);
      const want = [...data].flatMap((x) => a.push(x));
      expect(new BytesToUmp(5, protocol).feed(data)).toEqual(want);
    }
    const r = new BytesToUmp();
    r.free();
    expect(() => r.feed(b(0x90))).toThrow(/freed/);
  });

  test("UMP to bytes with running status and refresh", () => {
    const w = new UmpToBytes(true);
    expect(w.write([0x20903c64])).toEqual(b(0x90, 0x3c, 0x64));
    expect(w.write([0x20903e40])).toEqual(b(0x3e, 0x40));
    w.refreshStatus();
    expect(w.write([0x20903e40])).toEqual(b(0x90, 0x3e, 0x40));
  });

  test("bad values and a freed object", () => {
    expect(() => new BytesToUmp(16)).toThrow(InvalidValueError);
    expect(() => new BytesToUmp(0, 3)).toThrow(InvalidValueError);
    const r = new BytesToUmp();
    r.free();
    expect(() => r.push(0x90)).toThrow(/freed/);
  });
});
