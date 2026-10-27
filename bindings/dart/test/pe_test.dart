// The Property Exchange engine (REQ-DART-PE-*): two devices, A (Initiator) and B (Responder), each
// with a CiDevice and a PeEngine. M2-103: Get with many Chunks (5.3), ResourceList (9), 404 (8.6.2),
// Set with many Chunks, Subscription start, full and end (11), timeouts (5.3, 12), MIDI-CI ACK 0x10
// and NAK 0x20; the same cases as tests/c/pe.c of zmidi-ffi.
import 'dart:convert';
import 'dart:typed_data';

import 'package:test/test.dart';
import 'package:zetaohm_midi/zetaohm_midi.dart';

const now = 1000000;

CiDevice device(int seed) => CiDevice(
  manufacturer: [0x00, 0x21, 0x7F],
  family: 1,
  model: 2,
  revision: [0, 0, 1, 0],
  seed: seed,
);

/// Gives each message to [dev] and then to [eng]; gives the automatic replies and the engine events.
({List<Uint8List> replies, List<PeEvent> events}) deliver(
  CiDevice dev,
  PeEngine eng,
  List<Uint8List> messages,
) {
  final replies = <Uint8List>[];
  final events = <PeEvent>[];
  for (final m in messages) {
    replies.addAll(dev.receive(0, m, now).replies.map((r) => r.message));
    final ev = eng.receive(dev, now);
    if (ev != null) {
      events.add(ev);
    }
  }
  return (replies: replies, events: events);
}

List<Uint8List> pending(CiDevice dev, PeEngine eng) {
  final out = <Uint8List>[];
  for (
    var m = eng.writePending(dev, now);
    m != null;
    m = eng.writePending(dev, now)
  ) {
    out.add(m);
  }
  return out;
}

/// Sends [data] as the Chunks of the transaction.
List<Uint8List> sendAll(
  CiDevice dev,
  PeEngine eng,
  int peer,
  int rid,
  PeRole role, [
  List<int> data = const [],
]) {
  final chunks = <Uint8List>[];
  var sent = 0;
  for (;;) {
    final room = eng.dataRoom(dev, peer, rid, role);
    final end = (sent + room).clamp(0, data.length);
    final part = data.sublist(sent, end);
    sent = end;
    chunks.add(
      eng.send(dev, peer, rid, role, part, sent == data.length, nowUs: now),
    );
    if (sent == data.length) {
      return chunks;
    }
  }
}

final table = [
  const PeResource('DeviceInfo'),
  const PeResource('X-Big', canSet: PeCanSet.full),
  const PeResource('X-Sub', canSubscribe: true),
  PeResource(
    'X-Mc',
    encodings: const [PeEncoding.ascii, PeEncoding.mcoded7],
    schema: ascii.encode('{"type":"object"}'),
  ),
];

/// A and B after Discovery and PE Capabilities in both directions.
(CiDevice, PeEngine, CiDevice, PeEngine) pair() {
  final a = device(1), b = device(2);
  final pa = PeEngine(), pb = PeEngine(table);
  for (final (x, y, px, py) in [(a, b, pa, pb), (b, a, pb, pa)]) {
    deliver(x, px, deliver(y, py, [x.discovery()]).replies);
    expect(
      deliver(
        x,
        px,
        deliver(y, py, [x.peCapabilitiesInquiry(y.muid)]).replies,
      ).events,
      isEmpty,
    );
  }
  return (a, pa, b, pb);
}

