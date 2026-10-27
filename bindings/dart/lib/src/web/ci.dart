/// The web form of the MIDI-CI Device: the TypeScript package through dart:js_interop. The values
/// are checked by `ci.dart`.
library;

import 'dart:js_interop';
import 'dart:typed_data';

import '../ci.dart';
import 'runtime.dart';

CiReport _report(JSObject r) => CiReport(
  dataControl: intOf(prop(r, 'dataControl')),
  system: intOf(prop(r, 'system')),
  channel: intOf(prop(r, 'channel')),
  note: intOf(prop(r, 'note')),
);

JSObject _jsReport(CiReport r) => jsObject({
  'dataControl': r.dataControl.toJS,
  'system': r.system.toJS,
  'channel': r.channel.toJS,
  'note': r.note.toJS,
});

List<Uint8List> _byteLists(JSAny? a) => [
  for (final b in (a as JSArray<JSUint8Array>).toDart) bytes(b),
];

CiProfileEvent? _profile(JSAny? a) {
  if (a == null) {
    return null;
  }
  final o = a as JSObject;
  final report = stringOrNull(prop(o, 'report'));
  return newCiProfileEvent(
    deviceId: intOf(prop(o, 'deviceId')),
    group: intOf(prop(o, 'group')),
    profile: bytes(prop(o, 'profile')),
    report: report == null ? null : CiProfileReport.values.byName(report),
    on: boolOf(prop(o, 'on')),
    channels: intOf(prop(o, 'channels')),
    target: intOf(prop(o, 'target')),
    notification: boolOf(prop(o, 'notification')),
    enabledProfiles: _byteLists(prop(o, 'enabledProfiles')),
    disabledProfiles: _byteLists(prop(o, 'disabledProfiles')),
    data: bytes(prop(o, 'data')),
  );
}

CiProcessEvent? _process(JSAny? a) {
  if (a == null) {
    return null;
  }
  final o = a as JSObject;
  return newCiProcessEvent(
    deviceId: intOf(prop(o, 'deviceId')),
    group: intOf(prop(o, 'group')),
    midiMessageReport: boolOf(prop(o, 'midiMessageReport')),
    report: _report(prop<JSObject>(o, 'report')),
  );
}

/// The CI event of a TypeScript event object, or null.
CiEvent? ciEvent(JSAny? a) {
  if (a == null) {
    return null;
  }
  final o = a as JSObject;
  return newCiEvent(
    kind: CiEventKind.values.byName(prop<JSString>(o, 'kind').toDart),
    muid: intOrNull(prop(o, 'muid')),
    requestId: intOrNull(prop(o, 'requestId')),
    chunkNumber: intOrNull(prop(o, 'chunkNumber')),
    chunkTotal: intOrNull(prop(o, 'chunkTotal')),
    resource: stringOrNull(prop(o, 'resource')),
    resId: stringOrNull(prop(o, 'resId')),
    header: bytes(prop(o, 'header')),
    data: bytes(prop(o, 'data')),
    profile: _profile(prop(o, 'profile')),
    process: _process(prop(o, 'process')),
  );
}

List<CiMessage> _messages(JSAny? a) => [
  for (final m in (a as JSArray<JSObject>).toDart)
    (group: intOf(prop(m, 'group')), message: bytes(prop(m, 'message'))),
];

({int requestId, List<Uint8List> chunks}) _request(JSObject r) => (
  requestId: intOf(prop(r, 'requestId')),
  chunks: _byteLists(prop(r, 'chunks')),
);

