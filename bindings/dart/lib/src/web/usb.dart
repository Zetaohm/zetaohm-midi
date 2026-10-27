/// The web form of the USB link: the TypeScript package through dart:js_interop. The values are
/// checked by `usb.dart`.
library;

import 'dart:js_interop';
import 'dart:js_interop_unsafe';
import 'dart:typed_data';

import '../usb.dart';
import 'runtime.dart';

/// The TypeScript UsbLink.
final class UsbLinkState {
  /// A link at alternate setting 0.
  UsbLinkState(bool hostSide, int groupMask, int maxPacket, int protocol)
    : _o = tsNew('UsbLink', [
        jsObject({
          'hostSide': hostSide.toJS,
          'groupMask': groupMask.toJS,
          'maxPacket': maxPacket.toJS,
          'protocol': protocol.toJS,
        }),
      ]);

  final JSObject _o;

  /// Sets the alternate setting.
  void setAlternateSetting(int setting, int groupMask, int maxPacket) =>
      call<JSAny?>(_o, 'setAlternateSetting', [
        setting.toJS,
        groupMask.toJS,
        maxPacket.toJS,
      ]);

  /// Reports a USB reset.
  void usbReset() => call<JSAny?>(_o, 'usbReset');

  /// Reports a configuration change.
  void configurationChange() => call<JSAny?>(_o, 'configurationChange');

  /// The alternate setting.
  int get setting => intOf(prop(_o, 'setting'));

  /// True at alternate setting 1.
  bool get isUmp => boolOf(prop(_o, 'isUmp'));

  /// The size of the written packets.
  int get packetSize => intOf(prop(_o, 'packetSize'));

  /// True when the writer has bytes for the next packet.
  bool get pending => boolOf(prop(_o, 'pending'));

  /// True when the reader holds part of a message.
  bool get holdsPart => boolOf(prop(_o, 'holdsPart'));

  /// Reads one transfer.
  ({List<List<int>> umps, int errors}) read(List<int> data) {
    final r = call<JSObject>(_o, 'read', [jsBytes(data)]);
    return (umps: umps(prop(r, 'umps')), errors: intOf(prop(r, 'errors')));
  }

  /// Writes one packet.
  ({Uint8List packet, int taken}) writePacket(List<List<int>> words) {
    final r = call<JSObject>(_o, 'writePacket', [
      [for (final u in words) jsInts(u)].toJS,
    ]);
    return (packet: bytes(prop(r, 'packet')), taken: intOf(prop(r, 'taken')));
  }

  /// Frees the WASM memory.
  void free() => call<JSAny?>(_o, 'free');
}

JSString _direction(UsbDirection d) =>
    (d == UsbDirection.midiOut ? 'out' : 'in').toJS;

JSObject _block(UsbBlock b) => jsObject({
  'id': b.id.toJS,
  'kind': b.kind.name.toJS,
  'firstGroup': b.firstGroup.toJS,
  'groupCount': b.groupCount.toJS,
  'string': b.string.toJS,
  'protocol': b.protocol.toJS,
  'maxInputBandwidth': b.maxInputBandwidth.toJS,
  'maxOutputBandwidth': b.maxOutputBandwidth.toJS,
});

JSObject _setting(UsbSetting s) => jsObject({
  'string': s.string.toJS,
  'endpoints': [
    for (final e in s.endpoints)
      jsObject({
        'direction': _direction(e.direction),
        'number': e.number.toJS,
        'transfer': e.transfer.name.toJS,
        'interval': e.interval.toJS,
        'maxPacket': e.maxPacket?.toJS,
        'blocks': jsInts(e.blocks),
      }),
  ].toJS,
  'blocks': switch (s.blocks) {
    final b? => [for (final x in b) _block(x)].toJS,
    null => null,
  },
});

