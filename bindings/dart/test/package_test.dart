// The native library of the package (REQ-DART-PKG-001): the build hook builds the C ABI and the
// package checks its major version on the first call (DEC-019 rule 6). The native form only.
@TestOn('vm')
library;

import 'package:test/test.dart';
import 'package:zetaohm_midi/src/native.dart' show checkAbiVersion, splitUmps;
import 'package:zetaohm_midi/zetaohm_midi.dart';

void main() {
  test('initWeb does nothing on the native platforms', () async {
    await initWeb();
    await initWeb(moduleUrl: 'not a url');
    expect(wordCount(0x40000000), 2);
  });

  test('the build hook gives the native library', () {
    // The first call loads the code asset and checks the C ABI version.
    expect(wordCount(0x40000000), 2);
    expect(BytesToUmp().feed([0x90, 0x3C, 0x64]), [
      [0x20903C64],
    ]);
  });

  test('refuses a native library of C ABI 0.1 to 0.6 or 1.x', () {
    // REQ-DART-PKG-002: the package needs C ABI 0.7 or a later 0.x.
    expect(() => checkAbiVersion(0x00000001), throwsA(isA<MidiException>()));
    expect(() => checkAbiVersion(0x00010002), throwsA(isA<MidiException>()));
    expect(() => checkAbiVersion(0x00000002), throwsA(isA<MidiException>()));
    expect(() => checkAbiVersion(0x00000003), throwsA(isA<MidiException>()));
    expect(() => checkAbiVersion(0x00000004), throwsA(isA<MidiException>()));
    expect(() => checkAbiVersion(0x00000005), throwsA(isA<MidiException>()));
    expect(() => checkAbiVersion(0x00000006), throwsA(isA<MidiException>()));
    checkAbiVersion(0x00000007);
    checkAbiVersion(0x00000008);
  });

  test('the UMP split table is the word count of the library', () {
    // M2-104 Table 4: the Dart table of splitUmps gives the word count of zmidi_ump_word_count.
    for (var mt = 0; mt < 16; mt++) {
      final n = wordCount(mt << 28);
      final words = [mt << 28, for (var i = 1; i < 4; i++) i, mt << 28];
      expect(splitUmps(words).first.length, n, reason: 'Message Type $mt');
    }
  });
}
