// The Property Exchange engine of zmidi-ci-pe through the C ABI 0.3 (zmidi_pe_*; M2-103): Property
// Exchange with many Chunks, Flow Control and Subscriptions over a CiDevice. The program gives each
// received message to CiDevice.receive first, then calls PeEngine.receive with the same device. The
// start calls (reply, update, request, subscribe, unsubscribe) start a transaction and `send` writes
// its Chunks; the calls that queue a control message (replyStatus, refresh, wait, abort, corrupt)
// need `writePending`. The encodings are "ASCII" and "Mcoded7"; "zlib+Mcoded7" is not inflated.

import { InvalidValueError, MidiError, RejectedError } from "./errors.js";
import { CiDevice, ciHandle } from "./ci.js";
import { Blocks, Status, alloc, allocCString, fn, free as freeBlock, int, layout, readBytes, scoped, sizeOf, u64, view, writeBytes } from "./wasm.js";

/** The encoding of Property Data (M2-103 Table 12). */
export type PeEncoding = "ASCII" | "Mcoded7" | "zlib+Mcoded7";

/** The role of this Device in a transaction or a subscription. */
export type PeRole = "initiator" | "responder";

/** The kind of a PeEngine event (one for each PeEvent of zmidi-ci-pe). */
export type PeEventKind = "message" | "ready" | "resend" | "closed" | "subscriptionStarted" | "subscriptionEnded" | "invalidated";

/** Why a transaction closed. */
export type PeCloseReason =
  | "timeout"
  | "terminated"
  | "peerTimeout"
  | "outOfSequence"
  | "nak"
  | "badReply"
  | "refused"
  | "invalidated"
  | "resendFailed";

/** The "command" of a Subscription message (M2-103 11). */
export type PeCommand = "start" | "partial" | "full" | "notify" | "end";

/** Why the reply to a "start" gave no subscription (REQ-PE-SUB-006). */
export type PeStartError = "full" | "noSubscribeId" | "notSuccess" | "notPermitted" | "invalid";

/** One entry of the Resource table (M2-103 Table 14). */
export interface PeResource {
  /** "resource": 1 to 36 characters A-Z, a-z, 0-9 with an optional "X-" prefix. */
  readonly resource: string;
  /** "canGet" (default true). */
  readonly canGet?: boolean;
  /** "canSet" (default "none"). */
  readonly canSet?: "none" | "full" | "partial";
  /** "canSubscribe" (default false). */
  readonly canSubscribe?: boolean;
  /** "requireResId" (default false). */
  readonly requireResId?: boolean;
  /** "fcOnGet" (default false). */
  readonly fcOnGet?: boolean;
  /** "fcOnSet" (default false). */
  readonly fcOnSet?: boolean;
  /** "encodings" (default ["ASCII"]). */
  readonly encodings?: readonly PeEncoding[];
  /** "canPaginate" (default false). */
  readonly canPaginate?: boolean;
  /** The one "mediaType" ("type/subtype"); absent for "application/json". */
  readonly mediaType?: string;
  /** "schema" as JSON text. */
  readonly schema?: Uint8Array;
  /** Write every Property, also the ones with their default value (default false). */
  readonly writeDefaults?: boolean;
}

/** One event of a PeEngine. The fields that the kind does not use are null or empty. */
export interface PeEvent {
  readonly kind: PeEventKind;
  /** The MUID of the peer ("invalidated": the invalidated MUID). */
  readonly muid: number | null;
  /** Request ID of the transaction. */
  readonly requestId: number | null;
  /** The role of this Device. */
  readonly role: PeRole | null;
  /** The kind of the transaction. */
  readonly txn: "get" | "set" | "subscription" | null;
  /** "message": the end of this Chunk (M2-103 5.3). */
  readonly end: "more" | "complete" | "incomplete" | null;
  /** "message": the encoding of the Property Data. */
  readonly encoding: PeEncoding | null;
  /** "message": the "command" of the header. */
  readonly command: PeCommand | null;
  /** "message": the "status" of the header; "closed" with reason "refused": the status. */
  readonly status: number | null;
  /** "resend": the number of the Chunk to send again. */
  readonly chunk: number | null;
  /** "message": the index of the Resource in the Resource table. */
  readonly resourceIndex: number | null;
  /** "message": the subscription entry of the message; "subscriptionStarted": the new entry. */
  readonly subscription: number | null;
  /** "message" of the reply to a "start": the new subscription entry. */
  readonly started: number | null;
  /** "message" of the reply to a "start" with no new entry: why. */
  readonly startError: PeStartError | null;
  /** "closed": why. */
  readonly reason: PeCloseReason | null;
  /** "closed" with reason "nak": the NAK Status Code. */
  readonly nakCode: number | null;
  /** "closed" with reason "nak": the NAK Status Data. */
  readonly nakData: number | null;
  /** "invalidated": the number of removed subscriptions (see `takeEnded`). */
  readonly ended: number | null;
  /** "message": the decoded "resource"; "subscriptionEnded": the entry resource. */
  readonly resource: string | null;
  /** "message": the decoded "resId"; "subscriptionEnded": the entry resId. */
  readonly resId: string | null;
  /** "message": the decoded "subscribeId"; "subscriptionEnded": the entry subscribeId. */
  readonly subscribeId: string | null;
  /** "message": the Header Data (first Chunk only). */
  readonly header: Uint8Array;
  /** "message": the decoded Property Data of this Chunk. */
  readonly data: Uint8Array;
}

