// MIDI-CI Profile Configuration and Process Inquiry (REQ-DART-CI-003 to REQ-DART-CI-006): two
// devices, each message compared byte for byte with M2-101 Tables 17 to 28 and 40 to 46, the same
// vectors as crates/zmidi-ffi/tests/c/ci.c (SysEx7 payload: no F0 and F7).
import 'dart:typed_data';

import 'package:test/test.dart';
import 'package:zetaohm_midi/zetaohm_midi.dart';

// M2-101 3.2: a MUID as 4 bytes of 7 bits, LSB first.
List<int> muidBytes(int m) => [
  m & 0x7F,
  (m >> 7) & 0x7F,
  (m >> 14) & 0x7F,
  (m >> 21) & 0x7F,
];

// M2-101 Table 3: the 13-byte header of a MIDI-CI message, Message Version 0x02.
List<int> ciHeader(int deviceId, int subId2, int src, int dst) => [
  0x7E,
  deviceId,
  0x0D,
  subId2,
  0x02,
  ...muidBytes(src),
  ...muidBytes(dst),
];

const broadcast = 0x0FFFFFFF;

// A Standard Defined Profile (M2-101 Table 19): Bank 0, Number 1, Version 1, Level 0x01 Minimum.
const prof = [0x7E, 0x00, 0x01, 0x01, 0x01];

CiDevice device(int seed) => CiDevice(
  manufacturer: [0x00, 0x21, 0x7F],
  family: 1,
  model: 2,
  revision: [0, 0, 1, 0],
  productInstanceId: 'zmidi-dart',
  profiles: true,
  processInquiry: true,
  seed: seed,
);

({List<Uint8List> replies, CiEvent? event}) deliver(
  CiDevice dev,
  List<Uint8List> messages,
) {
  final replies = <Uint8List>[];
  CiEvent? event;
  for (final m in messages) {
    final r = dev.receive(0, m, 1000);
    replies.addAll(r.replies.map((x) => x.message));
    event = r.event;
  }
  return (replies: replies, event: event);
}

