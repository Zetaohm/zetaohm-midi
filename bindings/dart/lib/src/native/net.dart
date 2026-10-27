/// The native form of the Network MIDI 2.0 Host and Client: zmidi-net through dart:ffi and the C
/// ABI. The values are checked by `net.dart`.
library;

import 'dart:convert';
import 'dart:ffi';
import 'dart:io';
import 'dart:typed_data';

import 'package:ffi/ffi.dart';

import '../bindings.g.dart';
import '../native.dart';
import '../net.dart';

/// Random bytes that a Host gives to each receive call: enough for 4 CryptoNonces.
const _randomBytes = 64;

/// The allocator of the native credential copies. Internal: a test sets a recording allocator to
/// check that each block is cleared before it is freed (REQ-DART-NET-003).
Allocator credentialsAllocator = calloc;

/// The native copy of [c] for one call, with [random] for the CryptoNonces. Gives the result of
/// [call]; clears the secret bytes and frees every native block before it returns.
T _withCredentials<T>(
  NetCredentials c,
  Uint8List random,
  T Function(Pointer<ZmidiNetCredentials> creds) call,
) {
  final blocks = <(Pointer<Uint8>, int)>[];
  Pointer<Uint8> copy(List<int> data, String name) {
    for (var i = 0; i < data.length; i++) {
      checkInt('$name byte $i', data[i], 0xff);
    }
    final n = data.isEmpty ? 1 : data.length;
    final p = credentialsAllocator<Uint8>(n);
    blocks.add((p, n));
    p.asTypedList(n).setAll(0, data);
    return p;
  }

  try {
    final cr = credentialsAllocator<ZmidiNetCredentials>();
    blocks.add((cr.cast(), sizeOf<ZmidiNetCredentials>()));
    final users = credentialsAllocator<ZmidiNetUser>(
      c.users.isEmpty ? 1 : c.users.length,
    );
    blocks.add((
      users.cast(),
      sizeOf<ZmidiNetUser>() * (c.users.isEmpty ? 1 : c.users.length),
    ));
    for (var i = 0; i < c.users.length; i++) {
      final name = utf8.encode(c.users[i].username);
      users[i]
        ..username = copy(name, 'username')
        ..username_len = name.length
        ..password = copy(c.users[i].password, 'password')
        ..password_len = c.users[i].password.length;
    }
    final secret = c.sharedSecret;
    cr.ref
      ..shared_secret = secret == null ? nullptr : copy(secret, 'sharedSecret')
      ..shared_secret_len = secret?.length ?? 0
      ..users = c.users.isEmpty ? nullptr : users
      ..user_count = c.users.length
      ..random = random.isEmpty ? nullptr : copy(random, 'random')
      ..random_len = random.length
      ..random_used = 0;
    return call(cr);
  } finally {
    for (final (p, n) in blocks) {
      p.asTypedList(n).fillRange(0, n, 0);
      credentialsAllocator.free(p);
    }
  }
}

const _events = 128;
const _words = 1024;
const _out = 8 * (ZMIDI_NET_MAX_PAYLOAD + ZMIDI_NET_RECORD_HEADER);

void _writeAddr(Pointer<ZmidiNetAddr> p, NetAddress a) {
  final raw = a.address.rawAddress;
  p.ref
    ..family = raw.length == 4 ? 4 : 6
    ..port = a.port;
  for (var i = 0; i < 16; i++) {
    p.ref.ip[i] = i < raw.length ? raw[i] : 0;
  }
}

NetAddress _addr(int family, List<int> ip, int port) => NetAddress(
  InternetAddress.fromRawAddress(
    Uint8List.fromList(family == 4 ? ip.sublist(0, 4) : ip.sublist(0, 16)),
  ),
  port,
);

NetAddress _readAddr(ZmidiNetAddr a) =>
    _addr(a.family, [for (var i = 0; i < 16; i++) a.ip[i]], a.port);

