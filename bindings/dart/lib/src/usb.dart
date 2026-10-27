/// USB-MIDI 1.0 Event Packets and USB MIDI 2.0 UMP words (zmidi-usb MidiLink through the C ABI):
/// one cable, alternate setting 0 (USB-MIDI 1.0) or 1 (USB MIDI 2.0), with a translator to the
/// protocol of the program; and the USB descriptor writer with the Group Terminal Block requests
/// (zmidi-usb FunctionConfig with fixed capacities, C ABI 0.5).
library;

import 'dart:typed_data';

import 'checks.dart';
import 'errors.dart';
import 'native/usb.dart' if (dart.library.js_interop) 'web/usb.dart' as be;

/// One USB MIDI interface with one cable: transfers in, UMPs out, and back.
final class UsbLink {
  /// A link at alternate setting 0. [hostSide]: true on a host (the received endpoint is a MIDI IN
  /// endpoint), false on a device. [groupMask]: the Group Terminals of the received endpoint (bit
  /// n is Group n). [maxPacket]: wMaxPacketSize of the sent endpoint, 4 or more. [protocol]: the
  /// protocol of the UMPs that [read] gives, 1 (MIDI 1.0) or 2 (MIDI 2.0).
  UsbLink({
    bool hostSide = false,
    int groupMask = 0xffff,
    int maxPacket = 64,
    int protocol = 1,
  }) : _b = be.UsbLinkState(
         hostSide,
         checkInt('groupMask', groupMask, 0xffff),
         checkInt('maxPacket', maxPacket, 0xffff),
         checkProtocol(protocol),
       );

  final be.UsbLinkState _b;
  bool _freed = false;

  be.UsbLinkState get _live {
    checkFreed(_freed, this);
    return _b;
  }

  /// Sets the alternate setting (0 or 1) after SET_INTERFACE, with its Group Terminals and packet
  /// size.
  void setAlternateSetting(int setting, int groupMask, int maxPacket) =>
      _live.setAlternateSetting(
        checkInt('setting', setting, 255),
        checkInt('groupMask', groupMask, 0xffff),
        checkInt('maxPacket', maxPacket, 0xffff),
      );

  /// Reports a USB reset: the link goes to alternate setting 0.
  void usbReset() => _live.usbReset();

  /// Reports a configuration change (SET_CONFIGURATION): the link goes to alternate setting 0.
  void configurationChange() => _live.configurationChange();

  /// The alternate setting: 0 (USB-MIDI 1.0) or 1 (USB MIDI 2.0).
  int get setting => _live.setting;

  /// True at alternate setting 1 (UMP words on the wire).
  bool get isUmp => _live.isUmp;

  /// The size in bytes of the USB packets that the writer writes.
  int get packetSize => _live.packetSize;

  /// True when the writer has bytes for the next packet (a split UMP or a SysEx part).
  bool get pending => _live.pending;

  /// True when the reader holds part of a UMP or of a SysEx message from the last transfer.
  bool get holdsPart => _live.holdsPart;

  /// Reads the data of one received transfer. Gives its UMPs and the count of errors.
  ({List<List<int>> umps, int errors}) read(List<int> data) => _live.read(data);

  /// Writes one packet from [umps]. Gives the packet (empty: nothing to send) and the count of
  /// UMPs taken.
  ({Uint8List packet, int taken}) writePacket(List<List<int>> umps) =>
      _live.writePacket([for (final u in umps) List.of(checkUmp(u))]);

  /// Frees the memory of the link now.
  void free() {
    if (!_freed) {
      _freed = true;
      _b.free();
    }
  }
}

// --- USB descriptors and Group Terminal Block requests (C ABI 0.5, DEC-019 third addendum) ---

/// The most USB MIDI 2.0 Alternate Settings of a [UsbFunction] (ZMIDI_USB_MAX_SETTINGS).
const usbMaxSettings = 2;

/// The most endpoints of one USB MIDI 2.0 setting (ZMIDI_USB_MAX_ENDPOINTS).
const usbMaxEndpoints = 4;

/// The most Group Terminal Blocks of one setting, and the most block IDs of one endpoint
/// (ZMIDI_USB_MAX_BLOCKS).
const usbMaxBlocks = 16;