void main() {
  setUpAll(initWeb);

  test('Profile Configuration between two devices', () {
    final a = device(11);
    final b = device(12);
    final ma = a.muid;
    final mb = b.muid;
    var e = deliver(a, deliver(b, [a.discovery()]).replies).event!;
    expect(
      [e.kind, e.profile, e.process],
      [CiEventKind.discovered, null, null],
    );

    // B adds a disabled Profile on Channel 1: Profile Added Report to Broadcast (Table 20).
    final added = b.addProfile(0x00, prof, maxChannels: 1);
    expect(added.single.group, 0);
    expect(added.single.message, [
      ...ciHeader(0x00, 0x26, mb, broadcast),
      ...prof,
    ]);
    expect(
      () => b.addProfile(0x00, prof, maxChannels: 1),
      throwsA(isA<RejectedException>()),
    );
    // A reserved Device ID (Table 4).
    expect(
      () => b.addProfile(0x10, prof),
      throwsA(isA<InvalidValueException>()),
    );

    // Set Profile On before Profile Inquiry: rejected (M2-101 2.2).
    expect(
      () => a.setProfileOn(mb, 0x00, prof),
      throwsA(isA<RejectedException>()),
    );

    // Profile Inquiry (Table 17) and the automatic Reply to Profile Inquiry (Table 18).
    final inquiry = a.profileInquiry(mb, 0x00);
    expect(inquiry, ciHeader(0x00, 0x20, ma, mb));
    final reply = deliver(b, [inquiry]).replies;
    expect(reply, [
      [...ciHeader(0x00, 0x21, mb, ma), 0, 0, 1, 0, ...prof],
    ]);
    e = deliver(a, reply).event!;
    expect([e.kind, e.muid], [CiEventKind.profileInquiryReply, mb]);
    expect(e.profile!.deviceId, 0x00);
    expect(e.profile!.enabledProfiles, isEmpty);
    expect(e.profile!.disabledProfiles, [prof]);

    // Set Profile On (Table 24): Level 0x7F, 1 Channel; the SetProfile event; B enables the
    // Profile and sends the Profile Enabled Report (Table 26).
    final on = a.setProfileOn(mb, 0x00, prof);
    expect(on, [
      ...ciHeader(0x00, 0x22, ma, mb),
      ...prof.sublist(0, 4),
      0x7F,
      0x01,
      0x00,
    ]);
    var r = deliver(b, [on]);
    expect(
      [r.event?.kind, r.event?.muid, r.replies],
      [CiEventKind.setProfile, ma, isEmpty],
    );
    var pe = r.event!.profile!;
    expect([pe.on, pe.channels, pe.deviceId, pe.group], [true, 1, 0x00, 0]);
    expect(pe.profile.sublist(0, 4), prof.sublist(0, 4));
    final enabled = b.setProfileState(
      pe.deviceId,
      prof,
      group: pe.group,
      enabled: true,
      channels: pe.channels,
    );
    expect(enabled.single.message, [
      ...ciHeader(0x00, 0x24, mb, broadcast),
      ...prof,
      0x01,
      0x00,
    ]);
    e = deliver(a, [enabled.single.message]).event!;
    expect(e.kind, CiEventKind.profileReport);
    expect(
      [e.profile!.report, e.profile!.channels, e.profile!.profile],
      [CiProfileReport.enabled, 1, prof],
    );

    // Profile Details Inquiry (Table 22), Inquiry Target 0x00: B replies by itself with the
    // Number of MIDI Channels (M2-102 2.5.1): 1 in use, maximum 1.
    final details = a.profileDetailsInquiry(mb, 0x00, prof, 0x00);
    expect(details, [...ciHeader(0x00, 0x28, ma, mb), ...prof, 0x00]);
    e = deliver(a, deliver(b, [details]).replies).event!;
    expect(e.kind, CiEventKind.profileDetailsReply);
    expect(
      [e.profile!.target, e.profile!.notification, e.profile!.data],
      [
        0,
        false,
        [1, 0, 1, 0],
      ],
    );

    // Inquiry Target 0x40 (Profile Specific): the program of B answers (Table 23).
    r = deliver(b, [a.profileDetailsInquiry(mb, 0x00, prof, 0x40)]);
    expect(
      [r.event?.kind, r.replies],
      [CiEventKind.profileDetailsInquiry, isEmpty],
    );
    pe = r.event!.profile!;
    expect(pe.target, 0x40);
    final answer = b.profileDetailsReply(
      ma,
      pe.deviceId,
      pe.profile,
      pe.target,
      [0x01, 0x02],
    );
    expect(answer, [
      ...ciHeader(0x00, 0x29, mb, ma),
      ...prof,
      0x40,
      0x02,
      0x00,
      0x01,
      0x02,
    ]);
    pe = deliver(a, [answer]).event!.profile!;
    expect(
      [pe.target, pe.notification, pe.data],
      [
        0x40,
        false,
        [0x01, 0x02],
      ],
    );

    // Profile Specific Data (Table 28): a 4-byte length, LSB first.
    final psd = a.profileSpecificData(mb, 0x00, prof, [0x10, 0x20, 0x30]);
    expect(psd, [
      ...ciHeader(0x00, 0x2F, ma, mb),
      ...prof,
      3,
      0,
      0,
      0,
      0x10,
      0x20,
      0x30,
    ]);
    e = deliver(b, [psd]).event!;
    expect(
      [e.kind, e.profile!.data],
      [
        CiEventKind.profileSpecificData,
        [0x10, 0x20, 0x30],
      ],
    );

    // Set Profile Off (Table 25), then the Profile Disabled Report (Table 27).
    final off = a.setProfileOff(mb, 0x00, prof);
    expect(off, [...ciHeader(0x00, 0x23, ma, mb), ...prof, 0, 0]);
    expect(deliver(b, [off]).event!.profile!.on, false);
    final disabled = b.setProfileState(0x00, prof, enabled: false, channels: 1);
    expect(disabled.single.message, [
      ...ciHeader(0x00, 0x25, mb, broadcast),
      ...prof,
      0x01,
      0x00,
    ]);
    expect(
      deliver(a, [disabled.single.message]).event!.profile!.report,
      CiProfileReport.disabled,
    );

    // B removes the disabled Profile: Profile Removed Report only (Table 21).
    final removed = b.removeProfile(0x00, prof);
    expect(removed.single.message, [
      ...ciHeader(0x00, 0x27, mb, broadcast),
      ...prof,
    ]);
    expect(
      () => b.removeProfile(0x00, prof),
      throwsA(isA<RejectedException>()),
    );
    expect(
      () => b.setProfileState(0x00, prof, enabled: true, channels: 1),
      throwsA(isA<RejectedException>()),
    );

    // An enabled Profile: Added then Enabled; Disabled then Removed (M2-101 7.4, 7.5).
    final both = b.addProfile(
      0x00,
      prof,
      enabled: true,
      channels: 1,
      maxChannels: 1,
    );
    expect([for (final m in both) m.message[3]], [0x26, 0x24]);
    expect(
      [for (final m in b.removeProfile(0x00, prof)) m.message[3]],
      [0x25, 0x27],
    );
    a.free();
    b.free();
  });

  test('Process Inquiry between two devices', () {
    final a = device(11);
    final b = device(12);
    final ma = a.muid;
    final mb = b.muid;
    deliver(a, deliver(b, [a.discovery()]).replies);

    // Process Inquiry Capabilities (Table 40) and the automatic reply (Table 41: D0 set).
    final caps = a.piCapabilitiesInquiry(mb);
    expect(caps, ciHeader(0x7F, 0x40, ma, mb));
    final reply = deliver(b, [caps]).replies;
    expect(reply, [
      [...ciHeader(0x7F, 0x41, mb, ma), 0x01],
    ]);
    var e = deliver(a, reply).event!;
    expect(
      [e.kind, e.process?.midiMessageReport, e.profile],
      [CiEventKind.piCapabilities, true, null],
    );

    // Inquiry: MIDI Message Report (Table 43): full report of MTC Quarter Frame, Control Change
    // and Notes.
    const requested = CiReport(
      dataControl: 0x7F,
      system: 0x01,
      channel: 0x02,
      note: 0x01,
    );
    expect(a.reportOpen(mb), false);
    expect(
      () => a.messageReportInquiry(mb, const CiReport(dataControl: 0x80)),
      throwsA(isA<InvalidValueException>()),
    );
    final inquiry = a.messageReportInquiry(mb, requested);
    expect(inquiry, [
      ...ciHeader(0x7F, 0x42, ma, mb),
      0x7F,
      0x01,
      0x00,
      0x02,
      0x01,
    ]);
    expect(a.reportOpen(mb), true);
    final r = deliver(b, [inquiry]);
    expect([r.event?.kind, r.replies], [CiEventKind.messageReport, isEmpty]);
    final pi = r.event!.process!;
    expect([pi.deviceId, pi.group, pi.report], [0x7F, 0, requested]);

    // Reply to MIDI Message Report (Table 45): only the bits both requested and supported.
    final answer = b.messageReportReply(
      ma,
      pi.report,
      const CiReport(channel: 0x03),
      deviceId: pi.deviceId,
    );
    expect(answer, [...ciHeader(0x7F, 0x43, mb, ma), 0x00, 0x00, 0x02, 0x00]);
    e = deliver(a, [answer]).event!;
    expect(
      [e.kind, e.process?.report],
      [CiEventKind.messageReportReply, const CiReport(channel: 0x02)],
    );

    // End of MIDI Message Report (Table 46) closes the report.
    final end = b.messageReportEnd(ma);
    expect(end, ciHeader(0x7F, 0x44, mb, ma));
    expect(deliver(a, [end]).event?.kind, CiEventKind.messageReportEnd);
    expect(a.reportOpen(mb), false);

    // Abort: NAK (Table 15) for Sub-ID#2 0x42 with Status Code 0x20, or 0x40 for an error
    // (M2-101 9.8.1).
    final nak = b.abortReport(ma);
    expect([nak[1], nak[13], nak[14]], [0x7F, 0x42, 0x20]);
    expect(b.abortReport(ma, error: true)[14], 0x40);
    a.free();
    b.free();
  });

  test('Profile and Process Inquiry bad values', () {
    final a = device(11);
    expect(
      () => a.messageReportEnd(1, group: 16),
      throwsA(isA<InvalidValueException>()),
    );
    expect(
      () => a.profileInquiry(1, 0x10),
      throwsA(isA<InvalidValueException>()),
    );
    expect(
      () => a.setProfileOn(1, 0x00, [1, 2, 3]),
      throwsA(isA<InvalidValueException>()),
    );
    expect(
      () => a.addProfile(0x00, prof, channels: 0x4000),
      throwsA(isA<InvalidValueException>()),
    );
    expect(
      () => a.messageReportInquiry(1 << 28, const CiReport()),
      throwsA(isA<InvalidValueException>()),
    );
    a.free();
    expect(() => a.reportOpen(1), throwsA(isA<RejectedException>()));
    expect(() => a.addProfile(0, prof), throwsA(isA<RejectedException>()));
  });
}
