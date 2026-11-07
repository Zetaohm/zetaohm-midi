/// MIDI 1.0 typed messages on bytes and the MIDI 1.0 receiver states (zmidi-io through the C ABI,
/// module Midi1 of zmidi-ffi): M1 v4.2.1, RP-015, RP-018. A message has the bytes as on the wire
/// and no UMP Group.
library;

import 'dart:typed_data';

import 'checks.dart';
import 'errors.dart';
import 'native/midi1.dart' if (dart.library.js_interop) 'web/midi1.dart' as be;

/// The kind of a MIDI 1.0 message (the order of ZmidiMidi1Kind, from 1).
enum Midi1Kind {
  /// A Channel Voice or Channel Mode message (status 80H to EFH).
  channelVoice,

  /// A System Common or System Real Time message (status F1H to FFH, not F7H).
  system,

  /// A complete System Exclusive message, F0H to F7H.
  sysex,

  /// A System Exclusive message that was too long for the reader; only its length is known.
  sysexTooLong,
}

/// One MIDI 1.0 message with the bytes as on the wire, and no UMP Group.
final class Midi1Message {
  /// A Channel Voice (status 80H to EFH) or System (F1H to FFH, not F7H) message; [data1] and
  /// [data2] are 0 to 127, and a byte that the message does not have is 0.
  factory Midi1Message(int status, [int data1 = 0, int data2 = 0]) {
    checkInt('status', status, 255);
    checkInt('data1', data1, 127);
    checkInt('data2', data2, 127);
    if (status < 0x80 || status == 0xF0 || status == 0xF7) {
      throw InvalidValueException(
        'status must be 80H to EFH or F1H to FFH (not F7H); use Midi1Message.sysex',
      );
    }
    final kind = status < 0xF0 ? Midi1Kind.channelVoice : Midi1Kind.system;
    return Midi1Message._(kind, status, data1, data2, Uint8List(0), 0);
  }

  /// A System Exclusive message: F0H, data bytes 0 to 127, F7H.
  factory Midi1Message.sysex(List<int> data) {
    final ok =
        data.length >= 2 &&
        data.first == 0xF0 &&
        data.last == 0xF7 &&
        data.sublist(1, data.length - 1).every((b) => b >= 0 && b < 0x80);
    if (!ok) {
      throw InvalidValueException(
        'a System Exclusive message is F0H, data bytes 0 to 127, F7H',
      );
    }
    final bytes = Uint8List.fromList(data);
    return Midi1Message._(Midi1Kind.sysex, 0xF0, 0, 0, bytes, bytes.length);
  }

  const Midi1Message._(
    this.kind,
    this.status,
    this.data1,
    this.data2,
    this.data,
    this.length,
  );

  /// The message of a back-end record.
  factory Midi1Message._raw(be.RawMessage r) => Midi1Message._(
    Midi1Kind.values[(r.$1 < 1 ? 1 : r.$1) - 1],
    r.$2,
    r.$3,
    r.$4,
    r.$5,
    r.$6,
  );

  /// The kind.
  final Midi1Kind kind;

  /// The status byte; F0H for a System Exclusive message.
  final int status;

  /// The first data byte, or 0.
  final int data1;

  /// The second data byte, or 0.
  final int data2;

  /// The bytes of a System Exclusive message, F0H to F7H; empty for the other kinds.
  final Uint8List data;

  /// The length of a System Exclusive message (also when it was too long for the reader).
  final int length;

  be.RawMessage get _raw =>
      (kind.index + 1, status, data1, data2, data, length);

  @override
  bool operator ==(Object other) =>
      other is Midi1Message &&
      other.kind == kind &&
      other.status == status &&
      other.data1 == data1 &&
      other.data2 == data2 &&
      other.length == length &&
      _same(other.data, data);

  @override
  int get hashCode =>
      Object.hash(kind, status, data1, data2, length, Object.hashAll(data));

  @override
  String toString() =>
      'Midi1Message(${kind.name}, status: 0x${status.toRadixString(16)}, data1: $data1, '
      'data2: $data2, length: $length)';
}

bool _same(Uint8List a, Uint8List b) {
  if (a.length != b.length) {
    return false;
  }
  for (var i = 0; i < a.length; i++) {
    if (a[i] != b[i]) {
      return false;
    }
  }
  return true;
}

List<Midi1Message> _messages(List<be.RawMessage> raw) => [
  for (final r in raw) Midi1Message._raw(r),
];

/// Reads a MIDI 1.0 byte stream and gives typed messages; a System Exclusive message comes whole.
final class Midi1Reader {
  /// A reader that holds System Exclusive messages of up to [sysexCapacity] bytes (F0H to F7H); a
  /// longer one gives the kind [Midi1Kind.sysexTooLong] with its length.
  Midi1Reader({int sysexCapacity = 256})
    : _b = be.Midi1ReaderState(
        checkInt('sysexCapacity', sysexCapacity, 0x7FFFFFFF),
      );

  final be.Midi1ReaderState _b;
  bool _freed = false;

  /// Reads one byte; gives the messages that it completes (at most two).
  List<Midi1Message> push(int byte) {
    checkFreed(_freed, this);
    return _messages(_b.push(checkInt('byte', byte, 255)));
  }

  /// Reads [data]; gives all the messages that it completes, in order.
  List<Midi1Message> feed(List<int> data) {
    checkFreed(_freed, this);
    return [for (final b in checkBytes('data', data)) ..._messages(_b.push(b))];
  }

  /// The System Reset action of the program (AMB-M1-13): no running status, no partial message.
  void reset() {
    checkFreed(_freed, this);
    _b.reset();
  }

  /// Frees the memory of the reader now.
  void free() {
    if (!_freed) {
      _freed = true;
      _b.free();
    }
  }
}

