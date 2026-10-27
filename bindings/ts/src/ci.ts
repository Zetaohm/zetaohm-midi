// A MIDI-CI Device with the one-chunk Property Exchange Responder (zmidi-ci, zmidi-ci-pe through the C
// ABI; M2-101, M2-103): messages in, automatic replies and events out, a reply call for each PE
// request. Profile Configuration (M2-101 7) and Process Inquiry (M2-101 9) need C ABI 0.2.

import { InvalidValueError } from "./errors.js";
import { Blocks, Status, alloc, allocCString, check, fn, int, layout, readBytes, scoped, sizeOf, u64, view, writeBytes } from "./wasm.js";

/** The kind of a CI event: one for each zmidi-ci Event, Property Exchange Chunks by kind. */
export type CiEventKind =
  | "discovered"
  | "discoveryEnded"
  | "muidChanged"
  | "peerInvalidated"
  | "endpointInfo"
  | "ack"
  | "nak"
  | "profileInquiryReply"
  | "profileReport"
  | "setProfile"
  | "profileDetailsInquiry"
  | "profileDetailsReply"
  | "profileSpecificData"
  | "peCapabilities"
  | "peGet"
  | "peSet"
  | "peSubscription"
  | "peGetReply"
  | "peSetReply"
  | "peSubscriptionReply"
  | "peNotify"
  | "piCapabilities"
  | "messageReport"
  | "messageReportReply"
  | "messageReportEnd"
  | "rejected";

const KINDS: readonly (CiEventKind | null)[] = [
  null,
  "discovered",
  "discoveryEnded",
  "muidChanged",
  "peerInvalidated",
  "endpointInfo",
  "ack",
  "nak",
  "profileInquiryReply",
  "profileReport",
  "setProfile",
  "profileDetailsInquiry",
  "profileDetailsReply",
  "profileSpecificData",
  "peCapabilities",
  "peGet",
  "peSet",
  "peSubscription",
  "peGetReply",
  "peSetReply",
  "peSubscriptionReply",
  "peNotify",
  "piCapabilities",
  "messageReport",
  "messageReportReply",
  "messageReportEnd",
  "rejected",
];

/** The kind of a Profile report (M2-101 7.4, 7.5, 7.10, 7.11; ZmidiCiProfileReport). */
export type CiProfileReport = "added" | "removed" | "enabled" | "disabled";

const REPORTS: readonly (CiProfileReport | null)[] = [null, "added", "removed", "enabled", "disabled"];

/** The Profile fields of a Profile event (M2-101 7; zmidi_ci_profile_event). */
export interface CiProfileEvent {
  /** Device ID of the message: Channel 0x00 to 0x0F, Group 0x7E, Function Block 0x7F. */
  readonly deviceId: number;
  /**
   * UMP Group of the message; for "setProfile" and "profileDetailsInquiry" the Group offset of
   * the local Profile address.
   */
  readonly group: number;
  /** Profile ID, 5 bytes (zeros for "profileInquiryReply"). */
  readonly profile: Uint8Array;
  /** "profileReport": the kind of the report; null for the other events. */
  readonly report: CiProfileReport | null;
  /** "setProfile": true for Set Profile On, false for Set Profile Off. */
  readonly on: boolean;
  /** "setProfile", "profileReport": Number of Channels (Requested, Enabled or Disabled). */
  readonly channels: number;
  /** "profileDetailsInquiry", "profileDetailsReply": Inquiry Target. */
  readonly target: number;
  /** "profileDetailsReply": true when this Device did not ask for it (M2-101 7.7). */
  readonly notification: boolean;
  /** "profileInquiryReply": the Currently Enabled Profiles, 5 bytes each (Table 18). */
  readonly enabledProfiles: readonly Uint8Array[];
  /** "profileInquiryReply": the Currently Disabled Profiles Supported, 5 bytes each. */
  readonly disabledProfiles: readonly Uint8Array[];
  /** "profileDetailsReply": Inquiry Target Data; "profileSpecificData": Profile Specific Data. */
  readonly data: Uint8Array;
}

/** The bitmaps of a MIDI Message Report (M2-101 Tables 43 to 45): one 7-bit value each. */
export interface CiReport {
  /**
   * Message Data Control (Table 44): 0x00 Begin and End only, 0x01 non-default values, 0x7F
   * full report. Not used for the supported set of `messageReportReply`.
   */
  readonly dataControl: number;
  /** System Messages: D0 MTC Quarter Frame, D1 Song Position, D2 Song Select. */
  readonly system: number;
  /**
   * Channel Controller Messages: D0 Pitchbend, D1 Control Change, D2 RPN, D3 NRPN, D4 Program
   * Change, D5 Channel Pressure.
   */
  readonly channel: number;
  /**
   * Note Data Messages: D0 Notes, D1 Poly Pressure, D2 Per-Note Pitchbend, D3 Registered
   * Per-Note Controller, D4 Assignable Per-Note Controller.
   */
  readonly note: number;
}

