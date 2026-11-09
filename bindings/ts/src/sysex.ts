// Universal System Exclusive, MIDI Time Code, the MIDI Machine Control transport tier and Notation
// Information (zmidi-sysex through the C ABI, module Sysex of zmidi-ffi): M1 v4.2.1, RP-024, CA-025,
// MIDI Tuning, RP-004, RP-013. A reader takes one whole message, with or without F0H and F7H, and gives null for a message
// of another part; a bad message of its part throws MalformedError. A writer gives the bytes of the
// whole message; a value out of range throws InvalidValueError.

import { InvalidValueError, MalformedError } from "./errors.js";
import { Blocks, Status, alloc, check, fn, int, layout, readBytes, scoped, sizeOf, view, writeBytes } from "./wasm.js";

/** The size of a writer output: every message of the parts fits. */
const OUT = 600;

/** Runs `body` with a copy of `data` in the WASM memory. */
function withBytes<T>(data: Uint8Array, body: (ptr: number) => T): T {
  return scoped([Math.max(1, data.length)], (p) => {
    writeBytes(p, data);
    return body(p);
  });
}

/** Calls a C writer with an output of OUT bytes and gives the bytes. */
function written(what: string, call: (out: number, length: number) => number): Uint8Array {
  return scoped([OUT, 4], (out, len) => {
    check(call(out, len), what);
    return readBytes(out, view().getUint32(len, true));
  });
}

/** Calls a C part reader on `data`: null for a message of another part (REQ-FFI-SX-002). */
function read<T>(data: Uint8Array, what: string, call: (ptr: number, len: number) => number, ok: () => T): T | null {
  return withBytes(data, (p) => {
    const status = call(p, data.length);
    if (status === Status.NO_PENDING) {
      return null;
    }
    if (status === Status.INVALID_ARGUMENT) {
      throw new MalformedError(`${what}: not a valid message`);
    }
    check(status, what);
    return ok();
  });
}

/** The ID bytes of the C structs (`[id, 0, 0]` or `[00H, b2, b3]`) of 1 or 3 ID bytes. */
function cManufacturer(id: Uint8Array): Uint8Array {
  return id.length === 1 ? Uint8Array.of(id[0] ?? 0, 0, 0) : id.slice(0, 3);
}

/** The 1 or 3 ID bytes of the C form. */
function manufacturerOf(c: Uint8Array): Uint8Array {
  return c[0] === 0 ? c.slice(0, 3) : c.slice(0, 1);
}

const u8 = (p: number): number => view().getUint8(p);
const bool = (p: number): boolean => view().getUint8(p) !== 0;

// --- the sysex base ---

/** The class of a System Exclusive message (M1 Table VII). */
export type SysexClass = "manufacturer" | "nonCommercial" | "nonRealTime" | "realTime";
const CLASSES: readonly SysexClass[] = ["manufacturer", "nonCommercial", "nonRealTime", "realTime"];

/** One System Exclusive message. */
export interface SysexInfo {
  readonly kind: SysexClass;
  /** The 1 or 3 ID bytes of a manufacturer message, else empty. */
  readonly manufacturer: Uint8Array;
  /** The Device ID, Sub-ID#1 and Sub-ID#2 of a Universal message. */
  readonly deviceId: number | null;
  readonly subId1: number | null;
  readonly subId2: number | null;
  /** The data after the header (after the ID of a manufacturer or Non-Commercial message), without F7H. */
  readonly data: Uint8Array;
}

const SYSEX = layout([
  ["kind", "u32"],
  ["manufacturer", ["u8", 3]],
  ["device_id", "u8"],
  ["sub_id1", "u8"],
  ["sub_id2", "u8"],
  ["has_sub_id2", "bool"],
  ["data_offset", "u32"],
  ["data_length", "u32"],
]);

/** Reads one whole System Exclusive message, with or without F0H and F7H. */
export function parseSysex(data: Uint8Array): SysexInfo {
  return withBytes(data, (p) =>
    scoped([SYSEX.size], (s) => {
      if (fn("zmidi_sysex_parse")(p, data.length, s) !== Status.OK) {
        throw new MalformedError("parseSysex: not a valid System Exclusive message");
      }
      const v = view();
      const o = (f: string): number => s + (SYSEX.off[f] ?? 0);
      const kind = CLASSES[v.getUint32(o("kind"), true)] ?? "nonCommercial";
      const universal = kind === "nonRealTime" || kind === "realTime";
      const at = v.getUint32(o("data_offset"), true);
      return {
        kind,
        manufacturer: kind === "manufacturer" ? manufacturerOf(readBytes(o("manufacturer"), 3)) : new Uint8Array(),
        deviceId: universal ? u8(o("device_id")) : null,
        subId1: universal ? u8(o("sub_id1")) : null,
        subId2: universal && bool(o("has_sub_id2")) ? u8(o("sub_id2")) : null,
        data: data.slice(at, at + v.getUint32(o("data_length"), true)),
      };
    }),
  );
}

/** True when a received Device ID addresses the device: 7FH or `own`. */
export function sysexAddressed(deviceId: number, own: number): boolean {
  return scoped([1], (r) => {
    check(fn("zmidi_sysex_addressed")(int("deviceId", deviceId, 127), int("own", own, 127), r), "sysexAddressed");
    return bool(r);
  });
}

/** The bytes of a manufacturer message: F0H, the 1 or 3 ID bytes, `data` and F7H. */
export function manufacturerSysex(manufacturer: Uint8Array, data: Uint8Array): Uint8Array {
  return withBytes(cManufacturer(manufacturer), (m) =>
    withBytes(data, (d) =>
      written("manufacturerSysex", (out, len) =>
        fn("zmidi_sysex_write_manufacturer")(m, d, data.length, out, OUT, len),
      ),
    ),
  );
}

/** The bytes of a Universal message; `subId2` is given exactly for a family that has one. */
export function universalSysex(
  realTime: boolean,
  deviceId: number,
  subId1: number,
  subId2: number | null,
  data: Uint8Array,
): Uint8Array {
  const args = [realTime ? 1 : 0, int("deviceId", deviceId, 255), int("subId1", subId1, 255)] as const;
  return withBytes(data, (d) =>
    written("universalSysex", (out, len) =>
      fn("zmidi_sysex_write_universal")(
        ...args,
        subId2 === null ? 0 : 1,
        int("subId2", subId2 ?? 0, 255),
        d,
        data.length,
        out,
        OUT,
        len,
      ),
    ),
  );
}

// --- inquiry ---

/** The identity of a device in an Identity Reply (M1 General Information). */
export interface Identity {
  /** 1 byte, or 3 bytes 00H b2 b3. */
  readonly manufacturer: Uint8Array;
  /** 14 bits each. */
  readonly family: number;
  readonly member: number;
  /** 4 bytes of 7 bits. */
  readonly revision: Uint8Array;
}

