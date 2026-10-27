// UMP functions (zmidi-ump through the C ABI): the word count of M2-104 Table 4, a parse into plain
// fields, writers of the MIDI 2.0 Channel Voice Messages (M2-104 7.4), the MIDI 2.0 to 1.0
// translation (M2-104 Appendix D.2) and the scaling of M2-115.

import { InvalidValueError, MalformedError } from "./errors.js";
import { Status, check, fn, int, layout, readWords, scoped, umpWords, view, writeWords } from "./wasm.js";

/** One UMP as plain fields. */
export interface Ump {
  /** Message Type, 0x0 to 0xF. */
  readonly messageType: number;
  /** Group, 0 to 15, or null when the Message Type has no Group. */
  readonly group: number | null;
  /** The opcode or status nibble (Message Types 0x0, 0x2 to 0x5) or the status byte (0x1), or null. */
  readonly status: number | null;
  /** Channel, 0 to 15, or null when the Message Type has no Channel. */
  readonly channel: number | null;
  /** True when this build decodes the Message Type. */
  readonly decoded: boolean;
  /** The words of the UMP. */
  readonly words: readonly number[];
}

const INFO = layout([
  ["message_type", "u8"],
  ["word_count", "u8"],
  ["has_group", "bool"],
  ["group", "u8"],
  ["has_status", "bool"],
  ["status", "u8"],
  ["has_channel", "bool"],
  ["channel", "u8"],
  ["decoded", "bool"],
  ["words", ["u32", 4]],
]);

/** The word count of a UMP from its first word (M2-104 Table 4). */
export function wordCount(word0: number): number {
  return fn("zmidi_ump_word_count")(int("word0", word0, 0xffffffff)) >>> 0;
}

/** Reads the first UMP of `words` into plain fields. Throws MalformedError when `words` is too short. */
export function parse(words: readonly number[]): Ump {
  if (words.length === 0) {
    throw new MalformedError("no words");
  }
  const n = wordCount(words[0]!);
  if (words.length < n) {
    throw new MalformedError(`the Message Type needs ${n} words, not ${words.length}`);
  }
  const w = umpWords(words.slice(0, n));
  return scoped([16, INFO.size], (pw, pi) => {
    writeWords(pw, w);
    check(fn("zmidi_ump_parse")(pw, n, pi), "parse");
    const v = view();
    const u8 = (f: string): number => v.getUint8(pi + INFO.off[f]!);
    const opt = (has: string, f: string): number | null => (u8(has) !== 0 ? u8(f) : null);
    return {
      messageType: u8("message_type"),
      group: opt("has_group", "group"),
      status: opt("has_status", "status"),
      channel: opt("has_channel", "channel"),
      decoded: u8("decoded") !== 0,
      words: readWords(pi + INFO.off["words"]!, u8("word_count")),
    };
  });
}

function writer(name: string, args: number[]): number[] {
  return scoped([8], (out) => {
    check(fn(name)(...args, out), name);
    return readWords(out, 2);
  });
}

function gc(group: number, channel: number): [number, number] {
  return [int("group", group, 15), int("channel", channel, 15)];
}

/** MIDI 2.0 Note On (M2-104 7.4.2): velocity 16 bits. */
export function midi2NoteOn(group: number, channel: number, note: number, velocity: number): number[] {
  return writer("zmidi_ump_midi2_note_on", [...gc(group, channel), int("note", note, 127), int("velocity", velocity, 0xffff)]);
}

/** MIDI 2.0 Note Off (M2-104 7.4.1): velocity 16 bits. */
export function midi2NoteOff(group: number, channel: number, note: number, velocity: number): number[] {
  return writer("zmidi_ump_midi2_note_off", [...gc(group, channel), int("note", note, 127), int("velocity", velocity, 0xffff)]);
}

/** MIDI 2.0 Control Change (M2-104 7.4.6): index 7 bits, data 32 bits. */
export function midi2ControlChange(group: number, channel: number, index: number, data: number): number[] {
  return writer("zmidi_ump_midi2_control_change", [...gc(group, channel), int("index", index, 127), int("data", data, 0xffffffff)]);
}

/** MIDI 2.0 Program Change (M2-104 7.4.9), with the Bank `[msb, lsb]` when given. */
export function midi2ProgramChange(group: number, channel: number, program: number, bank?: readonly [number, number]): number[] {
  const [msb, lsb] = bank ?? [0, 0];
  return writer("zmidi_ump_midi2_program_change", [
    ...gc(group, channel),
    int("program", program, 127),
    bank ? 1 : 0,
    int("bank msb", msb, 127),
    int("bank lsb", lsb, 127),
  ]);
}

/** MIDI 2.0 Channel Pressure (M2-104 7.4.10): value 32 bits. */
export function midi2ChannelPressure(group: number, channel: number, value: number): number[] {
  return writer("zmidi_ump_midi2_channel_pressure", [...gc(group, channel), int("value", value, 0xffffffff)]);
}

/** MIDI 2.0 Pitch Bend (M2-104 7.4.11): value 32 bits, 0x80000000 is the center. */
export function midi2PitchBend(group: number, channel: number, value: number): number[] {
  return writer("zmidi_ump_midi2_pitch_bend", [...gc(group, channel), int("value", value, 0xffffffff)]);
}

/**
 * Translates one MIDI 2.0 Channel Voice UMP to MIDI 1.0 Channel Voice UMPs (M2-104 Appendix D.2).
 * Gives null for a UMP that has no translation.
 */
export function midi2ToMidi1(words: readonly number[]): number[] | null {
  const w = umpWords(words);
  return scoped([16, 64, 4], (pw, out, count) => {
    writeWords(pw, w);
    const st = fn("zmidi_ump_midi2_to_midi1")(pw, w.length, out, 16, count);
    if (st === Status.NOT_SUPPORTED) {
      return null;
    }
    check(st, "midi2ToMidi1");
    return readWords(out, view().getUint32(count, true));
  });
}

function scale(name: string, value: number, srcBits: number, dstBits: number): number {
  int("srcBits", srcBits, 32);
  int("dstBits", dstBits, 32);
  if (srcBits === 0 || dstBits === 0) {
    throw new InvalidValueError("a bit count is 1 to 32");
  }
  int("value", value, 2 ** srcBits - 1);
  return scoped([4], (out) => {
    check(fn(name)(value, srcBits, dstBits, out), name);
    return view().getUint32(out, true);
  });
}

/** Scales `value` of `srcBits` bits up to `dstBits` bits (M2-115, min-center-max). */
export function upscale(value: number, srcBits: number, dstBits: number): number {
  return scale("zmidi_ump_upscale", value, srcBits, dstBits);
}

/** Scales `value` of `srcBits` bits down to `dstBits` bits (M2-115, a shift). */
export function downscale(value: number, srcBits: number, dstBits: number): number {
  return scale("zmidi_ump_downscale", value, srcBits, dstBits);
}
