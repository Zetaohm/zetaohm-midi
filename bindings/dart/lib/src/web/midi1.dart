/// The web form of the MIDI 1.0 parts: the TypeScript package through dart:js_interop. The values
/// are checked by `midi1.dart`.
library;

import 'dart:js_interop';
import 'dart:js_interop_unsafe';
import 'dart:typed_data';

import 'runtime.dart';

/// A message: the ZmidiMidi1Kind value, the status byte, the two data bytes, the System Exclusive
/// bytes and the length.
typedef RawMessage = (
  int kind,
  int status,
  int data1,
  int data2,
  Uint8List data,
  int length,
);

/// The mode information: mode, requested mode, alternate, Basic Channel, M, Local Control.
typedef ModeInfo = (int, int, bool, int, int, bool);

/// The TypeScript kinds, at the index of their C value.
const _kinds = ['', 'channelVoice', 'system', 'sysex', 'sysexTooLong'];
const _modeKinds = [
  'ignore',
  'voice',
  'global',
  'allNotesOff',
  'allSoundOff',
  'resetAllControllers',
  'localControl',
  'modeChange',
];
const _controllerKinds = [
  '',
  'controller14',
  'controller7',
  'program',
  'portamento',
  'resetAllControllers',
];
const _changes = ['', 'dataEntryMsb', 'dataEntryLsb', 'increment', 'decrement'];

int _index(List<String> names, JSAny? a) {
  final i = names.indexOf(stringOrNull(a) ?? '');
  return i < 0 ? 0 : i;
}

JSObject _messageIn(RawMessage m) => jsObject({
  'kind': _kinds[m.$1 < _kinds.length ? m.$1 : 0].toJS,
  'status': m.$2.toJS,
  'data1': m.$3.toJS,
  'data2': m.$4.toJS,
  'data': jsBytes(m.$5),
  'length': m.$6.toJS,
});

RawMessage _messageOut(JSAny? a) {
  final o = a! as JSObject;
  return (
    _index(_kinds, prop(o, 'kind')),
    intOf(prop(o, 'status')),
    intOf(prop(o, 'data1')),
    intOf(prop(o, 'data2')),
    bytes(prop(o, 'data')),
    intOf(prop(o, 'length')),
  );
}

List<RawMessage> _messagesOut(JSAny? a) => [
  for (final m in (a! as JSArray<JSObject>).toDart) _messageOut(m),
];

JSObject _parameter(bool registered, int msb, int lsb) =>
    jsObject({'msb': msb.toJS, 'lsb': lsb.toJS, 'registered': registered.toJS});

/// The TypeScript Midi1Reader.
final class Midi1ReaderState {
  /// A reader with a System Exclusive buffer of [sysexCapacity] bytes.
  Midi1ReaderState(int sysexCapacity)
    : _o = tsNew('Midi1Reader', [sysexCapacity.toJS]);

  final JSObject _o;

  /// Reads one byte.
  List<RawMessage> push(int byte) =>
      _messagesOut(call(_o, 'push', [byte.toJS]));

  /// The System Reset action of the program.
  void reset() => call<JSAny?>(_o, 'reset');

  /// Frees the WASM memory.
  void free() => call<JSAny?>(_o, 'free');
}

/// The TypeScript Midi1Writer.
final class Midi1WriterState {
  /// A writer with or without running status.
  Midi1WriterState(bool runningStatus)
    : _o = tsNew('Midi1Writer', [runningStatus.toJS]);

  final JSObject _o;

  /// Writes one message.
  Uint8List write(RawMessage m) => bytes(call(_o, 'write', [_messageIn(m)]));

  /// Sends the next status byte.
  void refreshStatus() => call<JSAny?>(_o, 'refreshStatus');

  /// Frees the WASM memory.
  void free() => call<JSAny?>(_o, 'free');
}

/// The TypeScript ModeState.
final class ModeCore {
  /// A mode state from its configuration.
  ModeCore(
    int basicChannel,
    int implemented,
    bool alternate,
    int voices,
    bool globalControllers,
  ) : _o = tsNew('ModeState', [
        jsObject({
          'basicChannel': basicChannel.toJS,
          'implemented': jsInts([
            for (var n = 1; n <= 4; n++)
              if (implemented >> (n - 1) & 1 != 0) n,
          ]),
          'alternate': alternate.toJS,
          'voices': voices.toJS,
          'globalControllers': globalControllers.toJS,
        }),
      ]);

  final JSObject _o;