/** An Identity Request (identity null) or an Identity Reply. */
export interface InquiryMessage {
  readonly isReply: boolean;
  readonly deviceId: number;
  readonly identity: Identity | null;
}

const IDENTITY = layout([
  ["manufacturer", ["u8", 3]],
  ["family", "u16"],
  ["member", "u16"],
  ["revision", ["u8", 4]],
]);

function writeIdentity(p: number, i: Identity): void {
  if (i.revision.length !== 4) {
    throw new MalformedError("identity: the revision is 4 bytes");
  }
  const v = view();
  writeBytes(p + (IDENTITY.off.manufacturer ?? 0), cManufacturer(i.manufacturer));
  v.setUint16(p + (IDENTITY.off.family ?? 0), int("family", i.family, 0xffff), true);
  v.setUint16(p + (IDENTITY.off.member ?? 0), int("member", i.member, 0xffff), true);
  writeBytes(p + (IDENTITY.off.revision ?? 0), i.revision);
}

function readIdentity(p: number): Identity {
  const v = view();
  return {
    manufacturer: manufacturerOf(readBytes(p + (IDENTITY.off.manufacturer ?? 0), 3)),
    family: v.getUint16(p + (IDENTITY.off.family ?? 0), true),
    member: v.getUint16(p + (IDENTITY.off.member ?? 0), true),
    revision: readBytes(p + (IDENTITY.off.revision ?? 0), 4),
  };
}

/** Reads an Identity Request or Reply, or gives null. */
export function readInquiry(data: Uint8Array): InquiryMessage | null {
  return scoped([1, 1, IDENTITY.size], (reply, dev, ident) =>
    read(
      data,
      "readInquiry",
      (p, n) => fn("zmidi_inquiry_read")(p, n, reply, dev, ident),
      () => ({ isReply: bool(reply), deviceId: u8(dev), identity: bool(reply) ? readIdentity(ident) : null }),
    ),
  );
}

/** The bytes of an Identity Request to `deviceId` (7FH: all devices). */
export function identityRequest(deviceId: number): Uint8Array {
  return written("identityRequest", (out, len) =>
    fn("zmidi_inquiry_request_write")(int("deviceId", deviceId, 255), out, OUT, len),
  );
}

/** The bytes of an Identity Reply from `deviceId`. */
export function identityReply(deviceId: number, identity: Identity): Uint8Array {
  return scoped([IDENTITY.size], (ident) => {
    writeIdentity(ident, identity);
    return written("identityReply", (out, len) =>
      fn("zmidi_inquiry_reply_write")(int("deviceId", deviceId, 255), ident, out, OUT, len),
    );
  });
}

/** The Identity Reply to an Identity Request for `ownDeviceId` or for all devices, or null. */
export function respondIdentity(data: Uint8Array, ownDeviceId: number, identity: Identity): Uint8Array | null {
  return scoped([IDENTITY.size, OUT, 4], (ident, out, len) => {
    writeIdentity(ident, identity);
    return read(
      data,
      "respondIdentity",
      (p, n) => fn("zmidi_inquiry_respond")(p, n, int("ownDeviceId", ownDeviceId, 255), ident, out, OUT, len),
      () => readBytes(out, view().getUint32(len, true)),
    );
  });
}

// --- gm and device-control ---

/** Reads a General MIDI System message: command 1 (GM1 System On), 2 (GM System Off) or 3 (GM2 System On). */
export function readGmSystem(data: Uint8Array): { deviceId: number; command: number } | null {
  return scoped([1, 1], (dev, cmd) =>
    read(
      data,
      "readGmSystem",
      (p, n) => fn("zmidi_gm_system_read")(p, n, dev, cmd),
      () => ({ deviceId: u8(dev), command: u8(cmd) }),
    ),
  );
}

/** The bytes of a General MIDI System message (command 1, 2 or 3). */
export function gmSystem(deviceId: number, command: number): Uint8Array {
  return written("gmSystem", (out, len) =>
    fn("zmidi_gm_system_write")(int("deviceId", deviceId, 255), int("command", command, 255), out, OUT, len),
  );
}

/** The kind of a Device Control message (M1, CA-025). */
export type DeviceControlKind = "masterVolume" | "masterBalance" | "masterFineTuning" | "masterCoarseTuning";
const DC_KINDS: readonly DeviceControlKind[] = ["masterVolume", "masterBalance", "masterFineTuning", "masterCoarseTuning"];

/** One Device Control message: a 14-bit value, or the 7-bit MSB of Master Coarse Tuning. */
export interface DeviceControl {
  readonly kind: DeviceControlKind;
  readonly deviceId: number;
  readonly value: number;
}

const DEVICE_CONTROL = layout([
  ["kind", "u32"],
  ["device_id", "u8"],
  ["value", "u16"],
]);

/** Reads a Device Control message, or gives null. */
export function readDeviceControl(data: Uint8Array): DeviceControl | null {
  return scoped([DEVICE_CONTROL.size], (dc) =>
    read(
      data,
      "readDeviceControl",
      (p, n) => fn("zmidi_device_control_read")(p, n, dc),
      () => {
        const v = view();
        return {
          kind: DC_KINDS[v.getUint32(dc + (DEVICE_CONTROL.off.kind ?? 0), true)] ?? "masterVolume",
          deviceId: u8(dc + (DEVICE_CONTROL.off.device_id ?? 0)),
          value: v.getUint16(dc + (DEVICE_CONTROL.off.value ?? 0), true),
        };
      },
    ),
  );
}

/** The bytes of a Device Control message. */
export function deviceControlBytes(control: DeviceControl): Uint8Array {
  return scoped([DEVICE_CONTROL.size], (dc) => {
    const v = view();
    v.setUint32(dc + (DEVICE_CONTROL.off.kind ?? 0), Math.max(0, DC_KINDS.indexOf(control.kind)), true);
    v.setUint8(dc + (DEVICE_CONTROL.off.device_id ?? 0), int("deviceId", control.deviceId, 255));
    v.setUint16(dc + (DEVICE_CONTROL.off.value ?? 0), int("value", control.value, 0xffff), true);
    return written("deviceControlBytes", (out, len) => fn("zmidi_device_control_write")(dc, out, OUT, len));
  });
}

/** The signed offset from 8192 of a 14-bit Master Fine Tuning value (CA-025). */
export function fineTuningOffset(value: number): number {
  return scoped([2], (p) => {
    check(fn("zmidi_fine_tuning_offset")(int("value", value, 0xffff), p), "fineTuningOffset");
    return view().getInt16(p, true);
  });
}

/** The semitones from A440 of a Master Coarse Tuning MSB, -64 to +63 (CA-025). */
export function coarseTuningSemitones(msb: number): number {
  return scoped([1], (p) => {
    check(fn("zmidi_coarse_tuning_semitones")(int("msb", msb, 255), p), "coarseTuningSemitones");
    return view().getInt8(p);
  });
}

