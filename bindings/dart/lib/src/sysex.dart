/// Universal System Exclusive, MIDI Time Code and the MIDI Machine Control transport tier
/// (zmidi-sysex through the C ABI, module Sysex of zmidi-ffi): M1 v4.2.1, RP-024, CA-025, MIDI
/// Tuning, RP-004, RP-013. A reader takes one whole message, with or without F0H and F7H, and gives
/// null for a message of another part; a bad message of its part throws [MalformedException]. A
/// writer gives the bytes of the whole message; a value out of range throws
/// [InvalidValueException].
library;

import 'dart:typed_data';

import 'checks.dart';
import 'errors.dart';
import 'native/sysex.dart' if (dart.library.js_interop) 'web/sysex.dart' as be;

Uint8List _bytes(String name, List<int> data) =>
    Uint8List.fromList(checkBytes(name, data));

/// Checks that [data] has [length] bytes.
Uint8List _fixed(String name, List<int> data, int length) {
  if (data.length != length) {
    throw InvalidValueException('$name has $length bytes, not ${data.length}');
  }
  return _bytes(name, data);
}

bool _same(List<int> a, List<int> b) {
  if (a.length != b.length) {
    return false;
  }
  for (var i = 0; i < a.length; i++) {
    if (a[i] != b[i]) {
      return false;
    }
  }
  return true;
}

/// The C form of 3 bytes (`[id, 0, 0]` or `[00H, b2, b3]`) of 1 or 3 ID bytes.
Uint8List _cId(List<int> id) {
  if (id.length != 1 && id.length != 3) {
    throw InvalidValueException(
      'a Manufacturer ID has 1 or 3 bytes, not ${id.length}',
    );
  }
  final b = _bytes('manufacturer', id);
  return b.length == 1 ? Uint8List.fromList([b[0], 0, 0]) : b;
}

/// The 1 or 3 ID bytes of the C form.
Uint8List _idOf(Uint8List c) =>
    c[0] == 0 ? Uint8List.fromList(c) : Uint8List.fromList([c[0]]);

// --- the sysex base ---

/// The ID class of a System Exclusive message (M1 Table VII).
enum SysexClass {
  /// A Manufacturer ID of 1 or 3 bytes.
  manufacturer,

  /// 7DH: Non-Commercial.
  nonCommercial,

  /// 7EH: Universal Non-Real Time.
  nonRealTime,

  /// 7FH: Universal Real Time.
  realTime,
}

/// One System Exclusive message.
final class SysexInfo {
  const SysexInfo._(
    this.kind,
    this.manufacturer,
    this.deviceId,
    this.subId1,
    this.subId2,
    this.data,
  );

  /// The ID class.
  final SysexClass kind;

  /// The 1 or 3 ID bytes of a manufacturer message, else empty.
  final Uint8List manufacturer;

  /// The Device ID of a Universal message, else null.
  final int? deviceId;

  /// The Sub-ID#1 of a Universal message, else null.
  final int? subId1;

  /// The Sub-ID#2 of a Universal family that has one, else null.
  final int? subId2;

  /// The data after the header (after the ID of a manufacturer or Non-Commercial message), without
  /// F7H.
  final Uint8List data;
}

/// Reads one whole System Exclusive message, with or without F0H and F7H.
SysexInfo parseSysex(List<int> data) {
  final r = be.parseSysex(_bytes('data', data));
  final kind = SysexClass.values[r.$1];
  final universal =
      kind == SysexClass.nonRealTime || kind == SysexClass.realTime;
  return SysexInfo._(
    kind,
    kind == SysexClass.manufacturer ? _idOf(r.$2) : Uint8List(0),
    universal ? r.$3 : null,
    universal ? r.$4 : null,
    universal ? r.$5 : null,
    r.$6,
  );
}

/// True when a received Device ID addresses the device: 7FH or [own].
bool sysexAddressed(int deviceId, int own) => be.sysexAddressed(
  checkInt('deviceId', deviceId, 127),
  checkInt('own', own, 127),
);

/// The bytes of a manufacturer message: F0H, the 1 or 3 ID bytes, [data] and F7H.
Uint8List manufacturerSysex(List<int> manufacturer, List<int> data) =>
    be.manufacturerSysex(_cId(manufacturer), _bytes('data', data));