/// The length of a SETUP packet.
const _setupLen = 8;

/// The kind of a Group Terminal Block (USB-MIDI v2.0 Table 5-6, bGrpTrmBlkType).
enum UsbBlockKind {
  /// 0x00: bidirectional.
  bidirectional,

  /// 0x01: input only.
  inputOnly,

  /// 0x02: output only.
  outputOnly,
}

/// The direction of an endpoint.
enum UsbDirection {
  /// A MIDI OUT endpoint: the host sends.
  midiOut,

  /// A MIDI IN endpoint: the device sends.
  midiIn,
}

/// The transfer type of an endpoint.
enum UsbTransfer {
  /// Bulk.
  bulk,

  /// Interrupt.
  interrupt,
}

/// What the device does with a SETUP packet.
enum UsbSetupResponse {
  /// Send the bytes of the answer.
  answered,

  /// Stall the request.
  stall,

  /// Another handler takes the request.
  notHandled,
}

/// One Group Terminal Block of a USB MIDI 2.0 setting (USB-MIDI v2.0 5.4.2.1).
final class UsbBlock {
  /// The block [id] (1 to 255) with its first Group [firstGroup] (0 to 15; 0 is Group 1).
  const UsbBlock({
    required this.id,
    this.kind = UsbBlockKind.bidirectional,
    required this.firstGroup,
    this.groupCount = 1,
    this.string = 0,
    this.protocol = 0x11,
    this.maxInputBandwidth = 0,
    this.maxOutputBandwidth = 0,
  });

  /// bGrpTrmBlkID.
  final int id;

  /// bGrpTrmBlkType.
  final UsbBlockKind kind;

  /// nGroupTrm: the first Group.
  final int firstGroup;

  /// nNumGroupTrm: the number of Groups, 1 to 16.
  final int groupCount;

  /// iBlockItem: the string index (0: no string).
  final int string;

  /// bMIDIProtocol (0x11: MIDI 2.0).
  final int protocol;

  /// wMaxInputBandwidth in 4,000 bytes per second (0: not known).
  final int maxInputBandwidth;

  /// wMaxOutputBandwidth in 4,000 bytes per second (0: not known).
  final int maxOutputBandwidth;
}

/// One endpoint of a USB MIDI 2.0 setting.
final class UsbEndpoint {
  /// The endpoint [number] (1 to 15) of [direction]. [maxPacket]: wMaxPacketSize, or null for 64
  /// at full speed and 512 at high speed. [blocks]: the IDs of its Group Terminal Blocks, 1 to 16.
  const UsbEndpoint({
    required this.direction,
    required this.number,
    this.transfer = UsbTransfer.bulk,
    this.interval = 0,
    this.maxPacket,
    this.blocks = const [1],
  });

  /// The direction.
  final UsbDirection direction;

  /// The endpoint number.
  final int number;

  /// The transfer type.
  final UsbTransfer transfer;

  /// bInterval (1 or more for an interrupt endpoint).
  final int interval;

  /// wMaxPacketSize, or null for the default of the speed.
  final int? maxPacket;

  /// The IDs of the Group Terminal Blocks of the endpoint.
  final List<int> blocks;
}

/// One USB MIDI 2.0 Alternate Setting (1, then 2).
final class UsbSetting {
  /// A setting with 1 to 4 [endpoints] and 1 to 16 [blocks] (null: one bidirectional block, ID 1,
  /// Group 1, bMIDIProtocol 0x11).
  const UsbSetting({this.string = 0, required this.endpoints, this.blocks});

  /// iInterface.
  final int string;

  /// The endpoints.
  final List<UsbEndpoint> endpoints;

  /// The Group Terminal Blocks, or null for the default block.
  final List<UsbBlock>? blocks;
}

/// Alternate Setting 0 (USB-MIDI 1.0) as cable counts.
final class UsbAlt0 {
  /// [outCables] and [inCables] cables (0 to 16) on the endpoints [outEndpoint] and [inEndpoint].
  /// [maxPacket]: wMaxPacketSize, or 0 for 64 at full speed and 512 at high speed.
  const UsbAlt0({
    this.string = 0,
    this.outCables = 1,
    this.inCables = 1,
    this.outEndpoint = 1,
    this.inEndpoint = 1,
    this.maxPacket = 0,
  });

