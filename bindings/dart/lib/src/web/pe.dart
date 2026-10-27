/// The web form of the Property Exchange engine: the TypeScript package through dart:js_interop.
/// The values are checked by `pe.dart`.
library;

import 'dart:js_interop';
import 'dart:js_interop_unsafe';
import 'dart:typed_data';

import '../pe.dart';
import 'ci.dart';
import 'runtime.dart';

const _encodingNames = ['ASCII', 'Mcoded7', 'zlib+Mcoded7'];

JSString _encoding(PeEncoding e) => _encodingNames[e.index].toJS;

/// A JavaScript object of [fields] with `length` set, also to null (null: not known).
JSObject _withLength(Map<String, JSAny?> fields, int? length) =>
    jsObject(fields)..setProperty('length'.toJS, length?.toJS);

JSObject _resource(PeResource r) => jsObject({
  'resource': r.resource.toJS,
  'canGet': r.canGet.toJS,
  'canSet': r.canSet.name.toJS,
  'canSubscribe': r.canSubscribe.toJS,
  'requireResId': r.requireResId.toJS,
  'fcOnGet': r.fcOnGet.toJS,
  'fcOnSet': r.fcOnSet.toJS,
  'encodings': [for (final e in r.encodings) _encoding(e)].toJS,
  'canPaginate': r.canPaginate.toJS,
  'mediaType': r.mediaType?.toJS,
  'schema': switch (r.schema) {
    final s? => jsBytes(s),
    null => null,
  },
  'writeDefaults': r.writeDefaults.toJS,
});

T? _enum<T extends Enum>(List<T> values, JSAny? a) {
  final name = stringOrNull(a);
  return name == null ? null : values.byName(name);
}

PeEvent? _event(JSAny? a) {
  if (a == null) {
    return null;
  }
  final o = a as JSObject;
  final encoding = stringOrNull(prop(o, 'encoding'));
  return newPeEvent(
    kind: PeEventKind.values.byName(prop<JSString>(o, 'kind').toDart),
    muid: intOrNull(prop(o, 'muid')),
    requestId: intOrNull(prop(o, 'requestId')),
    role: _enum(PeRole.values, prop(o, 'role')),
    txn: _enum(PeTxnKind.values, prop(o, 'txn')),
    end: _enum(PeChunkEnd.values, prop(o, 'end')),
    encoding: encoding == null
        ? null
        : PeEncoding.values[_encodingNames.indexOf(encoding)],
    command: _enum(PeCommand.values, prop(o, 'command')),
    status: intOrNull(prop(o, 'status')),
    chunk: intOrNull(prop(o, 'chunk')),
    resourceIndex: intOrNull(prop(o, 'resourceIndex')),
    subscription: intOrNull(prop(o, 'subscription')),
    started: intOrNull(prop(o, 'started')),
    startError: _enum(PeStartError.values, prop(o, 'startError')),
    reason: _enum(PeCloseReason.values, prop(o, 'reason')),
    nakCode: intOrNull(prop(o, 'nakCode')),
    nakData: intOrNull(prop(o, 'nakData')),
    ended: intOrNull(prop(o, 'ended')),
    resource: stringOrNull(prop(o, 'resource')),
    resId: stringOrNull(prop(o, 'resId')),
    subscribeId: stringOrNull(prop(o, 'subscribeId')),
    header: bytes(prop(o, 'header')),
    data: bytes(prop(o, 'data')),
  );
}

PeSubscription? _subscription(JSAny? a) {
  if (a == null) {
    return null;
  }
  final o = a as JSObject;
  return newPeSubscription(
    intOf(prop(o, 'muid')),
    PeRole.values.byName(prop<JSString>(o, 'role').toDart),
    prop<JSString>(o, 'subscribeId').toDart,
    prop<JSString>(o, 'resource').toDart,
    stringOrNull(prop(o, 'resId')),
  );
}

({int requestId, bool resourceListMissing}) _started(JSObject r) => (
  requestId: intOf(prop(r, 'requestId')),
  resourceListMissing: boolOf(prop(r, 'resourceListMissing')),
);

/// The TypeScript PeEngine.
final class PeState {
  /// An engine with the Resource table [resources].
  PeState(List<PeResource> resources)
    : _o = tsNew('PeEngine', [
        [for (final r in resources) _resource(r)].toJS,
      ]);

  final JSObject _o;

  /// Gives the last event of [dev] to the engine.
  PeEvent? receive(CiState dev, int nowUs) =>
      _event(call(_o, 'receive', [dev.js, nowUs.toJS]));

  /// Runs the deadlines.
  PeEvent? poll(CiState dev, int nowUs) =>
      _event(call(_o, 'poll', [dev.js, nowUs.toJS]));

  /// Writes the next queued control message, or null.
  Uint8List? writePending(CiState dev, int nowUs) {
    final r = call<JSAny?>(_o, 'writePending', [dev.js, nowUs.toJS]);
    return r == null ? null : bytes(r);
  }

  /// Starts a reply.
  void reply(
    CiState dev,
    int muid,
    int requestId,
    int status,
    int? length,
    PeEncoding encoding,
    String? mediaType,
    int? totalCount,
    int? cacheTime,
    int nowUs,
  ) => call<JSAny?>(_o, 'reply', [
    dev.js,
    muid.toJS,
    requestId.toJS,
    _withLength({
      'status': status.toJS,
      'encoding': _encoding(encoding),
      'mediaType': mediaType?.toJS,
      'totalCount': totalCount?.toJS,
      'cacheTime': cacheTime?.toJS,
      'nowUs': nowUs.toJS,
    }, length),
  ]);

