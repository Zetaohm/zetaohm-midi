/// The native form of the MIDI-CI Device: zmidi-ci and zmidi-ci-pe through dart:ffi and the C ABI.
/// The values are checked by `ci.dart`.
library;

import 'dart:convert';
import 'dart:ffi';
import 'dart:typed_data';

import 'package:ffi/ffi.dart';

import '../bindings.g.dart';
import '../ci.dart';
import '../native.dart';

String? _cString(Array<Uint8> a) {
  final b = <int>[];
  for (var i = 0; i < ZMIDI_PE_ID_LEN && a[i] != 0; i++) {
    b.add(a[i]);
  }
  return b.isEmpty ? null : utf8.decode(b);
}

CiEvent? _event(
  ZmidiCiEvent e, {
  CiProfileEvent? profile,
  CiProcessEvent? process,
}) {
  final code = e.kindAsInt;
  if (code == 0 || code > CiEventKind.values.length) {
    return null;
  }
  final kind = CiEventKind.values[code - 1];
  final pe = code >= 15 && code <= 21;
  final inquiry = kind == CiEventKind.peGet || kind == CiEventKind.peSet;
  return newCiEvent(
    kind: kind,
    muid: kind == CiEventKind.discoveryEnded || kind == CiEventKind.rejected
        ? null
        : e.muid,
    requestId: pe ? e.request_id : null,
    chunkNumber: pe ? e.chunk_number : null,
    chunkTotal: pe ? e.chunk_total : null,
    resource: inquiry ? _cString(e.resource) : null,
    resId: inquiry ? _cString(e.res_id) : null,
    header: pe ? bytesOut(e.header, e.header_len) : Uint8List(0),
    data: pe ? bytesOut(e.data, e.data_len) : Uint8List(0),
    profile: profile,
    process: process,
  );
}

/// The Profile IDs of the last Reply to Profile Inquiry of [p]: the enabled or the disabled list.
List<Uint8List> _profileIds(Pointer<ZmidiCi> p, bool enabled) => using((arena) {
  final count = arena<UintPtr>();
  final st = zmidi_ci_profile_ids(p, enabled, nullptr, 0, count);
  if (st == ZmidiStatus.ZMIDI_STATUS_OK) {
    return const <Uint8List>[];
  }
  if (st != ZmidiStatus.ZMIDI_STATUS_BUFFER_TOO_SMALL) {
    check(st, 'receive');
  }
  final n = count.value;
  final out = arena<Uint8>(5 * n);
  check(zmidi_ci_profile_ids(p, enabled, out, 5 * n, count), 'receive');
  return [for (var i = 0; i < n; i++) bytesOut(out + 5 * i, 5)];
});

/// The Profile fields of the last event of [p], or null when it is not a Profile event.
CiProfileEvent? _profileEvent(Pointer<ZmidiCi> p, CiEventKind kind) =>
    using((arena) {
      final pe = arena<ZmidiCiProfileEvent>();
      final st = zmidi_ci_profile_event(p, pe);
      if (st == ZmidiStatus.ZMIDI_STATUS_NO_PENDING) {
        return null;
      }
      check(st, 'receive');
      final r = pe.ref;
      final reply = kind == CiEventKind.profileInquiryReply;
      return newCiProfileEvent(
        deviceId: r.device_id,
        group: r.group,
        profile: Uint8List.fromList([for (var i = 0; i < 5; i++) r.profile[i]]),
        report: switch (r.report) {
          ZmidiCiProfileReport.ZMIDI_CI_PROFILE_REPORT_NONE => null,
          ZmidiCiProfileReport.ZMIDI_CI_PROFILE_REPORT_ADDED =>
            CiProfileReport.added,
          ZmidiCiProfileReport.ZMIDI_CI_PROFILE_REPORT_REMOVED =>
            CiProfileReport.removed,
          ZmidiCiProfileReport.ZMIDI_CI_PROFILE_REPORT_ENABLED =>
            CiProfileReport.enabled,
          ZmidiCiProfileReport.ZMIDI_CI_PROFILE_REPORT_DISABLED =>
            CiProfileReport.disabled,
        },
        on: r.on,
        channels: r.channels,
        target: r.target,
        notification: r.notification,
        enabledProfiles: reply ? _profileIds(p, true) : const [],
        disabledProfiles: reply ? _profileIds(p, false) : const [],
        data: bytesOut(r.data, r.data_len),
      );
    });

