// The Property Exchange engine (REQ-TS-PE-*): two devices, A (Initiator) and B (Responder), each with
// a CiDevice and a PeEngine. M2-103: Get with many Chunks (5.3), ResourceList (9), 404 (8.6.2), Set
// with many Chunks, Subscription start, full and end (11), timeouts (5.3, 12), MIDI-CI ACK 0x10 and
// NAK 0x20; the same cases as tests/c/pe.c of zmidi-ffi.
import { beforeAll, describe, expect, test } from "vitest";

import { CiDevice, InvalidValueError, PeEngine, RejectedError, init } from "./index.js";
import type { PeEvent, PeResource, PeRole } from "./index.js";

beforeAll(async () => {
  await init();
});

const NOW = 1_000_000;
const enc = (s: string): Uint8Array => new TextEncoder().encode(s);
const text = (b: Uint8Array): string => new TextDecoder().decode(b);
const join = (parts: Uint8Array[]): Uint8Array => {
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let at = 0;
  for (const p of parts) {
    out.set(p, at);
    at += p.length;
  }
  return out;
};

function device(seed: number): CiDevice {
  return new CiDevice({ manufacturer: new Uint8Array([0x00, 0x21, 0x7f]), family: 1, model: 2, revision: new Uint8Array([0, 0, 1, 0]), seed });
}

/** Gives each message to `dev` and then to `eng`; gives the automatic replies and the engine events. */
function deliver(dev: CiDevice, eng: PeEngine, messages: Uint8Array[]): { replies: Uint8Array[]; events: PeEvent[] } {
  const replies: Uint8Array[] = [];
  const events: PeEvent[] = [];
  for (const m of messages) {
    replies.push(...dev.receive(0, m, NOW).replies.map((r) => r.message));
    const ev = eng.receive(dev, NOW);
    if (ev !== null) {
      events.push(ev);
    }
  }
  return { replies, events };
}

function pending(dev: CiDevice, eng: PeEngine): Uint8Array[] {
  const out: Uint8Array[] = [];
  for (let m = eng.writePending(dev, NOW); m !== null; m = eng.writePending(dev, NOW)) {
    out.push(m);
  }
  return out;
}

/** Sends `data` as the Chunks of the transaction. */
function sendAll(dev: CiDevice, eng: PeEngine, peer: number, rid: number, role: PeRole, data: Uint8Array = new Uint8Array(0)): Uint8Array[] {
  const chunks: Uint8Array[] = [];
  let sent = 0;
  for (;;) {
    const room = eng.dataRoom(dev, peer, rid, role);
    const part = data.subarray(sent, sent + room);
    sent += part.length;
    chunks.push(eng.send(dev, peer, rid, role, part, sent === data.length, NOW));
    if (sent === data.length) {
      return chunks;
    }
  }
}

const TABLE: PeResource[] = [
  { resource: "DeviceInfo" },
  { resource: "X-Big", canSet: "full" },
  { resource: "X-Sub", canSubscribe: true },
  { resource: "X-Mc", encodings: ["ASCII", "Mcoded7"], schema: enc('{"type":"object"}') },
];

/** A and B after Discovery and PE Capabilities in both directions. */
function pair(): { a: CiDevice; pa: PeEngine; b: CiDevice; pb: PeEngine } {
  const a = device(1);
  const b = device(2);
  const pa = new PeEngine();
  const pb = new PeEngine(TABLE);
  for (const [x, y, px, py] of [
    [a, b, pa, pb],
    [b, a, pb, pa],
  ] as const) {
    deliver(x, px, deliver(y, py, [x.discovery()]).replies);
    expect(deliver(x, px, deliver(y, py, [x.peCapabilitiesInquiry(y.muid)]).replies).events).toEqual([]);
  }
  return { a, pa, b, pb };
}

