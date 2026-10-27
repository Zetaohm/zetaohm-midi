/// The native form of the clip reader and writer: zmidi-file through dart:ffi and the C ABI. The
/// values are checked by `file.dart`.
library;

import 'dart:ffi';
import 'dart:typed_data';

import 'package:ffi/ffi.dart';

import '../bindings.g.dart';
import '../errors.dart';
import '../native.dart';

/// One Set Profile On of the configuration: the values are checked.
typedef ClipProfileOn = ({
  List<int> profileId,
  int destination,
  int channel,
  int channels,
});

/// The checked configuration of a clip.
typedef ClipConfig = ({
  int ticksPerQuarterNote,
  int group,
  int? tempo10ns,
  (int, int, int)? timeSignature,
  List<ClipProfileOn> profiles,
});

Exception _clipError(int code, String what) {
  const text = [
    'no error',
    'the bytes do not start with the File Header',
    'a trailing partial word, or a UMP cut by the end of the data',
    'a byte follows End of Clip',
    'the buffer cannot hold the bytes',
    'a bad clip configuration',
    'the words are not one whole UMP',
    'a tick earlier than the last one',
    'a Property Exchange message',
    'a message that the writer writes itself',
    'Set Tempo off the 1/24 quarter note grid',
    'Set Time Signature not at a bar boundary',
    'a different tick inside one multi-UMP Lyrics or Ruby message',
    'the call is not allowed in the current phase',
  ];
  final message =
      '$what: ${code < text.length ? text[code] : 'clip error $code'}';
  if (code >= 1 && code <= 3) {
    return MalformedException(message);
  }
  if (code == 5 || code == 6) {
    return InvalidValueException(message);
  }
  return RejectedException(message);
}

/// Reads all messages of a MIDI Clip File: `(section, ticks, words)`, section 0 for the Clip
/// Configuration Header.
List<(int, int, List<int>)> readClip(List<int> data) {
  checkAbi();
  return using((arena) {
    final reader = arena<Uint8>(
      zmidi_clip_reader_size(),
    ).cast<ZmidiClipReader>();
    final ev = arena<ZmidiClipEvent>();
    final has = arena<Bool>();
    final err = arena<UnsignedInt>();
    if (zmidi_clip_reader_init(
          reader,
          bytesIn(arena, data),
          data.length,
          err,
        ) !=
        ZmidiStatus.ZMIDI_STATUS_OK) {
      throw _clipError(err.value, 'readClip');
    }
    final out = <(int, int, List<int>)>[];
    for (;;) {
      if (zmidi_clip_reader_next(reader, ev, has, err) !=
          ZmidiStatus.ZMIDI_STATUS_OK) {
        throw _clipError(err.value, 'readClip');
      }
      if (!has.value) {
        return out;
      }
      final e = ev.ref;
      out.add((
        e.section,
        e.ticks,
        [for (var k = 0; k < e.word_count; k++) e.words[k]],
      ));
    }
  });
}

/// The code of a buffer too small (ZMIDI_CLIP_ERROR_BUFFER_TOO_SMALL).
const _bufferTooSmall = 4;

sealed class _Step {}

final class _Config extends _Step {
  _Config(this.words);
  final List<int> words;
}

final class _Start extends _Step {
  _Start(this.tick, this.tempo10ns);
  final int tick;
  final int? tempo10ns;
}

final class _Event extends _Step {
  _Event(this.tick, this.words);
  final int tick;
  final List<int> words;
}

/// The bytes of a clip: native memory that the garbage collector frees with its owner.
final class _Buffer implements Finalizable {
  _Buffer(int size) {
    p = allocState(this, size);
  }

  late Pointer<Uint8> p;

  void free() {
    if (p != nullptr) {
      releaseState(this, p);
      p = nullptr;
    }
  }
}

