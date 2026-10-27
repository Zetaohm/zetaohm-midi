/// MIDI Clip Files (zmidi-file through the C ABI, M2-116): a reader of all messages and a writer that
/// checks each call against the rules of zmidi-file.
library;

import 'dart:typed_data';

import 'checks.dart';
import 'errors.dart';
import 'native/file.dart' if (dart.library.js_interop) 'web/file.dart' as be;

/// The section of a clip message.
enum ClipSection {
  /// The Clip Configuration Header.
  config,

  /// The Clip Sequence Data.
  sequence,
}

/// One message of a clip.
final class ClipEvent {
  const ClipEvent._(this.section, this.ticks, this.words);

  /// The section of the message.
  final ClipSection section;

  /// Absolute ticks since the start of the file, in the unit of the DCTPQ in force.
  final int ticks;

  /// The words of the UMP.
  final List<int> words;
}

/// The destination of a Set Profile On message.
enum ClipDestination {
  /// One Channel.
  channel,

  /// The whole Group.
  group,

  /// The whole Function Block.
  functionBlock,
}

/// One Set Profile On message of the Clip Configuration Header.
final class ClipProfile {
  /// A Set Profile On of [profileId] (5 bytes) to [destination].
  const ClipProfile(
    this.profileId,
    this.destination, {
    this.channel = 0,
    this.channels = 0,
  });

  /// Profile ID, 5 bytes of 7-bit data.
  final List<int> profileId;

  /// The destination.
  final ClipDestination destination;

  /// Channel, 0 to 15, when the destination is [ClipDestination.channel].
  final int channel;

  /// Number of Channels Requested (14 bits).
  final int channels;
}

/// Reads all messages of a MIDI Clip File. Throws [MalformedException] for a bad file.
List<ClipEvent> readClip(List<int> data) => [
  for (final (section, ticks, words) in be.readClip(data))
    ClipEvent._(
      section == 0 ? ClipSection.config : ClipSection.sequence,
      ticks,
      words,
    ),
];

/// Writes a MIDI Clip File. Each call is checked at once against the rules of zmidi-file; a call
/// that fails changes nothing. [finish] ends the clip and gives its bytes.
final class ClipWriter {
  /// A clip with Ticks Per Quarter Note [ticksPerQuarterNote] (1 to 65535), Flex Data and Profile
  /// messages on [group], a Set Tempo of [tempo10ns] (units of 10 ns per quarter note) when given,
  /// a Set Time Signature `(numerator, denominator as a power of 2, 1/32 notes)` when given, and
  /// at most 16 Set Profile On messages.
  ClipWriter({
    int ticksPerQuarterNote = 96,
    int group = 0,
    int? tempo10ns,
    (int, int, int)? timeSignature,
    List<ClipProfile> profiles = const [],
  }) : _b = be.ClipWriterState(
         _config(
           ticksPerQuarterNote,
           group,
           tempo10ns,
           timeSignature,
           profiles,
         ),
       );

  static be.ClipConfig _config(
    int ticksPerQuarterNote,
    int group,
    int? tempo10ns,
    (int, int, int)? timeSignature,
    List<ClipProfile> profiles,
  ) {
    checkInt('group', group, 15);
    checkInt('ticksPerQuarterNote', ticksPerQuarterNote, 0xffff);
    if (profiles.length > 16) {
      throw const InvalidValueException(
        'a clip has at most 16 Set Profile On messages',
      );
    }
    if (tempo10ns != null) {
      checkInt('tempo10ns', tempo10ns, 0xffffffff);
    }
    if (timeSignature != null) {
      checkInt('numerator', timeSignature.$1, 255);
      checkInt('denominator', timeSignature.$2, 255);
      checkInt('thirtySecondNotes', timeSignature.$3, 255);
    }
    final ps = <be.ClipProfileOn>[];
    for (final p in profiles) {
      if (p.profileId.length != 5) {
        throw const InvalidValueException('a Profile ID has 5 bytes');
      }
      for (var k = 0; k < 5; k++) {
        checkInt('profileId', p.profileId[k], 0x7f);
      }
      ps.add((
        profileId: List.of(p.profileId),
        destination: p.destination.index,
        channel: checkInt('channel', p.channel, 15),
        channels: checkInt('channels', p.channels, 0x3fff),
      ));
    }
    return (
      ticksPerQuarterNote: ticksPerQuarterNote,
      group: group,
      tempo10ns: tempo10ns,
      timeSignature: timeSignature,
      profiles: ps,
    );
  }

  final be.ClipWriterState _b;
  bool _freed = false;
  bool _finished = false;

  be.ClipWriterState get _live {
    checkFreed(_freed, this);
    if (_finished) {
      throw const RejectedException('the clip is finished');
    }
    return _b;
  }

  /// Writes one more message of the Clip Configuration Header.
  void configEvent(List<int> words) {
    final w = List.of(checkUmp(words));
    _live.configEvent(w);
  }

  /// Writes Start of Clip at [tick], with a Set Tempo of [tempo10ns] when given, and starts the
  /// Clip Sequence Data.
  void start({int tick = 0, int? tempo10ns}) {
    checkTime('tick', tick);
    if (tempo10ns != null) {
      checkInt('tempo10ns', tempo10ns, 0xffffffff);
    }
    _live.start(tick, tempo10ns);
  }

  /// Writes one message of the Clip Sequence Data at absolute [tick].
  void event(int tick, List<int> words) {
    checkTime('tick', tick);
    final w = List.of(checkUmp(words));
    _live.event(tick, w);
  }

  /// Writes End of Clip at [tick] and gives the bytes of the clip. Later calls throw
  /// [RejectedException].
  Uint8List finish(int tick) {
    checkTime('tick', tick);
    final r = _live.finish(tick);
    _finished = true;
    return r;
  }

  /// Frees the memory of the writer now.
  void free() {
    if (!_freed) {
      _freed = true;
      _b.free();
    }
  }
}
