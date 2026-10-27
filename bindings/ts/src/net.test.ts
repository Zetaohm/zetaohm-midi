// Network MIDI 2.0 (REQ-TS-NET-001): a Host and a Client in memory, M2-124. Authentication
// (REQ-TS-NET-002, REQ-TS-NET-003): the CryptoNonces, secrets and digests of the M2-124 6.9 and
// 6.10 examples, as tests/c/net.c of zmidi-ffi uses them.
import { readFile } from "node:fs/promises";

import { beforeAll, describe, expect, test } from "vitest";

import {
  InvalidValueError,
  NET_AUTH_SHARED_SECRET,
  NET_AUTH_USER,
  NET_AUTH_USERNAME_NOT_FOUND,
  NetClient,
  NetHost,
  RejectedError,
  init,
  wasmUrl,
} from "./index.js";
import type { NetEvent } from "./index.js";
import { view } from "./wasm.js";

/** The start of the heap of the package: the initial size of the WASM memory. */
let heapStart = 0;

beforeAll(async () => {
  await init();
  const fresh = new WebAssembly.Instance(new WebAssembly.Module(await readFile(wasmUrl)), {});
  heapStart = (fresh.exports.memory as WebAssembly.Memory).buffer.byteLength;
});

const HOST = { address: "192.168.1.10", port: 5673 };
const CLIENT = { address: "192.168.1.20", port: 50000 };

function pump(host: NetHost, client: NetClient, start: number): { atHost: NetEvent[]; atClient: NetEvent[] } {
  const atHost: NetEvent[] = [];
  const atClient: NetEvent[] = [];
  let now = start;
  for (let i = 0; i < 20; i++) {
    let moved = false;
    for (const p of client.writePending(now)) {
      atHost.push(...host.receive(now, CLIENT, p.payload).events);
      moved = true;
    }
    for (const p of host.writePending(now)) {
      expect(p.to).toEqual(CLIENT);
      atClient.push(...client.receive(now, HOST, p.payload).events);
      moved = true;
    }
    if (!moved) {
      break;
    }
    now += 10;
  }
  return { atHost, atClient };
}

const kinds = (events: NetEvent[]): string[] => events.map((e) => e.kind);

describe("net", () => {
  test("session, UMP Data, Ping and Bye", () => {
    const host = new NetHost("zmidi Host", "host-1");
    const client = new NetClient("zmidi Client", "client-1");
    client.connect(0, HOST);
    let r = pump(host, client, 0);
    expect(kinds(r.atHost)).toContain("established");
    expect(kinds(r.atClient)).toContain("established");

    client.sendUmp(HOST, [0x40903c00, 0xc0000000]);
    r = pump(host, client, 100);
    expect(r.atHost.filter((e) => e.kind === "data").map((e) => e.umps)).toEqual([[[0x40903c00, 0xc0000000]]]);

    const id = client.ping(200, HOST);
    r = pump(host, client, 200);
    expect(r.atClient.filter((e) => e.kind === "pingReply").map((e) => e.code)).toEqual([id]);

    const bye = client.bye(300, HOST, 1);
    r = pump(host, client, 300);
    expect(kinds(r.atHost)).toContain("closed");
    expect(kinds([...bye, ...r.atClient])).toContain("closed");
  });

  test("net bad values", () => {
    expect(() => new NetHost("", "x")).toThrow(InvalidValueError);
    const host = new NetHost("h", "p");
    expect(() => host.sendUmp({ address: "not an ip", port: 1 }, [0])).toThrow(InvalidValueError);
    expect(() => host.sendUmp(CLIENT, [0x20903c64])).toThrow(RejectedError);
    expect(host.nextDeadline()).toBeNull();
  });

  test("IPv6 addresses", () => {
    const host = new NetHost("h", "p");
    const client = new NetClient("c", "q");
    const h6 = { address: "fe80::1", port: 5673 };
    client.connect(0, h6);
    const [p] = client.writePending(0);
    expect(p?.to).toEqual(h6);
    const r = host.receive(0, { address: "2001:db8:0:0:1::2", port: 4000 }, p!.payload);
    expect(r.events[0]?.peer).toEqual({ address: "2001:db8::1:0:0:2", port: 4000 });
  });
});

// --- authentication (M2-124 6.7 to 6.10) ---

interface Traffic {
  atHost: NetEvent[];
  atClient: NetEvent[];
  fromHost: Uint8Array[];
  fromClient: Uint8Array[];
}

/**
 * Moves every pending payload of both sides to the other side until none is left. When nothing
 * moves and a side has a deadline within 5 s (failure delay, repeats), the time goes to it and both
 * sides poll.
 */
