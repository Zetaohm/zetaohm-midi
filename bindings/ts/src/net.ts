// Network MIDI 2.0 Host and Client over UDP (zmidi-net through the C ABI, M2-124), sans-IO: the
// program owns the socket, gives each received payload, and sends each payload that the endpoint
// writes. The authentication (M2-124 6.7 to 6.10) needs C ABI 0.2 with the feature net-auth: the
// program keeps its credentials, and the package copies them into WASM memory for one receive
// call only, then clears that copy.

import { InvalidValueError } from "./errors.js";
import {
  Blocks,
  alloc,
  allocCString,
  check,
  fn,
  free,
  int,
  layout,
  readBytes,
  readWords,
  scoped,
  sizeOf,
  splitUmps,
  u64,
  view,
  writeBytes,
  writeWords,
} from "./wasm.js";

/** An IP address (IPv4 dotted or IPv6 text) and a UDP port. */
export interface NetAddress {
  /** The IP address as text, for example "192.168.1.10" or "fe80::1". */
  readonly address: string;
  /** The UDP port, 0 to 65535. */
  readonly port: number;
}

/** The kind of a net event. */
export type NetEventKind =
  | "invitationReceived"
  | "pending"
  | "authRequested"
  | "authFailed"
  | "credentialsError"
  | "established"
  | "ending"
  | "closed"
  | "reset"
  | "resetStarted"
  | "nak"
  | "pingReply"
  | "unexpectedPingReply"
  | "timeout"
  | "stream"
  | "data"
  | "diagnostic";

const KINDS: readonly (NetEventKind | null)[] = [
  null,
  "invitationReceived",
  "pending",
  "authRequested",
  "authFailed",
  "credentialsError",
  "established",
  "ending",
  "closed",
  "reset",
  "resetStarted",
  "nak",
  "pingReply",
  "unexpectedPingReply",
  "timeout",
  "stream",
  "data",
  "diagnostic",
];

/** One event of a peer. */
export interface NetEvent {
  /** The kind of the event. */
  readonly kind: NetEventKind;
  /** The address of the peer. */
  readonly peer: NetAddress;
  /**
   * A code of the kind: the Ping Id ("pingReply", "unexpectedPingReply"), the Bye Reason
   * ("ending"), the NAK Reason ("nak"), the close cause ("closed": 0 Bye received, 1 Bye Reply,
   * 2 Bye timeout, 3 expired, 4 ended by this side), the diagnostic ("diagnostic": 0 bad signature,
   * 1 invalid peer address, 2 table full, 3 NAK sent).
   *
   * Authentication: the method, NET_AUTH_SHARED_SECRET or NET_AUTH_USER ("authRequested"; and
   * "established", 0 for no authentication); the method with NET_AUTH_USERNAME_NOT_FOUND when the
   * Host did not find the Username ("authFailed"); the problem with the credentials of the program
   * ("credentialsError": 0 no random bytes (Host), 1 no shared secret (Host), 2 empty shared
   * secret, 3 empty password, 4 Username not valid (Client), 5 Username too long (Client)).
   */
  readonly code: number;
  /**
   * A value of the kind: the round-trip time in ms ("pingReply"), the Bye Reason ("closed"), the
   * NAK Reason (a "diagnostic" of code 3), the count of failures of the session ("authFailed"),
   * the Authentication State ("authRequested": 0x00 first request, 0x01 digest not correct, 0x02
   * Username not found).
   */
  readonly value: number;
  /** "data": the UMPs, each a list of words. */
  readonly umps: readonly (readonly number[])[];
}

/** One UDP payload with its destination. */
export interface NetPayload {
  /** The destination. */
  readonly to: NetAddress;
  /** The UDP payload. */
  readonly payload: Uint8Array;
}

const ADDR = layout([
  ["family", "u8"],
  ["ip", ["u8", 16]],
  ["port", "u16"],
]);

const EVENT = layout([
  ["kind", "i32"],
  ["peer", ADDR],
  ["code", "u32"],
  ["value", "u64"],
  ["word_offset", "usize"],
  ["word_count", "usize"],
]);