  /// iInterface.
  final int string;

  /// The cables of the MIDI OUT endpoint.
  final int outCables;

  /// The cables of the MIDI IN endpoint.
  final int inCables;

  /// The number of the MIDI OUT endpoint.
  final int outEndpoint;

  /// The number of the MIDI IN endpoint.
  final int inEndpoint;

  /// wMaxPacketSize, or 0 for the default of the speed.
  final int maxPacket;
}

/// One endpoint for the USB stack: its Alternate Setting, bEndpointAddress (0x80 set for IN),
/// bmAttributes (2 bulk, 3 interrupt), bInterval and wMaxPacketSize.
typedef UsbSettingEndpoint = ({
  int alternateSetting,
  int address,
  int transfer,
  int interval,
  int maxPacket,
});

/// A Group Terminal Block that the host read, with the raw field values (the reader does not check
/// them): bGrpTrmBlkType [kind] is 0 bidirectional, 1 input only, 2 output only.
typedef UsbBlockReport = ({
  int id,
  int kind,
  int firstGroup,
  int groupCount,
  int string,
  int protocol,
  int maxInputBandwidth,
  int maxOutputBandwidth,
});

/// The answer to a SETUP packet: the response and the bytes to send (empty unless answered).
typedef UsbSetupAnswer = ({UsbSetupResponse response, Uint8List data});

void _checkBlock(UsbBlock b) {
  checkInt('block id', b.id, 255);
  checkInt('firstGroup', b.firstGroup, 15);
  checkInt('groupCount', b.groupCount, 16);
  checkInt('block string', b.string, 255);
  checkInt('protocol', b.protocol, 255);
  checkInt('maxInputBandwidth', b.maxInputBandwidth, 0xffff);
  checkInt('maxOutputBandwidth', b.maxOutputBandwidth, 0xffff);
}

void _checkSetting(UsbSetting s) {
  checkInt('setting string', s.string, 255);
  if (s.endpoints.length > usbMaxEndpoints) {
    throw InvalidValueException(
      'a setting has at most $usbMaxEndpoints endpoints, not ${s.endpoints.length}',
    );
  }
  for (final e in s.endpoints) {
    checkInt('endpoint number', e.number, 15);
    checkInt('interval', e.interval, 255);
    if (e.maxPacket != null) {
      checkInt('maxPacket', e.maxPacket!, 0xffff);
    }
    if (e.blocks.isEmpty || e.blocks.length > usbMaxBlocks) {
      throw InvalidValueException(
        'an endpoint names 1 to $usbMaxBlocks blocks, not ${e.blocks.length}',
      );
    }
    for (final id in e.blocks) {
      checkInt('block id', id, 255);
    }
  }
  final blocks = s.blocks;
  if (blocks != null) {
    if (blocks.length > usbMaxBlocks) {
      throw InvalidValueException(
        'a setting has at most $usbMaxBlocks blocks, not ${blocks.length}',
      );
    }
    blocks.forEach(_checkBlock);
  }
}

/// A USB MIDI function: the descriptors of the AudioControl and MIDI Streaming interfaces
/// (USB-MIDI v2.0 Appendix B), the endpoints for the USB stack, the Group Terminal masks for
/// [UsbLink], and the Group Terminal Block requests (USB-MIDI v2.0 section 6). The constructor
/// takes the full description and checks it against the rules of zmidi-usb: a broken rule throws
/// [RejectedException].
final class UsbFunction {
  /// A function with the AudioControl interface [acInterface], the MIDI Streaming interface
  /// [msInterface], Alternate Setting 0 [alt0] and 0 to 2 USB MIDI 2.0 [settings].
  UsbFunction({
    required int acInterface,
    required int msInterface,
    bool highSpeed = false,
    int acString = 0,
    UsbAlt0 alt0 = const UsbAlt0(),
    List<UsbSetting> settings = const [],
  }) : _b = be.UsbFunctionState(
         checkInt('acInterface', acInterface, 255),
         checkInt('msInterface', msInterface, 255),
         highSpeed,
         checkInt('acString', acString, 255),
         _checkAlt0(alt0),
         _checkSettings(settings),
       );

