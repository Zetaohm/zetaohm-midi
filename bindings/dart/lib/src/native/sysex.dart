/// The native form of the sysex parts: zmidi-sysex through dart:ffi and the C ABI (module Sysex of
/// zmidi-ffi). The values are checked by `sysex.dart`.
library;

import 'dart:ffi';
import 'dart:typed_data';

import 'package:ffi/ffi.dart';

import '../bindings.g.dart';
import '../errors.dart';
import '../native.dart';

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

/// The size of a writer output: every message of the parts fits.
const _out = 600;

/// Calls a C writer with an output of 600 bytes and gives the bytes.
Uint8List _written(
  Allocator arena,
  String what,
  ZmidiStatus Function(Pointer<Uint8> out, Pointer<UintPtr> length) f,
) {
  final out = arena<Uint8>(_out);
  final len = arena<UintPtr>();
  check(f(out, len), what);
  return bytesOut(out, len.value);
}

/// Calls a C part reader: null for a message of another part (REQ-FFI-SX-002).
T? _read<T>(String what, ZmidiStatus status, T Function() ok) {
  if (status == ZmidiStatus.ZMIDI_STATUS_NO_PENDING) {
    return null;
  }
  if (status == ZmidiStatus.ZMIDI_STATUS_INVALID_ARGUMENT) {
    throw MalformedException('$what: not a valid message');
  }
  check(status, what);
  return ok();
}

Uint8List _array(Array<Uint8> a, int n) =>
    Uint8List.fromList([for (var i = 0; i < n; i++) a[i]]);

/// (kind, manufacturer in 3 bytes, Device ID, Sub-ID#1, Sub-ID#2 or null, data).
(int, Uint8List, int, int, int?, Uint8List) parseSysex(Uint8List data) =>
    using((arena) {
      final p = bytesIn(arena, data);
      final s = arena<ZmidiSysex>();
      if (zmidi_sysex_parse(p, data.length, s) != ZmidiStatus.ZMIDI_STATUS_OK) {
        throw MalformedException(
          'parseSysex: not a valid System Exclusive message',
        );
      }
      final r = s.ref;
      final at = r.data_offset;
      return (
        r.kind,
        _array(r.manufacturer, 3),
        r.device_id,
        r.sub_id1,
        r.has_sub_id2 ? r.sub_id2 : null,
        Uint8List.fromList(data.sublist(at, at + r.data_length)),
      );
    });

/// The Device ID test.
bool sysexAddressed(int deviceId, int own) => using((arena) {
  final r = arena<Bool>();
  check(zmidi_sysex_addressed(deviceId, own, r), 'sysexAddressed');
  return r.value;
});

/// The bytes of a manufacturer message; [manufacturer] in the C form of 3 bytes.
Uint8List manufacturerSysex(Uint8List manufacturer, Uint8List data) =>
    using((arena) {
      final m = bytesIn(arena, manufacturer);
      final d = bytesIn(arena, data);
      return _written(
        arena,
        'manufacturerSysex',
        (o, l) => zmidi_sysex_write_manufacturer(m, d, data.length, o, _out, l),
      );
    });

/// The bytes of a Universal message.
Uint8List universalSysex(
  bool realTime,
  int deviceId,
  int subId1,
  int? subId2,
  Uint8List data,
) => using((arena) {
  final d = bytesIn(arena, data);
  return _written(
    arena,
    'universalSysex',
    (o, l) => zmidi_sysex_write_universal(
      realTime,
      deviceId,
      subId1,
      subId2 != null,
      subId2 ?? 0,
      d,
      data.length,
      o,
      _out,
      l,
    ),
  );
});

void _identityIn(Pointer<ZmidiIdentity> p, RawIdentity i) {
  for (var k = 0; k < 3; k++) {
    p.ref.manufacturer[k] = i.$1[k];
  }
  p.ref
    ..family = i.$2
    ..member = i.$3;
  for (var k = 0; k < 4; k++) {
    p.ref.revision[k] = i.$4[k];
  }
}