void _writeReport(ZmidiCiReport r, CiReport x) {
  r
    ..data_control = x.dataControl
    ..system = x.system
    ..channel = x.channel
    ..note = x.note;
}

/// The Process Inquiry fields of the last event of [p], or null when it is not a Process Inquiry
/// event.
CiProcessEvent? _processEvent(Pointer<ZmidiCi> p) => using((arena) {
  final pe = arena<ZmidiCiProcessEvent>();
  final st = zmidi_ci_process_event(p, pe);
  if (st == ZmidiStatus.ZMIDI_STATUS_NO_PENDING) {
    return null;
  }
  check(st, 'receive');
  final r = pe.ref.report;
  return newCiProcessEvent(
    deviceId: pe.ref.device_id,
    group: pe.ref.group,
    midiMessageReport: pe.ref.midi_message_report,
    report: CiReport(
      dataControl: r.data_control,
      system: r.system,
      channel: r.channel,
      note: r.note,
    ),
  );
});

/// The message buffer of a Device: a pending PE request points into it until a PE reply or the
/// next receive.
final class _Message implements Finalizable {
  _Message(this.capacity) {
    p = allocState(this, capacity);
  }

  final int capacity;
  late Pointer<Uint8> p;

  void free() {
    if (p != nullptr) {
      releaseState(this, p);
      p = nullptr;
    }
  }
}

/// The C state of a CiDevice.
final class CiState implements Finalizable {
  /// A Device of the checked configuration [cfg].
  CiState(CiConfig cfg) {
    final list = cfg.resourceList;
    final pidText = cfg.productInstanceId;
    final pid = pidText == null ? null : utf8.encode(pidText);
    _listLen = list.length;
    // One block: the C state, then the ResourceList and the Product Instance ID that it borrows.
    final cs = (zmidi_ci_size() + 15) ~/ 16 * 16;
    final size = cs + list.length + (pid?.length ?? 0) + 1;
    p = allocState(this, size);
    final pl = Pointer<Uint8>.fromAddress(p.address + cs);
    pl.asTypedList(list.length + 1).setAll(0, list);
    final pp = Pointer<Uint8>.fromAddress(pl.address + list.length);
    if (pid != null) {
      pp.asTypedList(pid.length + 1)
        ..setAll(0, pid)
        ..[pid.length] = 0;
    }
    using((arena) {
      final c = arena<ZmidiCiConfig>();
      final r = c.ref;
      for (var i = 0; i < 3; i++) {
        r.manufacturer[i] = cfg.manufacturer[i];
      }
      for (var i = 0; i < 4; i++) {
        r.revision[i] = cfg.revision[i];
      }
      r
        ..family = cfg.family
        ..model = cfg.model
        ..max_sysex_size = cfg.maxSysexSize
        ..has_function_block = cfg.functionBlock != null
        ..function_block = cfg.functionBlock ?? 0
        ..first_group = cfg.firstGroup
        ..group_count = cfg.groupCount
        ..channels_in_use = cfg.channelsInUse
        ..product_instance_id = pid == null ? nullptr : pp
        ..profiles = cfg.profiles
        ..property_exchange = cfg.propertyExchange
        ..process_inquiry = cfg.processInquiry
        ..sysex_framing = cfg.sysexFraming
        ..resource_list = pl
        ..resource_list_len = list.length;
      check(zmidi_ci_init(p, c, cfg.seed), 'CiDevice');
    });
  }

  /// The C state, for the Property Exchange engine.
  late Pointer<ZmidiCi> p;
  late final int _listLen;
  _Message? _msg;

  /// The MUID of the Device.
  int get muid => zmidi_ci_muid(p);