/// The C state and work arrays of a Host or a Client, and the requests that both have.
final class NetEndpoint implements Finalizable {
  /// A Host ([host]) or a Client with the authentication methods [methods] (0: none).
  NetEndpoint(
    this.host,
    String name,
    String productInstanceId, {
    int methods = 0,
    this.credentials,
  }) {
    final ss = (host ? zmidi_net_host_size() : zmidi_net_client_size()) + 15;
    final slot = ss ~/ 16 * 16;
    final bs = (sizeOf<ZmidiNetBuffers>() + 15) ~/ 16 * 16;
    final ev = sizeOf<ZmidiNetEvent>() * _events;
    // One block: the C state, the buffers struct, the events, the words, the reply records.
    _base = allocState(this, slot + bs + ev + 4 * _words + _out);
    _bufs = Pointer.fromAddress(_base.address + slot);
    _ev = Pointer.fromAddress(_bufs.address + bs);
    _w = Pointer.fromAddress(_ev.address + ev);
    _o = Pointer.fromAddress(_w.address + 4 * _words);
    using((arena) {
      final n = name.toNativeUtf8(allocator: arena).cast<Uint8>();
      final p = productInstanceId.toNativeUtf8(allocator: arena).cast<Uint8>();
      check(switch ((host, methods != 0)) {
        (true, false) => zmidi_net_host_init(_base.cast(), n, p),
        (false, false) => zmidi_net_client_init(_base.cast(), n, p),
        (true, true) => zmidi_net_host_init_auth(_base.cast(), n, p, methods),
        (false, true) => zmidi_net_client_init_auth(
          _base.cast(),
          n,
          p,
          methods,
        ),
      }, host ? 'NetHost' : 'NetClient');
    });
  }

  /// True for a Host.
  final bool host;

  /// The credentials of each receive call, or null for the receive calls with no credentials.
  NetCredentials? credentials;

  /// Host: the random bytes from [NetCredentials.random] that no CryptoNonce used yet.
  Uint8List _pool = Uint8List(0);
  late Pointer<Uint8> _base;
  late Pointer<ZmidiNetBuffers> _bufs;
  late Pointer<ZmidiNetEvent> _ev;
  late Pointer<Uint32> _w;
  late Pointer<Uint8> _o;

  Pointer<Uint8> get _live => _base;

  void _reset() {
    _bufs.ref
      ..events = _ev
      ..events_capacity = _events
      ..events_len = 0
      ..words = _w
      ..words_capacity = _words
      ..words_len = 0
      ..out = _o
      ..out_capacity = _out
      ..out_len = 0;
  }

  List<NetEvent> _readEvents() {
    final out = <NetEvent>[];
    for (var i = 0; i < _bufs.ref.events_len; i++) {
      final e = _ev[i];
      final k = e.kindAsInt;
      if (k == 0 || k > NetEventKind.values.length) {
        continue;
      }
      final kind = NetEventKind.values[k - 1];
      out.add(
        newNetEvent(
          kind,
          _readAddr(e.peer),
          e.code,
          e.value,
          kind == NetEventKind.data
              ? splitUmps([
                  for (var j = 0; j < e.word_count; j++) _w[e.word_offset + j],
                ])
              : const [],
        ),
      );
    }
    return out;
  }

  List<NetPayload> _replies() {
    final b = _o.asTypedList(_bufs.ref.out_len);
    final out = <NetPayload>[];
    var at = 0;
    while (at + ZMIDI_NET_RECORD_HEADER <= b.length) {
      final n = (b[at + 19] << 8) | b[at + 20];
      out.add((
        to: _addr(
          b[at],
          b.sublist(at + 1, at + 17),
          (b[at + 17] << 8) | b[at + 18],
        ),
        payload: Uint8List.fromList(
          b.sublist(
            at + ZMIDI_NET_RECORD_HEADER,
            at + ZMIDI_NET_RECORD_HEADER + n,
          ),
        ),
      ));
      at += ZMIDI_NET_RECORD_HEADER + n;
    }
    return out;
  }

  /// Starts an Invitation.
  void connect(int nowMs, NetAddress to) => using((arena) {
    final a = arena<ZmidiNetAddr>();
    _writeAddr(a, to);
    check(zmidi_net_client_connect(_live.cast(), nowMs, a), 'connect');
  });

  /// Reads one UDP payload.
  ({List<NetEvent> events, List<NetPayload> replies}) receive(
    int nowMs,
    NetAddress peer,
    List<int> payload,
  ) {
    final p = _live;
    final now = nowMs;
    return using((arena) {
      final a = arena<ZmidiNetAddr>();
      _writeAddr(a, peer);
      _reset();
      final data = bytesIn(arena, payload);
      final c = credentials;
      if (c != null) {
        _receiveAuth(c, now, a, data, payload.length);
      } else {
        check(
          host
              ? zmidi_net_host_receive(
                  p.cast(),
                  now,
                  a,
                  data,
                  payload.length,
                  _bufs,
                )
              : zmidi_net_client_receive(
                  p.cast(),
                  now,
                  a,
                  data,
                  payload.length,
                  _bufs,
                ),
          'receive',
        );
      }
      return (events: _readEvents(), replies: _replies());
    });
  }