/// The bytes of a Universal message; [subId2] is given exactly for a family that has one.
Uint8List universalSysex(
  bool realTime,
  int deviceId,
  int subId1,
  int? subId2,
  List<int> data,
) => be.universalSysex(
  realTime,
  checkInt('deviceId', deviceId, 255),
  checkInt('subId1', subId1, 255),
  subId2 == null ? null : checkInt('subId2', subId2, 255),
  _bytes('data', data),
);

// --- inquiry ---

/// The identity of a device in an Identity Reply (M1 General Information).
final class Identity {
  /// An identity: [manufacturer] is 1 byte or 3 bytes 00H b2 b3; [family] and [member] are 14
  /// bits; [revision] is 4 bytes of 7 bits.
  Identity(List<int> manufacturer, int family, int member, List<int> revision)
    : manufacturer = _idOf(_cId(manufacturer)),
      family = checkInt('family', family, 0xffff),
      member = checkInt('member', member, 0xffff),
      revision = _fixed('revision', revision, 4);

  /// 1 byte, or 3 bytes 00H b2 b3.
  final Uint8List manufacturer;

  /// The device family code, 14 bits.
  final int family;

  /// The device family member code, 14 bits.
  final int member;

  /// The software revision level, 4 bytes.
  final Uint8List revision;

  be.RawIdentity get _raw => (_cId(manufacturer), family, member, revision);

  static Identity _of(be.RawIdentity r) =>
      Identity(_idOf(r.$1), r.$2, r.$3, r.$4);

  @override
  bool operator ==(Object other) =>
      other is Identity &&
      _same(other.manufacturer, manufacturer) &&
      other.family == family &&
      other.member == member &&
      _same(other.revision, revision);

  @override
  int get hashCode => Object.hash(
    Object.hashAll(manufacturer),
    family,
    member,
    Object.hashAll(revision),
  );
}

/// An Identity Request (identity null) or an Identity Reply.
final class InquiryMessage {
  const InquiryMessage._(this.isReply, this.deviceId, this.identity);

  /// True for an Identity Reply.
  final bool isReply;

  /// The Device ID.
  final int deviceId;

  /// The identity of a reply, else null.
  final Identity? identity;
}

/// Reads an Identity Request or Reply, or gives null.
InquiryMessage? readInquiry(List<int> data) {
  final r = be.readInquiry(_bytes('data', data));
  return r == null
      ? null
      : InquiryMessage._(r.$1, r.$2, r.$3 == null ? null : Identity._of(r.$3!));
}

/// The bytes of an Identity Request to [deviceId] (7FH: all devices).
Uint8List identityRequest(int deviceId) =>
    be.identityRequest(checkInt('deviceId', deviceId, 255));

/// The bytes of an Identity Reply from [deviceId].
Uint8List identityReply(int deviceId, Identity identity) =>
    be.identityReply(checkInt('deviceId', deviceId, 255), identity._raw);

/// The Identity Reply to an Identity Request that addresses the device ([ownDeviceId] or 7FH), with
/// the own Device ID; null for another message.
Uint8List? respondIdentity(
  List<int> data,
  int ownDeviceId,
  Identity identity,
) => be.respondIdentity(
  _bytes('data', data),
  checkInt('ownDeviceId', ownDeviceId, 127),
  identity._raw,
);

// --- General MIDI System messages ---

/// Reads a General MIDI System message (RP-024 4.9.1: 01H GM1 System On, 02H GM System Off, 03H
/// GM2 System On), or gives null.
({int deviceId, int command})? readGmSystem(List<int> data) {
  final r = be.readGmSystem(_bytes('data', data));
  return r == null ? null : (deviceId: r.$1, command: r.$2);
}

/// The bytes of a General MIDI System message.
Uint8List gmSystem(int deviceId, int command) => be.gmSystem(
  checkInt('deviceId', deviceId, 255),
  checkInt('command', command, 255),
);

// --- Device Control ---

/// The kind of a Device Control message (M1, CA-025).
enum DeviceControlKind {
  /// 04H 01H: Master Volume.
  masterVolume,

  /// 04H 02H: Master Balance.
  masterBalance,

  /// 04H 03H: Master Fine Tuning.
  masterFineTuning,

  /// 04H 04H: Master Coarse Tuning.
  masterCoarseTuning,
}

