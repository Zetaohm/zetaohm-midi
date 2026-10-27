// USB-MIDI 1.0 Event Packets and USB MIDI 2.0 UMP words (REQ-DART-USB-001).
import 'package:test/test.dart';
import 'package:zetaohm_midi/zetaohm_midi.dart';

void main() {
  setUpAll(initWeb);

  descriptorTests();
  scratchTests();

  test('Event Packets at setting 0', () {
    final link = UsbLink();
    expect([link.setting, link.isUmp], [0, false]);
    final a = link.read([0x09, 0x90, 0x3C, 0x64]);
    expect(
      [a.umps, a.errors],
      [
        [
          [0x20903C64],
        ],
        0,
      ],
    );
    final b = link.read([0x04, 0xF0, 0x7E, 0x7F, 0x07, 0x06, 0x01, 0xF7]);
    expect(b.umps, [
      [0x30047E7F, 0x06010000],
    ]);
    final p = link.writePacket([
      [0x20903C64],
    ]);
    expect(
      [p.packet, p.taken],
      [
        [0x09, 0x90, 0x3C, 0x64],
        1,
      ],
    );
  });

  test('UMP words at setting 1 and a USB reset', () {
    final link = UsbLink()..setAlternateSetting(1, 0x0001, 64);
    expect([link.setting, link.isUmp], [1, true]);
    const data = [0x00, 0x3C, 0x90, 0x40, 0x00, 0x00, 0x00, 0xC0];
    expect(link.read(data).umps, [
      [0x40903C00, 0xC0000000],
    ]);
    final p = link.writePacket([
      [0x40903C00, 0xC0000000],
    ]);
    expect([p.packet, p.taken], [data, 1]);
    link.usbReset();
    expect(link.setting, 0);
  });

  test('MIDI 2.0 protocol needs no extra setup', () {
    expect(UsbLink(protocol: 2).read([0x09, 0x90, 0x3C, 0x64]).umps, [
      [0x40903C00, 0xC9240000],
    ]);
  });

  test('USB bad values', () {
    expect(() => UsbLink(maxPacket: 2), throwsA(isA<InvalidValueException>()));
    expect(() => UsbLink(protocol: 7), throwsA(isA<InvalidValueException>()));
  });
}

// USB descriptors and Group Terminal Block requests (REQ-DART-USB-002 to REQ-DART-USB-004).
// Vectors: USB-MIDI v2.0 Appendix B, Tables B-3 to B-20 (the function), B-21 and B-22 (the Group
// Terminal Block set), B-23 and B-24 (the requests); the same as crates/zmidi-ffi/tests/c/usb_desc.c.
const appendixBBytes = [
  ...[
    0x09,
    0x04,
    0x00,
    0x00,
    0x00,
    0x01,
    0x01,
    0x00,
    0x00,
  ], // B-3 standard AC interface
  ...[0x09, 0x24, 0x01, 0x00, 0x01, 0x09, 0x00, 0x01, 0x01], // B-4 AC header
  ...[
    0x09,
    0x04,
    0x01,
    0x00,
    0x02,
    0x01,
    0x03,
    0x00,
    0x00,
  ], // B-5 MS interface, alt 0
  ...[0x07, 0x24, 0x01, 0x00, 0x01, 0x41, 0x00], // B-6 MS header, bcdMSC 0x0100
  ...[0x06, 0x24, 0x02, 0x01, 0x01, 0x00], // B-7 Embedded MIDI IN Jack 1
  ...[0x06, 0x24, 0x02, 0x02, 0x02, 0x00], // B-8 External MIDI IN Jack 2
  ...[
    0x09,
    0x24,
    0x03,
    0x01,
    0x03,
    0x01,
    0x02,
    0x01,
    0x00,
  ], // B-9 Embedded MIDI OUT Jack 3
  ...[
    0x09,
    0x24,
    0x03,
    0x02,
    0x04,
    0x01,
    0x01,
    0x01,
    0x00,
  ], // B-10 External MIDI OUT Jack 4
  ...[
    0x09,
    0x05,
    0x01,
    0x02,
    0x40,
    0x00,
    0x00,
    0x00,
    0x00,
  ], // B-11 bulk OUT endpoint 1
  ...[0x05, 0x25, 0x01, 0x01, 0x01], // B-12 MS_GENERAL, Jack 1
  ...[
    0x09,
    0x05,
    0x81,
    0x02,
    0x40,
    0x00,
    0x00,
    0x00,
    0x00,
  ], // B-13 bulk IN endpoint 1
  ...[0x05, 0x25, 0x01, 0x01, 0x03], // B-14 MS_GENERAL, Jack 3
  ...[
    0x09,
    0x04,
    0x01,
    0x01,
    0x02,
    0x01,
    0x03,
    0x00,
    0x00,
  ], // B-15 MS interface, alt 1
  ...[
    0x07,
    0x24,
    0x01,
    0x00,
    0x02,
    0x07,
    0x00,
  ], // B-16 MS header, bcdMSC 0x0200
  ...[0x07, 0x05, 0x01, 0x02, 0x40, 0x00, 0x00], // B-17 bulk OUT endpoint 1
  ...[0x05, 0x25, 0x02, 0x01, 0x01], // B-18 MS_GENERAL_2_0, block 1
  ...[0x07, 0x05, 0x81, 0x03, 0x40, 0x00, 0x01], // B-19 interrupt IN endpoint 1
  ...[0x05, 0x25, 0x02, 0x01, 0x01], // B-20 MS_GENERAL_2_0, block 1
];
const b21b22 = [
  ...[0x05, 0x26, 0x01, 0x12, 0x00],
  ...[
    0x0D,
    0x26,
    0x02,
    0x01,
    0x00,
    0x00,
    0x01,
    0x04,
    0x00,
    0x01,
    0x00,
    0x00,
    0x00,
  ],
];
const b23 = [0x81, 0x06, 0x01, 0x26, 0x01, 0x00, 0x05, 0x00];
const b24 = [0x81, 0x06, 0x01, 0x26, 0x01, 0x00, 0x12, 0x00];