/** The Process Inquiry fields of a Process Inquiry event (M2-101 9; zmidi_ci_process_event). */
export interface CiProcessEvent {
  /** "messageReport": Device ID of the inquiry (0 for the other events). */
  readonly deviceId: number;
  /** "messageReport": UMP Group of the inquiry (0 for the other events). */
  readonly group: number;
  /** "piCapabilities": the peer supports MIDI Message Report (Table 42, D0). */
  readonly midiMessageReport: boolean;
  /**
   * "messageReport": the requested bitmaps and Message Data Control; "messageReportReply": the
   * bitmaps that the peer reports (dataControl 0). Zeros for the other events.
   */
  readonly report: CiReport;
}

/** One local Profile of the Device (M2-101 7.1 to 7.3; ZmidiCiProfile). */
export interface CiProfile {
  /** Device ID of the address: Channel 0x00 to 0x0F, Group 0x7E, Function Block 0x7F. */
  readonly deviceId: number;
  /** Group offset of the address in the Function Block, 0 to 15 (default 0; 0 for 0x7F). */
  readonly group?: number;
  /** Profile ID, 5 bytes (Table 19). */
  readonly profile: Uint8Array;
  /** True when the Profile is enabled now (default false). */
  readonly enabled?: boolean;
  /** Number of Channels enabled (Manager and Members), 14 bits (default 0). */
  readonly channels?: number;
  /** Maximum Number of Channels, 14 bits (default 0). */
  readonly maxChannels?: number;
}

/** One event of a CI Device. */
export interface CiEvent {
  /** The kind of the event. */
  readonly kind: CiEventKind;
  /** The MUID of the peer (the new MUID for "muidChanged"), or null when the event has none. */
  readonly muid: number | null;
  /** Property Exchange: Request ID, or null for an event that is not a Chunk. */
  readonly requestId: number | null;
  /** Property Exchange: Number of This Chunk, or null. */
  readonly chunkNumber: number | null;
  /** Property Exchange: Number of Chunks in Message (0 when not known), or null. */
  readonly chunkTotal: number | null;
  /** PE Get or Set: the decoded "resource", or null. */
  readonly resource: string | null;
  /** PE Get or Set: the decoded "resId", or null. */
  readonly resId: string | null;
  /** Property Exchange: the Header Data (empty for other events). */
  readonly header: Uint8Array;
  /** Property Exchange: the Property Data of this Chunk (empty for other events). */
  readonly data: Uint8Array;
  /** A Profile event of `receive`: its Profile fields; null for the other events. */
  readonly profile: CiProfileEvent | null;
  /** A Process Inquiry event of `receive`: its Process Inquiry fields; null for the other events. */
  readonly process: CiProcessEvent | null;
}

/** One message with its Group. */
export interface CiMessage {
  /** The Group, 0 to 15. */
  readonly group: number;
  /** The MIDI-CI message. */
  readonly message: Uint8Array;
}

/** The configuration of a CI Device (M2-101 Discovery, Endpoint Information). */
export interface CiDeviceOptions {
  /** Manufacturer SysEx ID: 1 byte, or 3 bytes that start with 0x00. */
  readonly manufacturer: Uint8Array;
  /** Device Family, 14 bits (default 0). */
  readonly family?: number;
  /** Device Family Model Number, 14 bits (default 0). */
  readonly model?: number;
  /** Software Revision Level, 4 bytes of 7 bits (default zeros). */
  readonly revision?: Uint8Array;
  /** Receivable Maximum SysEx Message Size (default 512; at least 512 with Profiles or PE). */
  readonly maxSysexSize?: number;
  /** Function Block number, 0 to 0x7E, when the Device is a Function Block. */
  readonly functionBlock?: number;
  /** First Group of the Function Block, 0 to 15 (default 0). */
  readonly firstGroup?: number;
  /** Number of Groups, 1 to 16 (default 1). */
  readonly groupCount?: number;
  /** Channels in use on each Group: bit n is Channel n+1 (default 1). */
  readonly channelsInUse?: number;
  /** Product Instance ID, at most 42 ASCII characters. */
  readonly productInstanceId?: string;
  /** Profile Configuration supported (default false). */
  readonly profiles?: boolean;
  /** Property Exchange supported (default true). */
  readonly propertyExchange?: boolean;
  /** Process Inquiry supported, with MIDI Message Report (default false). */
  readonly processInquiry?: boolean;
  /** True: messages with 0xF0/0xF7 (MIDI 1.0 byte stream); false: SysEx7 payload only (default). */
  readonly sysexFraming?: boolean;
  /** ResourceList JSON for a Get of "ResourceList" (default "[]"). */
  readonly resourceList?: Uint8Array;
  /** Seed of the MUID generator (default 1). */
  readonly seed?: number;
}

const CONFIG = layout([
  ["manufacturer", ["u8", 3]],
  ["family", "u16"],
  ["model", "u16"],
  ["revision", ["u8", 4]],
  ["max_sysex_size", "u32"],
  ["has_function_block", "bool"],
  ["function_block", "u8"],
  ["first_group", "u8"],
  ["group_count", "u8"],
  ["channels_in_use", "u16"],
  ["product_instance_id", "ptr"],
  ["profiles", "bool"],
  ["property_exchange", "bool"],
  ["process_inquiry", "bool"],
  ["sysex_framing", "bool"],
  ["resource_list", "ptr"],
  ["resource_list_len", "usize"],
]);

/** ZMIDI_PE_ID_LEN. */
const PE_ID_LEN = 37;