/// One Device Control message: a 14-bit value, or the 7-bit MSB of Master Coarse Tuning.
final class DeviceControl {
  /// A Device Control message; the C layer checks the ranges.
  DeviceControl(this.kind, int deviceId, int value)
    : deviceId = checkInt('deviceId', deviceId, 255),
      value = checkInt('value', value, 0xffff);

  /// The kind.
  final DeviceControlKind kind;

  /// The Device ID.
  final int deviceId;

  /// The value.
  final int value;

  @override
  bool operator ==(Object other) =>
      other is DeviceControl &&
      other.kind == kind &&
      other.deviceId == deviceId &&
      other.value == value;

  @override
  int get hashCode => Object.hash(kind, deviceId, value);
}

/// Reads a Device Control message, or gives null.
DeviceControl? readDeviceControl(List<int> data) {
  final r = be.readDeviceControl(_bytes('data', data));
  return r == null
      ? null
      : DeviceControl(DeviceControlKind.values[r.$1], r.$2, r.$3);
}

/// The bytes of a Device Control message.
Uint8List deviceControlBytes(DeviceControl control) =>
    be.deviceControlBytes(control.kind.index, control.deviceId, control.value);

/// The offset of a Master Fine Tuning value: 2000H is 0, steps of 100/8192 cent.
int fineTuningOffset(int value) =>
    be.fineTuningOffset(checkInt('value', value, 0xffff));

/// The semitones of a Master Coarse Tuning MSB: 40H is 0.
int coarseTuningSemitones(int msb) =>
    be.coarseTuningSemitones(checkInt('msb', msb, 255));

// --- MIDI Tuning ---

/// A frequency word: the semitone and the 14-bit fraction.
final class FrequencyWord {
  /// A frequency word.
  FrequencyWord(int semitone, int fraction)
    : semitone = checkInt('semitone', semitone, 255),
      fraction = checkInt('fraction', fraction, 0xffff);

  /// The semitone, 0 to 127 (7FH 7FH 7FH: no change).
  final int semitone;

  /// The fraction, 14 bits, steps of 100/16384 cent.
  final int fraction;

  @override
  bool operator ==(Object other) =>
      other is FrequencyWord &&
      other.semitone == semitone &&
      other.fraction == fraction;

  @override
  int get hashCode => Object.hash(semitone, fraction);
}

/// One note of a tuning.
final class TuningNote {
  /// A note: the key and its frequency word.
  TuningNote(int key, int semitone, int fraction)
    : key = checkInt('key', key, 255),
      semitone = checkInt('semitone', semitone, 255),
      fraction = checkInt('fraction', fraction, 0xffff);

  /// The MIDI key number.
  final int key;

  /// The semitone of the frequency word.
  final int semitone;

  /// The fraction of the frequency word.
  final int fraction;

  @override
  bool operator ==(Object other) =>
      other is TuningNote &&
      other.key == key &&
      other.semitone == semitone &&
      other.fraction == fraction;

  @override
  int get hashCode => Object.hash(key, semitone, fraction);
}

/// The kind of a MIDI Tuning message.
enum TuningKind {
  /// 08H 00H or 03H: Bulk Tuning Dump Request.
  dumpRequest,

  /// 08H 01H or 04H: a key-based tuning dump of 128 notes.
  keyDump,

  /// 08H 02H or 07H: Single Note Tuning Change.
  noteChange,

  /// 08H 05H or 06H: Scale/Octave Tuning Dump.
  scaleDump,

  /// 08H 08H or 09H: Scale/Octave Tuning.
  scaleOctave,
}

/// One MIDI Tuning message; the fields that the kind does not have are 0, null or empty.
final class TuningMessage {
  const TuningMessage._(
    this.kind,
    this.realTime,
    this.deviceId,
    this.bank,
    this.program,
    this.name,
    this.checksumOk,
    this.channels,
    this.twoByte,
    this.offsets,
    this.notes,
  );

  /// The kind.
  final TuningKind kind;

  /// True for a Real Time message.
  final bool realTime;

  /// The Device ID.
  final int deviceId;

  /// The tuning bank, or null for a message without a bank.
  final int? bank;

  /// The tuning program.
  final int program;

  /// The 16-byte name of a dump.
  final Uint8List name;

  /// The checksum result of a 01H Bulk Tuning Dump.
  final bool checksumOk;

