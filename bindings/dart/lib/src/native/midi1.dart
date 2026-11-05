/// The native form of the MIDI 1.0 parts: zmidi-io through dart:ffi and the C ABI (module Midi1
/// of zmidi-ffi). The values are checked by `midi1.dart`.
library;

import 'dart:ffi';
import 'dart:typed_data';

import 'package:ffi/ffi.dart';

import '../bindings.g.dart';
import '../native.dart';

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

void _messageIn(Pointer<ZmidiMidi1Message> p, RawMessage m) {
  p.ref
    ..kind = m.$1
    ..status = m.$2
    ..data1 = m.$3
    ..data2 = m.$4
    ..length = m.$6;
}

RawMessage _messageOut(Pointer<ZmidiMidi1Message> p, [Pointer<Uint8>? sysex]) {
  final m = p.ref;
  final data =
      m.kind == ZmidiMidi1Kind.ZMIDI_MIDI1_KIND_SYS_EX.value && sysex != null
      ? bytesOut(sysex, m.length)
      : Uint8List(0);
  return (m.kind, m.status, m.data1, m.data2, data, m.length);
}

/// Calls a C writer of at most [max] messages and gives them.
List<RawMessage> _written(
  int max,
  ZmidiStatus Function(Pointer<ZmidiMidi1Message> out, Pointer<UintPtr> count)
  f,
  String what,
) => using((arena) {
  final out = arena<ZmidiMidi1Message>(max);
  final count = arena<UintPtr>();
  check(f(out, count), what);
  return [for (var i = 0; i < count.value; i++) _messageOut(out + i)];
});

/// The C state of a Midi1Reader and its System Exclusive buffer, in one block.
final class Midi1ReaderState implements Finalizable {
  /// A reader with a System Exclusive buffer of [sysexCapacity] bytes.
  Midi1ReaderState(this._capacity) {
    _offset = (zmidi_midi1_reader_size() + 15) ~/ 16 * 16;
    _p = allocState(this, _offset + (_capacity < 1 ? 1 : _capacity));
    check(zmidi_midi1_reader_init(_p.cast()), 'Midi1Reader');
  }

  final int _capacity;
  late final int _offset;
  final _s = Scratch();
  late Pointer<Uint8> _p;

  /// Reads one byte.
  List<RawMessage> push(int byte) => _s.run((arena) {
    final out = arena<ZmidiMidi1Message>(2);
    final count = arena<UintPtr>();
    final sysex = _p + _offset;
    check(
      zmidi_midi1_reader_push(_p.cast(), byte, sysex, _capacity, out, 2, count),
      'push',
    );
    return [for (var i = 0; i < count.value; i++) _messageOut(out + i, sysex)];
  });

  /// The System Reset action of the program.
  void reset() => check(zmidi_midi1_reader_reset(_p.cast()), 'reset');

  /// Frees the C state and the scratch block.
  void free() {
    if (_p != nullptr) {
      releaseState(this, _p);
      _p = nullptr;
    }
    _s.release();
  }
}

/// The C state of a Midi1Writer.
final class Midi1WriterState implements Finalizable {
  /// A writer with or without running status.
  Midi1WriterState(bool runningStatus) {
    _p = allocState(this, zmidi_midi1_writer_size());
    check(zmidi_midi1_writer_init(_p, runningStatus), 'Midi1Writer');
  }

  final _s = Scratch();
  late Pointer<ZmidiMidi1Writer> _p;

  /// Writes one message.
  Uint8List write(RawMessage m) => _s.run((arena) {
    final pm = arena<ZmidiMidi1Message>();
    _messageIn(pm, m);
    final sysex = m.$1 == ZmidiMidi1Kind.ZMIDI_MIDI1_KIND_SYS_EX.value
        ? bytesIn(arena, m.$5)
        : nullptr.cast<Uint8>();
    final cap = m.$6 < 4 ? 4 : m.$6;
    final out = arena<Uint8>(cap);
    final count = arena<UintPtr>();
    check(zmidi_midi1_writer_write(_p, pm, sysex, out, cap, count), 'write');
    return bytesOut(out, count.value);
  });

  /// Sends the next status byte.
  void refreshStatus() =>
      check(zmidi_midi1_writer_refresh_status(_p), 'refreshStatus');

  /// Frees the C state and the scratch block.
  void free() {
    if (_p != nullptr) {
      releaseState(this, _p);
      _p = nullptr;
    }
    _s.release();
  }
}