/// Writes typed MIDI 1.0 messages as bytes.
final class Midi1Writer {
  /// A writer; [runningStatus] leaves out a status byte equal to the last one.
  Midi1Writer({bool runningStatus = false})
    : _b = be.Midi1WriterState(runningStatus);

  final be.Midi1WriterState _b;
  bool _freed = false;

  /// Gives the bytes of [message].
  Uint8List write(Midi1Message message) {
    checkFreed(_freed, this);
    return _b.write(message._raw);
  }

  /// Makes the next Channel Voice message send its status byte (a refresh of running status).
  void refreshStatus() {
    checkFreed(_freed, this);
    _b.refreshStatus();
  }

  /// Frees the memory of the writer now.
  void free() {
    if (!_freed) {
      _freed = true;
      _b.free();
    }
  }
}

/// The kind of a mode action (the order of ZmidiModeActionKind).
enum ModeActionKind {
  /// The message is not for this instrument.
  ignore,

  /// A voice message for the instrument; [ModeAction.voice] is the Mode 4 voice.
  voice,

  /// A Global Controller message (Mode 4).
  global,

  /// All Notes Off for [ModeAction.channel].
  allNotesOff,

  /// All Sound Off for [ModeAction.channel].
  allSoundOff,

  /// Reset All Controllers for [ModeAction.channel].
  resetAllControllers,

  /// Local Control: [ModeAction.on].
  localControl,

  /// A mode message: all notes off for all voices; [ModeAction.changed] and
  /// [ModeAction.alternate].
  modeChange,
}

/// The action of one received message for a mode state.
final class ModeAction {
  const ModeAction._(
    this.kind,
    this.channel,
    this.voice,
    this.on,
    this.changed,
    this.alternate,
  );

  /// The kind.
  final ModeActionKind kind;

  /// The channel of a Mode 4 action, or null for all channels of the instrument.
  final int? channel;

  /// The Mode 4 voice of a voice message, or null.
  final int? voice;

  /// Local Control on.
  final bool on;

  /// The mode message changed the mode.
  final bool changed;

  /// The mode in use is an alternate mode.
  final bool alternate;

  @override
  String toString() =>
      'ModeAction(${kind.name}, channel: $channel, voice: $voice, on: $on, changed: $changed, '
      'alternate: $alternate)';
}

ModeAction _modeAction((int, int, int, bool, bool, bool) a) => ModeAction._(
  ModeActionKind.values[a.$1],
  a.$2 == 0xFF ? null : a.$2,
  a.$3 == 0xFF ? null : a.$3,
  a.$4,
  a.$5,
  a.$6,
);

/// The MIDI 1.0 receiver mode state of one instrument (Modes 1 to 4, M1 pp. 20 to 26).
final class ModeState {
  /// A mode state in Mode 1 with Local Control on. [implemented] lists the mode numbers that the
  /// instrument implements (Mode 1 always); [alternate] selects the alternate mode of p. 23 for the
  /// others (else the Mode message is ignored); [voices] is the count for M = 0.
  ModeState({
    int basicChannel = 0,
    List<int> implemented = const [1, 2, 3, 4],
    bool alternate = true,
    int voices = 16,
    bool globalControllers = false,
  }) : _b = be.ModeCore(
         checkInt('basicChannel', basicChannel, 15),
         _mask(implemented),
         alternate,
         checkInt('voices', voices, 255),
         globalControllers,
       );

  static int _mask(List<int> modes) {
    var mask = 0;
    for (final n in modes) {
      if (checkInt('implemented', n, 4) < 1) {
        throw InvalidValueException('mode numbers are 1 to 4');
      }
      mask |= 1 << (n - 1);
    }
    return mask;
  }

  final be.ModeCore _b;
  bool _freed = false;

  /// Takes one Channel Voice message; gives its action.
  ModeAction receive(Midi1Message message) {
    checkFreed(_freed, this);
    return _modeAction(_b.receive(message._raw));
  }

  /// The System Reset action: Omni On, Poly and Local On; the Basic Channel and M stay.
  void systemReset() {
    checkFreed(_freed, this);
    _b.systemReset();
  }

  be.ModeInfo get _info {
    checkFreed(_freed, this);
    return _b.info();
  }

  /// The mode in use, 1 to 4.
  int get mode => _info.$1;

  /// The mode of the Omni and Mono flags, 1 to 4.
  int get requestedMode => _info.$2;

  /// The mode in use is an alternate mode.
  bool get isAlternate => _info.$3;

  /// The Basic Channel, 0 to 15.
  int get basicChannel => _info.$4;

  set basicChannel(int channel) {
    checkFreed(_freed, this);
    _b.setBasicChannel(checkInt('basicChannel', channel, 15));
  }

  /// M of the last Mono On.
  int get m => _info.$5;

  /// Local Control on.
  bool get local => _info.$6;

  set local(bool on) {
    checkFreed(_freed, this);
    _b.setLocal(on);
  }

  /// Sets the Omni flag, as Omni On or Omni Off does.
  void setOmni(bool on) {
    checkFreed(_freed, this);
    _b.setOmni(on);
  }

  /// Selects Poly, as Poly On does.
  void setPoly() {
    checkFreed(_freed, this);
    _b.setPoly();
  }

  /// Selects Mono with [m] (0 to 127), as Mono On does.
  void setMono(int m) {
    checkFreed(_freed, this);
    _b.setMono(checkInt('m', m, 127));
  }

  /// Frees the memory of the state now.
  void free() {
    if (!_freed) {
      _freed = true;
      _b.free();
    }
  }
}

/// The kind of a controller event (the order of ZmidiControllerEventKind, from 1).
enum ControllerEventKind {
  /// A 14-bit controller: [ControllerEvent.controller] 0 to 31 and the 14-bit value.
  controller14,

  /// A single-byte controller: [ControllerEvent.controller] 64 to 119 and the value.
  controller7,

  /// Program Change: [ControllerEvent.program] and [ControllerEvent.bank].
  program,