const BUFFERS = layout([
  ["events", "ptr"],
  ["events_capacity", "usize"],
  ["events_len", "usize"],
  ["words", "ptr"],
  ["words_capacity", "usize"],
  ["words_len", "usize"],
  ["out", "ptr"],
  ["out_capacity", "usize"],
  ["out_len", "usize"],
]);

/** ZMIDI_NET_MAX_PAYLOAD and ZMIDI_NET_RECORD_HEADER. */
const MAX_PAYLOAD = 1400;
const RECORD_HEADER = 21;

const EVENTS = 128;
const WORDS = 1024;
const OUT = 8 * (MAX_PAYLOAD + RECORD_HEADER);

// --- authentication (M2-124 6.7 to 6.10) ---

/** ZMIDI_NET_AUTH_SHARED_SECRET: the shared secret method (M2-124 6.9), Capabilities bit D0. */
export const NET_AUTH_SHARED_SECRET = 1;

/** ZMIDI_NET_AUTH_USER: the user method (M2-124 6.10), Capabilities bit D1. */
export const NET_AUTH_USER = 2;

/** ZMIDI_NET_AUTH_USERNAME_NOT_FOUND: the flag in the code of "authFailed" for an unknown Username. */
export const NET_AUTH_USERNAME_NOT_FOUND = 0x100;

/** An authentication method: the shared secret (M2-124 6.9) or the user (6.10) method. */
export type NetAuthMethod = "sharedSecret" | "user";

/** One user of the user method (M2-124 6.10). A string is sent as UTF-8. */
export interface NetUser {
  /** The Username. */
  readonly username: string | Uint8Array;
  /** The password. */
  readonly password: string | Uint8Array;
}

/**
 * The credentials of a Host or a Client (ZmidiNetCredentials). Host: the shared secret and the
 * users verify the digests. Client: the shared secret, or the first user, answers a request of
 * the Host; with neither, the Client declines (Bye 0x80).
 */
export interface NetCredentials {
  /** The shared secret (M2-124 6.9). */
  readonly sharedSecret?: string | Uint8Array;
  /** The user table (Host) or the one user (Client) of the user method. */
  readonly users?: readonly NetUser[];
  /**
   * Host: random bytes for the CryptoNonces, 16 for each. The Host uses them in order (see
   * `randomUsed`). Default: 64 new bytes of `crypto.getRandomValues` for each receive call.
   */
  readonly random?: Uint8Array;
}

/** The authentication of a Host or a Client. */
export interface NetAuthOptions {
  /**
   * Host: the methods that it asks for (with both, a Client that has both gets the user method).
   * Client: the methods of the Capabilities of its Invitations. One or both.
   */
  readonly methods: readonly NetAuthMethod[];
  /** The credentials (default none; set them later with `setCredentials`). */
  readonly credentials?: NetCredentials;
}

const USER = layout([
  ["username", "ptr"],
  ["username_len", "usize"],
  ["password", "ptr"],
  ["password_len", "usize"],
]);

const CREDENTIALS = layout([
  ["shared_secret", "ptr"],
  ["shared_secret_len", "usize"],
  ["users", "ptr"],
  ["user_count", "usize"],
  ["random", "ptr"],
  ["random_len", "usize"],
  ["random_used", "usize"],
]);

/** Random bytes of one receive call of a Host with no `random`: 4 CryptoNonces. */
const RANDOM = 64;

function bytesOf(x: string | Uint8Array): Uint8Array {
  return typeof x === "string" ? new TextEncoder().encode(x) : x;
}

function methodBits(methods: readonly NetAuthMethod[]): number {
  let bits = 0;
  for (const m of methods) {
    if (m === "sharedSecret") {
      bits |= NET_AUTH_SHARED_SECRET;
    } else if (m === "user") {
      bits |= NET_AUTH_USER;
    } else {
      throw new InvalidValueError(`an authentication method is "sharedSecret" or "user", not ${String(m)}`);
    }
  }
  return bits;
}

