// Network MIDI 2.0 (REQ-DART-NET-001): a Host and a Client in memory, M2-124.
import 'package:test/test.dart';
import 'package:zetaohm_midi/zetaohm_midi.dart';

final hostAddr = NetAddress.parse('192.168.1.10', 5673);
final clientAddr = NetAddress.parse('192.168.1.20', 50000);

({List<NetEvent> atHost, List<NetEvent> atClient}) pump(
  NetHost host,
  NetClient client,
  int start,
) {
  final atHost = <NetEvent>[];
  final atClient = <NetEvent>[];
  var now = start;
  for (var i = 0; i < 20; i++) {
    var moved = false;
    for (final p in client.writePending(now)) {
      atHost.addAll(host.receive(now, clientAddr, p.payload).events);
      moved = true;
    }
    for (final p in host.writePending(now)) {
      expect(p.to, clientAddr);
      atClient.addAll(client.receive(now, hostAddr, p.payload).events);
      moved = true;
    }
    if (!moved) {
      break;
    }
    now += 10;
  }
  return (atHost: atHost, atClient: atClient);
}

List<NetEventKind> kinds(List<NetEvent> events) => [
  for (final e in events) e.kind,
];

void main() {
  setUpAll(initWeb);

  test('session, UMP Data, Ping and Bye', () {
    final host = NetHost('zmidi Host', 'host-1');
    final client = NetClient('zmidi Client', 'client-1')..connect(0, hostAddr);
    var r = pump(host, client, 0);
    expect(kinds(r.atHost), contains(NetEventKind.established));
    expect(kinds(r.atClient), contains(NetEventKind.established));

    client.sendUmp(hostAddr, [0x40903C00, 0xC0000000]);
    r = pump(host, client, 100);
    expect(
      [
        for (final e in r.atHost)
          if (e.kind == NetEventKind.data) e.umps,
      ],
      [
        [
          [0x40903C00, 0xC0000000],
        ],
      ],
    );

    final id = client.ping(200, hostAddr);
    r = pump(host, client, 200);
    expect(
      [
        for (final e in r.atClient)
          if (e.kind == NetEventKind.pingReply) e.code,
      ],
      [id],
    );

    final bye = client.bye(300, hostAddr);
    r = pump(host, client, 300);
    expect(kinds(r.atHost), contains(NetEventKind.closed));
    expect(kinds([...bye, ...r.atClient]), contains(NetEventKind.closed));
  });

  test('net bad values', () {
    expect(() => NetHost('', 'x'), throwsA(isA<InvalidValueException>()));
    expect(
      () => NetAddress.parse('not an ip', 1),
      throwsA(isA<InvalidValueException>()),
    );
    final host = NetHost('h', 'p');
    expect(
      () => host.sendUmp(clientAddr, [0x20903C64]),
      throwsA(isA<RejectedException>()),
    );
    expect(host.nextDeadline(), isNull);
  });

  test('IPv6 addresses', () {
    final host = NetHost('h', 'p');
    final h6 = NetAddress.parse('fe80::1', 5673);
    final client = NetClient('c', 'q')..connect(0, h6);
    final p = client.writePending(0).single;
    expect(p.to, h6);
    final peer = NetAddress.parse('2001:db8::1:0:0:2', 4000);
    expect(host.receive(0, peer, p.payload).events.first.peer, peer);
  });
}