  /// A Note On after a Portamento Control: [ControllerEvent.source].
  portamento,

  /// Reset All Controllers (RP-015): see [resetValue].
  resetAllControllers,
}

/// The event of one received message for a controller state; a field is null when the kind has
/// none.
final class ControllerEvent {
  const ControllerEvent._(
    this.kind,
    this.controller,
    this.value,
    this.program,
    this.bank,
    this.source,
  );

  /// The kind.
  final ControllerEventKind kind;

  /// The controller number.
  final int? controller;

  /// The 14-bit or 7-bit controller value.
  final int? value;

  /// The program number.
  final int? program;

  /// The 14-bit bank of a program event, or null before the first Bank Select.
  final int? bank;

  /// The Portamento Control source note.
  final int? source;

  @override
  String toString() =>
      'ControllerEvent(${kind.name}, controller: $controller, value: $value, program: $program, '
      'bank: $bank, source: $source)';
}

/// The MIDI 1.0 controller state of one channel: 14-bit pairs, bank and program, Portamento
/// Control, Reset All Controllers (M1 pp. 11 to 17, RP-015).
final class ControllerState {
  /// A new state: no bank and no Portamento Control source.
  ControllerState() : _b = be.ControllerCore();

  final be.ControllerCore _b;
  bool _freed = false;

  /// Takes one Channel Voice message of the channel of this state; gives its event, or null.
  ControllerEvent? receive(Midi1Message message) {
    checkFreed(_freed, this);
    final e = _b.receive(message._raw);
    if (e.$1 == 0) {
      return null;
    }
    final kind = ControllerEventKind.values[e.$1 - 1];
    final controller =
        kind == ControllerEventKind.controller14 ||
        kind == ControllerEventKind.controller7;
    final program = kind == ControllerEventKind.program;
    return ControllerEvent._(
      kind,
      controller ? e.$2 : null,
      controller ? e.$3 : null,
      program ? e.$4 : null,
      program && e.$5 ? e.$6 : null,
      kind == ControllerEventKind.portamento ? e.$7 : null,
    );
  }

  /// The Reset All Controllers action of the program (a ModeState action or General MIDI System
  /// On).
  void resetAllControllers() {
    checkFreed(_freed, this);
    _b.reset();
  }

  /// The GM reset of this state for [channel] (0 to 15): the GM2 defaults, the Rhythm bank on
  /// channel 10 and the Melody bank on the others (RP-024 3.3, 4.9.1).
  void gmReset(int channel) {
    checkFreed(_freed, this);
    _b.gmReset(checkInt('channel', channel, 15));
  }

  /// The held 14-bit bank, or null before the first Bank Select.
  int? get bank {
    checkFreed(_freed, this);
    return _b.bank();
  }

  /// Frees the memory of the state now.
  void free() {
    if (!_freed) {
      _freed = true;
      _b.free();
    }
  }
}

/// The RP-015 value of [controller] at Reset All Controllers, or null when it keeps its value.
int? resetValue(int controller) =>
    be.resetValue(checkInt('controller', controller, 255));

/// The two messages of a 14-bit controller value ([controller] 0 to 31, [value] 0 to 3FFFH): MSB,
/// then LSB.
List<Midi1Message> controller14Messages(
  int channel,
  int controller,
  int value,
) => _messages(
  be.controller14Messages(
    checkInt('channel', channel, 15),
    checkInt('controller', controller, 255),
    checkInt('value', value, 0xFFFF),
  ),
);

/// The three messages of a [bank] (0 to 3FFFH) and [program] (0 to 127) selection.
List<Midi1Message> bankProgramMessages(int channel, int bank, int program) =>
    _messages(
      be.bankProgramMessages(
        checkInt('channel', channel, 15),
        checkInt('bank', bank, 0xFFFF),
        checkInt('program', program, 255),
      ),
    );

/// A parameter number; `Parameter(0x7F, 0x7F)` is the null number, which ends a selection.
final class Parameter {
  /// A Registered (default) or Non-Registered Parameter Number; [msb] and [lsb] are 0 to 127.
  Parameter(int msb, int lsb, {this.registered = true})
    : msb = checkInt('msb', msb, 127),
      lsb = checkInt('lsb', lsb, 127);

  /// The MSB, 0 to 127.
  final int msb;

  /// The LSB, 0 to 127.
  final int lsb;

  /// True for a Registered Parameter Number (controllers 101 and 100).
  final bool registered;

  @override
  bool operator ==(Object other) =>
      other is Parameter &&
      other.msb == msb &&
      other.lsb == lsb &&
      other.registered == registered;

  @override
  int get hashCode => Object.hash(msb, lsb, registered);

  @override
  String toString() =>
      'Parameter(msb: $msb, lsb: $lsb, registered: $registered)';
}

/// The change of a parameter event (the order of ZmidiParameterChange, from 1).
enum ParameterChange {
  /// Data Entry MSB (controller 6).
  dataEntryMsb,

  /// Data Entry LSB (controller 38).
  dataEntryLsb,

  /// Data Increment (controller 96).
  increment,

  /// Data Decrement (controller 97).
  decrement,
}

/// The event of a Data Entry, Data Increment or Data Decrement for the selected parameter.
final class ParameterEvent {
  const ParameterEvent._(this.parameter, this.change, this.value);

  /// The selected parameter.
  final Parameter parameter;

  /// The change.
  final ParameterChange change;

  /// The Data Entry byte, or null for a step.
  final int? value;

  /// The new 14-bit value of the parameter from its [current] value (0 to 16383), with the RP-018
  /// steps: RPN 0 steps the cents with a carry at 100, RPN 2 to 4 step the MSB, the others the
  /// 14-bit value; a step stops at the limits.
  int apply(int current) => be.applyParameter(
    change.index + 1,
    parameter.registered,
    parameter.msb,
    parameter.lsb,
    value ?? 0,
    checkInt('current', current, 0xFFFF),
  );

