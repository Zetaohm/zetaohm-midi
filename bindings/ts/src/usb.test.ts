// USB-MIDI 1.0 Event Packets and USB MIDI 2.0 UMP words (REQ-TS-USB-001).
import { beforeAll, describe, expect, test } from "vitest";

import { InvalidValueError, RejectedError, USB_MAX_BLOCKS, UsbFunction, UsbLink, gtbBlocks, gtbRequest, gtbTotalLength, init } from "./index.js";
import type { UsbBlock } from "./index.js";

beforeAll(async () => {
  await init();
});

const b = (...x: number[]): Uint8Array => new Uint8Array(x);

describe("usb", () => {
  test("Event Packets at setting 0", () => {
    const link = new UsbLink();
    expect([link.setting, link.isUmp]).toEqual([0, false]);
    expect(link.read(b(0x09, 0x90, 0x3c, 0x64))).toEqual({ umps: [[0x20903c64]], errors: 0 });
    expect(link.read(b(0x04, 0xf0, 0x7e, 0x7f, 0x07, 0x06, 0x01, 0xf7))).toEqual({ umps: [[0x30047e7f, 0x06010000]], errors: 0 });
    expect(link.writePacket([[0x20903c64]])).toEqual({ packet: b(0x09, 0x90, 0x3c, 0x64), taken: 1 });
  });

  test("UMP words at setting 1 and a USB reset", () => {
    const link = new UsbLink();
    link.setAlternateSetting(1, 0x0001, 64);
    expect([link.setting, link.isUmp]).toEqual([1, true]);
    const data = b(0x00, 0x3c, 0x90, 0x40, 0x00, 0x00, 0x00, 0xc0);
    expect(link.read(data)).toEqual({ umps: [[0x40903c00, 0xc0000000]], errors: 0 });
    expect(link.writePacket([[0x40903c00, 0xc0000000]])).toEqual({ packet: data, taken: 1 });
    link.usbReset();
    expect(link.setting).toBe(0);
  });

  test("MIDI 2.0 protocol needs no extra setup", () => {
    expect(new UsbLink({ protocol: 2 }).read(b(0x09, 0x90, 0x3c, 0x64))).toEqual({ umps: [[0x40903c00, 0xc9240000]], errors: 0 });
  });

  test("a large transfer after a small one", () => {
    // The link keeps one scratch block for its calls; a larger transfer makes it larger.
    const link = new UsbLink();
    expect(link.read(b(0x09, 0x90, 0x3c, 0x64)).umps).toEqual([[0x20903c64]]);
    const many = new Uint8Array(4 * 300);
    for (let i = 0; i < 300; i++) {
      many.set([0x09, 0x90, i & 0x7f, 0x40], 4 * i);
    }
    const r = link.read(many);
    expect(r.errors).toBe(0);
    expect(r.umps.length).toBe(300);
    expect(r.umps[299]).toEqual([0x20900040 | ((299 & 0x7f) << 8)]);
    expect(link.read(b(0x09, 0x80, 0x3c, 0x00)).umps).toEqual([[0x20803c00]]);
    link.free();
    expect(() => link.read(b(0x09, 0x90, 0x3c, 0x64))).toThrow(RejectedError);
  });

  test("USB bad values", () => {
    expect(() => new UsbLink({ maxPacket: 2 })).toThrow(InvalidValueError);
    expect(() => new UsbLink({ protocol: 7 })).toThrow(InvalidValueError);
  });
});

