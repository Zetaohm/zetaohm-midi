/// A MIDI-CI Device with the one-chunk Property Exchange Responder, Profile Configuration and
/// Process Inquiry (zmidi-ci, zmidi-ci-pe through the C ABI; M2-101, M2-103): messages in, automatic
/// replies and events out, a reply call for each PE request, Profile and MIDI Message Report
/// messages out.
library;

import 'dart:convert';
import 'dart:typed_data';

import 'checks.dart';
import 'errors.dart';
import 'native/ci.dart' if (dart.library.js_interop) 'web/ci.dart' as be;

/// The kind of a CI event: one for each zmidi-ci Event, Property Exchange Chunks by kind.
enum CiEventKind {
  /// Reply to Discovery from a new peer.
  discovered,

  /// The Discovery window ended.
  discoveryEnded,

  /// This Device took a new MUID.
  muidChanged,

  /// A peer was invalidated.
  peerInvalidated,

  /// Reply to Endpoint Information.
  endpointInfo,

  /// ACK.
  ack,

  /// NAK.
  nak,

  /// Reply to Profile Inquiry.
  profileInquiryReply,

  /// Profile Added, Removed, Enabled or Disabled Report.
  profileReport,

  /// Set Profile On or Off to this Device.
  setProfile,

  /// Inquiry: Profile Details to this Device.
  profileDetailsInquiry,

  /// Reply to Profile Details.
  profileDetailsReply,

  /// Profile Specific Data.
  profileSpecificData,

  /// Reply to Property Exchange Capabilities.
  peCapabilities,

  /// Inquiry: Get Property Data: answer with [CiDevice.peReply].
  peGet,

  /// Inquiry: Set Property Data: answer with [CiDevice.peReply].
  peSet,

  /// Subscription: the one-chunk Responder answers it with 405 in [CiDevice.peReply].
  peSubscription,

  /// Reply to Get Property Data.
  peGetReply,

  /// Reply to Set Property Data.
  peSetReply,

  /// Reply to Subscription.
  peSubscriptionReply,

  /// Notify.
  peNotify,

  /// Reply to Process Inquiry Capabilities.
  piCapabilities,

  /// Inquiry: MIDI Message Report to this Device.
  messageReport,

  /// Reply to MIDI Message Report.
  messageReportReply,

  /// End of MIDI Message Report.
  messageReportEnd,

  /// A message that zmidi-ci rejected.
  rejected,
}

/// One event of a CI Device.
final class CiEvent {
  const CiEvent._({
    required this.kind,
    required this.muid,
    required this.requestId,
    required this.chunkNumber,
    required this.chunkTotal,
    required this.resource,
    required this.resId,
    required this.header,
    required this.data,
    required this.profile,
    required this.process,
  });

  /// The kind of the event.
  final CiEventKind kind;

  /// The MUID of the peer (the new MUID for [CiEventKind.muidChanged]), or null when the event has
  /// none.
  final int? muid;

  /// Property Exchange: Request ID, or null for an event that is not a Chunk.
  final int? requestId;

  /// Property Exchange: Number of This Chunk, or null.
  final int? chunkNumber;

  /// Property Exchange: Number of Chunks in Message (0 when not known), or null.
  final int? chunkTotal;

  /// PE Get or Set: the decoded "resource", or null.
  final String? resource;

  /// PE Get or Set: the decoded "resId", or null.
  final String? resId;

  /// Property Exchange: the Header Data (empty for other events).
  final Uint8List header;

  /// Property Exchange: the Property Data of this Chunk (empty for other events).
  final Uint8List data;

  /// The Profile fields of a [CiEventKind.profileInquiryReply], [CiEventKind.profileReport],
  /// [CiEventKind.setProfile], [CiEventKind.profileDetailsInquiry],
  /// [CiEventKind.profileDetailsReply] or [CiEventKind.profileSpecificData] event; null for the
  /// other kinds.
  final CiProfileEvent? profile;

  /// The Process Inquiry fields of a [CiEventKind.piCapabilities], [CiEventKind.messageReport],
  /// [CiEventKind.messageReportReply] or [CiEventKind.messageReportEnd] event; null for the other
  /// kinds.
  final CiProcessEvent? process;
}