  @override
  String toString() =>
      'ParameterEvent($parameter, ${change.name}, value: $value)';
}

/// The MIDI 1.0 RPN and NRPN receiver of one channel (M1 p. 17, RP-015, RP-018). It holds no
/// parameter value: the program holds them and uses [ParameterEvent.apply].
final class ParameterState {
  /// A receiver with RPN reception on and NRPN reception off.
  ParameterState() : _b = be.ParameterCore();

  final be.ParameterCore _b;
  bool _freed = false;

  /// Takes one Channel Voice message of the channel of this receiver; gives its event, or null.
  ParameterEvent? receive(Midi1Message message) {
    checkFreed(_freed, this);
    final e = _b.receive(message._raw);
    if (e.$1 == 0) {
      return null;
    }
    final change = ParameterChange.values[e.$1 - 1];
    final entry =
        change == ParameterChange.dataEntryMsb ||
        change == ParameterChange.dataEntryLsb;
    return ParameterEvent._(
      Parameter(e.$3, e.$4, registered: e.$2),
      change,
      entry ? e.$5 : null,
    );
  }

  /// The Reset All Controllers action of the program: no parameter is selected.
  void resetAllControllers() {
    checkFreed(_freed, this);
    _b.reset();
  }

  /// The parameter that a Data Entry acts on now, or null.
  Parameter? get selection {
    checkFreed(_freed, this);
    final s = _b.selection();
    return s == null ? null : Parameter(s.$2, s.$3, registered: s.$1);
  }

  /// RPN reception on.
  bool get rpnReception {
    checkFreed(_freed, this);
    return _b.reception().$1;
  }

  set rpnReception(bool on) {
    checkFreed(_freed, this);
    _b.setReception(on, _b.reception().$2);
  }

  /// NRPN reception on.
  bool get nrpnReception {
    checkFreed(_freed, this);
    return _b.reception().$2;
  }

  set nrpnReception(bool on) {
    checkFreed(_freed, this);
    _b.setReception(_b.reception().$1, on);
  }

  /// Frees the memory of the receiver now.
  void free() {
    if (!_freed) {
      _freed = true;
      _b.free();
    }
  }
}

/// The two messages of a parameter number: MSB, then LSB.
List<Midi1Message> selectMessages(int channel, Parameter parameter) =>
    _messages(
      be.selectMessages(
        checkInt('channel', channel, 15),
        parameter.registered,
        parameter.msb,
        parameter.lsb,
      ),
    );

/// The message of one [change]; [value] is the Data Entry byte (0 to 127).
Midi1Message changeMessage(
  int channel,
  ParameterChange change, [
  int value = 0,
]) => Midi1Message._raw(
  be.changeMessage(
    checkInt('channel', channel, 15),
    change.index + 1,
    checkInt('value', value, 255),
  ),
);

/// The four messages of a parameter [value] (0 to 3FFFH): the number, then Data Entry MSB and
/// LSB.
List<Midi1Message> valueMessages(int channel, Parameter parameter, int value) =>
    _messages(
      be.valueMessages(
        checkInt('channel', channel, 15),
        parameter.registered,
        parameter.msb,
        parameter.lsb,
        checkInt('value', value, 0xFFFF),
      ),
    );

/// The kind of a sync event (the order of ZmidiSyncEventKind, from 1).
enum SyncEventKind {
  /// Start: the position is 0 and the receiver waits for the next Timing Clock.
  start,

  /// The Continue message (FBH): the receiver waits at the held position for the next Timing
  /// Clock. (`continue` is a Dart keyword.)
  resume,

  /// Stop: the receiver stopped and keeps its position.
  stop,

  /// A Timing Clock while the receiver plays: [SyncEvent.position] is the position of this clock.
  clock,

  /// An accepted Song Position Pointer: [SyncEvent.position] is the new position.
  locate,

  /// An accepted Song Select: [SyncEvent.song] is the song number.
  song,
}

/// The run state of a sync receiver.
enum SyncRun {
  /// Stopped: a Timing Clock does not advance the position.
  stopped,

  /// After Start or Continue, until the first Timing Clock.
  waiting,

  /// Playing: each Timing Clock advances the position.
  playing,
}

/// A command of the program (a front panel), with the transitions of the MIDI command.
enum SyncCommand {
  /// Start: position 0, then play at the next Timing Clock.
  start,

  /// Continue: play from the held position at the next Timing Clock. (`continue` is a Dart
  /// keyword.)
  resume,

  /// Stop: stop at once and keep the position.
  stop,
}

/// The event of one message or command for a sync receiver; a field is null when the kind has
/// none.
final class SyncEvent {
  const SyncEvent._(this.kind, this.position, this.song);

  /// The kind.
  final SyncEventKind kind;

  /// The position in MIDI clocks of a clock or locate event.
  final int? position;

  /// The song number of a song event.
  final int? song;

  @override
  String toString() =>
      'SyncEvent(${kind.name}, position: $position, song: $song)';
}

/// The MIDI 1.0 sync receiver of a sequencer or drum machine: Start, Stop, Continue, Timing Clock,
/// Song Position Pointer and Song Select (M1 pp. 27 to 32). The position is a count of MIDI clocks.
final class SyncReceiver {
  /// A receiver that is stopped at position 0, with song 0, in MIDI sync mode.
  SyncReceiver() : _b = be.SyncCore();

  final be.SyncCore _b;
  bool _freed = false;

  SyncEvent? _event((int, int, int) e) {
    if (e.$1 == 0) {
      return null;
    }
    final kind = SyncEventKind.values[e.$1 - 1];
    final position =
        kind == SyncEventKind.clock || kind == SyncEventKind.locate;
    return SyncEvent._(
      kind,
      position ? e.$2 : null,
      kind == SyncEventKind.song ? e.$3 : null,
    );
  }