function freshRandom(): Uint8Array {
  const b = new Uint8Array(RANDOM);
  if (typeof globalThis.crypto?.getRandomValues !== "function") {
    // No random source: the Host gives a "credentialsError" event of code 0.
    return new Uint8Array(0);
  }
  globalThis.crypto.getRandomValues(b);
  return b;
}

// --- addresses ---

function parseIpv4(s: string): number[] | null {
  const parts = s.split(".");
  if (parts.length !== 4 || !parts.every((p) => /^\d{1,3}$/.test(p) && Number(p) <= 255)) {
    return null;
  }
  return parts.map(Number);
}

function parseIpv6(s: string): number[] | null {
  const halves = s.split("::");
  if (halves.length > 2) {
    return null;
  }
  const groups = (h: string): number[] | null => {
    if (h === "") {
      return [];
    }
    const out: number[] = [];
    const parts = h.split(":");
    for (let i = 0; i < parts.length; i++) {
      const p = parts[i]!;
      if (i === parts.length - 1 && p.includes(".")) {
        const v4 = parseIpv4(p);
        if (v4 === null) {
          return null;
        }
        out.push((v4[0]! << 8) | v4[1]!, (v4[2]! << 8) | v4[3]!);
      } else if (/^[0-9a-fA-F]{1,4}$/.test(p)) {
        out.push(parseInt(p, 16));
      } else {
        return null;
      }
    }
    return out;
  };
  const head = groups(halves[0]!);
  const tail = halves.length === 2 ? groups(halves[1]!) : [];
  if (head === null || tail === null) {
    return null;
  }
  const fill = 8 - head.length - tail.length;
  if ((halves.length === 2 && fill < 1) || (halves.length === 1 && fill !== 0)) {
    return null;
  }
  const all = [...head, ...new Array<number>(fill).fill(0), ...tail];
  return all.flatMap((g) => [g >> 8, g & 0xff]);
}

function formatIpv6(b: Uint8Array): string {
  const g: number[] = [];
  for (let i = 0; i < 16; i += 2) {
    g.push((b[i]! << 8) | b[i + 1]!);
  }
  // RFC 5952: compress the longest run of two or more zero groups (the first one on a tie).
  let best = -1;
  let bestLen = 1;
  for (let i = 0; i < 8; ) {
    if (g[i] !== 0) {
      i++;
      continue;
    }
    let j = i;
    while (j < 8 && g[j] === 0) {
      j++;
    }
    if (j - i > bestLen) {
      best = i;
      bestLen = j - i;
    }
    i = j;
  }
  const hex = (xs: number[]): string => xs.map((x) => x.toString(16)).join(":");
  return best < 0 ? hex(g) : `${hex(g.slice(0, best))}::${hex(g.slice(best + bestLen))}`;
}

/** Writes `a` to a ZmidiNetAddr at `ptr`. */
function writeAddr(ptr: number, a: NetAddress): void {
  const port = int("port", a.port, 0xffff);
  const v4 = parseIpv4(a.address);
  const ip = v4 ?? parseIpv6(a.address);
  if (ip === null) {
    throw new InvalidValueError(`not an IP address: ${a.address}`);
  }
  const v = view();
  v.setUint8(ptr + ADDR.off["family"]!, v4 ? 4 : 6);
  new Uint8Array(v.buffer, ptr + ADDR.off["ip"]!, 16).set(ip);
  v.setUint16(ptr + ADDR.off["port"]!, port, true);
}

function readAddr(ptr: number): NetAddress {
  const v = view();
  const ip = readBytes(ptr + ADDR.off["ip"]!, 16);
  const address = v.getUint8(ptr + ADDR.off["family"]!) === 4 ? [...ip.subarray(0, 4)].join(".") : formatIpv6(ip);
  return { address, port: v.getUint16(ptr + ADDR.off["port"]!, true) };
}

// --- the shared requests of the Host and the Client ---

