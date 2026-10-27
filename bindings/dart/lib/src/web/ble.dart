/// The web form of the BLE-MIDI reader and writer: the TypeScript package through dart:js_interop.
/// The values are checked by `ble.dart`.
library;

import 'dart:js_interop';
import 'dart:typed_data';

import 'runtime.dart';

/// The TypeScript BleReader.
final class BleReaderState {
  /// A reader for [group] and [protocol].
  BleReaderState(int group, int protocol)
    : _o = tsNew('BleReader', [group.toJS, protocol.toJS]);

  final JSObject _o;

  /// Reads one packet: `(timestamp, words)` for each UMP.
  ({List<(int, List<int>)> umps, int dropped}) read(List<int> packet) {
    final r = call<JSObject>(_o, 'read', [jsBytes(packet)]);
    return (
      umps: [
        for (final u in (prop<JSArray<JSObject>>(r, 'umps')).toDart)
          (intOf(prop(u, 'millis')), ints(prop(u, 'words'))),
      ],
      dropped: intOf(prop(r, 'dropped')),
    );
  }

  /// Drops the open SysEx message and the last timestamp.
  void reset() => call<JSAny?>(_o, 'reset');

  /// Frees the WASM memory.
  void free() => call<JSAny?>(_o, 'free');
}

/// The TypeScript BleWriter.
final class BleWriterState {
  /// A writer for [attMtu].
  BleWriterState(int attMtu, bool runningStatus)
    : _o = tsNew('BleWriter', [attMtu.toJS, runningStatus.toJS]);

  final JSObject _o;

  /// Sets a new ATT MTU.
  void setAttMtu(int attMtu) => call<JSAny?>(_o, 'setAttMtu', [attMtu.toJS]);

  /// The largest packet in bytes.
  int get packetLen => intOf(prop(_o, 'packetLen'));

  /// True when the writer holds bytes for the next packet.
  bool get hasHeld => boolOf(prop(_o, 'hasHeld'));

  /// Drops the held bytes and the running status.
  void reset() => call<JSAny?>(_o, 'reset');

  /// Writes one packet from `(millis, words)` UMPs.
  ({Uint8List packet, int taken}) writePacket(List<(int, List<int>)> umps) {
    final r = call<JSObject>(_o, 'writePacket', [
      [
        for (final (millis, words) in umps)
          jsObject({'millis': millis.toJS, 'words': jsInts(words)}),
      ].toJS,
    ]);
    return (packet: bytes(prop(r, 'packet')), taken: intOf(prop(r, 'taken')));
  }

  /// Frees the WASM memory.
  void free() => call<JSAny?>(_o, 'free');
}