  /// Takes one System message; gives its event, or null.
  SyncEvent? receive(Midi1Message message) {
    checkFreed(_freed, this);
    return _event(_b.receive(message._raw));
  }

  /// Takes one command of the program with the transitions of the MIDI command; gives its event,
  /// or null.
  SyncEvent? command(SyncCommand command) {
    checkFreed(_freed, this);
    return _event(_b.command(command.index + 1));
  }

  /// The System Reset action of the program: position 0 and stopped; the song and the MIDI sync
  /// mode stay.
  void systemReset() {
    checkFreed(_freed, this);
    _b.systemReset();
  }

  (int, int, int, bool) get _info {
    checkFreed(_freed, this);
    return _b.info();
  }

  /// The position in MIDI clocks: the position of the next Timing Clock that plays.
  int get position => _info.$1;

  /// The run state.
  SyncRun get run => SyncRun.values[_info.$2];

  /// The song of the last accepted Song Select.
  int get song => _info.$3;

  /// MIDI sync mode on (the default); out of it the receiver ignores Song Position Pointer and
  /// Song Select.
  bool get syncMode => _info.$4;

  set syncMode(bool on) {
    checkFreed(_freed, this);
    _b.setMode(on);
  }

  /// Frees the memory of the receiver now.
  void free() {
    if (!_freed) {
      _freed = true;
      _b.free();
    }
  }
}

/// The default timeout of the Active Sensing monitor: 330 ms, in microseconds.
const activeSensingTimeoutUs = 330000;

/// The default interval of the Active Sensing sender: 270 ms, in microseconds.
const activeSensingIntervalUs = 270000;

/// The MIDI 1.0 Active Sensing monitor of a receiver (M1 pp. 32, 33); the program gives each time
/// in microseconds.
final class ActiveSensingMonitor {
  /// A monitor with a timeout of [timeoutUs] microseconds, not armed.
  ActiveSensingMonitor({this.timeoutUs = activeSensingTimeoutUs})
    : _b = be.MonitorCore(checkInt('timeoutUs', timeoutUs, 0xFFFFFFFF));

  /// The timeout, in microseconds.
  final int timeoutUs;
  final be.MonitorCore _b;
  bool _freed = false;

  /// Takes one received byte at the time [nowUs]: FEH arms the monitor; while it is armed, each
  /// byte restarts the timer.
  void receiveByte(int byte, int nowUs) {
    checkFreed(_freed, this);
    _b.receiveByte(checkInt('byte', byte, 255), checkTime('nowUs', nowUs));
  }

  /// Takes one received message of a transport with no bytes at the time [nowUs].
  void receive(bool activeSensing, int nowUs) {
    checkFreed(_freed, this);
    _b.receive(activeSensing, checkTime('nowUs', nowUs));
  }

  /// Checks the timer at the time [nowUs]: true one time when more than the timeout passed since
  /// the last received byte; the monitor then waits for FEH again.
  bool poll(int nowUs) {
    checkFreed(_freed, this);
    return _b.poll(checkTime('nowUs', nowUs));
  }

  /// The monitor received FEH and gave no timeout after it.
  bool get isArmed {
    checkFreed(_freed, this);
    return _b.isArmed();
  }

  /// Frees the memory of the monitor now.
  void free() {
    if (!_freed) {
      _freed = true;
      _b.free();
    }
  }
}

/// The MIDI 1.0 Active Sensing sender of a transmitter (M1 p. 32); the program gives each time in
/// microseconds.
final class ActiveSensingSender {
  /// A sender with an interval of [intervalUs] microseconds that starts at time 0.
  ActiveSensingSender({this.intervalUs = activeSensingIntervalUs})
    : _b = be.SenderCore(checkInt('intervalUs', intervalUs, 0xFFFFFFFF));

  /// The interval, in microseconds.
  final int intervalUs;
  final be.SenderCore _b;
  bool _freed = false;

  /// The program sent a byte at the time [nowUs]; the interval starts again.
  void sent(int nowUs) {
    checkFreed(_freed, this);
    _b.sent(checkTime('nowUs', nowUs));
  }

  /// Checks the interval at the time [nowUs]: true when no byte was sent for the interval; the
  /// program then sends one FEH byte, and the interval starts again.
  bool poll(int nowUs) {
    checkFreed(_freed, this);
    return _b.poll(checkTime('nowUs', nowUs));
  }

  /// Frees the memory of the sender now.
  void free() {
    if (!_freed) {
      _freed = true;
      _b.free();
    }
  }
}

/// A Zone of MIDI Polyphonic Expression (M1-100 2.2.1; the order of ZmidiMpeZone).
enum MpeZone {
  /// Manager Channel 1, Member Channels from Channel 2 upwards.
  lower,

  /// Manager Channel 16, Member Channels from Channel 15 downwards.
  upper,
}

/// The role of a channel in an MPE layout (the order of ZmidiMpeRole).
enum MpeRole {
  /// The channel is in no Zone: a conventional MIDI 1.0 channel.
  noZone,

  /// The Manager Channel of a Zone.
  manager,

  /// A Member Channel of a Zone.
  member,
}

/// An MPE Zone layout: the Member Channel count of each Zone (0 for an inactive Zone).
final class MpeLayout {
  const MpeLayout._(this.lower, this.upper);

  /// A layout with [lower] and [upper] Member Channels (0 to 15 each; two active Zones have at
  /// most 14 in all).
  factory MpeLayout([int lower = 0, int upper = 0]) {
    final (l, u) = be.mpeLayoutNew(
      checkInt('lower', lower, 255),
      checkInt('upper', upper, 255),
    );
    return MpeLayout._(l, u);
  }

  /// The Lower Zone with 15 Member Channels (M1-100 2.2.2).
  static const defaultLayout = MpeLayout._(15, 0);

  /// No Zone: MPE Mode off.
  static const none = MpeLayout._(0, 0);

