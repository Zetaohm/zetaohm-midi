// MIDI Clip Files (REQ-DART-FILE-001): M2-116.
import 'dart:convert';
import 'dart:typed_data';

import 'package:test/test.dart';
import 'package:zetaohm_midi/zetaohm_midi.dart';

List<int> words(Uint8List data) {
  final v = ByteData.sublistView(data);
  return [for (var at = 8; at < data.length; at += 4) v.getUint32(at)];
}

void main() {
  setUpAll(initWeb);

  test('minimal clip', () {
    final w = ClipWriter()..start();
    final data = w.finish(0);
    expect(ascii.decode(data.sublist(0, 8)), 'SMF2CLIP');
    expect(words(data), [
      0x00400000,
      0x00300060,
      0x00400000,
      0xF0200000,
      0,
      0,
      0,
      0x00400000,
      0xF0210000,
      0,
      0,
      0,
    ]);
    expect(() => w.event(0, [0x20903C64]), throwsA(isA<RejectedException>()));
  });

  test('write and read back', () {
    const noteOn = [0x40903C00, 0xFFFF0000];
    const noteOff = [0x40803C00, 0];
    final w =
        ClipWriter(
            tempo10ns: 50000000,
            timeSignature: (4, 2, 8),
            profiles: [
              const ClipProfile(
                [0x7E, 0, 1, 2, 3],
                ClipDestination.group,
                channels: 5,
              ),
            ],
          )
          ..start()
          ..event(0, noteOn)
          ..event(96, noteOff);
    expect(() => w.event(10, noteOn), throwsA(isA<RejectedException>()));
    expect(
      () => w.event(96, noteOn.sublist(0, 1)),
      throwsA(isA<InvalidValueException>()),
    );
    final notes = [
      for (final e in readClip(w.finish(192)))
        if (e.section == ClipSection.sequence &&
            (e.words[0] == noteOn[0] || e.words[0] == noteOff[0]))
          [e.ticks, e.words],
    ];
    expect(notes, [
      [0, noteOn],
      [96, noteOff],
    ]);
  });

  test('a long clip grows its buffer', () {
    final w = ClipWriter()..start();
    for (var t = 0; t < 2000; t++) {
      w.event(t, [0x20903C64]);
    }
    final n = readClip(w.finish(2000))
        .where(
          (e) => e.section == ClipSection.sequence && e.words[0] == 0x20903C64,
        )
        .length;
    expect(n, 2000);
  });

  test('bad clip input', () {
    expect(
      () => readClip(ascii.encode('SMF1CLIP')),
      throwsA(isA<MalformedException>()),
    );
    expect(
      () => ClipWriter(ticksPerQuarterNote: 0),
      throwsA(isA<InvalidValueException>()),
    );
  });
}
