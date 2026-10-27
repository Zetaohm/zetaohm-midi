/// The native form of the Property Exchange engine: zmidi-ci-pe through dart:ffi and the C ABI.
/// The values are checked by `pe.dart`.
library;

import 'dart:convert';
import 'dart:ffi';
import 'dart:typed_data';

import 'package:ffi/ffi.dart';

import '../bindings.g.dart';
import '../errors.dart';
import '../native.dart';
import '../pe.dart';
import 'ci.dart';

/// ZmidiPeError, by value.
const _errors = [
  'none',
  'device refused',
  'write failed',
  'header refused',
  'subscription rule refused',
  'no such transaction',
  'no free transaction slot',
  'peer has its maximum of open requests',
  'deadline passed',
  'not permitted in this state',
  'refused by the ResourceList of the peer',
  'mediaType needed',
  'totalCount needed',
  'header too large',
  'buffer size',
  'pending queue full',
  'invalid wait',
  'invalid \$ref',
];

/// SIZE_MAX of the platform: a length that is not known.
final int _unknown = sizeOf<UintPtr>() == 8 ? -1 : 0xffffffff;

/// One control message or Chunk header and more.
const _messageMax = 512;

String _cText(Array<Uint8> a) {
  final b = <int>[];
  for (var i = 0; i < ZMIDI_PE_ID_LEN && a[i] != 0; i++) {
    b.add(a[i]);
  }
  return utf8.decode(b);
}

String? _orNull(String s) => s.isEmpty ? null : s;

int _length(int? len) => len ?? _unknown;

Pointer<Uint8> _cString(Arena arena, String s) {
  final b = utf8.encode(s);
  final p = arena<Uint8>(b.length + 1);
  p.asTypedList(b.length + 1)
    ..setAll(0, b)
    ..[b.length] = 0;
  return p;
}

int _encodings(List<PeEncoding> e) =>
    e.fold(0, (bits, x) => bits | (1 << x.index));

/// Writes the ZmidiPeResource of [r] at [p]; its texts go into [arena].
void _writeResource(Arena arena, Pointer<ZmidiPeResource> p, PeResource r) {
  final schema = r.schema;
  p.ref
    ..resource = _cString(arena, r.resource)
    ..can_get = r.canGet
    ..can_set = r.canSet.index
    ..can_subscribe = r.canSubscribe
    ..require_res_id = r.requireResId
    ..fc_on_get = r.fcOnGet
    ..fc_on_set = r.fcOnSet
    ..can_paginate = r.canPaginate
    ..write_defaults = r.writeDefaults
    ..encodings = _encodings(r.encodings)
    ..media_type = r.mediaType == null ? nullptr : _cString(arena, r.mediaType!)
    ..schema = schema == null ? nullptr : bytesIn(arena, schema)
    ..schema_len = schema?.length ?? 0;
}

PeEvent? _event(Pointer<ZmidiPeEvent> p) {
  final e = p.ref;
  final code = e.kindAsInt;
  if (code == 0) {
    return null;
  }
  final kind = PeEventKind.values[code - 1];
  final message = kind == PeEventKind.message;
  final txnLike =
      kind != PeEventKind.subscriptionStarted &&
      kind != PeEventKind.invalidated;
  final reason = e.reasonAsInt == 0
      ? null
      : PeCloseReason.values[e.reasonAsInt - 1];
  int? index(int v) => v < 0 ? null : v;
  return newPeEvent(
    kind: kind,
    muid: kind == PeEventKind.subscriptionStarted ? null : e.muid,
    requestId: txnLike && kind != PeEventKind.subscriptionEnded
        ? e.request_id
        : null,
    role: txnLike ? PeRole.values[e.roleAsInt] : null,
    txn: message || kind == PeEventKind.subscriptionEnded
        ? PeTxnKind.values[e.txnAsInt]
        : null,
    end: message ? PeChunkEnd.values[e.endAsInt] : null,
    encoding: message ? PeEncoding.values[e.encodingAsInt] : null,
    command: e.commandAsInt == 0 ? null : PeCommand.values[e.commandAsInt - 1],
    status: (message || reason == PeCloseReason.refused) && e.status != 0
        ? e.status
        : null,
    chunk: kind == PeEventKind.resend ? e.chunk : null,
    resourceIndex: index(e.resource_index),
    subscription: index(e.subscription),
    started: index(e.started),
    startError: e.start_errorAsInt == 0
        ? null
        : PeStartError.values[e.start_errorAsInt - 1],
    reason: reason,
    nakCode: reason == PeCloseReason.nak ? e.nak_code : null,
    nakData: reason == PeCloseReason.nak ? e.nak_data : null,
    ended: kind == PeEventKind.invalidated ? e.ended : null,
    resource: _orNull(_cText(e.resource)),
    resId: _orNull(_cText(e.res_id)),
    subscribeId: _orNull(_cText(e.subscribe_id)),
    header: bytesOut(e.header, e.header_len),
    data: bytesOut(e.data, e.data_len),
  );
}

