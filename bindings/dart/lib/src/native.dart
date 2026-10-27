/// Internal helpers of the package over the C ABI of zmidi-ffi (DEC-019): the storage of each
/// object, the checks of the values, and the map of a ZmidiStatus to an exception. The entry
/// library does not export this library.
library;

import 'dart:ffi';
import 'dart:typed_data';

import 'package:ffi/ffi.dart';

import 'bindings.g.dart';
import 'errors.dart';

import 'checks.dart';

export 'checks.dart';

bool _checked = false;

/// Throws when the loaded library has another C ABI major version, or a minor version below
/// [abiMinor] (DEC-019 rule 6).
void checkAbi() {
  if (_checked) {
    return;
  }
  checkAbiVersion(zmidi_abi_version());
  _checked = true;
}

/// Frees the C storage of an object when the garbage collector removes it.
final stateFinalizer = NativeFinalizer(calloc.nativeFree);

/// Gives `size` bytes of zeroed C storage, aligned to 16 (the largest alignment of the C ABI),
/// that `owner` owns until `release` or until the garbage collector removes it.
Pointer<T> allocState<T extends NativeType>(Finalizable owner, int size) {
  checkAbi();
  final p = calloc<Uint8>(size);
  if (p.address % 16 != 0) {
    calloc.free(p);
    throw MidiException('the allocator gave storage that is not aligned to 16');
  }
  stateFinalizer.attach(owner, p.cast(), detach: owner, externalSize: size);
  return p.cast();
}

/// Frees the storage of `owner` from `allocState` now.
void releaseState(Finalizable owner, Pointer<NativeType> p) {
  stateFinalizer.detach(owner);
  calloc.free(p);
}

/// A native block that one object keeps for the arguments of its calls (an [Allocator] with the
/// use of `using`), so that a call on a hot path does not allocate native memory. Each [run]
/// starts at the start of the block; a request that does not fit takes a temporary block that
/// the run frees, and the next run makes the kept block large enough. The memory of each request
/// is zeroed, as `calloc` gives it.
final class Scratch implements Allocator, Finalizable {
  Pointer<Uint8> _p = nullptr;
  int _size = 0;
  int _at = 0;
  int _need = 0;
  final List<Pointer<Uint8>> _extra = [];

  /// Runs [f] with this allocator; the memory of its requests is valid until [f] returns.
  T run<T>(T Function(Scratch arena) f) {
    if (_need > _size) {
      if (_p != nullptr) {
        releaseState(this, _p);
      }
      _size = (_need + 15) ~/ 16 * 16;
      _p = allocState(this, _size);
    }
    _at = 0;
    _need = 0;
    try {
      return f(this);
    } finally {
      for (final e in _extra) {
        calloc.free(e);
      }
      _extra.clear();
    }
  }

  @override
  Pointer<T> allocate<T extends NativeType>(int byteCount, {int? alignment}) {
    final start = (_at + 15) ~/ 16 * 16;
    final end = start + (byteCount < 1 ? 1 : byteCount);
    _at = end;
    _need = end;
    if (end <= _size) {
      final p = _p + start;
      p.asTypedList(end - start).fillRange(0, end - start, 0);
      return p.cast();
    }
    final e = calloc<Uint8>(end - start);
    _extra.add(e);
    return e.cast();
  }

  /// Nothing: [run] frees the memory.
  @override
  void free(Pointer<NativeType> pointer) {}

  /// Frees the kept block now.
  void release() {
    if (_p != nullptr) {
      releaseState(this, _p);
      _p = nullptr;
      _size = 0;
    }
  }
}

/// Throws the exception of a status that is not OK. `what` names the call.
void check(ZmidiStatus status, String what) {
  switch (status) {
    case ZmidiStatus.ZMIDI_STATUS_OK:
      return;
    case ZmidiStatus.ZMIDI_STATUS_INVALID_ARGUMENT:
      throw InvalidValueException('$what: a value is out of its range');
    case ZmidiStatus.ZMIDI_STATUS_NO_PENDING:
      throw RejectedException('$what: no pending request');
    case ZmidiStatus.ZMIDI_STATUS_REJECTED:
      throw RejectedException('$what: refused in the current state');
    case ZmidiStatus.ZMIDI_STATUS_NOT_SUPPORTED:
      throw RejectedException('$what: not supported by this build');
    case ZmidiStatus.ZMIDI_STATUS_BUFFER_TOO_SMALL:
      // The package sizes every buffer, so this is a defect of the package.
      throw MidiException('$what: buffer too small');
  }
}

/// Copies `data` into new memory of `arena`.
Pointer<Uint8> bytesIn(Allocator arena, List<int> data) {
  final p = arena<Uint8>(data.isEmpty ? 1 : data.length);
  p.asTypedList(data.isEmpty ? 1 : data.length).setAll(0, data);
  return p;
}

/// Copies `words` into new memory of `arena`.
Pointer<Uint32> wordsIn(Allocator arena, List<int> words) {
  final p = arena<Uint32>(words.isEmpty ? 1 : words.length);
  p.asTypedList(words.isEmpty ? 1 : words.length).setAll(0, words);
  return p;
}

/// A copy of `len` bytes at `p`.
Uint8List bytesOut(Pointer<Uint8> p, int len) =>
    len == 0 ? Uint8List(0) : Uint8List.fromList(p.asTypedList(len));

/// The word count of each Message Type 0x0 to 0xF (M2-104 Table 4), so that [splitUmps] needs no
/// FFI call for each UMP.
const _wordCounts = [1, 1, 1, 2, 2, 4, 1, 1, 2, 2, 2, 3, 3, 4, 4, 4];

/// Splits words into UMPs by the word count of each first word (M2-104 Table 4).
List<List<int>> splitUmps(List<int> words) {
  final out = <List<int>>[];
  var at = 0;
  while (at < words.length) {
    final n = _wordCounts[(words[at] >> 28) & 0xF];
    out.add(words.sublist(at, (at + n).clamp(0, words.length)));
    at += n;
  }
  return out;
}

/// The records `[group][len lo][len hi][bytes]` of `len` bytes at `p`: `(group, message)`.
List<(int, Uint8List)> records(Pointer<Uint8> p, int len) {
  final b = p.asTypedList(len);
  final out = <(int, Uint8List)>[];
  var at = 0;
  while (at + 3 <= b.length) {
    final n = b[at + 1] | (b[at + 2] << 8);
    out.add((b[at], Uint8List.fromList(b.sublist(at + 3, at + 3 + n))));
    at += 3 + n;
  }
  return out;
}

/// Throws when the object is freed.
void checkLive(Pointer<NativeType> p, Object owner) {
  if (p == nullptr) {
    throw RejectedException('the ${owner.runtimeType} is freed');
  }
}
