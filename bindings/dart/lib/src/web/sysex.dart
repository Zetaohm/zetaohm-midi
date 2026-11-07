/// The web form of the sysex parts: the TypeScript package through dart:js_interop. The values are
/// checked by `sysex.dart`.
library;

import 'dart:js_interop';
import 'dart:typed_data';

import 'runtime.dart';

/// (hours, minutes, seconds, frames, rate).
typedef RawTime = (int, int, int, int, int);

/// (time, colorFrame, blank, negative, statusForm, subframes, estimated, invalid, videoField1,
/// noTimeCode).
typedef RawTimeCode = (
  RawTime,
  bool,
  bool,
  bool,
  bool,
  int,
  bool,
  bool,
  bool,
  bool,
);

/// (manufacturer in the C form of 3 bytes, family, member, revision).
typedef RawIdentity = (Uint8List, int, int, Uint8List);

/// (kind, register, destination, source, target, name, data).
typedef RawCommand = (int, int, int, int, RawTimeCode?, Uint8List, Uint8List);

/// (kind, field, code, name, data).
typedef RawField = (int, int, RawTimeCode?, Uint8List, Uint8List);

/// The TypeScript kinds, at the index of their C value.
const _classes = ['manufacturer', 'nonCommercial', 'nonRealTime', 'realTime'];
const _dcKinds = [
  'masterVolume',
  'masterBalance',
  'masterFineTuning',
  'masterCoarseTuning',
];
const _tuningKinds = [
  'dumpRequest',
  'keyDump',
  'noteChange',
  'scaleDump',
  'scaleOctave',
];
const _commandKinds = [
  'stop',
  'play',
  'deferredPlay',
  'fastForward',
  'rewind',
  'recordStrobe',
  'recordExit',
  'pause',
  'reset',
  'write',
  'locateField',
  'locateTarget',
  'move',
  'other',
];
const _fieldKinds = ['timeCode', 'short', 'other'];

int _index(List<String> names, JSAny? a) {
  final i = names.indexOf(stringOrNull(a) ?? '');
  return i < 0 ? 0 : i;
}

/// The 1 or 3 ID bytes of the TypeScript form, of the C form.
JSUint8Array _tsId(Uint8List c) => jsBytes(c[0] == 0 ? c : c.sublist(0, 1));

/// The C form of 3 bytes, of the 1 or 3 ID bytes of the TypeScript form.
Uint8List _cId(JSAny? a) {
  final b = bytes(a);
  return b.length == 1
      ? Uint8List.fromList([b[0], 0, 0])
      : (b.isEmpty ? Uint8List(3) : b);
}

T? _orNull<T>(JSAny? a, T Function(JSObject o) f) =>
    a == null || a.isUndefinedOrNull ? null : f(a as JSObject);

/// (kind, manufacturer in 3 bytes, Device ID, Sub-ID#1, Sub-ID#2 or null, data).
(int, Uint8List, int, int, int?, Uint8List) parseSysex(Uint8List data) {
  final o = tsCall<JSObject>('parseSysex', [jsBytes(data)]);
  return (
    _index(_classes, prop(o, 'kind')),
    _cId(prop(o, 'manufacturer')),
    intOrNull(prop(o, 'deviceId')) ?? 0,
    intOrNull(prop(o, 'subId1')) ?? 0,
    intOrNull(prop(o, 'subId2')),
    bytes(prop(o, 'data')),
  );
}

/// The Device ID test.
bool sysexAddressed(int deviceId, int own) =>
    boolOf(tsCall('sysexAddressed', [deviceId.toJS, own.toJS]));

/// The bytes of a manufacturer message; [manufacturer] in the C form of 3 bytes.
Uint8List manufacturerSysex(Uint8List manufacturer, Uint8List data) =>
    bytes(tsCall('manufacturerSysex', [_tsId(manufacturer), jsBytes(data)]));

/// The bytes of a Universal message.
Uint8List universalSysex(
  bool realTime,
  int deviceId,
  int subId1,
  int? subId2,
  Uint8List data,
) => bytes(
  tsCall('universalSysex', [
    realTime.toJS,
    deviceId.toJS,
    subId1.toJS,
    subId2?.toJS,
    jsBytes(data),
  ]),
);

JSObject _identityIn(RawIdentity i) => jsObject({
  'manufacturer': _tsId(i.$1),
  'family': i.$2.toJS,
  'member': i.$3.toJS,
  'revision': jsBytes(i.$4),
});

