/// The exceptions of the package. Every function throws one of these for a failure that the
/// program can handle.
library;

/// Base class of the zetaohm_midi exceptions.
class MidiException implements Exception {
  /// An exception with the text [message].
  const MidiException(this.message);

  /// What failed.
  final String message;

  @override
  String toString() => '$runtimeType: $message';
}

/// A value is outside the range that the specification allows.
class InvalidValueException extends MidiException {
  /// An exception with the text [message].
  const InvalidValueException(super.message);
}

/// The data is not a valid message or file.
class MalformedException extends MidiException {
  /// An exception with the text [message].
  const MalformedException(super.message);
}

/// The library refused the request in its current state.
class RejectedException extends MidiException {
  /// An exception with the text [message].
  const RejectedException(super.message);
}