/** A copy of one subscription table entry. */
export interface PeSubscription {
  /** The MUID of the peer. */
  readonly muid: number;
  /** The role of this Device: "initiator" sent the "start". */
  readonly role: PeRole;
  /** "subscribeId". */
  readonly subscribeId: string;
  /** "resource". */
  readonly resource: string;
  /** "resId", or null. */
  readonly resId: string | null;
}

/** The options of `PeEngine.reply`. */
export interface PeReplyOptions {
  /** The status (default 200). */
  readonly status?: number;
  /** Bytes of the encoded Property Data (default 0); null when not known. */
  readonly length?: number | null;
  /** The encoding: the one of the inquiry (default "ASCII"). */
  readonly encoding?: PeEncoding;
  /** "mediaType" of Property Data that is not JSON. */
  readonly mediaType?: string;
  /** "totalCount" (needed for a Resource with "canPaginate"). */
  readonly totalCount?: number;
  /** "cacheTime" in seconds. */
  readonly cacheTime?: number;
  /** Clock time in microseconds (default 0). */
  readonly nowUs?: number;
}

/** The options of `PeEngine.request`. */
export interface PeRequestOptions {
  /** "resId". */
  readonly resId?: string;
  /** "mediaType" of the Property Data of a Set that is not JSON. */
  readonly mediaType?: string;
  /** "offset"; with "limit" (M2-103 8.5). */
  readonly offset?: number;
  /** "limit"; with "offset". */
  readonly limit?: number;
  /** "setPartial" (default false). */
  readonly setPartial?: boolean;
  /** The encoding of the data that this Device sends; for a Get, the encoding it asks for. */
  readonly encoding?: PeEncoding;
  /** Bytes of the encoded Property Data (default 0); null when not known. */
  readonly length?: number | null;
  /** The capabilities of the Resource in the ResourceList of the peer. */
  readonly caps?: PeResource;
  /** Clock time in microseconds (default 0). */
  readonly nowUs?: number;
}

/** The options of `PeEngine.subscribe`. */
export interface PeSubscribeOptions {
  /** "resId". */
  readonly resId?: string;
  /** The encoding of the updates (default "ASCII"). */
  readonly encoding?: PeEncoding;
  /** The capabilities of the Resource in the ResourceList of the peer. */
  readonly caps?: PeResource;
  /** Clock time in microseconds (default 0). */
  readonly nowUs?: number;
}

/** The options of `PeEngine.update`. */
export interface PeUpdateOptions {
  /** Bytes of the encoded Property Data (default 0); null when not known. */
  readonly length?: number | null;
  /** The encoding (default "ASCII"). */
  readonly encoding?: PeEncoding;
  /** Clock time in microseconds (default 0). */
  readonly nowUs?: number;
}

/** ZMIDI_PE_ID_LEN. */
const ID_LEN = 37;
/** ZMIDI_PE_PENDING_MAX and more: one control message. */
const MESSAGE_MAX = 512;
/** SIZE_MAX of wasm32: a length that is not known. */
const UNKNOWN = 0xffffffff;