RawIdentity _identityOut(Pointer<ZmidiIdentity> p) => (
  _array(p.ref.manufacturer, 3),
  p.ref.family,
  p.ref.member,
  _array(p.ref.revision, 4),
);

/// (reply, Device ID, identity of a reply), or null.
(bool, int, RawIdentity?)? readInquiry(Uint8List data) => using((arena) {
  final p = bytesIn(arena, data);
  final reply = arena<Bool>();
  final dev = arena<Uint8>();
  final id = arena<ZmidiIdentity>();
  return _read(
    'readInquiry',
    zmidi_inquiry_read(p, data.length, reply, dev, id),
    () => (reply.value, dev.value, reply.value ? _identityOut(id) : null),
  );
});

/// The bytes of an Identity Request.
Uint8List identityRequest(int deviceId) => using(
  (arena) => _written(
    arena,
    'identityRequest',
    (o, l) => zmidi_inquiry_request_write(deviceId, o, _out, l),
  ),
);

/// The bytes of an Identity Reply.
Uint8List identityReply(int deviceId, RawIdentity identity) => using((arena) {
  final id = arena<ZmidiIdentity>();
  _identityIn(id, identity);
  return _written(
    arena,
    'identityReply',
    (o, l) => zmidi_inquiry_reply_write(deviceId, id, o, _out, l),
  );
});

/// The Identity Reply to a request for the device, or null.
Uint8List? respondIdentity(Uint8List data, int own, RawIdentity identity) =>
    using((arena) {
      final p = bytesIn(arena, data);
      final id = arena<ZmidiIdentity>();
      _identityIn(id, identity);
      final out = arena<Uint8>(_out);
      final len = arena<UintPtr>();
      return _read(
        'respondIdentity',
        zmidi_inquiry_respond(p, data.length, own, id, out, _out, len),
        () => bytesOut(out, len.value),
      );
    });

/// (Device ID, command), or null.
(int, int)? readGmSystem(Uint8List data) => using((arena) {
  final p = bytesIn(arena, data);
  final dev = arena<Uint8>();
  final cmd = arena<Uint8>();
  return _read(
    'readGmSystem',
    zmidi_gm_system_read(p, data.length, dev, cmd),
    () => (dev.value, cmd.value),
  );
});

/// The bytes of a General MIDI System message.
Uint8List gmSystem(int deviceId, int command) => using(
  (arena) => _written(
    arena,
    'gmSystem',
    (o, l) => zmidi_gm_system_write(deviceId, command, o, _out, l),
  ),
);

/// (kind, Device ID, value), or null.
(int, int, int)? readDeviceControl(Uint8List data) => using((arena) {
  final p = bytesIn(arena, data);
  final dc = arena<ZmidiDeviceControl>();
  return _read(
    'readDeviceControl',
    zmidi_device_control_read(p, data.length, dc),
    () => (dc.ref.kind, dc.ref.device_id, dc.ref.value),
  );
});

/// The bytes of a Device Control message.
Uint8List deviceControlBytes(int kind, int deviceId, int value) =>
    using((arena) {
      final dc = arena<ZmidiDeviceControl>();
      dc.ref
        ..kind = kind
        ..device_id = deviceId
        ..value = value;
      return _written(
        arena,
        'deviceControlBytes',
        (o, l) => zmidi_device_control_write(dc, o, _out, l),
      );
    });

/// The offset of a Master Fine Tuning value.
int fineTuningOffset(int value) => using((arena) {
  final r = arena<Int16>();
  check(zmidi_fine_tuning_offset(value, r), 'fineTuningOffset');
  return r.value;
});