  /// The receive call with the credentials [c]: their native copy lives only for this call. A
  /// Host takes the random bytes of its CryptoNonces from the pool and keeps the bytes that the
  /// call did not use.
  void _receiveAuth(
    NetCredentials c,
    int now,
    Pointer<ZmidiNetAddr> from,
    Pointer<Uint8> data,
    int len,
  ) {
    if (host && _pool.length < _randomBytes) {
      final more = c.random(_randomBytes - _pool.length);
      _pool = Uint8List.fromList([..._pool, ...more]);
    }
    final random = host ? _pool : Uint8List(0);
    final used = _withCredentials(c, random, (cr) {
      check(
        host
            ? zmidi_net_host_receive_auth(
                _base.cast(),
                now,
                from,
                data,
                len,
                cr,
                _bufs,
              )
            : zmidi_net_client_receive_auth(
                _base.cast(),
                now,
                from,
                data,
                len,
                cr,
                _bufs,
              ),
        'receive',
      );
      return cr.ref.random_used;
    });
    if (host && used > 0) {
      _pool = Uint8List.sublistView(_pool, used.clamp(0, _pool.length));
    }
  }

  /// Runs the timers.
  List<NetEvent> poll(int nowMs) {
    final p = _live;
    _reset();
    check(
      host
          ? zmidi_net_host_poll(p.cast(), nowMs, _bufs)
          : zmidi_net_client_poll(p.cast(), nowMs, _bufs),
      'poll',
    );
    return _readEvents();
  }

  /// Gives every pending UDP payload.
  List<NetPayload> writePending(int nowMs) {
    final p = _live;
    final now = nowMs;
    return using((arena) {
      final buf = arena<Uint8>(ZMIDI_NET_MAX_PAYLOAD);
      final to = arena<ZmidiNetAddr>();
      final len = arena<UintPtr>();
      final out = <NetPayload>[];
      for (;;) {
        check(
          host
              ? zmidi_net_host_write_pending(
                  p.cast(),
                  now,
                  buf,
                  ZMIDI_NET_MAX_PAYLOAD,
                  to,
                  len,
                )
              : zmidi_net_client_write_pending(
                  p.cast(),
                  now,
                  buf,
                  ZMIDI_NET_MAX_PAYLOAD,
                  to,
                  len,
                ),
          'writePending',
        );
        if (len.value == 0) {
          return out;
        }
        out.add((to: _readAddr(to.ref), payload: bytesOut(buf, len.value)));
      }
    });
  }

  /// The next deadline, or null.
  int? nextDeadline() {
    final p = _live;
    return using((arena) {
      final d = arena<Uint64>();
      final has = host
          ? zmidi_net_host_next_deadline(p.cast(), d)
          : zmidi_net_client_next_deadline(p.cast(), d);
      return has ? d.value : null;
    });
  }

  /// Queues UMP messages.
  void sendUmp(NetAddress peer, List<int> words) {
    final p = _live;
    using((arena) {
      final a = arena<ZmidiNetAddr>();
      _writeAddr(a, peer);
      final w = wordsIn(arena, words);
      check(
        host
            ? zmidi_net_host_send_ump(p.cast(), a, w, words.length)
            : zmidi_net_client_send_ump(p.cast(), a, w, words.length),
        'sendUmp',
      );
    });
  }

  /// Sends a Ping.
  int ping(int nowMs, NetAddress peer) {
    final p = _live;
    return using((arena) {
      final a = arena<ZmidiNetAddr>();
      _writeAddr(a, peer);
      final id = arena<Uint32>();
      check(
        host
            ? zmidi_net_host_ping(p.cast(), nowMs, a, id)
            : zmidi_net_client_ping(p.cast(), nowMs, a, id),
        'ping',
      );
      return id.value;
    });
  }

  /// Ends a session.
  List<NetEvent> bye(int nowMs, NetAddress peer, int reason) {
    final p = _live;
    return using((arena) {
      final a = arena<ZmidiNetAddr>();
      _writeAddr(a, peer);
      _reset();
      check(
        host
            ? zmidi_net_host_bye(p.cast(), nowMs, a, reason, _bufs)
            : zmidi_net_client_bye(p.cast(), nowMs, a, reason, _bufs),
        'bye',
      );
      return _readEvents();
    });
  }

  /// The count of dropped events.
  int get droppedEvents => host
      ? zmidi_net_host_dropped_events(_live.cast())
      : zmidi_net_client_dropped_events(_live.cast());

  /// Frees the C state.
  void free() {
    if (_base != nullptr) {
      releaseState(this, _base);
      _base = nullptr;
    }
  }
}