PeSubscription _subscription(Pointer<ZmidiPeSubscription> p) {
  final s = p.ref;
  return newPeSubscription(
    s.muid,
    PeRole.values[s.roleAsInt],
    _cText(s.subscribe_id),
    _cText(s.resource),
    _orNull(_cText(s.res_id)),
  );
}

/// The C state of a PeEngine.
final class PeState implements Finalizable {
  /// An engine with the Resource table [resources].
  PeState(List<PeResource> resources) {
    // One block: the C state, then the texts of the table that the engine borrows.
    final texts = Arena(calloc);
    try {
      final table = texts<ZmidiPeResource>(
        resources.isEmpty ? 1 : resources.length,
      );
      for (var i = 0; i < resources.length; i++) {
        _writeResource(texts, table + i, resources[i]);
      }
      _p = allocState(this, zmidi_pe_size());
      final st = zmidi_pe_init(_p, table, resources.length);
      if (st != ZmidiStatus.ZMIDI_STATUS_OK) {
        texts.releaseAll();
        _check(st, 'PeEngine');
      }
      _texts = texts;
    } catch (_) {
      texts.releaseAll();
      rethrow;
    }
  }

  late Pointer<ZmidiPe> _p;
  Arena? _texts;

  /// Gives the last event of [dev] to the engine.
  PeEvent? receive(CiState dev, int nowUs) {
    for (var cap = 1024; ; cap *= 2) {
      final r = using((arena) {
        final scratch = arena<Uint8>(cap);
        final ev = arena<ZmidiPeEvent>();
        final st = zmidi_pe_receive(_p, dev.p, nowUs, scratch, cap, ev);
        if (st == ZmidiStatus.ZMIDI_STATUS_BUFFER_TOO_SMALL && cap < 0x10000) {
          return null;
        }
        _check(st, 'receive');
        return (_event(ev),);
      });
      if (r != null) {
        return r.$1;
      }
    }
  }

  /// Runs the deadlines.
  PeEvent? poll(CiState dev, int nowUs) => using((arena) {
    final ev = arena<ZmidiPeEvent>();
    _check(zmidi_pe_poll(_p, dev.p, nowUs, ev), 'poll');
    return _event(ev);
  });