/// The semitones of a Master Coarse Tuning MSB.
int coarseTuningSemitones(int msb) => using((arena) {
  final r = arena<Int8>();
  check(zmidi_coarse_tuning_semitones(msb, r), 'coarseTuningSemitones');
  return r.value;
});

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
readTuning(Uint8List data) => using((arena) {
  final p = bytesIn(arena, data);
  final t = arena<ZmidiTuning>();
  final notes = arena<ZmidiTuningNote>(128);
  return _read(
    'readTuning',
    zmidi_tuning_read(p, data.length, t, notes, 128),
    () {
      final r = t.ref;
      final named = r.kind == 1 || r.kind == 3;
      final scale = r.kind == 3 || r.kind == 4;
      return (
        r.kind,
        r.real_time,
        r.device_id,
        r.has_bank ? r.bank : null,
        r.program,
        named ? _array(r.name, 16) : Uint8List(0),
        r.checksum_ok,
        r.channels,
        r.offsets.two_byte,
        scale ? [for (var i = 0; i < 12; i++) r.offsets.offsets[i]] : <int>[],
        [
          for (var i = 0; i < r.count; i++)
            (
              (notes + i).ref.key,
              (notes + i).ref.word.semitone,
              (notes + i).ref.word.fraction,
            ),
        ],
      );
    },
  );
});

/// The bytes of a Bulk Tuning Dump Request.
Uint8List tuningDumpRequest(int deviceId, int program, int? bank) => using(
  (arena) => _written(
    arena,
    'tuningDumpRequest',
    (o, l) => zmidi_tuning_dump_request_write(
      deviceId,
      bank != null,
      bank ?? 0,
      program,
      o,
      _out,
      l,
    ),
  ),
);

/// The bytes of a key-based tuning dump from 128 (semitone, fraction) words.
Uint8List tuningKeyDump(
  int deviceId,
  int program,
  Uint8List name,
  List<(int, int)> words,
  int? bank,
) => using((arena) {
  final pn = bytesIn(arena, name);
  final pw = arena<ZmidiFrequencyWord>(128);
  for (var i = 0; i < 128; i++) {
    (pw + i).ref
      ..semitone = words[i].$1
      ..fraction = words[i].$2;
  }
  return _written(
    arena,
    'tuningKeyDump',
    (o, l) => zmidi_tuning_key_dump_write(
      deviceId,
      bank != null,
      bank ?? 0,
      program,
      pn,
      pw,
      o,
      _out,
      l,
    ),
  );
});

/// The bytes of a Single Note Tuning Change from (key, semitone, fraction) notes.
Uint8List tuningNoteChange(
  bool realTime,
  int deviceId,
  int program,
  List<(int, int, int)> notes,
  int? bank,
) => using((arena) {
  final pn = arena<ZmidiTuningNote>(notes.isEmpty ? 1 : notes.length);
  for (var i = 0; i < notes.length; i++) {
    (pn + i).ref
      ..key = notes[i].$1
      ..word.semitone = notes[i].$2
      ..word.fraction = notes[i].$3;
  }
  return _written(
    arena,
    'tuningNoteChange',
    (o, l) => zmidi_tuning_note_change_write(
      realTime,
      deviceId,
      bank != null,
      bank ?? 0,
      program,
      pn,
      notes.length,
      o,
      _out,
      l,
    ),
  );
});

Pointer<ZmidiTuningOffsets> _offsetsIn(
  Allocator arena,
  List<int> offsets,
  bool twoByte,
) {
  final p = arena<ZmidiTuningOffsets>();
  p.ref.two_byte = twoByte;
  for (var i = 0; i < 12; i++) {
    p.ref.offsets[i] = offsets[i];
  }
  return p;
}

/// The bytes of a Scale/Octave Tuning Dump.
Uint8List tuningScaleDump(
  int deviceId,
  int bank,
  int program,
  Uint8List name,
  List<int> offsets,
  bool twoByte,
) => using((arena) {
  final pn = bytesIn(arena, name);
  final po = _offsetsIn(arena, offsets, twoByte);
  return _written(
    arena,
    'tuningScaleDump',
    (o, l) => zmidi_tuning_scale_dump_write(
      deviceId,
      bank,
      program,
      pn,
      po,
      o,
      _out,
      l,
    ),
  );
});