const EVENT = layout([
  ["kind", "i32"],
  ["muid", "u32"],
  ["request_id", "u8"],
  ["chunk_total", "u16"],
  ["chunk_number", "u16"],
  ["resource", ["u8", PE_ID_LEN]],
  ["res_id", ["u8", PE_ID_LEN]],
  ["header", "ptr"],
  ["header_len", "usize"],
  ["data", "ptr"],
  ["data_len", "usize"],
]);

/** ZMIDI_CI_REPLY_BUFFER: the automatic replies of one message. */
const REPLY_BUFFER = 693;

/** ZMIDI_CI_REPORT_BUFFER: the Profile reports of one Profile table call. */
const REPORT_BUFFER = 126;

const PROFILE = layout([
  ["device_id", "u8"],
  ["group", "u8"],
  ["profile", ["u8", 5]],
  ["enabled", "bool"],
  ["channels", "u16"],
  ["max_channels", "u16"],
]);

const PROFILE_EVENT = layout([
  ["device_id", "u8"],
  ["group", "u8"],
  ["profile", ["u8", 5]],
  ["report", "i32"],
  ["on", "bool"],
  ["channels", "u16"],
  ["target", "u8"],
  ["notification", "bool"],
  ["enabled_count", "usize"],
  ["disabled_count", "usize"],
  ["data", "ptr"],
  ["data_len", "usize"],
]);

const REPORT = layout([
  ["data_control", "u8"],
  ["system", "u8"],
  ["channel", "u8"],
  ["note", "u8"],
]);

const PROCESS_EVENT = layout([
  ["device_id", "u8"],
  ["group", "u8"],
  ["midi_message_report", "bool"],
  ["report", REPORT],
]);

/** The records `[group][len lo][len hi][bytes]` of `len` bytes at `ptr`. */
function records(ptr: number, len: number): CiMessage[] {
  const b = readBytes(ptr, len);
  const out: CiMessage[] = [];
  let at = 0;
  while (at + 3 <= b.length) {
    const n = b[at + 1]! | (b[at + 2]! << 8);
    out.push({ group: b[at]!, message: b.slice(at + 3, at + 3 + n) });
    at += 3 + n;
  }
  return out;
}

function cString(ptr: number): string | null {
  const b = readBytes(ptr, PE_ID_LEN);
  const end = b.indexOf(0);
  const s = new TextDecoder().decode(b.subarray(0, end < 0 ? b.length : end));
  return s === "" ? null : s;
}

/** Checks a Profile ID: 5 bytes of 7 bits. */
function profileId(profile: Uint8Array): Uint8Array {
  if (profile.length !== 5) {
    throw new InvalidValueError(`a Profile ID has 5 bytes, not ${profile.length}`);
  }
  profile.forEach((x, i) => int(`profile byte ${i}`, x, 0x7f));
  return profile;
}

/** Writes the bitmaps `r` to a ZmidiCiReport at `ptr`; the C ABI refuses a value over 0x7F. */
function writeReport(ptr: number, r: CiReport): void {
  const v = view();
  v.setUint8(ptr + REPORT.off["data_control"]!, int("dataControl", r.dataControl, 0xff));
  v.setUint8(ptr + REPORT.off["system"]!, int("system", r.system, 0xff));
  v.setUint8(ptr + REPORT.off["channel"]!, int("channel", r.channel, 0xff));
  v.setUint8(ptr + REPORT.off["note"]!, int("note", r.note, 0xff));
}

function readReport(ptr: number): CiReport {
  const v = view();
  return {
    dataControl: v.getUint8(ptr + REPORT.off["data_control"]!),
    system: v.getUint8(ptr + REPORT.off["system"]!),
    channel: v.getUint8(ptr + REPORT.off["channel"]!),
    note: v.getUint8(ptr + REPORT.off["note"]!),
  };
}

/** The Profile IDs of the last ProfileInquiryReply: the enabled ones or the disabled ones. */
function profileIds(ci: number, enabled: boolean, count: number): Uint8Array[] {
  return scoped([Math.max(1, 5 * count), 4], (out, n) => {
    check(fn("zmidi_ci_profile_ids")(ci, enabled ? 1 : 0, out, 5 * count, n), "receive");
    const ids = readBytes(out, 5 * view().getUint32(n, true));
    const list: Uint8Array[] = [];
    for (let at = 0; at < ids.length; at += 5) {
      list.push(ids.slice(at, at + 5));
    }
    return list;
  });
}

/** The Profile fields of the last event of `receive`, or null when it is not a Profile event. */
function readProfileEvent(ci: number, kind: CiEventKind): CiProfileEvent | null {
  return scoped([PROFILE_EVENT.size], (pe) => {
    const st = fn("zmidi_ci_profile_event")(ci, pe);
    if (st === Status.NO_PENDING) {
      return null;
    }
    check(st, "receive");
    const v = view();
    const o = PROFILE_EVENT.off;
    const reply = kind === "profileInquiryReply";
    const dataLen = v.getUint32(pe + o["data_len"]!, true);
    return {
      deviceId: v.getUint8(pe + o["device_id"]!),
      group: v.getUint8(pe + o["group"]!),
      profile: readBytes(pe + o["profile"]!, 5),
      report: REPORTS[v.getInt32(pe + o["report"]!, true)] ?? null,
      on: v.getUint8(pe + o["on"]!) !== 0,
      channels: v.getUint16(pe + o["channels"]!, true),
      target: v.getUint8(pe + o["target"]!),
      notification: v.getUint8(pe + o["notification"]!) !== 0,
      enabledProfiles: reply ? profileIds(ci, true, v.getUint32(pe + o["enabled_count"]!, true)) : [],
      disabledProfiles: reply ? profileIds(ci, false, v.getUint32(pe + o["disabled_count"]!, true)) : [],
      data: dataLen === 0 ? new Uint8Array(0) : readBytes(v.getUint32(pe + o["data"]!, true), dataLen),
    };
  });
}