const ENCODINGS: readonly PeEncoding[] = ["ASCII", "Mcoded7", "zlib+Mcoded7"];
const ROLES: readonly PeRole[] = ["initiator", "responder"];
const TXNS = ["get", "set", "subscription"] as const;
const ENDS = ["more", "complete", "incomplete"] as const;
const COMMANDS: readonly (PeCommand | null)[] = [null, "start", "partial", "full", "notify", "end"];
const KINDS: readonly (PeEventKind | null)[] = [
  null,
  "message",
  "ready",
  "resend",
  "closed",
  "subscriptionStarted",
  "subscriptionEnded",
  "invalidated",
];
const REASONS: readonly (PeCloseReason | null)[] = [
  null,
  "timeout",
  "terminated",
  "peerTimeout",
  "outOfSequence",
  "nak",
  "badReply",
  "refused",
  "invalidated",
  "resendFailed",
];
const START_ERRORS: readonly (PeStartError | null)[] = [null, "full", "noSubscribeId", "notSuccess", "notPermitted", "invalid"];
/** ZmidiPeError, by value. */
const ERRORS = [
  "none",
  "device refused",
  "write failed",
  "header refused",
  "subscription rule refused",
  "no such transaction",
  "no free transaction slot",
  "peer has its maximum of open requests",
  "deadline passed",
  "not permitted in this state",
  "refused by the ResourceList of the peer",
  "mediaType needed",
  "totalCount needed",
  "header too large",
  "buffer size",
  "pending queue full",
  "invalid wait",
  "invalid $ref",
];

const RESOURCE = layout([
  ["resource", "ptr"],
  ["can_get", "bool"],
  ["can_set", "u8"],
  ["can_subscribe", "bool"],
  ["require_res_id", "bool"],
  ["fc_on_get", "bool"],
  ["fc_on_set", "bool"],
  ["can_paginate", "bool"],
  ["write_defaults", "bool"],
  ["encodings", "u8"],
  ["media_type", "ptr"],
  ["schema", "ptr"],
  ["schema_len", "usize"],
]);

const HEADER = layout([
  ["resource", "ptr"],
  ["res_id", "ptr"],
  ["media_type", "ptr"],
  ["has_offset", "bool"],
  ["offset", "u32"],
  ["limit", "u32"],
  ["set_partial", "bool"],
  ["encoding", "u8"],
  ["length", "usize"],
]);

const EVENT = layout([
  ["kind", "i32"],
  ["muid", "u32"],
  ["request_id", "u8"],
  ["role", "i32"],
  ["txn", "i32"],
  ["end", "i32"],
  ["encoding", "i32"],
  ["command", "i32"],
  ["status", "u16"],
  ["chunk", "u16"],
  ["resource_index", "i32"],
  ["subscription", "i32"],
  ["started", "i32"],
  ["start_error", "i32"],
  ["reason", "i32"],
  ["nak_code", "u8"],
  ["nak_data", "u8"],
  ["ended", "u32"],
  ["header", "ptr"],
  ["header_len", "usize"],
  ["data", "ptr"],
  ["data_len", "usize"],
  ["resource", ["u8", ID_LEN]],
  ["res_id", ["u8", ID_LEN]],
  ["subscribe_id", ["u8", ID_LEN]],
]);

const SUBSCRIPTION = layout([
  ["muid", "u32"],
  ["role", "i32"],
  ["subscribe_id", ["u8", ID_LEN]],
  ["resource", ["u8", ID_LEN]],
  ["res_id", ["u8", ID_LEN]],
]);

function encodingValue(e: PeEncoding | undefined): number {
  const i = ENCODINGS.indexOf(e ?? "ASCII");
  if (i < 0) {
    throw new InvalidValueError(`an encoding is "ASCII", "Mcoded7" or "zlib+Mcoded7", not ${String(e)}`);
  }
  return i;
}

function roleValue(r: PeRole): number {
  const i = ROLES.indexOf(r);
  if (i < 0) {
    throw new InvalidValueError(`a role is "initiator" or "responder", not ${String(r)}`);
  }
  return i;
}

function peer(muid: number): number {
  return int("peer", muid, 0x0fffffff);
}

function lengthValue(len: number | null | undefined): number {
  return len === null ? UNKNOWN : int("length", len ?? 0, UNKNOWN - 1);
}

function cText(ptr: number): string {
  const b = readBytes(ptr, ID_LEN);
  const end = b.indexOf(0);
  return new TextDecoder().decode(b.subarray(0, end < 0 ? b.length : end));
}

function orNull(s: string): string | null {
  return s === "" ? null : s;
}

