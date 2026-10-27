/// The web form of the clip reader and writer: the TypeScript package through dart:js_interop. The
/// values are checked by `file.dart`.
library;

import 'dart:js_interop';
import 'dart:typed_data';

import 'runtime.dart';

/// One Set Profile On of the configuration: the values are checked.
typedef ClipProfileOn = ({
  List<int> profileId,
  int destination,
  int channel,
  int channels,
});

/// The checked configuration of a clip.
typedef ClipConfig = ({
  int ticksPerQuarterNote,
  int group,
  int? tempo10ns,
  (int, int, int)? timeSignature,
  List<ClipProfileOn> profiles,
});

const _destinations = ['channel', 'group', 'functionBlock'];

/// Reads all messages of a MIDI Clip File: `(section, ticks, words)`, section 0 for the Clip
/// Configuration Header.
List<(int, int, List<int>)> readClip(List<int> data) => [
  for (final e in tsCall<JSArray<JSObject>>('readClip', [jsBytes(data)]).toDart)
    (
      (prop<JSString>(e, 'section')).toDart == 'config' ? 0 : 1,
      intOf(prop(e, 'ticks')),
      ints(prop(e, 'words')),
    ),
];

/// The TypeScript ClipWriter.
final class ClipWriterState {
  /// A writer of the checked configuration [cfg].
  ClipWriterState(ClipConfig cfg)
    : _o = tsNew('ClipWriter', [
        jsObject({
          'ticksPerQuarterNote': cfg.ticksPerQuarterNote.toJS,
          'group': cfg.group.toJS,
          'tempo10ns': cfg.tempo10ns?.toJS,
          'timeSignature': switch (cfg.timeSignature) {
            final t? => jsInts([t.$1, t.$2, t.$3]),
            null => null,
          },
          'profiles': [
            for (final p in cfg.profiles)
              jsObject({
                'profileId': jsBytes(p.profileId),
                'destination': _destinations[p.destination].toJS,
                'channel': p.channel.toJS,
                'channels': p.channels.toJS,
              }),
          ].toJS,
        }),
      ]);

  final JSObject _o;

  /// Writes one more message of the Clip Configuration Header.
  void configEvent(List<int> words) =>
      call<JSAny?>(_o, 'configEvent', [jsInts(words)]);

  /// Writes Start of Clip.
  void start(int tick, int? tempo10ns) => call<JSAny?>(_o, 'start', [
    tick.toJS,
    if (tempo10ns != null) tempo10ns.toJS,
  ]);

  /// Writes one message of the Clip Sequence Data.
  void event(int tick, List<int> words) =>
      call<JSAny?>(_o, 'event', [tick.toJS, jsInts(words)]);

  /// Writes End of Clip and gives the bytes of the clip.
  Uint8List finish(int tick) => bytes(call(_o, 'finish', [tick.toJS]));

  /// Frees the WASM memory.
  void free() => call<JSAny?>(_o, 'free');
}