/// The bytes of a Scale/Octave Tuning message.
Uint8List tuningScaleOctave(
  bool realTime,
  int deviceId,
  int channels,
  List<int> offsets,
  bool twoByte,
) => using((arena) {
  final po = _offsetsIn(arena, offsets, twoByte);
  return _written(
    arena,
    'tuningScaleOctave',
    (o, l) => zmidi_tuning_scale_octave_write(
      realTime,
      deviceId,
      channels,
      po,
      o,
      _out,
      l,
    ),
  );
});

/// The checksum of the bytes of a tuning dump.
int tuningChecksum(Uint8List data) => using((arena) {
  final p = bytesIn(arena, data);
  final c = arena<Uint8>();
  check(zmidi_tuning_checksum(p, data.length, c), 'tuningChecksum');
  return c.value;
});

/// The (semitone, fraction) of three bytes.
(int, int) frequencyWord(Uint8List data) => using((arena) {
  final p = bytesIn(arena, data);
  final w = arena<ZmidiFrequencyWord>();
  check(zmidi_frequency_word_from_bytes(p, w), 'frequencyWord');
  return (w.ref.semitone, w.ref.fraction);
});

/// The three bytes of a frequency word.
Uint8List frequencyWordBytes(int semitone, int fraction) => using((arena) {
  final w = arena<ZmidiFrequencyWord>();
  w.ref
    ..semitone = semitone
    ..fraction = fraction;
  final b = arena<Uint8>(3);
  check(zmidi_frequency_word_to_bytes(w, b), 'frequencyWordBytes');
  return bytesOut(b, 3);
});

void _timeIn(ZmidiMtcTime t, RawTime r) {
  t
    ..hours = r.$1
    ..minutes = r.$2
    ..seconds = r.$3
    ..frames = r.$4
    ..rate = r.$5;
}

RawTime _timeOut(ZmidiMtcTime t) =>
    (t.hours, t.minutes, t.seconds, t.frames, t.rate);

Pointer<ZmidiMtcTime> _time(Allocator arena, RawTime r) {
  final p = arena<ZmidiMtcTime>();
  _timeIn(p.ref, r);
  return p;
}

/// The validity of a time.
bool mtcTimeIsValid(RawTime t) => using((arena) {
  final r = arena<Bool>();
  check(zmidi_mtc_time_is_valid(_time(arena, t), r), 'mtcTimeIsValid');
  return r.value;
});

/// The next valid time.
RawTime mtcNextValid(RawTime t) => using((arena) {
  final o = arena<ZmidiMtcTime>();
  check(zmidi_mtc_next_valid(_time(arena, t), o), 'mtcNextValid');
  return _timeOut(o.ref);
});

/// The frame count of a time.
int mtcToFrameCount(RawTime t) => using((arena) {
  final c = arena<Uint32>();
  check(zmidi_mtc_to_frame_count(_time(arena, t), c), 'mtcToFrameCount');
  return c.value;
});

/// The time of a frame count.
RawTime mtcFromFrameCount(int count, int rate) => using((arena) {
  final o = arena<ZmidiMtcTime>();
  check(zmidi_mtc_from_frame_count(count, rate, o), 'mtcFromFrameCount');
  return _timeOut(o.ref);
});

/// The time some frames later or earlier.
RawTime mtcAddFrames(RawTime t, int delta) => using((arena) {
  final o = arena<ZmidiMtcTime>();
  check(zmidi_mtc_add_frames(_time(arena, t), delta, o), 'mtcAddFrames');
  return _timeOut(o.ref);
});