  /// Takes one message; gives (kind, channel, voice, on, changed, alternate).
  (int, int, int, bool, bool, bool) receive(RawMessage m) {
    final a = call<JSObject>(_o, 'receive', [_messageIn(m)]);
    return (
      _index(_modeKinds, prop(a, 'kind')),
      intOrNull(prop(a, 'channel')) ?? 0xFF,
      intOrNull(prop(a, 'voice')) ?? 0xFF,
      boolOf(prop(a, 'on')),
      boolOf(prop(a, 'changed')),
      boolOf(prop(a, 'alternate')),
    );
  }

  /// The System Reset action.
  void systemReset() => call<JSAny?>(_o, 'systemReset');

  /// The mode information.
  ModeInfo info() => (
    intOf(prop(_o, 'mode')),
    intOf(prop(_o, 'requestedMode')),
    boolOf(prop(_o, 'isAlternate')),
    intOf(prop(_o, 'basicChannel')),
    intOf(prop(_o, 'm')),
    boolOf(prop(_o, 'local')),
  );

  /// Sets the Basic Channel.
  void setBasicChannel(int channel) =>
      guard(() => _o.setProperty('basicChannel'.toJS, channel.toJS));

  /// Sets Local Control.
  void setLocal(bool on) => guard(() => _o.setProperty('local'.toJS, on.toJS));

  /// Sets the Omni flag.
  void setOmni(bool on) => call<JSAny?>(_o, 'setOmni', [on.toJS]);

  /// Selects Poly.
  void setPoly() => call<JSAny?>(_o, 'setPoly');

  /// Selects Mono with M.
  void setMono(int m) => call<JSAny?>(_o, 'setMono', [m.toJS]);

  /// Frees the WASM memory.
  void free() => call<JSAny?>(_o, 'free');
}

/// The TypeScript ControllerState.
final class ControllerCore {
  /// A new controller state.
  ControllerCore() : _o = tsNew('ControllerState', []);

  final JSObject _o;

  /// Takes one message; gives (kind, controller, value, program, has bank, bank, source).
  (int, int, int, int, bool, int, int) receive(RawMessage m) {
    final e = call<JSObject?>(_o, 'receive', [_messageIn(m)]);
    if (e == null) {
      return (0, 0, 0, 0, false, 0, 0);
    }
    final bank = intOrNull(prop(e, 'bank'));
    return (
      _index(_controllerKinds, prop(e, 'kind')),
      intOrNull(prop(e, 'controller')) ?? 0,
      intOrNull(prop(e, 'value')) ?? 0,
      intOrNull(prop(e, 'program')) ?? 0,
      bank != null,
      bank ?? 0,
      intOrNull(prop(e, 'source')) ?? 0,
    );
  }

  /// The Reset All Controllers action.
  void reset() => call<JSAny?>(_o, 'resetAllControllers');

  /// The held bank, or null.
  int? bank() => intOrNull(prop(_o, 'bank'));

  /// Frees the WASM memory.
  void free() => call<JSAny?>(_o, 'free');
}

/// The RP-015 value of [controller], or null to keep it.
int? resetValue(int controller) =>
    intOrNull(tsCall('resetValue', [controller.toJS]));

/// The two messages of a 14-bit controller value.
List<RawMessage> controller14Messages(int channel, int controller, int value) =>
    _messagesOut(
      tsCall('controller14Messages', [
        channel.toJS,
        controller.toJS,
        value.toJS,
      ]),
    );

/// The three messages of a bank and program selection.
List<RawMessage> bankProgramMessages(int channel, int bank, int program) =>
    _messagesOut(
      tsCall('bankProgramMessages', [channel.toJS, bank.toJS, program.toJS]),
    );

/// The TypeScript ParameterState.
final class ParameterCore {
  /// A new parameter receiver.
  ParameterCore() : _o = tsNew('ParameterState', []);

  final JSObject _o;

  /// Takes one message; gives (change, registered, MSB, LSB, value).
  (int, bool, int, int, int) receive(RawMessage m) {
    final e = call<JSObject?>(_o, 'receive', [_messageIn(m)]);
    if (e == null) {
      return (0, false, 0, 0, 0);
    }
    final p = prop<JSObject>(e, 'parameter');
    return (
      _index(_changes, prop(e, 'change')),
      boolOf(prop(p, 'registered')),
      intOf(prop(p, 'msb')),
      intOf(prop(p, 'lsb')),
      intOrNull(prop(e, 'value')) ?? 0,
    );
  }

  /// The Reset All Controllers action.
  void reset() => call<JSAny?>(_o, 'resetAllControllers');