// --- tuning ---

/** A frequency word: the semitone and the 14-bit fraction. */
export interface FrequencyWord {
  readonly semitone: number;
  readonly fraction: number;
}

/** One note of a tuning. */
export interface TuningNote extends FrequencyWord {
  readonly key: number;
}

/** The kind of a MIDI Tuning message. */
export type TuningKind = "dumpRequest" | "keyDump" | "noteChange" | "scaleDump" | "scaleOctave";
const TUNING_KINDS: readonly TuningKind[] = ["dumpRequest", "keyDump", "noteChange", "scaleDump", "scaleOctave"];

/** One MIDI Tuning message; the fields that the kind does not have are 0, null or empty. */
export interface TuningMessage {
  readonly kind: TuningKind;
  readonly realTime: boolean;
  readonly deviceId: number;
  readonly bank: number | null;
  readonly program: number;
  /** The 16-byte name of a dump. */
  readonly name: Uint8Array;
  /** The checksum result of a 01H Bulk Tuning Dump. */
  readonly checksumOk: boolean;
  /** The channel mask of scaleOctave. */
  readonly channels: number;
  readonly twoByte: boolean;
  /** The 12 offsets of scaleDump and scaleOctave. */
  readonly offsets: number[];
  /** The notes of keyDump (128) and noteChange. */
  readonly notes: TuningNote[];
}

const WORD = layout([
  ["semitone", "u8"],
  ["fraction", "u16"],
]);
const NOTE = layout([
  ["key", "u8"],
  ["word", WORD],
]);
const OFFSETS = layout([
  ["two_byte", "bool"],
  ["offsets", ["u16", 12]],
]);
const TUNING = layout([
  ["kind", "u32"],
  ["real_time", "bool"],
  ["device_id", "u8"],
  ["has_bank", "bool"],
  ["bank", "u8"],
  ["program", "u8"],
  ["name", ["u8", 16]],
  ["checksum_ok", "bool"],
  ["channels", "u16"],
  ["offsets", OFFSETS],
  ["count", "u32"],
]);

function writeWord(p: number, w: FrequencyWord): void {
  view().setUint8(p + (WORD.off.semitone ?? 0), int("semitone", w.semitone, 255));
  view().setUint16(p + (WORD.off.fraction ?? 0), int("fraction", w.fraction, 0xffff), true);
}

function readWord(p: number): FrequencyWord {
  return { semitone: u8(p + (WORD.off.semitone ?? 0)), fraction: view().getUint16(p + (WORD.off.fraction ?? 0), true) };
}

function writeOffsets(p: number, offsets: readonly number[], twoByte: boolean): void {
  if (offsets.length !== 12) {
    throw new MalformedError("offsets: 12 values");
  }
  const v = view();
  v.setUint8(p + (OFFSETS.off.two_byte ?? 0), twoByte ? 1 : 0);
  offsets.forEach((x, i) => {
    if (!Number.isInteger(x) || x < -32768 || x > 32767) {
      throw new MalformedError("offsets: 16-bit values");
    }
    v.setInt16(p + (OFFSETS.off.offsets ?? 0) + 2 * i, x, true);
  });
}

/** Reads a MIDI Tuning message, or gives null. */
export function readTuning(data: Uint8Array): TuningMessage | null {
  return scoped([TUNING.size, 128 * NOTE.size], (t, notes) =>
    read(
      data,
      "readTuning",
      (p, n) => fn("zmidi_tuning_read")(p, n, t, notes, 128),
      () => {
        const v = view();
        const o = (f: string): number => t + (TUNING.off[f] ?? 0);
        const offs = o("offsets") + (OFFSETS.off.offsets ?? 0);
        const kind = TUNING_KINDS[v.getUint32(o("kind"), true)] ?? "dumpRequest";
        const named = kind === "keyDump" || kind === "scaleDump";
        const scale = kind === "scaleDump" || kind === "scaleOctave";
        return {
          kind,
          realTime: bool(o("real_time")),
          deviceId: u8(o("device_id")),
          bank: bool(o("has_bank")) ? u8(o("bank")) : null,
          program: u8(o("program")),
          name: named ? readBytes(o("name"), 16) : new Uint8Array(),
          checksumOk: bool(o("checksum_ok")),
          channels: v.getUint16(o("channels"), true),
          twoByte: bool(o("offsets") + (OFFSETS.off.two_byte ?? 0)),
          offsets: scale ? Array.from({ length: 12 }, (_, i) => v.getInt16(offs + 2 * i, true)) : [],
          notes: Array.from({ length: v.getUint32(o("count"), true) }, (_, i) => {
            const p = notes + i * NOTE.size;
            return { key: u8(p + (NOTE.off.key ?? 0)), ...readWord(p + (NOTE.off.word ?? 0)) };
          }),
        };
      },
    ),
  );
}

/** The bytes of a Bulk Tuning Dump Request (00H, or 03H with a bank). */
export function tuningDumpRequest(deviceId: number, program: number, bank: number | null = null): Uint8Array {
  return written("tuningDumpRequest", (out, len) =>
    fn("zmidi_tuning_dump_request_write")(
      int("deviceId", deviceId, 255),
      bank === null ? 0 : 1,
      int("bank", bank ?? 0, 255),
      int("program", program, 255),
      out,
      OUT,
      len,
    ),
  );
}

function name16(name: Uint8Array): Uint8Array {
  if (name.length !== 16) {
    throw new MalformedError("name: 16 bytes");
  }
  return name;
}

/** The bytes of a Bulk Tuning Dump (01H), or a Key-Based Tuning Dump (04H) with a bank, from 128 words. */
export function tuningKeyDump(
  deviceId: number,
  program: number,
  name: Uint8Array,
  words: readonly FrequencyWord[],
  bank: number | null = null,
): Uint8Array {
  if (words.length !== 128) {
    throw new MalformedError("tuningKeyDump: 128 frequency words");
  }
  return withBytes(name16(name), (pn) =>
    scoped([128 * WORD.size], (pw) => {
      words.forEach((w, i) => writeWord(pw + i * WORD.size, w));
      return written("tuningKeyDump", (out, len) =>
        fn("zmidi_tuning_key_dump_write")(
          int("deviceId", deviceId, 255),
          bank === null ? 0 : 1,
          int("bank", bank ?? 0, 255),
          int("program", program, 255),
          pn,
          pw,
          out,
          OUT,
          len,
        ),
      );
    }),
  );
}

