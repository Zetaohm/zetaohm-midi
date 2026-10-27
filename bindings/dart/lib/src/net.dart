/// Network MIDI 2.0 Host and Client over UDP (zmidi-net through the C ABI, M2-124), sans-IO: the
/// program owns the socket, gives each received payload, and sends each payload that the endpoint
/// writes. Authentication with a shared secret or a user table (M2-124 6.7 to 6.10) is optional.
library;

import 'dart:math';
import 'dart:typed_data';

import 'address.dart' if (dart.library.js_interop) 'web/address.dart';
import 'checks.dart';
import 'errors.dart';
import 'native/net.dart' if (dart.library.js_interop) 'web/net.dart' as be;

/// The shared secret method bit (ZMIDI_NET_AUTH_SHARED_SECRET). The entry library hides it.
const netAuthSharedSecret = 1;

/// The user method bit (ZMIDI_NET_AUTH_USER). The entry library hides it.
const netAuthUser = 2;

/// ZMIDI_NET_AUTH_USERNAME_NOT_FOUND.
const _usernameNotFound = 256;

/// An IP address and a UDP port.
final class NetAddress {
  /// The address [address] at the UDP [port] (0 to 65535).
  NetAddress(this.address, this.port) {
    checkInt('port', port, 0xffff);
    if (address.type != InternetAddressType.IPv4 &&
        address.type != InternetAddressType.IPv6) {
      throw InvalidValueException('not an IP address: $address');
    }
  }

  /// The address of [text] (IPv4 dotted or IPv6) at the UDP [port]. Throws
  /// [InvalidValueException] when [text] is not an IP address.
  factory NetAddress.parse(String text, int port) {
    final a = InternetAddress.tryParse(text);
    if (a == null) {
      throw InvalidValueException('not an IP address: $text');
    }
    return NetAddress(a, port);
  }

  /// The IP address.
  final InternetAddress address;

  /// The UDP port.
  final int port;

  @override
  bool operator ==(Object other) =>
      other is NetAddress && other.address == address && other.port == port;

  @override
  int get hashCode => Object.hash(address, port);

  @override
  String toString() => address.type == InternetAddressType.IPv6
      ? '[${address.address}]:$port'
      : '${address.address}:$port';
}

/// The kind of a net event.
enum NetEventKind {
  /// Host: a new Client asks for a session.
  invitationReceived,

  /// Client: the Host answered Pending.
  pending,

  /// Client: the Host asks for authentication, and the Client answers at once with its
  /// [NetCredentials] (or declines with Bye 0x80 when it has none); [NetEvent.authMethod] is the
  /// method, [NetEvent.value] the Authentication State (0x00 first request, 0x01 digest not
  /// correct, 0x02 Username not found; M2-124 Tables 15 and 17).
  authRequested,

  /// Host: a failed authentication; [NetEvent.authMethod] is the method,
  /// [NetEvent.usernameNotFound] is true when the Host did not find the Username, and
  /// [NetEvent.value] is the count of failures of the session.
  authFailed,

  /// A problem with the [NetCredentials] of the program; [NetEvent.code]: 0 no random bytes
  /// (Host), 1 no shared secret (Host), 2 empty shared secret, 3 empty password, 4 Username not
  /// valid (Client), 5 Username too long (Client).
  credentialsError,

  /// The session is established; [NetEvent.authMethod] is the authentication method, or null for
  /// none.
  established,

  /// This side ends the session with a Bye; [NetEvent.code] is the Bye Reason.
  ending,

  /// The session went to Idle; [NetEvent.code]: 0 Bye received ([NetEvent.value]: its Bye Reason),
  /// 1 Bye Reply, 2 Bye timeout, 3 expired, 4 ended by this side ([NetEvent.value]: the Bye
  /// Reason).
  closed,

  /// The session was reset.
  reset,

  /// This side started a Session Reset.
  resetStarted,

  /// A NAK was received; [NetEvent.code] is the NAK Reason.
  nak,

  /// A Ping Reply; [NetEvent.code] is the Ping Id, [NetEvent.value] the round-trip time in ms.
  pingReply,

  /// A Ping Reply with no open Ping; [NetEvent.code] is the Ping Id.
  unexpectedPingReply,