  /// The selected parameter (registered, MSB, LSB), or null.
  (bool, int, int)? selection() {
    final p = prop<JSObject?>(_o, 'selection');
    if (p == null) {
      return null;
    }
    return (
      boolOf(prop(p, 'registered')),
      intOf(prop(p, 'msb')),
      intOf(prop(p, 'lsb')),
    );
  }

  /// RPN and NRPN reception.
  (bool, bool) reception() =>
      (boolOf(prop(_o, 'rpnReception')), boolOf(prop(_o, 'nrpnReception')));

  /// Turns RPN and NRPN reception on or off.
  void setReception(bool rpn, bool nrpn) => guard(() {
    _o.setProperty('rpnReception'.toJS, rpn.toJS);
    _o.setProperty('nrpnReception'.toJS, nrpn.toJS);
  });

  /// Frees the WASM memory.
  void free() => call<JSAny?>(_o, 'free');
}

/// The new value of a parameter from an event and the current value (RP-018 steps).
int applyParameter(
  int change,
  bool registered,
  int msb,
  int lsb,
  int value,
  int current,
) => intOf(
  tsCall('applyParameter', [
    jsObject({
      'parameter': _parameter(registered, msb, lsb),
      'change': _changes[change < _changes.length ? change : 0].toJS,
      'value': value.toJS,
    }),
    current.toJS,
  ]),
);

/// The two messages of a parameter number.
List<RawMessage> selectMessages(
  int channel,
  bool registered,
  int msb,
  int lsb,
) => _messagesOut(
  tsCall('selectMessages', [channel.toJS, _parameter(registered, msb, lsb)]),
);

/// The message of one change.
RawMessage changeMessage(int channel, int change, int value) => _messageOut(
  tsCall('changeMessage', [
    channel.toJS,
    _changes[change < _changes.length ? change : 0].toJS,
    value.toJS,
  ]),
);

/// The four messages of a parameter value.
List<RawMessage> valueMessages(
  int channel,
  bool registered,
  int msb,
  int lsb,
  int value,
) => _messagesOut(
  tsCall('valueMessages', [
    channel.toJS,
    _parameter(registered, msb, lsb),
    value.toJS,
  ]),
);

const _syncKinds = ['', 'start', 'continue', 'stop', 'clock', 'locate', 'song'];
const _syncRuns = ['stopped', 'waiting', 'playing'];

/// The TypeScript SyncReceiver.
final class SyncCore {
  /// A new sync receiver.
  SyncCore() : _o = tsNew('SyncReceiver', []);

  final JSObject _o;

  (int, int, int) _out(JSObject? e) => e == null
      ? (0, 0, 0)
      : (
          _index(_syncKinds, prop(e, 'kind')),
          intOrNull(prop(e, 'position')) ?? 0,
          intOrNull(prop(e, 'song')) ?? 0,
        );

  /// Takes one message; gives (kind, position, song).
  (int, int, int) receive(RawMessage m) =>
      _out(call<JSObject?>(_o, 'receive', [_messageIn(m)]));

  /// Takes one command (1 start, 2 continue, 3 stop); gives (kind, position, song).
  (int, int, int) command(int command) => _out(
    call<JSObject?>(_o, 'command', [
      _syncKinds[command < 4 ? command : 0].toJS,
    ]),
  );

  /// The System Reset action.
  void systemReset() => call<JSAny?>(_o, 'systemReset');

  /// (position, run, song, sync mode).
  (int, int, int, bool) info() => (
    intOf(prop(_o, 'position')),
    _index(_syncRuns, prop(_o, 'run')),
    intOf(prop(_o, 'song')),
    boolOf(prop(_o, 'syncMode')),
  );

  /// Sets MIDI sync mode.
  void setMode(bool on) =>
      guard(() => _o.setProperty('syncMode'.toJS, on.toJS));

  /// Frees the WASM memory.
  void free() => call<JSAny?>(_o, 'free');
}

/// The TypeScript ActiveSensingMonitor.
final class MonitorCore {
  /// A monitor with a timeout in microseconds.
  MonitorCore(int timeoutUs)
    : _o = tsNew('ActiveSensingMonitor', [timeoutUs.toJS]);

  final JSObject _o;

  /// Takes one received byte.
  void receiveByte(int byte, int nowUs) =>
      call<JSAny?>(_o, 'receiveByte', [byte.toJS, nowUs.toJS]);

  /// Takes one received message.
  void receive(bool activeSensing, int nowUs) =>
      call<JSAny?>(_o, 'receive', [activeSensing.toJS, nowUs.toJS]);

  /// Checks the timer; gives the timeout.
  bool poll(int nowUs) => boolOf(call(_o, 'poll', [nowUs.toJS]));

