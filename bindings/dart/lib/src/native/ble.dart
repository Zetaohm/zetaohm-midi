/// The native form of the BLE-MIDI reader and writer: zmidi-ble through dart:ffi and the C ABI. The
/// values are checked by `ble.dart`.
library;

import 'dart:ffi';
import 'dart:typed_data';

import '../bindings.g.dart';
import '../native.dart';

/// The C state of a BleReader.
final class BleReaderState implements Finalizable {
  /// A reader for [group] and [protocol].
  BleReaderState(int group, int protocol) {
    _p = allocState(this, zmidi_ble_reader_size());
    check(zmidi_ble_reader_init(_p, group, protocol), 'BleReader');
  }

  /// The arguments of the calls (no native allocation for each call).
  final _s = Scratch();

  late Pointer<ZmidiBleReader> _p;

  /// Reads one packet: `(timestamp, words)` for each UMP.
  ({List<(int, List<int>)> umps, int dropped}) read(List<int> packet) {
    final cap = packet.isEmpty ? 1 : packet.length;
    return _s.run((arena) {
      final out = arena<ZmidiBleUmp>(cap);
      final count = arena<UintPtr>();
      final dropped = arena<UintPtr>();
      check(
        zmidi_ble_read(
          _p,
          bytesIn(arena, packet),
          packet.length,
          out,
          cap,
          count,
          dropped,
        ),
        'read',
      );
      return (
        umps: [
          for (var i = 0; i < count.value; i++)
            (
              out[i].timestamp_ms,
              [for (var k = 0; k < out[i].word_count; k++) out[i].words[k]],
            ),
        ],
        dropped: dropped.value,
      );
    });
  }

  /// Drops the open SysEx message and the last timestamp.
  void reset() => check(zmidi_ble_reader_reset(_p), 'reset');

  /// Frees the C state and the scratch block.
  void free() {
    if (_p != nullptr) {
      releaseState(this, _p);
      _p = nullptr;
    }
    _s.release();
  }
}

/// The C state of a BleWriter.
final class BleWriterState implements Finalizable {
  /// A writer for [attMtu].
  BleWriterState(int attMtu, bool runningStatus) {
    _p = allocState(this, zmidi_ble_writer_size());
    check(zmidi_ble_writer_init(_p, attMtu), 'BleWriter');
    check(zmidi_ble_writer_set_running_status(_p, runningStatus), 'BleWriter');
  }

  /// The arguments of the calls (no native allocation for each call).
  final _s = Scratch();

  late Pointer<ZmidiBleWriter> _p;

  /// Sets a new ATT MTU.
  void setAttMtu(int attMtu) =>
      check(zmidi_ble_writer_set_att_mtu(_p, attMtu), 'setAttMtu');

  /// The largest packet in bytes.
  int get packetLen => zmidi_ble_writer_packet_len(_p);

  /// True when the writer holds bytes for the next packet.
  bool get hasHeld => zmidi_ble_writer_has_held(_p);

  /// Drops the held bytes and the running status.
  void reset() => check(zmidi_ble_writer_reset(_p), 'reset');

  /// Writes one packet from `(millis, words)` UMPs.
  ({Uint8List packet, int taken}) writePacket(List<(int, List<int>)> umps) {
    final n = packetLen;
    return _s.run((arena) {
      final pu = arena<ZmidiBleTimedUmp>(umps.isEmpty ? 1 : umps.length);
      for (var i = 0; i < umps.length; i++) {
        final (millis, w) = umps[i];
        pu[i].millis = millis;
        pu[i].word_count = w.length;
        for (var k = 0; k < w.length; k++) {
          pu[i].words[k] = w[k];
        }
      }
      final out = arena<Uint8>(n);
      final len = arena<UintPtr>();
      final taken = arena<UintPtr>();
      check(
        zmidi_ble_write_packet(_p, pu, umps.length, out, n, len, taken),
        'writePacket',
      );
      return (packet: bytesOut(out, len.value), taken: taken.value);
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