/// The 8 Quarter Frame data bytes of a time.
Uint8List mtcQuarterFrames(RawTime t) => using((arena) {
  final o = arena<Uint8>(8);
  check(zmidi_mtc_quarter_frames(_time(arena, t), o), 'mtcQuarterFrames');
  return bytesOut(o, 8);
});

/// The C state of an MtcGenerator.
final class MtcGeneratorCore implements Finalizable {
  /// A running generator at [time].
  MtcGeneratorCore(RawTime time, bool reverse) {
    _p = allocState(this, zmidi_mtc_generator_size());
    _s.run(
      (arena) => check(
        zmidi_mtc_generator_init(_p, _time(arena, time), reverse),
        'MtcGenerator',
      ),
    );
  }

  final _s = Scratch();
  late Pointer<ZmidiMtcGenerator> _p;

  /// The next data byte, or null while stopped.
  int? nextByte() => _s.run((arena) {
    final b = arena<Uint8>();
    final status = zmidi_mtc_generator_next(_p, b);
    if (status == ZmidiStatus.ZMIDI_STATUS_NO_PENDING) {
      return null;
    }
    check(status, 'nextByte');
    return b.value;
  });

  /// Runs or stops the generator.
  void setRunning(bool running) =>
      check(zmidi_mtc_generator_set_running(_p, running), 'setRunning');

  /// Sets the direction.
  void setDirection(bool reverse) =>
      check(zmidi_mtc_generator_set_direction(_p, reverse), 'setDirection');

  /// Moves the generator to [time].
  void locate(RawTime time) => _s.run(
    (arena) =>
        check(zmidi_mtc_generator_locate(_p, _time(arena, time)), 'locate'),
  );

  /// The time of the current or next sequence.
  RawTime time() => _s.run((arena) {
    final o = arena<ZmidiMtcTime>();
    check(zmidi_mtc_generator_time(_p, o), 'time');
    return _timeOut(o.ref);
  });

  /// Frees the C state and the scratch block.
  void free() {
    if (_p != nullptr) {
      releaseState(this, _p);
      _p = nullptr;
    }
    _s.release();
  }
}

/// The C state of an MtcReader.
final class MtcReaderCore implements Finalizable {
  /// A reader that is not locked and is stopped.
  MtcReaderCore() {
    _p = allocState(this, zmidi_mtc_reader_size());
    check(zmidi_mtc_reader_init(_p), 'MtcReader');
  }

  final _s = Scratch();
  late Pointer<ZmidiMtcReader> _p;

  /// (piece, boundary, raw, current, reverse, discontinuity).
  (int, bool, RawTime?, RawTime?, bool, bool) receive(int byte) =>
      _s.run((arena) {
        final e = arena<ZmidiMtcEvent>();
        check(zmidi_mtc_reader_receive(_p, byte, e), 'receive');
        final r = e.ref;
        final seq = r.has_sequence;
        return (
          r.piece,
          r.boundary,
          seq ? _timeOut(r.raw) : null,
          seq ? _timeOut(r.current) : null,
          r.reverse,
          r.discontinuity,
        );
      });

  /// Takes the time of a Full Message.
  void fullMessage(RawTime time) => _s.run(
    (arena) => check(
      zmidi_mtc_reader_full_message(_p, _time(arena, time)),
      'fullMessage',
    ),
  );

  /// (locked, running, direction 0 none 1 forward 2 reverse, time).
  (bool, bool, int, RawTime?) info() => _s.run((arena) {
    final i = arena<ZmidiMtcReaderInfo>();
    check(zmidi_mtc_reader_info(_p, i), 'info');
    final r = i.ref;
    return (
      r.locked,
      r.running,
      r.has_direction ? (r.reverse ? 2 : 1) : 0,
      r.has_time ? _timeOut(r.time) : null,
    );
  });

  /// Frees the C state and the scratch block.
  void free() {
    if (_p != nullptr) {
      releaseState(this, _p);
      _p = nullptr;
    }
    _s.release();
  }
}