  /// The armed flag.
  bool isArmed() => boolOf(prop(_o, 'isArmed'));

  /// Frees the WASM memory.
  void free() => call<JSAny?>(_o, 'free');
}

/// The TypeScript ActiveSensingSender.
final class SenderCore {
  /// A sender with an interval in microseconds.
  SenderCore(int intervalUs)
    : _o = tsNew('ActiveSensingSender', [intervalUs.toJS]);

  final JSObject _o;

  /// The program sent a byte.
  void sent(int nowUs) => call<JSAny?>(_o, 'sent', [nowUs.toJS]);

  /// Checks the interval; gives true to send FEH.
  bool poll(int nowUs) => boolOf(call(_o, 'poll', [nowUs.toJS]));

  /// Frees the WASM memory.
  void free() => call<JSAny?>(_o, 'free');
}

/// A parameter event for the MPE receiver: (change, registered, MSB, LSB, value); change 0 is
/// none.
typedef RawParameterEvent = (int, bool, int, int, int);

/// The MPE information: lower, upper, the mode of each Zone, the Manager and the Member
/// sensitivity (semitones, cents) of each Zone.
typedef MpeInfo = (int, int, List<int>, List<(int, int)>, List<(int, int)>);

const _zones = ['lower', 'upper'];
const _roles = ['noZone', 'manager', 'member'];
const _mpeKinds = [
  'noZone',
  'zone',
  'layout',
  'sensitivity',
  'mode',
  'ignored',
];
const _classes = ['mandatory', 'optional', 'prohibited'];
const _mpeMessages = [
  'mcm',
  'pitchBendSensitivity',
  'pitchBend',
  'channelPressure',
  'brightness',
  'polyPressure',
  'allSoundOff',
  'resetAllControllers',
  'localControl',
  'allNotesOff',
  'omniOff',
  'omniOn',
  'monoModeOn',
  'polyModeOn',
  'otherControlChange',
  'otherRpn',
  'nrpn',
  'programChange',
  'bankSelect',
  'note',
  'system',
];

JSString _zone(int zone) => _zones[zone < 2 ? zone : 0].toJS;

JSObject _layout(int lower, int upper) =>
    jsObject({'lower': lower.toJS, 'upper': upper.toJS});

(int, int) _layoutOut(JSAny? a) {
  final o = a! as JSObject;
  return (intOf(prop(o, 'lower')), intOf(prop(o, 'upper')));
}

/// A layout after the check: (lower, upper).
(int, int) mpeLayoutNew(int lower, int upper) =>
    _layoutOut(tsCall('mpeLayout', [lower.toJS, upper.toJS]));

/// The layout after one MCM: (lower, upper).
(int, int) mpeLayoutWithMcm(int lower, int upper, int zone, int members) =>
    _layoutOut(
      tsCall('mpeLayoutWithMcm', [
        _layout(lower, upper),
        _zone(zone),
        members.toJS,
      ]),
    );

/// The role (0 no Zone, 1 Manager, 2 Member) and the Zone of a channel.
(int, int) mpeRole(int lower, int upper, int channel) {
  final o = tsCall<JSObject>('mpeRole', [_layout(lower, upper), channel.toJS]);
  return (_index(_roles, prop(o, 'role')), _index(_zones, prop(o, 'zone')));
}

/// The lowest Member Channel of a Zone, or null.
int? mpeLowestMember(int lower, int upper, int zone) =>
    intOrNull(tsCall('mpeLowestMember', [_layout(lower, upper), _zone(zone)]));

/// The C.3 Pitch Bend value.
int pitchBendValue(int offset, int semitones, int cents) =>
    intOf(tsCall('pitchBendValue', [offset.toJS, semitones.toJS, cents.toJS]));

/// The C.5 pitch offset.
int pitchOffset(int value, int semitones, int cents) =>
    intOf(tsCall('pitchOffset', [value.toJS, semitones.toJS, cents.toJS]));

/// The Appendix E class (0 Mandatory, 1 Optional, 2 Prohibited).
int mpeClass(int message, bool manager, bool transmit, bool mono) => _index(
  _classes,
  tsCall('mpeClass', [
    _mpeMessages[message < _mpeMessages.length ? message : 0].toJS,
    manager.toJS,
    transmit.toJS,
    mono.toJS,
  ]),
);

/// The three messages of an MCM.
List<RawMessage> mcmMessages(int zone, int members) =>
    _messagesOut(tsCall('mcmMessages', [_zone(zone), members.toJS]));

/// The three messages of RPN 0 on one channel.
List<RawMessage> sensitivityMessages(int channel, int semitones) =>
    _messagesOut(tsCall('sensitivityMessages', [channel.toJS, semitones.toJS]));

