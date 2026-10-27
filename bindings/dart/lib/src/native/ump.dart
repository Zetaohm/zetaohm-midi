/// The native form of the UMP functions: zmidi-ump through dart:ffi and the C ABI. The values are
/// checked by `ump.dart`.
library;

import 'dart:ffi';

import 'package:ffi/ffi.dart';

import '../bindings.g.dart';
import '../native.dart';

/// The fields of one UMP.
typedef UmpFields = ({
  int messageType,
  int? group,
  int? status,
  int? channel,
  bool decoded,
  List<int> words,
});

/// The word count of a UMP from its first word.
int wordCount(int word0) {
  checkAbi();
  return zmidi_ump_word_count(word0);
}

/// The fields of the UMP [words].
UmpFields parse(List<int> words) {
  checkAbi();
  return using((arena) {
    final info = arena<ZmidiUmpInfo>();
    check(zmidi_ump_parse(wordsIn(arena, words), words.length, info), 'parse');
    final i = info.ref;
    return (
      messageType: i.message_type,
      group: i.has_group ? i.group : null,
      status: i.has_status ? i.status : null,
      channel: i.has_channel ? i.channel : null,
      decoded: i.decoded,
      words: [for (var k = 0; k < i.word_count; k++) i.words[k]],
    );
  });
}

List<int> _writer(String what, ZmidiStatus Function(Pointer<Uint32> out) call) {
  checkAbi();
  return using((arena) {
    final out = arena<Uint32>(2);
    check(call(out), what);
    return [out[0], out[1]];
  });
}

/// MIDI 2.0 Note On.
List<int> midi2NoteOn(int group, int channel, int note, int velocity) =>
    _writer(
      'midi2NoteOn',
      (out) => zmidi_ump_midi2_note_on(group, channel, note, velocity, out),
    );

/// MIDI 2.0 Note Off.
List<int> midi2NoteOff(int group, int channel, int note, int velocity) =>
    _writer(
      'midi2NoteOff',
      (out) => zmidi_ump_midi2_note_off(group, channel, note, velocity, out),
    );

/// MIDI 2.0 Control Change.
List<int> midi2ControlChange(int group, int channel, int index, int data) =>
    _writer(
      'midi2ControlChange',
      (out) => zmidi_ump_midi2_control_change(group, channel, index, data, out),
    );

/// MIDI 2.0 Program Change, with the Bank when [bank] is given.
List<int> midi2ProgramChange(
  int group,
  int channel,
  int program,
  (int, int)? bank,
) => _writer(
  'midi2ProgramChange',
  (out) => zmidi_ump_midi2_program_change(
    group,
    channel,
    program,
    bank != null,
    bank?.$1 ?? 0,
    bank?.$2 ?? 0,
    out,
  ),
);

/// MIDI 2.0 Channel Pressure.
List<int> midi2ChannelPressure(int group, int channel, int value) => _writer(
  'midi2ChannelPressure',
  (out) => zmidi_ump_midi2_channel_pressure(group, channel, value, out),
);

/// MIDI 2.0 Pitch Bend.
List<int> midi2PitchBend(int group, int channel, int value) => _writer(
  'midi2PitchBend',
  (out) => zmidi_ump_midi2_pitch_bend(group, channel, value, out),
);

/// The MIDI 1.0 translation of [words], or null.
List<int>? midi2ToMidi1(List<int> words) {
  checkAbi();
  return using((arena) {
    final out = arena<Uint32>(16);
    final count = arena<UintPtr>();
    final st = zmidi_ump_midi2_to_midi1(
      wordsIn(arena, words),
      words.length,
      out,
      16,
      count,
    );
    if (st == ZmidiStatus.ZMIDI_STATUS_NOT_SUPPORTED) {
      return null;
    }
    check(st, 'midi2ToMidi1');
    return [for (var i = 0; i < count.value; i++) out[i]];
  });
}

int _scale(
  String what,
  int value,
  int srcBits,
  int dstBits,
  ZmidiStatus Function(int, int, int, Pointer<Uint32>) f,
) {
  checkAbi();
  return using((arena) {
    final out = arena<Uint32>();
    check(f(value, srcBits, dstBits, out), what);
    return out.value;
  });
}

/// M2-115 upscale.
int upscale(int value, int srcBits, int dstBits) =>
    _scale('upscale', value, srcBits, dstBits, zmidi_ump_upscale);

/// M2-115 downscale.
int downscale(int value, int srcBits, int dstBits) =>
    _scale('downscale', value, srcBits, dstBits, zmidi_ump_downscale);