  /// Writes the next queued control message, or null.
  Uint8List? writePending(CiState dev, int nowUs) => using((arena) {
    final out = arena<Uint8>(_messageMax);
    final len = arena<UintPtr>();
    final st = zmidi_pe_write_pending(_p, dev.p, nowUs, out, _messageMax, len);
    if (st == ZmidiStatus.ZMIDI_STATUS_NO_PENDING) {
      return null;
    }
    _check(st, 'writePending');
    return records(out, len.value).first.$2;
  });

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
  ) => using((arena) {
    final mt = mediaType == null ? nullptr : _cString(arena, mediaType);
    _check(
      zmidi_pe_reply(
        _p,
        dev.p,
        muid,
        requestId,
        status,
        encoding.index,
        mt,
        totalCount ?? -1,
        cacheTime ?? -1,
        _length(length),
        nowUs,
      ),
      'reply',
    );
  });

  /// Replies with a status and no Property Data.
  void replyStatus(
    CiState dev,
    int muid,
    int requestId,
    int status,
    int nowUs,
  ) => _check(
    zmidi_pe_reply_status(_p, dev.p, muid, requestId, status, nowUs),
    'replyStatus',
  );

  /// Starts a subscription update.
  int update(
    CiState dev,
    int index,
    PeCommand command,
    int? length,
    PeEncoding encoding,
    int nowUs,
  ) => using((arena) {
    final id = arena<Uint8>();
    _check(
      zmidi_pe_update(
        _p,
        dev.p,
        index,
        command.index + 1,
        encoding.index,
        _length(length),
        nowUs,
        id,
      ),
      'update',
    );
    return id.value;
  });

  /// The ResourceList JSON of the Resource table.
  Uint8List resourceList() {
    for (var cap = 1024; ; cap *= 2) {
      final r = using((arena) {
        final out = arena<Uint8>(cap);
        final len = arena<UintPtr>();
        final st = zmidi_pe_resource_list(_p, out, cap, len);
        if (st == ZmidiStatus.ZMIDI_STATUS_BUFFER_TOO_SMALL) {
          return null;
        }
        _check(st, 'resourceList');
        return bytesOut(out, len.value);
      });
      if (r != null) {
        return r;
      }
    }
  }

  /// Starts a Get or Set inquiry.
  ({int requestId, bool resourceListMissing}) request(
    CiState dev,
    int muid,
    PeTxnKind kind,
    PeHeader header,
    PeResource? caps,
    int nowUs,
  ) => using((arena) {
    final h = _header(arena, header);
    final c = _caps(arena, caps);
    final id = arena<Uint8>();
    final missing = arena<Bool>();
    _check(
      zmidi_pe_request(_p, dev.p, muid, kind.index, h, c, nowUs, id, missing),
      'request',
    );
    return (requestId: id.value, resourceListMissing: missing.value);
  });

  /// Starts and writes the Get of the ResourceList of [muid].
  ({int requestId, Uint8List message}) requestResourceList(
    CiState dev,
    int muid,
    int nowUs,
  ) => using((arena) {
    final out = arena<Uint8>(_messageMax);
    final len = arena<UintPtr>();
    final id = arena<Uint8>();
    _check(
      zmidi_pe_request_resource_list(
        _p,
        dev.p,
        muid,
        nowUs,
        out,
        _messageMax,
        len,
        id,
      ),
      'requestResourceList',
    );
    return (requestId: id.value, message: records(out, len.value).first.$2);
  });

  /// Starts a "start" Subscription message.
  ({int requestId, bool resourceListMissing}) subscribe(
    CiState dev,
    int muid,
    PeHeader header,
    PeResource? caps,
    int nowUs,
  ) => using((arena) {
    final h = _header(arena, header);
    final c = _caps(arena, caps);
    final id = arena<Uint8>();
    final missing = arena<Bool>();
    _check(
      zmidi_pe_subscribe(_p, dev.p, muid, h, c, nowUs, id, missing),
      'subscribe',
    );
    return (requestId: id.value, resourceListMissing: missing.value);
  });

  /// Starts an "end" Subscription message.
  int unsubscribe(CiState dev, int index, int nowUs) => using((arena) {
    final id = arena<Uint8>();
    _check(zmidi_pe_unsubscribe(_p, dev.p, index, nowUs, id), 'unsubscribe');
    return id.value;
  });

  /// Asks again for the Property Data of a subscription.
  void refresh(int index) => _check(zmidi_pe_refresh(_p, index), 'refresh');

  /// The data room of the next Chunk.
  int dataRoom(CiState dev, int muid, int requestId, PeRole role) =>
      using((arena) {
        final room = arena<UintPtr>();
        _check(
          zmidi_pe_data_room(_p, dev.p, muid, requestId, role.index, room),
          'dataRoom',
        );
        return room.value;
      });

  /// Writes the next Chunk.
  Uint8List send(
    CiState dev,
    int muid,
    int requestId,
    PeRole role,
    List<int> part,
    bool last,
    int nowUs,
  ) => using((arena) {
    final pd = bytesIn(arena, part);
    final cap = _messageMax + part.length;
    final out = arena<Uint8>(cap);
    final len = arena<UintPtr>();
    _check(
      zmidi_pe_send(
        _p,
        dev.p,
        muid,
        requestId,
        role.index,
        pd,
        part.length,
        last,
        nowUs,
        out,
        cap,
        len,
      ),
      'send',
    );
    return records(out, len.value).first.$2;
  });

  /// Queues an ACK that asks the peer to wait.
  void wait(int muid, int requestId, PeRole role, int ms) =>
      _check(zmidi_pe_wait(_p, muid, requestId, role.index, ms), 'wait');

  /// Ends a transaction with a NAK.
  void abort(CiState dev, int muid, int requestId, PeRole role) =>
      _check(zmidi_pe_abort(_p, dev.p, muid, requestId, role.index), 'abort');

  /// Asks again for a corrupt Chunk.
  void corrupt(int muid, int requestId, PeRole role) =>
      _check(zmidi_pe_corrupt(_p, muid, requestId, role.index), 'corrupt');

  /// The subscription entry [index], or null.
  PeSubscription? subscription(int index) => using((arena) {
    final out = arena<ZmidiPeSubscription>();
    final st = zmidi_pe_subscription(_p, index, out);
    if (st == ZmidiStatus.ZMIDI_STATUS_NO_PENDING) {
      return null;
    }
    _check(st, 'subscription');
    return _subscription(out);
  });

  /// The next subscription that an Invalidate MUID removed, or null.
  PeSubscription? takeEnded() => using((arena) {
    final out = arena<ZmidiPeSubscription>();
    final st = zmidi_pe_take_ended(_p, out);
    if (st == ZmidiStatus.ZMIDI_STATUS_NO_PENDING) {
      return null;
    }
    _check(st, 'takeEnded');
    return _subscription(out);
  });

  /// Frees the C state and the texts of the table.
  void free() {
    if (_p != nullptr) {
      releaseState(this, _p);
      _p = nullptr;
      _texts?.releaseAll();
      _texts = null;
    }
  }

  /// Throws the exception of a status that is not OK, with the reason of the engine.
  void _check(ZmidiStatus status, String what) {
    if (status == ZmidiStatus.ZMIDI_STATUS_OK) {
      return;
    }
    final reason = using((arena) {
      final st = arena<Uint16>();
      final e = zmidi_pe_last_error(_p, st);
      final text = _errors[e.value];
      return e == ZmidiPeError.ZMIDI_PE_ERROR_REFUSED
          ? '$text: status ${st.value}'
          : text;
    });
    switch (status) {
      case ZmidiStatus.ZMIDI_STATUS_INVALID_ARGUMENT:
        throw InvalidValueException(
          '$what: ${reason == 'none' ? 'a value is out of its range' : reason}',
        );
      case ZmidiStatus.ZMIDI_STATUS_BUFFER_TOO_SMALL:
        if (reason == 'buffer size') {
          throw InvalidValueException(
            '$what: a part larger than the data room',
          );
        }
        throw MidiException('$what: buffer too small');
      case ZmidiStatus.ZMIDI_STATUS_NO_PENDING:
      case ZmidiStatus.ZMIDI_STATUS_REJECTED:
        throw RejectedException('$what: $reason');
      case ZmidiStatus.ZMIDI_STATUS_NOT_SUPPORTED:
        throw RejectedException('$what: not supported by this build');
      case ZmidiStatus.ZMIDI_STATUS_OK:
        return;
    }
  }

  Pointer<ZmidiPeHeader> _header(Arena arena, PeHeader x) {
    final h = arena<ZmidiPeHeader>();
    h.ref
      ..resource = _cString(arena, x.resource)
      ..res_id = x.resId == null ? nullptr : _cString(arena, x.resId!)
      ..media_type = x.mediaType == null
          ? nullptr
          : _cString(arena, x.mediaType!)
      ..has_offset = x.offset != null && x.limit != null
      ..offset = x.offset ?? 0
      ..limit = x.limit ?? 0
      ..set_partial = x.setPartial
      ..encoding = x.encoding.index
      ..length = _length(x.length);
    return h;
  }

  Pointer<ZmidiPeResource> _caps(Arena arena, PeResource? caps) {
    if (caps == null) {
      return nullptr;
    }
    final c = arena<ZmidiPeResource>();
    _writeResource(arena, c, caps);
    return c;
  }
}