// USB descriptors and Group Terminal Block requests (REQ-TS-USB-002 to REQ-TS-USB-004). Vectors:
// USB-MIDI v2.0 Appendix B, Tables B-3 to B-20 (the function), B-21 and B-22 (the Group Terminal
// Block set), B-23 and B-24 (the requests); the same as crates/zmidi-ffi/tests/c/usb_desc.c.
const APPENDIX_B = b(
  ...[0x09, 0x04, 0x00, 0x00, 0x00, 0x01, 0x01, 0x00, 0x00], // B-3 standard AC interface
  ...[0x09, 0x24, 0x01, 0x00, 0x01, 0x09, 0x00, 0x01, 0x01], // B-4 AC header
  ...[0x09, 0x04, 0x01, 0x00, 0x02, 0x01, 0x03, 0x00, 0x00], // B-5 MS interface, alt 0
  ...[0x07, 0x24, 0x01, 0x00, 0x01, 0x41, 0x00], // B-6 MS header, bcdMSC 0x0100
  ...[0x06, 0x24, 0x02, 0x01, 0x01, 0x00], // B-7 Embedded MIDI IN Jack 1
  ...[0x06, 0x24, 0x02, 0x02, 0x02, 0x00], // B-8 External MIDI IN Jack 2
  ...[0x09, 0x24, 0x03, 0x01, 0x03, 0x01, 0x02, 0x01, 0x00], // B-9 Embedded MIDI OUT Jack 3
  ...[0x09, 0x24, 0x03, 0x02, 0x04, 0x01, 0x01, 0x01, 0x00], // B-10 External MIDI OUT Jack 4
  ...[0x09, 0x05, 0x01, 0x02, 0x40, 0x00, 0x00, 0x00, 0x00], // B-11 bulk OUT endpoint 1
  ...[0x05, 0x25, 0x01, 0x01, 0x01], // B-12 MS_GENERAL, Jack 1
  ...[0x09, 0x05, 0x81, 0x02, 0x40, 0x00, 0x00, 0x00, 0x00], // B-13 bulk IN endpoint 1
  ...[0x05, 0x25, 0x01, 0x01, 0x03], // B-14 MS_GENERAL, Jack 3
  ...[0x09, 0x04, 0x01, 0x01, 0x02, 0x01, 0x03, 0x00, 0x00], // B-15 MS interface, alt 1
  ...[0x07, 0x24, 0x01, 0x00, 0x02, 0x07, 0x00], // B-16 MS header, bcdMSC 0x0200
  ...[0x07, 0x05, 0x01, 0x02, 0x40, 0x00, 0x00], // B-17 bulk OUT endpoint 1
  ...[0x05, 0x25, 0x02, 0x01, 0x01], // B-18 MS_GENERAL_2_0, block 1
  ...[0x07, 0x05, 0x81, 0x03, 0x40, 0x00, 0x01], // B-19 interrupt IN endpoint 1
  ...[0x05, 0x25, 0x02, 0x01, 0x01], // B-20 MS_GENERAL_2_0, block 1
);
const B21_B22 = b(0x05, 0x26, 0x01, 0x12, 0x00, 0x0d, 0x26, 0x02, 0x01, 0x00, 0x00, 0x01, 0x04, 0x00, 0x01, 0x00, 0x00, 0x00);
const B23 = b(0x81, 0x06, 0x01, 0x26, 0x01, 0x00, 0x05, 0x00);
const B24 = b(0x81, 0x06, 0x01, 0x26, 0x01, 0x00, 0x12, 0x00);
/** The block of Table B-22: ID 1, bidirectional, Group Terminal 1, iBlockItem 4, bMIDIProtocol 0x00, wMaxInputBandwidth 1. */
const B22: UsbBlock = { id: 1, kind: "bidirectional", firstGroup: 0, groupCount: 1, string: 4, protocol: 0x00, maxInputBandwidth: 1 };

function appendixB(): UsbFunction {
  return new UsbFunction({
    acInterface: 0,
    msInterface: 1,
    settings: [
      {
        endpoints: [
          { direction: "out", number: 1 },
          { direction: "in", number: 1, transfer: "interrupt", interval: 1 },
        ],
        blocks: [B22],
      },
    ],
  });
}