RawIdentity _identityOut(JSObject o) => (
  _cId(prop(o, 'manufacturer')),
  intOf(prop(o, 'family')),
  intOf(prop(o, 'member')),
  bytes(prop(o, 'revision')),
);

/// (reply, Device ID, identity of a reply), or null.
(bool, int, RawIdentity?)? readInquiry(Uint8List data) =>
    _orNull(tsCall('readInquiry', [jsBytes(data)]), (o) {
      return (
        boolOf(prop(o, 'isReply')),
        intOf(prop(o, 'deviceId')),
        _orNull(prop(o, 'identity'), _identityOut),
      );
    });

/// The bytes of an Identity Request.
Uint8List identityRequest(int deviceId) =>
    bytes(tsCall('identityRequest', [deviceId.toJS]));

/// The bytes of an Identity Reply.
Uint8List identityReply(int deviceId, RawIdentity identity) =>
    bytes(tsCall('identityReply', [deviceId.toJS, _identityIn(identity)]));

/// The Identity Reply to a request for the device, or null.
Uint8List? respondIdentity(Uint8List data, int own, RawIdentity identity) {
  final r = tsCall<JSAny?>('respondIdentity', [
    jsBytes(data),
    own.toJS,
    _identityIn(identity),
  ]);
  return r == null || r.isUndefinedOrNull ? null : bytes(r);
}

/// (Device ID, command), or null.
(int, int)? readGmSystem(Uint8List data) => _orNull(
  tsCall('readGmSystem', [jsBytes(data)]),
  (o) => (intOf(prop(o, 'deviceId')), intOf(prop(o, 'command'))),
);

/// The bytes of a General MIDI System message.
Uint8List gmSystem(int deviceId, int command) =>
    bytes(tsCall('gmSystem', [deviceId.toJS, command.toJS]));

/// (kind, Device ID, value), or null.
(int, int, int)? readDeviceControl(Uint8List data) => _orNull(
  tsCall('readDeviceControl', [jsBytes(data)]),
  (o) => (
    _index(_dcKinds, prop(o, 'kind')),
    intOf(prop(o, 'deviceId')),
    intOf(prop(o, 'value')),
  ),
);

/// The bytes of a Device Control message.
Uint8List deviceControlBytes(int kind, int deviceId, int value) => bytes(
  tsCall('deviceControlBytes', [
    jsObject({
      'kind': _dcKinds[kind].toJS,
      'deviceId': deviceId.toJS,
      'value': value.toJS,
    }),
  ]),
);

/// The offset of a Master Fine Tuning value.
int fineTuningOffset(int value) =>
    intOf(tsCall('fineTuningOffset', [value.toJS]));

/// The semitones of a Master Coarse Tuning MSB.
int coarseTuningSemitones(int msb) =>
    intOf(tsCall('coarseTuningSemitones', [msb.toJS]));

/// (kind, real time, Device ID, bank, program, name, checksum ok, channels, two-byte, offsets,
/// notes as (key, semitone, fraction)), or null.
(
  int,
  bool,
  int,
  int?,
  int,
  Uint8List,
  bool,
  int,
  bool,
  List<int>,
  List<(int, int, int)>,
)?
readTuning(Uint8List data) =>
    _orNull(tsCall('readTuning', [jsBytes(data)]), (o) {
      return (
        _index(_tuningKinds, prop(o, 'kind')),
        boolOf(prop(o, 'realTime')),
        intOf(prop(o, 'deviceId')),
        intOrNull(prop(o, 'bank')),
        intOf(prop(o, 'program')),
        bytes(prop(o, 'name')),
        boolOf(prop(o, 'checksumOk')),
        intOf(prop(o, 'channels')),
        boolOf(prop(o, 'twoByte')),
        ints(prop(o, 'offsets')),
        [
          for (final n in (prop<JSArray<JSObject>>(o, 'notes')).toDart)
            (
              intOf(prop(n, 'key')),
              intOf(prop(n, 'semitone')),
              intOf(prop(n, 'fraction')),
            ),
        ],
      );
    });

/// The bytes of a Bulk Tuning Dump Request.
Uint8List tuningDumpRequest(int deviceId, int program, int? bank) => bytes(
  tsCall('tuningDumpRequest', [deviceId.toJS, program.toJS, bank?.toJS]),
);

JSObject _word(int semitone, int fraction) =>
    jsObject({'semitone': semitone.toJS, 'fraction': fraction.toJS});