  /// Reads one MIDI-CI message.
  ({List<CiMessage> replies, CiEvent? event}) receive(
    int group,
    List<int> message,
    int nowUs,
  ) {
    var msg = _msg;
    if (msg == null || message.length > msg.capacity) {
      msg?.free();
      msg = _Message(message.length < 64 ? 64 : message.length);
      _msg = msg;
    }
    msg.p.asTypedList(message.length).setAll(0, message);
    return using((arena) {
      final out = arena<Uint8>(ZMIDI_CI_REPLY_BUFFER);
      final len = arena<UintPtr>();
      final ev = arena<ZmidiCiEvent>();
      check(
        zmidi_ci_receive(
          p,
          group,
          msg!.p,
          message.length,
          nowUs,
          out,
          ZMIDI_CI_REPLY_BUFFER,
          len,
          ev,
        ),
        'receive',
      );
      return (
        replies: [
          for (final (g, m) in records(out, len.value)) (group: g, message: m),
        ],
        event: _receivedEvent(ev.ref),
      );
    });
  }

  /// The event of a receive call, with its Profile or Process Inquiry fields (read while the
  /// message buffer still holds the message).
  CiEvent? _receivedEvent(ZmidiCiEvent e) {
    final code = e.kindAsInt;
    if (code == 0 || code > CiEventKind.values.length) {
      return null;
    }
    final kind = CiEventKind.values[code - 1];
    return _event(
      e,
      profile: code >= 8 && code <= 13 ? _profileEvent(p, kind) : null,
      process: code >= 22 && code <= 25 ? _processEvent(p) : null,
    );
  }

  /// Runs the timers.
  CiEvent? poll(int nowUs) => using((arena) {
    final ev = arena<ZmidiCiEvent>();
    check(zmidi_ci_poll(p, nowUs, ev), 'poll');
    return _event(ev.ref);
  });

  /// The Discovery message.
  Uint8List discovery(int outputPathId, int nowUs) => _one(
    (out, cap, len) =>
        zmidi_ci_discovery(p, outputPathId, nowUs, out, cap, len),
    'discovery',
  );

  /// The Inquiry: Endpoint Information message.
  Uint8List endpointInfoInquiry(int peer) => _one(
    (out, cap, len) => zmidi_ci_endpoint_info_inquiry(p, peer, out, cap, len),
    'endpointInfoInquiry',
  );

  /// The Inquiry: Property Exchange Capabilities message.
  Uint8List peCapabilitiesInquiry(int peer) => _one(
    (out, cap, len) => zmidi_ci_pe_capabilities_inquiry(p, peer, out, cap, len),
    'peCapabilitiesInquiry',
  );

  /// An Inquiry: Get Property Data.
  ({int requestId, List<Uint8List> chunks}) peGet(int peer, List<int> header) =>
      _request(peer, header, null);

  /// An Inquiry: Set Property Data.
  ({int requestId, List<Uint8List> chunks}) peSet(
    int peer,
    List<int> header,
    List<int> data,
  ) => _request(peer, header, data);

  /// Frees a Request ID.
  void peEnd(int peer, int requestId) =>
      check(zmidi_ci_pe_end(p, peer, requestId), 'peEnd');

  /// Answers the pending Property Exchange request.
  List<Uint8List> peReply(int status, List<int> data) {
    final need = 2 * (data.length > _listLen ? data.length : _listLen) + 64;
    return using((arena) {
      final scratch = arena<Uint8>(need);
      final out = arena<Uint8>(need);
      final len = arena<UintPtr>();
      check(
        zmidi_ci_pe_reply(
          p,
          status,
          bytesIn(arena, data),
          data.length,
          scratch,
          need,
          out,
          need,
          len,
        ),
        'peReply',
      );
      return [for (final (_, m) in records(out, len.value)) m];
    });
  }

  /// Adds a local Profile.
  List<CiMessage> addProfile(
    int deviceId,
    List<int> profile,
    int group,
    bool enabled,
    int channels,
    int maxChannels,
  ) => using((arena) {
    final e = arena<ZmidiCiProfile>();
    e.ref
      ..device_id = deviceId
      ..group = group
      ..enabled = enabled
      ..channels = channels
      ..max_channels = maxChannels;
    for (var i = 0; i < 5; i++) {
      e.ref.profile[i] = profile[i];
    }
    return _reports(
      (out, cap, len) => zmidi_ci_add_profile(p, e, out, cap, len),
      'addProfile',
    );
  });

