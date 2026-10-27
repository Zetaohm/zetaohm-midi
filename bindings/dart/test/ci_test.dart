// MIDI-CI and one-chunk Property Exchange (REQ-DART-CI-001, REQ-DART-CI-002): two devices, M2-101
// and M2-103.
import 'dart:convert';
import 'dart:typed_data';

import 'package:test/test.dart';
import 'package:zetaohm_midi/zetaohm_midi.dart';

final list = ascii.encode(
  '[{"resource":"DeviceInfo"},{"resource":"X-Volume","canSet":"full"}]',
);

CiDevice device(int seed) => CiDevice(
  manufacturer: [0x00, 0x21, 0x7F],
  family: 1,
  model: 2,
  revision: [0, 0, 1, 0],
  productInstanceId: 'zmidi-dart',
  resourceList: list,
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

  test('Discovery, PE Get and Set between two devices', () {
    final a = device(1);
    final b = device(2);
    expect(a.muid, isNot(b.muid));
    expect(
      () => a.peGet(b.muid, ascii.encode('{}')),
      throwsA(isA<RejectedException>()),
    );
    var ev = deliver(a, deliver(b, [a.discovery()]).replies).event;
    expect([ev?.kind, ev?.muid], [CiEventKind.discovered, b.muid]);
    expect(
      deliver(
        a,
        deliver(b, [a.endpointInfoInquiry(b.muid)]).replies,
      ).event?.kind,
      CiEventKind.endpointInfo,
    );
    expect(
      deliver(
        a,
        deliver(b, [a.peCapabilitiesInquiry(b.muid)]).replies,
      ).event?.kind,
      CiEventKind.peCapabilities,
    );

    final get = a.peGet(b.muid, ascii.encode('{"resource":"DeviceInfo"}'));
    ev = deliver(b, get.chunks).event;
    expect(
      [ev?.kind, ev?.resource, ev?.requestId],
      [CiEventKind.peGet, 'DeviceInfo', get.requestId],
    );
    final info = ascii.encode('{"manufacturerId":[0,33,127]}');
    ev = deliver(a, b.peReply(200, info)).event;
    expect(ev?.kind, CiEventKind.peGetReply);
    expect(ev?.header, ascii.encode('{"status":200}'));
    expect(ev?.data, info);
    expect(() => b.peReply(200), throwsA(isA<RejectedException>()));

    deliver(
      b,
      a.peGet(b.muid, ascii.encode('{"resource":"ResourceList"}')).chunks,
    );
    expect(deliver(a, b.peReply(404)).event?.data, list);

    deliver(b, a.peGet(b.muid, ascii.encode('{"resource":"X-None"}')).chunks);
    expect(
      deliver(a, b.peReply(404)).event?.header,
      ascii.encode('{"status":404}'),
    );

    ev = deliver(
      b,
      a
          .peSet(
            b.muid,
            ascii.encode('{"resource":"X-Volume"}'),
            ascii.encode('{"value":100}'),
          )
          .chunks,
    ).event;
    expect(
      [ev?.kind, ev?.data],
      [CiEventKind.peSet, ascii.encode('{"value":100}')],
    );
    expect(deliver(a, b.peReply(200)).event?.kind, CiEventKind.peSetReply);
    a.free();
    b.free();
  });

  test('bad CI configuration', () {
    expect(
      () => CiDevice(manufacturer: [0x00, 0x21]),
      throwsA(isA<InvalidValueException>()),
    );
    expect(
      () => CiDevice(manufacturer: [0x7D], maxSysexSize: 100),
      throwsA(isA<InvalidValueException>()),
    );
  });

  test('the Request IDs of peGet and peSet are freed', () {
    // REQ-DART-CI-007: the last Chunk of a reply frees the ID at the next receive; peEnd frees
    // the ID of a request with no reply. The device has 4 Request IDs.
    final a = device(1);
    final b = device(2);
    deliver(a, deliver(b, [a.discovery()]).replies);
    deliver(a, deliver(b, [a.peCapabilitiesInquiry(b.muid)]).replies);
    final get = ascii.encode('{"resource":"DeviceInfo"}');
    final empty = ascii.encode('{}');
    var rid = -1;
    for (var i = 0; i < 6; i++) {
      final r = a.peGet(b.muid, get);
      rid = r.requestId;
      deliver(b, r.chunks);
      final ev = deliver(a, b.peReply(200, empty)).event;
      expect([ev?.kind, ev?.requestId], [CiEventKind.peGetReply, rid]);
    }
    // Until the next receive, a new request gets another ID.
    final second = a.peGet(b.muid, get);
    expect(second.requestId, isNot(rid));
    deliver(b, second.chunks);
    deliver(a, b.peReply(200, empty));

    a.peEnd(b.muid, second.requestId);
    final ids = [for (var i = 0; i < 4; i++) a.peGet(b.muid, empty).requestId];
    expect(ids.first, second.requestId);
    deliver(a, deliver(b, [a.endpointInfoInquiry(b.muid)]).replies);
    expect(() => a.peGet(b.muid, empty), throwsA(isA<RejectedException>()));
    a.peEnd(b.muid, ids[2]);
    expect(a.peGet(b.muid, empty).requestId, ids[2]);
    expect(() => a.peEnd(b.muid, 0x80), throwsA(isA<InvalidValueException>()));
    expect(() => a.peEnd(0x10000000, 0), throwsA(isA<InvalidValueException>()));
    for (final id in ids) {
      a.peEnd(b.muid, id);
    }

    // A reply of 2 Chunks (M2-101 Table 29): only the last Chunk frees the ID.
    List<int> u14(int v) => [v & 0x7F, v >> 7];
    List<int> muidBytes(int m) => [
      for (final s in [0, 7, 14, 21]) (m >> s) & 0x7F,
    ];
    Uint8List chunk(
      int id,
      String header,
      int total,
      int number,
      String data,
    ) => Uint8List.fromList([
      0x7E,
      0x7F,
      0x0D,
      0x35,
      0x02,
      ...muidBytes(b.muid),
      ...muidBytes(a.muid),
      id,
      ...u14(header.length),
      ...ascii.encode(header),
      ...u14(total),
      ...u14(number),
      ...u14(data.length),
      ...ascii.encode(data),
    ]);
    final first = a.peGet(b.muid, empty).requestId;
    var ev = deliver(a, [chunk(first, '{"status":200}', 2, 1, '{"a":')]).event;
    expect([ev?.chunkNumber, ev?.chunkTotal], [1, 2]);
    ev = deliver(a, [chunk(first, '', 2, 2, '1}')]).event;
    expect([ev?.chunkNumber, ev?.requestId], [2, first]);
    expect(a.peGet(b.muid, empty).requestId, isNot(first));
    a.free();
    b.free();
  });
}