/// The kind of a Profile report (M2-101 7.4, 7.5, 7.10, 7.11).
enum CiProfileReport {
  /// Profile Added Report (0x26).
  added,

  /// Profile Removed Report (0x27).
  removed,

  /// Profile Enabled Report (0x24).
  enabled,

  /// Profile Disabled Report (0x25).
  disabled,
}

/// The Profile fields of an event (M2-101 7).
final class CiProfileEvent {
  const CiProfileEvent._({
    required this.deviceId,
    required this.group,
    required this.profile,
    required this.report,
    required this.on,
    required this.channels,
    required this.target,
    required this.notification,
    required this.enabledProfiles,
    required this.disabledProfiles,
    required this.data,
  });

  /// The Device ID of the message: Channel 0x00 to 0x0F, Group 0x7E, Function Block 0x7F.
  final int deviceId;

  /// The UMP Group of the message; for [CiEventKind.setProfile] and
  /// [CiEventKind.profileDetailsInquiry] the Group offset of the local Profile address.
  final int group;

  /// The Profile ID, 5 bytes (zero for [CiEventKind.profileInquiryReply]).
  final Uint8List profile;

  /// [CiEventKind.profileReport]: the kind of the report; null for the other kinds.
  final CiProfileReport? report;

  /// [CiEventKind.setProfile]: true for Set Profile On, false for Set Profile Off.
  final bool on;

  /// [CiEventKind.setProfile], [CiEventKind.profileReport]: the Number of Channels (Requested,
  /// Enabled or Disabled).
  final int channels;

  /// [CiEventKind.profileDetailsInquiry], [CiEventKind.profileDetailsReply]: the Inquiry Target.
  final int target;

  /// [CiEventKind.profileDetailsReply]: true when this Device did not ask for it (M2-101 7.7).
  final bool notification;

  /// [CiEventKind.profileInquiryReply]: the Currently Enabled Profiles, 5 bytes each (M2-101
  /// Table 18); empty for the other kinds.
  final List<Uint8List> enabledProfiles;

  /// [CiEventKind.profileInquiryReply]: the Currently Disabled Profiles Supported, 5 bytes each;
  /// empty for the other kinds.
  final List<Uint8List> disabledProfiles;

  /// [CiEventKind.profileDetailsReply]: the Inquiry Target Data;
  /// [CiEventKind.profileSpecificData]: the Profile Specific Data; empty for the other kinds.
  final Uint8List data;
}

/// The bitmaps of a MIDI Message Report (M2-101 Tables 43 to 45), each 0 to 0x7F with the
/// reserved bits zero.
final class CiReport {
  /// A report of [dataControl] (Table 44: 0x00 Begin and End only, 0x01 non-default values, 0x7F
  /// full report), [system] (D0 MTC Quarter Frame, D1 Song Position, D2 Song Select), [channel]
  /// (D0 Pitchbend, D1 Control Change, D2 RPN, D3 NRPN, D4 Program Change, D5 Channel Pressure)
  /// and [note] (D0 Notes, D1 Poly Pressure, D2 Per-Note Pitchbend, D3 Registered Per-Note
  /// Controller, D4 Assignable Per-Note Controller).
  const CiReport({
    this.dataControl = 0,
    this.system = 0,
    this.channel = 0,
    this.note = 0,
  });

  /// Message Data Control (Table 44); not used for the supported set of
  /// [CiDevice.messageReportReply], and 0 in a Reply to MIDI Message Report.
  final int dataControl;

  /// The System Messages bitmap.
  final int system;

  /// The Channel Controller Messages bitmap.
  final int channel;

  /// The Note Data Messages bitmap.
  final int note;

  @override
  bool operator ==(Object other) =>
      other is CiReport &&
      other.dataControl == dataControl &&
      other.system == system &&
      other.channel == channel &&
      other.note == note;

  @override
  int get hashCode => Object.hash(dataControl, system, channel, note);

