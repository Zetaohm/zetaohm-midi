/// The native form of the USB link: zmidi-usb MidiLink through dart:ffi and the C ABI. The values
/// are checked by `usb.dart`.
library;

import 'dart:ffi';
import 'dart:typed_data';

import 'package:ffi/ffi.dart';

import '../bindings.g.dart';
import '../errors.dart';
import '../native.dart';
import '../usb.dart';

/// The C state of a UsbLink.
final class UsbLinkState implements Finalizable {
  /// A link at alternate setting 0.
  UsbLinkState(bool hostSide, int groupMask, int maxPacket, int protocol) {
    _p = allocState(this, zmidi_usb_link_size());
    using((arena) {
      final c = arena<ZmidiUsbLinkConfig>();
      c.ref
        ..host_side = hostSide
        ..group_mask = groupMask
        ..max_packet = maxPacket
        ..protocol = protocol;
      check(zmidi_usb_link_init(_p, c), 'UsbLink');
    });
  }

  /// The arguments of the calls (no native allocation for each call).
  final _s = Scratch();

  late Pointer<ZmidiUsbLink> _p;

  /// Sets the alternate setting.
  void setAlternateSetting(int setting, int groupMask, int maxPacket) => check(
    zmidi_usb_link_set_alternate_setting(_p, setting, groupMask, maxPacket),
    'setAlternateSetting',
  );

  /// Reports a USB reset.
  void usbReset() => check(zmidi_usb_link_usb_reset(_p), 'usbReset');

  /// Reports a configuration change.
  void configurationChange() =>
      check(zmidi_usb_link_configuration_change(_p), 'configurationChange');

  /// The alternate setting.
  int get setting => zmidi_usb_link_setting(_p);

  /// True at alternate setting 1.
  bool get isUmp => zmidi_usb_link_is_ump(_p);

  /// The size of the written packets.
  int get packetSize => zmidi_usb_link_packet_size(_p);

  /// True when the writer has bytes for the next packet.
  bool get pending => zmidi_usb_link_pending(_p);

  /// True when the reader holds part of a message.
  bool get holdsPart => zmidi_usb_link_holds_part(_p);

  /// Reads one transfer.
  ({List<List<int>> umps, int errors}) read(List<int> data) {
    final cap = data.length ~/ 4 + 1;
    return _s.run((arena) {
      final out = arena<ZmidiUsbUmp>(cap);
      final count = arena<UintPtr>();
      final errors = arena<UintPtr>();
      check(
        zmidi_usb_link_read(
          _p,
          bytesIn(arena, data),
          data.length,
          out,
          cap,
          count,
          errors,
        ),
        'read',
      );
      return (
        umps: [
          for (var i = 0; i < count.value; i++)
            [for (var k = 0; k < out[i].word_count; k++) out[i].words[k]],
        ],
        errors: errors.value,
      );
    });
  }

  /// Writes one packet.
  ({Uint8List packet, int taken}) writePacket(List<List<int>> umps) {
    final n = packetSize;
    return _s.run((arena) {
      final pu = arena<ZmidiUsbUmp>(umps.isEmpty ? 1 : umps.length);
      for (var i = 0; i < umps.length; i++) {
        final w = umps[i];
        pu[i].word_count = w.length;
        for (var k = 0; k < w.length; k++) {
          pu[i].words[k] = w[k];
        }
      }
      final out = arena<Uint8>(n);
      final len = arena<UintPtr>();
      final taken = arena<UintPtr>();
      check(
        zmidi_usb_link_write_packet(_p, pu, umps.length, out, n, len, taken),
        'writePacket',
      );
      return (packet: bytesOut(out, len.value), taken: taken.value);
    });
  }

  /// Frees the C state and the scratch block.
  void free() {
    if (_p != nullptr) {
      releaseState(this, _p);
      _p = nullptr;
    }
    _s.release();
  }
}