/// The TypeScript CiDevice.
final class CiState {
  /// A Device of the checked configuration [cfg].
  CiState(CiConfig cfg)
    : js = tsNew('CiDevice', [
        jsObject({
          // The TypeScript package takes a 1-byte ID as 1 byte.
          'manufacturer': jsBytes(
            cfg.manufacturer[0] != 0 ? [cfg.manufacturer[0]] : cfg.manufacturer,
          ),
          'family': cfg.family.toJS,
          'model': cfg.model.toJS,
          'revision': jsBytes(cfg.revision),
          'maxSysexSize': cfg.maxSysexSize.toJS,
          'functionBlock': cfg.functionBlock?.toJS,
          'firstGroup': cfg.firstGroup.toJS,
          'groupCount': cfg.groupCount.toJS,
          'channelsInUse': cfg.channelsInUse.toJS,
          'productInstanceId': cfg.productInstanceId?.toJS,
          'profiles': cfg.profiles.toJS,
          'propertyExchange': cfg.propertyExchange.toJS,
          'processInquiry': cfg.processInquiry.toJS,
          'sysexFraming': cfg.sysexFraming.toJS,
          'resourceList': jsBytes(cfg.resourceList),
          'seed': cfg.seed.toJS,
        }),
      ]);

  /// The TypeScript CiDevice, for the Property Exchange engine.
  final JSObject js;

  /// The MUID of the Device.
  int get muid => intOf(prop(js, 'muid'));

  /// Reads one MIDI-CI message.
  ({List<CiMessage> replies, CiEvent? event}) receive(
    int group,
    List<int> message,
    int nowUs,
  ) {
    final r = call<JSObject>(js, 'receive', [
      group.toJS,
      jsBytes(message),
      nowUs.toJS,
    ]);
    return (
      replies: _messages(prop(r, 'replies')),
      event: ciEvent(prop(r, 'event')),
    );
  }

  /// Runs the timers.
  CiEvent? poll(int nowUs) => ciEvent(call(js, 'poll', [nowUs.toJS]));

  /// The Discovery message.
  Uint8List discovery(int outputPathId, int nowUs) =>
      bytes(call(js, 'discovery', [outputPathId.toJS, nowUs.toJS]));

  /// The Inquiry: Endpoint Information message.
  Uint8List endpointInfoInquiry(int peer) =>
      bytes(call(js, 'endpointInfoInquiry', [peer.toJS]));

  /// The Inquiry: Property Exchange Capabilities message.
  Uint8List peCapabilitiesInquiry(int peer) =>
      bytes(call(js, 'peCapabilitiesInquiry', [peer.toJS]));

  /// An Inquiry: Get Property Data.
  ({int requestId, List<Uint8List> chunks}) peGet(int peer, List<int> header) =>
      _request(call(js, 'peGet', [peer.toJS, jsBytes(header)]));

  /// An Inquiry: Set Property Data.
  ({int requestId, List<Uint8List> chunks}) peSet(
    int peer,
    List<int> header,
    List<int> data,
  ) => _request(call(js, 'peSet', [peer.toJS, jsBytes(header), jsBytes(data)]));

  /// Frees a Request ID.
  void peEnd(int peer, int requestId) =>
      call<JSAny?>(js, 'peEnd', [peer.toJS, requestId.toJS]);

  /// Answers the pending Property Exchange request.
  List<Uint8List> peReply(int status, List<int> data) =>
      _byteLists(call(js, 'peReply', [status.toJS, jsBytes(data)]));

  /// Adds a local Profile.
  List<CiMessage> addProfile(
    int deviceId,
    List<int> profile,
    int group,
    bool enabled,
    int channels,
    int maxChannels,
  ) => _messages(
    call(js, 'addProfile', [
      jsObject({
        'deviceId': deviceId.toJS,
        'group': group.toJS,
        'profile': jsBytes(profile),
        'enabled': enabled.toJS,
        'channels': channels.toJS,
        'maxChannels': maxChannels.toJS,
      }),
    ]),
  );

  /// Removes a local Profile.
  List<CiMessage> removeProfile(int deviceId, List<int> profile, int group) =>
      _messages(
        call(js, 'removeProfile', [
          deviceId.toJS,
          group.toJS,
          jsBytes(profile),
        ]),
      );

  /// Sets the state of a local Profile.
  List<CiMessage> setProfileState(
    int deviceId,
    List<int> profile,
    int group,
    bool enabled,
    int channels,
  ) {
    final m = call<JSAny?>(js, 'setProfileState', [
      deviceId.toJS,
      group.toJS,
      jsBytes(profile),
      enabled.toJS,
      channels.toJS,
    ]);
    return m == null ? const [] : _messages([m].toJS);
  }