  /// Removes a local Profile.
  List<CiMessage> removeProfile(int deviceId, List<int> profile, int group) =>
      using(
        (arena) => _reports(
          (out, cap, len) => zmidi_ci_remove_profile(
            p,
            deviceId,
            group,
            bytesIn(arena, profile),
            out,
            cap,
            len,
          ),
          'removeProfile',
        ),
      );

  /// Sets the state of a local Profile.
  List<CiMessage> setProfileState(
    int deviceId,
    List<int> profile,
    int group,
    bool enabled,
    int channels,
  ) => using(
    (arena) => _reports(
      (out, cap, len) => zmidi_ci_set_profile_state(
        p,
        deviceId,
        group,
        bytesIn(arena, profile),
        enabled,
        channels,
        out,
        cap,
        len,
      ),
      'setProfileState',
    ),
  );

  /// The Reply to Profile Details Inquiry.
  Uint8List profileDetailsReply(
    int peer,
    int deviceId,
    List<int> profile,
    int target,
    List<int> data,
    int group,
  ) => using(
    (arena) => _one(
      (out, cap, len) => zmidi_ci_profile_details_reply(
        p,
        peer,
        group,
        deviceId,
        bytesIn(arena, profile),
        target,
        bytesIn(arena, data),
        data.length,
        out,
        cap,
        len,
      ),
      'profileDetailsReply',
      128 + data.length,
    ),
  );

  /// The Profile Specific Data message.
  Uint8List profileSpecificData(
    int peer,
    int deviceId,
    List<int> profile,
    List<int> data,
    int group,
  ) => using(
    (arena) => _one(
      (out, cap, len) => zmidi_ci_profile_specific_data(
        p,
        peer,
        group,
        deviceId,
        bytesIn(arena, profile),
        bytesIn(arena, data),
        data.length,
        out,
        cap,
        len,
      ),
      'profileSpecificData',
      128 + data.length,
    ),
  );

  /// The Profile Inquiry message.
  Uint8List profileInquiry(int peer, int deviceId, int group) => _one(
    (out, cap, len) =>
        zmidi_ci_profile_inquiry(p, peer, group, deviceId, out, cap, len),
    'profileInquiry',
  );

  /// The Set Profile On message.
  Uint8List setProfileOn(
    int peer,
    int deviceId,
    List<int> profile,
    int group,
    int channels,
    int maxChannels,
  ) => using(
    (arena) => _one(
      (out, cap, len) => zmidi_ci_set_profile_on(
        p,
        peer,
        group,
        deviceId,
        bytesIn(arena, profile),
        channels,
        maxChannels,
        out,
        cap,
        len,
      ),
      'setProfileOn',
    ),
  );

  /// The Set Profile Off message.
  Uint8List setProfileOff(
    int peer,
    int deviceId,
    List<int> profile,
    int group,
  ) => using(
    (arena) => _one(
      (out, cap, len) => zmidi_ci_set_profile_off(
        p,
        peer,
        group,
        deviceId,
        bytesIn(arena, profile),
        out,
        cap,
        len,
      ),
      'setProfileOff',
    ),
  );

  /// The Profile Details Inquiry message.
  Uint8List profileDetailsInquiry(
    int peer,
    int deviceId,
    List<int> profile,
    int target,
    int group,
  ) => using(
    (arena) => _one(
      (out, cap, len) => zmidi_ci_profile_details_inquiry(
        p,
        peer,
        group,
        deviceId,
        bytesIn(arena, profile),
        target,
        out,
        cap,
        len,
      ),
      'profileDetailsInquiry',
    ),
  );

  /// The Inquiry: Process Inquiry Capabilities message.
  Uint8List piCapabilitiesInquiry(int peer) => _one(
    (out, cap, len) => zmidi_ci_pi_capabilities_inquiry(p, peer, out, cap, len),
    'piCapabilitiesInquiry',
  );