  /// The channel mask of scaleOctave.
  final int channels;

  /// True for the 2-byte offsets.
  final bool twoByte;

  /// The 12 offsets of scaleDump and scaleOctave.
  final List<int> offsets;

  /// The notes of keyDump (128) and noteChange.
  final List<TuningNote> notes;
}

/// Reads a MIDI Tuning message, or gives null.
TuningMessage? readTuning(List<int> data) {
  final r = be.readTuning(_bytes('data', data));
  if (r == null) {
    return null;
  }
  return TuningMessage._(
    TuningKind.values[r.$1],
    r.$2,
    r.$3,
    r.$4,
    r.$5,
    r.$6,
    r.$7,
    r.$8,
    r.$9,
    r.$10,
    [for (final n in r.$11) TuningNote(n.$1, n.$2, n.$3)],
  );
}

int? _bank(int? bank) => bank == null ? null : checkInt('bank', bank, 255);

List<int> _offsets(List<int> offsets) {
  if (offsets.length != 12) {
    throw InvalidValueException('offsets has 12 values, not ${offsets.length}');
  }
  for (final o in offsets) {
    if (o < -32768 || o > 32767) {
      throw InvalidValueException('an offset is 16 bits, not $o');
    }
  }
  return offsets;
}

/// The bytes of a Bulk Tuning Dump Request (00H, or 03H with a bank).
Uint8List tuningDumpRequest(int deviceId, int program, [int? bank]) =>
    be.tuningDumpRequest(
      checkInt('deviceId', deviceId, 255),
      checkInt('program', program, 255),
      _bank(bank),
    );

/// The bytes of a key-based tuning dump (01H, or 04H with a bank) of 128 frequency words and a
/// 16-byte name.
Uint8List tuningKeyDump(
  int deviceId,
  int program,
  List<int> name,
  List<FrequencyWord> words, [
  int? bank,
]) {
  if (words.length != 128) {
    throw InvalidValueException('words has 128 values, not ${words.length}');
  }
  return be.tuningKeyDump(
    checkInt('deviceId', deviceId, 255),
    checkInt('program', program, 255),
    _fixed('name', name, 16),
    [for (final w in words) (w.semitone, w.fraction)],
    _bank(bank),
  );
}

/// The bytes of a Single Note Tuning Change (02H, or 07H with a bank).
Uint8List tuningNoteChange(
  bool realTime,
  int deviceId,
  int program,
  List<TuningNote> notes, [
  int? bank,
]) => be.tuningNoteChange(
  realTime,
  checkInt('deviceId', deviceId, 255),
  checkInt('program', program, 255),
  [for (final n in notes) (n.key, n.semitone, n.fraction)],
  _bank(bank),
);

/// The bytes of a Scale/Octave Tuning Dump (05H 1-byte, 06H 2-byte) with a 16-byte name.
Uint8List tuningScaleDump(
  int deviceId,
  int bank,
  int program,
  List<int> name,
  List<int> offsets,
  bool twoByte,
) => be.tuningScaleDump(
  checkInt('deviceId', deviceId, 255),
  checkInt('bank', bank, 255),
  checkInt('program', program, 255),
  _fixed('name', name, 16),
  _offsets(offsets),
  twoByte,
);

/// The bytes of a Scale/Octave Tuning message (08H 1-byte, 09H 2-byte) for the [channels] mask.
Uint8List tuningScaleOctave(
  bool realTime,
  int deviceId,
  int channels,
  List<int> offsets,
  bool twoByte,
) => be.tuningScaleOctave(
  realTime,
  checkInt('deviceId', deviceId, 255),
  checkInt('channels', channels, 0xffff),
  _offsets(offsets),
  twoByte,
);

/// The checksum of a tuning dump: the XOR of all bytes after F0H and before the checksum, AND
/// 7FH.
int tuningChecksum(List<int> data) => be.tuningChecksum(_bytes('data', data));

/// The frequency word of its 3 bytes.
FrequencyWord frequencyWord(List<int> data) {
  final r = be.frequencyWord(_fixed('data', data, 3));
  return FrequencyWord(r.$1, r.$2);
}

/// The 3 bytes of a frequency word.
Uint8List frequencyWordBytes(int semitone, int fraction) =>
    be.frequencyWordBytes(
      checkInt('semitone', semitone, 255),
      checkInt('fraction', fraction, 0xffff),
    );