/// (Device ID, time), or null.
(int, RawTime)? readMtcFull(Uint8List data) => using((arena) {
  final p = bytesIn(arena, data);
  final dev = arena<Uint8>();
  final t = arena<ZmidiMtcTime>();
  return _read(
    'readMtcFull',
    zmidi_mtc_full_read(p, data.length, dev, t),
    () => (dev.value, _timeOut(t.ref)),
  );
});

/// The bytes of an MTC Full Message.
Uint8List mtcFull(int deviceId, RawTime time) => using((arena) {
  final t = _time(arena, time);
  return _written(
    arena,
    'mtcFull',
    (o, l) => zmidi_mtc_full_write(deviceId, t, o, _out, l),
  );
});

/// (Device ID, groups, flags), or null.
(int, Uint8List, int)? readMtcUserBits(Uint8List data) => using((arena) {
  final p = bytesIn(arena, data);
  final dev = arena<Uint8>();
  final u = arena<ZmidiMtcUserBits>();
  return _read(
    'readMtcUserBits',
    zmidi_mtc_user_bits_read(p, data.length, dev, u),
    () => (dev.value, _array(u.ref.groups, 8), u.ref.flags),
  );
});

/// The bytes of an MTC User Bits message.
Uint8List mtcUserBits(int deviceId, Uint8List groups, int flags) =>
    using((arena) {
      final u = arena<ZmidiMtcUserBits>();
      for (var i = 0; i < 8; i++) {
        u.ref.groups[i] = groups[i];
      }
      u.ref.flags = flags;
      return _written(
        arena,
        'mtcUserBits',
        (o, l) => zmidi_mtc_user_bits_write(deviceId, u, o, _out, l),
      );
    });

void _codeIn(ZmidiTimeCode c, RawTimeCode r) {
  _timeIn(c.time, r.$1);
  c
    ..color_frame = r.$2
    ..blank = r.$3
    ..negative = r.$4
    ..status_form = r.$5
    ..subframes = r.$6
    ..estimated = r.$7
    ..invalid = r.$8
    ..video_field1 = r.$9
    ..no_time_code = r.$10;
}

RawTimeCode _codeOut(ZmidiTimeCode c) => (
  _timeOut(c.time),
  c.color_frame,
  c.blank,
  c.negative,
  c.status_form,
  c.subframes,
  c.estimated,
  c.invalid,
  c.video_field1,
  c.no_time_code,
);

Pointer<ZmidiTimeCode> _code(Allocator arena, RawTimeCode r) {
  final p = arena<ZmidiTimeCode>();
  _codeIn(p.ref, r);
  return p;
}

/// A time code of its 5 bytes.
RawTimeCode timeCodeFromBytes(Uint8List data) => using((arena) {
  final o = arena<ZmidiTimeCode>();
  check(
    zmidi_time_code_from_bytes(bytesIn(arena, data), o),
    'timeCodeFromBytes',
  );
  return _codeOut(o.ref);
});

/// The 5 bytes of a time code.
Uint8List timeCodeBytes(RawTimeCode tc) => using((arena) {
  final o = arena<Uint8>(5);
  check(zmidi_time_code_to_bytes(_code(arena, tc), o), 'timeCodeBytes');
  return bytesOut(o, 5);
});

/// The time code with a short time code applied.
RawTimeCode timeCodeWithShort(RawTimeCode last, Uint8List data) =>
    using((arena) {
      final o = arena<ZmidiTimeCode>();
      check(
        zmidi_time_code_with_short(_code(arena, last), bytesIn(arena, data), o),
        'timeCodeWithShort',
      );
      return _codeOut(o.ref);
    });

/// The 2 bytes of the short form.
Uint8List timeCodeShortBytes(RawTimeCode tc) => using((arena) {
  final o = arena<Uint8>(2);
  check(zmidi_time_code_short_bytes(_code(arena, tc), o), 'timeCodeShortBytes');
  return bytesOut(o, 2);
});