  @override
  String toString() =>
      'CiReport(dataControl: $dataControl, system: $system, channel: $channel, note: $note)';
}

/// The Process Inquiry fields of an event (M2-101 9).
final class CiProcessEvent {
  const CiProcessEvent._({
    required this.deviceId,
    required this.group,
    required this.midiMessageReport,
    required this.report,
  });

  /// [CiEventKind.messageReport]: the Device ID of the inquiry; 0 for the other kinds.
  final int deviceId;

  /// [CiEventKind.messageReport]: the UMP Group of the inquiry; 0 for the other kinds.
  final int group;

  /// [CiEventKind.piCapabilities]: the MIDI Message Report bit of Process Inquiry Supported
  /// Features (Table 42).
  final bool midiMessageReport;

  /// [CiEventKind.messageReport]: the requested bitmaps with Message Data Control;
  /// [CiEventKind.messageReportReply]: the bitmaps that the peer reports. Zero for the other
  /// kinds.
  final CiReport report;
}

/// One MIDI-CI message with its Group.
typedef CiMessage = ({int group, Uint8List message});

/// A CI event of the native or the web form. The entry library hides it.
CiEvent newCiEvent({
  required CiEventKind kind,
  required int? muid,
  required int? requestId,
  required int? chunkNumber,
  required int? chunkTotal,
  required String? resource,
  required String? resId,
  required Uint8List header,
  required Uint8List data,
  CiProfileEvent? profile,
  CiProcessEvent? process,
}) => CiEvent._(
  kind: kind,
  muid: muid,
  requestId: requestId,
  chunkNumber: chunkNumber,
  chunkTotal: chunkTotal,
  resource: resource,
  resId: resId,
  header: header,
  data: data,
  profile: profile,
  process: process,
);

/// The Profile fields of an event of the native or the web form. The entry library hides it.
CiProfileEvent newCiProfileEvent({
  required int deviceId,
  required int group,
  required Uint8List profile,
  required CiProfileReport? report,
  required bool on,
  required int channels,
  required int target,
  required bool notification,
  required List<Uint8List> enabledProfiles,
  required List<Uint8List> disabledProfiles,
  required Uint8List data,
}) => CiProfileEvent._(
  deviceId: deviceId,
  group: group,
  profile: profile,
  report: report,
  on: on,
  channels: channels,
  target: target,
  notification: notification,
  enabledProfiles: enabledProfiles,
  disabledProfiles: disabledProfiles,
  data: data,
);

/// The Process Inquiry fields of an event of the native or the web form. The entry library hides
/// it.
CiProcessEvent newCiProcessEvent({
  required int deviceId,
  required int group,
  required bool midiMessageReport,
  required CiReport report,
}) => CiProcessEvent._(
  deviceId: deviceId,
  group: group,
  midiMessageReport: midiMessageReport,
  report: report,
);

/// The checked configuration of a Device. The entry library hides it.
typedef CiConfig = ({
  List<int> manufacturer,
  int family,
  int model,
  List<int> revision,
  int maxSysexSize,
  int? functionBlock,
  int firstGroup,
  int groupCount,
  int channelsInUse,
  String? productInstanceId,
  bool profiles,
  bool propertyExchange,
  bool processInquiry,
  bool sysexFraming,
  List<int> resourceList,
  int seed,
});

/// Checks a Profile ID: 5 bytes of 7 bits (M2-101 Table 19).
List<int> _checkProfile(List<int> profile) {
  if (profile.length != 5) {
    throw InvalidValueException(
      'a Profile ID has 5 bytes, not ${profile.length}',
    );
  }
  for (var i = 0; i < 5; i++) {
    checkInt('profile byte $i', profile[i], 0x7f);
  }
  return profile;
}

/// Checks the bitmaps of a report: each 0 to 0x7F.
CiReport _checkReport(CiReport r) {
  checkInt('dataControl', r.dataControl, 0x7f);
  checkInt('system', r.system, 0x7f);
  checkInt('channel', r.channel, 0x7f);
  checkInt('note', r.note, 0x7f);
  return r;
}

