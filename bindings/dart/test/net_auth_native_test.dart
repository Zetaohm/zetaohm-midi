// The native copies of the credentials (REQ-DART-NET-003): the native form only.
@TestOn('vm')
library;

import 'dart:convert';
import 'dart:ffi';

import 'package:ffi/ffi.dart';
import 'package:test/test.dart';
import 'package:zetaohm_midi/src/native/net.dart' show credentialsAllocator;
import 'package:zetaohm_midi/zetaohm_midi.dart';

import 'net_auth_support.dart';

// An allocator that records each block and, at its free, whether every byte is zero.
class RecordingAllocator implements Allocator {
  final live = <int, int>{};
  var freed = 0;
  var notCleared = 0;

  @override
  Pointer<T> allocate<T extends NativeType>(int byteCount, {int? alignment}) {
    final p = calloc.allocate<T>(byteCount, alignment: alignment);
    live[p.address] = byteCount;
    return p;
  }

  @override
  void free(Pointer<NativeType> pointer) {
    final n = live.remove(pointer.address)!;
    if (pointer.cast<Uint8>().asTypedList(n).any((b) => b != 0)) {
      notCleared++;
    }
    freed++;
    calloc.free(pointer);
  }
}

void main() {
  test('the native copies of the credentials are cleared and freed', () {
    // REQ-DART-NET-003: each receive call copies the credentials into native memory, then clears
    // each block to zero and frees it.
    final rec = RecordingAllocator();
    credentialsAllocator = rec;
    addTearDown(() => credentialsAllocator = calloc);
    final host = NetHost.auth(
      'Host',
      'P1',
      methods: {NetAuthMethod.sharedSecret, NetAuthMethod.user},
      credentials: NetCredentials(
        sharedSecret: ascii.encode('5483'),
        users: [NetUser('Rosa', ascii.encode('RPBqBno'))],
      ),
    );
    final client = NetClient.auth(
      'zmidi Client',
      'client-1',
      methods: {NetAuthMethod.user},
      credentials: NetCredentials(
        users: [NetUser('Rosa', ascii.encode('RPBqBno'))],
      ),
    )..connect(0, hostAddr);
    final r = pump(host, client);
    expect(only(r.atHost, NetEventKind.established).length, 1);
    expect(rec.freed, greaterThan(0));
    expect(rec.live, isEmpty);
    expect(rec.notCleared, 0);
  });
}