/// The C state of a ModeState.
final class ModeCore implements Finalizable {
  /// A mode state from its configuration.
  ModeCore(
    int basicChannel,
    int implemented,
    bool alternate,
    int voices,
    bool globalControllers,
  ) {
    _p = allocState(this, zmidi_mode_state_size());
    using((arena) {
      final c = arena<ZmidiModeConfig>();
      c.ref
        ..basic_channel = basicChannel
        ..implemented = implemented
        ..alternate = alternate
        ..voices = voices
        ..global_controllers = globalControllers;
      check(zmidi_mode_init(_p, c), 'ModeState');
    });
  }

  final _s = Scratch();
  late Pointer<ZmidiModeState> _p;

  /// Takes one message; gives (kind, channel, voice, on, changed, alternate).
  (int, int, int, bool, bool, bool) receive(RawMessage m) => _s.run((arena) {
    final pm = arena<ZmidiMidi1Message>();
    _messageIn(pm, m);
    final a = arena<ZmidiModeAction>();
    check(zmidi_mode_receive(_p, pm, a), 'receive');
    final r = a.ref;
    return (r.kind, r.channel, r.voice, r.on, r.changed, r.alternate);
  });

  /// The System Reset action.
  void systemReset() => check(zmidi_mode_system_reset(_p), 'systemReset');

  /// The mode information.
  ModeInfo info() => _s.run((arena) {
    final i = arena<ZmidiModeInfo>();
    check(zmidi_mode_info(_p, i), 'info');
    final r = i.ref;
    return (
      r.mode,
      r.requested_mode,
      r.alternate,
      r.basic_channel,
      r.m,
      r.local,
    );
  });

  /// Sets the Basic Channel.
  void setBasicChannel(int channel) =>
      check(zmidi_mode_set_basic_channel(_p, channel), 'basicChannel');

  /// Sets Local Control.
  void setLocal(bool on) => check(zmidi_mode_set_local(_p, on), 'local');

  /// Sets the Omni flag.
  void setOmni(bool on) => check(zmidi_mode_set_omni(_p, on), 'setOmni');

  /// Selects Poly.
  void setPoly() => check(zmidi_mode_set_poly(_p), 'setPoly');

  /// Selects Mono with M.
  void setMono(int m) => check(zmidi_mode_set_mono(_p, m), 'setMono');

  /// Frees the C state and the scratch block.
  void free() {
    if (_p != nullptr) {
      releaseState(this, _p);
      _p = nullptr;
    }
    _s.release();
  }
}

/// The C state of a ControllerState.
final class ControllerCore implements Finalizable {
  /// A new controller state.
  ControllerCore() {
    _p = allocState(this, zmidi_controller_state_size());
    check(zmidi_controllers_init(_p), 'ControllerState');
  }

  final _s = Scratch();
  late Pointer<ZmidiControllerState> _p;

  /// Takes one message; gives (kind, controller, value, program, has bank, bank, source).
  (int, int, int, int, bool, int, int) receive(RawMessage m) => _s.run((arena) {
    final pm = arena<ZmidiMidi1Message>();
    _messageIn(pm, m);
    final e = arena<ZmidiControllerEvent>();
    check(zmidi_controllers_receive(_p, pm, e), 'receive');
    final r = e.ref;
    return (
      r.kind,
      r.controller,
      r.value,
      r.program,
      r.has_bank,
      r.bank,
      r.source,
    );
  });

  /// The Reset All Controllers action.
  void reset() => check(zmidi_controllers_reset(_p), 'resetAllControllers');

  /// The held bank, or null.
  int? bank() => _s.run((arena) {
    final has = arena<Bool>();
    final bank = arena<Uint16>();
    check(zmidi_controllers_bank(_p, has, bank), 'bank');
    return has.value ? bank.value : null;
  });

  /// Frees the C state and the scratch block.
  void free() {
    if (_p != nullptr) {
      releaseState(this, _p);
      _p = nullptr;
    }
    _s.release();
  }
}

/// The RP-015 value of [controller], or null to keep it.
int? resetValue(int controller) => using((arena) {
  final v = arena<Uint8>();
  check(zmidi_controller_reset_value(controller, v), 'resetValue');
  return v.value == 0xFF ? null : v.value;
});

/// The two messages of a 14-bit controller value.
List<RawMessage> controller14Messages(int channel, int controller, int value) =>
    _written(
      2,
      (out, count) => zmidi_controller14_messages(
        channel,
        controller,
        value,
        out,
        2,
        count,
      ),
      'controller14Messages',
    );