// --- MIDI Time Code ---

/// An MTC time: [rate] 0 (24), 1 (25), 2 (30 drop-frame) or 3 (30 frames per second).
final class MtcTime {
  /// A time; each count is a byte, and the C layer checks the ranges.
  MtcTime(int hours, int minutes, int seconds, int frames, int rate)
    : hours = checkInt('hours', hours, 255),
      minutes = checkInt('minutes', minutes, 255),
      seconds = checkInt('seconds', seconds, 255),
      frames = checkInt('frames', frames, 255),
      rate = checkInt('rate', rate, 255);

  static MtcTime _of(be.RawTime t) => MtcTime(t.$1, t.$2, t.$3, t.$4, t.$5);

  /// The hours.
  final int hours;

  /// The minutes.
  final int minutes;

  /// The seconds.
  final int seconds;

  /// The frames.
  final int frames;

  /// The rate code.
  final int rate;

  be.RawTime get _raw => (hours, minutes, seconds, frames, rate);

  @override
  bool operator ==(Object other) =>
      other is MtcTime &&
      other.hours == hours &&
      other.minutes == minutes &&
      other.seconds == seconds &&
      other.frames == frames &&
      other.rate == rate;

  @override
  int get hashCode => Object.hash(hours, minutes, seconds, frames, rate);

  @override
  String toString() => 'MtcTime($hours:$minutes:$seconds:$frames, rate $rate)';
}

/// True when each count of [t] is in range and the frame number is not one that drop-frame
/// skips.
bool mtcTimeIsValid(MtcTime t) => be.mtcTimeIsValid(t._raw);

/// The next valid time: [t], or the first frame after a frame number that drop-frame skips.
MtcTime mtcNextValid(MtcTime t) => MtcTime._of(be.mtcNextValid(t._raw));

/// The count of frames from 00:00:00:00 to [t].
int mtcToFrameCount(MtcTime t) => be.mtcToFrameCount(t._raw);

/// The time of a frame count at [rate].
MtcTime mtcFromFrameCount(int count, int rate) => MtcTime._of(
  be.mtcFromFrameCount(
    checkInt('count', count, 0xffffffff),
    checkInt('rate', rate, 255),
  ),
);

/// The time [delta] frames after [t] (before it for a negative delta), modulo 24 hours.
MtcTime mtcAddFrames(MtcTime t, int delta) {
  if (delta < -0x80000000 || delta > 0x7fffffff) {
    throw InvalidValueException('delta is 32 bits, not $delta');
  }
  return MtcTime._of(be.mtcAddFrames(t._raw, delta));
}

/// The 8 Quarter Frame data bytes of [t], piece 0 to 7.
Uint8List mtcQuarterFrames(MtcTime t) => be.mtcQuarterFrames(t._raw);

/// The Quarter Frame generator (RP-004 p. 3).
final class MtcGenerator {
  /// A running generator: the first sequence encodes [time] from piece 0, or from piece 7 when
  /// [reverse].
  MtcGenerator(MtcTime time, {bool reverse = false})
    : _b = be.MtcGeneratorCore(time._raw, reverse);

  final be.MtcGeneratorCore _b;
  bool _freed = false;

  /// The data byte of the next piece, or null while stopped.
  int? nextByte() {
    checkFreed(_freed, this);
    return _b.nextByte();
  }

  /// Runs or stops the generator.
  void setRunning(bool running) {
    checkFreed(_freed, this);
    _b.setRunning(running);
  }

  /// Sets the direction; the next piece is the next one in the new order.
  void setDirection(bool reverse) {
    checkFreed(_freed, this);
    _b.setDirection(reverse);
  }

  /// A new time; the next piece is the first piece of the direction.
  void locate(MtcTime time) {
    checkFreed(_freed, this);
    _b.locate(time._raw);
  }

  /// The time that the current or next sequence encodes at its piece 0.
  MtcTime get time {
    checkFreed(_freed, this);
    return MtcTime._of(_b.time());
  }

  /// Frees the memory of the generator now.
  void free() {
    if (!_freed) {
      _freed = true;
      _b.free();
    }
  }
}