  /// The session timed out.
  timeout,

  /// An event of the UMP Data stream (data loss, Retransmit Error).
  stream,

  /// UMP Data: [NetEvent.umps].
  data,

  /// A diagnostic; [NetEvent.code]: 0 bad signature, 1 invalid peer address, 2 table full, 3 NAK
  /// sent ([NetEvent.value]: its NAK Reason).
  diagnostic,
}

/// One event of a peer.
final class NetEvent {
  const NetEvent._(this.kind, this.peer, this.code, this.value, this.umps);

  /// The kind of the event.
  final NetEventKind kind;

  /// The address of the peer.
  final NetAddress peer;

  /// A code of the kind (see [NetEventKind]).
  final int code;

  /// A value of the kind (see [NetEventKind]).
  final int value;

  /// [NetEventKind.data]: the UMPs, each a list of words.
  final List<List<int>> umps;

  /// [NetEventKind.authRequested], [NetEventKind.authFailed], [NetEventKind.established]: the
  /// authentication method of [code]; null for the other kinds and for a session with no
  /// authentication.
  NetAuthMethod? get authMethod => switch (kind) {
    NetEventKind.authRequested ||
    NetEventKind.authFailed ||
    NetEventKind.established => switch (code & 0xff) {
      netAuthSharedSecret => NetAuthMethod.sharedSecret,
      netAuthUser => NetAuthMethod.user,
      _ => null,
    },
    _ => null,
  };

  /// [NetEventKind.authFailed]: true when the Host did not find the Username (M2-124 6.10,
  /// Authentication State 0x02); false for the other kinds.
  bool get usernameNotFound =>
      kind == NetEventKind.authFailed && code & _usernameNotFound != 0;
}

/// An authentication method of Network MIDI 2.0.
enum NetAuthMethod {
  /// The shared secret method (M2-124 6.9): Capabilities bit D0.
  sharedSecret,

  /// The user method with a Username and a password (M2-124 6.10): Capabilities bit D1.
  user,
}

int _methods(Set<NetAuthMethod> methods) {
  if (methods.isEmpty) {
    throw const InvalidValueException('no authentication method');
  }
  return (methods.contains(NetAuthMethod.sharedSecret)
          ? netAuthSharedSecret
          : 0) |
      (methods.contains(NetAuthMethod.user) ? netAuthUser : 0);
}

/// One user of the user method (M2-124 6.10): a Username and its password.
final class NetUser {
  /// The user [username] (UTF-8 on the wire) with the password bytes [password].
  const NetUser(this.username, this.password);

  /// The Username.
  final String username;

  /// The password bytes.
  final List<int> password;
}

/// The credentials of a Host or a Client for the authentication (M2-124 6.7 to 6.10). The package
/// keeps them in Dart memory and copies them into native memory only for the length of each
/// receive call; it clears and frees that copy before the call returns (the C ABI keeps no
/// pointer).
///
/// - Host: [sharedSecret] verifies the digest of the shared secret method; [users] is the user
///   table of the user method; [random] gives the bytes of each new CryptoNonce.
/// - Client: [sharedSecret] answers the shared secret method; the first of [users] answers the
///   user method. With no value the Client declines (Bye 0x80).
final class NetCredentials {
  /// Credentials with the shared secret bytes [sharedSecret], the user table [users] and the
  /// source of random bytes [random] (it gives `count` bytes; default: [Random.secure]).
  NetCredentials({
    this.sharedSecret,
    this.users = const [],
    List<int> Function(int count)? random,
  }) : random = random ?? _secureRandom;

  /// The shared secret bytes, or null for none.
  final List<int>? sharedSecret;

  /// The user table (Host) or the user of this Client (the first entry).
  final List<NetUser> users;

  /// The source of random bytes for the CryptoNonces of a Host (M2-124 6.7): each call gives
  /// `count` bytes from 0 to 255.
  final List<int> Function(int count) random;

  static final _rng = Random.secure();

  static List<int> _secureRandom(int count) => [
    for (var i = 0; i < count; i++) _rng.nextInt(256),
  ];
}