void main() {
  setUpAll(initWeb);

  test('Resource checks', () {
    // REQ-DART-PE-001.
    for (final bad in ['Bad-Name', 'X-', '', 'A' * 37]) {
      expect(
        () => PeEngine([PeResource(bad)]),
        throwsA(isA<InvalidValueException>()),
      );
    }
    expect(
      () => PeEngine([const PeResource('DeviceInfo', encodings: [])]),
      throwsA(isA<InvalidValueException>()),
    );
    expect(
      () => PeEngine([const PeResource('DeviceInfo', mediaType: 'json')]),
      throwsA(isA<InvalidValueException>()),
    );
    expect(
      () =>
          PeEngine([PeResource('DeviceInfo', schema: List.filled(600, 0x78))]),
      throwsA(isA<InvalidValueException>()),
    );
    expect(
      () => PeEngine(List.filled(17, const PeResource('DeviceInfo'))),
      throwsA(isA<InvalidValueException>()),
    );
    PeEngine([
      const PeResource('DeviceInfo', mediaType: 'application/octet-stream'),
    ]).free();
    final a = device(1);
    expect(
      () => PeEngine().request(a, 0x10000000, PeTxnKind.get, 'DeviceInfo'),
      throwsA(isA<InvalidValueException>()),
    );
    // Before Discovery and PE Capabilities the device refuses (M2-101 8.4).
    expect(
      () => PeEngine().request(a, 0x123, PeTxnKind.get, 'DeviceInfo'),
      throwsA(isA<RejectedException>()),
    );
  });

  test('ResourceList and a Get in many Chunks', () {
    // REQ-DART-PE-002, -003, -005.
    final (a, pa, b, pb) = pair();
    expect(pa.receive(a, now), isNull);

    final r = pa.requestResourceList(a, b.muid, nowUs: now);
    var events = deliver(b, pb, [r.message]).events;
    expect(events, hasLength(1));
    final ev = events.single;
    expect(ev.kind, PeEventKind.message);
    expect(ev.role, PeRole.responder);
    expect(ev.txn, PeTxnKind.get);
    expect(ev.end, PeChunkEnd.complete);
    expect(ev.resource, 'ResourceList');
    expect(ev.resourceIndex, isNull);
    expect(ev.requestId, r.requestId);
    expect(ev.muid, a.muid);
    expect(utf8.decode(ev.header), '{"resource":"ResourceList"}');
    final listing = pb.resourceList();
    expect(
      utf8.decode(listing),
      contains('"resource":"X-Sub","canSubscribe":true'),
    );
    expect(utf8.decode(listing), contains('"schema":{"type":"object"}'));
    pb.reply(b, a.muid, r.requestId, length: listing.length);
    events = deliver(
      a,
      pa,
      sendAll(b, pb, a.muid, r.requestId, PeRole.responder, listing),
    ).events;
    expect(events.first.status, 200);
    expect(events.first.role, PeRole.initiator);
    expect(events.last.end, PeChunkEnd.complete);
    expect(events.expand((e) => e.data).toList(), listing);

    final g = pa.request(a, b.muid, PeTxnKind.get, 'DeviceInfo');
    expect(g.resourceListMissing, isFalse);
    events = deliver(
      b,
      pb,
      sendAll(a, pa, b.muid, g.requestId, PeRole.initiator),
    ).events;
    expect(events.first.resource, 'DeviceInfo');
    expect(events.first.resourceIndex, 0);
    final body = ascii.encode(
      '"${List.generate(1198, (i) => String.fromCharCode(97 + i % 26)).join()}"',
    );
    pb.reply(b, a.muid, g.requestId, length: body.length);
    expect(
      () => pb.reply(b, a.muid, g.requestId, length: body.length),
      throwsA(isA<RejectedException>()),
    );
    expect(
      () => pb.send(
        b,
        a.muid,
        g.requestId,
        PeRole.responder,
        body,
        false,
        nowUs: now,
      ),
      throwsA(isA<InvalidValueException>()),
    );
    final chunks = sendAll(b, pb, a.muid, g.requestId, PeRole.responder, body);
    expect(chunks, hasLength(3));
    events = deliver(a, pa, chunks).events;
    expect(events.map((e) => e.end), [
      PeChunkEnd.more,
      PeChunkEnd.more,
      PeChunkEnd.complete,
    ]);
    expect(events[0].header, isNotEmpty);
    expect(events[1].header, isEmpty);
    expect(events.expand((e) => e.data).toList(), body);
    expect(pa.receive(a, now), isNull);
    expect(
      () => pb.send(
        b,
        a.muid,
        g.requestId,
        PeRole.responder,
        const [],
        true,
        nowUs: now,
      ),
      throwsA(isA<RejectedException>()),
    );
    pa.free();
    pb.free();
  });

  test('404, a Set in two Chunks and a refusal', () {
    // REQ-DART-PE-003, -005.
    final (a, pa, b, pb) = pair();
    final n = pa.request(a, b.muid, PeTxnKind.get, 'X-None');
    expect(n.resourceListMissing, isTrue);
    expect(
      deliver(
        b,
        pb,
        sendAll(a, pa, b.muid, n.requestId, PeRole.initiator),
      ).events,
      isEmpty,
    );
    var events = deliver(a, pa, pending(b, pb)).events;
    expect(events.first.status, 404);
    expect(events.first.end, PeChunkEnd.complete);

    // REQ-DART-CI-007: a request between CiDevice.receive and PeEngine.receive of the last Chunk
    // of a reply gets another Request ID; the engine then closes only the old request.
    final id1 = pa.request(a, b.muid, PeTxnKind.get, 'X-None').requestId;
    deliver(b, pb, sendAll(a, pa, b.muid, id1, PeRole.initiator));
    final [reply] = pending(b, pb);
    a.receive(0, reply, now);
    final id2 = pa.request(a, b.muid, PeTxnKind.get, 'X-None').requestId;
    expect(id2, isNot(id1));
    final ev = pa.receive(a, now);
    expect([ev?.requestId, ev?.status], [id1, 404]);
    deliver(b, pb, sendAll(a, pa, b.muid, id2, PeRole.initiator));
    events = deliver(a, pa, pending(b, pb)).events;
    expect([events.first.requestId, events.first.status], [id2, 404]);

    expect(
      () => pa.request(
        a,
        b.muid,
        PeTxnKind.set,
        'DeviceInfo',
        length: 2,
        caps: table[0],
      ),
      throwsA(
        isA<RejectedException>().having(
          (e) => e.toString(),
          'message',
          contains('status 405'),
        ),
      ),
    );
    expect(
      () => pa.request(a, b.muid, PeTxnKind.subscription, 'DeviceInfo'),
      throwsA(isA<InvalidValueException>()),
    );
    expect(
      () => pa.request(a, b.muid, PeTxnKind.get, 'DeviceInfo', offset: 0),
      throwsA(isA<InvalidValueException>()),
    );

    final data = ascii.encode('x' * 700);
    final s = pa.request(
      a,
      b.muid,
      PeTxnKind.set,
      'X-Big',
      length: data.length,
      caps: table[1],
    );
    final chunks = sendAll(a, pa, b.muid, s.requestId, PeRole.initiator, data);
    expect(chunks, hasLength(2));
    events = deliver(b, pb, chunks).events;
    expect(events.first.txn, PeTxnKind.set);
    expect(events.first.resourceIndex, 1);
    expect(events.last.end, PeChunkEnd.complete);
    expect(events.expand((e) => e.data).toList(), data);
    pb.reply(b, a.muid, s.requestId);
    events = deliver(
      a,
      pa,
      sendAll(b, pb, a.muid, s.requestId, PeRole.responder),
    ).events;
    expect(events.first.txn, PeTxnKind.set);
    expect(events.first.status, 200);
  });

  test('Subscription start, full, refresh and end', () {
    // REQ-DART-PE-004, -005, -006.
    final (a, pa, b, pb) = pair();
    final st = pa.subscribe(a, b.muid, 'X-Sub', caps: table[2]);
    var events = deliver(
      b,
      pb,
      sendAll(a, pa, b.muid, st.requestId, PeRole.initiator),
    ).events;
    final index = events
        .firstWhere((e) => e.kind == PeEventKind.subscriptionStarted)
        .subscription!;
    final sub = pb.subscription(index)!;
    expect(sub.muid, a.muid);
    expect(sub.role, PeRole.responder);
    expect(sub.resource, 'X-Sub');
    events = deliver(a, pa, pending(b, pb)).events;
    expect(events, hasLength(1));
    final started = events.single;
    expect(started.txn, PeTxnKind.subscription);
    expect(started.status, 200);
    expect(started.startError, isNull);
    expect(started.subscribeId, sub.subscribeId);
    final mine = pa.subscription(started.started!)!;
    expect(mine.role, PeRole.initiator);
    expect(mine.muid, b.muid);
    expect(mine.resId, isNull);
    expect(pa.subscription(3), isNull);

    expect(
      () => pb.update(b, index, PeCommand.start),
      throwsA(isA<InvalidValueException>()),
    );
    final rid = pb.update(b, index, PeCommand.full, length: 20);
    final full = ascii.encode('0123456789abcdefghij');
    events = deliver(
      a,
      pa,
      sendAll(b, pb, a.muid, rid, PeRole.initiator, full),
    ).events;
    expect(events.first.command, PeCommand.full);
    expect(events.first.role, PeRole.responder);
    expect(events.first.subscription, 0);
    expect(events.first.data, full);
    events = deliver(b, pb, pending(a, pa)).events;
    expect(events.first.status, 200);
    expect(events.first.role, PeRole.initiator);

    expect(() => pa.refresh(99), throwsA(isA<RejectedException>()));
    pa.refresh(0);
    events = deliver(b, pb, pending(a, pa)).events;
    expect(events.first.txn, PeTxnKind.get);
    expect(events.first.resource, 'X-Sub');
    final g = events.first.requestId!;
    pb.reply(b, a.muid, g, length: 2);
    events = deliver(
      a,
      pa,
      sendAll(b, pb, a.muid, g, PeRole.responder, ascii.encode('{}')),
    ).events;
    expect(events.first.subscription, 0);

    final end = pa.unsubscribe(a, 0);
    expect(pa.subscription(0), isNull);
    events = deliver(
      b,
      pb,
      sendAll(a, pa, b.muid, end, PeRole.initiator),
    ).events;
    expect(events.last.kind, PeEventKind.subscriptionEnded);
    expect(events.last.muid, a.muid);
    expect(events.last.resource, 'X-Sub');
    expect(pb.subscription(index), isNull);
    deliver(a, pa, pending(b, pb));
    expect(pa.takeEnded(), isNull);
  });

  test('wait, abort, timeouts and a late reply', () {
    // REQ-DART-PE-003, -007.
    final (a, pa, b, pb) = pair();
    var g = pa.request(a, b.muid, PeTxnKind.get, 'DeviceInfo');
    deliver(b, pb, sendAll(a, pa, b.muid, g.requestId, PeRole.initiator));
    for (final bad in [0, 12701]) {
      expect(
        () => pb.wait(a.muid, g.requestId, PeRole.responder, bad),
        throwsA(isA<InvalidValueException>()),
      );
    }
    pb.wait(a.muid, g.requestId, PeRole.responder, 500);
    final ack = pending(b, pb).single;
    expect(ack[3], 0x7D);
    deliver(a, pa, [ack]);
    expect(
      () => pb.corrupt(a.muid, g.requestId, PeRole.responder),
      throwsA(isA<RejectedException>()),
    );
    pb.abort(b, a.muid, g.requestId, PeRole.responder);
    expect(
      () => pb.abort(b, a.muid, g.requestId, PeRole.responder),
      throwsA(isA<RejectedException>()),
    );
    final nak = pending(b, pb).single;
    expect(nak[3], 0x7F);
    var events = deliver(a, pa, [nak]).events;
    expect(events.first.kind, PeEventKind.closed);
    expect(events.first.reason, PeCloseReason.terminated);
    expect(events.first.requestId, g.requestId);

    g = pa.request(a, b.muid, PeTxnKind.get, 'DeviceInfo');
    sendAll(a, pa, b.muid, g.requestId, PeRole.initiator);
    expect(pa.poll(a, now + 1000000), isNull);
    final closed = pa.poll(a, now + 3002000)!;
    expect(closed.kind, PeEventKind.closed);
    expect(closed.reason, PeCloseReason.timeout);
    expect(closed.requestId, g.requestId);
    expect(pa.poll(a, now + 3002000), isNull);
    final timeout = pending(a, pa).single;
    // NAK of a Get, Status Code 0x42 (M2-101 Table 16).
    expect([timeout[3], timeout[13], timeout[14]], [0x7F, 0x34, 0x42]);

    g = pa.request(a, b.muid, PeTxnKind.get, 'DeviceInfo');
    deliver(b, pb, sendAll(a, pa, b.muid, g.requestId, PeRole.initiator));
    expect(
      () => pb.reply(b, a.muid, g.requestId, length: 2, nowUs: now + 4000000),
      throwsA(
        isA<RejectedException>().having(
          (e) => e.toString(),
          'message',
          contains('deadline'),
        ),
      ),
    );
    expect(
      () => pb.replyStatus(b, a.muid, g.requestId, 404),
      throwsA(isA<RejectedException>()),
    );

    g = pa.request(a, b.muid, PeTxnKind.get, 'DeviceInfo');
    deliver(b, pb, sendAll(a, pa, b.muid, g.requestId, PeRole.initiator));
    pb.replyStatus(b, a.muid, g.requestId, 413);
    events = deliver(a, pa, pending(b, pb)).events;
    expect(events.first.status, 413);
    // A freed engine refuses.
    pb.free();
    expect(() => pb.resourceList(), throwsA(isA<RejectedException>()));
  });
}