/** The Process Inquiry fields of the last event of `receive`, or null for another event. */
function readProcessEvent(ci: number): CiProcessEvent | null {
  return scoped([PROCESS_EVENT.size], (pe) => {
    const st = fn("zmidi_ci_process_event")(ci, pe);
    if (st === Status.NO_PENDING) {
      return null;
    }
    check(st, "receive");
    const v = view();
    const o = PROCESS_EVENT.off;
    return {
      deviceId: v.getUint8(pe + o["device_id"]!),
      group: v.getUint8(pe + o["group"]!),
      midiMessageReport: v.getUint8(pe + o["midi_message_report"]!) !== 0,
      report: readReport(pe + o["report"]!),
    };
  });
}

/** Profile events (M2-101 7) and Process Inquiry events (M2-101 9) by kind. */
const PROFILE_KINDS: ReadonlySet<CiEventKind> = new Set<CiEventKind>([
  "profileInquiryReply",
  "profileReport",
  "setProfile",
  "profileDetailsInquiry",
  "profileDetailsReply",
  "profileSpecificData",
]);
const PROCESS_KINDS: ReadonlySet<CiEventKind> = new Set<CiEventKind>(["piCapabilities", "messageReport", "messageReportReply", "messageReportEnd"]);

/**
 * The event at `ev`. With `ci` (an event of `receive`, whose message is still in the message
 * buffer), also its Profile or Process Inquiry fields.
 */
function readEvent(ev: number, ci?: number): CiEvent | null {
  const v = view();
  const kind = KINDS[v.getInt32(ev + EVENT.off["kind"]!, true)] ?? null;
  if (kind === null) {
    return null;
  }
  const code = v.getInt32(ev + EVENT.off["kind"]!, true);
  const pe = code >= 15 && code <= 21;
  const inquiry = kind === "peGet" || kind === "peSet";
  const bytes = (p: string, n: string): Uint8Array => {
    const len = v.getUint32(ev + EVENT.off[n]!, true);
    return len === 0 ? new Uint8Array(0) : readBytes(v.getUint32(ev + EVENT.off[p]!, true), len);
  };
  return {
    kind,
    muid: kind === "discoveryEnded" || kind === "rejected" ? null : v.getUint32(ev + EVENT.off["muid"]!, true),
    requestId: pe ? v.getUint8(ev + EVENT.off["request_id"]!) : null,
    chunkNumber: pe ? v.getUint16(ev + EVENT.off["chunk_number"]!, true) : null,
    chunkTotal: pe ? v.getUint16(ev + EVENT.off["chunk_total"]!, true) : null,
    resource: inquiry ? cString(ev + EVENT.off["resource"]!) : null,
    resId: inquiry ? cString(ev + EVENT.off["res_id"]!) : null,
    header: pe ? bytes("header", "header_len") : new Uint8Array(0),
    data: pe ? bytes("data", "data_len") : new Uint8Array(0),
    profile: ci !== undefined && PROFILE_KINDS.has(kind) ? readProfileEvent(ci, kind) : null,
    process: ci !== undefined && PROCESS_KINDS.has(kind) ? readProcessEvent(ci) : null,
  };
}

/**
 * The ZmidiCi storage of a Device, for the Property Exchange engine (pe.ts). Not in the public names
 * of the package.
 */
export let ciHandle: (dev: CiDevice) => number;

/** A MIDI-CI Device (4 peers, 8 Profiles, 4 Property Exchange requests). */
export class CiDevice {
  static {
    ciHandle = (dev) => dev.#b.main;
  }

  readonly #b = new Blocks(this);
  readonly #listLen: number;
  /** The message buffer: a pending PE request points into it until `peReply` or the next `receive`. */
  #msg = 0;
  #msgCap = 0;

