// gatt: connects "zetaohm-midi" over GATT (no CoreMIDI), prints the characteristic properties,
// the read value and the write lengths, writes the test packets of T-011 and prints each
// notification. `gatt > out.txt; python3 check.py out.txt` compares the echo with the input.
import CoreBluetooth
import Foundation
let SVC = CBUUID(string: "03B80E5A-EDE8-4B33-A751-6CE34EC4C700")
let CHR = CBUUID(string: "7772E5DB-3868-4112-A1A9-F2669D106BF3")
func hex(_ d: Data) -> String { d.map { String(format: "%02X", $0) }.joined(separator: " ") }
func t0() -> (UInt8, UInt8) { let ms = UInt16(UInt64(Date().timeIntervalSince1970 * 1000) & 0x1FFF); return (0x80 | UInt8(ms >> 7), 0x80 | UInt8(ms & 0x7F)) }
class D: NSObject, CBCentralManagerDelegate, CBPeripheralDelegate {
  var p: CBPeripheral?; var c: CBCharacteristic?; var reads = 0
  func centralManagerDidUpdateState(_ m: CBCentralManager) { if m.state == .poweredOn { m.scanForPeripherals(withServices: [SVC]) } }
  func centralManager(_ m: CBCentralManager, didDiscover p: CBPeripheral, advertisementData a: [String: Any], rssi r: NSNumber) {
    m.stopScan(); self.p = p; p.delegate = self; m.connect(p); print("connect", p.name ?? "?")
  }
  func centralManager(_ m: CBCentralManager, didConnect p: CBPeripheral) { p.discoverServices([SVC]) }
  func centralManager(_ m: CBCentralManager, didDisconnectPeripheral p: CBPeripheral, error: Error?) { print("disconnected", error as Any) }
  func peripheral(_ p: CBPeripheral, didDiscoverServices e: Error?) { for s in p.services ?? [] { p.discoverCharacteristics([CHR], for: s) } }
  func peripheral(_ p: CBPeripheral, didDiscoverCharacteristicsFor s: CBService, error e: Error?) {
    guard let c = s.characteristics?.first else { print("no characteristic"); return }
    self.c = c
    print(String(format: "properties 0x%02X", c.properties.rawValue))
    print("max write without response", p.maximumWriteValueLength(for: .withoutResponse), "with response", p.maximumWriteValueLength(for: .withResponse))
    p.readValue(for: c)
  }
  func peripheral(_ p: CBPeripheral, didUpdateNotificationStateFor c: CBCharacteristic, error e: Error?) {
    print("notify on", c.isNotifying, e as Any); send()
  }
  func peripheral(_ p: CBPeripheral, didUpdateValueFor c: CBCharacteristic, error e: Error?) {
    let v = c.value ?? Data()
    if reads == 0 { reads = 1; print("read value:", v.count, "bytes", hex(v)); p.setNotifyValue(true, for: c); return }
    print("notify:", hex(v))
  }
  func w(_ b: [UInt8], _ t: CBCharacteristicWriteType = .withoutResponse) { print("write:", hex(Data(b))); p!.writeValue(Data(b), for: c!, type: t) }
  func send() {
    var (h, l) = t0()
    w([h, l, 0x90, 0x3C, 0x64])                                  // Note On
    (h, l) = t0(); w([h, l, 0x80, 0x3C, 0x00, 0x40, 0x00])       // Note Off with running status
    (h, l) = t0(); w([h, l, 0xB2, 0x07, 0x50, l, 0xE2, 0x00, 0x40], .withResponse) // CC, Pitch Bend; write with response
    (h, l) = t0(); w([h, l, 0xF8, l, 0xC1, 0x05])                // Clock (ignored), Program Change
    // SysEx of 40 data bytes in 2 packets, Clock inside it
    (h, l) = t0(); w([h, l, 0xF0, 0x7D] + (1...20).map { UInt8($0) } + [l, 0xF8])
    (h, l) = t0(); w([h] + (21...39).map { UInt8($0) } + [l, 0xF7])
    // long SysEx 300 bytes: echo must come back in more than one notification
    let big = [0xF0, 0x7D] + (0..<298).map { UInt8($0 % 128) } + [0xF7] as [UInt8]
    var i = 0; var first = true
    while i < big.count - 1 {
      (h, l) = t0(); let room = 100
      var pk: [UInt8] = first ? [h, l] : [h]; first = false
      let n = min(room, big.count - 1 - i); pk += big[i..<(i+n)]; i += n
      if i == big.count - 1 { pk += [l, 0xF7] }
      w(pk)
    }
    DispatchQueue.main.asyncAfter(deadline: .now() + 1.0) { (h, l) = t0(); self.w([h, l, 0xF3]) } // Song Select with no data: incomplete
    DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) { (h, l) = t0(); self.w([h, l, 0x99, 0x24, 0x7F]) }
  }
}
let d = D(); let m = CBCentralManager(delegate: d, queue: nil)
RunLoop.main.run(until: Date().addingTimeInterval(8))
if let p = d.p { m.cancelPeripheralConnection(p) }
RunLoop.main.run(until: Date().addingTimeInterval(1))
