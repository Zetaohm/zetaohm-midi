// Network MIDI 2.0 authentication (REQ-DART-NET-002 to REQ-DART-NET-004): a Host and a Client in
// memory with the CryptoNonces, secrets and digests of the M2-124 6.9 and 6.10 examples, the same
// vectors as crates/zmidi-ffi/tests/c/net.c.
import 'dart:convert';

import 'package:test/test.dart';
import 'package:zetaohm_midi/zetaohm_midi.dart';

import 'net_auth_support.dart';

void main() {
  setUpAll(initWeb);

  test('each new session gets a new CryptoNonce', () {
    // M2-124 6.7: each new Session gets its own CryptoNonce, also for a Client that the Host knows
    // already. The Host takes new random bytes from its pool for each session.
    final host = NetHost.auth(
      'Host',
      'P1',
      methods: {NetAuthMethod.sharedSecret},
      credentials: NetCredentials(sharedSecret: ascii.encode('5483')),
    );
    final sent = <String>[];
    for (var i = 0; i < 2; i++) {
      final client = NetClient.auth(
        'zmidi Client',
        'client-1',
        methods: {NetAuthMethod.sharedSecret},
        credentials: NetCredentials(sharedSecret: ascii.encode('5483')),
      )..connect(0, hostAddr);
      final r = pump(host, client);
      expect(only(r.atHost, NetEventKind.established).length, 1);
      sent.addAll(nonces(r.hostPayloads));
      client.bye(0, hostAddr);
      pump(host, client);
      client.free();
    }
    expect(sent.length, 2);
    expect(sent[0], isNot(sent[1]));
  });

  test('shared secret method of example 6.9', () {
    const nonce = r'nUWrn*@#$hjfwnkL';
    const digest = [
      0x67, 0x6E, 0xBE, 0x82, 0x58, 0x7C, 0xEC, 0xA8, 0xF8, 0x2F, 0xC3, //
      0x33, 0xD7, 0x87, 0x95, 0x1E, 0xEC, 0x2B, 0x00, 0xAD, 0x31, 0x61,
      0x3C, 0xC4, 0xDB, 0x18, 0xCF, 0x27, 0x37, 0x3A, 0xFB, 0x82,
    ];
    final host = NetHost.auth(
      'Host',
      'P1',
      methods: {NetAuthMethod.sharedSecret},
      credentials: NetCredentials(
        sharedSecret: ascii.encode('5483'),
        random: randomFor(nonce),
      ),
    );
    final client = NetClient.auth(
      'zmidi Client',
      'client-1',
      methods: {NetAuthMethod.sharedSecret},
      credentials: NetCredentials(sharedSecret: ascii.encode('5483')),
    )..connect(0, hostAddr);
    final r = pump(
      host,
      client,
      hostExpect: ascii.encode(nonce),
      clientExpect: digest,
    );
    // Invitation Reply: Authentication Required with the CryptoNonce, state 0x00.
    expect(r.hostSent, isTrue);
    expect(nonces(r.hostPayloads), [nonce]);
    final req = only(r.atClient, NetEventKind.authRequested).single;
    expect([req.authMethod, req.value], [NetAuthMethod.sharedSecret, 0x00]);
    // Invitation with Authentication with the digest of the example, and the session.
    expect(r.clientSent, isTrue);
    final hostUp = only(r.atHost, NetEventKind.established).single;
    final clientUp = only(r.atClient, NetEventKind.established).single;
    expect(hostUp.authMethod, NetAuthMethod.sharedSecret);
    expect(clientUp.authMethod, NetAuthMethod.sharedSecret);
    expect(only(r.atHost, NetEventKind.authFailed), isEmpty);
    host.free();
    client.free();
  });

  test('user method of example 6.10', () {
    const nonce = 'XI|~=NNRVaD;XCPL';
    const digest = [
      0x3A, 0x2D, 0x5E, 0xBE, 0xEF, 0x92, 0xE8, 0x46, 0x35, 0x35, 0xC2, //
      0x7F, 0xB4, 0xB7, 0xB3, 0xE2, 0xD1, 0xAC, 0xF5, 0x7A, 0x84, 0x4C,
      0x6D, 0x08, 0x08, 0xE0, 0x41, 0xC1, 0x02, 0xB7, 0xFF, 0x1A,
    ];
    final host = NetHost.auth(
      'Host',
      'P1',
      methods: {NetAuthMethod.user},
      credentials: NetCredentials(
        users: [
          NetUser('Ana', ascii.encode('other')),
          NetUser('Rosa', ascii.encode('RPBqBno')),
        ],
        random: randomFor(nonce),
      ),
    );
    final client = NetClient.auth(
      'zmidi Client',
      'client-1',
      methods: {NetAuthMethod.sharedSecret, NetAuthMethod.user},
      credentials: NetCredentials(
        users: [NetUser('Rosa', ascii.encode('RPBqBno'))],
      ),
    )..connect(0, hostAddr);
    final r = pump(
      host,
      client,
      hostExpect: ascii.encode(nonce),
      clientExpect: digest,
    );
    expect(r.hostSent, isTrue);
    expect(
      only(r.atClient, NetEventKind.authRequested).single.authMethod,
      NetAuthMethod.user,
    );
    expect(r.clientSent, isTrue);
    expect(
      only(r.atHost, NetEventKind.established).single.authMethod,
      NetAuthMethod.user,
    );
    expect(
      only(r.atClient, NetEventKind.established).single.authMethod,
      NetAuthMethod.user,
    );
  });

  test('wrong password fails with Bye 0x43', () {
    failure('Rosa', 'wrong', notFound: false, state: 0x01, bye: 0x43);
  });

  test('unknown Username fails with Bye 0x44', () {
    failure('Nobody', 'RPBqBno', notFound: true, state: 0x02, bye: 0x44);
  });

  test('net authentication bad values', () {
    expect(
      () => NetHost.auth(
        'Host',
        'P1',
        methods: {},
        credentials: NetCredentials(),
      ),
      throwsA(isA<InvalidValueException>()),
    );
    expect(
      () => NetClient.auth(
        '',
        'c',
        methods: {NetAuthMethod.sharedSecret},
        credentials: NetCredentials(),
      ),
      throwsA(isA<InvalidValueException>()),
    );
    // A Host with no random bytes can not ask for authentication: CredentialsError code 0.
    final host = NetHost.auth(
      'Host',
      'P1',
      methods: {NetAuthMethod.sharedSecret, NetAuthMethod.user},
      credentials: NetCredentials(
        sharedSecret: ascii.encode('5483'),
        random: (_) => const [],
      ),
    );
    final client = NetClient.auth(
      'zmidi Client',
      'client-1',
      methods: {NetAuthMethod.sharedSecret},
      credentials: NetCredentials(sharedSecret: ascii.encode('5483')),
    )..connect(0, hostAddr);
    final r = pump(host, client);
    final err = only(r.atHost, NetEventKind.credentialsError);
    expect(err, isNotEmpty);
    expect(err.first.code, 0);
    expect(only(r.atHost, NetEventKind.established), isEmpty);
    // A secret byte over 255.
    host.credentials = NetCredentials(sharedSecret: [256]);
    expect(
      () => host.receive(0, clientAddr, [0]),
      throwsA(isA<InvalidValueException>()),
    );
    expect(host.credentials?.sharedSecret, [256]);
    host.credentials = null;
    expect(host.receive(0, clientAddr, [0]).events, isA<List<NetEvent>>());
    host.free();
    expect(
      () => host.receive(0, clientAddr, [0]),
      throwsA(isA<RejectedException>()),
    );
  });
}