  /** A Device with the configuration `options`. */
  constructor(options: CiDeviceOptions) {
    const ci = this.#b.add(alloc(sizeOf("zmidi_ci_size")));
    const m = options.manufacturer;
    let manufacturer: number[];
    if (m.length === 1) {
      manufacturer = [m[0]!, 0, 0];
    } else if (m.length === 3 && m[0] === 0) {
      manufacturer = [...m];
    } else {
      throw new InvalidValueError("a manufacturer ID has 1 byte, or 3 bytes that start with 0x00");
    }
    const revision = options.revision ?? new Uint8Array(4);
    if (revision.length !== 4) {
      throw new InvalidValueError("a revision has 4 bytes");
    }
    const list = options.resourceList ?? new TextEncoder().encode("[]");
    this.#listLen = list.length;
    const pid = options.productInstanceId === undefined ? 0 : this.#b.add(allocCString(options.productInstanceId));
    const pl = this.#b.add(alloc(list.length));
    writeBytes(pl, list);
    const seed = int("seed", options.seed ?? 1, 0xffffffff);
    scoped([CONFIG.size], (pc) => {
      const v = view();
      const o = CONFIG.off;
      manufacturer.forEach((x, i) => v.setUint8(pc + o["manufacturer"]! + i, int("manufacturer", x, 0x7f)));
      v.setUint16(pc + o["family"]!, int("family", options.family ?? 0, 0x3fff), true);
      v.setUint16(pc + o["model"]!, int("model", options.model ?? 0, 0x3fff), true);
      revision.forEach((x, i) => v.setUint8(pc + o["revision"]! + i, int("revision", x, 0x7f)));
      v.setUint32(pc + o["max_sysex_size"]!, int("maxSysexSize", options.maxSysexSize ?? 512, 0xffffffff), true);
      if (options.functionBlock !== undefined) {
        v.setUint8(pc + o["has_function_block"]!, 1);
        v.setUint8(pc + o["function_block"]!, int("functionBlock", options.functionBlock, 0x7e));
      }
      v.setUint8(pc + o["first_group"]!, int("firstGroup", options.firstGroup ?? 0, 15));
      v.setUint8(pc + o["group_count"]!, int("groupCount", options.groupCount ?? 1, 16));
      v.setUint16(pc + o["channels_in_use"]!, int("channelsInUse", options.channelsInUse ?? 1, 0xffff), true);
      v.setUint32(pc + o["product_instance_id"]!, pid, true);
      v.setUint8(pc + o["profiles"]!, options.profiles ? 1 : 0);
      v.setUint8(pc + o["property_exchange"]!, options.propertyExchange ?? true ? 1 : 0);
      v.setUint8(pc + o["process_inquiry"]!, options.processInquiry ? 1 : 0);
      v.setUint8(pc + o["sysex_framing"]!, options.sysexFraming ? 1 : 0);
      v.setUint32(pc + o["resource_list"]!, pl, true);
      v.setUint32(pc + o["resource_list_len"]!, list.length, true);
      check(fn("zmidi_ci_init")(ci, pc, seed), "CiDevice");
    });
  }