  /// The Member Channels of the Lower Zone.
  final int lower;

  /// The Member Channels of the Upper Zone.
  final int upper;

  /// MPE Mode is on: at least one Zone is active.
  bool get isActive => lower > 0 || upper > 0;

  /// The layout after one MPE Configuration Message with [members] for [zone] (the newest MCM
  /// takes the channels; a value above 15 changes nothing).
  MpeLayout withMcm(MpeZone zone, int members) {
    final (l, u) = be.mpeLayoutWithMcm(
      lower,
      upper,
      zone.index,
      checkInt('members', members, 255),
    );
    return MpeLayout._(l, u);
  }

  /// The role of [channel] (0 to 15) and its Zone (null for no Zone).
  (MpeRole, MpeZone?) role(int channel) {
    final (r, z) = be.mpeRole(lower, upper, checkInt('channel', channel, 15));
    final role = MpeRole.values[r];
    return (role, role == MpeRole.noZone ? null : MpeZone.values[z]);
  }

  /// The lowest Member Channel of [zone], which takes the Mode messages, or null.
  int? lowestMember(MpeZone zone) =>
      be.mpeLowestMember(lower, upper, zone.index);

  @override
  bool operator ==(Object other) =>
      other is MpeLayout && other.lower == lower && other.upper == upper;

  @override
  int get hashCode => Object.hash(lower, upper);

  @override
  String toString() => 'MpeLayout(lower: $lower, upper: $upper)';
}

/// A Pitch Bend Sensitivity in semitones and cents.
final class MpeSensitivity {
  /// A sensitivity of [semitones] and [cents].
  const MpeSensitivity(this.semitones, [this.cents = 0]);

  /// The semitones.
  final int semitones;

  /// The cents.
  final int cents;

  @override
  bool operator ==(Object other) =>
      other is MpeSensitivity &&
      other.semitones == semitones &&
      other.cents == cents;

  @override
  int get hashCode => Object.hash(semitones, cents);

  @override
  String toString() => 'MpeSensitivity($semitones, $cents)';
}

/// The kind of an MPE event (the order of ZmidiMpeEventKind).
enum MpeEventKind {
  /// The channel is in no Zone: a conventional MIDI 1.0 message.
  noZone,

  /// A message on a channel of a Zone.
  zone,

  /// An MCM changed the layout; [MpeEvent.reset] holds the channels to reset.
  layout,

  /// RPN 0 changed the Manager or Member sensitivity of a Zone.
  sensitivity,

  /// A Mode message changed the MIDI Mode of a Zone.
  mode,

  /// A message that MPE prohibits or that has no function.
  ignored,
}

/// The event of one message for an MPE receiver.
final class MpeEvent {
  const MpeEvent._(this.kind, this.zone, this.manager, this.reset);

  /// The kind.
  final MpeEventKind kind;

  /// The Zone of a zone, sensitivity or mode event, or null.
  final MpeZone? zone;

  /// The Manager Channel (zone) or the Manager sensitivity (sensitivity).
  final bool manager;

  /// The channels to reset after a layout event: bit n is channel n.
  final int reset;

  @override
  bool operator ==(Object other) =>
      other is MpeEvent &&
      other.kind == kind &&
      other.zone == zone &&
      other.manager == manager &&
      other.reset == reset;

  @override
  int get hashCode => Object.hash(kind, zone, manager, reset);

  @override
  String toString() =>
      'MpeEvent(${kind.name}, zone: ${zone?.name}, manager: $manager, '
      'reset: 0x${reset.toRadixString(16)})';
}

/// Pitch units for one semitone: an MPE pitch offset is in hundredths of a cent.
const pitchUnitsPerSemitone = 10000;

int _checkI32(String name, int value) {
  if (value < -0x80000000 || value > 0x7FFFFFFF) {
    throw InvalidValueException(
      '$name must be a signed 32-bit integer, not $value',
    );
  }
  return value;
}

/// The C.3 Pitch Bend value (0 to 16383) of a sender for [offset] in hundredths of a cent at a
/// sensitivity of [semitones] and [cents].
int pitchBendValue(int offset, int semitones, [int cents = 0]) =>
    be.pitchBendValue(
      _checkI32('offset', offset),
      checkInt('semitones', semitones, 255),
      checkInt('cents', cents, 255),
    );

/// The C.5 pitch offset in hundredths of a cent of a received Pitch Bend [value] (0 to 16383) at a
/// sensitivity of [semitones] and [cents].
int pitchOffset(int value, int semitones, [int cents = 0]) => be.pitchOffset(
  checkInt('value', value, 0xFFFF),
  checkInt('semitones', semitones, 255),
  checkInt('cents', cents, 255),
);

/// A message kind of Appendix E (the order of ZmidiMpeMessage).
enum MpeMessageKind {
  /// RPN 00H 06H, the MPE Configuration Message.
  mcm,

  /// RPN 0, Pitch Bend Sensitivity.
  pitchBendSensitivity,

  /// Pitch Bend.
  pitchBend,

  /// Channel Pressure.
  channelPressure,

  /// Control Change 74.
  brightness,

  /// Polyphonic Key Pressure.
  polyPressure,

  /// Control Change 120.
  allSoundOff,

  /// Control Change 121.
  resetAllControllers,

  /// Control Change 122.
  localControl,

  /// Control Change 123.
  allNotesOff,

  /// Control Change 124.
  omniOff,

  /// Control Change 125.
  omniOn,

  /// Control Change 126.
  monoModeOn,

  /// Control Change 127.
  polyModeOn,

  /// Each other Control Change.
  otherControlChange,

  /// Each other RPN.
  otherRpn,

  /// Each NRPN.
  nrpn,

  /// Program Change.
  programChange,

  /// Bank Select.
  bankSelect,

  /// Note On and Note Off.
  note,

  /// System messages.
  system,
}