function authPump(host: NetHost, client: NetClient, start = 0): Traffic {
  const t: Traffic = { atHost: [], atClient: [], fromHost: [], fromClient: [] };
  let now = start;
  for (let round = 0; round < 60; round++) {
    let moved = false;
    for (const p of client.writePending(now)) {
      t.fromClient.push(p.payload);
      t.atHost.push(...host.receive(now, CLIENT, p.payload).events);
      moved = true;
    }
    for (const p of host.writePending(now)) {
      t.fromHost.push(p.payload);
      t.atClient.push(...client.receive(now, HOST, p.payload).events);
      moved = true;
    }
    if (moved) {
      now += 10;
      continue;
    }
    const next = Math.min(host.nextDeadline() ?? Infinity, client.nextDeadline() ?? Infinity);
    if (next > now + 5000) {
      break;
    }
    now = Math.max(now, next);
    t.atHost.push(...host.poll(now));
    t.atClient.push(...client.poll(now));
  }
  return t;
}

/** True when one of `payloads` holds the bytes `x`. */
function sent(payloads: readonly Uint8Array[], x: Uint8Array): boolean {
  return payloads.some((p) => {
    for (let i = 0; i + x.length <= p.length; i++) {
      if (x.every((b, k) => p[i + k] === b)) {
        return true;
      }
    }
    return false;
  });
}

const of = (events: NetEvent[], kind: string): NetEvent[] => events.filter((e) => e.kind === kind);
const enc = (s: string): Uint8Array => new TextEncoder().encode(s);

/** The random bytes that give a CryptoNonce: CryptoNonce byte = 32 + (random byte mod 95). */
const randomFor = (nonce: string): Uint8Array => new Uint8Array([...nonce].map((c) => c.charCodeAt(0) - 32));

