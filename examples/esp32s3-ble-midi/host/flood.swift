// flood <bytes>...: each SysEx of that many data bytes in full packets, as fast as CoreBluetooth
// allows; checks that the echo is the same.
import CoreBluetooth
import Foundation
let SVC = CBUUID(string: "03B80E5A-EDE8-4B33-A751-6CE34EC4C700")
let CHR = CBUUID(string: "7772E5DB-3868-4112-A1A9-F2669D106BF3")
var sizes = CommandLine.arguments.dropFirst().compactMap { Int($0) }
class D: NSObject, CBCentralManagerDelegate, CBPeripheralDelegate {
  var p: CBPeripheral!; var c: CBCharacteristic!
  var queue: [[UInt8]] = []; var echo: [UInt8] = []; var sent: [UInt8] = []; var t0 = Date(); var ts = false
  func centralManagerDidUpdateState(_ m: CBCentralManager) { if m.state == .poweredOn { m.scanForPeripherals(withServices: [SVC]) } }
  func centralManager(_ m: CBCentralManager, didDiscover p: CBPeripheral, advertisementData a: [String: Any], rssi r: NSNumber) {
    m.stopScan(); self.p = p; p.delegate = self; m.connect(p) }
  func centralManager(_ m: CBCentralManager, didConnect p: CBPeripheral) { p.discoverServices([SVC]) }
  func peripheral(_ p: CBPeripheral, didDiscoverServices e: Error?) { p.discoverCharacteristics([CHR], for: p.services![0]) }
  func peripheral(_ p: CBPeripheral, didDiscoverCharacteristicsFor s: CBService, error e: Error?) {
    c = s.characteristics![0]; p.setNotifyValue(true, for: c) }
  func peripheral(_ p: CBPeripheral, didUpdateNotificationStateFor c: CBCharacteristic, error e: Error?) { next() }
  func next() {
    guard !sizes.isEmpty else { print("done"); exit(0) }
    let n = sizes.removeFirst()
    let len = p.maximumWriteValueLength(for: .withoutResponse)
    sent = [0xF0] + (0..<n).map { UInt8($0 % 128) } + [0xF7]; echo = []
    queue = []; var i = 0
    while i < sent.count {
      var pk: [UInt8] = [0x80]
      if i == 0 { pk.append(0x80) }
      let room = len - pk.count - 1
      let k = min(room, sent.count - i)
      for b in sent[i..<(i + k)] { if b == 0xF7 { pk.append(0x80) }; pk.append(b) }
      i += k; queue.append(pk)
    }
    t0 = Date(); pump()
    let total = sent.count
    func check(_ left: Int) {
      DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) {
        if self.echo.count >= total || left == 0 {
          let dt = Date().timeIntervalSince(self.t0)
          print("\(n): \(self.queue.isEmpty ? "sent" : "NOT sent"), echo \(self.echo.count) of \(total) bytes, equal \(self.echo == self.sent), \(String(format: "%.2f", dt)) s")
          self.next()
        } else { check(left - 1) }
      }
    }
    check(40)
  }
  func pump() {
    while !queue.isEmpty && p.canSendWriteWithoutResponse { p.writeValue(Data(queue.removeFirst()), for: c, type: .withoutResponse) }
  }
  func peripheralIsReady(toSendWriteWithoutResponse p: CBPeripheral) { pump() }
  func peripheral(_ p: CBPeripheral, didUpdateValueFor c: CBCharacteristic, error e: Error?) {
    // strip header and timestamp bytes: a byte with bit 7 set after a non-timestamp is a timestamp
    let v = [UInt8](c.value ?? Data()); ts = false
    for x in v.dropFirst() { if x >= 0x80 && !ts { ts = true; continue }; ts = false; echo.append(x) }
  }
}
let d = D(); let m = CBCentralManager(delegate: d, queue: nil)
RunLoop.main.run(until: Date().addingTimeInterval(120))