describe("usb descriptors", () => {
  test("the function of Appendix B", () => {
    const f = appendixB();
    expect(f.descriptors()).toEqual(APPENDIX_B);
    expect(f.endpoints()).toEqual([
      { alternateSetting: 0, address: 0x01, transfer: 2, interval: 0, maxPacket: 64 },
      { alternateSetting: 0, address: 0x81, transfer: 2, interval: 0, maxPacket: 64 },
      { alternateSetting: 1, address: 0x01, transfer: 2, interval: 0, maxPacket: 64 },
      { alternateSetting: 1, address: 0x81, transfer: 3, interval: 1, maxPacket: 64 },
    ]);
    expect([f.groupMask(0, "out"), f.groupMask(1, "out"), f.groupMask(1, "in"), f.groupMask(2, "out")]).toEqual([0xffff, 1, 1, 0]);
    expect(f.gtbSet(1)).toEqual(B21_B22);
    expect(() => f.gtbSet(0)).toThrow(InvalidValueError);
    f.free();
    expect(() => f.descriptors()).toThrow(RejectedError);
  });

  test("the Group Terminal Block requests of section 6", () => {
    const f = appendixB();
    expect(f.answerSetup(B23)).toEqual({ response: "answered", data: B21_B22.slice(0, 5) });
    expect(f.answerSetup(B24)).toEqual({ response: "answered", data: B21_B22 });
    const other = B24.slice();
    other[4] = 0x02; // another interface
    expect(f.answerSetup(other)).toEqual({ response: "stall", data: b() });
    expect(f.answerSetup(b(0x80, 0x06, 0x00, 0x01, 0x00, 0x00, 0x12, 0x00))).toEqual({ response: "notHandled", data: b() });
    expect(() => f.answerSetup(b(1, 2, 3))).toThrow(InvalidValueError);
    // The host side.
    expect(gtbRequest(1, 1, 5)).toEqual(B23);
    expect(gtbRequest(1, 1, 0x12)).toEqual(B24);
    expect(gtbTotalLength(B21_B22.slice(0, 5))).toBe(0x12);
    expect(() => gtbTotalLength(B23)).toThrow(RejectedError);
    expect(gtbBlocks(B21_B22)).toEqual([
      { id: 1, kind: 0, firstGroup: 0, groupCount: 1, string: 4, protocol: 0, maxInputBandwidth: 1, maxOutputBandwidth: 0 },
    ]);
    expect(gtbBlocks(B21_B22.slice(0, 5))).toEqual([]);
  });

  test("the default function and the capacities", () => {
    const d = UsbFunction.defaultFunction(0, 1);
    const bytes = d.descriptors();
    expect(bytes.slice(0, 123)).toEqual(APPENDIX_B.slice(0, 123));
    expect([bytes[123], bytes[126]]).toEqual([0x02, 0x00]); // bulk IN, bInterval 0
    expect(gtbBlocks(d.gtbSet(1))).toEqual([
      { id: 1, kind: 0, firstGroup: 0, groupCount: 1, string: 0, protocol: 0x11, maxInputBandwidth: 0, maxOutputBandwidth: 0 },
    ]);
    expect(UsbFunction.defaultFunction(0, 1, true).endpoints()[3]!.maxPacket).toBe(512);
    // 16 blocks, one for each Group, on one MIDI OUT and one MIDI IN endpoint that name all 16.
    const ids = Array.from({ length: USB_MAX_BLOCKS }, (_, i) => i + 1);
    const all = new UsbFunction({
      acInterface: 0,
      msInterface: 1,
      settings: [
        {
          endpoints: [
            { direction: "out", number: 1, blocks: ids },
            { direction: "in", number: 1, blocks: ids },
          ],
          blocks: ids.map((id) => ({ ...B22, id, firstGroup: id - 1 })),
        },
      ],
    });
    expect([all.groupMask(1, "out"), all.groupMask(1, "in")]).toEqual([0xffff, 0xffff]);
    expect(all.gtbSet(1).length).toBe(5 + 13 * USB_MAX_BLOCKS);
    // Alternate Setting 0 only, with strings: the 92 bytes of USB-MIDI v1.0 Appendix B.
    const only = new UsbFunction({ acInterface: 0, msInterface: 1, acString: 5, alt0: { string: 6 } });
    const o = only.descriptors();
    expect([o.length, o[8], o[26]]).toEqual([92, 5, 6]);
    expect(only.answerSetup(B24).response).toBe("stall");
  });

  test("USB descriptor rules and bad values", () => {
    const ep = { direction: "out", number: 1 } as const;
    // The same interface number twice; no endpoint; a block named by no endpoint; no Group.
    expect(() => new UsbFunction({ acInterface: 1, msInterface: 1 })).toThrow(RejectedError);
    expect(() => new UsbFunction({ acInterface: 0, msInterface: 1, settings: [{ endpoints: [] }] })).toThrow(RejectedError);
    expect(() => new UsbFunction({ acInterface: 0, msInterface: 1, settings: [{ endpoints: [ep], blocks: [] }] })).toThrow(RejectedError);
    expect(() => new UsbFunction({ acInterface: 0, msInterface: 1, settings: [{ endpoints: [ep, { direction: "in", number: 1 }], blocks: [{ ...B22, groupCount: 0 }] }] })).toThrow(RejectedError);
    expect(() => new UsbFunction({ acInterface: 0, msInterface: 1, alt0: { outCables: 0, inCables: 0 } })).toThrow(RejectedError);
    // The capacities.
    const s = { endpoints: [ep, { direction: "in", number: 1 } as const] };
    expect(() => new UsbFunction({ acInterface: 0, msInterface: 1, settings: [s, s, s] })).toThrow(InvalidValueError);
    expect(() => new UsbFunction({ acInterface: 0, msInterface: 1, settings: [{ endpoints: [ep, ep, ep, ep, ep] }] })).toThrow(InvalidValueError);
    expect(() => new UsbFunction({ acInterface: 0, msInterface: 1, settings: [{ endpoints: [{ ...ep, blocks: [] }] }] })).toThrow(InvalidValueError);
    // Values out of range.
    expect(() => new UsbFunction({ acInterface: 256, msInterface: 1 })).toThrow(InvalidValueError);
    expect(() => new UsbFunction({ acInterface: 0, msInterface: 1, settings: [{ endpoints: [{ ...ep, number: 16 }] }] })).toThrow(InvalidValueError);
    expect(() => new UsbFunction({ acInterface: 0, msInterface: 1, settings: [{ endpoints: [{ direction: "x" as "in", number: 1 }] }] })).toThrow(InvalidValueError);
    expect(() => new UsbFunction({ acInterface: 0, msInterface: 1, settings: [{ endpoints: [ep], blocks: [{ ...B22, firstGroup: 16 }] }] })).toThrow(InvalidValueError);
    expect(() => gtbRequest(1, 1, 0x10000)).toThrow(InvalidValueError);
    expect(() => appendixB().groupMask(1, "x" as "in")).toThrow(InvalidValueError);
  });
});
