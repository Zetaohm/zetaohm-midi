// BLE-MIDI v1.0 packets (REQ-DART-BLE-001): sections 7 and 8.
import 'package:test/test.dart';
import 'package:zetaohm_midi/zetaohm_midi.dart';

int hdr(int t) => 0x80 | (t >> 7);
int ts(int t) => 0x80 | (t & 0x7F);

void main() {
  setUpAll(initWeb);

  test('reader of section 7', () {
    final r = BleReader();
    final a = r.read([
      hdr(0x0A55),
      ts(0x0A55),
      0x90,
      0x3C,
      0x64,
      ts(0x0A57),
      0x80,
      0x3C,
      0x00,
    ]);
    expect(a.umps, [
      const BleUmp(0x0A55, [0x20903C64]),
      const BleUmp(0x0A57, [0x20803C00]),
    ]);
    expect(a.dropped, 0);
    expect(
      r.read([hdr(5), ts(5), 0x90, 0x3C, 0x64, 0x40, 0x64]).umps[1],
      const BleUmp(5, [0x20904064]),
    );
    final b = r.read([0x10, 0x90, 0x3C, 0x64]);
    expect([b.umps.length, b.dropped], [0, 1]);
  });

  test('writer of section 7 with running status', () {
    final w = BleWriter();
    expect(w.packetLen, 20);
    var r = w.writePacket([
      const BleUmp(0x0A55, [0x20903C64]),
    ]);
    expect(r.packet, [hdr(0x0A55), ts(0x0A55), 0x90, 0x3C, 0x64]);
    expect(r.taken, 1);
    w.reset(); // times never decrease (section 7); start a new stream
    r = w.writePacket([
      const BleUmp(5, [0x20903C64]),
      const BleUmp(5, [0x20904064]),
    ]);
    expect(r.packet, [hdr(5), ts(5), 0x90, 0x3C, 0x64, 0x40, 0x64]);
    expect(r.taken, 2);
    r = w.writePacket([]);
    expect([r.packet.length, r.taken], [0, 0]);
  });

  test('SysEx over packets and back', () {
    const umps = [
      BleUmp(1, [0x30160001, 0x02030405]),
      BleUmp(1, [0x30260607, 0x08090A0B]),
      BleUmp(1, [0x30360C0D, 0x0E0F1011]),
    ];
    final w = BleWriter();
    final r = BleReader();
    final got = <List<int>>[];
    var left = umps.toList();
    for (var i = 0; i < 16; i++) {
      final p = w.writePacket(left);
      left = left.sublist(p.taken);
      if (p.packet.isEmpty) {
        break;
      }
      got.addAll(r.read(p.packet).umps.map((u) => u.words));
    }
    expect(got, [for (final u in umps) u.words]);
  });

  test('BLE bad values', () {
    expect(() => BleWriter(attMtu: 22), throwsA(isA<InvalidValueException>()));
    expect(() => BleReader(group: 16), throwsA(isA<InvalidValueException>()));
  });
}