UsbBlockReport _report(JSObject o) => (
  id: intOf(prop(o, 'id')),
  kind: intOf(prop(o, 'kind')),
  firstGroup: intOf(prop(o, 'firstGroup')),
  groupCount: intOf(prop(o, 'groupCount')),
  string: intOf(prop(o, 'string')),
  protocol: intOf(prop(o, 'protocol')),
  maxInputBandwidth: intOf(prop(o, 'maxInputBandwidth')),
  maxOutputBandwidth: intOf(prop(o, 'maxOutputBandwidth')),
);

/// The TypeScript UsbFunction.
final class UsbFunctionState {
  /// A function of the checked description.
  UsbFunctionState(
    int acInterface,
    int msInterface,
    bool highSpeed,
    int acString,
    UsbAlt0 alt0,
    List<UsbSetting> settings,
  ) : _o = tsNew('UsbFunction', [
        jsObject({
          'acInterface': acInterface.toJS,
          'msInterface': msInterface.toJS,
          'highSpeed': highSpeed.toJS,
          'acString': acString.toJS,
          'alt0': jsObject({
            'string': alt0.string.toJS,
            'outCables': alt0.outCables.toJS,
            'inCables': alt0.inCables.toJS,
            'outEndpoint': alt0.outEndpoint.toJS,
            'inEndpoint': alt0.inEndpoint.toJS,
            'maxPacket': alt0.maxPacket.toJS,
          }),
          'settings': [for (final s in settings) _setting(s)].toJS,
        }),
      ]);

  /// The default function of USB-MIDI v2.0 Appendix B.
  UsbFunctionState.defaultFunction(
    int acInterface,
    int msInterface,
    bool highSpeed,
  ) : _o = guard(
        () => tsClass('UsbFunction').callMethodVarArgs<JSObject>(
          'defaultFunction'.toJS,
          [acInterface.toJS, msInterface.toJS, highSpeed.toJS],
        ),
      );

  final JSObject _o;

  /// The descriptors of the function.
  Uint8List descriptors() => bytes(call(_o, 'descriptors'));

  /// The endpoints of every setting.
  List<UsbSettingEndpoint> endpoints() => [
    for (final e in call<JSArray<JSObject>>(_o, 'endpoints').toDart)
      (
        alternateSetting: intOf(prop(e, 'alternateSetting')),
        address: intOf(prop(e, 'address')),
        transfer: intOf(prop(e, 'transfer')),
        interval: intOf(prop(e, 'interval')),
        maxPacket: intOf(prop(e, 'maxPacket')),
      ),
  ];

  /// The Group Terminal mask of a direction in a setting.
  int groupMask(int setting, UsbDirection direction) =>
      intOf(call(_o, 'groupMask', [setting.toJS, _direction(direction)]));

  /// The Group Terminal Block set of a setting.
  Uint8List gtbSet(int setting) => bytes(call(_o, 'gtbSet', [setting.toJS]));

  /// The answer to a SETUP packet.
  UsbSetupAnswer answerSetup(List<int> setup) {
    final r = call<JSObject>(_o, 'answerSetup', [jsBytes(setup)]);
    return (
      response: UsbSetupResponse.values.byName(
        prop<JSString>(r, 'response').toDart,
      ),
      data: bytes(prop(r, 'data')),
    );
  }

  /// Frees the WASM memory.
  void free() => call<JSAny?>(_o, 'free');
}

/// The SETUP packet of the Group Terminal Block request.
Uint8List gtbRequest(int setting, int interface, int length) =>
    bytes(tsCall('gtbRequest', [setting.toJS, interface.toJS, length.toJS]));

/// wTotalLength of the header of an answer.
int gtbTotalLength(List<int> answer) =>
    intOf(tsCall('gtbTotalLength', [jsBytes(answer)]));

/// The Group Terminal Blocks of an answer.
List<UsbBlockReport> gtbBlocks(List<int> answer) => [
  for (final b in tsCall<JSArray<JSObject>>('gtbBlocks', [
    jsBytes(answer),
  ]).toDart)
    _report(b),
];