/// The C writer of a ClipWriter, with the steps that make it again in a larger buffer.
final class ClipWriterState implements Finalizable {
  /// A writer of the checked configuration [cfg].
  ClipWriterState(ClipConfig cfg) : _group = cfg.group {
    final profiles = cfg.profiles;
    // One block: the C writer state, then the configuration (which the C writer copies at
    // init), then the profiles.
    final ws = (zmidi_clip_writer_size() + 15) ~/ 16 * 16;
    _w = allocState(
      this,
      ws + sizeOf<ZmidiClipConfig>() + sizeOf<ZmidiClipProfileOn>() * 16,
    );
    _config = Pointer<ZmidiClipConfig>.fromAddress(_w.address + ws);
    final ts = cfg.timeSignature;
    final c = _config.ref
      ..ticks_per_quarter_note = cfg.ticksPerQuarterNote
      ..group = _group
      ..has_tempo = cfg.tempo10ns != null
      ..tempo_10ns = cfg.tempo10ns ?? 0
      ..has_time_signature = ts != null
      ..numerator = ts?.$1 ?? 0
      ..denominator = ts?.$2 ?? 0
      ..thirty_second_notes = ts?.$3 ?? 0
      ..profile_count = profiles.length;
    final pp = Pointer<ZmidiClipProfileOn>.fromAddress(
      _config.address + sizeOf<ZmidiClipConfig>(),
    );
    c.profiles = profiles.isEmpty ? nullptr : pp;
    for (var i = 0; i < profiles.length; i++) {
      final p = profiles[i];
      for (var k = 0; k < 5; k++) {
        pp[i].profile_id[k] = p.profileId[k];
      }
      pp[i]
        ..destination = p.destination
        ..channel = p.channel
        ..channels = p.channels;
    }
    _rebuild();
  }

  final int _group;
  final List<_Step> _steps = [];
  late Pointer<ZmidiClipConfig> _config;
  late Pointer<ZmidiClipWriter> _w;
  _Buffer? _buffer;
  int _capacity = 1024;

  /// Writes one more message of the Clip Configuration Header.
  void configEvent(List<int> words) => _step(_Config(words));

  /// Writes Start of Clip.
  void start(int tick, int? tempo10ns) => _step(_Start(tick, tempo10ns));

  /// Writes one message of the Clip Sequence Data.
  void event(int tick, List<int> words) => _step(_Event(tick, words));

  /// Writes End of Clip and gives the bytes of the clip.
  Uint8List finish(int tick) {
    for (;;) {
      final r = using((arena) {
        final err = arena<UnsignedInt>();
        final len = arena<UintPtr>();
        final st = zmidi_clip_writer_finish(_w, tick, len, err);
        return _failed(st, err.value, 'finish')
            ? null
            : bytesOut(_buffer!.p, len.value);
      });
      if (r != null) {
        return r;
      }
      _capacity *= 2;
      _rebuild();
    }
  }

  /// Frees the C writer and its buffer.
  void free() {
    if (_w != nullptr) {
      releaseState(this, _w);
      _w = nullptr;
      _buffer?.free();
      _buffer = null;
    }
  }

  void _step(_Step step) {
    for (;;) {
      bool full;
      try {
        full = _call(step);
      } on MidiException {
        // A failed call can leave the C writer in a changed state: make it again from the steps.
        _rebuild();
        rethrow;
      }
      if (!full) {
        _steps.add(step);
        return;
      }
      _capacity *= 2;
      _rebuild();
    }
  }

  /// Sets up the C writer in a new buffer of [_capacity] bytes and runs the kept steps.
  void _rebuild() {
    for (;;) {
      _buffer?.free();
      final buf = _Buffer(_capacity);
      _buffer = buf;
      final full = using((arena) {
        final err = arena<UnsignedInt>();
        final st = zmidi_clip_writer_init(_w, buf.p, _capacity, _config, err);
        return _failed(st, err.value, 'ClipWriter');
      });
      if (!full && !_steps.any(_call)) {
        return;
      }
      _capacity *= 2;
    }
  }

  /// Runs one step on the C writer; true when the buffer is too small.
  bool _call(_Step s) => using((arena) {
    final err = arena<UnsignedInt>();
    final ZmidiStatus st;
    switch (s) {
      case _Start(:final tick, :final tempo10ns):
        st = zmidi_clip_writer_start(
          _w,
          tick,
          tempo10ns != null,
          _group,
          tempo10ns ?? 0,
          err,
        );
      case _Config(:final words):
        st = zmidi_clip_writer_config_event(
          _w,
          wordsIn(arena, words),
          words.length,
          err,
        );
      case _Event(:final tick, :final words):
        st = zmidi_clip_writer_event(
          _w,
          tick,
          wordsIn(arena, words),
          words.length,
          err,
        );
    }
    return _failed(st, err.value, 'clip');
  });

  /// False for OK, true for a buffer too small; throws the exception of any other status.
  bool _failed(ZmidiStatus st, int code, String what) {
    if (st == ZmidiStatus.ZMIDI_STATUS_OK) {
      return false;
    }
    if (code == _bufferTooSmall) {
      return true;
    }
    if (code == 0) {
      check(st, what);
    }
    throw _clipError(code, what);
  }
}
