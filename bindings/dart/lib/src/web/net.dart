/// The web form of the Network MIDI 2.0 Host and Client: the TypeScript package through
/// dart:js_interop. The values are checked by `net.dart`.
library;

import 'dart:js_interop';
import 'dart:typed_data';

import '../net.dart';
import 'runtime.dart';

/// Random bytes that a Host gives to each receive call: enough for 4 CryptoNonces.
const _randomBytes = 64;

JSObject _addr(NetAddress a) =>
    jsObject({'address': a.address.address.toJS, 'port': a.port.toJS});

NetAddress _readAddr(JSAny? a) {
  final o = a as JSObject;
  return NetAddress.parse(
    prop<JSString>(o, 'address').toDart,
    intOf(prop(o, 'port')),
  );
}

List<NetEvent> _events(JSAny? a) => [
  for (final e in (a as JSArray<JSObject>).toDart)
    newNetEvent(
      NetEventKind.values.byName(prop<JSString>(e, 'kind').toDart),
      _readAddr(prop(e, 'peer')),
      intOf(prop(e, 'code')),
      intOf(prop(e, 'value')),
      umps(prop(e, 'umps')),
    ),
];

List<NetPayload> _payloads(JSAny? a) => [
  for (final p in (a as JSArray<JSObject>).toDart)
    (to: _readAddr(prop(p, 'to')), payload: bytes(prop(p, 'payload'))),
];

/// The TypeScript NetHost or NetClient.
final class NetEndpoint {
  /// A Host ([host]) or a Client with the authentication methods [methods] (0: none).
  NetEndpoint(
    this.host,
    String name,
    String productInstanceId, {
    int methods = 0,
    this.credentials,
  }) : _o = tsNew(host ? 'NetHost' : 'NetClient', [
         name.toJS,
         productInstanceId.toJS,
         if (methods != 0)
           jsObject({
             'methods': [
               if (methods & netAuthSharedSecret != 0) 'sharedSecret'.toJS,
               if (methods & netAuthUser != 0) 'user'.toJS,
             ].toJS,
           }),
       ]);

  /// True for a Host.
  final bool host;

  /// The credentials of each receive call, or null for the receive calls with no credentials.
  NetCredentials? credentials;

  final JSObject _o;

  /// Host: the random bytes from [NetCredentials.random] that no CryptoNonce used yet.
  Uint8List _pool = Uint8List(0);

  /// Starts an Invitation.
  void connect(int nowMs, NetAddress to) =>
      call<JSAny?>(_o, 'connect', [nowMs.toJS, _addr(to)]);

  /// Reads one UDP payload. With credentials, the TypeScript package copies them into the WASM
  /// memory for this call only, and clears that copy before the call returns.
  ({List<NetEvent> events, List<NetPayload> replies}) receive(
    int nowMs,
    NetAddress peer,
    List<int> payload,
  ) {
    final c = credentials;
    if (c != null && host && _pool.length < _randomBytes) {
      final more = c.random(_randomBytes - _pool.length);
      _pool = Uint8List.fromList([..._pool, ...more]);
    }
    call<JSAny?>(_o, 'setCredentials', [
      if (c == null)
        null
      else
        jsObject({
          'sharedSecret': switch (c.sharedSecret) {
            final s? => jsBytes(s),
            null => null,
          },
          'users': [
            for (final u in c.users)
              jsObject({
                'username': u.username.toJS,
                'password': jsBytes(u.password),
              }),
          ].toJS,
          'random': host ? jsBytes(_pool) : null,
        }),
    ]);
    final r = call<JSObject>(_o, 'receive', [
      nowMs.toJS,
      _addr(peer),
      jsBytes(payload),
    ]);
    if (c != null && host) {
      final used = intOf(prop(_o, 'randomUsed'));
      _pool = Uint8List.sublistView(_pool, used.clamp(0, _pool.length));
    }
    return (
      events: _events(prop(r, 'events')),
      replies: _payloads(prop(r, 'replies')),
    );
  }

  /// Runs the timers.
  List<NetEvent> poll(int nowMs) => _events(call(_o, 'poll', [nowMs.toJS]));

  /// Gives every pending UDP payload.
  List<NetPayload> writePending(int nowMs) =>
      _payloads(call(_o, 'writePending', [nowMs.toJS]));

  /// The next deadline, or null.
  int? nextDeadline() => intOrNull(call(_o, 'nextDeadline'));

  /// Queues UMP messages.
  void sendUmp(NetAddress peer, List<int> words) =>
      call<JSAny?>(_o, 'sendUmp', [_addr(peer), jsInts(words)]);

  /// Sends a Ping.
  int ping(int nowMs, NetAddress peer) =>
      intOf(call(_o, 'ping', [nowMs.toJS, _addr(peer)]));

  /// Ends a session.
  List<NetEvent> bye(int nowMs, NetAddress peer, int reason) =>
      _events(call(_o, 'bye', [nowMs.toJS, _addr(peer), reason.toJS]));

  /// The count of dropped events.
  int get droppedEvents => intOf(prop(_o, 'droppedEvents'));

  /// Frees the WASM memory.
  void free() => call<JSAny?>(_o, 'free');
}