/** The bytes of a Single Note Tuning Change: Real Time 02H, or 07H with a bank, from 1 to 127 notes. */
export function tuningNoteChange(
  realTime: boolean,
  deviceId: number,
  program: number,
  notes: readonly TuningNote[],
  bank: number | null = null,
): Uint8Array {
  return scoped([Math.max(1, notes.length) * NOTE.size], (pn) => {
    notes.forEach((n, i) => {
      view().setUint8(pn + i * NOTE.size + (NOTE.off.key ?? 0), int("key", n.key, 255));
      writeWord(pn + i * NOTE.size + (NOTE.off.word ?? 0), n);
    });
    return written("tuningNoteChange", (out, len) =>
      fn("zmidi_tuning_note_change_write")(
        realTime ? 1 : 0,
        int("deviceId", deviceId, 255),
        bank === null ? 0 : 1,
        int("bank", bank ?? 0, 255),
        int("program", program, 255),
        pn,
        notes.length,
        out,
        OUT,
        len,
      ),
    );
  });
}

/** The bytes of a Scale/Octave Tuning Dump (05H, or 06H for `twoByte`). */
export function tuningScaleDump(
  deviceId: number,
  bank: number,
  program: number,
  name: Uint8Array,
  offsets: readonly number[],
  twoByte: boolean,
): Uint8Array {
  return withBytes(name16(name), (pn) =>
    scoped([OFFSETS.size], (po) => {
      writeOffsets(po, offsets, twoByte);
      const args = [int("deviceId", deviceId, 255), int("bank", bank, 255), int("program", program, 255)] as const;
      return written("tuningScaleDump", (out, len) =>
        fn("zmidi_tuning_scale_dump_write")(...args, pn, po, out, OUT, len),
      );
    }),
  );
}

/** The bytes of a Scale/Octave Tuning message (08H, or 09H for `twoByte`) for the channel mask `channels`. */
export function tuningScaleOctave(
  realTime: boolean,
  deviceId: number,
  channels: number,
  offsets: readonly number[],
  twoByte: boolean,
): Uint8Array {
  return scoped([OFFSETS.size], (po) => {
    writeOffsets(po, offsets, twoByte);
    return written("tuningScaleOctave", (out, len) =>
      fn("zmidi_tuning_scale_octave_write")(
        realTime ? 1 : 0,
        int("deviceId", deviceId, 255),
        int("channels", channels, 0xffff),
        po,
        out,
        OUT,
        len,
      ),
    );
  });
}

/** The checksum of the bytes of a tuning dump: the XOR of the bytes, masked to 7 bits. */
export function tuningChecksum(data: Uint8Array): number {
  return withBytes(data, (p) =>
    scoped([1], (c) => {
      check(fn("zmidi_tuning_checksum")(p, data.length, c), "tuningChecksum");
      return u8(c);
    }),
  );
}

/** The frequency word of three bytes. */
export function frequencyWord(data: Uint8Array): FrequencyWord {
  if (data.length !== 3) {
    throw new MalformedError("frequencyWord: 3 bytes");
  }
  return withBytes(data, (p) =>
    scoped([WORD.size], (w) => {
      check(fn("zmidi_frequency_word_from_bytes")(p, w), "frequencyWord");
      return readWord(w);
    }),
  );
}

/** The three bytes of a frequency word. */
export function frequencyWordBytes(semitone: number, fraction: number): Uint8Array {
  return scoped([WORD.size, 3], (w, b) => {
    writeWord(w, { semitone, fraction });
    check(fn("zmidi_frequency_word_to_bytes")(w, b), "frequencyWordBytes");
    return readBytes(b, 3);
  });
}

// --- mtc ---

/** An MTC time: `rate` 0 (24), 1 (25), 2 (30 drop-frame) or 3 (30 frames per second). */
export interface MtcTime {
  readonly hours: number;
  readonly minutes: number;
  readonly seconds: number;
  readonly frames: number;
  readonly rate: number;
}

const TIME = layout([
  ["hours", "u8"],
  ["minutes", "u8"],
  ["seconds", "u8"],
  ["frames", "u8"],
  ["rate", "u8"],
]);
const TIME_FIELDS = ["hours", "minutes", "seconds", "frames", "rate"] as const;

function writeTime(p: number, t: MtcTime): void {
  for (const f of TIME_FIELDS) {
    view().setUint8(p + (TIME.off[f] ?? 0), int(f, t[f], 255));
  }
}

function readTime(p: number): MtcTime {
  const [hours, minutes, seconds, frames, rate] = TIME_FIELDS.map((f) => u8(p + (TIME.off[f] ?? 0)));
  return { hours: hours ?? 0, minutes: minutes ?? 0, seconds: seconds ?? 0, frames: frames ?? 0, rate: rate ?? 0 };
}

/** Runs a C function of a time and an output. */
function onTime<T>(t: MtcTime, size: number, what: string, call: (pt: number, out: number) => number, get: (out: number) => T): T {
  return scoped([TIME.size, size], (pt, out) => {
    writeTime(pt, t);
    check(call(pt, out), what);
    return get(out);
  });
}

/** True when each count of `t` is in range and the frame number is not one that drop-frame skips. */
export function mtcTimeIsValid(t: MtcTime): boolean {
  return onTime(t, 1, "mtcTimeIsValid", (pt, o) => fn("zmidi_mtc_time_is_valid")(pt, o), bool);
}

/** The time, or frame 02 for a frame number that the drop-frame type skips. */
export function mtcNextValid(t: MtcTime): MtcTime {
  return onTime(t, TIME.size, "mtcNextValid", (pt, o) => fn("zmidi_mtc_next_valid")(pt, o), readTime);
}

/** The frames from 00:00:00:00 to `t` by the rule of its type. */
export function mtcToFrameCount(t: MtcTime): number {
  return onTime(t, 4, "mtcToFrameCount", (pt, o) => fn("zmidi_mtc_to_frame_count")(pt, o), (o) => view().getUint32(o, true));
}

/** The time of `count` frames from 00:00:00:00, with the 24-hour wrap. */
export function mtcFromFrameCount(count: number, rate: number): MtcTime {
  return scoped([TIME.size], (o) => {
    check(fn("zmidi_mtc_from_frame_count")(int("count", count, 0xffffffff), int("rate", rate, 255), o), "mtcFromFrameCount");
    return readTime(o);
  });
}

/** The time `delta` frames later (or earlier), with the 24-hour wrap. */
export function mtcAddFrames(t: MtcTime, delta: number): MtcTime {
  if (!Number.isInteger(delta) || delta < -0x80000000 || delta > 0x7fffffff) {
    throw new MalformedError("delta: a 32-bit integer");
  }
  return onTime(t, TIME.size, "mtcAddFrames", (pt, o) => fn("zmidi_mtc_add_frames")(pt, delta, o), readTime);
}

/** The 8 Quarter Frame data bytes of `t`, piece 0 first. */
export function mtcQuarterFrames(t: MtcTime): Uint8Array {
  return onTime(t, 8, "mtcQuarterFrames", (pt, o) => fn("zmidi_mtc_quarter_frames")(pt, o), (o) => readBytes(o, 8));
}

/** The Quarter Frame generator (RP-004 p. 3): one data byte at each call, with no clock. */
export class MtcGenerator {
  readonly #b = new Blocks(this);