/// A net event of the native or the web form. The entry library hides it.
NetEvent newNetEvent(
  NetEventKind kind,
  NetAddress peer,
  int code,
  int value,
  List<List<int>> umps,
) => NetEvent._(kind, peer, code, value, umps);

/// One UDP payload with its destination.
typedef NetPayload = ({NetAddress to, Uint8List payload});

/// Checks the bytes of [c] (0 to 255 each).
NetCredentials? _checkCredentials(NetCredentials? c) {
  if (c == null) {
    return null;
  }
  final secret = c.sharedSecret;
  if (secret != null) {
    checkBytes('sharedSecret', secret);
  }
  for (final u in c.users) {
    checkBytes('password', u.password);
  }
  return c;
}

/// The checks and the free state of a Host or a Client over its endpoint of the native or the web
/// form.
final class _Checked {
  _Checked(this.e);

  final be.NetEndpoint e;
  bool freed = false;

  be.NetEndpoint get live {
    checkFreed(freed, e.host ? 'NetHost' : 'NetClient');
    return e;
  }

  void connect(int nowMs, NetAddress to) {
    final b = live;
    b.connect(checkTime('nowMs', nowMs), to);
  }

  ({List<NetEvent> events, List<NetPayload> replies}) receive(
    int nowMs,
    NetAddress peer,
    List<int> payload,
  ) {
    final b = live;
    final now = checkTime('nowMs', nowMs);
    _checkCredentials(b.credentials);
    return b.receive(now, peer, payload);
  }

  List<NetEvent> poll(int nowMs) {
    final b = live;
    return b.poll(checkTime('nowMs', nowMs));
  }

  List<NetPayload> writePending(int nowMs) {
    final b = live;
    return b.writePending(checkTime('nowMs', nowMs));
  }

  int? nextDeadline() => live.nextDeadline();

  void sendUmp(NetAddress peer, List<int> words) {
    final b = live;
    if (words.isEmpty) {
      throw const InvalidValueException('no words');
    }
    for (var i = 0; i < words.length; i++) {
      checkInt('word $i', words[i], 0xffffffff);
    }
    b.sendUmp(peer, words);
  }

  int ping(int nowMs, NetAddress peer) {
    final b = live;
    return b.ping(checkTime('nowMs', nowMs), peer);
  }

  List<NetEvent> bye(int nowMs, NetAddress peer, int reason) {
    final b = live;
    checkInt('reason', reason, 0xff);
    return b.bye(checkTime('nowMs', nowMs), peer, reason);
  }

  int get droppedEvents => live.droppedEvents;

  void free() {
    if (!freed) {
      freed = true;
      e.free();
    }
  }
}

/// A Network MIDI 2.0 Host with 4 sessions. With no authentication it accepts each Client; with
/// [NetHost.auth] it asks each new Client for authentication.
final class NetHost {
  /// A Host with the UMP Endpoint Name [name] and the Product Instance Id [productInstanceId],
  /// with no authentication.
  NetHost(String name, String productInstanceId)
    : _e = _Checked(be.NetEndpoint(true, name, productInstanceId));

  /// A Host that asks each new Client for authentication with [methods] (M2-124 6.7, 6.8): with
  /// both, a Client that has both methods gets the user method. A Client with no matching
  /// Capabilities bit gets Bye 0x45. [credentials] verify the digests and give the random bytes
  /// of the CryptoNonces. Throws [InvalidValueException] for an empty [methods].
  NetHost.auth(
    String name,
    String productInstanceId, {
    required Set<NetAuthMethod> methods,
    required NetCredentials credentials,
  }) : _e = _Checked(
         be.NetEndpoint(
           true,
           name,
           productInstanceId,
           methods: _methods(methods),
           credentials: credentials,
         ),
       );

  final _Checked _e;

  /// The credentials of each receive call; null: the receive calls with no credentials.
  NetCredentials? get credentials => _e.e.credentials;

  /// Sets the credentials of the next receive calls (for example a new user table).
  set credentials(NetCredentials? value) => _e.e.credentials = value;

  /// Reads one UDP [payload] from [peer] at [nowMs]. Gives the events and the reply payloads to
  /// send.
  ({List<NetEvent> events, List<NetPayload> replies}) receive(
    int nowMs,
    NetAddress peer,
    List<int> payload,
  ) => _e.receive(nowMs, peer, payload);