/// What one Quarter Frame data byte gives; [raw] and [current] at the end of a full sequence only.
final class MtcEvent {
  const MtcEvent._(
    this.piece,
    this.boundary,
    this.raw,
    this.current,
    this.reverse,
    this.discontinuity,
  );

  /// The piece number, 0 to 7.
  final int piece;

  /// True for pieces 0 and 4, which mark frame boundaries (RP-004 p. 3).
  final bool boundary;

  /// The time of the sequence as received.
  final MtcTime? raw;

  /// The raw time plus 2 frames forward, minus 1 frame in reverse.
  final MtcTime? current;

  /// True in reverse.
  final bool reverse;

  /// True when [raw] is not the previous raw time plus 2 frames (minus 2 in reverse).
  final bool discontinuity;
}

/// The direction of an MTC reader.
enum MtcDirection {
  /// Forward: pieces 0 to 7.
  forward,

  /// Reverse: pieces 7 to 0.
  reverse,
}

/// The state of an MTC reader.
final class MtcReaderInfo {
  const MtcReaderInfo._(this.locked, this.running, this.direction, this.time);

  /// True after a full sequence, until a broken order or a direction change.
  final bool locked;

  /// True after a Quarter Frame data byte, false after a Full Message.
  final bool running;

  /// The direction, once two pieces gave it; else null.
  final MtcDirection? direction;

  /// The current time of the last full sequence, or the time of a Full Message after it.
  final MtcTime? time;
}

MtcTime? _timeOrNull(be.RawTime? t) => t == null ? null : MtcTime._of(t);

/// The Quarter Frame reader (RP-004 p. 3): lock, direction and the current time.
final class MtcReader {
  /// A reader that is not locked and is stopped.
  MtcReader() : _b = be.MtcReaderCore();

  final be.MtcReaderCore _b;
  bool _freed = false;

  /// Takes one Quarter Frame data byte, 0 to 127.
  MtcEvent receive(int byte) {
    checkFreed(_freed, this);
    final e = _b.receive(checkInt('byte', byte, 255));
    return MtcEvent._(
      e.$1,
      e.$2,
      _timeOrNull(e.$3),
      _timeOrNull(e.$4),
      e.$5,
      e.$6,
    );
  }

  /// Takes the time of a Full Message: the reader has this time and is stopped until the next
  /// data byte.
  void fullMessage(MtcTime time) {
    checkFreed(_freed, this);
    _b.fullMessage(time._raw);
  }

  /// The lock, the run state, the direction and the time.
  MtcReaderInfo get info {
    checkFreed(_freed, this);
    final i = _b.info();
    return MtcReaderInfo._(
      i.$1,
      i.$2,
      i.$3 == 0 ? null : MtcDirection.values[i.$3 - 1],
      _timeOrNull(i.$4),
    );
  }

  /// Frees the memory of the reader now.
  void free() {
    if (!_freed) {
      _freed = true;
      _b.free();
    }
  }
}

/// Reads an MTC Full Message (RP-004 p. 5), or gives null.
({int deviceId, MtcTime time})? readMtcFull(List<int> data) {
  final r = be.readMtcFull(_bytes('data', data));
  return r == null ? null : (deviceId: r.$1, time: MtcTime._of(r.$2));
}

/// The bytes of an MTC Full Message.
Uint8List mtcFull(int deviceId, MtcTime time) =>
    be.mtcFull(checkInt('deviceId', deviceId, 255), time._raw);

/// Reads an MTC User Bits message (RP-004 p. 6), or gives null.
({int deviceId, Uint8List groups, int flags})? readMtcUserBits(List<int> data) {
  final r = be.readMtcUserBits(_bytes('data', data));
  return r == null ? null : (deviceId: r.$1, groups: r.$2, flags: r.$3);
}

/// The bytes of an MTC User Bits message: 8 groups of 4 bits and the 2 flag bits.
Uint8List mtcUserBits(int deviceId, List<int> groups, int flags) =>
    be.mtcUserBits(
      checkInt('deviceId', deviceId, 255),
      _fixed('groups', groups, 8),
      checkInt('flags', flags, 255),
    );

// --- the MMC transport tier ---