  /** A running generator: the first sequence encodes `time` from piece 0, or from piece 7 when `reverse`. */
  constructor(time: MtcTime, reverse = false) {
    const p = this.#b.add(alloc(sizeOf("zmidi_mtc_generator_size")));
    scoped([TIME.size], (pt) => {
      writeTime(pt, time);
      check(fn("zmidi_mtc_generator_init")(p, pt, reverse ? 1 : 0), "MtcGenerator");
    });
  }

  /** The data byte of the next piece, or null while stopped. */
  nextByte(): number | null {
    return scoped([1], (b) => {
      const status = fn("zmidi_mtc_generator_next")(this.#b.main, b);
      if (status === Status.NO_PENDING) {
        return null;
      }
      check(status, "nextByte");
      return u8(b);
    });
  }

  /** Runs or stops the generator. */
  setRunning(running: boolean): void {
    check(fn("zmidi_mtc_generator_set_running")(this.#b.main, running ? 1 : 0), "setRunning");
  }

  /** Sets the direction; the next piece is the next one in the new order. */
  setDirection(reverse: boolean): void {
    check(fn("zmidi_mtc_generator_set_direction")(this.#b.main, reverse ? 1 : 0), "setDirection");
  }

  /** A new time; the next piece is the first piece of the direction. */
  locate(time: MtcTime): void {
    scoped([TIME.size], (pt) => {
      writeTime(pt, time);
      check(fn("zmidi_mtc_generator_locate")(this.#b.main, pt), "locate");
    });
  }

  /** The time that the current or next sequence encodes at its piece 0. */
  get time(): MtcTime {
    return scoped([TIME.size], (pt) => {
      check(fn("zmidi_mtc_generator_time")(this.#b.main, pt), "time");
      return readTime(pt);
    });
  }

  /** Frees the WASM memory of the generator now. */
  free(): void {
    this.#b.release();
  }
}

/** What one Quarter Frame data byte gives; `raw` and `current` at the end of a full sequence only. */
export interface MtcEvent {
  readonly piece: number;
  readonly boundary: boolean;
  readonly raw: MtcTime | null;
  /** The raw time plus 2 frames forward, minus 1 frame in reverse. */
  readonly current: MtcTime | null;
  readonly reverse: boolean;
  readonly discontinuity: boolean;
}

/** The state of an MTC reader. */
export interface MtcReaderInfo {
  readonly locked: boolean;
  readonly running: boolean;
  readonly direction: "forward" | "reverse" | null;
  readonly time: MtcTime | null;
}

const MTC_EVENT = layout([
  ["piece", "u8"],
  ["boundary", "bool"],
  ["has_sequence", "bool"],
  ["raw", TIME],
  ["current", TIME],
  ["reverse", "bool"],
  ["discontinuity", "bool"],
]);
const MTC_INFO = layout([
  ["locked", "bool"],
  ["running", "bool"],
  ["has_direction", "bool"],
  ["reverse", "bool"],
  ["has_time", "bool"],
  ["time", TIME],
]);

/** The Quarter Frame reader (RP-004 p. 3): lock, direction and the current time. */
export class MtcReader {
  readonly #b = new Blocks(this);

  constructor() {
    const p = this.#b.add(alloc(sizeOf("zmidi_mtc_reader_size")));
    check(fn("zmidi_mtc_reader_init")(p), "MtcReader");
  }

  /** Takes one Quarter Frame data byte, 0 to 127. */
  receive(byte: number): MtcEvent {
    return scoped([MTC_EVENT.size], (e) => {
      check(fn("zmidi_mtc_reader_receive")(this.#b.main, int("byte", byte, 255), e), "receive");
      const o = (f: string): number => e + (MTC_EVENT.off[f] ?? 0);
      const seq = bool(o("has_sequence"));
      return {
        piece: u8(o("piece")),
        boundary: bool(o("boundary")),
        raw: seq ? readTime(o("raw")) : null,
        current: seq ? readTime(o("current")) : null,
        reverse: bool(o("reverse")),
        discontinuity: bool(o("discontinuity")),
      };
    });
  }

  /** Takes the time of a Full Message: the reader has this time and is stopped until the next data byte. */
  fullMessage(time: MtcTime): void {
    scoped([TIME.size], (pt) => {
      writeTime(pt, time);
      check(fn("zmidi_mtc_reader_full_message")(this.#b.main, pt), "fullMessage");
    });
  }

  /** The lock, the run state, the direction and the time. */
  get info(): MtcReaderInfo {
    return scoped([MTC_INFO.size], (i) => {
      check(fn("zmidi_mtc_reader_info")(this.#b.main, i), "info");
      const o = (f: string): number => i + (MTC_INFO.off[f] ?? 0);
      return {
        locked: bool(o("locked")),
        running: bool(o("running")),
        direction: bool(o("has_direction")) ? (bool(o("reverse")) ? "reverse" : "forward") : null,
        time: bool(o("has_time")) ? readTime(o("time")) : null,
      };
    });
  }

  /** Frees the WASM memory of the reader now. */
  free(): void {
    this.#b.release();
  }
}

/** Reads an MTC Full Message, or gives null. */
export function readMtcFull(data: Uint8Array): { deviceId: number; time: MtcTime } | null {
  return scoped([1, TIME.size], (dev, pt) =>
    read(
      data,
      "readMtcFull",
      (p, n) => fn("zmidi_mtc_full_read")(p, n, dev, pt),
      () => ({ deviceId: u8(dev), time: readTime(pt) }),
    ),
  );
}

/** The bytes of an MTC Full Message. */
export function mtcFull(deviceId: number, time: MtcTime): Uint8Array {
  return scoped([TIME.size], (pt) => {
    writeTime(pt, time);
    return written("mtcFull", (out, len) => fn("zmidi_mtc_full_write")(int("deviceId", deviceId, 255), pt, out, OUT, len));
  });
}

/** Reads an MTC User Bits message, or gives null. */
export function readMtcUserBits(data: Uint8Array): { deviceId: number; groups: Uint8Array; flags: number } | null {
  return scoped([1, 9], (dev, bits) =>
    read(
      data,
      "readMtcUserBits",
      (p, n) => fn("zmidi_mtc_user_bits_read")(p, n, dev, bits),
      () => ({ deviceId: u8(dev), groups: readBytes(bits, 8), flags: u8(bits + 8) }),
    ),
  );
}

/** The bytes of an MTC User Bits message: 8 groups of 4 bits and the 2 flag bits. */
export function mtcUserBits(deviceId: number, groups: Uint8Array, flags: number): Uint8Array {
  if (groups.length !== 8) {
    throw new MalformedError("mtcUserBits: 8 groups");
  }
  return scoped([9], (bits) => {
    writeBytes(bits, groups);
    view().setUint8(bits + 8, int("flags", flags, 255));
    return written("mtcUserBits", (out, len) =>
      fn("zmidi_mtc_user_bits_write")(int("deviceId", deviceId, 255), bits, out, OUT, len),
    );
  });
}

// --- mmc-transport ---

/** An MMC Standard Time Code (RP-013 pp. 8, 9): the subframes form, or the status form with its flags. */
export interface TimeCode {
  readonly time: MtcTime;
  readonly colorFrame: boolean;
  readonly blank: boolean;
  readonly negative: boolean;
  readonly statusForm: boolean;
  readonly subframes: number;
  readonly estimated: boolean;
  readonly invalid: boolean;
  readonly videoField1: boolean;
  readonly noTimeCode: boolean;
}

const TIME_CODE = layout([
  ["time", TIME],
  ["color_frame", "bool"],
  ["blank", "bool"],
  ["negative", "bool"],
  ["status_form", "bool"],
  ["subframes", "u8"],
  ["estimated", "bool"],
  ["invalid", "bool"],
  ["video_field1", "bool"],
  ["no_time_code", "bool"],
]);
const FLAGS = [
  ["colorFrame", "color_frame"],
  ["blank", "blank"],
  ["negative", "negative"],
  ["statusForm", "status_form"],
  ["estimated", "estimated"],
  ["invalid", "invalid"],
  ["videoField1", "video_field1"],
  ["noTimeCode", "no_time_code"],
] as const;

function writeTimeCode(p: number, tc: TimeCode): void {
  writeTime(p + (TIME_CODE.off.time ?? 0), tc.time);
  for (const [k, c] of FLAGS) {
    view().setUint8(p + (TIME_CODE.off[c] ?? 0), tc[k] ? 1 : 0);
  }
  view().setUint8(p + (TIME_CODE.off.subframes ?? 0), int("subframes", tc.subframes, 255));
}

function readTimeCode(p: number): TimeCode {
  const f = (c: string): boolean => bool(p + (TIME_CODE.off[c] ?? 0));
  return {
    time: readTime(p + (TIME_CODE.off.time ?? 0)),
    colorFrame: f("color_frame"),
    blank: f("blank"),
    negative: f("negative"),
    statusForm: f("status_form"),
    subframes: u8(p + (TIME_CODE.off.subframes ?? 0)),
    estimated: f("estimated"),
    invalid: f("invalid"),
    videoField1: f("video_field1"),
    noTimeCode: f("no_time_code"),
  };
}

/** A time code with default flags: the subframes form with subframes 0. */
export function timeCode(time: MtcTime, options: Partial<Omit<TimeCode, "time">> = {}): TimeCode {
  return {
    time,
    colorFrame: false,
    blank: false,
    negative: false,
    statusForm: false,
    subframes: 0,
    estimated: false,
    invalid: false,
    videoField1: false,
    noTimeCode: false,
    ...options,
  };
}

/** Runs a C function of a time code. */
function onCode<T>(tc: TimeCode, size: number, what: string, call: (pc: number, out: number) => number, get: (out: number) => T): T {
  return scoped([TIME_CODE.size, size], (pc, out) => {
    writeTimeCode(pc, tc);
    check(call(pc, out), what);
    return get(out);
  });
}

/** Reads a Standard Time Code from its 5 bytes; the counts are as received. */
export function timeCodeFromBytes(data: Uint8Array): TimeCode {
  if (data.length !== 5) {
    throw new MalformedError("timeCodeFromBytes: 5 bytes");
  }
  return withBytes(data, (p) =>
    scoped([TIME_CODE.size], (pc) => {
      check(fn("zmidi_time_code_from_bytes")(p, pc), "timeCodeFromBytes");
      return readTimeCode(pc);
    }),
  );
}

/** The 5 bytes of a Standard Time Code; a time that is not valid throws InvalidValueError. */
export function timeCodeBytes(tc: TimeCode): Uint8Array {
  return onCode(tc, 5, "timeCodeBytes", (pc, o) => fn("zmidi_time_code_to_bytes")(pc, o), (o) => readBytes(o, 5));
}

/** The time code with the 2 bytes of a short time code applied to the last full value `last`. */
export function timeCodeWithShort(last: TimeCode, data: Uint8Array): TimeCode {
  if (data.length !== 2) {
    throw new MalformedError("timeCodeWithShort: 2 bytes");
  }
  return withBytes(data, (p) =>
    onCode(last, TIME_CODE.size, "timeCodeWithShort", (pc, o) => fn("zmidi_time_code_with_short")(pc, p, o), readTimeCode),
  );
}

/** The 2 bytes of the short form of a time code. */
export function timeCodeShortBytes(tc: TimeCode): Uint8Array {
  return onCode(tc, 2, "timeCodeShortBytes", (pc, o) => fn("zmidi_time_code_short_bytes")(pc, o), (o) => readBytes(o, 2));
}

/** The MOVE rule to the subframes form (RP-013 p. 32). */
export function timeCodeToSubframesForm(tc: TimeCode): TimeCode {
  return onCode(tc, TIME_CODE.size, "timeCodeToSubframesForm", (pc, o) => fn("zmidi_time_code_to_subframes_form")(pc, o), readTimeCode);
}

/** The MOVE rule to the status form, with the video field 1 flag of the caller (RP-013 p. 32). */
export function timeCodeToStatusForm(tc: TimeCode, videoField1: boolean): TimeCode {
  return onCode(
    tc,
    TIME_CODE.size,
    "timeCodeToStatusForm",
    (pc, o) => fn("zmidi_time_code_to_status_form")(pc, videoField1 ? 1 : 0, o),
    readTimeCode,
  );
}

/** The short name (the name plus 20H) of a time code field 01H to 1FH, or null. */
export function mmcShortName(field: number): number | null {
  return scoped([1], (o) => (fn("zmidi_mmc_short_name")(int("field", field, 255), o) === Status.OK ? u8(o) : null));
}

/** The time code field of a short name 21H to 3FH, or null. */
export function mmcFullName(short: number): number | null {
  return scoped([1], (o) => (fn("zmidi_mmc_full_name")(int("short", short, 255), o) === Status.OK ? u8(o) : null));
}

/** The kind of an MMC command (RP-013 pp. 20 to 32). */
export type MmcCommandKind =
  | "stop"
  | "play"
  | "deferredPlay"
  | "fastForward"
  | "rewind"
  | "recordStrobe"
  | "recordExit"
  | "pause"
  | "reset"
  | "write"
  | "locateField"
  | "locateTarget"
  | "move"
  | "other";
const COMMAND_KINDS: readonly MmcCommandKind[] = [
  "stop",
  "play",
  "deferredPlay",
  "fastForward",
  "rewind",
  "recordStrobe",
  "recordExit",
  "pause",
  "reset",
  "write",
  "locateField",
  "locateTarget",
  "move",
  "other",
];

/** One MMC command: `data` is the Information Field string of write, or the data of other. */
export interface MmcCommand {
  readonly kind: MmcCommandKind;
  readonly register?: number;
  readonly destination?: number;
  readonly source?: number;
  readonly target?: TimeCode | null;
  readonly name?: Uint8Array;
  readonly data?: Uint8Array;
}

/** The kind of an MMC Information Field. */
export type MmcFieldKind = "timeCode" | "short" | "other";
const FIELD_KINDS: readonly MmcFieldKind[] = ["timeCode", "short", "other"];

/** One MMC Information Field; of short, `code` has only the frames, the sign and the final byte. */
export interface MmcField {
  readonly kind: MmcFieldKind;
  readonly field?: number;
  readonly code?: TimeCode | null;
  readonly name?: Uint8Array;
  readonly data?: Uint8Array;
}

/** An MMC command or response message. */
export interface MmcMessage {
  readonly response: boolean;
  readonly deviceId: number;
  readonly commands: MmcCommand[];
  readonly fields: MmcField[];
}

const MMC_MESSAGE = layout([
  ["response", "bool"],
  ["device_id", "u8"],
  ["string_offset", "u32"],
  ["string_length", "u32"],
]);
const MMC_COMMAND = layout([
  ["kind", "u32"],
  ["register", "u8"],
  ["destination", "u8"],
  ["source", "u8"],
  ["target", TIME_CODE],
  ["name_offset", "u32"],
  ["name_length", "u32"],
  ["data_offset", "u32"],
  ["data_length", "u32"],
]);
const MMC_FIELD = layout([
  ["kind", "u32"],
  ["field", "u8"],
  ["code", TIME_CODE],
  ["name_offset", "u32"],
  ["name_length", "u32"],
  ["data_offset", "u32"],
  ["data_length", "u32"],
]);

/** Reads each item of a string with a C `next` function. */
function items<T>(
  bytes: Uint8Array,
  size: number,
  what: string,
  next: (p: number, n: number, offset: number, out: number) => number,
  get: (out: number, part: (off: number, len: number) => Uint8Array) => T,
): T[] {
  return withBytes(bytes, (p) =>
    scoped([4, size], (offset, out) => {
      view().setUint32(offset, 0, true);
      const r: T[] = [];
      for (;;) {
        const status = next(p, bytes.length, offset, out);
        if (status === Status.NO_PENDING) {
          return r;
        }
        if (status === Status.INVALID_ARGUMENT) {
          throw new MalformedError(`${what}: a string cut short`);
        }
        check(status, what);
        r.push(
          get(out, (off, len) => {
            const at = view().getUint32(out + off, true);
            return bytes.slice(at, at + view().getUint32(out + len, true));
          }),
        );
      }
    }),
  );
}

function readCommands(bytes: Uint8Array): MmcCommand[] {
  const o = (f: string): number => MMC_COMMAND.off[f] ?? 0;
  return items(
    bytes,
    MMC_COMMAND.size,
    "readMmc",
    (p, n, off, out) => fn("zmidi_mmc_next_command")(p, n, off, out),
    (out, part) => {
      const kind = COMMAND_KINDS[view().getUint32(out + o("kind"), true)] ?? "other";
      return {
        kind,
        register: u8(out + o("register")),
        destination: u8(out + o("destination")),
        source: u8(out + o("source")),
        target: kind === "locateTarget" ? readTimeCode(out + o("target")) : null,
        name: kind === "other" ? part(o("name_offset"), o("name_length")) : new Uint8Array(),
        data: kind === "write" || kind === "other" ? part(o("data_offset"), o("data_length")) : new Uint8Array(),
      };
    },
  );
}

/** Reads the Information Fields of a response string or of the data of WRITE. */
export function readMmcFields(bytes: Uint8Array): MmcField[] {
  const o = (f: string): number => MMC_FIELD.off[f] ?? 0;
  return items(
    bytes,
    MMC_FIELD.size,
    "readMmcFields",
    (p, n, off, out) => fn("zmidi_mmc_next_field")(p, n, off, out),
    (out, part) => {
      const kind = FIELD_KINDS[view().getUint32(out + o("kind"), true)] ?? "other";
      return {
        kind,
        field: u8(out + o("field")),
        code: kind === "other" ? null : readTimeCode(out + o("code")),
        name: part(o("name_offset"), o("name_length")),
        data: part(o("data_offset"), o("data_length")),
      };
    },
  );
}

/** Reads an MMC message, or gives null; a string cut short throws MalformedError. */
export function readMmc(data: Uint8Array): MmcMessage | null {
  return scoped([MMC_MESSAGE.size], (m) =>
    read(
      data,
      "readMmc",
      (p, n) => fn("zmidi_mmc_read")(p, n, m),
      () => {
        const o = (f: string): number => m + (MMC_MESSAGE.off[f] ?? 0);
        const at = view().getUint32(o("string_offset"), true);
        const string = data.slice(at, at + view().getUint32(o("string_length"), true));
        const response = bool(o("response"));
        return {
          response,
          deviceId: u8(o("device_id")),
          commands: response ? [] : readCommands(string),
          fields: response ? readMmcFields(string) : [],
        };
      },
    ),
  );
}

/** The data bytes that the offsets of the C items point into, and a function that places one part. */
function dataOf(): { bytes: number[]; put: (b: Uint8Array | undefined) => [number, number] } {
  const bytes: number[] = [];
  return {
    bytes,
    put: (b) => {
      const at = bytes.length;
      bytes.push(...(b ?? []));
      return [at, b?.length ?? 0];
    },
  };
}

/** Writes C items into an array block and calls a C writer with them and their data. */
function writeItems<T>(
  list: readonly T[],
  size: number,
  what: string,
  put: (p: number, item: T, data: ReturnType<typeof dataOf>) => void,
  call: (items: number, count: number, data: number, dataLength: number, out: number, length: number) => number,
): Uint8Array {
  const d = dataOf();
  return scoped([Math.max(1, list.length) * size], (pi) => {
    new Uint8Array(view().buffer, pi, Math.max(1, list.length) * size).fill(0);
    list.forEach((x, i) => put(pi + i * size, x, d));
    return withBytes(Uint8Array.from(d.bytes), (pd) =>
      written(what, (out, len) => call(pi, list.length, pd, d.bytes.length, out, len)),
    );
  });
}

function putCommand(p: number, c: MmcCommand, d: ReturnType<typeof dataOf>): void {
  const v = view();
  const o = (f: string): number => p + (MMC_COMMAND.off[f] ?? 0);
  const k = COMMAND_KINDS.indexOf(c.kind);
  v.setUint32(o("kind"), k < 0 ? 0xffff : k, true);
  v.setUint8(o("register"), int("register", c.register ?? 0, 255));
  v.setUint8(o("destination"), int("destination", c.destination ?? 0, 255));
  v.setUint8(o("source"), int("source", c.source ?? 0, 255));
  if (c.target) {
    writeTimeCode(o("target"), c.target);
  }
  const [no, nl] = d.put(c.name);
  const [dof, dl] = d.put(c.data);
  v.setUint32(o("name_offset"), no, true);
  v.setUint32(o("name_length"), nl, true);
  v.setUint32(o("data_offset"), dof, true);
  v.setUint32(o("data_length"), dl, true);
}

function putField(p: number, f: MmcField, d: ReturnType<typeof dataOf>): void {
  const v = view();
  const o = (x: string): number => p + (MMC_FIELD.off[x] ?? 0);
  const k = FIELD_KINDS.indexOf(f.kind);
  v.setUint32(o("kind"), k < 0 ? 0xffff : k, true);
  v.setUint8(o("field"), int("field", f.field ?? 0, 255));
  if (f.code) {
    writeTimeCode(o("code"), f.code);
  }
  const [no, nl] = d.put(f.name);
  const [dof, dl] = d.put(f.data);
  v.setUint32(o("name_offset"), no, true);
  v.setUint32(o("name_length"), nl, true);
  v.setUint32(o("data_offset"), dof, true);
  v.setUint32(o("data_length"), dl, true);
}

/** The bytes of an MMC command message to `deviceId` (7FH: all devices). */
export function mmcCommands(deviceId: number, commands: readonly MmcCommand[]): Uint8Array {
  const dev = int("deviceId", deviceId, 255);
  return writeItems(commands, MMC_COMMAND.size, "mmcCommands", putCommand, (pi, n, pd, dl, out, len) =>
    fn("zmidi_mmc_write_commands")(dev, pi, n, pd, dl, out, OUT, len),
  );
}

/** The bytes of an MMC response message from `deviceId` (00H to 7EH). */
export function mmcResponses(deviceId: number, fields: readonly MmcField[]): Uint8Array {
  const dev = int("deviceId", deviceId, 255);
  return writeItems(fields, MMC_FIELD.size, "mmcResponses", putField, (pi, n, pd, dl, out, len) =>
    fn("zmidi_mmc_write_responses")(dev, pi, n, pd, dl, out, OUT, len),
  );
}

/** The name and data bytes of Information Fields, for the data of a WRITE command. */
export function mmcFields(fields: readonly MmcField[]): Uint8Array {
  return writeItems(fields, MMC_FIELD.size, "mmcFields", putField, (pi, n, pd, dl, out, len) =>
    fn("zmidi_mmc_write_fields")(pi, n, pd, dl, out, OUT, len),
  );
}

// --- notation ---

/** The Bar Marker bar number Not Running, -8192 (M1 Notation Information). */
export const BAR_NOT_RUNNING = -8192;
/** The Bar Marker bar number Running with an unknown bar number, 8191. */
export const BAR_UNKNOWN = 8191;

/** One Bar Marker message: the signed bar number, -8192 to 8191; -8191 to 0 are count-in bars. */
export interface BarMarker {
  readonly deviceId: number;
  readonly bar: number;
}

/** Reads a Bar Marker, or gives null. */
export function readBarMarker(data: Uint8Array): BarMarker | null {
  return scoped([1, 2], (dev, bar) =>
    read(
      data,
      "readBarMarker",
      (p, n) => fn("zmidi_bar_marker_read")(p, n, dev, bar),
      () => ({ deviceId: u8(dev), bar: view().getInt16(bar, true) }),
    ),
  );
}

/** The bytes of a Bar Marker. */
export function barMarkerBytes(marker: BarMarker): Uint8Array {
  const { bar } = marker;
  if (!Number.isInteger(bar) || bar < BAR_NOT_RUNNING || bar > BAR_UNKNOWN) {
    throw new InvalidValueError(`bar must be an integer from -8192 to 8191, not ${bar}`);
  }
  return written("barMarkerBytes", (out, len) =>
    fn("zmidi_bar_marker_write")(int("deviceId", marker.deviceId, 255), bar, out, OUT, len),
  );
}

/**
 * One Time Signature message. `delayed` is the Delayed form (42H), which takes effect at the next Bar
 * Marker; `denominatorPower` is a negative power of 2 (2 = quarter note); `additional` holds the
 * numerator and denominator-power pairs of a compound time signature, at most 61 pairs.
 */
export interface TimeSignature {
  readonly deviceId: number;
  readonly delayed: boolean;
  readonly numerator: number;
  readonly denominatorPower: number;
  readonly clocksPerClick: number;
  readonly thirtySecondsPerQuarter: number;
  readonly additional: Uint8Array;
}

const TIME_SIGNATURE = layout([
  ["device_id", "u8"],
  ["delayed", "bool"],
  ["numerator", "u8"],
  ["denominator_power", "u8"],
  ["clocks_per_click", "u8"],
  ["thirty_seconds_per_quarter", "u8"],
  ["additional_offset", "u32"],
  ["additional_length", "u32"],
]);

/** Reads a Time Signature, Immediate or Delayed, or gives null. */
export function readTimeSignature(data: Uint8Array): TimeSignature | null {
  return scoped([TIME_SIGNATURE.size], (t) =>
    read(
      data,
      "readTimeSignature",
      (p, n) => fn("zmidi_time_signature_read")(p, n, t),
      () => {
        const v = view();
        const o = (k: string): number => t + (TIME_SIGNATURE.off[k] ?? 0);
        const at = v.getUint32(o("additional_offset"), true);
        return {
          deviceId: u8(o("device_id")),
          delayed: bool(o("delayed")),
          numerator: u8(o("numerator")),
          denominatorPower: u8(o("denominator_power")),
          clocksPerClick: u8(o("clocks_per_click")),
          thirtySecondsPerQuarter: u8(o("thirty_seconds_per_quarter")),
          additional: data.slice(at, at + v.getUint32(o("additional_length"), true)),
        };
      },
    ),
  );
}

/** The additional pairs of a Time Signature, `[numerator, denominatorPower]`. */
export function timeSignaturePairs(signature: TimeSignature): [number, number][] {
  const a = signature.additional;
  return Array.from({ length: a.length >> 1 }, (_, i) => [a[2 * i] ?? 0, a[2 * i + 1] ?? 0]);
}

/** The bytes of a Time Signature. */
export function timeSignatureBytes(signature: TimeSignature): Uint8Array {
  const s = signature;
  const args = [
    int("deviceId", s.deviceId, 255),
    s.delayed ? 1 : 0,
    int("numerator", s.numerator, 255),
    int("denominatorPower", s.denominatorPower, 255),
    int("clocksPerClick", s.clocksPerClick, 255),
    int("thirtySecondsPerQuarter", s.thirtySecondsPerQuarter, 255),
  ] as const;
  return withBytes(s.additional, (pa) =>
    written("timeSignatureBytes", (out, len) =>
      fn("zmidi_time_signature_write")(...args, pa, s.additional.length, out, OUT, len),
    ),
  );
}