/// The three messages of a bank and program selection.
List<RawMessage> bankProgramMessages(int channel, int bank, int program) =>
    _written(
      3,
      (out, count) =>
          zmidi_bank_program_messages(channel, bank, program, out, 3, count),
      'bankProgramMessages',
    );

/// The C state of a ParameterState.
final class ParameterCore implements Finalizable {
  /// A new parameter receiver.
  ParameterCore() {
    _p = allocState(this, zmidi_parameter_state_size());
    check(zmidi_parameters_init(_p), 'ParameterState');
  }

  final _s = Scratch();
  late Pointer<ZmidiParameterState> _p;

  /// Takes one message; gives (change, registered, MSB, LSB, value).
  (int, bool, int, int, int) receive(RawMessage m) => _s.run((arena) {
    final pm = arena<ZmidiMidi1Message>();
    _messageIn(pm, m);
    final e = arena<ZmidiParameterEvent>();
    check(zmidi_parameters_receive(_p, pm, e), 'receive');
    final r = e.ref;
    return (
      r.change,
      r.parameter.registered,
      r.parameter.msb,
      r.parameter.lsb,
      r.value,
    );
  });

  /// The Reset All Controllers action.
  void reset() => check(zmidi_parameters_reset(_p), 'resetAllControllers');

  /// The selected parameter (registered, MSB, LSB), or null.
  (bool, int, int)? selection() => _s.run((arena) {
    final selected = arena<Bool>();
    final p = arena<ZmidiParameter>();
    check(zmidi_parameters_selection(_p, selected, p), 'selection');
    return selected.value ? (p.ref.registered, p.ref.msb, p.ref.lsb) : null;
  });

  /// RPN and NRPN reception.
  (bool, bool) reception() => _s.run((arena) {
    final rpn = arena<Bool>();
    final nrpn = arena<Bool>();
    check(zmidi_parameters_reception(_p, rpn, nrpn), 'reception');
    return (rpn.value, nrpn.value);
  });

  /// Turns RPN and NRPN reception on or off.
  void setReception(bool rpn, bool nrpn) =>
      check(zmidi_parameters_set_reception(_p, rpn, nrpn), 'reception');

  /// Frees the C state and the scratch block.
  void free() {
    if (_p != nullptr) {
      releaseState(this, _p);
      _p = nullptr;
    }
    _s.release();
  }
}

void _parameterIn(
  Pointer<ZmidiParameter> p,
  bool registered,
  int msb,
  int lsb,
) {
  p.ref
    ..registered = registered
    ..msb = msb
    ..lsb = lsb;
}

/// The new value of a parameter from an event and the current value (RP-018 steps).
int applyParameter(
  int change,
  bool registered,
  int msb,
  int lsb,
  int value,
  int current,
) => using((arena) {
  final e = arena<ZmidiParameterEvent>();
  e.ref
    ..change = change
    ..value = value;
  e.ref.parameter
    ..registered = registered
    ..msb = msb
    ..lsb = lsb;
  final out = arena<Uint16>();
  check(zmidi_parameter_apply(e, current, out), 'apply');
  return out.value;
});

/// The two messages of a parameter number.
List<RawMessage> selectMessages(
  int channel,
  bool registered,
  int msb,
  int lsb,
) => using((arena) {
  final p = arena<ZmidiParameter>();
  _parameterIn(p, registered, msb, lsb);
  return _written(
    2,
    (out, count) => zmidi_parameter_select_messages(channel, p, out, 2, count),
    'selectMessages',
  );
});

/// The message of one change.
RawMessage changeMessage(int channel, int change, int value) => using((arena) {
  final out = arena<ZmidiMidi1Message>();
  check(
    zmidi_parameter_change_message(channel, change, value, out),
    'changeMessage',
  );
  return _messageOut(out);
});

/// The four messages of a parameter value.
List<RawMessage> valueMessages(
  int channel,
  bool registered,
  int msb,
  int lsb,
  int value,
) => using((arena) {
  final p = arena<ZmidiParameter>();
  _parameterIn(p, registered, msb, lsb);
  return _written(
    4,
    (out, count) =>
        zmidi_parameter_value_messages(channel, p, value, out, 4, count),
    'valueMessages',
  );
});

