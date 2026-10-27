// MIDI 1.0 byte stream to UMP and back (REQ-DART-IO-001): M2-104 Appendix D, MIDI 1.0.
import 'package:test/test.dart';
import 'package:zetaohm_midi/zetaohm_midi.dart';

void main() {
  setUpAll(initWeb);

  test('MIDI 1.0 bytes to UMP with running status', () {
    final r = BytesToUmp(group: 3);
    expect(r.feed([0x90, 0x3C, 0x64]), [
      [0x23903C64],
    ]);
    expect(r.feed([0x3E, 0x40]), [
      [0x23903E40],
    ]);
    // Timing Clock between data bytes comes out at once.
    expect(r.feed([0x90, 0x3C, 0xF8, 0x64]), [
      [0x13F80000],
      [0x23903C64],
    ]);
    r.free();
  });

  test('bytes to MIDI 2.0 UMP and SysEx', () {
    expect(BytesToUmp(protocol: 2).feed([0x90, 0x3C, 0x64]), [
      [0x40903C00, 0xC9240000],
    ]);
    expect(BytesToUmp().feed([0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7]), [
      [0x30047E7F, 0x06010000],
    ]);
  });

  test('feed of a long stream gives the UMPs of push', () {
    // OPT-035: one C call for many bytes, over more than one 4096-byte chunk and full word
    // buffers. Notes with running status and Clocks between data bytes, then a long SysEx.
    final bytes = <int>[0x90];
    for (var i = 0; i < 3000; i++) {
      bytes.addAll([i & 0x7F, i % 3 == 0 ? 0xF8 : 0x40, 0x40]);
    }
    bytes.addAll([0xF0, for (var i = 0; i < 5000; i++) i & 0x7F, 0xF7]);
    for (final protocol in [1, 2]) {
      final a = BytesToUmp(group: 5, protocol: protocol);
      final want = [for (final x in bytes) ...a.push(x)];
      expect(BytesToUmp(group: 5, protocol: protocol).feed(bytes), want);
    }
    expect(
      () => BytesToUmp().feed([0x90, 256]),
      throwsA(isA<InvalidValueException>()),
    );
    final r = BytesToUmp()..free();
    expect(() => r.feed([0x90]), throwsA(isA<RejectedException>()));
  });

  test('UMP to bytes with running status and refresh', () {
    final w = UmpToBytes(runningStatus: true);
    expect(w.write([0x20903C64]), [0x90, 0x3C, 0x64]);
    expect(w.write([0x20903E40]), [0x3E, 0x40]);
    w.refreshStatus();
    expect(w.write([0x20903E40]), [0x90, 0x3E, 0x40]);
  });

  test('bad values and a freed object', () {
    expect(() => BytesToUmp(group: 16), throwsA(isA<InvalidValueException>()));
    expect(
      () => BytesToUmp(protocol: 3),
      throwsA(isA<InvalidValueException>()),
    );
    final r = BytesToUmp()..free();
    expect(() => r.push(0x90), throwsA(isA<RejectedException>()));
  });
}