/** Writes the ZmidiPeResource of `r` at `ptr`; its texts go into blocks from `keep`. */
function writeResource(ptr: number, r: PeResource, keep: (p: number) => number): void {
  const can = ["none", "full", "partial"].indexOf(r.canSet ?? "none");
  if (can < 0) {
    throw new InvalidValueError(`canSet is "none", "full" or "partial", not ${String(r.canSet)}`);
  }
  let encodings = 0;
  for (const e of r.encodings ?? ["ASCII"]) {
    encodings |= 1 << encodingValue(e);
  }
  const o = RESOURCE.off;
  const name = keep(allocCString(r.resource));
  const media = r.mediaType === undefined ? 0 : keep(allocCString(r.mediaType));
  const schema = r.schema === undefined ? 0 : keep(alloc(Math.max(1, r.schema.length)));
  if (r.schema !== undefined) {
    writeBytes(schema, r.schema);
  }
  const v = view();
  v.setUint32(ptr + o["resource"]!, name, true);
  v.setUint8(ptr + o["can_get"]!, (r.canGet ?? true) ? 1 : 0);
  v.setUint8(ptr + o["can_set"]!, can);
  v.setUint8(ptr + o["can_subscribe"]!, r.canSubscribe ? 1 : 0);
  v.setUint8(ptr + o["require_res_id"]!, r.requireResId ? 1 : 0);
  v.setUint8(ptr + o["fc_on_get"]!, r.fcOnGet ? 1 : 0);
  v.setUint8(ptr + o["fc_on_set"]!, r.fcOnSet ? 1 : 0);
  v.setUint8(ptr + o["can_paginate"]!, r.canPaginate ? 1 : 0);
  v.setUint8(ptr + o["write_defaults"]!, r.writeDefaults ? 1 : 0);
  v.setUint8(ptr + o["encodings"]!, encodings);
  v.setUint32(ptr + o["media_type"]!, media, true);
  v.setUint32(ptr + o["schema"]!, schema, true);
  v.setUint32(ptr + o["schema_len"]!, r.schema?.length ?? 0, true);
}

function readEvent(ev: number): PeEvent | null {
  const v = view();
  const o = EVENT.off;
  const i32 = (n: string): number => v.getInt32(ev + o[n]!, true);
  const kind = KINDS[i32("kind")] ?? null;
  if (kind === null) {
    return null;
  }
  const message = kind === "message";
  const txnLike = kind !== "subscriptionStarted" && kind !== "invalidated";
  const roleLike = txnLike;
  const bytes = (p: string, n: string): Uint8Array => {
    const len = v.getUint32(ev + o[n]!, true);
    return len === 0 ? new Uint8Array(0) : readBytes(v.getUint32(ev + o[p]!, true), len);
  };
  const index = (n: string): number | null => (i32(n) < 0 ? null : i32(n));
  const status = v.getUint16(ev + o["status"]!, true);
  const reason = REASONS[i32("reason")] ?? null;
  return {
    kind,
    muid: kind === "subscriptionStarted" ? null : v.getUint32(ev + o["muid"]!, true),
    requestId: kind === "subscriptionStarted" || kind === "invalidated" || kind === "subscriptionEnded" ? null : v.getUint8(ev + o["request_id"]!),
    role: roleLike ? ROLES[i32("role")]! : null,
    txn: message || kind === "subscriptionEnded" ? TXNS[i32("txn")]! : null,
    end: message ? ENDS[i32("end")]! : null,
    encoding: message ? ENCODINGS[i32("encoding")]! : null,
    command: COMMANDS[i32("command")] ?? null,
    status: (message || reason === "refused") && status !== 0 ? status : null,
    chunk: kind === "resend" ? v.getUint16(ev + o["chunk"]!, true) : null,
    resourceIndex: index("resource_index"),
    subscription: index("subscription"),
    started: index("started"),
    startError: START_ERRORS[i32("start_error")] ?? null,
    reason,
    nakCode: reason === "nak" ? v.getUint8(ev + o["nak_code"]!) : null,
    nakData: reason === "nak" ? v.getUint8(ev + o["nak_data"]!) : null,
    ended: kind === "invalidated" ? v.getUint32(ev + o["ended"]!, true) : null,
    resource: orNull(cText(ev + o["resource"]!)),
    resId: orNull(cText(ev + o["res_id"]!)),
    subscribeId: orNull(cText(ev + o["subscribe_id"]!)),
    header: bytes("header", "header_len"),
    data: bytes("data", "data_len"),
  };
}

