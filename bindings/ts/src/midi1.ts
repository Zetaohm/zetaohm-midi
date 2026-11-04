// MIDI 1.0 typed messages on bytes and the MIDI 1.0 receiver states (zmidi-io through the C ABI,
// module Midi1 of zmidi-ffi): M1 v4.2.1, RP-015, RP-018. A message has the bytes as on the wire and no
// UMP Group.

import { InvalidValueError } from "./errors.js";
import {
  Blocks,
  alloc,
  allocBytes,
  check,
  fn,
  free,
  int,
  layout,
  readBytes,
  scoped,
  sizeOf,
  view,
} from "./wasm.js";

/** The kind of a MIDI 1.0 message. */
export type Midi1Kind = "channelVoice" | "system" | "sysex" | "sysexTooLong";

/** One MIDI 1.0 message with the bytes as on the wire, and no UMP Group. */
export interface Midi1Message {
  readonly kind: Midi1Kind;
  /** The status byte; F0H for a System Exclusive message. */
  readonly status: number;
  /** The first data byte, or 0. */
  readonly data1: number;
  /** The second data byte, or 0. */
  readonly data2: number;
  /** The bytes of a System Exclusive message, F0H to F7H; empty for the other kinds. */
  readonly data: Uint8Array;
  /** The length of a System Exclusive message (also when it was too long for the reader). */
  readonly length: number;
}

const KINDS: readonly Midi1Kind[] = ["channelVoice", "channelVoice", "system", "sysex", "sysexTooLong"];

const MESSAGE = layout([
  ["kind", "u32"],
  ["status", "u8"],
  ["data1", "u8"],
  ["data2", "u8"],
  ["length", "u32"],
]);

const EMPTY = new Uint8Array(0);

/**
 * A Channel Voice (status 80H to EFH) or System (F1H to FFH, not F7H) message. `data1` and `data2`
 * are 0 to 127; a byte that the message does not have is 0.
 */
export function midi1Message(status: number, data1 = 0, data2 = 0): Midi1Message {
  int("status", status, 255);
  int("data1", data1, 127);
  int("data2", data2, 127);
  if (status < 0x80 || status === 0xf0 || status === 0xf7) {
    throw new InvalidValueError("status must be 80H to EFH or F1H to FFH (not F7H); use sysexMessage");
  }
  return { kind: status < 0xf0 ? "channelVoice" : "system", status, data1, data2, data: EMPTY, length: 0 };
}

/** A System Exclusive message: F0H, data bytes 0 to 127, F7H. */
export function sysexMessage(data: Uint8Array): Midi1Message {
  const ok = data.length >= 2 && data[0] === 0xf0 && data[data.length - 1] === 0xf7 && data.subarray(1, -1).every((b) => b < 0x80);
  if (!ok) {
    throw new InvalidValueError("a System Exclusive message is F0H, data bytes 0 to 127, F7H");
  }
  return { kind: "sysex", status: 0xf0, data1: 0, data2: 0, data: data.slice(), length: data.length };
}

/** Writes the C form of `m` at `ptr`. */
function writeMessage(ptr: number, m: Midi1Message): void {
  const v = view();
  v.setUint32(ptr + MESSAGE.off.kind, Math.max(0, KINDS.indexOf(m.kind, 1)), true);
  v.setUint8(ptr + MESSAGE.off.status, m.status & 0xff);
  v.setUint8(ptr + MESSAGE.off.data1, m.data1 & 0xff);
  v.setUint8(ptr + MESSAGE.off.data2, m.data2 & 0xff);
  v.setUint32(ptr + MESSAGE.off.length, m.length, true);
}

/** Reads the C message at `ptr`; the bytes of a System Exclusive message are at `sysex`. */
function readMessage(ptr: number, sysex = 0): Midi1Message {
  const v = view();
  const kind = KINDS[v.getUint32(ptr + MESSAGE.off.kind, true)] ?? "channelVoice";
  const length = v.getUint32(ptr + MESSAGE.off.length, true);
  return {
    kind,
    status: v.getUint8(ptr + MESSAGE.off.status),
    data1: v.getUint8(ptr + MESSAGE.off.data1),
    data2: v.getUint8(ptr + MESSAGE.off.data2),
    data: kind === "sysex" ? readBytes(sysex, length) : EMPTY,
    length,
  };
}

/** Reads `count` C messages from `ptr`. */
function readMessages(ptr: number, count: number): Midi1Message[] {
  return Array.from({ length: count }, (_, i) => readMessage(ptr + i * MESSAGE.size));
}

