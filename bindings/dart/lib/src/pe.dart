/// The Property Exchange engine of zmidi-ci-pe through the C ABI 0.3 (zmidi_pe_*; M2-103): Property
/// Exchange with many Chunks, Flow Control and Subscriptions over a [CiDevice]. The program gives
/// each received message to [CiDevice.receive] first, then calls [PeEngine.receive] with the same
/// device. The start calls (reply, update, request, subscribe, unsubscribe) start a transaction and
/// [PeEngine.send] writes its Chunks; the calls that queue a control message (replyStatus, refresh,
/// wait, abort, corrupt) need [PeEngine.writePending]. The encodings are "ASCII" and "Mcoded7";
/// "zlib+Mcoded7" is not inflated.
library;

import 'dart:typed_data';

import 'checks.dart';
import 'ci.dart';
import 'errors.dart';
import 'native/pe.dart' if (dart.library.js_interop) 'web/pe.dart' as be;

/// The encoding of Property Data (M2-103 Table 12).
enum PeEncoding {
  /// "ASCII".
  ascii,

  /// "Mcoded7".
  mcoded7,

  /// "zlib+Mcoded7": the engine gives the zlib stream.
  zlibMcoded7,
}

/// "canSet" of a Resource (M2-103 Table 14).
enum PeCanSet {
  /// "none".
  none,

  /// "full".
  full,

  /// "partial".
  partial,
}

/// The role of this Device in a transaction or a subscription.
enum PeRole {
  /// This Device sent the inquiry or the "start".
  initiator,

  /// This Device answers.
  responder,
}

/// The kind of a transaction.
enum PeTxnKind {
  /// Get Property Data and its reply.
  get,

  /// Set Property Data and its reply.
  set,

  /// Subscription and its reply.
  subscription,
}

/// The end of a received Chunk (M2-103 5.3).
enum PeChunkEnd {
  /// More Chunks follow.
  more,

  /// The last Chunk: the Property Data is complete.
  complete,

  /// The last Chunk: the Property Data is not known to be usable.
  incomplete,
}

/// The "command" of a Subscription message (M2-103 11).
enum PeCommand {
  /// "start".
  start,

  /// "partial".
  partial,

  /// "full".
  full,

  /// "notify".
  notify,

  /// "end".
  end,
}

/// The kind of a [PeEvent] (one for each PeEvent of zmidi-ci-pe).
enum PeEventKind {
  /// A received Chunk of a message.
  message,

  /// Flow Control: send the next Chunk.
  ready,

  /// Flow Control: send the Chunk [PeEvent.chunk] again.
  resend,

  /// A transaction closed; [PeEvent.reason] tells why.
  closed,

  /// This Device accepted a "start"; [PeEvent.subscription] is the new entry.
  subscriptionStarted,

  /// The peer ended a subscription with "end".
  subscriptionEnded,

  /// An Invalidate MUID closed the transactions of [PeEvent.muid].
  invalidated,
}

/// Why a transaction closed.
enum PeCloseReason {
  /// A deadline of this Device passed.
  timeout,

  /// The peer ended it: NAK 0x20 or Notify 144.
  terminated,

  /// The peer timed out: NAK 0x42 or Notify 408.
  peerTimeout,

  /// A Chunk came out of sequence; this Device sent NAK 0x21.
  outOfSequence,

  /// The peer sent another NAK: [PeEvent.nakCode] and [PeEvent.nakData].
  nak,

  /// The Header Data of the reply is not valid.
  badReply,

  /// This Device replied with the status [PeEvent.status] after an error.
  refused,

  /// An Invalidate MUID ended it.
  invalidated,

  /// A Flow Control resend that this Device can not do.
  resendFailed,
}

/// Why the reply to a "start" gave no subscription (REQ-PE-SUB-006).
enum PeStartError {
  /// The subscription table is full.
  full,

  /// The reply has no "subscribeId".
  noSubscribeId,

  /// The reply has no status from 200 to 299.
  notSuccess,

  /// A command or Property Data that the role does not permit.
  notPermitted,

  /// A header value is too long, or a Property is missing.
  invalid,
}

