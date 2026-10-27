// MIDI-CI and one-chunk Property Exchange (REQ-TS-CI-001, REQ-TS-CI-002): two devices, M2-101 and
// M2-103. Profile Configuration and Process Inquiry (REQ-TS-CI-003 to REQ-TS-CI-005): the messages
// are compared byte for byte with M2-101 Tables 17 to 28 and 40 to 46, as tests/c/ci.c of zmidi-ffi
// does (SysEx7 payload: no F0 and F7).
import { beforeAll, describe, expect, test } from "vitest";

import { CiDevice, InvalidValueError, RejectedError, init } from "./index.js";
import type { CiEvent, CiMessage } from "./index.js";

beforeAll(async () => {
  await init();
});

const enc = (s: string): Uint8Array => new TextEncoder().encode(s);
const LIST = enc('[{"resource":"DeviceInfo"},{"resource":"X-Volume","canSet":"full"}]');

function device(seed: number): CiDevice {
  return new CiDevice({
    manufacturer: new Uint8Array([0x00, 0x21, 0x7f]),
    family: 1,
    model: 2,
    revision: new Uint8Array([0, 0, 1, 0]),
    productInstanceId: "zmidi-ts",
    resourceList: LIST,
    seed,
  });
}

function deliver(dev: CiDevice, messages: Uint8Array[]): { replies: Uint8Array[]; event: CiEvent | null } {
  const replies: Uint8Array[] = [];
  let event: CiEvent | null = null;
  for (const m of messages) {
    const r = dev.receive(0, m, 1000);
    replies.push(...r.replies.map((x) => x.message));
    event = r.event;
  }
  return { replies, event };
}

describe("ci", () => {
  test("Discovery, PE Get and Set between two devices", () => {
    const a = device(1);
    const b = device(2);
    expect(a.muid).not.toBe(b.muid);
    expect(() => a.peGet(b.muid, enc("{}"))).toThrow(RejectedError);
    let ev = deliver(a, deliver(b, [a.discovery()]).replies).event;
    expect(ev?.kind).toBe("discovered");
    expect(ev?.muid).toBe(b.muid);
    expect(deliver(a, deliver(b, [a.endpointInfoInquiry(b.muid)]).replies).event?.kind).toBe("endpointInfo");
    expect(deliver(a, deliver(b, [a.peCapabilitiesInquiry(b.muid)]).replies).event?.kind).toBe("peCapabilities");

    const get = a.peGet(b.muid, enc('{"resource":"DeviceInfo"}'));
    ev = deliver(b, get.chunks).event;
    expect([ev?.kind, ev?.resource, ev?.requestId]).toEqual(["peGet", "DeviceInfo", get.requestId]);
    const info = enc('{"manufacturerId":[0,33,127]}');
    ev = deliver(a, b.peReply(200, info)).event;
    expect(ev?.kind).toBe("peGetReply");
    expect(ev?.header).toEqual(enc('{"status":200}'));
    expect(ev?.data).toEqual(info);
    expect(() => b.peReply(200)).toThrow(RejectedError);

    deliver(b, a.peGet(b.muid, enc('{"resource":"ResourceList"}')).chunks);
    expect(deliver(a, b.peReply(404)).event?.data).toEqual(LIST);

    deliver(b, a.peGet(b.muid, enc('{"resource":"X-None"}')).chunks);
    expect(deliver(a, b.peReply(404)).event?.header).toEqual(enc('{"status":404}'));

    ev = deliver(b, a.peSet(b.muid, enc('{"resource":"X-Volume"}'), enc('{"value":100}')).chunks).event;
    expect(ev?.kind).toBe("peSet");
    expect(ev?.data).toEqual(enc('{"value":100}'));
    expect(deliver(a, b.peReply(200)).event?.kind).toBe("peSetReply");
    a.free();
    b.free();
  });

  test("bad CI configuration", () => {
    expect(() => new CiDevice({ manufacturer: new Uint8Array([0x00, 0x21]) })).toThrow(InvalidValueError);
    expect(() => new CiDevice({ manufacturer: new Uint8Array([0x7d]), maxSysexSize: 100 })).toThrow(InvalidValueError);
  });
});

