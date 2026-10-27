// BLE-MIDI v1.0 packets (REQ-TS-BLE-001): sections 7 and 8.
import { beforeAll, describe, expect, test } from "vitest";

import { BleReader, BleWriter, InvalidValueError, init } from "./index.js";

beforeAll(async () => {
  await init();
});

const hdr = (t: number): number => 0x80 | (t >> 7);
const ts = (t: number): number => 0x80 | (t & 0x7f);
const b = (...x: number[]): Uint8Array => new Uint8Array(x);

describe("ble", () => {
  test("reader of section 7", () => {
    const r = new BleReader();
    expect(r.read(b(hdr(0x0a55), ts(0x0a55), 0x90, 0x3c, 0x64, ts(0x0a57), 0x80, 0x3c, 0x00))).toEqual({
      umps: [
        { millis: 0x0a55, words: [0x20903c64] },
        { millis: 0x0a57, words: [0x20803c00] },
      ],
      dropped: 0,
    });
    expect(r.read(b(hdr(5), ts(5), 0x90, 0x3c, 0x64, 0x40, 0x64)).umps[1]).toEqual({ millis: 5, words: [0x20904064] });
    expect(r.read(b(0x10, 0x90, 0x3c, 0x64))).toEqual({ umps: [], dropped: 1 });
  });

  test("writer of section 7 with running status", () => {
    const w = new BleWriter(23);
    expect(w.packetLen).toBe(20);
    expect(w.writePacket([{ millis: 0x0a55, words: [0x20903c64] }])).toEqual({ packet: b(hdr(0x0a55), ts(0x0a55), 0x90, 0x3c, 0x64), taken: 1 });
    w.reset(); // times never decrease (section 7); start a new stream
    expect(
      w.writePacket([
        { millis: 5, words: [0x20903c64] },
        { millis: 5, words: [0x20904064] },
      ]),
    ).toEqual({ packet: b(hdr(5), ts(5), 0x90, 0x3c, 0x64, 0x40, 0x64), taken: 2 });
    expect(w.writePacket([])).toEqual({ packet: b(), taken: 0 });
  });

  test("SysEx over packets and back", () => {
    const umps = [
      { millis: 1, words: [0x30160001, 0x02030405] },
      { millis: 1, words: [0x30260607, 0x08090a0b] },
      { millis: 1, words: [0x30360c0d, 0x0e0f1011] },
    ];
    const w = new BleWriter(23);
    const r = new BleReader();
    const got: (readonly number[])[] = [];
    let left = umps;
    for (let i = 0; i < 16; i++) {
      const { packet, taken } = w.writePacket(left);
      left = left.slice(taken);
      if (packet.length === 0) {
        break;
      }
      got.push(...r.read(packet).umps.map((u) => u.words));
    }
    expect(got).toEqual(umps.map((u) => u.words));
  });

  test("BLE bad values", () => {
    expect(() => new BleWriter(22)).toThrow(InvalidValueError);
    expect(() => new BleReader(16)).toThrow(InvalidValueError);
  });
});