/// One entry of the Resource table (M2-103 Table 14).
final class PeResource {
  /// An entry for [resource] (1 to 36 characters A-Z, a-z, 0-9 with an optional "X-" prefix).
  const PeResource(
    this.resource, {
    this.canGet = true,
    this.canSet = PeCanSet.none,
    this.canSubscribe = false,
    this.requireResId = false,
    this.fcOnGet = false,
    this.fcOnSet = false,
    this.encodings = const [PeEncoding.ascii],
    this.canPaginate = false,
    this.mediaType,
    this.schema,
    this.writeDefaults = false,
  });

  /// "resource".
  final String resource;

  /// "canGet".
  final bool canGet;

  /// "canSet".
  final PeCanSet canSet;

  /// "canSubscribe".
  final bool canSubscribe;

  /// "requireResId".
  final bool requireResId;

  /// "fcOnGet".
  final bool fcOnGet;

  /// "fcOnSet".
  final bool fcOnSet;

  /// "encodings"; at least one.
  final List<PeEncoding> encodings;

  /// "canPaginate".
  final bool canPaginate;

  /// The one "mediaType" ("type/subtype"), or null for "application/json".
  final String? mediaType;

  /// "schema" as JSON text, or null.
  final List<int>? schema;

  /// Write every Property, also the ones with their default value.
  final bool writeDefaults;
}

/// One event of a [PeEngine]. The fields that the kind does not use are null or empty.
final class PeEvent {
  PeEvent._({
    required this.kind,
    this.muid,
    this.requestId,
    this.role,
    this.txn,
    this.end,
    this.encoding,
    this.command,
    this.status,
    this.chunk,
    this.resourceIndex,
    this.subscription,
    this.started,
    this.startError,
    this.reason,
    this.nakCode,
    this.nakData,
    this.ended,
    this.resource,
    this.resId,
    this.subscribeId,
    required this.header,
    required this.data,
  });

  /// The kind of the event.
  final PeEventKind kind;

  /// The MUID of the peer ([PeEventKind.invalidated]: the invalidated MUID).
  final int? muid;

  /// Request ID of the transaction.
  final int? requestId;

  /// The role of this Device.
  final PeRole? role;

  /// The kind of the transaction.
  final PeTxnKind? txn;

  /// Message: the end of this Chunk.
  final PeChunkEnd? end;

  /// Message: the encoding of the Property Data.
  final PeEncoding? encoding;

  /// Message: the "command" of the header.
  final PeCommand? command;

  /// Message: the "status" of the header; closed with [PeCloseReason.refused]: the status.
  final int? status;

  /// Resend: the number of the Chunk to send again.
  final int? chunk;

  /// Message: the index of the Resource in the Resource table.
  final int? resourceIndex;

  /// Message: the subscription entry of the message; subscriptionStarted: the new entry.
  final int? subscription;

  /// Message of the reply to a "start": the new subscription entry.
  final int? started;

  /// Message of the reply to a "start" with no new entry: why.
  final PeStartError? startError;

  /// Closed: why.
  final PeCloseReason? reason;

  /// Closed with [PeCloseReason.nak]: the NAK Status Code.
  final int? nakCode;

  /// Closed with [PeCloseReason.nak]: the NAK Status Data.
  final int? nakData;

  /// Invalidated: the number of removed subscriptions (see [PeEngine.takeEnded]).
  final int? ended;

  /// Message: the decoded "resource"; subscriptionEnded: the entry resource.
  final String? resource;

  /// Message: the decoded "resId"; subscriptionEnded: the entry resId.
  final String? resId;

  /// Message: the decoded "subscribeId"; subscriptionEnded: the entry subscribeId.
  final String? subscribeId;

  /// Message: the Header Data (first Chunk only).
  final Uint8List header;

  /// Message: the decoded Property Data of this Chunk.
  final Uint8List data;

  @override
  String toString() =>
      'PeEvent(kind: ${kind.name}, muid: $muid, requestId: $requestId)';
}

/// A copy of one subscription table entry.
final class PeSubscription {
  PeSubscription._(
    this.muid,
    this.role,
    this.subscribeId,
    this.resource,
    this.resId,
  );

  /// The MUID of the peer.
  final int muid;