/// RPN 0 on the Manager Channel and on each Member Channel of a Zone.
List<RawMessage> zoneSensitivityMessages(
  int lower,
  int upper,
  int zone,
  int manager,
  int member,
) => _messagesOut(
  tsCall('zoneSensitivityMessages', [
    _layout(lower, upper),
    _zone(zone),
    manager.toJS,
    member.toJS,
  ]),
);

/// The Mode message of a Zone.
RawMessage modeMessage(int lower, int upper, int zone, bool mono) =>
    _messageOut(
      tsCall('modeMessage', [_layout(lower, upper), _zone(zone), mono.toJS]),
    );

/// The four messages that start a note.
List<RawMessage> noteMessages(
  int channel,
  int note,
  int velocity,
  int pitchBend,
  int brightness,
  int pressure,
) => _messagesOut(
  tsCall('noteMessages', [
    channel.toJS,
    note.toJS,
    velocity.toJS,
    pitchBend.toJS,
    brightness.toJS,
    pressure.toJS,
  ]),
);

/// The TypeScript MpeReceiver.
final class MpeCore {
  /// A receiver with no Zone.
  MpeCore() : _o = tsNew('MpeReceiver', []);

  final JSObject _o;

  /// Takes one message and its parameter event; gives (kind, zone, manager, reset).
  (int, int, bool, int) receive(RawMessage m, RawParameterEvent p) {
    final event = p.$1 == 0
        ? null
        : jsObject({
            'parameter': _parameter(p.$2, p.$3, p.$4),
            'change': _changes[p.$1 < _changes.length ? p.$1 : 0].toJS,
            'value': p.$5.toJS,
          });
    final e = call<JSObject>(_o, 'receive', [_messageIn(m), event]);
    return (
      _index(_mpeKinds, prop(e, 'kind')),
      _index(_zones, prop(e, 'zone')),
      boolOf(prop(e, 'manager')),
      intOf(prop(e, 'reset')),
    );
  }

  /// Sets the layout of the program; gives the reset mask.
  int setLayout(int lower, int upper) =>
      intOf(call(_o, 'setLayout', [_layout(lower, upper)]));

  /// The layout, the modes and the sensitivities.
  MpeInfo info() {
    final (lower, upper) = _layoutOut(prop(_o, 'layout'));
    (int, int) sens(String zone, bool manager) {
      final s = call<JSObject>(_o, 'sensitivity', [zone.toJS, manager.toJS]);
      return (intOf(prop(s, 'semitones')), intOf(prop(s, 'cents')));
    }

    return (
      lower,
      upper,
      [
        for (final z in _zones) intOf(call(_o, 'mode', [z.toJS])),
      ],
      [for (final z in _zones) sens(z, true)],
      [for (final z in _zones) sens(z, false)],
    );
  }

  /// The values of one channel: (Pitch Bend, Channel Pressure, Control Change 74, pitch offset).
  (int, int, int, int?) channel(int channel) => (
    intOf(call(_o, 'pitchBend', [channel.toJS])),
    intOf(call(_o, 'pressure', [channel.toJS])),
    intOf(call(_o, 'brightness', [channel.toJS])),
    intOrNull(call(_o, 'pitchOffset', [channel.toJS])),
  );

  /// Frees the WASM memory.
  void free() => call<JSAny?>(_o, 'free');
}

/// The TypeScript MpeNoteAllocator.
final class AllocatorCore {
  /// An allocator with no active note.
  AllocatorCore() : _o = tsNew('MpeNoteAllocator', []);

  final JSObject _o;

  /// The Member Channel for a new note, or null for an inactive Zone.
  int? noteOn(int lower, int upper, int zone, int note) => intOrNull(
    call(_o, 'noteOn', [_layout(lower, upper), _zone(zone), note.toJS]),
  );

  /// One note on a channel ended.
  void noteOff(int channel) => call<JSAny?>(_o, 'noteOff', [channel.toJS]);

  /// The count of active notes on a channel.
  int activeNotes(int channel) =>
      intOf(call(_o, 'activeNotes', [channel.toJS]));

  /// The same-note option.
  bool sameNote() => boolOf(prop(_o, 'sameNote'));

  /// Sets the same-note option.
  void setSameNote(bool on) =>
      guard(() => _o.setProperty('sameNote'.toJS, on.toJS));

  /// No active note and no age.
  void reset() => call<JSAny?>(_o, 'reset');

  /// Frees the WASM memory.
  void free() => call<JSAny?>(_o, 'free');
}