/// The bytes of a key-based tuning dump from 128 (semitone, fraction) words.
Uint8List tuningKeyDump(
  int deviceId,
  int program,
  Uint8List name,
  List<(int, int)> words,
  int? bank,
) => bytes(
  tsCall('tuningKeyDump', [
    deviceId.toJS,
    program.toJS,
    jsBytes(name),
    [for (final w in words) _word(w.$1, w.$2)].toJS,
    bank?.toJS,
  ]),
);

/// The bytes of a Single Note Tuning Change from (key, semitone, fraction) notes.
Uint8List tuningNoteChange(
  bool realTime,
  int deviceId,
  int program,
  List<(int, int, int)> notes,
  int? bank,
) => bytes(
  tsCall('tuningNoteChange', [
    realTime.toJS,
    deviceId.toJS,
    program.toJS,
    [
      for (final n in notes)
        jsObject({
          'key': n.$1.toJS,
          'semitone': n.$2.toJS,
          'fraction': n.$3.toJS,
        }),
    ].toJS,
    bank?.toJS,
  ]),
);

/// The bytes of a Scale/Octave Tuning Dump.
Uint8List tuningScaleDump(
  int deviceId,
  int bank,
  int program,
  Uint8List name,
  List<int> offsets,
  bool twoByte,
) => bytes(
  tsCall('tuningScaleDump', [
    deviceId.toJS,
    bank.toJS,
    program.toJS,
    jsBytes(name),
    jsInts(offsets),
    twoByte.toJS,
  ]),
);

/// The bytes of a Scale/Octave Tuning message.
Uint8List tuningScaleOctave(
  bool realTime,
  int deviceId,
  int channels,
  List<int> offsets,
  bool twoByte,
) => bytes(
  tsCall('tuningScaleOctave', [
    realTime.toJS,
    deviceId.toJS,
    channels.toJS,
    jsInts(offsets),
    twoByte.toJS,
  ]),
);

/// The checksum of the bytes of a tuning dump.
int tuningChecksum(Uint8List data) =>
    intOf(tsCall('tuningChecksum', [jsBytes(data)]));

/// The (semitone, fraction) of three bytes.
(int, int) frequencyWord(Uint8List data) {
  final o = tsCall<JSObject>('frequencyWord', [jsBytes(data)]);
  return (intOf(prop(o, 'semitone')), intOf(prop(o, 'fraction')));
}

/// The three bytes of a frequency word.
Uint8List frequencyWordBytes(int semitone, int fraction) =>
    bytes(tsCall('frequencyWordBytes', [semitone.toJS, fraction.toJS]));

JSObject _timeIn(RawTime t) => jsObject({
  'hours': t.$1.toJS,
  'minutes': t.$2.toJS,
  'seconds': t.$3.toJS,
  'frames': t.$4.toJS,
  'rate': t.$5.toJS,
});

RawTime _timeOut(JSObject o) => (
  intOf(prop(o, 'hours')),
  intOf(prop(o, 'minutes')),
  intOf(prop(o, 'seconds')),
  intOf(prop(o, 'frames')),
  intOf(prop(o, 'rate')),
);

/// The validity of a time.
bool mtcTimeIsValid(RawTime t) =>
    boolOf(tsCall('mtcTimeIsValid', [_timeIn(t)]));

/// The next valid time.
RawTime mtcNextValid(RawTime t) =>
    _timeOut(tsCall('mtcNextValid', [_timeIn(t)]));

/// The frame count of a time.
int mtcToFrameCount(RawTime t) =>
    intOf(tsCall('mtcToFrameCount', [_timeIn(t)]));

/// The time of a frame count.
RawTime mtcFromFrameCount(int count, int rate) =>
    _timeOut(tsCall('mtcFromFrameCount', [count.toJS, rate.toJS]));

/// The time some frames later or earlier.
RawTime mtcAddFrames(RawTime t, int delta) =>
    _timeOut(tsCall('mtcAddFrames', [_timeIn(t), delta.toJS]));

/// The 8 Quarter Frame data bytes of a time.
Uint8List mtcQuarterFrames(RawTime t) =>
    bytes(tsCall('mtcQuarterFrames', [_timeIn(t)]));

/// The TypeScript MtcGenerator.
final class MtcGeneratorCore {
  /// A running generator at [time].
  MtcGeneratorCore(RawTime time, bool reverse)
    : _o = tsNew('MtcGenerator', [_timeIn(time), reverse.toJS]);

  final JSObject _o;

  /// The next data byte, or null while stopped.
  int? nextByte() => intOrNull(call(_o, 'nextByte'));