  /// The role of this Device: [PeRole.initiator] sent the "start".
  final PeRole role;

  /// "subscribeId".
  final String subscribeId;

  /// "resource".
  final String resource;

  /// "resId", or null.
  final String? resId;
}

/// A PE event of the native or the web form. The entry library hides it.
PeEvent newPeEvent({
  required PeEventKind kind,
  int? muid,
  int? requestId,
  PeRole? role,
  PeTxnKind? txn,
  PeChunkEnd? end,
  PeEncoding? encoding,
  PeCommand? command,
  int? status,
  int? chunk,
  int? resourceIndex,
  int? subscription,
  int? started,
  PeStartError? startError,
  PeCloseReason? reason,
  int? nakCode,
  int? nakData,
  int? ended,
  String? resource,
  String? resId,
  String? subscribeId,
  required Uint8List header,
  required Uint8List data,
}) => PeEvent._(
  kind: kind,
  muid: muid,
  requestId: requestId,
  role: role,
  txn: txn,
  end: end,
  encoding: encoding,
  command: command,
  status: status,
  chunk: chunk,
  resourceIndex: resourceIndex,
  subscription: subscription,
  started: started,
  startError: startError,
  reason: reason,
  nakCode: nakCode,
  nakData: nakData,
  ended: ended,
  resource: resource,
  resId: resId,
  subscribeId: subscribeId,
  header: header,
  data: data,
);

/// A subscription entry of the native or the web form. The entry library hides it.
PeSubscription newPeSubscription(
  int muid,
  PeRole role,
  String subscribeId,
  String resource,
  String? resId,
) => PeSubscription._(muid, role, subscribeId, resource, resId);

/// The largest Resource table (ZMIDI_PE_RESOURCES_MAX).
const _resourcesMax = 16;

int _peer(int muid) => checkInt('peer', muid, 0x0fffffff);

int _requestId(int id) => checkInt('requestId', id, 0x7f);

int? _length(int? len) =>
    len == null ? null : checkInt('length', len, 0x7fffffff);

/// The Property Exchange engine of zmidi-ci-pe over a [CiDevice] (4 transactions with 256 header
/// bytes, 4 subscriptions, 4 peers with a known ResourceList, 16 queued control messages).
final class PeEngine {
  /// An engine with the Resource table [resources] (at most 16 entries). Throws
  /// [InvalidValueException] for an entry that is not valid (M2-103 8.1 "resource", a "mediaType"
  /// that is not "type/subtype", no encoding, more than 512 bytes of ResourceList JSON).
  PeEngine([List<PeResource> resources = const []])
    : _b = be.PeState(_checkTable(resources));

  static List<PeResource> _checkTable(List<PeResource> resources) {
    if (resources.length > _resourcesMax) {
      throw InvalidValueException(
        'a Resource table has at most $_resourcesMax entries, '
        'not ${resources.length}',
      );
    }
    return List.of(resources);
  }

  final be.PeState _b;
  bool _freed = false;

  be.PeState get _live {
    checkFreed(_freed, this);
    return _b;
  }

  /// Gives the last event of [dev] (from [CiDevice.receive]) to the engine at [nowUs]. Gives the
  /// engine event, or null. An event that the engine took is not given again. A refusal (for
  /// example a full queue: call [writePending] first) throws and changes nothing.
  PeEvent? receive(CiDevice dev, int nowUs) {
    final b = _live;
    return b.receive(ciHandle(dev), checkTime('nowUs', nowUs));
  }

  /// Runs the deadlines at [nowUs]. Gives one event, or null; call it until it gives null.
  PeEvent? poll(CiDevice dev, int nowUs) {
    final b = _live;
    return b.poll(ciHandle(dev), checkTime('nowUs', nowUs));
  }

  /// Writes the next queued control message (status reply, ACK, NAK, a Get after a "notify"), or
  /// gives null when the queue is empty.
  Uint8List? writePending(CiDevice dev, int nowUs) {
    final b = _live;
    return b.writePending(ciHandle(dev), checkTime('nowUs', nowUs));
  }

