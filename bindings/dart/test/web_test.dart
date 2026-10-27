// The web form (REQ-DART-WEB-002): initWeb loads the TypeScript package and its WASM build, and
// the errors of the TypeScript package become the exceptions of the package.
@TestOn('browser')
library;

import 'package:test/test.dart';
import 'package:zetaohm_midi/src/web/runtime.dart' show mapError, ts;
import 'package:zetaohm_midi/zetaohm_midi.dart';

void main() {
  test('a call before initWeb throws RejectedException', () {
    expect(() => wordCount(0x40000000), throwsA(isA<RejectedException>()));
  });

  test('initWeb loads the WASM build once', () async {
    await initWeb();
    final first = ts;
    await initWeb();
    expect(identical(ts, first), isTrue);
    expect(wordCount(0x40000000), 2);
  });

  test(
    'an error of the TypeScript package is an exception of the package',
    () async {
      await initWeb();
      // No pending request: the TypeScript package throws RejectedError.
      final dev = CiDevice(manufacturer: [0x7D]);
      expect(() => dev.peReply(200), throwsA(isA<RejectedException>()));
      // A Dart exception goes through unchanged.
      expect(mapError(const MalformedException('x')), isNull);
      expect(mapError(StateError('x')), isNull);
    },
  );
}
