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
    final a = _b.receive(message._raw);
    return ModeAction._(
      ModeActionKind.values[a.$1],
      a.$2 == 0xFF ? null : a.$2,
      a.$3 == 0xFF ? null : a.$3,
      a.$4,
      a.$5,
      a.$6,
    );
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