void _checkPeer(int peer, int deviceId, int group) {
  checkInt('peer', peer, 0x0fffffff);
  checkInt('deviceId', deviceId, 0x7f);
  checkInt('group', group, 15);
}

/// The state of [dev] in the native or the web form, for the Property Exchange engine (pe.dart).
/// The entry library hides it.
be.CiState ciHandle(CiDevice dev) => dev._live;

/// A MIDI-CI Device (4 peers, 8 Profiles, 4 Property Exchange requests).
final class CiDevice {
  /// A Device. [manufacturer]: the SysEx ID, 1 byte, or 3 bytes that start with 0x00.
  /// [revision]: 4 bytes of 7 bits. [maxSysexSize]: at least 512 with Profiles or Property
  /// Exchange. [functionBlock]: 0 to 0x7E when the Device is a Function Block. [channelsInUse]: bit
  /// n is Channel n+1. [productInstanceId]: at most 42 ASCII characters. [sysexFraming]: messages
  /// with 0xF0/0xF7 when true, the SysEx7 payload only when false. [resourceList]: the
  /// ResourceList JSON for a Get of "ResourceList". [seed] starts the MUID generator.
  CiDevice({
    required List<int> manufacturer,
    int family = 0,
    int model = 0,
    List<int> revision = const [0, 0, 0, 0],
    int maxSysexSize = 512,
    int? functionBlock,
    int firstGroup = 0,
    int groupCount = 1,
    int channelsInUse = 1,
    String? productInstanceId,
    bool profiles = false,
    bool propertyExchange = true,
    bool processInquiry = false,
    bool sysexFraming = false,
    List<int>? resourceList,
    int seed = 1,
  }) : _b = be.CiState(
         _config(
           manufacturer: manufacturer,
           family: family,
           model: model,
           revision: revision,
           maxSysexSize: maxSysexSize,
           functionBlock: functionBlock,
           firstGroup: firstGroup,
           groupCount: groupCount,
           channelsInUse: channelsInUse,
           productInstanceId: productInstanceId,
           profiles: profiles,
           propertyExchange: propertyExchange,
           processInquiry: processInquiry,
           sysexFraming: sysexFraming,
           resourceList: resourceList,
           seed: seed,
         ),
       );

  static CiConfig _config({
    required List<int> manufacturer,
    required int family,
    required int model,
    required List<int> revision,
    required int maxSysexSize,
    required int? functionBlock,
    required int firstGroup,
    required int groupCount,
    required int channelsInUse,
    required String? productInstanceId,
    required bool profiles,
    required bool propertyExchange,
    required bool processInquiry,
    required bool sysexFraming,
    required List<int>? resourceList,
    required int seed,
  }) {
    final List<int> m;
    if (manufacturer.length == 1) {
      m = [manufacturer[0], 0, 0];
    } else if (manufacturer.length == 3 && manufacturer[0] == 0) {
      m = List.of(manufacturer);
    } else {
      throw const InvalidValueException(
        'a manufacturer ID has 1 byte, or 3 bytes that start with 0x00',
      );
    }
    if (revision.length != 4) {
      throw const InvalidValueException('a revision has 4 bytes');
    }
    for (var i = 0; i < 3; i++) {
      checkInt('manufacturer', m[i], 0x7f);
    }
    for (var i = 0; i < 4; i++) {
      checkInt('revision', revision[i], 0x7f);
    }
    return (
      manufacturer: m,
      family: checkInt('family', family, 0x3fff),
      model: checkInt('model', model, 0x3fff),
      revision: List.of(revision),
      maxSysexSize: checkInt('maxSysexSize', maxSysexSize, 0xffffffff),
      functionBlock: functionBlock == null
          ? null
          : checkInt('functionBlock', functionBlock, 0x7e),
      firstGroup: checkInt('firstGroup', firstGroup, 15),
      groupCount: checkInt('groupCount', groupCount, 16),
      channelsInUse: checkInt('channelsInUse', channelsInUse, 0xffff),
      productInstanceId: productInstanceId,
      profiles: profiles,
      propertyExchange: propertyExchange,
      processInquiry: processInquiry,
      sysexFraming: sysexFraming,
      resourceList: List.of(resourceList ?? ascii.encode('[]')),
      seed: checkInt('seed', seed, 0xffffffff),
    );
  }