/// The C state of a SyncReceiver.
final class SyncCore implements Finalizable {
  /// A new sync receiver.
  SyncCore() {
    _p = allocState(this, zmidi_sync_receiver_size());
    check(zmidi_sync_init(_p), 'SyncReceiver');
  }

  final _s = Scratch();
  late Pointer<ZmidiSyncReceiver> _p;

  (int, int, int) _out(Pointer<ZmidiSyncEvent> e) =>
      (e.ref.kind, e.ref.position, e.ref.song);

  /// Takes one message; gives (kind, position, song).
  (int, int, int) receive(RawMessage m) => _s.run((arena) {
    final pm = arena<ZmidiMidi1Message>();
    _messageIn(pm, m);
    final e = arena<ZmidiSyncEvent>();
    check(zmidi_sync_receive(_p, pm, e), 'receive');
    return _out(e);
  });

  /// Takes one command (1 start, 2 continue, 3 stop); gives (kind, position, song).
  (int, int, int) command(int command) => _s.run((arena) {
    final e = arena<ZmidiSyncEvent>();
    check(zmidi_sync_command(_p, command, e), 'command');
    return _out(e);
  });

  /// The System Reset action.
  void systemReset() => check(zmidi_sync_system_reset(_p), 'systemReset');

  /// (position, run, song, sync mode).
  (int, int, int, bool) info() => _s.run((arena) {
    final i = arena<ZmidiSyncInfo>();
    check(zmidi_sync_info(_p, i), 'info');
    return (i.ref.position, i.ref.run, i.ref.song, i.ref.sync_mode);
  });

  /// Sets MIDI sync mode.
  void setMode(bool on) => check(zmidi_sync_set_mode(_p, on), 'syncMode');

  /// Frees the C state and the scratch block.
  void free() {
    if (_p != nullptr) {
      releaseState(this, _p);
      _p = nullptr;
    }
    _s.release();
  }
}

/// The C state of an ActiveSensingMonitor.
final class MonitorCore implements Finalizable {
  /// A monitor with a timeout in microseconds.
  MonitorCore(int timeoutUs) {
    _p = allocState(this, zmidi_active_sensing_monitor_size());
    check(
      zmidi_active_sensing_monitor_init(_p, timeoutUs),
      'ActiveSensingMonitor',
    );
  }

  final _s = Scratch();
  late Pointer<ZmidiActiveSensingMonitor> _p;

  /// Takes one received byte.
  void receiveByte(int byte, int nowUs) => check(
    zmidi_active_sensing_monitor_receive_byte(_p, byte, nowUs),
    'receiveByte',
  );

  /// Takes one received message.
  void receive(bool activeSensing, int nowUs) => check(
    zmidi_active_sensing_monitor_receive(_p, activeSensing, nowUs),
    'receive',
  );

  /// Checks the timer; gives the timeout.
  bool poll(int nowUs) => _s.run((arena) {
    final timeout = arena<Bool>();
    final armed = arena<Bool>();
    check(zmidi_active_sensing_monitor_poll(_p, nowUs, timeout, armed), 'poll');
    return timeout.value;
  });

  /// The armed flag.
  bool isArmed() => _s.run((arena) {
    final armed = arena<Bool>();
    check(zmidi_active_sensing_monitor_is_armed(_p, armed), 'isArmed');
    return armed.value;
  });

  /// Frees the C state and the scratch block.
  void free() {
    if (_p != nullptr) {
      releaseState(this, _p);
      _p = nullptr;
    }
    _s.release();
  }
}

/// The C state of an ActiveSensingSender.
final class SenderCore implements Finalizable {
  /// A sender with an interval in microseconds.
  SenderCore(int intervalUs) {
    _p = allocState(this, zmidi_active_sensing_sender_size());
    check(
      zmidi_active_sensing_sender_init(_p, intervalUs),
      'ActiveSensingSender',
    );
  }

  final _s = Scratch();
  late Pointer<ZmidiActiveSensingSender> _p;

  /// The program sent a byte.
  void sent(int nowUs) =>
      check(zmidi_active_sensing_sender_sent(_p, nowUs), 'sent');

  /// Checks the interval; gives true to send FEH.
  bool poll(int nowUs) => _s.run((arena) {
    final send = arena<Bool>();
    check(zmidi_active_sensing_sender_poll(_p, nowUs, send), 'poll');
    return send.value;
  });

  /// Frees the C state and the scratch block.
  void free() {
    if (_p != nullptr) {
      releaseState(this, _p);
      _p = nullptr;
    }
    _s.release();
  }
}