  /// The default function (USB-MIDI v2.0 Appendix B): one cable in each direction, and one USB
  /// MIDI 2.0 setting with one bidirectional block of Group 1.
  UsbFunction.defaultFunction(
    int acInterface,
    int msInterface, {
    bool highSpeed = false,
  }) : _b = be.UsbFunctionState.defaultFunction(
         checkInt('acInterface', acInterface, 255),
         checkInt('msInterface', msInterface, 255),
         highSpeed,
       );

  static UsbAlt0 _checkAlt0(UsbAlt0 a) {
    checkInt('alt0 string', a.string, 255);
    checkInt('outCables', a.outCables, 255);
    checkInt('inCables', a.inCables, 255);
    checkInt('outEndpoint', a.outEndpoint, 15);
    checkInt('inEndpoint', a.inEndpoint, 15);
    checkInt('alt0 maxPacket', a.maxPacket, 0xffff);
    return a;
  }

  static List<UsbSetting> _checkSettings(List<UsbSetting> settings) {
    if (settings.length > usbMaxSettings) {
      throw InvalidValueException(
        'a function has at most $usbMaxSettings USB MIDI 2.0 settings, '
        'not ${settings.length}',
      );
    }
    settings.forEach(_checkSetting);
    return List.of(settings);
  }

  final be.UsbFunctionState _b;
  bool _freed = false;

  be.UsbFunctionState get _live {
    checkFreed(_freed, this);
    return _b;
  }

  /// The descriptors of the function (USB-MIDI v2.0 Appendix B order), for the Configuration
  /// descriptor.
  Uint8List descriptors() => _live.descriptors();

  /// The endpoints of every setting in descriptor order, for the USB stack.
  List<UsbSettingEndpoint> endpoints() => _live.endpoints();

  /// The Group Terminals of the endpoints of [direction] in Alternate Setting [setting] (bit n is
  /// Group n), for [UsbLink.setAlternateSetting]: 0xFFFF for Alternate Setting 0, 0 for a setting
  /// that does not exist.
  int groupMask(int setting, UsbDirection direction) {
    final b = _live;
    return b.groupMask(checkInt('setting', setting, 255), direction);
  }

  /// The Group Terminal Block descriptor set of USB MIDI 2.0 Alternate Setting [setting]. Throws
  /// [InvalidValueException] for Alternate Setting 0 or a setting that does not exist.
  Uint8List gtbSet(int setting) {
    final b = _live;
    return b.gtbSet(checkInt('setting', setting, 255));
  }

  /// The device side of the Group Terminal Block request: what to do with the 8 bytes of a SETUP
  /// packet.
  UsbSetupAnswer answerSetup(List<int> setup) {
    final b = _live;
    if (setup.length != _setupLen) {
      throw InvalidValueException(
        'a SETUP packet has $_setupLen bytes, not ${setup.length}',
      );
    }
    return b.answerSetup(checkBytes('setup', setup));
  }

  /// Frees the memory of the function now.
  void free() {
    if (!_freed) {
      _freed = true;
      _b.free();
    }
  }
}

/// The host side: the 8 bytes of the SETUP packet of the Group Terminal Block request for Alternate
/// Setting [setting] of the interface [interface] with wLength [length]. Send wLength 5 first (the
/// header), then the wTotalLength that [gtbTotalLength] reads from the answer.
Uint8List gtbRequest(int setting, int interface, int length) => be.gtbRequest(
  checkInt('setting', setting, 255),
  checkInt('interface', interface, 255),
  checkInt('length', length, 0xffff),
);

/// The host side: wTotalLength of the Group Terminal Block Header at the start of [answer]. Throws
/// [RejectedException] when the answer has no header.
int gtbTotalLength(List<int> answer) =>
    be.gtbTotalLength(checkBytes('answer', answer));

/// The host side: the Group Terminal Blocks of [answer], read with the lenient reader of zmidi-usb.
List<UsbBlockReport> gtbBlocks(List<int> answer) =>
    be.gtbBlocks(checkBytes('answer', answer));