  /** The MUID of the Device. */
  get muid(): number {
    return fn("zmidi_ci_muid")(this.#b.main) >>> 0;
  }

  /**
   * Reads one MIDI-CI message received on `group` at `nowUs` (microseconds). Gives the automatic
   * replies and the event (or null). After a "peGet", "peSet" or "peSubscription" event, answer
   * with `peReply`; after "setProfile", with `setProfileState`; after "profileDetailsInquiry",
   * with `profileDetailsReply`; after "messageReport", with `messageReportReply`. A Profile event
   * has its Profile fields in `profile`, a Process Inquiry event its fields in `process`. A
   * PeEngine reads the Property Exchange event of the last message with `PeEngine.receive`.
   */
  receive(group: number, message: Uint8Array, nowUs: number): { replies: CiMessage[]; event: CiEvent | null } {
    const ci = this.#b.main;
    const g = int("group", group, 15);
    const now = u64("nowUs", nowUs);
    if (message.length > this.#msgCap) {
      this.#b.drop(this.#msg);
      this.#msgCap = Math.max(64, message.length);
      this.#msg = this.#b.add(alloc(this.#msgCap));
    }
    writeBytes(this.#msg, message);
    return scoped([REPLY_BUFFER, 4, EVENT.size], (out, len, ev) => {
      check(fn("zmidi_ci_receive")(ci, g, this.#msg, message.length, now, out, REPLY_BUFFER, len, ev), "receive");
      return { replies: records(out, view().getUint32(len, true)), event: readEvent(ev, ci) };
    });
  }

  /** Runs the timers at `nowUs` and gives their event, or null. */
  poll(nowUs: number): CiEvent | null {
    const now = u64("nowUs", nowUs);
    return scoped([EVENT.size], (ev) => {
      check(fn("zmidi_ci_poll")(this.#b.main, now, ev), "poll");
      return readEvent(ev);
    });
  }

  /** The Discovery message for the output path `outputPathId` at `nowUs`. */
  discovery(outputPathId = 0, nowUs = 0): Uint8Array {
    const path = int("outputPathId", outputPathId, 0x7f);
    const now = u64("nowUs", nowUs);
    return this.#one((out, cap, len) => fn("zmidi_ci_discovery")(this.#b.main, path, now, out, cap, len), "discovery");
  }

  /** The Inquiry: Endpoint Information message to `peer`. */
  endpointInfoInquiry(peer: number): Uint8Array {
    const p = int("peer", peer, 0x0fffffff);
    return this.#one((out, cap, len) => fn("zmidi_ci_endpoint_info_inquiry")(this.#b.main, p, out, cap, len), "endpointInfoInquiry");
  }

  /** The Inquiry: Property Exchange Capabilities message to `peer`. */
  peCapabilitiesInquiry(peer: number): Uint8Array {
    const p = int("peer", peer, 0x0fffffff);
    return this.#one((out, cap, len) => fn("zmidi_ci_pe_capabilities_inquiry")(this.#b.main, p, out, cap, len), "peCapabilitiesInquiry");
  }

  /** An Inquiry: Get Property Data with the Header Data `header` to `peer`. Gives the Request ID and the Chunks. */
  peGet(peer: number, header: Uint8Array): { requestId: number; chunks: Uint8Array[] } {
    return this.#request("zmidi_ci_pe_get", peer, header, undefined);
  }

  /** An Inquiry: Set Property Data with `header` and `data` to `peer`. Gives the Request ID and the Chunks. */
  peSet(peer: number, header: Uint8Array, data: Uint8Array): { requestId: number; chunks: Uint8Array[] } {
    return this.#request("zmidi_ci_pe_set", peer, header, data);
  }

  /**
   * Frees the Request ID `requestId` of a peGet or peSet to `peer` that has no reply (a timeout or
   * a NAK). The last Chunk of a reply frees the ID at the next receive without this call. Not for a
   * request of a PeEngine. A MUID or Request ID out of range throws InvalidValueError.
   */
  peEnd(peer: number, requestId: number): void {
    const p = int("peer", peer, 0x0fffffff);
    const id = int("requestId", requestId, 0x7f);
    check(fn("zmidi_ci_pe_end")(this.#b.main, p, id), "peEnd");
  }

  /**
   * Answers the pending Property Exchange request with `status` (200: success) and, for a Get, the
   * Property Data `data`. Gives the reply Chunks. Throws RejectedError when no request is pending.
   */
  peReply(status: number, data: Uint8Array = new Uint8Array(0)): Uint8Array[] {
    const st = int("status", status, 0xffff);
    const need = 2 * Math.max(data.length, this.#listLen) + 64;
    return scoped([Math.max(1, data.length), need, need, 4], (pd, scratch, out, len) => {
      writeBytes(pd, data);
      check(fn("zmidi_ci_pe_reply")(this.#b.main, st, pd, data.length, scratch, need, out, need, len), "peReply");
      return records(out, view().getUint32(len, true)).map((r) => r.message);
    });
  }

  // --- Profile Configuration (M2-101 7) ---

  /**
   * Adds the local Profile `p` to the table (M2-101 7.4). Gives the Profile Added Report, and the
   * Profile Enabled Report for an enabled Profile. Throws RejectedError for a Profile that is in
   * the table or for a full table.
   */
  addProfile(p: CiProfile): CiMessage[] {
    const id = profileId(p.profile);
    return scoped([PROFILE.size], (pp) => {
      const v = view();
      const o = PROFILE.off;
      v.setUint8(pp + o["device_id"]!, int("deviceId", p.deviceId, 0x7f));
      v.setUint8(pp + o["group"]!, int("group", p.group ?? 0, 15));
      writeBytes(pp + o["profile"]!, id);
      v.setUint8(pp + o["enabled"]!, p.enabled ? 1 : 0);
      v.setUint16(pp + o["channels"]!, int("channels", p.channels ?? 0, 0x3fff), true);
      v.setUint16(pp + o["max_channels"]!, int("maxChannels", p.maxChannels ?? 0, 0x3fff), true);
      return this.#records(REPORT_BUFFER, (out, cap, len) => fn("zmidi_ci_add_profile")(this.#b.main, pp, out, cap, len), "addProfile");
    });
  }

  /**
   * Removes the local Profile `profile` at `deviceId` and the Group offset `group` (M2-101 7.5).
   * Gives the Profile Disabled Report for an enabled Profile, then the Profile Removed Report.
   * Throws RejectedError for a Profile that is not in the table.
   */
  removeProfile(deviceId: number, group: number, profile: Uint8Array): CiMessage[] {
    const d = int("deviceId", deviceId, 0x7f);
    const g = int("group", group, 15);
    return this.#withProfile(profile, (pid) =>
      this.#records(REPORT_BUFFER, (out, cap, len) => fn("zmidi_ci_remove_profile")(this.#b.main, d, g, pid, out, cap, len), "removeProfile"),
    );
  }

  /**
   * Sets the local Profile `profile` at `deviceId` and `group` to `enabled` with `channels`
   * Channels (M2-101 7.10, 7.11): the answer to a "setProfile" event (false refuses a Set Profile
   * On), or the report of a local change. Gives the Profile Enabled or Disabled Report. Throws
   * RejectedError for a Profile that is not in the table.
   */
  setProfileState(deviceId: number, group: number, profile: Uint8Array, enabled: boolean, channels: number): CiMessage {
    const d = int("deviceId", deviceId, 0x7f);
    const g = int("group", group, 15);
    const c = int("channels", channels, 0x3fff);
    const r = this.#withProfile(profile, (pid) =>
      this.#records(
        REPORT_BUFFER,
        (out, cap, len) => fn("zmidi_ci_set_profile_state")(this.#b.main, d, g, pid, enabled ? 1 : 0, c, out, cap, len),
        "setProfileState",
      ),
    );
    return r[0]!;
  }

  /**
   * Gives Reply to Profile Details Inquiry (M2-101 7.7, Table 23) for `profile` and the Inquiry
   * Target `target` with the Inquiry Target Data `data` to `peer` on `group`: the answer to a
   * "profileDetailsInquiry" event.
   */
  profileDetailsReply(peer: number, group: number, deviceId: number, profile: Uint8Array, target: number, data: Uint8Array): Uint8Array {
    const p = int("peer", peer, 0x0fffffff);
    const g = int("group", group, 15);
    const d = int("deviceId", deviceId, 0x7f);
    const t = int("target", target, 0x7f);
    return this.#withProfile(profile, (pid) =>
      this.#withBytes(data, (pd) =>
        this.#one(
          (out, cap, len) => fn("zmidi_ci_profile_details_reply")(this.#b.main, p, g, d, pid, t, pd, data.length, out, cap, len),
          "profileDetailsReply",
          data.length + 64,
        ),
      ),
    );
  }

  /**
   * Gives Profile Specific Data (M2-101 7.12, Table 28) for `profile` with `data` to `peer`
   * (0x0FFFFFFF: Broadcast) on `group`.
   */
  profileSpecificData(peer: number, group: number, deviceId: number, profile: Uint8Array, data: Uint8Array): Uint8Array {
    const p = int("peer", peer, 0x0fffffff);
    const g = int("group", group, 15);
    const d = int("deviceId", deviceId, 0x7f);
    return this.#withProfile(profile, (pid) =>
      this.#withBytes(data, (pd) =>
        this.#one(
          (out, cap, len) => fn("zmidi_ci_profile_specific_data")(this.#b.main, p, g, d, pid, pd, data.length, out, cap, len),
          "profileSpecificData",
          data.length + 64,
        ),
      ),
    );
  }

  /** Gives Profile Inquiry (M2-101 7.2, Table 17) to `peer` for the address `deviceId` on `group`. */
  profileInquiry(peer: number, group: number, deviceId: number): Uint8Array {
    const p = int("peer", peer, 0x0fffffff);
    const g = int("group", group, 15);
    const d = int("deviceId", deviceId, 0x7f);
    return this.#one((out, cap, len) => fn("zmidi_ci_profile_inquiry")(this.#b.main, p, g, d, out, cap, len), "profileInquiry");
  }

  /**
   * Gives Set Profile On (M2-101 7.8, Table 24) for `profile` with `channels` Number of Channels
   * Requested to `peer` on `group`. `maxChannels` is the Maximum Number of Channels that the peer
   * reported (default 0: not known). Throws RejectedError before a Profile Inquiry to the peer.
   */
  setProfileOn(peer: number, group: number, deviceId: number, profile: Uint8Array, channels: number, maxChannels = 0): Uint8Array {
    const p = int("peer", peer, 0x0fffffff);
    const g = int("group", group, 15);
    const d = int("deviceId", deviceId, 0x7f);
    const c = int("channels", channels, 0x3fff);
    const m = int("maxChannels", maxChannels, 0x3fff);
    return this.#withProfile(profile, (pid) =>
      this.#one((out, cap, len) => fn("zmidi_ci_set_profile_on")(this.#b.main, p, g, d, pid, c, m, out, cap, len), "setProfileOn"),
    );
  }

  /**
   * Gives Set Profile Off (M2-101 7.9, Table 25) for `profile` to `peer` on `group`. Throws
   * RejectedError before a Profile Inquiry to the peer.
   */
  setProfileOff(peer: number, group: number, deviceId: number, profile: Uint8Array): Uint8Array {
    const p = int("peer", peer, 0x0fffffff);
    const g = int("group", group, 15);
    const d = int("deviceId", deviceId, 0x7f);
    return this.#withProfile(profile, (pid) =>
      this.#one((out, cap, len) => fn("zmidi_ci_set_profile_off")(this.#b.main, p, g, d, pid, out, cap, len), "setProfileOff"),
    );
  }

  /**
   * Gives Profile Details Inquiry (M2-101 7.6, Table 22) for `profile` and the Inquiry Target
   * `target` to `peer` on `group`.
   */
  profileDetailsInquiry(peer: number, group: number, deviceId: number, profile: Uint8Array, target: number): Uint8Array {
    const p = int("peer", peer, 0x0fffffff);
    const g = int("group", group, 15);
    const d = int("deviceId", deviceId, 0x7f);
    const t = int("target", target, 0x7f);
    return this.#withProfile(profile, (pid) =>
      this.#one((out, cap, len) => fn("zmidi_ci_profile_details_inquiry")(this.#b.main, p, g, d, pid, t, out, cap, len), "profileDetailsInquiry"),
    );
  }

  // --- Process Inquiry (M2-101 9) ---

  /** Gives Inquiry: Process Inquiry Capabilities (M2-101 9.2, Table 40) to `peer`. */
  piCapabilitiesInquiry(peer: number): Uint8Array {
    const p = int("peer", peer, 0x0fffffff);
    return this.#one((out, cap, len) => fn("zmidi_ci_pi_capabilities_inquiry")(this.#b.main, p, out, cap, len), "piCapabilitiesInquiry");
  }

  /**
   * Gives Inquiry: MIDI Message Report (M2-101 9.5, Table 43) with the bitmaps `inquiry` to `peer`
   * for `deviceId` (0x7F when System Messages are requested) on `group`, and opens the report.
   */
  messageReportInquiry(peer: number, group: number, deviceId: number, inquiry: CiReport): Uint8Array {
    const p = int("peer", peer, 0x0fffffff);
    const g = int("group", group, 15);
    const d = int("deviceId", deviceId, 0x7f);
    return scoped([REPORT.size], (pr) => {
      writeReport(pr, inquiry);
      return this.#one((out, cap, len) => fn("zmidi_ci_message_report_inquiry")(this.#b.main, p, g, d, pr, out, cap, len), "messageReportInquiry");
    });
  }

  /** True while a MIDI Message Report from `peer` is open: after `messageReportInquiry`, until End of MIDI Message Report. */
  reportOpen(peer: number): boolean {
    const p = int("peer", peer, 0x0fffffff);
    return fn("zmidi_ci_report_open")(this.#b.main, p) !== 0;
  }

  /**
   * Gives Reply to MIDI Message Report (M2-101 9.6, Table 45) to `peer` on `group` with the
   * Device ID of the inquiry and the bitmaps that are in both `requested` and `supported`: the
   * answer to a "messageReport" event. Then send the declared messages with their current values
   * (M2-101 9.7) and call `messageReportEnd`.
   */
  messageReportReply(peer: number, group: number, deviceId: number, requested: CiReport, supported: CiReport): Uint8Array {
    const p = int("peer", peer, 0x0fffffff);
    const g = int("group", group, 15);
    const d = int("deviceId", deviceId, 0x7f);
    return scoped([REPORT.size, REPORT.size], (pr, ps) => {
      writeReport(pr, requested);
      writeReport(ps, supported);
      return this.#one((out, cap, len) => fn("zmidi_ci_message_report_reply")(this.#b.main, p, g, d, pr, ps, out, cap, len), "messageReportReply");
    });
  }

  /** Gives End of MIDI Message Report (M2-101 9.8, Table 46) to `peer` with the Device ID of the inquiry on `group`. */
  messageReportEnd(peer: number, group: number, deviceId: number): Uint8Array {
    const p = int("peer", peer, 0x0fffffff);
    const g = int("group", group, 15);
    const d = int("deviceId", deviceId, 0x7f);
    return this.#one((out, cap, len) => fn("zmidi_ci_message_report_end")(this.#b.main, p, g, d, out, cap, len), "messageReportEnd");
  }

  /**
   * Stops a MIDI Message Report to `peer`: gives a NAK with Status Code 0x20 (Responder
   * terminates) or, when `error`, 0x40 (M2-101 9.8.1) on `group`.
   */
  abortReport(peer: number, group: number, deviceId: number, error = false): Uint8Array {
    const p = int("peer", peer, 0x0fffffff);
    const g = int("group", group, 15);
    const d = int("deviceId", deviceId, 0x7f);
    return this.#one((out, cap, len) => fn("zmidi_ci_abort_report")(this.#b.main, p, g, d, error ? 1 : 0, out, cap, len), "abortReport");
  }

  /** Frees the WASM memory of the Device now. */
  free(): void {
    this.#msg = 0;
    this.#msgCap = 0;
    this.#b.release();
  }

  /** Runs `call` with an output buffer of `cap` bytes for one record; gives the message of the record. */
  #one(call: (out: number, cap: number, len: number) => number, what: string, cap = 128): Uint8Array {
    return this.#records(cap, call, what)[0]?.message ?? new Uint8Array(0);
  }

  /** Runs `call` with an output buffer of `cap` bytes; gives its records. */
  #records(cap: number, call: (out: number, cap: number, len: number) => number, what: string): CiMessage[] {
    return scoped([cap, 4], (out, len) => {
      check(call(out, cap, len), what);
      return records(out, view().getUint32(len, true));
    });
  }

  /** Runs `body` with the checked Profile ID `profile` in WASM memory. */
  #withProfile<T>(profile: Uint8Array, body: (ptr: number) => T): T {
    const id = profileId(profile);
    return scoped([5], (pid) => {
      writeBytes(pid, id);
      return body(pid);
    });
  }