  /// Starts the reply to the complete inquiry [requestId] of [muid]: [status], the [encoding] of
  /// the inquiry, [length] bytes of encoded Property Data (null when not known), "mediaType",
  /// "totalCount" and "cacheTime". [send] writes the Chunks. A reply after the deadline throws
  /// [RejectedException] and closes the transaction.
  void reply(
    CiDevice dev,
    int muid,
    int requestId, {
    int status = 200,
    int? length = 0,
    PeEncoding encoding = PeEncoding.ascii,
    String? mediaType,
    int? totalCount,
    int? cacheTime,
    int nowUs = 0,
  }) {
    final b = _live;
    b.reply(
      ciHandle(dev),
      _peer(muid),
      _requestId(requestId),
      checkInt('status', status, 0xffff),
      _length(length),
      encoding,
      mediaType,
      totalCount == null
          ? null
          : checkInt('totalCount', totalCount, 0xffffffff),
      cacheTime == null ? null : checkInt('cacheTime', cacheTime, 0xffffffff),
      checkTime('nowUs', nowUs),
    );
  }

  /// Replies to the open inquiry [requestId] of [muid] with [status] and no Property Data, and
  /// closes it. [writePending] writes the reply.
  void replyStatus(
    CiDevice dev,
    int muid,
    int requestId,
    int status, {
    int nowUs = 0,
  }) {
    final b = _live;
    b.replyStatus(
      ciHandle(dev),
      _peer(muid),
      _requestId(requestId),
      checkInt('status', status, 0xffff),
      checkTime('nowUs', nowUs),
    );
  }

  /// Starts a [PeCommand.partial], [PeCommand.full] or [PeCommand.notify] Subscription message for
  /// the subscription [index] in which this Device is the responder. Gives its Request ID; [send]
  /// (role [PeRole.initiator]) writes the Chunks.
  int update(
    CiDevice dev,
    int index,
    PeCommand command, {
    int? length = 0,
    PeEncoding encoding = PeEncoding.ascii,
    int nowUs = 0,
  }) {
    if (command != PeCommand.partial &&
        command != PeCommand.full &&
        command != PeCommand.notify) {
      throw InvalidValueException(
        'an update is partial, full or notify, not ${command.name}',
      );
    }
    final b = _live;
    return b.update(
      ciHandle(dev),
      checkInt('index', index, 0xffff),
      command,
      _length(length),
      encoding,
      checkTime('nowUs', nowUs),
    );
  }

  /// The ResourceList JSON of the Resource table, for the reply to a Get of "ResourceList".
  Uint8List resourceList() => _live.resourceList();

  /// Starts a Get ([PeTxnKind.get]) or Set ([PeTxnKind.set]) inquiry to [muid] for [resource].
  /// [caps] are the capabilities of the Resource in the ResourceList of the peer. Gives the Request
  /// ID and whether this Device has no ResourceList of the peer yet; [send] writes the Chunks.
  ({int requestId, bool resourceListMissing}) request(
    CiDevice dev,
    int muid,
    PeTxnKind kind,
    String resource, {
    String? resId,
    String? mediaType,
    int? offset,
    int? limit,
    bool setPartial = false,
    PeEncoding encoding = PeEncoding.ascii,
    int? length = 0,
    PeResource? caps,
    int nowUs = 0,
  }) {
    if (kind == PeTxnKind.subscription) {
      throw const InvalidValueException('a request is get or set');
    }
    if ((offset == null) != (limit == null)) {
      throw const InvalidValueException(
        'offset and limit come together (M2-103 8.5)',
      );
    }
    final b = _live;
    final peer = _peer(muid);
    final now = checkTime('nowUs', nowUs);
    return b.request(
      ciHandle(dev),
      peer,
      kind,
      (
        resource: resource,
        resId: resId,
        mediaType: mediaType,
        offset: offset == null ? null : checkInt('offset', offset, 0xffffffff),
        limit: limit == null ? null : checkInt('limit', limit, 0xffffffff),
        setPartial: setPartial,
        encoding: encoding,
        length: _length(length),
      ),
      caps,
      now,
    );
  }