  /// Runs or stops the generator.
  void setRunning(bool running) =>
      call<JSAny?>(_o, 'setRunning', [running.toJS]);

  /// Sets the direction.
  void setDirection(bool reverse) =>
      call<JSAny?>(_o, 'setDirection', [reverse.toJS]);

  /// Moves the generator to [time].
  void locate(RawTime time) => call<JSAny?>(_o, 'locate', [_timeIn(time)]);

  /// The time of the current or next sequence.
  RawTime time() => _timeOut(prop(_o, 'time'));

  /// Frees the WASM memory.
  void free() => call<JSAny?>(_o, 'free');
}

/// The TypeScript MtcReader.
final class MtcReaderCore {
  /// A reader that is not locked and is stopped.
  MtcReaderCore() : _o = tsNew('MtcReader', []);

  final JSObject _o;

  /// (piece, boundary, raw, current, reverse, discontinuity).
  (int, bool, RawTime?, RawTime?, bool, bool) receive(int byte) {
    final e = call<JSObject>(_o, 'receive', [byte.toJS]);
    return (
      intOf(prop(e, 'piece')),
      boolOf(prop(e, 'boundary')),
      _orNull(prop(e, 'raw'), _timeOut),
      _orNull(prop(e, 'current'), _timeOut),
      boolOf(prop(e, 'reverse')),
      boolOf(prop(e, 'discontinuity')),
    );
  }

  /// Takes the time of a Full Message.
  void fullMessage(RawTime time) =>
      call<JSAny?>(_o, 'fullMessage', [_timeIn(time)]);

  /// (locked, running, direction 0 none 1 forward 2 reverse, time).
  (bool, bool, int, RawTime?) info() {
    final i = prop<JSObject>(_o, 'info');
    final dir = stringOrNull(prop(i, 'direction'));
    return (
      boolOf(prop(i, 'locked')),
      boolOf(prop(i, 'running')),
      dir == null ? 0 : (dir == 'reverse' ? 2 : 1),
      _orNull(prop(i, 'time'), _timeOut),
    );
  }

  /// Frees the WASM memory.
  void free() => call<JSAny?>(_o, 'free');
}

/// (Device ID, time), or null.
(int, RawTime)? readMtcFull(Uint8List data) => _orNull(
  tsCall('readMtcFull', [jsBytes(data)]),
  (o) => (intOf(prop(o, 'deviceId')), _timeOut(prop(o, 'time'))),
);

/// The bytes of an MTC Full Message.
Uint8List mtcFull(int deviceId, RawTime time) =>
    bytes(tsCall('mtcFull', [deviceId.toJS, _timeIn(time)]));

/// (Device ID, groups, flags), or null.
(int, Uint8List, int)? readMtcUserBits(Uint8List data) => _orNull(
  tsCall('readMtcUserBits', [jsBytes(data)]),
  (o) => (
    intOf(prop(o, 'deviceId')),
    bytes(prop(o, 'groups')),
    intOf(prop(o, 'flags')),
  ),
);

/// The bytes of an MTC User Bits message.
Uint8List mtcUserBits(int deviceId, Uint8List groups, int flags) =>
    bytes(tsCall('mtcUserBits', [deviceId.toJS, jsBytes(groups), flags.toJS]));

JSObject _codeIn(RawTimeCode c) => jsObject({
  'time': _timeIn(c.$1),
  'colorFrame': c.$2.toJS,
  'blank': c.$3.toJS,
  'negative': c.$4.toJS,
  'statusForm': c.$5.toJS,
  'subframes': c.$6.toJS,
  'estimated': c.$7.toJS,
  'invalid': c.$8.toJS,
  'videoField1': c.$9.toJS,
  'noTimeCode': c.$10.toJS,
});

RawTimeCode _codeOut(JSObject o) => (
  _timeOut(prop(o, 'time')),
  boolOf(prop(o, 'colorFrame')),
  boolOf(prop(o, 'blank')),
  boolOf(prop(o, 'negative')),
  boolOf(prop(o, 'statusForm')),
  intOf(prop(o, 'subframes')),
  boolOf(prop(o, 'estimated')),
  boolOf(prop(o, 'invalid')),
  boolOf(prop(o, 'videoField1')),
  boolOf(prop(o, 'noTimeCode')),
);

/// A time code of its 5 bytes.
RawTimeCode timeCodeFromBytes(Uint8List data) =>
    _codeOut(tsCall('timeCodeFromBytes', [jsBytes(data)]));

/// The 5 bytes of a time code.
Uint8List timeCodeBytes(RawTimeCode tc) =>
    bytes(tsCall('timeCodeBytes', [_codeIn(tc)]));