describe("pe", () => {
  test("Resource checks", () => {
    // REQ-TS-PE-001.
    for (const bad of ["Bad-Name", "X-", "", "A".repeat(37)]) {
      expect(() => new PeEngine([{ resource: bad }])).toThrow(InvalidValueError);
    }
    expect(() => new PeEngine([{ resource: "DeviceInfo", encodings: [] }])).toThrow(InvalidValueError);
    expect(() => new PeEngine([{ resource: "DeviceInfo", mediaType: "json" }])).toThrow(InvalidValueError);
    expect(() => new PeEngine([{ resource: "DeviceInfo", schema: new Uint8Array(600).fill(0x78) }])).toThrow(InvalidValueError);
    expect(() => new PeEngine(new Array<PeResource>(17).fill({ resource: "DeviceInfo" }))).toThrow(InvalidValueError);
    // @ts-expect-error: a canSet that M2-103 does not name.
    expect(() => new PeEngine([{ resource: "DeviceInfo", canSet: "some" }])).toThrow(InvalidValueError);
    new PeEngine([{ resource: "DeviceInfo", mediaType: "application/octet-stream" }]).free();
    const a = device(1);
    expect(() => new PeEngine().request(a, 0x1000_0000, "get", "DeviceInfo")).toThrow(InvalidValueError);
    // Before Discovery and PE Capabilities the device refuses (M2-101 8.4).
    expect(() => new PeEngine().request(a, 0x123, "get", "DeviceInfo")).toThrow(RejectedError);
  });

  test("ResourceList and a Get in many Chunks", () => {
    // REQ-TS-PE-002, -003, -005.
    const { a, pa, b, pb } = pair();
    expect(pa.receive(a, NOW)).toBeNull();

    const r = pa.requestResourceList(a, b.muid, NOW);
    let { events } = deliver(b, pb, [r.message]);
    expect(events).toHaveLength(1);
    const ev = events[0]!;
    expect(ev).toMatchObject({ kind: "message", role: "responder", txn: "get", end: "complete", resource: "ResourceList", resourceIndex: null, requestId: r.requestId, muid: a.muid });
    expect(text(ev.header)).toBe('{"resource":"ResourceList"}');
    const listing = pb.resourceList();
    expect(text(listing)).toContain('"resource":"X-Sub","canSubscribe":true');
    expect(text(listing)).toContain('"schema":{"type":"object"}');
    pb.reply(b, a.muid, r.requestId, { length: listing.length });
    events = deliver(a, pa, sendAll(b, pb, a.muid, r.requestId, "responder", listing)).events;
    expect(events[0]!.status).toBe(200);
    expect(events[0]!.role).toBe("initiator");
    expect(events.at(-1)!.end).toBe("complete");
    expect(join(events.map((e) => e.data))).toEqual(listing);

    const g = pa.request(a, b.muid, "get", "DeviceInfo");
    expect(g.resourceListMissing).toBe(false);
    events = deliver(b, pb, sendAll(a, pa, b.muid, g.requestId, "initiator")).events;
    expect(events[0]).toMatchObject({ resource: "DeviceInfo", resourceIndex: 0 });
    const body = enc(`"${Array.from({ length: 1198 }, (_, i) => String.fromCharCode(97 + (i % 26))).join("")}"`);
    pb.reply(b, a.muid, g.requestId, { length: body.length });
    expect(() => pb.reply(b, a.muid, g.requestId, { length: body.length })).toThrow(RejectedError);
    expect(() => pb.send(b, a.muid, g.requestId, "responder", body, false, NOW)).toThrow(InvalidValueError);
    const chunks = sendAll(b, pb, a.muid, g.requestId, "responder", body);
    expect(chunks).toHaveLength(3);
    events = deliver(a, pa, chunks).events;
    expect(events.map((e) => e.end)).toEqual(["more", "more", "complete"]);
    expect(events[0]!.header.length).toBeGreaterThan(0);
    expect(events[1]!.header.length).toBe(0);
    expect(join(events.map((e) => e.data))).toEqual(body);
    expect(pa.receive(a, NOW)).toBeNull();
    expect(() => pb.send(b, a.muid, g.requestId, "responder", new Uint8Array(0), true, NOW)).toThrow(RejectedError);
    for (const x of [pa, pb]) {
      x.free();
    }
  });

  test("404, a Set in two Chunks and a refusal", () => {
    // REQ-TS-PE-003, -005.
    const { a, pa, b, pb } = pair();
    const n = pa.request(a, b.muid, "get", "X-None");
    expect(n.resourceListMissing).toBe(true);
    expect(deliver(b, pb, sendAll(a, pa, b.muid, n.requestId, "initiator")).events).toEqual([]);
    let { events } = deliver(a, pa, pending(b, pb));
    expect(events[0]).toMatchObject({ status: 404, end: "complete" });

    // REQ-TS-CI-006: a request between CiDevice.receive and PeEngine.receive of the last Chunk of a
    // reply gets another Request ID; the engine then closes only the old request.
    const id1 = pa.request(a, b.muid, "get", "X-None").requestId;
    deliver(b, pb, sendAll(a, pa, b.muid, id1, "initiator"));
    const [reply] = pending(b, pb);
    a.receive(0, reply!, NOW);
    const id2 = pa.request(a, b.muid, "get", "X-None").requestId;
    expect(id2).not.toBe(id1);
    expect(pa.receive(a, NOW)).toMatchObject({ requestId: id1, status: 404 });
    deliver(b, pb, sendAll(a, pa, b.muid, id2, "initiator"));
    ({ events } = deliver(a, pa, pending(b, pb)));
    expect(events[0]).toMatchObject({ requestId: id2, status: 404 });

    expect(() => pa.request(a, b.muid, "set", "DeviceInfo", { length: 2, caps: TABLE[0]! })).toThrow(/status 405/);
    // @ts-expect-error: a kind that is not a request.
    expect(() => pa.request(a, b.muid, "put", "DeviceInfo")).toThrow(InvalidValueError);
    expect(() => pa.request(a, b.muid, "get", "DeviceInfo", { offset: 0 })).toThrow(InvalidValueError);

    const data = enc("x".repeat(700));
    const s = pa.request(a, b.muid, "set", "X-Big", { length: data.length, caps: TABLE[1]! });
    const chunks = sendAll(a, pa, b.muid, s.requestId, "initiator", data);
    expect(chunks).toHaveLength(2);
    events = deliver(b, pb, chunks).events;
    expect(events[0]).toMatchObject({ txn: "set", resourceIndex: 1 });
    expect(events[1]!.end).toBe("complete");
    expect(join(events.map((e) => e.data))).toEqual(data);
    pb.reply(b, a.muid, s.requestId);
    events = deliver(a, pa, sendAll(b, pb, a.muid, s.requestId, "responder")).events;
    expect(events[0]).toMatchObject({ txn: "set", status: 200 });
  });

  test("Subscription start, full, refresh and end", () => {
    // REQ-TS-PE-004, -005, -006.
    const { a, pa, b, pb } = pair();
    const st = pa.subscribe(a, b.muid, "X-Sub", { caps: TABLE[2]! });
    let { events } = deliver(b, pb, sendAll(a, pa, b.muid, st.requestId, "initiator"));
    const index = events.find((e) => e.kind === "subscriptionStarted")!.subscription!;
    const sub = pb.subscription(index)!;
    expect(sub).toMatchObject({ muid: a.muid, role: "responder", resource: "X-Sub" });
    events = deliver(a, pa, pending(b, pb)).events;
    expect(events).toHaveLength(1);
    expect(events[0]).toMatchObject({ txn: "subscription", status: 200, startError: null, subscribeId: sub.subscribeId });
    const mine = pa.subscription(events[0]!.started!)!;
    expect(mine).toMatchObject({ role: "initiator", muid: b.muid, resId: null });
    expect(pa.subscription(3)).toBeNull();

    // @ts-expect-error: "start" is not an update.
    expect(() => pb.update(b, index, "start")).toThrow(InvalidValueError);
    const rid = pb.update(b, index, "full", { length: 20 });
    events = deliver(a, pa, sendAll(b, pb, a.muid, rid, "initiator", enc("0123456789abcdefghij"))).events;
    expect(events[0]).toMatchObject({ command: "full", role: "responder", subscription: 0 });
    expect(text(events[0]!.data)).toBe("0123456789abcdefghij");
    events = deliver(b, pb, pending(a, pa)).events;
    expect(events[0]).toMatchObject({ status: 200, role: "initiator" });

    expect(() => pa.refresh(99)).toThrow(RejectedError);
    pa.refresh(0);
    events = deliver(b, pb, pending(a, pa)).events;
    expect(events[0]).toMatchObject({ txn: "get", resource: "X-Sub" });
    const g = events[0]!.requestId!;
    pb.reply(b, a.muid, g, { length: 2 });
    events = deliver(a, pa, sendAll(b, pb, a.muid, g, "responder", enc("{}"))).events;
    expect(events[0]).toMatchObject({ subscription: 0 });

    const end = pa.unsubscribe(a, 0);
    expect(pa.subscription(0)).toBeNull();
    events = deliver(b, pb, sendAll(a, pa, b.muid, end, "initiator")).events;
    expect(events.at(-1)).toMatchObject({ kind: "subscriptionEnded", muid: a.muid, resource: "X-Sub" });
    expect(pb.subscription(index)).toBeNull();
    deliver(a, pa, pending(b, pb));
    expect(pa.takeEnded()).toBeNull();
  });

  test("wait, abort, timeouts and a late reply", () => {
    // REQ-TS-PE-003, -007.
    const { a, pa, b, pb } = pair();
    let g = pa.request(a, b.muid, "get", "DeviceInfo");
    deliver(b, pb, sendAll(a, pa, b.muid, g.requestId, "initiator"));
    for (const bad of [0, 12701]) {
      expect(() => pb.wait(a.muid, g.requestId, "responder", bad)).toThrow(InvalidValueError);
    }
    // @ts-expect-error: a role that M2-103 does not name.
    expect(() => pb.wait(a.muid, g.requestId, "observer", 500)).toThrow(InvalidValueError);
    pb.wait(a.muid, g.requestId, "responder", 500);
    const [ack] = pending(b, pb);
    expect(ack![3]).toBe(0x7d);
    deliver(a, pa, [ack!]);
    expect(() => pb.corrupt(a.muid, g.requestId, "responder")).toThrow(RejectedError);
    pb.abort(b, a.muid, g.requestId, "responder");
    expect(() => pb.abort(b, a.muid, g.requestId, "responder")).toThrow(RejectedError);
    const [nak] = pending(b, pb);
    expect(nak![3]).toBe(0x7f);
    let { events } = deliver(a, pa, [nak!]);
    expect(events[0]).toMatchObject({ kind: "closed", reason: "terminated", requestId: g.requestId });

    g = pa.request(a, b.muid, "get", "DeviceInfo");
    sendAll(a, pa, b.muid, g.requestId, "initiator");
    expect(pa.poll(a, NOW + 1_000_000)).toBeNull();
    expect(pa.poll(a, NOW + 3_002_000)).toMatchObject({ kind: "closed", reason: "timeout", requestId: g.requestId });
    expect(pa.poll(a, NOW + 3_002_000)).toBeNull();
    const [timeout] = pending(a, pa);
    // NAK of a Get, Status Code 0x42 (M2-101 Table 16).
    expect([timeout![3], timeout![13], timeout![14]]).toEqual([0x7f, 0x34, 0x42]);

    g = pa.request(a, b.muid, "get", "DeviceInfo");
    deliver(b, pb, sendAll(a, pa, b.muid, g.requestId, "initiator"));
    expect(() => pb.reply(b, a.muid, g.requestId, { length: 2, nowUs: NOW + 4_000_000 })).toThrow(/deadline/);
    expect(() => pb.replyStatus(b, a.muid, g.requestId, 404)).toThrow(RejectedError);

    g = pa.request(a, b.muid, "get", "DeviceInfo");
    deliver(b, pb, sendAll(a, pa, b.muid, g.requestId, "initiator"));
    pb.replyStatus(b, a.muid, g.requestId, 413);
    events = deliver(a, pa, pending(b, pb)).events;
    expect(events[0]!.status).toBe(413);
    // A freed engine refuses.
    pb.free();
    expect(() => pb.resourceList()).toThrow(RejectedError);
  });
});