  final be.CiState _b;
  bool _freed = false;

  be.CiState get _live {
    checkFreed(_freed, this);
    return _b;
  }

  /// The MUID of the Device.
  int get muid => _live.muid;

  /// Reads one MIDI-CI message received on [group] at [nowUs] (microseconds). Gives the automatic
  /// replies and the event (or null). After a [CiEventKind.peGet], [CiEventKind.peSet] or
  /// [CiEventKind.peSubscription] event, answer with [peReply], or give the event to
  /// `PeEngine.receive`.
  ({List<CiMessage> replies, CiEvent? event}) receive(
    int group,
    List<int> message,
    int nowUs,
  ) {
    final b = _live;
    checkInt('group', group, 15);
    checkTime('nowUs', nowUs);
    return b.receive(group, message, nowUs);
  }

  /// Runs the timers at [nowUs] and gives their event, or null.
  CiEvent? poll(int nowUs) {
    final b = _live;
    return b.poll(checkTime('nowUs', nowUs));
  }

  /// The Discovery message for the output path [outputPathId] at [nowUs].
  Uint8List discovery({int outputPathId = 0, int nowUs = 0}) {
    final b = _live;
    checkInt('outputPathId', outputPathId, 0x7f);
    checkTime('nowUs', nowUs);
    return b.discovery(outputPathId, nowUs);
  }

  /// The Inquiry: Endpoint Information message to [peer].
  Uint8List endpointInfoInquiry(int peer) {
    final b = _live;
    return b.endpointInfoInquiry(checkInt('peer', peer, 0x0fffffff));
  }

  /// The Inquiry: Property Exchange Capabilities message to [peer].
  Uint8List peCapabilitiesInquiry(int peer) {
    final b = _live;
    return b.peCapabilitiesInquiry(checkInt('peer', peer, 0x0fffffff));
  }

  /// An Inquiry: Get Property Data with the Header Data [header] to [peer]. Gives the Request ID
  /// and the Chunks.
  ({int requestId, List<Uint8List> chunks}) peGet(int peer, List<int> header) {
    final b = _live;
    return b.peGet(checkInt('peer', peer, 0x0fffffff), header);
  }

  /// An Inquiry: Set Property Data with [header] and [data] to [peer]. Gives the Request ID and
  /// the Chunks.
  ({int requestId, List<Uint8List> chunks}) peSet(
    int peer,
    List<int> header,
    List<int> data,
  ) {
    final b = _live;
    return b.peSet(checkInt('peer', peer, 0x0fffffff), header, data);
  }

  /// Frees the Request ID [requestId] of a [peGet] or [peSet] to [peer] that has no reply (a
  /// timeout or a NAK). The last Chunk of a reply frees the ID at the next [receive] without this
  /// call. Not for a request of a `PeEngine`. A MUID or Request ID out of range throws
  /// [InvalidValueException].
  void peEnd(int peer, int requestId) {
    final b = _live;
    checkInt('peer', peer, 0x0fffffff);
    checkInt('requestId', requestId, 0x7f);
    b.peEnd(peer, requestId);
  }

  /// Answers the pending Property Exchange request with [status] (200: success) and, for a Get,
  /// the Property Data [data]. Gives the reply Chunks. Throws [RejectedException] when no request
  /// is pending.
  List<Uint8List> peReply(int status, [List<int> data = const []]) {
    final b = _live;
    return b.peReply(checkInt('status', status, 0xffff), data);
  }

  /// Adds the local Profile [profile] (5 bytes) at the Device ID [deviceId] and the Group offset
  /// [group] to the Profile table (M2-101 7.4), [enabled] with [channels] Channels and at most
  /// [maxChannels]. Gives the Profile Added Report, and the Profile Enabled Report for an enabled
  /// Profile. Throws [RejectedException] for a Profile that is in the table or a full table.
  List<CiMessage> addProfile(
    int deviceId,
    List<int> profile, {
    int group = 0,
    bool enabled = false,
    int channels = 0,
    int maxChannels = 0,
  }) {
    final b = _live;
    _checkProfile(profile);
    return b.addProfile(
      checkInt('deviceId', deviceId, 0x7f),
      profile,
      checkInt('group', group, 15),
      enabled,
      checkInt('channels', channels, 0x3fff),
      checkInt('maxChannels', maxChannels, 0x3fff),
    );
  }