  /// The Inquiry: MIDI Message Report message.
  Uint8List messageReportInquiry(
    int peer,
    CiReport inquiry,
    int deviceId,
    int group,
  ) => using((arena) {
    final r = arena<ZmidiCiReport>();
    _writeReport(r.ref, inquiry);
    return _one(
      (out, cap, len) => zmidi_ci_message_report_inquiry(
        p,
        peer,
        group,
        deviceId,
        r,
        out,
        cap,
        len,
      ),
      'messageReportInquiry',
    );
  });

  /// True while a MIDI Message Report from [peer] is open.
  bool reportOpen(int peer) => zmidi_ci_report_open(p, peer);

  /// The Reply to MIDI Message Report.
  Uint8List messageReportReply(
    int peer,
    CiReport requested,
    CiReport supported,
    int deviceId,
    int group,
  ) => using((arena) {
    final rq = arena<ZmidiCiReport>();
    final sp = arena<ZmidiCiReport>();
    _writeReport(rq.ref, requested);
    _writeReport(sp.ref, supported);
    return _one(
      (out, cap, len) => zmidi_ci_message_report_reply(
        p,
        peer,
        group,
        deviceId,
        rq,
        sp,
        out,
        cap,
        len,
      ),
      'messageReportReply',
    );
  });

  /// The End of MIDI Message Report message.
  Uint8List messageReportEnd(int peer, int deviceId, int group) => _one(
    (out, cap, len) =>
        zmidi_ci_message_report_end(p, peer, group, deviceId, out, cap, len),
    'messageReportEnd',
  );

  /// The NAK that stops a MIDI Message Report.
  Uint8List abortReport(int peer, int deviceId, int group, bool error) => _one(
    (out, cap, len) =>
        zmidi_ci_abort_report(p, peer, group, deviceId, error, out, cap, len),
    'abortReport',
  );

  /// Frees the C state and the message buffer.
  void free() {
    if (p != nullptr) {
      releaseState(this, p);
      p = nullptr;
      _msg?.free();
      _msg = null;
    }
  }

  Uint8List _one(
    ZmidiStatus Function(Pointer<Uint8> out, int cap, Pointer<UintPtr> len)
    call,
    String what, [
    int cap = 128,
  ]) => using((arena) {
    final out = arena<Uint8>(cap);
    final len = arena<UintPtr>();
    check(call(out, cap, len), what);
    final r = records(out, len.value);
    return r.isEmpty ? Uint8List(0) : r.first.$2;
  });

  List<CiMessage> _reports(
    ZmidiStatus Function(Pointer<Uint8> out, int cap, Pointer<UintPtr> len)
    call,
    String what,
  ) => using((arena) {
    final out = arena<Uint8>(ZMIDI_CI_REPORT_BUFFER);
    final len = arena<UintPtr>();
    check(call(out, ZMIDI_CI_REPORT_BUFFER, len), what);
    return [
      for (final (g, m) in records(out, len.value)) (group: g, message: m),
    ];
  });

  ({int requestId, List<Uint8List> chunks}) _request(
    int peer,
    List<int> header,
    List<int>? data,
  ) {
    final body = data ?? const <int>[];
    var cap = 2 * (header.length + body.length) + 256;
    for (;;) {
      final r = using((arena) {
        final out = arena<Uint8>(cap);
        final len = arena<UintPtr>();
        final id = arena<Uint8>();
        final ph = bytesIn(arena, header);
        final st = data == null
            ? zmidi_ci_pe_get(p, peer, ph, header.length, out, cap, len, id)
            : zmidi_ci_pe_set(
                p,
                peer,
                ph,
                header.length,
                bytesIn(arena, body),
                body.length,
                out,
                cap,
                len,
                id,
              );
        if (st == ZmidiStatus.ZMIDI_STATUS_BUFFER_TOO_SMALL) {
          return null;
        }
        check(st, data == null ? 'peGet' : 'peSet');
        return (
          requestId: id.value,
          chunks: [for (final (_, m) in records(out, len.value)) m],
        );
      });
      if (r != null) {
        return r;
      }
      cap *= 2;
    }
  }
}
