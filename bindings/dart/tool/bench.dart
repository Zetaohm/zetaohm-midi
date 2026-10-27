// Cost of a call through the Dart package (docs/optimization-backlog.md OPT-029 step 4).
//
// Wall-clock time on the host, not a gate. Each row gives the time of one call and of each MIDI
// message, so that the fixed cost of a call (dart:ffi, the arena) shows against the work in the
// library.
//
//     dart run tool/bench.dart

import 'dart:typed_data';

import 'package:zetaohm_midi/zetaohm_midi.dart';

final notes = Uint8List.fromList([
  for (var i = 0; i < 64; i++) ...[0x90, i, 0x40],
]);
final words = [for (var i = 0; i < 64; i++) 0x20900040 | (i << 8)];

void row(String name, void Function() f, int messages, [int n = 2000]) {
  var best = double.infinity;
  for (var r = 0; r < 5; r++) {
    final sw = Stopwatch()..start();
    for (var i = 0; i < n; i++) {
      f();
    }
    final us = sw.elapsedMicroseconds / n;
    if (us < best) best = us;
  }
  final perCall = best.toStringAsFixed(2).padLeft(9);
  final perMessage = (best * 1000 / messages).toStringAsFixed(1).padLeft(9);
  print('${name.padRight(40)} $perCall us/call $perMessage ns/message');
}

void main() {
  final r = BytesToUmp();
  final w = UmpToBytes(runningStatus: true);
  row('BytesToUmp.push, one byte', () => r.push(0xF8), 1, 20000);
  row('BytesToUmp.feed, 64 notes (192 bytes)', () => r.feed(notes), 64);
  row(
    'BytesToUmp.feed, one note (3 bytes)',
    () => r.feed(notes.sublist(0, 3)),
    1,
    20000,
  );
  row(
    'UmpToBytes.write, one note',
    () => w.write(words.sublist(0, 1)),
    1,
    20000,
  );
}