/// An MMC Standard Time Code (RP-013 pp. 8, 9): the subframes form, or the status form with its
/// flags.
final class TimeCode {
  /// A time code; the defaults give the subframes form with subframes 0.
  TimeCode(
    this.time, {
    this.colorFrame = false,
    this.blank = false,
    this.negative = false,
    this.statusForm = false,
    int subframes = 0,
    this.estimated = false,
    this.invalid = false,
    this.videoField1 = false,
    this.noTimeCode = false,
  }) : subframes = checkInt('subframes', subframes, 255);

  static TimeCode _of(be.RawTimeCode c) => TimeCode(
    MtcTime._of(c.$1),
    colorFrame: c.$2,
    blank: c.$3,
    negative: c.$4,
    statusForm: c.$5,
    subframes: c.$6,
    estimated: c.$7,
    invalid: c.$8,
    videoField1: c.$9,
    noTimeCode: c.$10,
  );

  /// The time.
  final MtcTime time;

  /// The color frame bit.
  final bool colorFrame;

  /// The blank bit: the time code was never set.
  final bool blank;

  /// The sign bit.
  final bool negative;

  /// True for the status form; false for the subframes form.
  final bool statusForm;

  /// The subframes of the subframes form, 0 to 99.
  final int subframes;

  /// The estimated code flag of the status form.
  final bool estimated;

  /// The invalid code flag of the status form.
  final bool invalid;

  /// The video field 1 flag of the status form.
  final bool videoField1;

  /// The no time code flag of the status form.
  final bool noTimeCode;

  be.RawTimeCode get _raw => (
    time._raw,
    colorFrame,
    blank,
    negative,
    statusForm,
    subframes,
    estimated,
    invalid,
    videoField1,
    noTimeCode,
  );

  @override
  bool operator ==(Object other) => other is TimeCode && other._raw == _raw;

  @override
  int get hashCode => _raw.hashCode;
}

/// Reads a Standard Time Code from its 5 bytes; the counts are as received.
TimeCode timeCodeFromBytes(List<int> data) =>
    TimeCode._of(be.timeCodeFromBytes(_fixed('data', data, 5)));

/// The 5 bytes of a time code.
Uint8List timeCodeBytes(TimeCode tc) => be.timeCodeBytes(tc._raw);

/// [last] with a short time code of 2 bytes (RP-013 p. 9) applied.
TimeCode timeCodeWithShort(TimeCode last, List<int> data) =>
    TimeCode._of(be.timeCodeWithShort(last._raw, _fixed('data', data, 2)));

/// The 2 bytes of the short form of [tc].
Uint8List timeCodeShortBytes(TimeCode tc) => be.timeCodeShortBytes(tc._raw);

/// The MOVE rule (RP-013 p. 32) to the subframes form: subframes 0.
TimeCode timeCodeToSubframesForm(TimeCode tc) =>
    TimeCode._of(be.timeCodeToSubframesForm(tc._raw));

/// The MOVE rule (RP-013 p. 32) to the status form: no time code, with [videoField1].
TimeCode timeCodeToStatusForm(TimeCode tc, bool videoField1) =>
    TimeCode._of(be.timeCodeToStatusForm(tc._raw, videoField1));

/// The short name of a time code field: 01H to 1FH give the name plus 20H (RP-013 p. 9); else
/// null.
int? mmcShortName(int field) => be.mmcShortName(checkInt('field', field, 255));

/// The time code field of a short name: 21H to 3FH give the name minus 20H; else null.
int? mmcFullName(int short) => be.mmcFullName(checkInt('short', short, 255));

/// The kind of an MMC command (RP-013 Set 1, PLAY and PAUSE).
enum MmcCommandKind {
  /// 01H: STOP.
  stop,

  /// 02H: PLAY.
  play,

  /// 03H: DEFERRED PLAY.
  deferredPlay,

  /// 04H: FAST FORWARD.
  fastForward,

  /// 05H: REWIND.
  rewind,

  /// 06H: RECORD STROBE.
  recordStrobe,

  /// 07H: RECORD EXIT.
  recordExit,

  /// 09H: PAUSE.
  pause,

  /// 0DH: MMC RESET.
  reset,

  /// 40H: WRITE, with an Information Field string.
  write,

  /// 44H 02H 00H: LOCATE [I/F].
  locateField,

  /// 44H 06H 01H: LOCATE [TARGET].
  locateTarget,

  /// 4CH: MOVE.
  move,

  /// Another command, with its name and data.
  other,
}