/** The work arrays of one endpoint: a ZmidiNetBuffers and its events, words and records. */
interface Arrays {
  bufs: number;
  events: number;
  words: number;
  out: number;
}

function reset(a: Arrays): void {
  const v = view();
  const o = BUFFERS.off;
  v.setUint32(a.bufs + o["events"]!, a.events, true);
  v.setUint32(a.bufs + o["events_capacity"]!, EVENTS, true);
  v.setUint32(a.bufs + o["events_len"]!, 0, true);
  v.setUint32(a.bufs + o["words"]!, a.words, true);
  v.setUint32(a.bufs + o["words_capacity"]!, WORDS, true);
  v.setUint32(a.bufs + o["words_len"]!, 0, true);
  v.setUint32(a.bufs + o["out"]!, a.out, true);
  v.setUint32(a.bufs + o["out_capacity"]!, OUT, true);
  v.setUint32(a.bufs + o["out_len"]!, 0, true);
}

function events(a: Arrays): NetEvent[] {
  const v = view();
  const n = v.getUint32(a.bufs + BUFFERS.off["events_len"]!, true);
  const out: NetEvent[] = [];
  for (let i = 0; i < n; i++) {
    const at = a.events + EVENT.size * i;
    const kind = KINDS[v.getInt32(at + EVENT.off["kind"]!, true)] ?? null;
    if (kind === null) {
      continue;
    }
    let umps: number[][] = [];
    if (kind === "data") {
      const off = v.getUint32(at + EVENT.off["word_offset"]!, true);
      const count = v.getUint32(at + EVENT.off["word_count"]!, true);
      umps = splitUmps(readWords(a.words + 4 * off, count));
    }
    out.push({
      kind,
      peer: readAddr(at + EVENT.off["peer"]!),
      code: v.getUint32(at + EVENT.off["code"]!, true),
      value: Number(v.getBigUint64(at + EVENT.off["value"]!, true)),
      umps,
    });
  }
  return out;
}

function replies(a: Arrays): NetPayload[] {
  const len = view().getUint32(a.bufs + BUFFERS.off["out_len"]!, true);
  const b = readBytes(a.out, len);
  const out: NetPayload[] = [];
  let at = 0;
  while (at + RECORD_HEADER <= b.length) {
    const family = b[at]!;
    const ip = b.subarray(at + 1, at + 17);
    const port = (b[at + 17]! << 8) | b[at + 18]!;
    const n = (b[at + 19]! << 8) | b[at + 20]!;
    const address = family === 4 ? [...ip.subarray(0, 4)].join(".") : formatIpv6(ip);
    out.push({ to: { address, port }, payload: b.slice(at + RECORD_HEADER, at + RECORD_HEADER + n) });
    at += RECORD_HEADER + n;
  }
  return out;
}

/** The C state and work arrays of a Host or a Client, and the requests that both have. */
class Endpoint {
  readonly #b: Blocks;
  readonly #side: "host" | "client";
  readonly #a: Arrays;
  #creds: NetCredentials | null = null;
  #randomUsed = 0;