// --- Profile Configuration (M2-101 7) and Process Inquiry (M2-101 9) ---

const BROADCAST = 0x0fffffff;
/** A Standard Defined Profile (M2-101 Table 19): Bank 0, Number 1, Version 1, Level 0x01 Minimum. */
const PROF = new Uint8Array([0x7e, 0x00, 0x01, 0x01, 0x01]);

/** M2-101 3.2: a MUID as 4 bytes of 7 bits, LSB first. */
const muidBytes = (m: number): number[] => [m & 0x7f, (m >>> 7) & 0x7f, (m >>> 14) & 0x7f, (m >>> 21) & 0x7f];

/** M2-101 Table 3: a MIDI-CI message, Message Version 0x02, with the 13-byte header and `body`. */
const ciMessage = (deviceId: number, subId2: number, src: number, dst: number, body: readonly number[] = []): Uint8Array =>
  new Uint8Array([0x7e, deviceId, 0x0d, subId2, 0x02, ...muidBytes(src), ...muidBytes(dst), ...body]);

/** Two devices A and B with Profiles and Process Inquiry, after Discovery. */
function pair(): { a: CiDevice; b: CiDevice; ma: number; mb: number } {
  const options = {
    manufacturer: new Uint8Array([0x00, 0x21, 0x7f]),
    family: 1,
    model: 2,
    revision: new Uint8Array([0, 0, 1, 0]),
    productInstanceId: "zmidi-ts",
    profiles: true,
    processInquiry: true,
    resourceList: LIST,
  };
  const a = new CiDevice({ ...options, seed: 11 });
  const b = new CiDevice({ ...options, seed: 12 });
  const ev = deliver(a, deliver(b, [a.discovery()]).replies).event;
  expect(ev?.kind).toBe("discovered");
  // Discovered is not a Profile or Process Inquiry event.
  expect([ev?.profile, ev?.process]).toEqual([null, null]);
  return { a, b, ma: a.muid, mb: b.muid };
}

const messages = (records: readonly CiMessage[]): Uint8Array[] => records.map((r) => r.message);