/// The class of a message kind in Appendix E (the order of ZmidiMpeClass).
enum MpeClass {
  /// M: Mandatory.
  mandatory,

  /// O: Optional.
  optional,

  /// P: Prohibited.
  prohibited,
}

/// The Appendix E class of [message] on the Manager Channel ([manager]) or a Member Channel, to
/// send ([transmit]) or to receive, with Mode 4 ([mono]) for Program Change and Bank Select.
MpeClass mpeClass(
  MpeMessageKind message,
  bool manager,
  bool transmit, [
  bool mono = false,
]) => MpeClass.values[be.mpeClass(message.index, manager, transmit, mono)];

/// The three messages of the MPE Configuration Message of [zone] with [members] (0 to 15).
List<Midi1Message> mcmMessages(MpeZone zone, int members) =>
    _messages(be.mcmMessages(zone.index, checkInt('members', members, 255)));

/// The three messages of RPN 0 with [semitones] (0 to 96) and no Data Entry LSB on [channel].
List<Midi1Message> sensitivityMessages(int channel, int semitones) => _messages(
  be.sensitivityMessages(
    checkInt('channel', channel, 15),
    checkInt('semitones', semitones, 255),
  ),
);

/// RPN 0 with [manager] semitones on the Manager Channel and [member] semitones on each Member
/// Channel of [zone].
List<Midi1Message> zoneSensitivityMessages(
  MpeLayout layout,
  MpeZone zone,
  int manager,
  int member,
) => _messages(
  be.zoneSensitivityMessages(
    layout.lower,
    layout.upper,
    zone.index,
    checkInt('manager', manager, 255),
    checkInt('member', member, 255),
  ),
);

/// The Mode message of [zone] on its lowest Member Channel: Mono Mode On with M = the Member
/// Channel count ([mono]), or Poly Mode On.
Midi1Message modeMessage(MpeLayout layout, MpeZone zone, bool mono) =>
    Midi1Message._raw(
      be.modeMessage(layout.lower, layout.upper, zone.index, mono),
    );

/// The four messages that start a note on [channel]: Pitch Bend, Control Change 74, Channel
/// Pressure and Note On (velocity 1 to 127).
List<Midi1Message> noteMessages(
  int channel,
  int note,
  int velocity, [
  int pitchBend = 8192,
  int brightness = 64,
  int pressure = 0,
]) => _messages(
  be.noteMessages(
    checkInt('channel', channel, 15),
    checkInt('note', note, 255),
    checkInt('velocity', velocity, 255),
    checkInt('pitchBend', pitchBend, 0xFFFF),
    checkInt('brightness', brightness, 255),
    checkInt('pressure', pressure, 255),
  ),
);

/// The MPE receiver of one MIDI 1.0 input (M1-100): the layout, the MIDI Mode and the Pitch Bend
/// Sensitivities of each Zone, and the last Pitch Bend, Channel Pressure and Control Change 74 of
/// each channel. Give each message with the event of the [ParameterState] of its channel.
final class MpeReceiver {
  /// A receiver with no Zone (MPE Mode off).
  MpeReceiver() : _b = be.MpeCore();

  final be.MpeCore _b;
  bool _freed = false;

  /// Takes one Channel Voice message and the event that the [ParameterState] of its channel gave
  /// for it (null for none); gives the MPE event.
  MpeEvent receive(Midi1Message message, [ParameterEvent? parameter]) {
    checkFreed(_freed, this);
    final p = parameter;
    final (kind, zone, manager, reset) = _b.receive(
      message._raw,
      p == null
          ? (0, false, 0, 0, 0)
          : (
              p.change.index + 1,
              p.parameter.registered,
              p.parameter.msb,
              p.parameter.lsb,
              p.value ?? 0,
            ),
    );
    final k = MpeEventKind.values[kind];
    final zoned =
        k == MpeEventKind.zone ||
        k == MpeEventKind.sensitivity ||
        k == MpeEventKind.mode;
    return MpeEvent._(k, zoned ? MpeZone.values[zone] : null, manager, reset);
  }

  /// Sets the layout of the program (power-up or front panel); gives the channels to reset (bit n
  /// is channel n).
  int setLayout(MpeLayout layout) {
    checkFreed(_freed, this);
    return _b.setLayout(layout.lower, layout.upper);
  }

  be.MpeInfo get _info {
    checkFreed(_freed, this);
    return _b.info();
  }

  /// The layout.
  MpeLayout get layout => MpeLayout._(_info.$1, _info.$2);

  /// The MIDI Mode of [zone]: 3 or 4.
  int mode(MpeZone zone) => _info.$3[zone.index];

  /// The Manager Channel ([manager]) or Member Channel Pitch Bend Sensitivity of [zone].
  MpeSensitivity sensitivity(MpeZone zone, bool manager) {
    final (s, c) = (manager ? _info.$4 : _info.$5)[zone.index];
    return MpeSensitivity(s, c);
  }

  (int, int, int, int?) _channel(int channel) {
    checkFreed(_freed, this);
    return _b.channel(checkInt('channel', channel, 15));
  }

  /// The last Pitch Bend of [channel], 0 to 16383.
  int pitchBend(int channel) => _channel(channel).$1;

  /// The last Channel Pressure of [channel].
  int pressure(int channel) => _channel(channel).$2;

  /// The last Control Change 74 of [channel].
  int brightness(int channel) => _channel(channel).$3;

  /// The pitch offset of [channel] in hundredths of a cent (the Manager plus the Member Pitch
  /// Bend), or null for a channel in no Zone.
  int? pitchOffset(int channel) => _channel(channel).$4;

  /// Frees the memory of the receiver now.
  void free() {
    if (!_freed) {
      _freed = true;
      _b.free();
    }
  }
}