  /** Runs `body` with a copy of `data` in WASM memory. */
  #withBytes<T>(data: Uint8Array, body: (ptr: number) => T): T {
    return scoped([Math.max(1, data.length)], (pd) => {
      writeBytes(pd, data);
      return body(pd);
    });
  }

  #request(name: string, peer: number, header: Uint8Array, data: Uint8Array | undefined): { requestId: number; chunks: Uint8Array[] } {
    const p = int("peer", peer, 0x0fffffff);
    const body = data ?? new Uint8Array(0);
    let cap = 2 * (header.length + body.length) + 256;
    for (;;) {
      const r = scoped([Math.max(1, header.length), Math.max(1, body.length), cap, 4, 4], (ph, pd, out, len, id) => {
        writeBytes(ph, header);
        writeBytes(pd, body);
        const st =
          data === undefined
            ? fn(name)(this.#b.main, p, ph, header.length, out, cap, len, id)
            : fn(name)(this.#b.main, p, ph, header.length, pd, body.length, out, cap, len, id);
        if (st === Status.BUFFER_TOO_SMALL) {
          return undefined;
        }
        check(st, name === "zmidi_ci_pe_get" ? "peGet" : "peSet");
        return { requestId: view().getUint8(id), chunks: records(out, view().getUint32(len, true)).map((x) => x.message) };
      });
      if (r !== undefined) {
        return r;
      }
      cap *= 2;
    }
  }
}
