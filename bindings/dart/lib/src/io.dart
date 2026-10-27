/// The MIDI 1.0 byte stream to UMP and back (zmidi-io through the C ABI): M2-104 Appendix D, MIDI
/// 1.0 running status and System Real Time between data bytes.
library;

import 'dart:typed_data';

import 'checks.dart';
import 'native/io.dart' if (dart.library.js_interop) 'web/io.dart' as be;

/// Reads a MIDI 1.0 byte stream and gives UMPs: MIDI 1.0 Protocol (Message Type 0x2) or MIDI 2.0
/// (0x4).
final class BytesToUmp {
  /// A reader for Group [group] (0 to 15) and [protocol] 1 (MIDI 1.0) or 2 (MIDI 2.0).
  BytesToUmp({int group = 0, int protocol = 1})
    : _b = be.BytesToUmpState(
        checkInt('group', group, 15),
        checkProtocol(protocol),
      );

  final be.BytesToUmpState _b;
  bool _freed = false;

  /// Reads one byte; gives the UMPs that it completes.
  List<List<int>> push(int byte) {
    checkFreed(_freed, this);
    return _b.push(checkInt('byte', byte, 255));
  }

  /// Reads [data]; gives the UMPs that it completes, in order. One C call reads many bytes
  /// (OPT-035).
  List<List<int>> feed(List<int> data) {
    checkFreed(_freed, this);
    return _b.feed(checkBytes('data', data));
  }

  /// Frees the memory of the reader now.
  void free() {
    if (!_freed) {
      _freed = true;
      _b.free();
    }
  }
}

/// Writes UMPs (Message Types 0x1, 0x2, 0x3) as a MIDI 1.0 byte stream.
final class UmpToBytes {
  /// A writer; [runningStatus] leaves out a status byte equal to the last one.
  UmpToBytes({bool runningStatus = false})
    : _b = be.UmpToBytesState(runningStatus);

  final be.UmpToBytesState _b;
  bool _freed = false;

  /// Writes one UMP; gives its bytes (empty for a UMP with no MIDI 1.0 form).
  Uint8List write(List<int> words) {
    checkFreed(_freed, this);
    return _b.write(checkUmp(words));
  }

  /// Makes the next message send its status byte (a refresh of running status).
  void refreshStatus() {
    checkFreed(_freed, this);
    _b.refreshStatus();
  }

  /// Frees the memory of the writer now.
  void free() {
    if (!_freed) {
      _freed = true;
      _b.free();
    }
  }
}