/** Calls a C writer of `max` messages and gives them. */
function written(max: number, call: (out: number, count: number) => number, what: string): Midi1Message[] {
  return scoped([max * MESSAGE.size, 4], (out, count) => {
    check(call(out, count), what);
    return readMessages(out, view().getUint32(count, true));
  });
}

/** Reads a MIDI 1.0 byte stream and gives typed messages; a System Exclusive message comes whole. */
export class Midi1Reader {
  readonly #b = new Blocks(this);
  readonly #sysex: number;
  readonly #capacity: number;

  /**
   * A reader that holds System Exclusive messages of up to `sysexCapacity` bytes (F0H to F7H); a
   * longer one gives the kind "sysexTooLong" with its length.
   */
  constructor(sysexCapacity = 256) {
    const p = this.#b.add(alloc(sizeOf("zmidi_midi1_reader_size")));
    check(fn("zmidi_midi1_reader_init")(p), "Midi1Reader");
    this.#capacity = int("sysexCapacity", sysexCapacity, 0x7fffffff);
    this.#sysex = this.#b.add(alloc(Math.max(1, this.#capacity)));
  }

  /** Reads one byte; gives the messages that it completes (at most two). */
  push(byte: number): Midi1Message[] {
    int("byte", byte, 255);
    const [out, count] = this.#b.carve([2 * MESSAGE.size, 4]);
    check(fn("zmidi_midi1_reader_push")(this.#b.main, byte, this.#sysex, this.#capacity, out, 2, count), "push");
    const n = view().getUint32(count, true);
    return Array.from({ length: n }, (_, i) => readMessage(out + i * MESSAGE.size, this.#sysex));
  }

  /** Reads bytes; gives all the messages that they complete, in order. */
  feed(data: Uint8Array): Midi1Message[] {
    const out: Midi1Message[] = [];
    for (const byte of data) {
      out.push(...this.push(byte));
    }
    return out;
  }

  /** The System Reset action of the program (AMB-M1-13): no running status, no partial message. */
  reset(): void {
    check(fn("zmidi_midi1_reader_reset")(this.#b.main), "reset");
  }

  /** Frees the WASM memory of the reader now. */
  free(): void {
    this.#b.release();
  }
}

/** Writes typed MIDI 1.0 messages as bytes. */
export class Midi1Writer {
  readonly #b = new Blocks(this);

  /** A writer; `runningStatus` leaves out a status byte equal to the last one. */
  constructor(runningStatus = false) {
    const p = this.#b.add(alloc(sizeOf("zmidi_midi1_writer_size")));
    check(fn("zmidi_midi1_writer_init")(p, runningStatus ? 1 : 0), "Midi1Writer");
  }

  /** Gives the bytes of `message`. */
  write(message: Midi1Message): Uint8Array {
    const sysex = message.kind === "sysex" ? allocBytes(message.data) : 0;
    try {
      const cap = Math.max(4, message.length);
      const [pm, out, count] = this.#b.carve([MESSAGE.size, cap, 4]);
      writeMessage(pm, message);
      check(fn("zmidi_midi1_writer_write")(this.#b.main, pm, sysex, out, cap, count), "write");
      return readBytes(out, view().getUint32(count, true));
    } finally {
      if (sysex !== 0) {
        free(sysex);
      }
    }
  }

  /** Makes the next Channel Voice message send its status byte (a refresh of running status). */
  refreshStatus(): void {
    check(fn("zmidi_midi1_writer_refresh_status")(this.#b.main), "refreshStatus");
  }

  /** Frees the WASM memory of the writer now. */
  free(): void {
    this.#b.release();
  }
}

/** The kind of a mode action. */
export type ModeActionKind =
  | "ignore"
  | "voice"
  | "global"
  | "allNotesOff"
  | "allSoundOff"
  | "resetAllControllers"
  | "localControl"
  | "modeChange";

/** The action of one received message for a mode state. */
export interface ModeAction {
  readonly kind: ModeActionKind;
  /** The channel of a Mode 4 action, or null for all channels of the instrument. */
  readonly channel: number | null;
  /** The Mode 4 voice of a voice message, or null. */
  readonly voice: number | null;
  /** Local Control on. */
  readonly on: boolean;
  /** The mode message changed the mode. */
  readonly changed: boolean;
  /** The mode in use is an alternate mode. */
  readonly alternate: boolean;
}

/** The configuration of a mode state. */
export interface ModeStateOptions {
  /** The Basic Channel, 0 to 15 (default 0). */
  readonly basicChannel?: number;
  /** The mode numbers that the instrument implements (default 1 to 4; Mode 1 always). */
  readonly implemented?: readonly number[];
  /** A mode that is not implemented gives the alternate mode of p. 23 (default), else it is ignored. */
  readonly alternate?: boolean;
  /** The voice count for M = 0 (default 16). */
  readonly voices?: number;
  /** The channel below the Basic Channel carries the Global Controllers in Mode 4 (default false). */
  readonly globalControllers?: boolean;
}

const MODE_KINDS: readonly ModeActionKind[] = [
  "ignore",
  "voice",
  "global",
  "allNotesOff",
  "allSoundOff",
  "resetAllControllers",
  "localControl",
  "modeChange",
];

const MODE_CONFIG = layout([
  ["basic_channel", "u8"],
  ["implemented", "u8"],
  ["alternate", "bool"],
  ["voices", "u8"],
  ["global_controllers", "bool"],
]);

const MODE_ACTION = layout([
  ["kind", "u32"],
  ["channel", "u8"],
  ["voice", "u8"],
  ["on", "bool"],
  ["changed", "bool"],
  ["alternate", "bool"],
]);

const MODE_INFO = layout([
  ["mode", "u8"],
  ["requested_mode", "u8"],
  ["alternate", "bool"],
  ["basic_channel", "u8"],
  ["m", "u8"],
  ["local", "bool"],
]);

/** The MIDI 1.0 receiver mode state of one instrument (Modes 1 to 4, M1 pp. 20 to 26). */
export class ModeState {
  readonly #b = new Blocks(this);

  /** A mode state in Mode 1 with Local Control on. */
  constructor(options: ModeStateOptions = {}) {
    let mask = 0;
    for (const n of options.implemented ?? [1, 2, 3, 4]) {
      if (int("implemented", n, 4) < 1) {
        throw new InvalidValueError("mode numbers are 1 to 4");
      }
      mask |= 1 << (n - 1);
    }
    const p = this.#b.add(alloc(sizeOf("zmidi_mode_state_size")));
    scoped([MODE_CONFIG.size], (c) => {
      const v = view();
      v.setUint8(c + MODE_CONFIG.off.basic_channel, int("basicChannel", options.basicChannel ?? 0, 15));
      v.setUint8(c + MODE_CONFIG.off.implemented, mask);
      v.setUint8(c + MODE_CONFIG.off.alternate, (options.alternate ?? true) ? 1 : 0);
      v.setUint8(c + MODE_CONFIG.off.voices, int("voices", options.voices ?? 16, 255));
      v.setUint8(c + MODE_CONFIG.off.global_controllers, options.globalControllers ? 1 : 0);
      check(fn("zmidi_mode_init")(p, c), "ModeState");
    });
  }

  /** Takes one Channel Voice message; gives its action. */
  receive(message: Midi1Message): ModeAction {
    const [pm, pa] = this.#b.carve([MESSAGE.size, MODE_ACTION.size]);
    writeMessage(pm, message);
    check(fn("zmidi_mode_receive")(this.#b.main, pm, pa), "receive");
    const v = view();
    const channel = v.getUint8(pa + MODE_ACTION.off.channel);
    const voice = v.getUint8(pa + MODE_ACTION.off.voice);
    return {
      kind: MODE_KINDS[v.getUint32(pa + MODE_ACTION.off.kind, true)] ?? "ignore",
      channel: channel === 0xff ? null : channel,
      voice: voice === 0xff ? null : voice,
      on: v.getUint8(pa + MODE_ACTION.off.on) !== 0,
      changed: v.getUint8(pa + MODE_ACTION.off.changed) !== 0,
      alternate: v.getUint8(pa + MODE_ACTION.off.alternate) !== 0,
    };
  }

  /** The System Reset action: Omni On, Poly and Local On; the Basic Channel and M stay. */
  systemReset(): void {
    check(fn("zmidi_mode_system_reset")(this.#b.main), "systemReset");
  }

  #info(field: keyof typeof MODE_INFO.off): number {
    return scoped([MODE_INFO.size], (p) => {
      check(fn("zmidi_mode_info")(this.#b.main, p), "info");
      return view().getUint8(p + (MODE_INFO.off[field] ?? 0));
    });
  }

  /** The mode in use, 1 to 4. */
  get mode(): number {
    return this.#info("mode");
  }

  /** The mode of the Omni and Mono flags, 1 to 4. */
  get requestedMode(): number {
    return this.#info("requested_mode");
  }

  /** The mode in use is an alternate mode. */
  get isAlternate(): boolean {
    return this.#info("alternate") !== 0;
  }

  /** The Basic Channel, 0 to 15. */
  get basicChannel(): number {
    return this.#info("basic_channel");
  }

  set basicChannel(channel: number) {
    check(fn("zmidi_mode_set_basic_channel")(this.#b.main, int("basicChannel", channel, 15)), "basicChannel");
  }

  /** M of the last Mono On. */
  get m(): number {
    return this.#info("m");
  }

  /** Local Control on. */
  get local(): boolean {
    return this.#info("local") !== 0;
  }

  set local(on: boolean) {
    check(fn("zmidi_mode_set_local")(this.#b.main, on ? 1 : 0), "local");
  }

  /** Sets the Omni flag, as Omni On or Omni Off does. */
  setOmni(on: boolean): void {
    check(fn("zmidi_mode_set_omni")(this.#b.main, on ? 1 : 0), "setOmni");
  }

  /** Selects Poly, as Poly On does. */
  setPoly(): void {
    check(fn("zmidi_mode_set_poly")(this.#b.main), "setPoly");
  }

  /** Selects Mono with M (0 to 127), as Mono On does. */
  setMono(m: number): void {
    check(fn("zmidi_mode_set_mono")(this.#b.main, int("m", m, 127)), "setMono");
  }

  /** Frees the WASM memory of the state now. */
  free(): void {
    this.#b.release();
  }
}

/** The kind of a controller event. */
export type ControllerEventKind = "controller14" | "controller7" | "program" | "portamento" | "resetAllControllers";

/** The event of one received message for a controller state; a field is null when the kind has none. */
export interface ControllerEvent {
  readonly kind: ControllerEventKind;
  /** The controller number (0 to 31 for "controller14", 64 to 119 for "controller7"). */
  readonly controller: number | null;
  /** The 14-bit or 7-bit controller value. */
  readonly value: number | null;
  /** The program number. */
  readonly program: number | null;
  /** The 14-bit bank of a program event, or null before the first Bank Select. */
  readonly bank: number | null;
  /** The Portamento Control source note. */
  readonly source: number | null;
}

const CONTROLLER_KINDS: readonly (ControllerEventKind | null)[] = [
  null,
  "controller14",
  "controller7",
  "program",
  "portamento",
  "resetAllControllers",
];

const CONTROLLER_EVENT = layout([
  ["kind", "u32"],
  ["controller", "u8"],
  ["program", "u8"],
  ["source", "u8"],
  ["has_bank", "bool"],
  ["value", "u16"],
  ["bank", "u16"],
]);

/**
 * The MIDI 1.0 controller state of one channel: 14-bit pairs, bank and program, Portamento Control,
 * Reset All Controllers (M1 pp. 11 to 17, RP-015).
 */
export class ControllerState {
  readonly #b = new Blocks(this);

  constructor() {
    const p = this.#b.add(alloc(sizeOf("zmidi_controller_state_size")));
    check(fn("zmidi_controllers_init")(p), "ControllerState");
  }

  /** Takes one Channel Voice message of the channel of this state; gives its event, or null. */
  receive(message: Midi1Message): ControllerEvent | null {
    const [pm, pe] = this.#b.carve([MESSAGE.size, CONTROLLER_EVENT.size]);
    writeMessage(pm, message);
    check(fn("zmidi_controllers_receive")(this.#b.main, pm, pe), "receive");
    const v = view();
    const kind = CONTROLLER_KINDS[v.getUint32(pe + CONTROLLER_EVENT.off.kind, true)] ?? null;
    if (kind === null) {
      return null;
    }
    const u8 = (f: string): number => v.getUint8(pe + (CONTROLLER_EVENT.off[f] ?? 0));
    const u16 = (f: string): number => v.getUint16(pe + (CONTROLLER_EVENT.off[f] ?? 0), true);
    const controller = kind === "controller14" || kind === "controller7";
    return {
      kind,
      controller: controller ? u8("controller") : null,
      value: controller ? u16("value") : null,
      program: kind === "program" ? u8("program") : null,
      bank: kind === "program" && u8("has_bank") !== 0 ? u16("bank") : null,
      source: kind === "portamento" ? u8("source") : null,
    };
  }

  /** The Reset All Controllers action of the program (a ModeState action or General MIDI System On). */
  resetAllControllers(): void {
    check(fn("zmidi_controllers_reset")(this.#b.main), "resetAllControllers");
  }

  /** The held 14-bit bank, or null before the first Bank Select. */
  get bank(): number | null {
    return scoped([1, 2], (has, bank) => {
      check(fn("zmidi_controllers_bank")(this.#b.main, has, bank), "bank");
      const v = view();
      return v.getUint8(has) !== 0 ? v.getUint16(bank, true) : null;
    });
  }

  /** Frees the WASM memory of the state now. */
  free(): void {
    this.#b.release();
  }
}

/** The RP-015 value of `controller` at Reset All Controllers, or null when it keeps its value. */
export function resetValue(controller: number): number | null {
  return scoped([1], (p) => {
    check(fn("zmidi_controller_reset_value")(int("controller", controller, 255), p), "resetValue");
    const v = view().getUint8(p);
    return v === 0xff ? null : v;
  });
}

/** The two messages of a 14-bit controller value (controller 0 to 31, value 0 to 3FFFH): MSB, then LSB. */
export function controller14Messages(channel: number, controller: number, value: number): Midi1Message[] {
  const args = [int("channel", channel, 15), int("controller", controller, 255), int("value", value, 0xffff)] as const;
  return written(2, (out, count) => fn("zmidi_controller14_messages")(...args, out, 2, count), "controller14Messages");
}

/** The three messages of a bank (0 to 3FFFH) and program (0 to 127) selection. */
export function bankProgramMessages(channel: number, bank: number, program: number): Midi1Message[] {
  const args = [int("channel", channel, 15), int("bank", bank, 0xffff), int("program", program, 255)] as const;
  return written(3, (out, count) => fn("zmidi_bank_program_messages")(...args, out, 3, count), "bankProgramMessages");
}

/** A parameter number; `{ msb: 0x7f, lsb: 0x7f, registered: true }` is the null number. */
export interface Parameter {
  /** The MSB, 0 to 127. */
  readonly msb: number;
  /** The LSB, 0 to 127. */
  readonly lsb: number;
  /** True for a Registered Parameter Number (controllers 101 and 100). */
  readonly registered: boolean;
}

/** The change of a parameter event. */
export type ParameterChange = "dataEntryMsb" | "dataEntryLsb" | "increment" | "decrement";

/** The event of a Data Entry, Data Increment or Data Decrement for the selected parameter. */
export interface ParameterEvent {
  readonly parameter: Parameter;
  readonly change: ParameterChange;
  /** The Data Entry byte, or null for a step. */
  readonly value: number | null;
}

const CHANGES: readonly (ParameterChange | null)[] = [null, "dataEntryMsb", "dataEntryLsb", "increment", "decrement"];

const PARAMETER = layout([
  ["registered", "bool"],
  ["msb", "u8"],
  ["lsb", "u8"],
]);

const PARAMETER_EVENT = layout([
  ["change", "u32"],
  ["parameter", PARAMETER],
  ["value", "u8"],
]);

function writeParameter(p: number, parameter: Parameter): void {
  const v = view();
  v.setUint8(p + PARAMETER.off.registered, parameter.registered ? 1 : 0);
  v.setUint8(p + PARAMETER.off.msb, int("msb", parameter.msb, 255));
  v.setUint8(p + PARAMETER.off.lsb, int("lsb", parameter.lsb, 255));
}

function readParameter(p: number): Parameter {
  const v = view();
  return {
    msb: v.getUint8(p + PARAMETER.off.msb),
    lsb: v.getUint8(p + PARAMETER.off.lsb),
    registered: v.getUint8(p + PARAMETER.off.registered) !== 0,
  };
}

function changeNumber(change: ParameterChange): number {
  const n = CHANGES.indexOf(change);
  if (n < 1) {
    throw new InvalidValueError("change must be dataEntryMsb, dataEntryLsb, increment or decrement");
  }
  return n;
}

/** The MIDI 1.0 RPN and NRPN receiver of one channel (M1 p. 17, RP-015, RP-018); it holds no value. */
export class ParameterState {
  readonly #b = new Blocks(this);

  /** A receiver with RPN reception on and NRPN reception off. */
  constructor() {
    const p = this.#b.add(alloc(sizeOf("zmidi_parameter_state_size")));
    check(fn("zmidi_parameters_init")(p), "ParameterState");
  }

  /** Takes one Channel Voice message of the channel of this receiver; gives its event, or null. */
  receive(message: Midi1Message): ParameterEvent | null {
    const [pm, pe] = this.#b.carve([MESSAGE.size, PARAMETER_EVENT.size]);
    writeMessage(pm, message);
    check(fn("zmidi_parameters_receive")(this.#b.main, pm, pe), "receive");
    const v = view();
    const change = CHANGES[v.getUint32(pe + PARAMETER_EVENT.off.change, true)] ?? null;
    if (change === null) {
      return null;
    }
    const entry = change === "dataEntryMsb" || change === "dataEntryLsb";
    return {
      parameter: readParameter(pe + PARAMETER_EVENT.off.parameter),
      change,
      value: entry ? v.getUint8(pe + PARAMETER_EVENT.off.value) : null,
    };
  }

  /** The Reset All Controllers action of the program: no parameter is selected. */
  resetAllControllers(): void {
    check(fn("zmidi_parameters_reset")(this.#b.main), "resetAllControllers");
  }

  /** The parameter that a Data Entry acts on now, or null. */
  get selection(): Parameter | null {
    return scoped([1, PARAMETER.size], (selected, p) => {
      check(fn("zmidi_parameters_selection")(this.#b.main, selected, p), "selection");
      return view().getUint8(selected) !== 0 ? readParameter(p) : null;
    });
  }

  #reception(): [boolean, boolean] {
    return scoped([1, 1], (rpn, nrpn) => {
      check(fn("zmidi_parameters_reception")(this.#b.main, rpn, nrpn), "reception");
      const v = view();
      return [v.getUint8(rpn) !== 0, v.getUint8(nrpn) !== 0];
    });
  }

  /** RPN reception on. */
  get rpnReception(): boolean {
    return this.#reception()[0];
  }

  set rpnReception(on: boolean) {
    check(fn("zmidi_parameters_set_reception")(this.#b.main, on ? 1 : 0, this.#reception()[1] ? 1 : 0), "rpnReception");
  }

  /** NRPN reception on. */
  get nrpnReception(): boolean {
    return this.#reception()[1];
  }

  set nrpnReception(on: boolean) {
    check(fn("zmidi_parameters_set_reception")(this.#b.main, this.#reception()[0] ? 1 : 0, on ? 1 : 0), "nrpnReception");
  }

  /** Frees the WASM memory of the receiver now. */
  free(): void {
    this.#b.release();
  }
}

/**
 * The new 14-bit value of the parameter of `event` from its `current` value (0 to 16383), with the
 * RP-018 steps: RPN 0 steps the cents with a carry at 100, RPN 2 to 4 step the MSB, the others the
 * 14-bit value; a step stops at the limits.
 */
export function applyParameter(event: ParameterEvent, current: number): number {
  return scoped([PARAMETER_EVENT.size, 2], (pe, out) => {
    writeParameter(pe + PARAMETER_EVENT.off.parameter, event.parameter);
    const v = view();
    v.setUint32(pe + PARAMETER_EVENT.off.change, changeNumber(event.change), true);
    v.setUint8(pe + PARAMETER_EVENT.off.value, int("value", event.value ?? 0, 255));
    check(fn("zmidi_parameter_apply")(pe, int("current", current, 0xffff), out), "applyParameter");
    return view().getUint16(out, true);
  });
}

/** The two messages of a parameter number: MSB, then LSB. */
export function selectMessages(channel: number, parameter: Parameter): Midi1Message[] {
  return scoped([PARAMETER.size], (pp) => {
    writeParameter(pp, parameter);
    return written(2, (out, count) => fn("zmidi_parameter_select_messages")(int("channel", channel, 15), pp, out, 2, count), "selectMessages");
  });
}

/** The message of one change; `value` is the Data Entry byte (0 to 127). */
export function changeMessage(channel: number, change: ParameterChange, value = 0): Midi1Message {
  const args = [int("channel", channel, 15), changeNumber(change), int("value", value, 255)] as const;
  return scoped([MESSAGE.size], (out) => {
    check(fn("zmidi_parameter_change_message")(...args, out), "changeMessage");
    return readMessage(out);
  });
}

/** The four messages of a parameter value (0 to 3FFFH): the number, then Data Entry MSB and LSB. */
export function valueMessages(channel: number, parameter: Parameter, value: number): Midi1Message[] {
  return scoped([PARAMETER.size], (pp) => {
    writeParameter(pp, parameter);
    return written(4, (out, count) => fn("zmidi_parameter_value_messages")(int("channel", channel, 15), pp, int("value", value, 0xffff), out, 4, count), "valueMessages");
  });
}
