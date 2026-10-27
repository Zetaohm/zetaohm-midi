/// UMP functions (zmidi-ump through the C ABI): the word count of M2-104 Table 4, a parse into
/// plain fields, writers of the MIDI 2.0 Channel Voice Messages (M2-104 7.4), the MIDI 2.0 to 1.0
/// translation (M2-104 Appendix D.2) and the scaling of M2-115.
library;

import 'checks.dart';
import 'errors.dart';
import 'native/ump.dart' if (dart.library.js_interop) 'web/ump.dart' as be;

/// One UMP as plain fields.
final class Ump {
  const Ump._(
    this.messageType,
    this.group,
    this.status,
    this.channel,
    this.decoded,
    this.words,
  );

  /// Message Type, 0x0 to 0xF.
  final int messageType;

  /// Group, 0 to 15, or null when the Message Type has no Group.
  final int? group;

  /// The opcode or status nibble (Message Types 0x0, 0x2 to 0x5) or the status byte (0x1), or
  /// null.
  final int? status;

  /// Channel, 0 to 15, or null when the Message Type has no Channel.
  final int? channel;

  /// True when this build decodes the Message Type.
  final bool decoded;

  /// The words of the UMP.
  final List<int> words;
}

/// The word count of a UMP from its first word (M2-104 Table 4).
int wordCount(int word0) => be.wordCount(checkInt('word0', word0, 0xffffffff));

/// Reads the first UMP of [words] into plain fields. Throws [MalformedException] when [words] is
/// too short.
Ump parse(List<int> words) {
  if (words.isEmpty) {
    throw const MalformedException('no words');
  }
  final n = wordCount(words[0]);
  if (words.length < n) {
    throw MalformedException(
      'the Message Type needs $n words, not ${words.length}',
    );
  }
  final i = be.parse(checkUmp(words.sublist(0, n)));
  return Ump._(i.messageType, i.group, i.status, i.channel, i.decoded, i.words);
}

/// MIDI 2.0 Note On (M2-104 7.4.2): [velocity] 16 bits.
List<int> midi2NoteOn(int group, int channel, int note, int velocity) =>
    be.midi2NoteOn(
      checkInt('group', group, 15),
      checkInt('channel', channel, 15),
      checkInt('note', note, 127),
      checkInt('velocity', velocity, 0xffff),
    );

/// MIDI 2.0 Note Off (M2-104 7.4.1): [velocity] 16 bits.
List<int> midi2NoteOff(int group, int channel, int note, int velocity) =>
    be.midi2NoteOff(
      checkInt('group', group, 15),
      checkInt('channel', channel, 15),
      checkInt('note', note, 127),
      checkInt('velocity', velocity, 0xffff),
    );

/// MIDI 2.0 Control Change (M2-104 7.4.6): [index] 7 bits, [data] 32 bits.
List<int> midi2ControlChange(int group, int channel, int index, int data) =>
    be.midi2ControlChange(
      checkInt('group', group, 15),
      checkInt('channel', channel, 15),
      checkInt('index', index, 127),
      checkInt('data', data, 0xffffffff),
    );

/// MIDI 2.0 Program Change (M2-104 7.4.9), with the Bank `(msb, lsb)` when [bank] is given.
List<int> midi2ProgramChange(
  int group,
  int channel,
  int program, {
  (int, int)? bank,
}) => be.midi2ProgramChange(
  checkInt('group', group, 15),
  checkInt('channel', channel, 15),
  checkInt('program', program, 127),
  bank == null
      ? null
      : (
          checkInt('bank msb', bank.$1, 127),
          checkInt('bank lsb', bank.$2, 127),
        ),
);

/// MIDI 2.0 Channel Pressure (M2-104 7.4.10): [value] 32 bits.
List<int> midi2ChannelPressure(int group, int channel, int value) =>
    be.midi2ChannelPressure(
      checkInt('group', group, 15),
      checkInt('channel', channel, 15),
      checkInt('value', value, 0xffffffff),
    );

/// MIDI 2.0 Pitch Bend (M2-104 7.4.11): [value] 32 bits, 0x80000000 is the center.
List<int> midi2PitchBend(int group, int channel, int value) =>
    be.midi2PitchBend(
      checkInt('group', group, 15),
      checkInt('channel', channel, 15),
      checkInt('value', value, 0xffffffff),
    );

/// Translates one MIDI 2.0 Channel Voice UMP to MIDI 1.0 Channel Voice UMPs (M2-104 Appendix
/// D.2). Gives null for a UMP that has no translation.
List<int>? midi2ToMidi1(List<int> words) => be.midi2ToMidi1(checkUmp(words));

void _checkScale(int value, int srcBits, int dstBits) {
  if (srcBits < 1 || srcBits > 32 || dstBits < 1 || dstBits > 32) {
    throw const InvalidValueException('a bit count is 1 to 32');
  }
  // 1 << 32 is 0 in JavaScript: compare with the largest value of the bit count another way.
  checkInt('value', value, srcBits == 32 ? 0xffffffff : (1 << srcBits) - 1);
}

/// Scales [value] of [srcBits] bits up to [dstBits] bits (M2-115, min-center-max).
int upscale(int value, int srcBits, int dstBits) {
  _checkScale(value, srcBits, dstBits);
  return be.upscale(value, srcBits, dstBits);
}

/// Scales [value] of [srcBits] bits down to [dstBits] bits (M2-115, a shift).
int downscale(int value, int srcBits, int dstBits) {
  _checkScale(value, srcBits, dstBits);
  return be.downscale(value, srcBits, dstBits);
}