/// The time code with a short time code applied.
RawTimeCode timeCodeWithShort(RawTimeCode last, Uint8List data) =>
    _codeOut(tsCall('timeCodeWithShort', [_codeIn(last), jsBytes(data)]));

/// The 2 bytes of the short form.
Uint8List timeCodeShortBytes(RawTimeCode tc) =>
    bytes(tsCall('timeCodeShortBytes', [_codeIn(tc)]));

/// The MOVE rule to the subframes form.
RawTimeCode timeCodeToSubframesForm(RawTimeCode tc) =>
    _codeOut(tsCall('timeCodeToSubframesForm', [_codeIn(tc)]));

/// The MOVE rule to the status form.
RawTimeCode timeCodeToStatusForm(RawTimeCode tc, bool videoField1) =>
    _codeOut(tsCall('timeCodeToStatusForm', [_codeIn(tc), videoField1.toJS]));

/// The short name of a time code field, or null.
int? mmcShortName(int field) => intOrNull(tsCall('mmcShortName', [field.toJS]));

/// The time code field of a short name, or null.
int? mmcFullName(int short) => intOrNull(tsCall('mmcFullName', [short.toJS]));

Uint8List _bytesOrEmpty(JSAny? a) =>
    a == null || a.isUndefinedOrNull ? Uint8List(0) : bytes(a);

RawCommand _commandOut(JSObject o) => (
  _index(_commandKinds, prop(o, 'kind')),
  intOrNull(prop(o, 'register')) ?? 0,
  intOrNull(prop(o, 'destination')) ?? 0,
  intOrNull(prop(o, 'source')) ?? 0,
  _orNull(prop(o, 'target'), _codeOut),
  _bytesOrEmpty(prop(o, 'name')),
  _bytesOrEmpty(prop(o, 'data')),
);

RawField _fieldOut(JSObject o) => (
  _index(_fieldKinds, prop(o, 'kind')),
  intOrNull(prop(o, 'field')) ?? 0,
  _orNull(prop(o, 'code'), _codeOut),
  _bytesOrEmpty(prop(o, 'name')),
  _bytesOrEmpty(prop(o, 'data')),
);

JSObject _commandIn(RawCommand c) => jsObject({
  'kind': _commandKinds[c.$1].toJS,
  'register': c.$2.toJS,
  'destination': c.$3.toJS,
  'source': c.$4.toJS,
  'target': c.$5 == null ? null : _codeIn(c.$5!),
  'name': jsBytes(c.$6),
  'data': jsBytes(c.$7),
});

JSObject _fieldIn(RawField f) => jsObject({
  'kind': _fieldKinds[f.$1].toJS,
  'field': f.$2.toJS,
  'code': f.$3 == null ? null : _codeIn(f.$3!),
  'name': jsBytes(f.$4),
  'data': jsBytes(f.$5),
});

List<RawField> _fields(JSAny? a) => [
  for (final f in (a! as JSArray<JSObject>).toDart) _fieldOut(f),
];

/// The Information Fields of a string.
List<RawField> readMmcFields(Uint8List s) =>
    _fields(tsCall('readMmcFields', [jsBytes(s)]));

/// (response, Device ID, commands, fields), or null.
(bool, int, List<RawCommand>, List<RawField>)? readMmc(Uint8List data) =>
    _orNull(tsCall('readMmc', [jsBytes(data)]), (o) {
      return (
        boolOf(prop(o, 'response')),
        intOf(prop(o, 'deviceId')),
        [
          for (final c in (prop<JSArray<JSObject>>(o, 'commands')).toDart)
            _commandOut(c),
        ],
        _fields(prop(o, 'fields')),
      );
    });

/// The bytes of an MMC command message.
Uint8List mmcCommands(int deviceId, List<RawCommand> commands) => bytes(
  tsCall('mmcCommands', [
    deviceId.toJS,
    [for (final c in commands) _commandIn(c)].toJS,
  ]),
);

/// The bytes of an MMC response message.
Uint8List mmcResponses(int deviceId, List<RawField> fields) => bytes(
  tsCall('mmcResponses', [
    deviceId.toJS,
    [for (final f in fields) _fieldIn(f)].toJS,
  ]),
);

/// The name and data bytes of Information Fields, for WRITE.
Uint8List mmcFields(List<RawField> fields) => bytes(
  tsCall('mmcFields', [
    [for (final f in fields) _fieldIn(f)].toJS,
  ]),
);