  /// Replies with a status and no Property Data.
  void replyStatus(
    CiState dev,
    int muid,
    int requestId,
    int status,
    int nowUs,
  ) => call<JSAny?>(_o, 'replyStatus', [
    dev.js,
    muid.toJS,
    requestId.toJS,
    status.toJS,
    nowUs.toJS,
  ]);

  /// Starts a subscription update.
  int update(
    CiState dev,
    int index,
    PeCommand command,
    int? length,
    PeEncoding encoding,
    int nowUs,
  ) => intOf(
    call(_o, 'update', [
      dev.js,
      index.toJS,
      command.name.toJS,
      _withLength({
        'encoding': _encoding(encoding),
        'nowUs': nowUs.toJS,
      }, length),
    ]),
  );

  /// The ResourceList JSON of the Resource table.
  Uint8List resourceList() => bytes(call(_o, 'resourceList'));

  JSObject _header(PeHeader h, PeResource? caps, int nowUs) => _withLength({
    'resId': h.resId?.toJS,
    'mediaType': h.mediaType?.toJS,
    'offset': h.offset?.toJS,
    'limit': h.limit?.toJS,
    'setPartial': h.setPartial.toJS,
    'encoding': _encoding(h.encoding),
    'caps': caps == null ? null : _resource(caps),
    'nowUs': nowUs.toJS,
  }, h.length);

  /// Starts a Get or Set inquiry.
  ({int requestId, bool resourceListMissing}) request(
    CiState dev,
    int muid,
    PeTxnKind kind,
    PeHeader header,
    PeResource? caps,
    int nowUs,
  ) => _started(
    call(_o, 'request', [
      dev.js,
      muid.toJS,
      kind.name.toJS,
      header.resource.toJS,
      _header(header, caps, nowUs),
    ]),
  );

  /// Starts and writes the Get of the ResourceList of [muid].
  ({int requestId, Uint8List message}) requestResourceList(
    CiState dev,
    int muid,
    int nowUs,
  ) {
    final r = call<JSObject>(_o, 'requestResourceList', [
      dev.js,
      muid.toJS,
      nowUs.toJS,
    ]);
    return (
      requestId: intOf(prop(r, 'requestId')),
      message: bytes(prop(r, 'message')),
    );
  }

  /// Starts a "start" Subscription message.
  ({int requestId, bool resourceListMissing}) subscribe(
    CiState dev,
    int muid,
    PeHeader header,
    PeResource? caps,
    int nowUs,
  ) => _started(
    call(_o, 'subscribe', [
      dev.js,
      muid.toJS,
      header.resource.toJS,
      jsObject({
        'resId': header.resId?.toJS,
        'encoding': _encoding(header.encoding),
        'caps': caps == null ? null : _resource(caps),
        'nowUs': nowUs.toJS,
      }),
    ]),
  );

  /// Starts an "end" Subscription message.
  int unsubscribe(CiState dev, int index, int nowUs) =>
      intOf(call(_o, 'unsubscribe', [dev.js, index.toJS, nowUs.toJS]));

  /// Asks again for the Property Data of a subscription.
  void refresh(int index) => call<JSAny?>(_o, 'refresh', [index.toJS]);

  /// The data room of the next Chunk.
  int dataRoom(CiState dev, int muid, int requestId, PeRole role) => intOf(
    call(_o, 'dataRoom', [dev.js, muid.toJS, requestId.toJS, role.name.toJS]),
  );

  /// Writes the next Chunk.
  Uint8List send(
    CiState dev,
    int muid,
    int requestId,
    PeRole role,
    List<int> part,
    bool last,
    int nowUs,
  ) => bytes(
    call(_o, 'send', [
      dev.js,
      muid.toJS,
      requestId.toJS,
      role.name.toJS,
      jsBytes(part),
      last.toJS,
      nowUs.toJS,
    ]),
  );

  /// Queues an ACK that asks the peer to wait.
  void wait(int muid, int requestId, PeRole role, int ms) => call<JSAny?>(
    _o,
    'wait',
    [muid.toJS, requestId.toJS, role.name.toJS, ms.toJS],
  );

  /// Ends a transaction with a NAK.
  void abort(CiState dev, int muid, int requestId, PeRole role) => call<JSAny?>(
    _o,
    'abort',
    [dev.js, muid.toJS, requestId.toJS, role.name.toJS],
  );

  /// Asks again for a corrupt Chunk.
  void corrupt(int muid, int requestId, PeRole role) =>
      call<JSAny?>(_o, 'corrupt', [muid.toJS, requestId.toJS, role.name.toJS]);

  /// The subscription entry [index], or null.
  PeSubscription? subscription(int index) =>
      _subscription(call(_o, 'subscription', [index.toJS]));

  /// The next subscription that an Invalidate MUID removed, or null.
  PeSubscription? takeEnded() => _subscription(call(_o, 'takeEnded'));

  /// Frees the WASM memory.
  void free() => call<JSAny?>(_o, 'free');
}