/// The bytes of a call that writes into a buffer: `call(out, capacity, written)`.
Uint8List _written(
  int capacity,
  ZmidiStatus Function(Pointer<Uint8> out, int cap, Pointer<UintPtr> written)
  call,
  String what,
) => using((arena) {
  final out = arena<Uint8>(capacity < 1 ? 1 : capacity);
  final written = arena<UintPtr>();
  check(call(out, capacity, written), what);
  return bytesOut(out, written.value);
});

/// The largest Group Terminal Block set: the header and 16 blocks.
const _gtbSetMax = 5 + 13 * ZMIDI_USB_MAX_BLOCKS;

void _writeBlock(ZmidiUsbBlock c, UsbBlock b) {
  c
    ..id = b.id
    ..kind = b.kind.index
    ..first_group = b.firstGroup
    ..num_groups = b.groupCount
    ..string_index = b.string
    ..protocol = b.protocol
    ..max_input_bandwidth = b.maxInputBandwidth
    ..max_output_bandwidth = b.maxOutputBandwidth;
}

UsbBlockReport _readBlock(ZmidiUsbBlock c) => (
  id: c.id,
  kind: c.kind,
  firstGroup: c.first_group,
  groupCount: c.num_groups,
  string: c.string_index,
  protocol: c.protocol,
  maxInputBandwidth: c.max_input_bandwidth,
  maxOutputBandwidth: c.max_output_bandwidth,
);

/// The C state of a UsbFunction (ZmidiUsbFunction).
final class UsbFunctionState implements Finalizable {
  /// A function of the checked description.
  UsbFunctionState(
    int acInterface,
    int msInterface,
    bool highSpeed,
    int acString,
    UsbAlt0 alt0,
    List<UsbSetting> settings,
  ) {
    _p = allocState(this, zmidi_usb_function_size());
    using((arena) {
      final c = arena<ZmidiUsbFunctionConfig>();
      c.ref
        ..ac_interface = acInterface
        ..ms_interface = msInterface
        ..high_speed = highSpeed
        ..ac_string = acString
        ..alt0_string = alt0.string
        ..n_out = alt0.outCables
        ..n_in = alt0.inCables
        ..out_endpoint = alt0.outEndpoint
        ..in_endpoint = alt0.inEndpoint
        ..max_packet = alt0.maxPacket;
      check(zmidi_usb_function_init(_p, c), 'UsbFunction');
    });
    for (final s in settings) {
      _addSetting(s, highSpeed);
    }
    _validate();
  }

  /// The default function of USB-MIDI v2.0 Appendix B.
  UsbFunctionState.defaultFunction(
    int acInterface,
    int msInterface,
    bool highSpeed,
  ) {
    _p = allocState(this, zmidi_usb_function_size());
    check(
      zmidi_usb_function_init_default(_p, acInterface, msInterface, highSpeed),
      'UsbFunction',
    );
    _validate();
  }

  late Pointer<ZmidiUsbFunction> _p;

  void _addSetting(UsbSetting s, bool highSpeed) => using((arena) {
    final alt = arena<Uint8>();
    check(zmidi_usb_function_add_setting(_p, s.string, alt), 'UsbFunction');
    final e = arena<ZmidiUsbEndpoint>();
    for (final ep in s.endpoints) {
      e.ref
        ..direction = ep.direction.index
        ..number = ep.number
        ..transfer = ep.transfer == UsbTransfer.bulk ? 2 : 3
        ..interval = ep.interval
        ..max_packet = ep.maxPacket ?? (highSpeed ? 512 : 64)
        ..block_count = ep.blocks.length;
      for (var i = 0; i < ep.blocks.length; i++) {
        e.ref.block_ids[i] = ep.blocks[i];
      }
      check(zmidi_usb_function_add_endpoint(_p, alt.value, e), 'UsbFunction');
    }
    final blocks = s.blocks;
    if (blocks != null) {
      final pb = arena<ZmidiUsbBlock>(blocks.isEmpty ? 1 : blocks.length);
      for (var i = 0; i < blocks.length; i++) {
        _writeBlock(pb[i], blocks[i]);
      }
      check(
        zmidi_usb_function_set_blocks(_p, alt.value, pb, blocks.length),
        'UsbFunction',
      );
    }
  });