/// The block of Table B-22: ID 1, bidirectional, Group Terminal 1, iBlockItem 4, bMIDIProtocol
/// 0x00, wMaxInputBandwidth 1.
UsbBlock b22({int id = 1, int firstGroup = 0, int groupCount = 1}) => UsbBlock(
  id: id,
  firstGroup: firstGroup,
  groupCount: groupCount,
  string: 4,
  protocol: 0x00,
  maxInputBandwidth: 1,
);

const outEp = UsbEndpoint(direction: UsbDirection.midiOut, number: 1);
const inEp = UsbEndpoint(direction: UsbDirection.midiIn, number: 1);

UsbFunction appendixB() => UsbFunction(
  acInterface: 0,
  msInterface: 1,
  settings: [
    UsbSetting(
      endpoints: const [
        outEp,
        UsbEndpoint(
          direction: UsbDirection.midiIn,
          number: 1,
          transfer: UsbTransfer.interrupt,
          interval: 1,
        ),
      ],
      blocks: [b22()],
    ),
  ],
);

UsbFunction one(UsbSetting s) =>
    UsbFunction(acInterface: 0, msInterface: 1, settings: [s]);

/// A large transfer after a small one: the link keeps one scratch block for its calls, and a
/// larger transfer makes it larger (the first large call uses a temporary block).
void scratchTests() {
  test('a large transfer after a small one', () {
    final link = UsbLink();
    expect(link.read([0x09, 0x90, 0x3C, 0x64]).umps, [
      [0x20903C64],
    ]);
    final many = [
      for (var i = 0; i < 300; i++) ...[0x09, 0x90, i & 0x7F, 0x40],
    ];
    for (var round = 0; round < 2; round++) {
      final r = link.read(many);
      expect(r.errors, 0);
      expect(r.umps.length, 300);
      expect(r.umps[299], [0x20900040 | ((299 & 0x7F) << 8)]);
    }
    expect(link.read([0x09, 0x80, 0x3C, 0x00]).umps, [
      [0x20803C00],
    ]);
    link.free();
    expect(
      () => link.read([0x09, 0x90, 0x3C, 0x64]),
      throwsA(isA<RejectedException>()),
    );
  });
}