function readSubscription(p: number): PeSubscription {
  const v = view();
  const o = SUBSCRIPTION.off;
  return {
    muid: v.getUint32(p + o["muid"]!, true),
    role: ROLES[v.getInt32(p + o["role"]!, true)]!,
    subscribeId: cText(p + o["subscribe_id"]!),
    resource: cText(p + o["resource"]!),
    resId: orNull(cText(p + o["res_id"]!)),
  };
}

/** The records `[group][len lo][len hi][bytes]` of `len` bytes at `ptr`: their messages. */
function messages(ptr: number, len: number): Uint8Array[] {
  const b = readBytes(ptr, len);
  const out: Uint8Array[] = [];
  let at = 0;
  while (at + 3 <= b.length) {
    const n = b[at + 1]! | (b[at + 2]! << 8);
    out.push(b.slice(at + 3, at + 3 + n));
    at += 3 + n;
  }
  return out;
}

/**
 * The Property Exchange engine of zmidi-ci-pe over a CiDevice (4 transactions with 256 header bytes,
 * 4 subscriptions, 4 peers with a known ResourceList, 16 queued control messages).
 */
export class PeEngine {
  readonly #b = new Blocks(this);

  /**
   * An engine with the Resource table `resources` (at most 16 entries). Throws InvalidValueError for
   * an entry that is not valid (M2-103 8.1 "resource", a "mediaType" that is not "type/subtype", no
   * encoding, more than 512 bytes of ResourceList JSON).
   */
  constructor(resources: readonly PeResource[] = []) {
    const pe = this.#b.add(alloc(sizeOf("zmidi_pe_size")));
    if (resources.length > 16) {
      throw new InvalidValueError(`a Resource table has at most 16 entries, not ${resources.length}`);
    }
    scoped([Math.max(1, RESOURCE.size * resources.length)], (table) => {
      resources.forEach((r, i) => writeResource(table + RESOURCE.size * i, r, (p) => this.#b.add(p)));
      this.#check(fn("zmidi_pe_init")(pe, table, resources.length), "PeEngine");
    });
  }

  /**
   * Gives the last event of `dev` (from CiDevice.receive) to the engine at `nowUs`. Gives the engine
   * event, or null. An event that the engine took is not given again. A refusal (for example a full
   * queue: call `writePending` first) throws and changes nothing.
   */
  receive(dev: CiDevice, nowUs: number): PeEvent | null {
    const now = u64("nowUs", nowUs);
    for (let cap = 1024; ; cap *= 2) {
      const r = scoped([cap, EVENT.size], (scratch, ev) => {
        const st = fn("zmidi_pe_receive")(this.#b.main, ciHandle(dev), now, scratch, cap, ev);
        if (st === Status.BUFFER_TOO_SMALL && cap < 0x10000) {
          return undefined;
        }
        this.#check(st, "receive");
        return { event: readEvent(ev) };
      });
      if (r !== undefined) {
        return r.event;
      }
    }
  }

  /** Runs the deadlines at `nowUs`. Gives one event, or null; call it until it gives null. */
  poll(dev: CiDevice, nowUs: number): PeEvent | null {
    const now = u64("nowUs", nowUs);
    return scoped([EVENT.size], (ev) => {
      this.#check(fn("zmidi_pe_poll")(this.#b.main, ciHandle(dev), now, ev), "poll");
      return readEvent(ev);
    });
  }

  /** Writes the next queued control message (status reply, ACK, NAK, a Get after a "notify"), or gives null. */
  writePending(dev: CiDevice, nowUs: number): Uint8Array | null {
    const now = u64("nowUs", nowUs);
    return scoped([MESSAGE_MAX, 4], (out, len) => {
      const st = fn("zmidi_pe_write_pending")(this.#b.main, ciHandle(dev), now, out, MESSAGE_MAX, len);
      if (st === Status.NO_PENDING) {
        return null;
      }
      this.#check(st, "writePending");
      return messages(out, view().getUint32(len, true))[0]!;
    });
  }

  /**
   * Starts the reply to the complete inquiry `requestId` of `muid`. `send` writes the Chunks. A reply
   * after the deadline throws RejectedError and closes the transaction.
   */
  reply(dev: CiDevice, muid: number, requestId: number, options: PeReplyOptions = {}): void {
    const p = peer(muid);
    const id = int("requestId", requestId, 0x7f);
    const status = int("status", options.status ?? 200, 0xffff);
    const enc = encodingValue(options.encoding);
    const len = lengthValue(options.length);
    const total = options.totalCount === undefined ? -1n : BigInt(int("totalCount", options.totalCount, 0xffffffff));
    const cache = options.cacheTime === undefined ? -1n : BigInt(int("cacheTime", options.cacheTime, 0xffffffff));
    const now = u64("nowUs", options.nowUs ?? 0);
    this.#withText(options.mediaType, (mt) =>
      this.#check(fn("zmidi_pe_reply")(this.#b.main, ciHandle(dev), p, id, status, enc, mt, total, cache, len, now), "reply"),
    );
  }

  /** Replies to the open inquiry `requestId` of `muid` with `status` and no Property Data, and closes it. */
  replyStatus(dev: CiDevice, muid: number, requestId: number, status: number, nowUs = 0): void {
    const args = [peer(muid), int("requestId", requestId, 0x7f), int("status", status, 0xffff), u64("nowUs", nowUs)] as const;
    this.#check(fn("zmidi_pe_reply_status")(this.#b.main, ciHandle(dev), ...args), "replyStatus");
  }

  /**
   * Starts a "partial", "full" or "notify" Subscription message for the subscription `index` in which
   * this Device is the responder. Gives its Request ID; `send` (role "initiator") writes the Chunks.
   */
  update(dev: CiDevice, index: number, command: "partial" | "full" | "notify", options: PeUpdateOptions = {}): number {
    const c = ["partial", "full", "notify"].indexOf(command);
    if (c < 0) {
      throw new InvalidValueError(`an update is "partial", "full" or "notify", not ${String(command)}`);
    }
    const args = [int("index", index, 0xffff), c + 2, encodingValue(options.encoding), lengthValue(options.length), u64("nowUs", options.nowUs ?? 0)] as const;
    return scoped([4], (id) => {
      this.#check(fn("zmidi_pe_update")(this.#b.main, ciHandle(dev), ...args, id), "update");
      return view().getUint8(id);
    });
  }

  /** The ResourceList JSON of the Resource table, for the reply to a Get of "ResourceList". */
  resourceList(): Uint8Array {
    for (let cap = 1024; ; cap *= 2) {
      const r = scoped([cap, 4], (out, len) => {
        const st = fn("zmidi_pe_resource_list")(this.#b.main, out, cap, len);
        if (st === Status.BUFFER_TOO_SMALL) {
          return undefined;
        }
        this.#check(st, "resourceList");
        return readBytes(out, view().getUint32(len, true));
      });
      if (r !== undefined) {
        return r;
      }
    }
  }

  /**
   * Starts a Get or Set inquiry to `muid` for `resource`. Gives the Request ID and whether this
   * Device has no ResourceList of the peer yet; `send` writes the Chunks. A refusal by the
   * capabilities in `options.caps` throws RejectedError with the status.
   */
  request(dev: CiDevice, muid: number, kind: "get" | "set", resource: string, options: PeRequestOptions = {}): { requestId: number; resourceListMissing: boolean } {
    const k = ["get", "set"].indexOf(kind);
    if (k < 0) {
      throw new InvalidValueError(`a request is "get" or "set", not ${String(kind)}`);
    }
    if ((options.offset === undefined) !== (options.limit === undefined)) {
      throw new InvalidValueError("offset and limit come together (M2-103 8.5)");
    }
    const p = peer(muid);
    const now = u64("nowUs", options.nowUs ?? 0);
    return this.#withHeader(resource, options, (h, caps) =>
      scoped([4, 4], (id, missing) => {
        this.#check(fn("zmidi_pe_request")(this.#b.main, ciHandle(dev), p, k, h, caps, now, id, missing), "request");
        return { requestId: view().getUint8(id), resourceListMissing: view().getUint8(missing) !== 0 };
      }),
    );
  }

  /** Starts the Get of the ResourceList of `muid` and writes it. */
  requestResourceList(dev: CiDevice, muid: number, nowUs = 0): { requestId: number; message: Uint8Array } {
    const p = peer(muid);
    const now = u64("nowUs", nowUs);
    return scoped([MESSAGE_MAX, 4, 4], (out, len, id) => {
      this.#check(fn("zmidi_pe_request_resource_list")(this.#b.main, ciHandle(dev), p, now, out, MESSAGE_MAX, len, id), "requestResourceList");
      return { requestId: view().getUint8(id), message: messages(out, view().getUint32(len, true))[0]! };
    });
  }

  /**
   * Starts a "start" Subscription message to `muid` for `resource`. Gives the Request ID and whether
   * this Device has no ResourceList of the peer yet; `send` writes it with no data.
   */
  subscribe(dev: CiDevice, muid: number, resource: string, options: PeSubscribeOptions = {}): { requestId: number; resourceListMissing: boolean } {
    const p = peer(muid);
    const now = u64("nowUs", options.nowUs ?? 0);
    const req: PeRequestOptions = { encoding: options.encoding, ...(options.resId === undefined ? {} : { resId: options.resId }), ...(options.caps === undefined ? {} : { caps: options.caps }) };
    return this.#withHeader(resource, req, (h, caps) =>
      scoped([4, 4], (id, missing) => {
        this.#check(fn("zmidi_pe_subscribe")(this.#b.main, ciHandle(dev), p, h, caps, now, id, missing), "subscribe");
        return { requestId: view().getUint8(id), resourceListMissing: view().getUint8(missing) !== 0 };
      }),
    );
  }

  /** Starts an "end" Subscription message for the subscription `index` and removes the entry. Gives the Request ID. */
  unsubscribe(dev: CiDevice, index: number, nowUs = 0): number {
    const i = int("index", index, 0xffff);
    const now = u64("nowUs", nowUs);
    return scoped([4], (id) => {
      this.#check(fn("zmidi_pe_unsubscribe")(this.#b.main, ciHandle(dev), i, now, id), "unsubscribe");
      return view().getUint8(id);
    });
  }

  /** Asks again for the Property Data of the subscription `index`; `writePending` writes the Get. */
  refresh(index: number): void {
    this.#check(fn("zmidi_pe_refresh")(this.#b.main, int("index", index, 0xffff)), "refresh");
  }

  /** The Property Data bytes that the next Chunk of the transaction takes. */
  dataRoom(dev: CiDevice, muid: number, requestId: number, role: PeRole): number {
    const args = [peer(muid), int("requestId", requestId, 0x7f), roleValue(role)] as const;
    return scoped([4], (room) => {
      this.#check(fn("zmidi_pe_data_room")(this.#b.main, ciHandle(dev), ...args, room), "dataRoom");
      return view().getUint32(room, true);
    });
  }

  /** Writes the next Chunk of the transaction with `part` (at most the data room); `last` ends the message. */
  send(dev: CiDevice, muid: number, requestId: number, role: PeRole, part: Uint8Array, last: boolean, nowUs = 0): Uint8Array {
    const args = [peer(muid), int("requestId", requestId, 0x7f), roleValue(role)] as const;
    const now = u64("nowUs", nowUs);
    const cap = MESSAGE_MAX + part.length;
    return scoped([Math.max(1, part.length), cap, 4], (pd, out, len) => {
      writeBytes(pd, part);
      this.#check(fn("zmidi_pe_send")(this.#b.main, ciHandle(dev), ...args, pd, part.length, last ? 1 : 0, now, out, cap, len), "send");
      return messages(out, view().getUint32(len, true))[0]!;
    });
  }

  /** Asks the peer to wait `ms` more milliseconds (1 to 12700) with a MIDI-CI ACK 0x10; `writePending` writes it. */
  wait(muid: number, requestId: number, role: PeRole, ms: number): void {
    const args = [peer(muid), int("requestId", requestId, 0x7f), roleValue(role), int("ms", ms, 0xffffffff)] as const;
    this.#check(fn("zmidi_pe_wait")(this.#b.main, ...args), "wait");
  }

  /** Ends the transaction with a MIDI-CI NAK 0x20; `writePending` writes it. */
  abort(dev: CiDevice, muid: number, requestId: number, role: PeRole): void {
    const args = [peer(muid), int("requestId", requestId, 0x7f), roleValue(role)] as const;
    this.#check(fn("zmidi_pe_abort")(this.#b.main, ciHandle(dev), ...args), "abort");
  }

  /** Asks again for a corrupt Chunk with Flow Control (NAK 0x12); `writePending` writes it. */
  corrupt(muid: number, requestId: number, role: PeRole): void {
    const args = [peer(muid), int("requestId", requestId, 0x7f), roleValue(role)] as const;
    this.#check(fn("zmidi_pe_corrupt")(this.#b.main, ...args), "corrupt");
  }

  /** The subscription entry `index`, or null for an empty index. */
  subscription(index: number): PeSubscription | null {
    const i = int("index", index, 0xffff);
    return scoped([SUBSCRIPTION.size], (out) => {
      const st = fn("zmidi_pe_subscription")(this.#b.main, i, out);
      if (st === Status.NO_PENDING) {
        return null;
      }
      this.#check(st, "subscription");
      return readSubscription(out);
    });
  }

  /** The next subscription that an Invalidate MUID removed, or null. */
  takeEnded(): PeSubscription | null {
    return scoped([SUBSCRIPTION.size], (out) => {
      const st = fn("zmidi_pe_take_ended")(this.#b.main, out);
      if (st === Status.NO_PENDING) {
        return null;
      }
      this.#check(st, "takeEnded");
      return readSubscription(out);
    });
  }

  /** Frees the WASM memory of the engine now. */
  free(): void {
    this.#b.release();
  }

  /** Throws the error of a status that is not OK, with the reason of the engine. */
  #check(status: number, what: string): void {
    if (status === Status.OK) {
      return;
    }
    const reason = scoped([2], (st) => {
      const e = fn("zmidi_pe_last_error")(this.#b.main, st);
      const text = ERRORS[e] ?? `error ${e}`;
      return e === 10 ? `${text}: status ${view().getUint16(st, true)}` : text;
    });
    switch (status) {
      case Status.INVALID_ARGUMENT:
        throw new InvalidValueError(`${what}: ${reason === "none" ? "a value is out of its range" : reason}`);
      case Status.BUFFER_TOO_SMALL:
        if (reason === "buffer size") {
          throw new InvalidValueError(`${what}: a part larger than the data room`);
        }
        throw new MidiError(`${what}: status ${status}`);
      case Status.NO_PENDING:
      case Status.REJECTED:
        throw new RejectedError(`${what}: ${reason}`);
      case Status.NOT_SUPPORTED:
        throw new RejectedError(`${what}: not supported by this build`);
      default:
        throw new MidiError(`${what}: status ${status}`);
    }
  }

  /** Runs `body` with `text` as a C string (0 for undefined). */
  #withText<T>(text: string | undefined, body: (ptr: number) => T): T {
    if (text === undefined) {
      return body(0);
    }
    const p = allocCString(text);
    try {
      return body(p);
    } finally {
      freeBlock(p);
    }
  }

  /** Runs `body` with a ZmidiPeHeader of `resource` and `o`, and the ZmidiPeResource of `o.caps` (or 0). */
  #withHeader<T>(resource: string, o: PeRequestOptions, body: (header: number, caps: number) => T): T {
    const texts: number[] = [];
    const keep = (p: number): number => {
      texts.push(p);
      return p;
    };
    try {
      return scoped([HEADER.size, RESOURCE.size], (h, c) => {
        // The blocks first: an alloc can grow the memory, and a grow empties the views before it.
        const name = keep(allocCString(resource));
        const resId = o.resId === undefined ? 0 : keep(allocCString(o.resId));
        const media = o.mediaType === undefined ? 0 : keep(allocCString(o.mediaType));
        const enc = encodingValue(o.encoding);
        const len = lengthValue(o.length);
        const v = view();
        const off = HEADER.off;
        v.setUint32(h + off["resource"]!, name, true);
        v.setUint32(h + off["res_id"]!, resId, true);
        v.setUint32(h + off["media_type"]!, media, true);
        if (o.offset !== undefined && o.limit !== undefined) {
          v.setUint8(h + off["has_offset"]!, 1);
          v.setUint32(h + off["offset"]!, int("offset", o.offset, 0xffffffff), true);
          v.setUint32(h + off["limit"]!, int("limit", o.limit, 0xffffffff), true);
        }
        v.setUint8(h + off["set_partial"]!, o.setPartial ? 1 : 0);
        v.setUint8(h + off["encoding"]!, enc);
        v.setUint32(h + off["length"]!, len, true);
        if (o.caps !== undefined) {
          writeResource(c, o.caps, keep);
        }
        return body(h, o.caps === undefined ? 0 : c);
      });
    } finally {
      texts.forEach(freeBlock);
    }
  }
}
