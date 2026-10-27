// USB-MIDI 1.0 Event Packets and USB MIDI 2.0 UMP words (zmidi-usb MidiLink through the C ABI): one
// cable, alternate setting 0 (USB-MIDI 1.0) or 1 (USB MIDI 2.0), with a translator to the protocol
// of the program.

import { InvalidValueError, RejectedError } from "./errors.js";
import { Blocks, Status, alloc, check, fn, int, layout, protocol, readBytes, readWords, scoped, sizeOf, umpWords, view, writeBytes, writeWords } from "./wasm.js";

/** The configuration of a USB link. */
export interface UsbLinkOptions {
  /** True on a host (the received endpoint is a MIDI IN endpoint); false on a device (default). */
  readonly hostSide?: boolean;
  /** The Group Terminals of the received endpoint: bit n is Group n (default 0xFFFF). */
  readonly groupMask?: number;
  /** wMaxPacketSize of the sent endpoint, 4 or more (default 64). */
  readonly maxPacket?: number;
  /** The protocol of the UMPs that `read` gives: 1 MIDI 1.0 (default), 2 MIDI 2.0. */
  readonly protocol?: number;
}

const CONFIG = layout([
  ["host_side", "bool"],
  ["group_mask", "u16"],
  ["max_packet", "u16"],
  ["protocol", "u32"],
]);

const UMP = layout([
  ["word_count", "u8"],
  ["words", ["u32", 4]],
]);

/** One USB MIDI interface with one cable: transfers in, UMPs out, and back. */
export class UsbLink {
  readonly #b = new Blocks(this);

  /** A link at alternate setting 0 with the configuration `options`. */
  constructor(options: UsbLinkOptions = {}) {
    const p = this.#b.add(alloc(sizeOf("zmidi_usb_link_size")));
    const pr = protocol(options.protocol ?? 1);
    scoped([CONFIG.size], (pc) => {
      const v = view();
      v.setUint8(pc + CONFIG.off["host_side"]!, options.hostSide ? 1 : 0);
      v.setUint16(pc + CONFIG.off["group_mask"]!, int("groupMask", options.groupMask ?? 0xffff, 0xffff), true);
      v.setUint16(pc + CONFIG.off["max_packet"]!, int("maxPacket", options.maxPacket ?? 64, 0xffff), true);
      v.setUint32(pc + CONFIG.off["protocol"]!, pr, true);
      check(fn("zmidi_usb_link_init")(p, pc), "UsbLink");
    });
  }