  /// Throws RejectedException when the function breaks a rule of zmidi-usb.
  void _validate() => using((arena) {
    final st = zmidi_usb_function_len(_p, arena<UintPtr>());
    if (st == ZmidiStatus.ZMIDI_STATUS_REJECTED) {
      throw const RejectedException(
        'UsbFunction: the description breaks a rule of USB-MIDI v2.0 (zmidi-usb)',
      );
    }
    check(st, 'UsbFunction');
  });

  /// The descriptors of the function.
  Uint8List descriptors() {
    final n = using((arena) {
      final len = arena<UintPtr>();
      check(zmidi_usb_function_len(_p, len), 'descriptors');
      return len.value;
    });
    return _written(
      n,
      (out, cap, w) => zmidi_usb_function_write(_p, out, cap, w),
      'descriptors',
    );
  }

  /// The endpoints of every setting.
  List<UsbSettingEndpoint> endpoints() => using((arena) {
    final e = arena<ZmidiUsbSettingEndpoint>();
    final out = <UsbSettingEndpoint>[];
    for (var i = 0; i < zmidi_usb_function_endpoint_count(_p); i++) {
      check(zmidi_usb_function_endpoint(_p, i, e), 'endpoints');
      out.add((
        alternateSetting: e.ref.alternate_setting,
        address: e.ref.address,
        transfer: e.ref.transfer,
        interval: e.ref.interval,
        maxPacket: e.ref.max_packet,
      ));
    }
    return out;
  });

  /// The Group Terminal mask of a direction in a setting.
  int groupMask(int setting, UsbDirection direction) =>
      zmidi_usb_function_group_mask(_p, setting, direction.index);

  /// The Group Terminal Block set of a setting.
  Uint8List gtbSet(int setting) => _written(
    _gtbSetMax,
    (out, cap, w) => zmidi_usb_gtb_set_write(_p, setting, out, cap, w),
    'gtbSet',
  );

  /// The answer to a SETUP packet.
  UsbSetupAnswer answerSetup(List<int> setup) => using((arena) {
    final out = arena<Uint8>(_gtbSetMax);
    final response = arena<UnsignedInt>();
    final written = arena<UintPtr>();
    check(
      zmidi_usb_gtb_setup(
        _p,
        bytesIn(arena, setup),
        out,
        _gtbSetMax,
        response,
        written,
      ),
      'answerSetup',
    );
    return (
      response: UsbSetupResponse.values[response.value],
      data: bytesOut(out, written.value),
    );
  });

  /// Frees the C state.
  void free() {
    if (_p != nullptr) {
      releaseState(this, _p);
      _p = nullptr;
    }
  }
}

/// The SETUP packet of the Group Terminal Block request.
Uint8List gtbRequest(int setting, int interface, int length) {
  checkAbi();
  return _written(_setupLen, (out, cap, w) {
    w.value = _setupLen;
    return zmidi_usb_gtb_request(setting, interface, length, out);
  }, 'gtbRequest');
}

/// The length of a SETUP packet.
const _setupLen = ZMIDI_USB_SETUP_LEN;

/// wTotalLength of the header of an answer.
int gtbTotalLength(List<int> answer) {
  checkAbi();
  return using((arena) {
    final total = arena<Uint16>();
    check(
      zmidi_usb_gtb_total_length(bytesIn(arena, answer), answer.length, total),
      'gtbTotalLength',
    );
    return total.value;
  });
}

/// The Group Terminal Blocks of an answer.
List<UsbBlockReport> gtbBlocks(List<int> answer) {
  checkAbi();
  return using((arena) {
    final pa = bytesIn(arena, answer);
    final b = arena<ZmidiUsbBlock>();
    final out = <UsbBlockReport>[];
    for (var i = 0; ; i++) {
      final st = zmidi_usb_gtb_block(pa, answer.length, i, b);
      if (st == ZmidiStatus.ZMIDI_STATUS_NO_PENDING) {
        return out;
      }
      check(st, 'gtbBlocks');
      out.add(_readBlock(b.ref));
    }
  });
}