  /// Removes the local Profile [profile] at [deviceId] and the Group offset [group] (M2-101 7.5).
  /// Gives the Profile Disabled Report for an enabled Profile, then the Profile Removed Report.
  /// Throws [RejectedException] for a Profile that is not in the table.
  List<CiMessage> removeProfile(
    int deviceId,
    List<int> profile, {
    int group = 0,
  }) {
    final b = _live;
    _checkProfile(profile);
    checkInt('deviceId', deviceId, 0x7f);
    checkInt('group', group, 15);
    return b.removeProfile(deviceId, profile, group);
  }

  /// Sets the local Profile [profile] at [deviceId] and the Group offset [group] to [enabled]
  /// with [channels] Channels (M2-101 7.10, 7.11): the answer to a [CiEventKind.setProfile] event
  /// (give false to refuse a Set Profile On), or a local change. Gives the Profile Enabled or
  /// Disabled Report. Throws [RejectedException] for a Profile that is not in the table.
  List<CiMessage> setProfileState(
    int deviceId,
    List<int> profile, {
    int group = 0,
    required bool enabled,
    int channels = 0,
  }) {
    final b = _live;
    _checkProfile(profile);
    checkInt('deviceId', deviceId, 0x7f);
    checkInt('group', group, 15);
    checkInt('channels', channels, 0x3fff);
    return b.setProfileState(deviceId, profile, group, enabled, channels);
  }

  /// The Reply to Profile Details Inquiry (M2-101 7.7) to [peer] for the Profile [profile] at
  /// [deviceId] with the Inquiry Target [target] and the Inquiry Target Data [data]: the answer
  /// to a [CiEventKind.profileDetailsInquiry] event.
  Uint8List profileDetailsReply(
    int peer,
    int deviceId,
    List<int> profile,
    int target,
    List<int> data, {
    int group = 0,
  }) {
    final b = _live;
    _checkPeer(peer, deviceId, group);
    _checkProfile(profile);
    checkInt('target', target, 0x7f);
    return b.profileDetailsReply(peer, deviceId, profile, target, data, group);
  }

  /// The Profile Specific Data message (M2-101 7.12) for the Profile [profile] at [deviceId] with
  /// [data] to [peer] (0x0FFFFFFF: Broadcast).
  Uint8List profileSpecificData(
    int peer,
    int deviceId,
    List<int> profile,
    List<int> data, {
    int group = 0,
  }) {
    final b = _live;
    _checkPeer(peer, deviceId, group);
    _checkProfile(profile);
    return b.profileSpecificData(peer, deviceId, profile, data, group);
  }

  /// The Profile Inquiry message (M2-101 7.2) to [peer] for the address [deviceId]. The reply
  /// gives a [CiEventKind.profileInquiryReply] event.
  Uint8List profileInquiry(int peer, int deviceId, {int group = 0}) {
    final b = _live;
    _checkPeer(peer, deviceId, group);
    return b.profileInquiry(peer, deviceId, group);
  }

  /// The Set Profile On message (M2-101 7.8) to [peer] for the Profile [profile] at [deviceId]
  /// with [channels] Channels Requested; [maxChannels] is the Maximum Number of Channels that the
  /// peer reported, or 0 when not known. Throws [RejectedException] before a Profile Inquiry to
  /// the peer (M2-101 2.2).
  Uint8List setProfileOn(
    int peer,
    int deviceId,
    List<int> profile, {
    int group = 0,
    int channels = 1,
    int maxChannels = 0,
  }) {
    final b = _live;
    _checkPeer(peer, deviceId, group);
    _checkProfile(profile);
    checkInt('channels', channels, 0x3fff);
    checkInt('maxChannels', maxChannels, 0x3fff);
    return b.setProfileOn(
      peer,
      deviceId,
      profile,
      group,
      channels,
      maxChannels,
    );
  }