  /** Sets the alternate setting (0 or 1) after SET_INTERFACE, with its Group Terminals and packet size. */
  setAlternateSetting(setting: number, groupMask: number, maxPacket: number): void {
    check(
      fn("zmidi_usb_link_set_alternate_setting")(this.#b.main, int("setting", setting, 255), int("groupMask", groupMask, 0xffff), int("maxPacket", maxPacket, 0xffff)),
      "setAlternateSetting",
    );
  }

  /** Reports a USB reset: the link goes to alternate setting 0. */
  usbReset(): void {
    check(fn("zmidi_usb_link_usb_reset")(this.#b.main), "usbReset");
  }

  /** Reports a configuration change (SET_CONFIGURATION): the link goes to alternate setting 0. */
  configurationChange(): void {
    check(fn("zmidi_usb_link_configuration_change")(this.#b.main), "configurationChange");
  }

  /** The alternate setting: 0 (USB-MIDI 1.0) or 1 (USB MIDI 2.0). */
  get setting(): number {
    return fn("zmidi_usb_link_setting")(this.#b.main) & 0xff;
  }

  /** True at alternate setting 1 (UMP words on the wire). */
  get isUmp(): boolean {
    return fn("zmidi_usb_link_is_ump")(this.#b.main) !== 0;
  }

  /** The size in bytes of the USB packets that the writer writes. */
  get packetSize(): number {
    return fn("zmidi_usb_link_packet_size")(this.#b.main) >>> 0;
  }

  /** True when the writer has bytes for the next packet (a split UMP or a SysEx part). */
  get pending(): boolean {
    return fn("zmidi_usb_link_pending")(this.#b.main) !== 0;
  }

  /** True when the reader holds part of a UMP or of a SysEx message from the last transfer. */
  get holdsPart(): boolean {
    return fn("zmidi_usb_link_holds_part")(this.#b.main) !== 0;
  }

  /** Reads the data of one received transfer. Gives its UMPs and the count of errors. */
  read(data: Uint8Array): { umps: number[][]; errors: number } {
    const cap = Math.floor(data.length / 4) + 1;
    const [pd, out, count, errors] = this.#b.carve([Math.max(1, data.length), UMP.size * cap, 4, 4]);
    writeBytes(pd, data);
    check(fn("zmidi_usb_link_read")(this.#b.main, pd, data.length, out, cap, count, errors), "read");
    const v = view();
    const umps: number[][] = [];
    for (let i = 0; i < v.getUint32(count, true); i++) {
      const at = out + UMP.size * i;
      umps.push(readWords(at + UMP.off["words"]!, v.getUint8(at + UMP.off["word_count"]!)));
    }
    return { umps, errors: v.getUint32(errors, true) };
  }

  /** Writes one packet from `umps`. Gives the packet (empty: nothing to send) and the count of UMPs taken. */
  writePacket(umps: readonly (readonly number[])[]): { packet: Uint8Array; taken: number } {
    const n = this.packetSize;
    const count = Math.max(1, umps.length);
    const [pu, out, len, taken] = this.#b.carve([UMP.size * count, n, 4, 4]);
    umps.forEach((u, i) => {
      const w = umpWords(u);
      const at = pu + UMP.size * i;
      view().setUint8(at + UMP.off["word_count"]!, w.length);
      writeWords(at + UMP.off["words"]!, w);
    });
    check(fn("zmidi_usb_link_write_packet")(this.#b.main, pu, umps.length, out, n, len, taken), "writePacket");
    const v = view();
    return { packet: readBytes(out, v.getUint32(len, true)), taken: v.getUint32(taken, true) };
  }

  /** Frees the WASM memory of the link now. */
  free(): void {
    this.#b.release();
  }
}

// --- USB descriptors and Group Terminal Block requests (C ABI 0.5, DEC-019 third addendum) ---

/** The most USB MIDI 2.0 Alternate Settings of a UsbFunction (ZMIDI_USB_MAX_SETTINGS). */
export const USB_MAX_SETTINGS = 2;
/** The most endpoints of one setting (ZMIDI_USB_MAX_ENDPOINTS). */
export const USB_MAX_ENDPOINTS = 4;
/** The most Group Terminal Blocks of one setting, and block IDs of one endpoint (ZMIDI_USB_MAX_BLOCKS). */
export const USB_MAX_BLOCKS = 16;

/** The kind of a Group Terminal Block (USB-MIDI v2.0 Table 5-6, bGrpTrmBlkType). */
export type UsbBlockKind = "bidirectional" | "inputOnly" | "outputOnly";

const BLOCK_KINDS: readonly UsbBlockKind[] = ["bidirectional", "inputOnly", "outputOnly"];

/** One Group Terminal Block of a USB MIDI 2.0 setting (USB-MIDI v2.0 5.4.2.1). */
export interface UsbBlock {
  /** bGrpTrmBlkID, 1 to 255. */
  readonly id: number;
  /** bGrpTrmBlkType (default "bidirectional"). */
  readonly kind?: UsbBlockKind;
  /** nGroupTrm: the first Group, 0 to 15 (0 is Group 1). */
  readonly firstGroup: number;
  /** nNumGroupTrm: the number of Groups, 1 to 16 (default 1). */
  readonly groupCount?: number;
  /** iBlockItem: the string index (default 0, no string). */
  readonly string?: number;
  /** bMIDIProtocol (default 0x11, MIDI 2.0). */
  readonly protocol?: number;
  /** wMaxInputBandwidth in 4,000 bytes per second (default 0, not known). */
  readonly maxInputBandwidth?: number;
  /** wMaxOutputBandwidth in 4,000 bytes per second (default 0, not known). */
  readonly maxOutputBandwidth?: number;
}

/** One endpoint of a USB MIDI 2.0 setting. */
export interface UsbEndpoint {
  /** "out": a MIDI OUT endpoint (host to device); "in": a MIDI IN endpoint. */
  readonly direction: "out" | "in";
  /** The endpoint number, 1 to 15. */
  readonly number: number;
  /** "bulk" (default) or "interrupt". */
  readonly transfer?: "bulk" | "interrupt";
  /** bInterval (default 0; 1 or more for an interrupt endpoint). */
  readonly interval?: number;
  /** wMaxPacketSize (default 64 at full speed, 512 at high speed). */
  readonly maxPacket?: number;
  /** The IDs of the Group Terminal Blocks of the endpoint, 1 to 16 IDs (default [1]). */
  readonly blocks?: readonly number[];
}

/** One USB MIDI 2.0 Alternate Setting (1, then 2). */
export interface UsbSetting {
  /** iInterface (default 0). */
  readonly string?: number;
  /** The endpoints, 1 to 4. */
  readonly endpoints: readonly UsbEndpoint[];
  /** The Group Terminal Blocks, 1 to 16 (default: one bidirectional block, ID 1, Group 1, 0x11). */
  readonly blocks?: readonly UsbBlock[];
}

/** Alternate Setting 0 (USB-MIDI 1.0) as cable counts. */
export interface UsbAlt0 {
  /** iInterface (default 0). */
  readonly string?: number;
  /** Cables of the MIDI OUT endpoint, 0 to 16 (default 1). */
  readonly outCables?: number;
  /** Cables of the MIDI IN endpoint, 0 to 16 (default 1). */
  readonly inCables?: number;
  /** The number of the MIDI OUT endpoint (default 1). */
  readonly outEndpoint?: number;
  /** The number of the MIDI IN endpoint (default 1). */
  readonly inEndpoint?: number;
  /** wMaxPacketSize (default 0: 64 at full speed, 512 at high speed). */
  readonly maxPacket?: number;
}

/** The full description of a USB MIDI function. */
export interface UsbFunctionOptions {
  /** bInterfaceNumber of the AudioControl interface. */
  readonly acInterface: number;
  /** bInterfaceNumber of the MIDI Streaming interface. */
  readonly msInterface: number;
  /** High speed (default false: full speed). */
  readonly highSpeed?: boolean;
  /** iInterface of the AudioControl interface (default 0). */
  readonly acString?: number;
  /** Alternate Setting 0 (default: one cable in each direction on endpoints 1). */
  readonly alt0?: UsbAlt0;
  /** The USB MIDI 2.0 settings, 0 to 2 (default none). */
  readonly settings?: readonly UsbSetting[];
}

/** One endpoint for the USB stack. */
export interface UsbSettingEndpoint {
  /** bAlternateSetting of the endpoint. */
  readonly alternateSetting: number;
  /** bEndpointAddress (0x80 set for IN). */
  readonly address: number;
  /** bmAttributes: 2 bulk, 3 interrupt. */
  readonly transfer: number;
  /** bInterval. */
  readonly interval: number;
  /** wMaxPacketSize. */
  readonly maxPacket: number;
}

/** A Group Terminal Block that the host read, with the raw field values (the reader does not check them). */
export interface UsbBlockReport {
  readonly id: number;
  /** bGrpTrmBlkType: 0 bidirectional, 1 input only, 2 output only. */
  readonly kind: number;
  readonly firstGroup: number;
  readonly groupCount: number;
  readonly string: number;
  readonly protocol: number;
  readonly maxInputBandwidth: number;
  readonly maxOutputBandwidth: number;
}

/** What the device does with a SETUP packet (zmidi_usb_gtb_setup). */
export interface UsbSetupAnswer {
  /** "answered": send `data`; "stall": stall the request; "notHandled": another handler takes it. */
  readonly response: "answered" | "stall" | "notHandled";
  /** The bytes to send ("answered"); empty for the other responses. */
  readonly data: Uint8Array;
}

const FN_CONFIG = layout([
  ["ac_interface", "u8"],
  ["ms_interface", "u8"],
  ["high_speed", "bool"],
  ["ac_string", "u8"],
  ["alt0_string", "u8"],
  ["n_out", "u8"],
  ["n_in", "u8"],
  ["out_endpoint", "u8"],
  ["in_endpoint", "u8"],
  ["max_packet", "u16"],
]);

const ENDPOINT = layout([
  ["direction", "u8"],
  ["number", "u8"],
  ["transfer", "u8"],
  ["interval", "u8"],
  ["max_packet", "u16"],
  ["block_count", "u8"],
  ["block_ids", ["u8", USB_MAX_BLOCKS]],
]);

const BLOCK = layout([
  ["id", "u8"],
  ["kind", "u8"],
  ["first_group", "u8"],
  ["num_groups", "u8"],
  ["string_index", "u8"],
  ["protocol", "u8"],
  ["max_input_bandwidth", "u16"],
  ["max_output_bandwidth", "u16"],
]);

const SETTING_ENDPOINT = layout([
  ["alternate_setting", "u8"],
  ["address", "u8"],
  ["transfer", "u8"],
  ["interval", "u8"],
  ["max_packet", "u16"],
]);

const SETUP_LEN = 8;
/** The key of the options of defaultFunction (not exported). */
const DEFAULT = Symbol("default");
/** The largest Group Terminal Block set: the header and 16 blocks. */
const GTB_SET_MAX = 5 + 13 * USB_MAX_BLOCKS;

function writeBlock(at: number, b: UsbBlock): void {
  const v = view();
  const o = BLOCK.off;
  const kind = BLOCK_KINDS.indexOf(b.kind ?? "bidirectional");
  if (kind < 0) {
    throw new InvalidValueError(`a block kind is "bidirectional", "inputOnly" or "outputOnly", not ${String(b.kind)}`);
  }
  v.setUint8(at + o["id"]!, int("block id", b.id, 255));
  v.setUint8(at + o["kind"]!, kind);
  v.setUint8(at + o["first_group"]!, int("firstGroup", b.firstGroup, 15));
  v.setUint8(at + o["num_groups"]!, int("groupCount", b.groupCount ?? 1, 16));
  v.setUint8(at + o["string_index"]!, int("block string", b.string ?? 0, 255));
  v.setUint8(at + o["protocol"]!, int("protocol", b.protocol ?? 0x11, 255));
  v.setUint16(at + o["max_input_bandwidth"]!, int("maxInputBandwidth", b.maxInputBandwidth ?? 0, 0xffff), true);
  v.setUint16(at + o["max_output_bandwidth"]!, int("maxOutputBandwidth", b.maxOutputBandwidth ?? 0, 0xffff), true);
}

function readBlock(at: number): UsbBlockReport {
  const v = view();
  const o = BLOCK.off;
  return {
    id: v.getUint8(at + o["id"]!),
    kind: v.getUint8(at + o["kind"]!),
    firstGroup: v.getUint8(at + o["first_group"]!),
    groupCount: v.getUint8(at + o["num_groups"]!),
    string: v.getUint8(at + o["string_index"]!),
    protocol: v.getUint8(at + o["protocol"]!),
    maxInputBandwidth: v.getUint16(at + o["max_input_bandwidth"]!, true),
    maxOutputBandwidth: v.getUint16(at + o["max_output_bandwidth"]!, true),
  };
}

/**
 * A USB MIDI function: the descriptors of the AudioControl and MIDI Streaming interfaces (USB-MIDI
 * v2.0 Appendix B), the endpoints for the USB stack, the Group Terminal masks for UsbLink, and the
 * Group Terminal Block requests (USB-MIDI v2.0 section 6). The constructor takes the full
 * description and checks it against the rules of zmidi-usb: a broken rule throws RejectedError.
 */
export class UsbFunction {
  readonly #b = new Blocks(this);

  /** A function of the description `options`. */
  constructor(options: UsbFunctionOptions) {
    const settings = options.settings ?? [];
    if (settings.length > USB_MAX_SETTINGS) {
      throw new InvalidValueError(`a function has at most ${USB_MAX_SETTINGS} USB MIDI 2.0 settings, not ${settings.length}`);
    }
    const hs = options.highSpeed ?? false;
    const f = this.#b.add(alloc(sizeOf("zmidi_usb_function_size")));
    if ((options as { [DEFAULT]?: boolean })[DEFAULT] === true) {
      const ac = int("acInterface", options.acInterface, 255);
      const ms = int("msInterface", options.msInterface, 255);
      check(fn("zmidi_usb_function_init_default")(f, ac, ms, hs ? 1 : 0), "UsbFunction");
      this.#validate();
      return;
    }
    const a = options.alt0 ?? {};
    scoped([FN_CONFIG.size], (pc) => {
      const v = view();
      const o = FN_CONFIG.off;
      v.setUint8(pc + o["ac_interface"]!, int("acInterface", options.acInterface, 255));
      v.setUint8(pc + o["ms_interface"]!, int("msInterface", options.msInterface, 255));
      v.setUint8(pc + o["high_speed"]!, hs ? 1 : 0);
      v.setUint8(pc + o["ac_string"]!, int("acString", options.acString ?? 0, 255));
      v.setUint8(pc + o["alt0_string"]!, int("alt0 string", a.string ?? 0, 255));
      v.setUint8(pc + o["n_out"]!, int("outCables", a.outCables ?? 1, 255));
      v.setUint8(pc + o["n_in"]!, int("inCables", a.inCables ?? 1, 255));
      v.setUint8(pc + o["out_endpoint"]!, int("outEndpoint", a.outEndpoint ?? 1, 15));
      v.setUint8(pc + o["in_endpoint"]!, int("inEndpoint", a.inEndpoint ?? 1, 15));
      v.setUint16(pc + o["max_packet"]!, int("alt0 maxPacket", a.maxPacket ?? 0, 0xffff), true);
      check(fn("zmidi_usb_function_init")(f, pc), "UsbFunction");
    });
    for (const s of settings) {
      this.#addSetting(f, s, hs);
    }
    this.#validate();
  }

  /** The default function (USB-MIDI v2.0 Appendix B): one cable in each direction, one USB MIDI 2.0 setting with one bidirectional block of Group 1. */
  static defaultFunction(acInterface: number, msInterface: number, highSpeed = false): UsbFunction {
    const options = { acInterface, msInterface, highSpeed, [DEFAULT]: true };
    return new UsbFunction(options);
  }

  #addSetting(f: number, s: UsbSetting, hs: boolean): void {
    if (s.endpoints.length > USB_MAX_ENDPOINTS) {
      throw new InvalidValueError(`a setting has at most ${USB_MAX_ENDPOINTS} endpoints, not ${s.endpoints.length}`);
    }
    if (s.blocks !== undefined && s.blocks.length > USB_MAX_BLOCKS) {
      throw new InvalidValueError(`a setting has at most ${USB_MAX_BLOCKS} blocks, not ${s.blocks.length}`);
    }
    const alt = scoped([4], (pa) => {
      check(fn("zmidi_usb_function_add_setting")(f, int("setting string", s.string ?? 0, 255), pa), "UsbFunction");
      return view().getUint8(pa);
    });
    for (const e of s.endpoints) {
      const ids = e.blocks ?? [1];
      if (ids.length < 1 || ids.length > USB_MAX_BLOCKS) {
        throw new InvalidValueError(`an endpoint names 1 to ${USB_MAX_BLOCKS} blocks, not ${ids.length}`);
      }
      if (e.direction !== "out" && e.direction !== "in") {
        throw new InvalidValueError(`an endpoint direction is "out" or "in", not ${String(e.direction)}`);
      }
      const transfer = e.transfer ?? "bulk";
      if (transfer !== "bulk" && transfer !== "interrupt") {
        throw new InvalidValueError(`an endpoint transfer is "bulk" or "interrupt", not ${String(transfer)}`);
      }
      scoped([ENDPOINT.size], (pe) => {
        const v = view();
        const o = ENDPOINT.off;
        v.setUint8(pe + o["direction"]!, e.direction === "out" ? 0 : 1);
        v.setUint8(pe + o["number"]!, int("endpoint number", e.number, 15));
        v.setUint8(pe + o["transfer"]!, transfer === "bulk" ? 2 : 3);
        v.setUint8(pe + o["interval"]!, int("interval", e.interval ?? 0, 255));
        v.setUint16(pe + o["max_packet"]!, int("maxPacket", e.maxPacket ?? (hs ? 512 : 64), 0xffff), true);
        v.setUint8(pe + o["block_count"]!, ids.length);
        ids.forEach((id, i) => v.setUint8(pe + o["block_ids"]! + i, int("block id", id, 255)));
        check(fn("zmidi_usb_function_add_endpoint")(f, alt, pe), "UsbFunction");
      });
    }
    const blocks = s.blocks;
    if (blocks !== undefined) {
      scoped([Math.max(1, BLOCK.size * blocks.length)], (pb) => {
        blocks.forEach((b, i) => writeBlock(pb + BLOCK.size * i, b));
        check(fn("zmidi_usb_function_set_blocks")(f, alt, pb, blocks.length), "UsbFunction");
      });
    }
  }

  /** Throws RejectedError when the function breaks a rule of zmidi-usb (the writer checks them all). */
  #validate(): void {
    scoped([4], (len) => {
      const st = fn("zmidi_usb_function_len")(this.#b.main, len);
      if (st === Status.REJECTED) {
        throw new RejectedError("UsbFunction: the description breaks a rule of USB-MIDI v2.0 (zmidi-usb)");
      }
      check(st, "UsbFunction");
    });
  }

  /** The descriptors of the function (USB-MIDI v2.0 Appendix B order), for the Configuration descriptor. */
  descriptors(): Uint8Array {
    const f = this.#b.main;
    const n = scoped([4], (len) => {
      check(fn("zmidi_usb_function_len")(f, len), "descriptors");
      return view().getUint32(len, true);
    });
    return scoped([Math.max(1, n), 4], (out, written) => {
      check(fn("zmidi_usb_function_write")(f, out, n, written), "descriptors");
      return readBytes(out, view().getUint32(written, true));
    });
  }

  /** The endpoints of every setting in descriptor order, for the USB stack. */
  endpoints(): UsbSettingEndpoint[] {
    const f = this.#b.main;
    const count = fn("zmidi_usb_function_endpoint_count")(f) >>> 0;
    const out: UsbSettingEndpoint[] = [];
    for (let i = 0; i < count; i++) {
      out.push(
        scoped([SETTING_ENDPOINT.size], (pe) => {
          check(fn("zmidi_usb_function_endpoint")(f, i, pe), "endpoints");
          const v = view();
          const o = SETTING_ENDPOINT.off;
          return {
            alternateSetting: v.getUint8(pe + o["alternate_setting"]!),
            address: v.getUint8(pe + o["address"]!),
            transfer: v.getUint8(pe + o["transfer"]!),
            interval: v.getUint8(pe + o["interval"]!),
            maxPacket: v.getUint16(pe + o["max_packet"]!, true),
          };
        }),
      );
    }
    return out;
  }

  /**
   * The Group Terminals of the endpoints of `direction` in Alternate Setting `setting` (bit n is
   * Group n), for UsbLink.setAlternateSetting: 0xFFFF for Alternate Setting 0, 0 for a setting
   * that does not exist.
   */
  groupMask(setting: number, direction: "out" | "in"): number {
    if (direction !== "out" && direction !== "in") {
      throw new InvalidValueError(`a direction is "out" or "in", not ${String(direction)}`);
    }
    return fn("zmidi_usb_function_group_mask")(this.#b.main, int("setting", setting, 255), direction === "out" ? 0 : 1) & 0xffff;
  }

  /** The Group Terminal Block descriptor set of USB MIDI 2.0 Alternate Setting `setting` (1 or 2). */
  gtbSet(setting: number): Uint8Array {
    const s = int("setting", setting, 255);
    return scoped([GTB_SET_MAX, 4], (out, written) => {
      check(fn("zmidi_usb_gtb_set_write")(this.#b.main, s, out, GTB_SET_MAX, written), "gtbSet");
      return readBytes(out, view().getUint32(written, true));
    });
  }

  /** The device side of the Group Terminal Block request: what to do with the 8 bytes of a SETUP packet. */
  answerSetup(setup: Uint8Array): UsbSetupAnswer {
    if (setup.length !== SETUP_LEN) {
      throw new InvalidValueError(`a SETUP packet has ${SETUP_LEN} bytes, not ${setup.length}`);
    }
    return scoped([SETUP_LEN, GTB_SET_MAX, 4, 4], (ps, out, response, written) => {
      writeBytes(ps, setup);
      check(fn("zmidi_usb_gtb_setup")(this.#b.main, ps, out, GTB_SET_MAX, response, written), "answerSetup");
      const v = view();
      const r = (["answered", "stall", "notHandled"] as const)[v.getInt32(response, true)] ?? "notHandled";
      return { response: r, data: readBytes(out, v.getUint32(written, true)) };
    });
  }

  /** Frees the WASM memory of the function now. */
  free(): void {
    this.#b.release();
  }
}

/**
 * The host side: the 8 bytes of the SETUP packet of the Group Terminal Block request for Alternate
 * Setting `setting` of the interface `iface` with wLength `length`. Send wLength 5 first (the
 * header), then the wTotalLength that gtbTotalLength reads from the answer.
 */
export function gtbRequest(setting: number, iface: number, length: number): Uint8Array {
  return scoped([SETUP_LEN], (out) => {
    check(fn("zmidi_usb_gtb_request")(int("setting", setting, 255), int("interface", iface, 255), int("length", length, 0xffff), out), "gtbRequest");
    return readBytes(out, SETUP_LEN);
  });
}

/** The host side: wTotalLength of the Group Terminal Block Header at the start of `answer`. Throws RejectedError when it has no header. */
export function gtbTotalLength(answer: Uint8Array): number {
  return scoped([Math.max(1, answer.length), 4], (pa, total) => {
    writeBytes(pa, answer);
    check(fn("zmidi_usb_gtb_total_length")(pa, answer.length, total), "gtbTotalLength");
    return view().getUint16(total, true);
  });
}

/** The host side: the Group Terminal Blocks of `answer`, read with the lenient reader of zmidi-usb. */
export function gtbBlocks(answer: Uint8Array): UsbBlockReport[] {
  return scoped([Math.max(1, answer.length), BLOCK.size], (pa, pb) => {
    writeBytes(pa, answer);
    const out: UsbBlockReport[] = [];
    for (let i = 0; ; i++) {
      const st = fn("zmidi_usb_gtb_block")(pa, answer.length, i, pb);
      if (st === Status.NO_PENDING) {
        return out;
      }
      check(st, "gtbBlocks");
      out.push(readBlock(pb));
    }
  });
}
