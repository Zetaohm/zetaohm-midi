// UMP functions (REQ-DART-UMP-001, REQ-DART-UMP-002): M2-104 Table 4, 7.4 figures, Appendix D.2,
// M2-115 Tables 5 and 8.
import 'package:test/test.dart';
import 'package:zetaohm_midi/zetaohm_midi.dart';

void main() {
  setUpAll(initWeb);

  test('word count of Table 4', () {
    const want = [1, 1, 1, 2, 2, 4, 1, 1, 2, 2, 2, 3, 3, 4, 4, 4];
    expect([for (var mt = 0; mt < 16; mt++) wordCount(mt << 28)], want);
  });

  test('MIDI 2.0 writers match the figures', () {
    expect(midi2NoteOn(0, 0, 60, 0xC000), [0x40903C00, 0xC0000000]);
    expect(midi2ControlChange(1, 0, 7, 0xFFFFFFFF), [0x41B00700, 0xFFFFFFFF]);
    expect(midi2ProgramChange(0, 0, 5, bank: (1, 2)), [0x40C00001, 0x05000102]);
    expect(midi2PitchBend(2, 15, 0x80000000), [0x42EF0000, 0x80000000]);
  });

  test('writers reject values out of range', () {
    expect(
      () => midi2NoteOn(16, 0, 60, 1),
      throwsA(isA<InvalidValueException>()),
    );
    expect(
      () => midi2NoteOn(0, 0, 128, 1),
      throwsA(isA<InvalidValueException>()),
    );
    expect(const InvalidValueException('x'), isA<MidiException>());
  });

  test('parse gives the fields', () {
    final u = parse([0x43923C00, 0xC0000000]);
    expect(
      [u.messageType, u.group, u.status, u.channel, u.decoded],
      [4, 3, 9, 2, true],
    );
    expect(u.words, [0x43923C00, 0xC0000000]);
    expect(() => parse([0x43923C00]), throwsA(isA<MalformedException>()));
  });

  test('MIDI 2.0 to 1.0 of Appendix D.2', () {
    expect(midi2ToMidi1(midi2NoteOn(0, 0, 60, 0xC000)), [0x20903C60]);
    expect(midi2ToMidi1(midi2NoteOn(0, 0, 60, 0x0100)), [0x20903C01]);
    expect(midi2ToMidi1([0x20903C64]), isNull);
  });

  test('scaling of M2-115', () {
    expect(
      [
        for (final v in [0x00, 0x40, 0x46, 0x60, 0x7F]) upscale(v, 7, 16),
      ],
      [0, 0x8000, 0x8C30, 0xC104, 0xFFFF],
    );
    expect(
      [
        for (final v in [0x1400, 0x8000, 0xAEBA, 0xFFFF]) downscale(v, 16, 7),
      ],
      [0x0A, 0x40, 0x57, 0x7F],
    );
    expect(() => upscale(0x80, 7, 16), throwsA(isA<InvalidValueException>()));
  });
}