  /// The Set Profile Off message (M2-101 7.9) to [peer] for the Profile [profile] at [deviceId].
  /// Throws [RejectedException] before a Profile Inquiry to the peer.
  Uint8List setProfileOff(
    int peer,
    int deviceId,
    List<int> profile, {
    int group = 0,
  }) {
    final b = _live;
    _checkPeer(peer, deviceId, group);
    _checkProfile(profile);
    return b.setProfileOff(peer, deviceId, profile, group);
  }

  /// The Profile Details Inquiry message (M2-101 7.6) to [peer] for the Profile [profile] at
  /// [deviceId] and the Inquiry Target [target]; version 2 peers only.
  Uint8List profileDetailsInquiry(
    int peer,
    int deviceId,
    List<int> profile,
    int target, {
    int group = 0,
  }) {
    final b = _live;
    _checkPeer(peer, deviceId, group);
    _checkProfile(profile);
    checkInt('target', target, 0x7f);
    return b.profileDetailsInquiry(peer, deviceId, profile, target, group);
  }

  /// The Inquiry: Process Inquiry Capabilities message (M2-101 9.2) to [peer]; version 2 peers
  /// only. The reply gives a [CiEventKind.piCapabilities] event.
  Uint8List piCapabilitiesInquiry(int peer) {
    final b = _live;
    return b.piCapabilitiesInquiry(checkInt('peer', peer, 0x0fffffff));
  }

  /// The Inquiry: MIDI Message Report message (M2-101 9.5) with the bitmaps of [inquiry] to [peer]
  /// for [deviceId] (0x7F when System Messages are requested). It opens the report: see
  /// [reportOpen]. Version 2 peers only.
  Uint8List messageReportInquiry(
    int peer,
    CiReport inquiry, {
    int deviceId = 0x7f,
    int group = 0,
  }) {
    final b = _live;
    _checkPeer(peer, deviceId, group);
    return b.messageReportInquiry(peer, _checkReport(inquiry), deviceId, group);
  }

  /// True while a MIDI Message Report from [peer] is open: after [messageReportInquiry], until
  /// End of MIDI Message Report.
  bool reportOpen(int peer) {
    final b = _live;
    return b.reportOpen(checkInt('peer', peer, 0x0fffffff));
  }

  /// The Reply to MIDI Message Report (M2-101 9.6) to [peer] with the Device ID [deviceId] of the
  /// inquiry and the bitmaps that are in both [requested] (the inquiry) and [supported]: the
  /// answer to a [CiEventKind.messageReport] event. Then the program sends the declared messages
  /// with their current values (M2-101 9.7) and calls [messageReportEnd].
  Uint8List messageReportReply(
    int peer,
    CiReport requested,
    CiReport supported, {
    int deviceId = 0x7f,
    int group = 0,
  }) {
    final b = _live;
    _checkPeer(peer, deviceId, group);
    _checkReport(requested);
    _checkReport(supported);
    return b.messageReportReply(peer, requested, supported, deviceId, group);
  }

  /// The End of MIDI Message Report message (M2-101 9.8) to [peer] with the Device ID [deviceId]
  /// of the inquiry.
  Uint8List messageReportEnd(int peer, {int deviceId = 0x7f, int group = 0}) {
    final b = _live;
    _checkPeer(peer, deviceId, group);
    return b.messageReportEnd(peer, deviceId, group);
  }

  /// The NAK that stops a MIDI Message Report to [peer] (M2-101 9.8.1): Status Code 0x20
  /// (Responder terminates), or 0x40 when [error].
  Uint8List abortReport(
    int peer, {
    int deviceId = 0x7f,
    int group = 0,
    bool error = false,
  }) {
    final b = _live;
    _checkPeer(peer, deviceId, group);
    return b.abortReport(peer, deviceId, group, error);
  }

  /// Frees the memory of the Device now.
  void free() {
    if (!_freed) {
      _freed = true;
      _b.free();
    }
  }
}
