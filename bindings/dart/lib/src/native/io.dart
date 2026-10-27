/// The native form of the byte stream reader and writer: zmidi-io through dart:ffi and the C ABI.
/// The values are checked by `io.dart`.
library;

import 'dart:ffi';
import 'dart:typed_data';

import '../bindings.g.dart';
import '../native.dart';

/// The C state of a BytesToUmp.
final class BytesToUmpState implements Finalizable {
  /// A reader for [group] and [protocol].
  BytesToUmpState(int group, int protocol) {
    _p = allocState(this, zmidi_bytes_to_ump_size());
    check(zmidi_bytes_to_ump_init(_p, group, protocol), 'BytesToUmp');
  }

  /// The arguments of the calls (no native allocation for each call).
  final _s = Scratch();

  late Pointer<ZmidiBytesToUmp> _p;

  /// Reads one byte.
  List<List<int>> push(int byte) => _s.run((arena) {
    final out = arena<Uint32>(4);
    final count = arena<UintPtr>();
    check(zmidi_bytes_to_ump_push(_p, byte, out, 4, count), 'push');
    return splitUmps([for (var i = 0; i < count.value; i++) out[i]]);
  });

  /// Reads many bytes with one C call for each chunk (zmidi_bytes_to_ump_feed).
  List<List<int>> feed(List<int> data) {
    final chunk = data.length < _feedChunk ? data.length : _feedChunk;
    // At most 4 words for each byte, but far fewer in a real stream: the call stops when the
    // buffer is nearly full, and the loop gives it the rest.
    final cap = 2 * chunk + 4;
    return _s.run((arena) {
      final pd = arena<Uint8>(chunk < 1 ? 1 : chunk);
      final pw = arena<Uint32>(cap);
      final read = arena<UintPtr>();
      final count = arena<UintPtr>();
      final words = <int>[];
      for (var at = 0; at < data.length; at += chunk) {
        final end = at + chunk < data.length ? at + chunk : data.length;
        pd.asTypedList(end - at).setAll(0, data.getRange(at, end));
        for (var done = 0; done < end - at;) {
          check(
            zmidi_bytes_to_ump_feed(
              _p,
              pd + done,
              end - at - done,
              pw,
              cap,
              read,
              count,
            ),
            'feed',
          );
          words.addAll(pw.asTypedList(count.value));
          done += read.value;
        }
      }
      return splitUmps(words);
    });
  }

  /// Frees the C state and the scratch block.
  void free() {
    if (_p != nullptr) {
      releaseState(this, _p);
      _p = nullptr;
    }
    _s.release();
  }
}

/// The most bytes that one feed call copies into native memory.
const _feedChunk = 4096;

/// The C state of a UmpToBytes.
final class UmpToBytesState implements Finalizable {
  /// A writer with or without running status.
  UmpToBytesState(bool runningStatus) {
    _p = allocState(this, zmidi_ump_to_bytes_size());
    check(zmidi_ump_to_bytes_init(_p, runningStatus), 'UmpToBytes');
  }

  /// The arguments of the calls (no native allocation for each call).
  final _s = Scratch();

  late Pointer<ZmidiUmpToBytes> _p;

  /// Writes one UMP.
  Uint8List write(List<int> words) => _s.run((arena) {
    final out = arena<Uint8>(16);
    final count = arena<UintPtr>();
    check(
      zmidi_ump_to_bytes_write(
        _p,
        wordsIn(arena, words),
        words.length,
        out,
        16,
        count,
      ),
      'write',
    );
    return bytesOut(out, count.value);
  });

  /// Sends the next status byte.
  void refreshStatus() =>
      check(zmidi_ump_to_bytes_refresh_status(_p), 'refreshStatus');

  /// Frees the C state and the scratch block.
  void free() {
    if (_p != nullptr) {
      releaseState(this, _p);
      _p = nullptr;
    }
    _s.release();
  }
}