  /// The Reply to Profile Details Inquiry.
  Uint8List profileDetailsReply(
    int peer,
    int deviceId,
    List<int> profile,
    int target,
    List<int> data,
    int group,
  ) => bytes(
    call(js, 'profileDetailsReply', [
      peer.toJS,
      group.toJS,
      deviceId.toJS,
      jsBytes(profile),
      target.toJS,
      jsBytes(data),
    ]),
  );

  /// The Profile Specific Data message.
  Uint8List profileSpecificData(
    int peer,
    int deviceId,
    List<int> profile,
    List<int> data,
    int group,
  ) => bytes(
    call(js, 'profileSpecificData', [
      peer.toJS,
      group.toJS,
      deviceId.toJS,
      jsBytes(profile),
      jsBytes(data),
    ]),
  );

  /// The Profile Inquiry message.
  Uint8List profileInquiry(int peer, int deviceId, int group) =>
      bytes(call(js, 'profileInquiry', [peer.toJS, group.toJS, deviceId.toJS]));

  /// The Set Profile On message.
  Uint8List setProfileOn(
    int peer,
    int deviceId,
    List<int> profile,
    int group,
    int channels,
    int maxChannels,
  ) => bytes(
    call(js, 'setProfileOn', [
      peer.toJS,
      group.toJS,
      deviceId.toJS,
      jsBytes(profile),
      channels.toJS,
      maxChannels.toJS,
    ]),
  );

  /// The Set Profile Off message.
  Uint8List setProfileOff(
    int peer,
    int deviceId,
    List<int> profile,
    int group,
  ) => bytes(
    call(js, 'setProfileOff', [
      peer.toJS,
      group.toJS,
      deviceId.toJS,
      jsBytes(profile),
    ]),
  );

  /// The Profile Details Inquiry message.
  Uint8List profileDetailsInquiry(
    int peer,
    int deviceId,
    List<int> profile,
    int target,
    int group,
  ) => bytes(
    call(js, 'profileDetailsInquiry', [
      peer.toJS,
      group.toJS,
      deviceId.toJS,
      jsBytes(profile),
      target.toJS,
    ]),
  );

  /// The Inquiry: Process Inquiry Capabilities message.
  Uint8List piCapabilitiesInquiry(int peer) =>
      bytes(call(js, 'piCapabilitiesInquiry', [peer.toJS]));

  /// The Inquiry: MIDI Message Report message.
  Uint8List messageReportInquiry(
    int peer,
    CiReport inquiry,
    int deviceId,
    int group,
  ) => bytes(
    call(js, 'messageReportInquiry', [
      peer.toJS,
      group.toJS,
      deviceId.toJS,
      _jsReport(inquiry),
    ]),
  );

  /// True while a MIDI Message Report from [peer] is open.
  bool reportOpen(int peer) => boolOf(call(js, 'reportOpen', [peer.toJS]));

  /// The Reply to MIDI Message Report.
  Uint8List messageReportReply(
    int peer,
    CiReport requested,
    CiReport supported,
    int deviceId,
    int group,
  ) => bytes(
    call(js, 'messageReportReply', [
      peer.toJS,
      group.toJS,
      deviceId.toJS,
      _jsReport(requested),
      _jsReport(supported),
    ]),
  );

  /// The End of MIDI Message Report message.
  Uint8List messageReportEnd(int peer, int deviceId, int group) => bytes(
    call(js, 'messageReportEnd', [peer.toJS, group.toJS, deviceId.toJS]),
  );

  /// The NAK that stops a MIDI Message Report.
  Uint8List abortReport(int peer, int deviceId, int group, bool error) => bytes(
    call(js, 'abortReport', [peer.toJS, group.toJS, deviceId.toJS, error.toJS]),
  );

  /// Frees the WASM memory.
  void free() => call<JSAny?>(js, 'free');
}