/// The MOVE rule to the subframes form.
RawTimeCode timeCodeToSubframesForm(RawTimeCode tc) => using((arena) {
  final o = arena<ZmidiTimeCode>();
  check(
    zmidi_time_code_to_subframes_form(_code(arena, tc), o),
    'timeCodeToSubframesForm',
  );
  return _codeOut(o.ref);
});

/// The MOVE rule to the status form.
RawTimeCode timeCodeToStatusForm(RawTimeCode tc, bool videoField1) =>
    using((arena) {
      final o = arena<ZmidiTimeCode>();
      check(
        zmidi_time_code_to_status_form(_code(arena, tc), videoField1, o),
        'timeCodeToStatusForm',
      );
      return _codeOut(o.ref);
    });

/// The short name of a time code field, or null.
int? mmcShortName(int field) => using((arena) {
  final o = arena<Uint8>();
  return zmidi_mmc_short_name(field, o) == ZmidiStatus.ZMIDI_STATUS_OK
      ? o.value
      : null;
});

/// The time code field of a short name, or null.
int? mmcFullName(int short) => using((arena) {
  final o = arena<Uint8>();
  return zmidi_mmc_full_name(short, o) == ZmidiStatus.ZMIDI_STATUS_OK
      ? o.value
      : null;
});

Uint8List _part(Uint8List s, int offset, int length) =>
    Uint8List.fromList(s.sublist(offset, offset + length));

/// Reads each item of a string with a C `next` function.
List<T> _items<T>(
  Uint8List s,
  String what,
  ZmidiStatus Function(Pointer<Uint8> p, Pointer<UintPtr> offset) next,
  T Function() get,
) => using((arena) {
  final p = bytesIn(arena, s);
  final offset = arena<UintPtr>()..value = 0;
  final r = <T>[];
  for (;;) {
    final status = next(p, offset);
    if (status == ZmidiStatus.ZMIDI_STATUS_NO_PENDING) {
      return r;
    }
    if (status == ZmidiStatus.ZMIDI_STATUS_INVALID_ARGUMENT) {
      throw MalformedException('$what: a string cut short');
    }
    check(status, what);
    r.add(get());
  }
});

List<RawCommand> _commands(Uint8List s) => using((arena) {
  final c = arena<ZmidiMmcCommand>();
  return _items(
    s,
    'readMmc',
    (p, off) => zmidi_mmc_next_command(p, s.length, off, c),
    () {
      final r = c.ref;
      return (
        r.kind,
        r.register_,
        r.destination,
        r.source,
        r.kind == 11 ? _codeOut(r.target) : null,
        r.kind == 13 ? _part(s, r.name_offset, r.name_length) : Uint8List(0),
        r.kind == 9 || r.kind == 13
            ? _part(s, r.data_offset, r.data_length)
            : Uint8List(0),
      );
    },
  );
});

/// The Information Fields of a string.
List<RawField> readMmcFields(Uint8List s) => using((arena) {
  final f = arena<ZmidiMmcField>();
  return _items(
    s,
    'readMmcFields',
    (p, off) => zmidi_mmc_next_field(p, s.length, off, f),
    () {
      final r = f.ref;
      return (
        r.kind,
        r.field,
        r.kind == 2 ? null : _codeOut(r.code),
        _part(s, r.name_offset, r.name_length),
        _part(s, r.data_offset, r.data_length),
      );
    },
  );
});

/// (response, Device ID, commands, fields), or null.
(bool, int, List<RawCommand>, List<RawField>)? readMmc(Uint8List data) =>
    using((arena) {
      final p = bytesIn(arena, data);
      final m = arena<ZmidiMmcMessage>();
      final header = _read(
        'readMmc',
        zmidi_mmc_read(p, data.length, m),
        () => (
          m.ref.response,
          m.ref.device_id,
          _part(data, m.ref.string_offset, m.ref.string_length),
        ),
      );
      if (header == null) {
        return null;
      }
      final (response, dev, s) = header;
      return (
        response,
        dev,
        response ? <RawCommand>[] : _commands(s),
        response ? readMmcFields(s) : <RawField>[],
      );
    });

