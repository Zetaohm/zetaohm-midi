/// The web form of the byte stream reader and writer: the TypeScript package through
/// dart:js_interop. The values are checked by `io.dart`.
library;

import 'dart:js_interop';
import 'dart:typed_data';

import 'runtime.dart';

/// The TypeScript BytesToUmp.
final class BytesToUmpState {
  /// A reader for [group] and [protocol].
  BytesToUmpState(int group, int protocol)
    : _o = tsNew('BytesToUmp', [group.toJS, protocol.toJS]);

  final JSObject _o;

  /// Reads one byte.
  List<List<int>> push(int byte) => umps(call(_o, 'push', [byte.toJS]));

  /// Reads many bytes (the TypeScript feed: one C call for each chunk).
  List<List<int>> feed(List<int> data) =>
      umps(call(_o, 'feed', [jsBytes(data)]));

  /// Frees the WASM memory.
  void free() => call<JSAny?>(_o, 'free');
}

/// The TypeScript UmpToBytes.
final class UmpToBytesState {
  /// A writer with or without running status.
  UmpToBytesState(bool runningStatus)
    : _o = tsNew('UmpToBytes', [runningStatus.toJS]);

  final JSObject _o;

  /// Writes one UMP.
  Uint8List write(List<int> words) => bytes(call(_o, 'write', [jsInts(words)]));

  /// Sends the next status byte.
  void refreshStatus() => call<JSAny?>(_o, 'refreshStatus');

  /// Frees the WASM memory.
  void free() => call<JSAny?>(_o, 'free');
}