describe("net authentication", () => {
  test("shared secret method, M2-124 6.9 example", () => {
    // CryptoNonce "nUWrn*@#$hjfwnkL", shared secret "5483", and the Authentication Digest.
    const nonce = "nUWrn*@#$hjfwnkL";
    const digest = new Uint8Array([
      0x67, 0x6e, 0xbe, 0x82, 0x58, 0x7c, 0xec, 0xa8, 0xf8, 0x2f, 0xc3, 0x33, 0xd7, 0x87, 0x95, 0x1e, 0xec, 0x2b, 0x00, 0xad, 0x31,
      0x61, 0x3c, 0xc4, 0xdb, 0x18, 0xcf, 0x27, 0x37, 0x3a, 0xfb, 0x82,
    ]);
    const host = new NetHost("Host", "P1", { methods: ["sharedSecret"], credentials: { sharedSecret: "5483", random: randomFor(nonce) } });
    const client = new NetClient("zmidi Client", "client-1", { methods: ["sharedSecret"], credentials: { sharedSecret: enc("5483") } });
    client.connect(0, HOST);
    const t = authPump(host, client);
    // Invitation Reply: Authentication Required with the CryptoNonce, state 0x00 (first request).
    expect(sent(t.fromHost, enc(nonce))).toBe(true);
    expect(of(t.atClient, "authRequested").map((e) => [e.code, e.value])).toEqual([[NET_AUTH_SHARED_SECRET, 0x00]]);
    // Invitation with Authentication with the digest of the example, and the session.
    expect(sent(t.fromClient, digest)).toBe(true);
    expect(of(t.atHost, "established").map((e) => e.code)).toEqual([NET_AUTH_SHARED_SECRET]);
    expect(of(t.atClient, "established").map((e) => e.code)).toEqual([NET_AUTH_SHARED_SECRET]);
    expect(of(t.atHost, "authFailed")).toEqual([]);
    expect(host.randomUsed).toBe(16);
    host.free();
    client.free();
  });

  test("user method, M2-124 6.10 example", () => {
    // CryptoNonce "XI|~=NNRVaD;XCPL", Username "Rosa", password "RPBqBno", and the digest.
    const nonce = "XI|~=NNRVaD;XCPL";
    const digest = new Uint8Array([
      0x3a, 0x2d, 0x5e, 0xbe, 0xef, 0x92, 0xe8, 0x46, 0x35, 0x35, 0xc2, 0x7f, 0xb4, 0xb7, 0xb3, 0xe2, 0xd1, 0xac, 0xf5, 0x7a, 0x84,
      0x4c, 0x6d, 0x08, 0x08, 0xe0, 0x41, 0xc1, 0x02, 0xb7, 0xff, 0x1a,
    ]);
    const users = [
      { username: "Ana", password: "other" },
      { username: "Rosa", password: "RPBqBno" },
    ];
    const host = new NetHost("Host", "P1", { methods: ["user"], credentials: { users, random: randomFor(nonce) } });
    const client = new NetClient("zmidi Client", "client-1", { methods: ["sharedSecret", "user"] });
    client.setCredentials({ users: [{ username: "Rosa", password: enc("RPBqBno") }] });
    client.connect(0, HOST);
    const t = authPump(host, client);
    expect(sent(t.fromHost, enc(nonce))).toBe(true);
    expect(of(t.atClient, "authRequested").map((e) => e.code)).toEqual([NET_AUTH_USER]);
    expect(sent(t.fromClient, digest)).toBe(true);
    expect(of(t.atHost, "established").map((e) => e.code)).toEqual([NET_AUTH_USER]);
    expect(of(t.atClient, "established").map((e) => e.code)).toEqual([NET_AUTH_USER]);
    // After a receive call, no password is left in the WASM memory: the package cleared its copy
    // of the credentials (heap), and zmidi-net finalizes the SHA-256 state in place, so the stack
    // of the Rust code keeps no copy of the last block (OPT-026).
    client.receive(1000, HOST, new Uint8Array(4));
    expect(sent([new Uint8Array(view().buffer, heapStart)], enc("RPBqBno"))).toBe(false);
    expect(sent([new Uint8Array(view().buffer)], enc("RPBqBno"))).toBe(false);
    host.free();
    client.free();
  });

  /**
   * A Client with `username` and `password` against a Host that knows "Rosa": AuthFailed on the
   * Host with `code`; the Client sees the Authentication State `state`; after the failure limit (5)
   * the Host ends the Invitation with Bye `bye`. No session.
   */
  function fails(username: string, password: string, code: number, state: number, bye: number): void {
    const random = new Uint8Array(64).map((_, i) => (i * 7 + 3) & 0xff);
    const host = new NetHost("Host", "P1", { methods: ["user"], credentials: { users: [{ username: "Rosa", password: "RPBqBno" }], random } });
    const client = new NetClient("zmidi Client", "client-1", { methods: ["user"], credentials: { users: [{ username, password }] } });
    client.connect(0, HOST);
    const t = authPump(host, client);
    const failed = of(t.atHost, "authFailed");
    expect(failed.length).toBe(5);
    expect([failed[4]?.code, failed[4]?.value]).toEqual([code, 5]);
    const requested = of(t.atClient, "authRequested");
    expect(requested.length).toBeGreaterThanOrEqual(2);
    expect(requested.at(-1)?.value).toBe(state);
    expect([...of(t.atHost, "established"), ...of(t.atClient, "established")]).toEqual([]);
    expect(of(t.atClient, "closed").map((e) => [e.code, e.value])).toEqual([[0, bye]]);
    expect(host.randomUsed).toBe(16);
    host.free();
    client.free();
  }

  test("user method with a wrong password fails", () => {
    fails("Rosa", "wrong", NET_AUTH_USER, 0x01, 0x43);
  });

  test("user method with an unknown Username fails", () => {
    fails("Nobody", "RPBqBno", NET_AUTH_USER | NET_AUTH_USERNAME_NOT_FOUND, 0x02, 0x44);
  });

  test("authentication bad values and credentials errors", () => {
    expect(() => new NetHost("Host", "P1", { methods: [] })).toThrow(InvalidValueError);
    expect(() => new NetClient("", "c", { methods: ["sharedSecret"] })).toThrow(InvalidValueError);
    expect(() => new NetHost("Host", "P1", { methods: ["other" as "user"] })).toThrow(InvalidValueError);
    // A Host with no random bytes left can not ask for authentication: credentialsError code 0.
    const host = new NetHost("Host", "P1", { methods: ["sharedSecret", "user"], credentials: { sharedSecret: "5483", random: new Uint8Array(0) } });
    const client = new NetClient("zmidi Client", "client-1", { methods: ["sharedSecret"], credentials: { sharedSecret: "5483" } });
    client.connect(0, HOST);
    const t = authPump(host, client);
    expect(of(t.atHost, "credentialsError").map((e) => e.code)).toContain(0);
    expect(of(t.atHost, "established")).toEqual([]);
    // With random bytes of crypto.getRandomValues (no `random`), the session starts.
    host.setCredentials({ sharedSecret: "5483" });
    const c2 = new NetClient("zmidi Client 2", "client-2", { methods: ["sharedSecret"], credentials: { sharedSecret: "5483" } });
    c2.connect(10_000, HOST);
    const t2 = authPump(host, c2, 10_000);
    expect(of(t2.atClient, "established").map((e) => e.code)).toEqual([NET_AUTH_SHARED_SECRET]);
    host.free();
    client.free();
    c2.free();
  });
});
