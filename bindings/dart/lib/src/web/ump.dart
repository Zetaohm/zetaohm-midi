/// The web form of the UMP functions: the TypeScript package through dart:js_interop. The values
/// are checked by `ump.dart`.
library;

import 'dart:js_interop';

import 'runtime.dart';

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
int wordCount(int word0) => intOf(tsCall('wordCount', [word0.toJS]));

/// The fields of the UMP [words].
UmpFields parse(List<int> words) {
  final u = tsCall<JSObject>('parse', [jsInts(words)]);
  return (
    messageType: intOf(prop(u, 'messageType')),
    group: intOrNull(prop(u, 'group')),
    status: intOrNull(prop(u, 'status')),
    channel: intOrNull(prop(u, 'channel')),
    decoded: boolOf(prop(u, 'decoded')),
    words: ints(prop(u, 'words')),
  );
}

List<int> _writer(String name, List<int> args) =>
    ints(tsCall(name, [for (final a in args) a.toJS]));

/// MIDI 2.0 Note On.
List<int> midi2NoteOn(int group, int channel, int note, int velocity) =>
    _writer('midi2NoteOn', [group, channel, note, velocity]);

/// MIDI 2.0 Note Off.
List<int> midi2NoteOff(int group, int channel, int note, int velocity) =>
    _writer('midi2NoteOff', [group, channel, note, velocity]);

/// MIDI 2.0 Control Change.
List<int> midi2ControlChange(int group, int channel, int index, int data) =>
    _writer('midi2ControlChange', [group, channel, index, data]);

/// MIDI 2.0 Program Change, with the Bank when [bank] is given.
List<int> midi2ProgramChange(
  int group,
  int channel,
  int program,
  (int, int)? bank,
) => ints(
  tsCall('midi2ProgramChange', [
    group.toJS,
    channel.toJS,
    program.toJS,
    if (bank != null) jsInts([bank.$1, bank.$2]),
  ]),
);

/// MIDI 2.0 Channel Pressure.
List<int> midi2ChannelPressure(int group, int channel, int value) =>
    _writer('midi2ChannelPressure', [group, channel, value]);

/// MIDI 2.0 Pitch Bend.
List<int> midi2PitchBend(int group, int channel, int value) =>
    _writer('midi2PitchBend', [group, channel, value]);

/// The MIDI 1.0 translation of [words], or null.
List<int>? midi2ToMidi1(List<int> words) {
  final r = tsCall<JSAny?>('midi2ToMidi1', [jsInts(words)]);
  return r == null ? null : ints(r);
}

/// M2-115 upscale.
int upscale(int value, int srcBits, int dstBits) =>
    intOf(tsCall('upscale', [value.toJS, srcBits.toJS, dstBits.toJS]));

/// M2-115 downscale.
int downscale(int value, int srcBits, int dstBits) =>
    intOf(tsCall('downscale', [value.toJS, srcBits.toJS, dstBits.toJS]));