/// The data bytes that the offsets of the C items point into.
final class _Data {
  final bytes = <int>[];

  (int, int) put(Uint8List b) {
    final at = bytes.length;
    bytes.addAll(b);
    return (at, b.length);
  }
}

Uint8List _writeItems<T extends Struct, R>(
  List<R> list,
  String what,
  Pointer<T> Function(Allocator arena, int n) alloc,
  void Function(Pointer<T> p, R item, _Data data) put,
  ZmidiStatus Function(
    Pointer<T> items,
    int count,
    Pointer<Uint8> data,
    int dataLength,
    Pointer<Uint8> out,
    Pointer<UintPtr> length,
  )
  call,
) => using((arena) {
  final d = _Data();
  final pi = alloc(arena, list.isEmpty ? 1 : list.length);
  for (var i = 0; i < list.length; i++) {
    put(pi, list[i], d);
  }
  final pd = bytesIn(arena, d.bytes);
  return _written(
    arena,
    what,
    (o, l) => call(pi, list.length, pd, d.bytes.length, o, l),
  );
});

void _putCommand(Pointer<ZmidiMmcCommand> base, int i, RawCommand c, _Data d) {
  final r = (base + i).ref
    ..kind = c.$1
    ..register_ = c.$2
    ..destination = c.$3
    ..source = c.$4;
  if (c.$5 != null) {
    _codeIn(r.target, c.$5!);
  }
  final (no, nl) = d.put(c.$6);
  final (dof, dl) = d.put(c.$7);
  r
    ..name_offset = no
    ..name_length = nl
    ..data_offset = dof
    ..data_length = dl;
}

void _putField(Pointer<ZmidiMmcField> base, int i, RawField f, _Data d) {
  final r = (base + i).ref
    ..kind = f.$1
    ..field = f.$2;
  if (f.$3 != null) {
    _codeIn(r.code, f.$3!);
  }
  final (no, nl) = d.put(f.$4);
  final (dof, dl) = d.put(f.$5);
  r
    ..name_offset = no
    ..name_length = nl
    ..data_offset = dof
    ..data_length = dl;
}

/// The bytes of an MMC command message.
Uint8List mmcCommands(int deviceId, List<RawCommand> commands) {
  var i = 0;
  return _writeItems<ZmidiMmcCommand, RawCommand>(
    commands,
    'mmcCommands',
    (arena, n) => arena<ZmidiMmcCommand>(n),
    (p, c, d) => _putCommand(p, i++, c, d),
    (pi, n, pd, dl, o, l) =>
        zmidi_mmc_write_commands(deviceId, pi, n, pd, dl, o, _out, l),
  );
}

/// The bytes of an MMC response message.
Uint8List mmcResponses(int deviceId, List<RawField> fields) {
  var i = 0;
  return _writeItems<ZmidiMmcField, RawField>(
    fields,
    'mmcResponses',
    (arena, n) => arena<ZmidiMmcField>(n),
    (p, f, d) => _putField(p, i++, f, d),
    (pi, n, pd, dl, o, l) =>
        zmidi_mmc_write_responses(deviceId, pi, n, pd, dl, o, _out, l),
  );
}

/// The name and data bytes of Information Fields, for WRITE.
Uint8List mmcFields(List<RawField> fields) {
  var i = 0;
  return _writeItems<ZmidiMmcField, RawField>(
    fields,
    'mmcFields',
    (arena, n) => arena<ZmidiMmcField>(n),
    (p, f, d) => _putField(p, i++, f, d),
    (pi, n, pd, dl, o, l) => zmidi_mmc_write_fields(pi, n, pd, dl, o, _out, l),
  );
}