  constructor(owner: object, side: "host" | "client", name: string, productInstanceId: string, auth?: NetAuthOptions) {
    this.#b = new Blocks(owner);
    this.#side = side;
    const slot = this.#b.add(alloc(sizeOf(`zmidi_net_${side}_size`)));
    this.#a = {
      bufs: this.#b.add(alloc(BUFFERS.size)),
      events: this.#b.add(alloc(EVENT.size * EVENTS)),
      words: this.#b.add(alloc(4 * WORDS)),
      out: this.#b.add(alloc(OUT)),
    };
    // The endpoint copies the texts at init.
    const bits = auth === undefined ? 0 : methodBits(auth.methods);
    const pn = allocCString(name);
    const pp = allocCString(productInstanceId);
    try {
      const what = side === "host" ? "NetHost" : "NetClient";
      if (auth === undefined) {
        check(fn(`zmidi_net_${side}_init`)(slot, pn, pp), what);
      } else {
        check(fn(`zmidi_net_${side}_init_auth`)(slot, pn, pp, bits), what);
      }
    } finally {
      free(pn);
      free(pp);
    }
    this.setCredentials(auth?.credentials ?? null);
  }

  setCredentials(c: NetCredentials | null): void {
    this.#creds = c;
    this.#randomUsed = 0;
  }

  get randomUsed(): number {
    return this.#randomUsed;
  }

  /**
   * Runs `body` with a ZmidiNetCredentials of the credentials in a new block, then clears and
   * frees the block: the C ABI keeps no pointer after the call.
   */
  #withCredentials<T>(c: NetCredentials, body: (pc: number) => T): T {
    const secret = c.sharedSecret === undefined ? new Uint8Array(0) : bytesOf(c.sharedSecret);
    const users = (c.users ?? []).map((u) => [bytesOf(u.username), bytesOf(u.password)] as const);
    const own = c.random !== undefined;
    const random = own ? c.random! : this.#side === "host" ? freshRandom() : new Uint8Array(0);
    const size = CREDENTIALS.size + USER.size * users.length + secret.length + random.length + users.reduce((n, [a, b]) => n + a.length + b.length, 0);
    const block = alloc(size);
    try {
      const v = view();
      let at = block + CREDENTIALS.size + USER.size * users.length;
      // Copies `b` after the structs; gives its address, or 0 for no bytes.
      const put = (b: Uint8Array): number => {
        if (b.length === 0) {
          return 0;
        }
        writeBytes(at, b);
        at += b.length;
        return at - b.length;
      };
      const o = CREDENTIALS.off;
      v.setUint32(block + o["shared_secret"]!, put(secret), true);
      v.setUint32(block + o["shared_secret_len"]!, secret.length, true);
      users.forEach(([name, password], i) => {
        const pu = block + CREDENTIALS.size + USER.size * i;
        v.setUint32(pu + USER.off["username"]!, put(name), true);
        v.setUint32(pu + USER.off["username_len"]!, name.length, true);
        v.setUint32(pu + USER.off["password"]!, put(password), true);
        v.setUint32(pu + USER.off["password_len"]!, password.length, true);
      });
      v.setUint32(block + o["users"]!, users.length === 0 ? 0 : block + CREDENTIALS.size, true);
      v.setUint32(block + o["user_count"]!, users.length, true);
      v.setUint32(block + o["random"]!, put(random), true);
      v.setUint32(block + o["random_len"]!, random.length, true);
      v.setUint32(block + o["random_used"]!, own ? Math.min(this.#randomUsed, random.length) : 0, true);
      const r = body(block);
      if (own) {
        this.#randomUsed = view().getUint32(block + o["random_used"]!, true);
      }
      return r;
    } finally {
      new Uint8Array(view().buffer, block, size).fill(0);
      free(block);
    }
  }

  get main(): number {
    return this.#b.main;
  }

  f(name: string): (...args: (number | bigint)[]) => number {
    return fn(`zmidi_net_${this.#side}_${name}`);
  }

  connect(nowMs: number, host: NetAddress): void {
    const now = u64("nowMs", nowMs);
    scoped([ADDR.size], (pa) => {
      writeAddr(pa, host);
      check(fn("zmidi_net_client_connect")(this.main, now, pa), "connect");
    });
  }

  receive(nowMs: number, peer: NetAddress, payload: Uint8Array): { events: NetEvent[]; replies: NetPayload[] } {
    const now = u64("nowMs", nowMs);
    return scoped([ADDR.size, Math.max(1, payload.length)], (pa, pp) => {
      writeAddr(pa, peer);
      writeBytes(pp, payload);
      reset(this.#a);
      const c = this.#creds;
      if (c === null) {
        check(this.f("receive")(this.main, now, pa, pp, payload.length, this.#a.bufs), "receive");
      } else {
        this.#withCredentials(c, (pc) => check(this.f("receive_auth")(this.main, now, pa, pp, payload.length, pc, this.#a.bufs), "receive"));
      }
      return { events: events(this.#a), replies: replies(this.#a) };
    });
  }

  poll(nowMs: number): NetEvent[] {
    const now = u64("nowMs", nowMs);
    reset(this.#a);
    check(this.f("poll")(this.main, now, this.#a.bufs), "poll");
    return events(this.#a);
  }

  writePending(nowMs: number): NetPayload[] {
    const now = u64("nowMs", nowMs);
    return scoped([MAX_PAYLOAD, ADDR.size, 4], (buf, to, len) => {
      const out: NetPayload[] = [];
      for (;;) {
        check(this.f("write_pending")(this.main, now, buf, MAX_PAYLOAD, to, len), "writePending");
        const n = view().getUint32(len, true);
        if (n === 0) {
          return out;
        }
        out.push({ to: readAddr(to), payload: readBytes(buf, n) });
      }
    });
  }

  nextDeadline(): number | null {
    return scoped([8], (d) => (this.f("next_deadline")(this.main, d) !== 0 ? Number(view().getBigUint64(d, true)) : null));
  }

  sendUmp(peer: NetAddress, words: readonly number[]): void {
    words.forEach((w, i) => int(`word ${i}`, w, 0xffffffff));
    if (words.length === 0) {
      throw new InvalidValueError("no words");
    }
    scoped([ADDR.size, 4 * words.length], (pa, pw) => {
      writeAddr(pa, peer);
      writeWords(pw, words);
      check(this.f("send_ump")(this.main, pa, pw, words.length), "sendUmp");
    });
  }

  ping(nowMs: number, peer: NetAddress): number {
    const now = u64("nowMs", nowMs);
    return scoped([ADDR.size, 4], (pa, id) => {
      writeAddr(pa, peer);
      check(this.f("ping")(this.main, now, pa, id), "ping");
      return view().getUint32(id, true);
    });
  }

  bye(nowMs: number, peer: NetAddress, reason: number): NetEvent[] {
    const now = u64("nowMs", nowMs);
    const r = int("reason", reason, 0xff);
    return scoped([ADDR.size], (pa) => {
      writeAddr(pa, peer);
      reset(this.#a);
      check(this.f("bye")(this.main, now, pa, r, this.#a.bufs), "bye");
      return events(this.#a);
    });
  }

  get droppedEvents(): number {
    return this.f("dropped_events")(this.main) >>> 0;
  }

  free(): void {
    this.#b.release();
  }
}

/**
 * A Network MIDI 2.0 Host with 4 sessions. With no `auth` it accepts each Client; with `auth` it
 * asks each new Client for authentication (M2-124 6.7, 6.8).
 */
export class NetHost {
  readonly #e: Endpoint;

  /**
   * A Host with the UMP Endpoint Name `name` and the Product Instance Id `productInstanceId`, and
   * with `auth`, the authentication methods that it asks for and its credentials.
   */
  constructor(name: string, productInstanceId: string, auth?: NetAuthOptions) {
    this.#e = new Endpoint(this, "host", name, productInstanceId, auth);
  }

  /**
   * Sets the credentials that verify the digests (null: none) and starts `randomUsed` at 0. Each
   * receive call copies them into WASM memory for the call only.
   */
  setCredentials(credentials: NetCredentials | null): void {
    this.#e.setCredentials(credentials);
  }

  /**
   * The count of the bytes of `credentials.random` that the CryptoNonces used (16 for each). When
   * fewer than 16 are left, a Host that needs a CryptoNonce gives a "credentialsError" event of
   * code 0: set new credentials.
   */
  get randomUsed(): number {
    return this.#e.randomUsed;
  }

  /** Reads one UDP payload from `peer` at `nowMs`. Gives the events and the reply payloads to send. */
  receive(nowMs: number, peer: NetAddress, payload: Uint8Array): { events: NetEvent[]; replies: NetPayload[] } {
    return this.#e.receive(nowMs, peer, payload);
  }

  /** Runs the timers at `nowMs`; gives their events. */
  poll(nowMs: number): NetEvent[] {
    return this.#e.poll(nowMs);
  }

  /** Gives every pending UDP payload (UMP Data, Pings, retransmits) with its destination. */
  writePending(nowMs: number): NetPayload[] {
    return this.#e.writePending(nowMs);
  }

  /** The next time (ms) at which the Host needs a poll, or null when there is none. */
  nextDeadline(): number | null {
    return this.#e.nextDeadline();
  }

  /** Queues whole UMP messages for the peer `peer`. */
  sendUmp(peer: NetAddress, words: readonly number[]): void {
    this.#e.sendUmp(peer, words);
  }

  /** Sends a Ping to `peer` at `nowMs`; gives its Ping Id. */
  ping(nowMs: number, peer: NetAddress): number {
    return this.#e.ping(nowMs, peer);
  }

  /** Ends the session with `peer` with a Bye of `reason` (default 0x01, user terminated); gives its events. */
  bye(nowMs: number, peer: NetAddress, reason = 1): NetEvent[] {
    return this.#e.bye(nowMs, peer, reason);
  }

  /** The count of events that did not fit and were dropped. */
  get droppedEvents(): number {
    return this.#e.droppedEvents;
  }

  /** Frees the WASM memory of the Host now. */
  free(): void {
    this.#e.free();
  }
}

/**
 * A Network MIDI 2.0 Client. With `auth` its Invitations show the authentication methods, and it
 * answers a request of the Host at once with its credentials.
 */
export class NetClient {
  readonly #e: Endpoint;

  /**
   * A Client with the UMP Endpoint Name `name` and the Product Instance Id `productInstanceId`,
   * and with `auth`, the methods of its Capabilities and its credentials.
   */
  constructor(name: string, productInstanceId: string, auth?: NetAuthOptions) {
    this.#e = new Endpoint(this, "client", name, productInstanceId, auth);
  }

  /**
   * Sets the credentials that answer a request for authentication (null: none; the Client then
   * answers no request). Each receive call copies them into WASM memory for the call only.
   */
  setCredentials(credentials: NetCredentials | null): void {
    this.#e.setCredentials(credentials);
  }

  /** Starts an Invitation to the Host at `host` at `nowMs`. */
  connect(nowMs: number, host: NetAddress): void {
    this.#e.connect(nowMs, host);
  }

  /** Reads one UDP payload from `peer` at `nowMs`. Gives the events and the reply payloads to send. */
  receive(nowMs: number, peer: NetAddress, payload: Uint8Array): { events: NetEvent[]; replies: NetPayload[] } {
    return this.#e.receive(nowMs, peer, payload);
  }

  /** Runs the timers at `nowMs`; gives their events. */
  poll(nowMs: number): NetEvent[] {
    return this.#e.poll(nowMs);
  }

  /** Gives every pending UDP payload (Invitation, UMP Data, Pings) with its destination. */
  writePending(nowMs: number): NetPayload[] {
    return this.#e.writePending(nowMs);
  }

  /** The next time (ms) at which the Client needs a poll, or null when there is none. */
  nextDeadline(): number | null {
    return this.#e.nextDeadline();
  }

  /** Queues whole UMP messages for the peer `peer`. */
  sendUmp(peer: NetAddress, words: readonly number[]): void {
    this.#e.sendUmp(peer, words);
  }

  /** Sends a Ping to `peer` at `nowMs`; gives its Ping Id. */
  ping(nowMs: number, peer: NetAddress): number {
    return this.#e.ping(nowMs, peer);
  }

  /** Ends the session with `peer` with a Bye of `reason` (default 0x01, user terminated); gives its events. */
  bye(nowMs: number, peer: NetAddress, reason = 1): NetEvent[] {
    return this.#e.bye(nowMs, peer, reason);
  }

  /** The count of events that did not fit and were dropped. */
  get droppedEvents(): number {
    return this.#e.droppedEvents;
  }

  /** Frees the WASM memory of the Client now. */
  free(): void {
    this.#e.free();
  }
}