void descriptorTests() {
  test('the function of Appendix B', () {
    final f = appendixB();
    expect(f.descriptors(), appendixBBytes);
    expect(f.endpoints(), [
      (
        alternateSetting: 0,
        address: 0x01,
        transfer: 2,
        interval: 0,
        maxPacket: 64,
      ),
      (
        alternateSetting: 0,
        address: 0x81,
        transfer: 2,
        interval: 0,
        maxPacket: 64,
      ),
      (
        alternateSetting: 1,
        address: 0x01,
        transfer: 2,
        interval: 0,
        maxPacket: 64,
      ),
      (
        alternateSetting: 1,
        address: 0x81,
        transfer: 3,
        interval: 1,
        maxPacket: 64,
      ),
    ]);
    expect(
      [
        f.groupMask(0, UsbDirection.midiOut),
        f.groupMask(1, UsbDirection.midiOut),
        f.groupMask(1, UsbDirection.midiIn),
        f.groupMask(2, UsbDirection.midiOut),
      ],
      [0xFFFF, 1, 1, 0],
    );
    expect(f.gtbSet(1), b21b22);
    expect(() => f.gtbSet(0), throwsA(isA<InvalidValueException>()));
    f.free();
    expect(() => f.descriptors(), throwsA(isA<RejectedException>()));
  });

  test('the Group Terminal Block requests of section 6', () {
    final f = appendixB();
    var a = f.answerSetup(b23);
    expect(
      [a.response, a.data],
      [UsbSetupResponse.answered, b21b22.sublist(0, 5)],
    );
    a = f.answerSetup(b24);
    expect([a.response, a.data], [UsbSetupResponse.answered, b21b22]);
    final other = [...b24]..[4] = 0x02; // another interface
    a = f.answerSetup(other);
    expect([a.response, a.data], [UsbSetupResponse.stall, <int>[]]);
    a = f.answerSetup([0x80, 0x06, 0x00, 0x01, 0x00, 0x00, 0x12, 0x00]);
    expect(a.response, UsbSetupResponse.notHandled);
    expect(
      () => f.answerSetup([1, 2, 3]),
      throwsA(isA<InvalidValueException>()),
    );
    // The host side.
    expect(gtbRequest(1, 1, 5), b23);
    expect(gtbRequest(1, 1, 0x12), b24);
    expect(gtbTotalLength(b21b22.sublist(0, 5)), 0x12);
    expect(() => gtbTotalLength(b23), throwsA(isA<RejectedException>()));
    expect(gtbBlocks(b21b22), [
      (
        id: 1,
        kind: 0,
        firstGroup: 0,
        groupCount: 1,
        string: 4,
        protocol: 0,
        maxInputBandwidth: 1,
        maxOutputBandwidth: 0,
      ),
    ]);
    expect(gtbBlocks(b21b22.sublist(0, 5)), isEmpty);
  });

  test('the default function and the capacities', () {
    final d = UsbFunction.defaultFunction(0, 1);
    final bytes = d.descriptors();
    expect(bytes.sublist(0, 123), appendixBBytes.sublist(0, 123));
    expect([bytes[123], bytes[126]], [0x02, 0x00]); // bulk IN, bInterval 0
    final block = gtbBlocks(d.gtbSet(1)).single;
    expect(
      [block.id, block.kind, block.firstGroup, block.protocol],
      [1, 0, 0, 0x11],
    );
    expect(
      UsbFunction.defaultFunction(
        0,
        1,
        highSpeed: true,
      ).endpoints()[3].maxPacket,
      512,
    );
    // 16 blocks, one for each Group, on one MIDI OUT and one MIDI IN endpoint that name all 16.
    final ids = [for (var i = 1; i <= usbMaxBlocks; i++) i];
    final all = one(
      UsbSetting(
        endpoints: [
          UsbEndpoint(direction: UsbDirection.midiOut, number: 1, blocks: ids),
          UsbEndpoint(direction: UsbDirection.midiIn, number: 1, blocks: ids),
        ],
        blocks: [for (final id in ids) b22(id: id, firstGroup: id - 1)],
      ),
    );
    expect(
      [
        all.groupMask(1, UsbDirection.midiOut),
        all.groupMask(1, UsbDirection.midiIn),
      ],
      [0xFFFF, 0xFFFF],
    );
    expect(all.gtbSet(1).length, 5 + 13 * usbMaxBlocks);
    // Alternate Setting 0 only, with strings: the 92 bytes of USB-MIDI v1.0 Appendix B.
    final only = UsbFunction(
      acInterface: 0,
      msInterface: 1,
      acString: 5,
      alt0: const UsbAlt0(string: 6),
    );
    final o = only.descriptors();
    expect([o.length, o[8], o[26]], [92, 5, 6]);
    expect(only.answerSetup(b24).response, UsbSetupResponse.stall);
  });

  test('USB descriptor rules and bad values', () {
    Matcher rejected = throwsA(isA<RejectedException>());
    Matcher invalid = throwsA(isA<InvalidValueException>());
    // The same interface number twice; no endpoint; no block; no Group; no port.
    expect(() => UsbFunction(acInterface: 1, msInterface: 1), rejected);
    expect(() => one(const UsbSetting(endpoints: [])), rejected);
    expect(
      () => one(const UsbSetting(endpoints: [outEp], blocks: [])),
      rejected,
    );
    expect(
      () => one(
        UsbSetting(
          endpoints: const [outEp, inEp],
          blocks: [b22(groupCount: 0)],
        ),
      ),
      rejected,
    );
    expect(
      () => UsbFunction(
        acInterface: 0,
        msInterface: 1,
        alt0: const UsbAlt0(outCables: 0, inCables: 0),
      ),
      rejected,
    );
    // The capacities.
    const s = UsbSetting(endpoints: [outEp, inEp]);
    expect(
      () => UsbFunction(
        acInterface: 0,
        msInterface: 1,
        settings: const [s, s, s],
      ),
      invalid,
    );
    expect(
      () =>
          one(const UsbSetting(endpoints: [outEp, outEp, outEp, outEp, outEp])),
      invalid,
    );
    expect(
      () => one(
        const UsbSetting(
          endpoints: [
            UsbEndpoint(direction: UsbDirection.midiOut, number: 1, blocks: []),
          ],
        ),
      ),
      invalid,
    );
    // Values out of range.
    expect(() => UsbFunction(acInterface: 256, msInterface: 1), invalid);
    expect(
      () => one(
        const UsbSetting(
          endpoints: [UsbEndpoint(direction: UsbDirection.midiOut, number: 16)],
        ),
      ),
      invalid,
    );
    expect(
      () => one(
        UsbSetting(endpoints: const [outEp], blocks: [b22(firstGroup: 16)]),
      ),
      invalid,
    );
    expect(() => gtbRequest(1, 1, 0x10000), invalid);
    expect(() => gtbBlocks([256]), invalid);
  });
}