  /// Runs the timers at [nowMs]; gives their events.
  List<NetEvent> poll(int nowMs) => _e.poll(nowMs);

  /// Gives every pending UDP payload (UMP Data, Pings, retransmits) with its destination.
  List<NetPayload> writePending(int nowMs) => _e.writePending(nowMs);

  /// The next time (ms) at which the Host needs a poll, or null when there is none.
  int? nextDeadline() => _e.nextDeadline();

  /// Queues whole UMP messages for [peer].
  void sendUmp(NetAddress peer, List<int> words) => _e.sendUmp(peer, words);

  /// Sends a Ping to [peer] at [nowMs]; gives its Ping Id.
  int ping(int nowMs, NetAddress peer) => _e.ping(nowMs, peer);

  /// Ends the session with [peer] with a Bye of [reason] (default 0x01, user terminated); gives
  /// its events.
  List<NetEvent> bye(int nowMs, NetAddress peer, {int reason = 1}) =>
      _e.bye(nowMs, peer, reason);

  /// The count of events that did not fit and were dropped.
  int get droppedEvents => _e.droppedEvents;

  /// Frees the memory of the Host now.
  void free() => _e.free();
}

/// A Network MIDI 2.0 Client. With [NetClient.auth] it declares authentication methods and
/// answers a request for authentication of the Host at once.
final class NetClient {
  /// A Client with the UMP Endpoint Name [name] and the Product Instance Id [productInstanceId],
  /// with no authentication.
  NetClient(String name, String productInstanceId)
    : _e = _Checked(be.NetEndpoint(false, name, productInstanceId));

  /// A Client with the Capabilities bits of [methods] in each Invitation (M2-124 Table 10) that
  /// answers a request for authentication with [credentials]: the shared secret, or the first
  /// user. Throws [InvalidValueException] for an empty [methods].
  NetClient.auth(
    String name,
    String productInstanceId, {
    required Set<NetAuthMethod> methods,
    required NetCredentials credentials,
  }) : _e = _Checked(
         be.NetEndpoint(
           false,
           name,
           productInstanceId,
           methods: _methods(methods),
           credentials: credentials,
         ),
       );

  final _Checked _e;

  /// The credentials of each receive call; null: the receive calls with no credentials.
  NetCredentials? get credentials => _e.e.credentials;

  /// Sets the credentials of the next receive calls.
  set credentials(NetCredentials? value) => _e.e.credentials = value;

  /// Starts an Invitation to the Host at [host] at [nowMs].
  void connect(int nowMs, NetAddress host) => _e.connect(nowMs, host);

  /// Reads one UDP [payload] from [peer] at [nowMs]. Gives the events and the reply payloads to
  /// send.
  ({List<NetEvent> events, List<NetPayload> replies}) receive(
    int nowMs,
    NetAddress peer,
    List<int> payload,
  ) => _e.receive(nowMs, peer, payload);

  /// Runs the timers at [nowMs]; gives their events.
  List<NetEvent> poll(int nowMs) => _e.poll(nowMs);

  /// Gives every pending UDP payload (Invitation, UMP Data, Pings) with its destination.
  List<NetPayload> writePending(int nowMs) => _e.writePending(nowMs);

  /// The next time (ms) at which the Client needs a poll, or null when there is none.
  int? nextDeadline() => _e.nextDeadline();

  /// Queues whole UMP messages for [peer].
  void sendUmp(NetAddress peer, List<int> words) => _e.sendUmp(peer, words);

  /// Sends a Ping to [peer] at [nowMs]; gives its Ping Id.
  int ping(int nowMs, NetAddress peer) => _e.ping(nowMs, peer);

  /// Ends the session with [peer] with a Bye of [reason] (default 0x01, user terminated); gives
  /// its events.
  List<NetEvent> bye(int nowMs, NetAddress peer, {int reason = 1}) =>
      _e.bye(nowMs, peer, reason);

  /// The count of events that did not fit and were dropped.
  int get droppedEvents => _e.droppedEvents;

  /// Frees the memory of the Client now.
  void free() => _e.free();
}