/// One MMC command; [data] is the Information Field string of write, or the data of other.
final class MmcCommand {
  /// A command; [register] is the field of locateField, [destination] and [source] are the
  /// fields of move.
  MmcCommand(
    this.kind, {
    int register = 0,
    int destination = 0,
    int source = 0,
    this.target,
    List<int> name = const [],
    List<int> data = const [],
  }) : register = checkInt('register', register, 255),
       destination = checkInt('destination', destination, 255),
       source = checkInt('source', source, 255),
       name = _bytes('name', name),
       data = _bytes('data', data);

  /// The kind.
  final MmcCommandKind kind;

  /// The field of locateField.
  final int register;

  /// The destination field of move.
  final int destination;

  /// The source field of move.
  final int source;

  /// The time code of locateTarget.
  final TimeCode? target;

  /// The command bytes of other.
  final Uint8List name;

  /// The data of write and other.
  final Uint8List data;

  be.RawCommand get _raw =>
      (kind.index, register, destination, source, target?._raw, name, data);

  static MmcCommand _of(be.RawCommand c) => MmcCommand(
    MmcCommandKind.values[c.$1],
    register: c.$2,
    destination: c.$3,
    source: c.$4,
    target: c.$5 == null ? null : TimeCode._of(c.$5!),
    name: c.$6,
    data: c.$7,
  );
}

/// The kind of an MMC Information Field.
enum MmcFieldKind {
  /// A time code field (01H to 1FH) with a Standard Time Code.
  timeCode,

  /// A short time code field (21H to 3FH).
  short,

  /// Another field, with its name and data.
  other,
}

/// One MMC Information Field; of short, [code] has only the frames, the sign and the final byte.
final class MmcField {
  /// A field.
  MmcField(
    this.kind, {
    int field = 0,
    this.code,
    List<int> name = const [],
    List<int> data = const [],
  }) : field = checkInt('field', field, 255),
       name = _bytes('name', name),
       data = _bytes('data', data);

  /// The kind.
  final MmcFieldKind kind;

  /// The field name byte of timeCode and short.
  final int field;

  /// The time code of timeCode and short.
  final TimeCode? code;

  /// The name bytes as received.
  final Uint8List name;

  /// The data bytes as received.
  final Uint8List data;

  be.RawField get _raw => (kind.index, field, code?._raw, name, data);

  static MmcField _of(be.RawField f) => MmcField(
    MmcFieldKind.values[f.$1],
    field: f.$2,
    code: f.$3 == null ? null : TimeCode._of(f.$3!),
    name: f.$4,
    data: f.$5,
  );
}

/// An MMC command or response message.
final class MmcMessage {
  const MmcMessage._(this.response, this.deviceId, this.commands, this.fields);

  /// True for a response (Sub-ID#1 07H).
  final bool response;

  /// The Device ID.
  final int deviceId;

  /// The commands of a command message.
  final List<MmcCommand> commands;

  /// The fields of a response.
  final List<MmcField> fields;
}

/// Reads an MMC message, or gives null; a string cut short throws [MalformedException].
MmcMessage? readMmc(List<int> data) {
  final r = be.readMmc(_bytes('data', data));
  if (r == null) {
    return null;
  }
  return MmcMessage._(
    r.$1,
    r.$2,
    [for (final c in r.$3) MmcCommand._of(c)],
    [for (final f in r.$4) MmcField._of(f)],
  );
}

/// Reads the Information Fields of a response string or of the data of WRITE.
List<MmcField> readMmcFields(List<int> bytes) => [
  for (final f in be.readMmcFields(_bytes('bytes', bytes))) MmcField._of(f),
];

/// The bytes of an MMC command message to [deviceId] (7FH: all devices).
Uint8List mmcCommands(int deviceId, List<MmcCommand> commands) =>
    be.mmcCommands(checkInt('deviceId', deviceId, 255), [
      for (final c in commands) c._raw,
    ]);

/// The bytes of an MMC response message from [deviceId] (00H to 7EH).
Uint8List mmcResponses(int deviceId, List<MmcField> fields) => be.mmcResponses(
  checkInt('deviceId', deviceId, 255),
  [for (final f in fields) f._raw],
);

/// The name and data bytes of Information Fields, for the data of a WRITE command.
Uint8List mmcFields(List<MmcField> fields) =>
    be.mmcFields([for (final f in fields) f._raw]);