/// The Member Channel allocator of an MPE sender (M1-100 2.2.4.1, A.3): the fewest active notes,
/// then the oldest last Note Off; with [sameNote], first a free channel that last played the same
/// Note Number.
final class MpeNoteAllocator {
  /// An allocator with no active note.
  MpeNoteAllocator({bool sameNote = false}) : _b = be.AllocatorCore() {
    _b.setSameNote(sameNote);
  }

  final be.AllocatorCore _b;
  bool _freed = false;

  /// The Member Channel of [zone] for a new [note], or null for an inactive Zone.
  int? noteOn(MpeLayout layout, MpeZone zone, int note) {
    checkFreed(_freed, this);
    return _b.noteOn(
      layout.lower,
      layout.upper,
      zone.index,
      checkInt('note', note, 255),
    );
  }

  /// One note on [channel] ended.
  void noteOff(int channel) {
    checkFreed(_freed, this);
    _b.noteOff(checkInt('channel', channel, 15));
  }

  /// The count of active notes on [channel].
  int activeNotes(int channel) {
    checkFreed(_freed, this);
    return _b.activeNotes(checkInt('channel', channel, 15));
  }

  /// The same-note option.
  bool get sameNote {
    checkFreed(_freed, this);
    return _b.sameNote();
  }

  set sameNote(bool on) {
    checkFreed(_freed, this);
    _b.setSameNote(on);
  }

  /// No active note and no age; the same-note option stays.
  void reset() {
    checkFreed(_freed, this);
    _b.reset();
  }

  /// Frees the memory of the allocator now.
  void free() {
    if (!_freed) {
      _freed = true;
      _b.free();
    }
  }
}

// --- General MIDI (RP-024) ---

/// The General MIDI level (the order of ZmidiGmLevel).
enum GmLevel {
  /// GM System Off.
  off,

  /// General MIDI Level 1.
  gm1,

  /// General MIDI 2.
  gm2,
}

/// The kind of a channel of a GM receiver.
enum GmChannelKind {
  /// A Melody Channel.
  melody,

  /// A Rhythm Channel.
  rhythm,
}

/// The default Rhythm Channel set of a GM receiver: channels 10 and 11 (bits 9 and 10).
const gmRhythmCapableDefault = 0x0600;

/// The General MIDI state of the 16 channels of one input (RP-024 2.4, 2.5, 3.5): the kind of each
/// channel (melody or rhythm), Mode 3 or 4, and the GM level.
final class GmReceiver {
  /// A receiver at the GM2 level. Bit n of [rhythmCapable] is set when the channel with packet
  /// value n can be a Rhythm Channel; with [rhythmMono], Mono On with M = 1 sets Mode 4 on a
  /// Rhythm Channel too.
  GmReceiver({
    int rhythmCapable = gmRhythmCapableDefault,
    bool rhythmMono = false,
  }) : _b = be.GmCore(
         checkInt('rhythmCapable', rhythmCapable, 0xFFFF),
         rhythmMono,
       );

  final be.GmCore _b;
  bool _freed = false;

  /// The GM reset of a GM1 or GM2 System On, or GM System Off ([GmLevel.off]).
  void reset(GmLevel level) {
    checkFreed(_freed, this);
    _b.reset(level.index);
  }

  /// GM System Off: the level off; no other state changes.
  void systemOff() => reset(GmLevel.off);

  /// The program selection of [channel] with the bank that its controller state holds (null: no
  /// bank).
  void program(int channel, int? bank) {
    checkFreed(_freed, this);
    _b.program(
      checkInt('channel', channel, 15),
      bank == null ? null : checkInt('bank', bank, 0xFFFF),
    );
  }

  /// The action of one Channel Voice message by the GM2 Channel Mode rules.
  ModeAction receive(Midi1Message message) {
    checkFreed(_freed, this);
    return _modeAction(_b.receive(message._raw));
  }

  (int, int, int) get _info {
    checkFreed(_freed, this);
    return _b.info();
  }

  /// The kind of [channel] (0 to 15).
  GmChannelKind kind(int channel) =>
      (_info.$2 >> checkInt('channel', channel, 15)) & 1 != 0
      ? GmChannelKind.rhythm
      : GmChannelKind.melody;

  /// True when [channel] is in Mode 4 (Omni Off, Mono), false in Mode 3.
  bool isMono(int channel) =>
      (_info.$3 >> checkInt('channel', channel, 15)) & 1 != 0;

  /// The Rhythm Channels: bit n is the channel with packet value n.
  int get rhythmChannels => _info.$2;

  /// The GM level.
  GmLevel get level => GmLevel.values[_info.$1];

  /// Frees the memory of the receiver now.
  void free() {
    if (!_freed) {
      _freed = true;
      _b.free();
    }
  }
}

/// The GM2 default of [controller] (0 to 127), or null (RP-024 3.3).
int? gm2ControllerDefault(int controller) =>
    be.gm2ControllerDefault(checkInt('controller', controller, 255));

/// True when GM2 recommends that a Rhythm Channel does not respond to [controller] (RP-024 3.3).
bool rhythmIgnoresController(int controller) =>
    be.rhythmIgnoresController(checkInt('controller', controller, 255));

/// True when GM2 recommends that a Rhythm Channel does not respond to [parameter] (RP-024 3.4).
bool rhythmIgnoresParameter(Parameter parameter) => be.rhythmIgnoresParameter(
  parameter.registered,
  parameter.msb,
  parameter.lsb,
);

/// True when GM2 recommends that a Rhythm Channel does not respond to [message] (RP-024 3.3, 3.6,
/// 3.7).
bool rhythmIgnoresMessage(Midi1Message message) =>
    be.rhythmIgnoresMessage(message._raw);

/// The Modulation Depth Range of a 14-bit RPN 00H 05H [value] in hundredths of a cent (RP-024
/// 3.4.4).
int modulationDepthRange(int value) =>
    be.modulationDepthRange(checkInt('value', value, 0xFFFF));