  /// Starts the Get of the ResourceList of [muid] and writes it.
  ({int requestId, Uint8List message}) requestResourceList(
    CiDevice dev,
    int muid, {
    int nowUs = 0,
  }) {
    final b = _live;
    final peer = _peer(muid);
    return b.requestResourceList(
      ciHandle(dev),
      peer,
      checkTime('nowUs', nowUs),
    );
  }

  /// Starts a "start" Subscription message to [muid] for [resource], with the [encoding] of the
  /// updates. Gives the Request ID and whether this Device has no ResourceList of the peer yet;
  /// [send] writes it with no data.
  ({int requestId, bool resourceListMissing}) subscribe(
    CiDevice dev,
    int muid,
    String resource, {
    String? resId,
    PeEncoding encoding = PeEncoding.ascii,
    PeResource? caps,
    int nowUs = 0,
  }) {
    final b = _live;
    final peer = _peer(muid);
    final now = checkTime('nowUs', nowUs);
    return b.subscribe(
      ciHandle(dev),
      peer,
      (
        resource: resource,
        resId: resId,
        mediaType: null,
        offset: null,
        limit: null,
        setPartial: false,
        encoding: encoding,
        length: 0,
      ),
      caps,
      now,
    );
  }

  /// Starts an "end" Subscription message for the subscription [index] and removes the entry.
  /// Gives the Request ID; [send] writes it with no data.
  int unsubscribe(CiDevice dev, int index, {int nowUs = 0}) {
    final b = _live;
    return b.unsubscribe(
      ciHandle(dev),
      checkInt('index', index, 0xffff),
      checkTime('nowUs', nowUs),
    );
  }

  /// Asks again for the Property Data of the subscription [index]; [writePending] writes the Get.
  void refresh(int index) {
    final b = _live;
    b.refresh(checkInt('index', index, 0xffff));
  }

  /// The Property Data bytes that the next Chunk of the transaction takes.
  int dataRoom(CiDevice dev, int muid, int requestId, PeRole role) {
    final b = _live;
    return b.dataRoom(ciHandle(dev), _peer(muid), _requestId(requestId), role);
  }

  /// Writes the next Chunk of the transaction with [part] (at most the data room) as its encoded
  /// Property Data; [last] ends the message. Gives the message.
  Uint8List send(
    CiDevice dev,
    int muid,
    int requestId,
    PeRole role,
    List<int> part,
    bool last, {
    int nowUs = 0,
  }) {
    final b = _live;
    final peer = _peer(muid);
    final id = _requestId(requestId);
    final now = checkTime('nowUs', nowUs);
    return b.send(ciHandle(dev), peer, id, role, part, last, now);
  }

  /// Asks the peer to wait [ms] more milliseconds (1 to 12700) with a MIDI-CI ACK 0x10;
  /// [writePending] writes it.
  void wait(int muid, int requestId, PeRole role, int ms) {
    final b = _live;
    b.wait(
      _peer(muid),
      _requestId(requestId),
      role,
      checkInt('ms', ms, 0xffffffff),
    );
  }

  /// Ends the transaction with a MIDI-CI NAK 0x20; [writePending] writes it.
  void abort(CiDevice dev, int muid, int requestId, PeRole role) {
    final b = _live;
    b.abort(ciHandle(dev), _peer(muid), _requestId(requestId), role);
  }

  /// Asks again for a corrupt Chunk with Flow Control (NAK 0x12); [writePending] writes it.
  void corrupt(int muid, int requestId, PeRole role) {
    final b = _live;
    b.corrupt(_peer(muid), _requestId(requestId), role);
  }

  /// The subscription entry [index], or null for an empty index.
  PeSubscription? subscription(int index) {
    final b = _live;
    return b.subscription(checkInt('index', index, 0xffff));
  }

  /// The next subscription that an Invalidate MUID removed, or null.
  PeSubscription? takeEnded() => _live.takeEnded();

  /// Frees the memory of the engine now.
  void free() {
    if (!_freed) {
      _freed = true;
      _b.free();
    }
  }
}

/// The checked header values of a request or a subscription. The entry library hides it.
typedef PeHeader = ({
  String resource,
  String? resId,
  String? mediaType,
  int? offset,
  int? limit,
  bool setPartial,
  PeEncoding encoding,
  int? length,
});
