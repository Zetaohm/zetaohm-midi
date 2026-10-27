// The errors of the package. Every function throws one of these, never a string.

/** Base class of the zetaohm-midi errors. */
export class MidiError extends Error {
  constructor(message: string) {
    super(message);
    this.name = new.target.name;
  }
}

/** A value is outside the range that the specification allows. */
export class InvalidValueError extends MidiError {}

/** The data is not a valid message or file. */
export class MalformedError extends MidiError {}

/** The library refused the request in its current state. */
export class RejectedError extends MidiError {}
