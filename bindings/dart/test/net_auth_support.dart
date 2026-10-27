// Shared values and steps of the Network MIDI 2.0 authentication tests (M2-124 6.7 to 6.10).
import 'dart:convert';

import 'package:test/test.dart';
import 'package:zetaohm_midi/zetaohm_midi.dart';

final hostAddr = NetAddress.parse('192.168.1.10', 5673);
final clientAddr = NetAddress.parse('192.168.1.20', 50000);

// The random bytes that give a CryptoNonce: CryptoNonce byte = 32 + (random byte mod 95).
List<int> Function(int) randomFor(String nonce) {
  var first = true;
  return (count) {
    final out = first ? [for (final c in ascii.encode(nonce)) c - 32] : <int>[];
    first = false;
    // The bytes after the first CryptoNonce do not matter.
    return [...out, for (var i = out.length; i < count; i++) i * 7 + 3];
  };
}

bool contains(List<int> data, List<int> part) {
  for (var i = 0; i + part.length <= data.length; i++) {
    var all = true;
    for (var k = 0; k < part.length && all; k++) {
      all = data[i + k] == part[k];
    }
    if (all) {
      return true;
    }
  }
  return false;
}

typedef Run = ({
  List<NetEvent> atHost,
  List<NetEvent> atClient,
  bool hostSent,
  bool clientSent,
  List<List<int>> hostPayloads,
});

// The CryptoNonce of each Invitation Reply: Authentication Required (0x12) or User Authentication
// Required (0x13) in [payloads]: the 16 bytes after the 4-byte command header (M2-124 Tables 14,
// 16). A payload is "MIDI" and then commands of 4 + 4 * pl bytes.
List<String> nonces(List<List<int>> payloads) => [
  for (final p in payloads)
    for (var i = 4; i + 4 <= p.length; i += 4 + 4 * p[i + 1])
      if (p[i] == 0x12 || p[i] == 0x13) ascii.decode(p.sublist(i + 4, i + 20)),
];

// Moves each pending payload to the other side until none is left. When nothing moves and a side
// has a deadline within 5 s, the time goes to it and both sides poll (failure delay, repeats).
Run pump(
  NetHost host,
  NetClient client, {
  List<int> hostExpect = const [],
  List<int> clientExpect = const [],
}) {
  final atHost = <NetEvent>[];
  final atClient = <NetEvent>[];
  var hostSent = false;
  var clientSent = false;
  final hostPayloads = <List<int>>[];
  var now = 0;
  for (var round = 0; round < 60; round++) {
    var moved = false;
    for (final p in client.writePending(now)) {
      clientSent |=
          clientExpect.isNotEmpty && contains(p.payload, clientExpect);
      atHost.addAll(host.receive(now, clientAddr, p.payload).events);
      moved = true;
    }
    for (final p in host.writePending(now)) {
      expect(p.to, clientAddr);
      hostSent |= hostExpect.isNotEmpty && contains(p.payload, hostExpect);
      hostPayloads.add(p.payload);
      atClient.addAll(client.receive(now, hostAddr, p.payload).events);
      moved = true;
    }
    if (moved) {
      now += 10;
      continue;
    }
    final d = [
      host.nextDeadline(),
      client.nextDeadline(),
    ].whereType<int>().toList();
    if (d.isEmpty) {
      break;
    }
    final next = d.reduce((a, b) => a < b ? a : b);
    if (next > now + 5000) {
      break;
    }
    if (next > now) {
      now = next;
    }
    atHost.addAll(host.poll(now));
    atClient.addAll(client.poll(now));
  }
  return (
    atHost: atHost,
    atClient: atClient,
    hostSent: hostSent,
    clientSent: clientSent,
    hostPayloads: hostPayloads,
  );
}

// The user method with a Client user [name] and [password] that the Host table (Rosa, RPBqBno)
// refuses: up to the failure limit (5), then Bye [bye] and no session.
void failure(
  String name,
  String password, {
  required bool notFound,
  required int state,
  required int bye,
}) {
  final host = NetHost.auth(
    'Host',
    'P1',
    methods: {NetAuthMethod.user},
    credentials: NetCredentials(
      users: [NetUser('Rosa', ascii.encode('RPBqBno'))],
    ),
  );
  final client = NetClient.auth(
    'zmidi Client',
    'client-1',
    methods: {NetAuthMethod.user},
    credentials: NetCredentials(users: [NetUser(name, ascii.encode(password))]),
  )..connect(0, hostAddr);
  final r = pump(host, client);
  final failed = only(r.atHost, NetEventKind.authFailed);
  expect(failed.length, 5);
  expect(
    [failed.last.authMethod, failed.last.usernameNotFound, failed.last.value],
    [NetAuthMethod.user, notFound, 5],
  );
  final req = only(r.atClient, NetEventKind.authRequested);
  expect(req.length, greaterThanOrEqualTo(2));
  expect(req.last.value, state);
  // One Invitation keeps one CryptoNonce for its retries (DEC-NET-001 of zmidi-net, AMB-M2-124-44).
  final sent = nonces(r.hostPayloads);
  expect(sent.length, req.length);
  expect(sent.toSet().length, 1);
  expect(only(r.atHost, NetEventKind.established), isEmpty);
  expect(only(r.atClient, NetEventKind.established), isEmpty);
  final closed = only(r.atClient, NetEventKind.closed).single;
  expect([closed.code, closed.value], [0, bye]);
}

List<NetEvent> only(List<NetEvent> events, NetEventKind kind) => [
  for (final e in events)
    if (e.kind == kind) e,
];
