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
