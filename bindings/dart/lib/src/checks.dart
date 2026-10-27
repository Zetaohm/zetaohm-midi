/// The value checks of the package, in plain Dart for the native and the web forms. The entry
/// library does not export this library.
library;

import 'errors.dart';
import 'limits.dart' if (dart.library.js_interop) 'web/limits.dart';

export 'limits.dart' if (dart.library.js_interop) 'web/limits.dart';

/// The major version of the C ABI that this package reads (ZMIDI_ABI_MAJOR).
const abiMajor = 0;

/// The lowest minor version of the C ABI that has every function that this package calls
/// (ZMIDI_ABI_MINOR 7: zmidi_bytes_to_ump_feed; 5: the USB descriptor writer and the Group
/// Terminal Block requests; 4:
/// zmidi_ci_pe_end; 3: the Property Exchange engine; 2: the Profile and Process Inquiry calls of ci
/// and the net-auth calls).
const abiMinor = 7;

/// Throws when the C ABI version [v] (major << 16 | minor) has another major version than
/// [abiMajor], or a minor version below [abiMinor].
void checkAbiVersion(int v) {
  if (v >> 16 != abiMajor || (v & 0xffff) < abiMinor) {
    throw MidiException(
      'the native library has C ABI ${v >> 16}.${v & 0xffff}; '
      'this package reads $abiMajor.$abiMinor or a later $abiMajor.x',
    );
  }
}

/// Checks that `value` is from 0 to `max`; gives it.
int checkInt(String name, int value, int max) {
  if (value < 0 || value > max) {
    throw InvalidValueException('$name must be from 0 to $max, not $value');
  }
  return value;
}

/// Checks a time or a tick count: 0 to [maxTime].
int checkTime(String name, int value) => checkInt(name, value, maxTime);

/// Checks a protocol number: 1 (MIDI 1.0) or 2 (MIDI 2.0).
int checkProtocol(int p) {
  if (p != 1 && p != 2) {
    throw InvalidValueException(
      'the protocol is 1 (MIDI 1.0) or 2 (MIDI 2.0), not $p',
    );
  }
  return p;
}

/// Checks the words of one UMP: 1 to 4 values of 32 bits.
List<int> checkUmp(List<int> words) {
  if (words.isEmpty || words.length > 4) {
    throw InvalidValueException('a UMP has 1 to 4 words, not ${words.length}');
  }
  for (var i = 0; i < words.length; i++) {
    checkInt('word $i', words[i], 0xffffffff);
  }
  return words;
}

/// Checks bytes: each 0 to 255.
List<int> checkBytes(String name, List<int> data) {
  for (var i = 0; i < data.length; i++) {
    checkInt('$name byte $i', data[i], 0xff);
  }
  return data;
}

/// Throws when an object is freed: `freed` is true after its `free` call. [owner] is the object
/// or its class name.
void checkFreed(bool freed, Object owner) {
  if (freed) {
    final name = owner is String ? owner : owner.runtimeType.toString();
    throw RejectedException('the $name is freed');
  }
}