describe("ci profiles and process inquiry", () => {
  test("Profile Configuration between two devices (M2-101 7)", () => {
    const { a, b, ma, mb } = pair();

    // B adds a disabled Profile on Channel 1: Profile Added Report to Broadcast (Table 20), Group 0.
    const added = b.addProfile({ deviceId: 0x00, profile: PROF, maxChannels: 1 });
    expect(added).toEqual([{ group: 0, message: ciMessage(0x00, 0x26, mb, BROADCAST, [...PROF]) }]);
    expect(() => b.addProfile({ deviceId: 0x00, profile: PROF, maxChannels: 1 })).toThrow(RejectedError);

    // Set Profile On before Profile Inquiry: rejected (M2-101 2.2).
    expect(() => a.setProfileOn(mb, 0, 0x00, PROF, 1)).toThrow(RejectedError);

    // Profile Inquiry (Table 17), the automatic Reply to Profile Inquiry (Table 18).
    const inquiry = a.profileInquiry(mb, 0, 0x00);
    expect(inquiry).toEqual(ciMessage(0x00, 0x20, ma, mb));
    const reply = deliver(b, [inquiry]).replies;
    expect(reply).toEqual([ciMessage(0x00, 0x21, mb, ma, [0, 0, 1, 0, ...PROF])]);
    let ev = deliver(a, reply).event;
    expect([ev?.kind, ev?.muid]).toEqual(["profileInquiryReply", mb]);
    expect(ev?.profile?.deviceId).toBe(0x00);
    expect(ev?.profile?.enabledProfiles).toEqual([]);
    expect(ev?.profile?.disabledProfiles).toEqual([PROF]);

    // Set Profile On (Table 24): Level 0x7F, 1 Channel; the SetProfile event; B enables the
    // Profile and sends the Profile Enabled Report (Table 26).
    const on = a.setProfileOn(mb, 0, 0x00, PROF, 1);
    expect(on).toEqual(ciMessage(0x00, 0x22, ma, mb, [...PROF.subarray(0, 4), 0x7f, 0x01, 0x00]));
    let r = deliver(b, [on]);
    expect([r.event?.kind, r.event?.muid, r.replies.length]).toEqual(["setProfile", ma, 0]);
    const set = r.event!.profile!;
    expect([set.on, set.channels, set.deviceId, set.group]).toEqual([true, 1, 0x00, 0]);
    expect(set.profile.subarray(0, 4)).toEqual(PROF.subarray(0, 4));
    const enabled = b.setProfileState(set.deviceId, set.group, set.profile, true, set.channels);
    expect(enabled).toEqual({ group: 0, message: ciMessage(0x00, 0x24, mb, BROADCAST, [...PROF, 0x01, 0x00]) });
    ev = deliver(a, [enabled.message]).event;
    expect(ev?.kind).toBe("profileReport");
    expect([ev?.profile?.report, ev?.profile?.channels, ev?.profile?.profile]).toEqual(["enabled", 1, PROF]);

    // Profile Details Inquiry (Table 22), Inquiry Target 0x00: B replies by itself with the Number
    // of MIDI Channels (M2-102 2.5.1): 1 in use, maximum 1.
    const details = a.profileDetailsInquiry(mb, 0, 0x00, PROF, 0x00);
    expect(details).toEqual(ciMessage(0x00, 0x28, ma, mb, [...PROF, 0x00]));
    ev = deliver(a, deliver(b, [details]).replies).event;
    expect(ev?.kind).toBe("profileDetailsReply");
    expect([ev?.profile?.target, ev?.profile?.notification, ev?.profile?.data]).toEqual([0, false, new Uint8Array([1, 0, 1, 0])]);

    // Inquiry Target 0x40 (Profile Specific): the program of B answers (Table 23).
    r = deliver(b, [a.profileDetailsInquiry(mb, 0, 0x00, PROF, 0x40)]);
    expect([r.event?.kind, r.replies.length, r.event?.profile?.target]).toEqual(["profileDetailsInquiry", 0, 0x40]);
    const p = r.event!.profile!;
    const detail = new Uint8Array([0x01, 0x02]);
    const answer = b.profileDetailsReply(ma, 0, p.deviceId, p.profile, p.target, detail);
    expect(answer).toEqual(ciMessage(0x00, 0x29, mb, ma, [...PROF, 0x40, 0x02, 0x00, 0x01, 0x02]));
    ev = deliver(a, [answer]).event;
    expect([ev?.profile?.target, ev?.profile?.notification, ev?.profile?.data]).toEqual([0x40, false, detail]);

    // Profile Specific Data (Table 28): a 4-byte length, LSB first.
    const psd = new Uint8Array([0x10, 0x20, 0x30]);
    const data = a.profileSpecificData(mb, 0, 0x00, PROF, psd);
    expect(data).toEqual(ciMessage(0x00, 0x2f, ma, mb, [...PROF, 3, 0, 0, 0, ...psd]));
    ev = deliver(b, [data]).event;
    expect([ev?.kind, ev?.profile?.data]).toEqual(["profileSpecificData", psd]);

    // Set Profile Off (Table 25), then the Profile Disabled Report (Table 27).
    const off = a.setProfileOff(mb, 0, 0x00, PROF);
    expect(off).toEqual(ciMessage(0x00, 0x23, ma, mb, [...PROF, 0, 0]));
    expect(deliver(b, [off]).event?.profile?.on).toBe(false);
    const disabled = b.setProfileState(0x00, 0, PROF, false, 1);
    expect(disabled.message).toEqual(ciMessage(0x00, 0x25, mb, BROADCAST, [...PROF, 0x01, 0x00]));
    expect(deliver(a, [disabled.message]).event?.profile?.report).toBe("disabled");

    // B removes the disabled Profile: Profile Removed Report only (Table 21).
    expect(messages(b.removeProfile(0x00, 0, PROF))).toEqual([ciMessage(0x00, 0x27, mb, BROADCAST, [...PROF])]);
    expect(() => b.removeProfile(0x00, 0, PROF)).toThrow(RejectedError);
    expect(() => b.setProfileState(0x00, 0, PROF, true, 1)).toThrow(RejectedError);

    // An enabled Profile: Added then Enabled; on removal Disabled then Removed (M2-101 7.4, 7.5).
    const both = b.addProfile({ deviceId: 0x7f, profile: PROF, enabled: true, channels: 1, maxChannels: 1 });
    expect(messages(both).map((m) => m[3])).toEqual([0x26, 0x24]);
    expect(messages(b.removeProfile(0x7f, 0, PROF)).map((m) => m[3])).toEqual([0x25, 0x27]);
    a.free();
    b.free();
  });

  test("Process Inquiry between two devices (M2-101 9)", () => {
    const { a, b, ma, mb } = pair();

    // Process Inquiry Capabilities (Table 40) and the automatic reply (Table 41: D0 set).
    const caps = a.piCapabilitiesInquiry(mb);
    expect(caps).toEqual(ciMessage(0x7f, 0x40, ma, mb));
    const reply = deliver(b, [caps]).replies;
    expect(reply).toEqual([ciMessage(0x7f, 0x41, mb, ma, [0x01])]);
    let ev = deliver(a, reply).event;
    expect([ev?.kind, ev?.process?.midiMessageReport, ev?.profile]).toEqual(["piCapabilities", true, null]);

    // Inquiry: MIDI Message Report (Table 43): full report of MTC Quarter Frame, Control Change
    // and Notes.
    const requested = { dataControl: 0x7f, system: 0x01, channel: 0x02, note: 0x01 };
    expect(a.reportOpen(mb)).toBe(false);
    const inquiry = a.messageReportInquiry(mb, 0, 0x7f, requested);
    expect(inquiry).toEqual(ciMessage(0x7f, 0x42, ma, mb, [0x7f, 0x01, 0x00, 0x02, 0x01]));
    expect(a.reportOpen(mb)).toBe(true);
    const r = deliver(b, [inquiry]);
    expect([r.event?.kind, r.replies.length]).toEqual(["messageReport", 0]);
    const pi = r.event!.process!;
    expect([pi.deviceId, pi.group, pi.report]).toEqual([0x7f, 0, requested]);

    // Reply to MIDI Message Report (Table 45): only the bits both requested and supported.
    const supported = { dataControl: 0x00, system: 0x00, channel: 0x03, note: 0x00 };
    const answer = b.messageReportReply(ma, 0, pi.deviceId, pi.report, supported);
    expect(answer).toEqual(ciMessage(0x7f, 0x43, mb, ma, [0x00, 0x00, 0x02, 0x00]));
    ev = deliver(a, [answer]).event;
    expect(ev?.kind).toBe("messageReportReply");
    expect(ev?.process?.report).toEqual({ dataControl: 0, system: 0, channel: 0x02, note: 0 });

    // End of MIDI Message Report (Table 46) closes the report.
    const end = b.messageReportEnd(ma, 0, 0x7f);
    expect(end).toEqual(ciMessage(0x7f, 0x44, mb, ma));
    expect(deliver(a, [end]).event?.kind).toBe("messageReportEnd");
    expect(a.reportOpen(mb)).toBe(false);

    // Abort: NAK (Table 15) for Sub-ID#2 0x42 with Status Code 0x20, or 0x40 (M2-101 9.8.1).
    const nak = b.abortReport(ma, 0, 0x7f);
    expect([nak[1], nak[3], nak[13], nak[14]]).toEqual([0x7f, 0x7f, 0x42, 0x20]);
    expect(b.abortReport(ma, 0, 0x7f, true)[14]).toBe(0x40);
    a.free();
    b.free();
  });

  test("Profile and Process Inquiry bad values", () => {
    const { a, b, mb } = pair();
    // A reserved Device ID (M2-101 Table 4), a Profile ID that is not 5 bytes, bad Groups.
    expect(() => b.addProfile({ deviceId: 0x10, profile: PROF })).toThrow(InvalidValueError);
    expect(() => b.addProfile({ deviceId: 0x00, profile: PROF.subarray(0, 4) })).toThrow(InvalidValueError);
    expect(() => b.addProfile({ deviceId: 0x00, profile: PROF, channels: 0x4000 })).toThrow(InvalidValueError);
    expect(() => b.setProfileState(0x00, 0, PROF, true, 0x4000)).toThrow(InvalidValueError);
    expect(() => a.profileInquiry(mb, 0, 0x10)).toThrow(InvalidValueError);
    expect(() => b.messageReportEnd(a.muid, 16, 0x7f)).toThrow(InvalidValueError);
    // A bitmap over 0x7F (M2-101 Table 43).
    expect(() => a.messageReportInquiry(mb, 0, 0x7f, { dataControl: 0x80, system: 0, channel: 0, note: 0 })).toThrow(InvalidValueError);
    a.free();
    b.free();
    expect(() => b.profileInquiry(mb, 0, 0)).toThrow(RejectedError);
  });

  test("the Request IDs of peGet and peSet are freed", () => {
    // REQ-TS-CI-006: the last Chunk of a reply frees the ID at the next receive; peEnd frees the
    // ID of a request with no reply. The device has 4 Request IDs.
    const a = device(1);
    const b = device(2);
    deliver(a, deliver(b, [a.discovery()]).replies);
    deliver(a, deliver(b, [a.peCapabilitiesInquiry(b.muid)]).replies);
    const get = enc('{"resource":"DeviceInfo"}');
    let rid = -1;
    for (let i = 0; i < 6; i++) {
      const r = a.peGet(b.muid, get);
      rid = r.requestId;
      deliver(b, r.chunks);
      const ev = deliver(a, b.peReply(200, enc("{}"))).event;
      expect([ev?.kind, ev?.requestId]).toEqual(["peGetReply", rid]);
    }
    // Until the next receive, a new request gets another ID.
    const second = a.peGet(b.muid, get);
    expect(second.requestId).not.toBe(rid);
    deliver(b, second.chunks);
    deliver(a, b.peReply(200, enc("{}")));

    a.peEnd(b.muid, second.requestId);
    const ids = [0, 1, 2, 3].map(() => a.peGet(b.muid, enc("{}")).requestId);
    expect(ids[0]).toBe(second.requestId);
    deliver(a, deliver(b, [a.endpointInfoInquiry(b.muid)]).replies);
    expect(() => a.peGet(b.muid, enc("{}"))).toThrow(RejectedError);
    a.peEnd(b.muid, ids[2]!);
    expect(a.peGet(b.muid, enc("{}")).requestId).toBe(ids[2]);
    expect(() => a.peEnd(b.muid, 0x80)).toThrow(InvalidValueError);
    expect(() => a.peEnd(0x10000000, 0)).toThrow(InvalidValueError);
    for (const id of ids) {
      a.peEnd(b.muid, id);
    }

    // A reply of 2 Chunks (M2-101 Table 29): only the last Chunk frees the ID.
    const u14 = (v: number): number[] => [v & 0x7f, v >> 7];
    const chunk = (id: number, header: string, total: number, number: number, data: string): Uint8Array =>
      ciMessage(0x7f, 0x35, b.muid, a.muid, [id, ...u14(header.length), ...enc(header), ...u14(total), ...u14(number), ...u14(data.length), ...enc(data)]);
    const first = a.peGet(b.muid, enc("{}")).requestId;
    let ev = deliver(a, [chunk(first, '{"status":200}', 2, 1, '{"a":')]).event;
    expect([ev?.chunkNumber, ev?.chunkTotal]).toEqual([1, 2]);
    ev = deliver(a, [chunk(first, "", 2, 2, "1}")]).event;
    expect([ev?.chunkNumber, ev?.requestId]).toEqual([2, first]);
    expect(a.peGet(b.muid, enc("{}")).requestId).not.toBe(first);
    a.free();
    b.free();
  });
});
