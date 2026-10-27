/// BLE-MIDI v1.0 packets (zmidi-ble through the C ABI): a packet reader that gives UMPs with their
/// 13-bit timestamps, and a packet writer with running status (BLE-MIDI v1.0 sections 7 and 8).
library;

import 'dart:typed_data';

import 'checks.dart';
import 'native/ble.dart' if (dart.library.js_interop) 'web/ble.dart' as be;

/// One UMP of a BLE-MIDI packet with the time of its message in milliseconds.
final class BleUmp {
  /// A UMP of [words] at [millis].
  const BleUmp(this.millis, this.words);

  /// Read: the 13-bit timestamp of the sender clock (0 to 8191). Write: the time of the program.
  final int millis;

  /// The words of the UMP.
  final List<int> words;

  @override
  bool operator ==(Object other) =>
      other is BleUmp &&
      other.millis == millis &&
      other.words.length == words.length &&
      [
        for (var i = 0; i < words.length; i++) other.words[i] == words[i],
      ].every((x) => x);

  @override
  int get hashCode => Object.hash(millis, Object.hashAll(words));

  @override
  String toString() => 'BleUmp($millis, $words)';
}

/// Reads BLE-MIDI packets and gives UMPs.
final class BleReader {
  /// A reader that gives UMPs on [group] (0 to 15) with [protocol] 1 (MIDI 1.0) or 2 (MIDI 2.0).
  BleReader({int group = 0, int protocol = 1})
    : _b = be.BleReaderState(
        checkInt('group', group, 15),
        checkProtocol(protocol),
      );

  final be.BleReaderState _b;
  bool _freed = false;

  /// Reads one packet. Gives its UMPs and the count of the parts of the packet that the reader
  /// dropped.
  ({List<BleUmp> umps, int dropped}) read(List<int> packet) {
    checkFreed(_freed, this);
    final r = _b.read(packet);
    return (
      umps: [for (final (t, w) in r.umps) BleUmp(t, w)],
      dropped: r.dropped,
    );
  }

  /// Drops an open SysEx message and the last timestamp, for example after a new connection.
  void reset() {
    checkFreed(_freed, this);
    _b.reset();
  }

  /// Frees the memory of the reader now.
  void free() {
    if (!_freed) {
      _freed = true;
      _b.free();
    }
  }
}

/// Writes UMPs into BLE-MIDI packets.
final class BleWriter {
  /// A writer for the ATT MTU [attMtu] (23 or more), with running status when [runningStatus].
  BleWriter({int attMtu = 23, bool runningStatus = true})
    : _b = be.BleWriterState(checkInt('attMtu', attMtu, 0xffff), runningStatus);

  final be.BleWriterState _b;
  bool _freed = false;

  be.BleWriterState get _live {
    checkFreed(_freed, this);
    return _b;
  }

  /// Sets a new ATT MTU (23 or more).
  void setAttMtu(int attMtu) =>
      _live.setAttMtu(checkInt('attMtu', attMtu, 0xffff));

  /// The largest packet in bytes: the ATT MTU less 3.
  int get packetLen => _live.packetLen;

  /// True when the writer holds bytes for the next packet (a SysEx part).
  bool get hasHeld => _live.hasHeld;

  /// Drops the held bytes and the running status; the next packet starts a new stream.
  void reset() => _live.reset();

  /// Writes one packet from the held bytes and then from [umps]. Gives the packet (empty: nothing
  /// to send) and the count of UMPs taken from the start of [umps]. Times never decrease in a
  /// stream.
  ({Uint8List packet, int taken}) writePacket(List<BleUmp> umps) =>
      _live.writePacket([
        for (final u in umps)
          (checkTime('millis', u.millis), List.of(checkUmp(u.words))),
      ]);

  /// Frees the memory of the writer now.
  void free() {
    if (!_freed) {
      _freed = true;
      _b.free();
    }
  }
}
