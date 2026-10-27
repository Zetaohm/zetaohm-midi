// Connects the BLE-MIDI peripheral "zetaohm-midi" and gives it to CoreMIDI, as Audio MIDI Setup
// does (CoreAudioKit AMSBTLEConnectionManager): after the connection, it sets the dictionary
// property "BLE MIDI Remote Peripheral" on the Bluetooth driver's null device, then closes
// its own link; the driver keeps the connection. Usage: connect [name]
import CoreBluetooth
import CoreMIDI
import Foundation
let SVC = CBUUID(string: "03B80E5A-EDE8-4B33-A751-6CE34EC4C700")
let NAME = CommandLine.arguments.dropFirst().first ?? "zetaohm-midi"
func nullDevice() -> MIDIDeviceRef? {
  for i in 0..<MIDIGetNumberOfDevices() {
    let d = MIDIGetDevice(i); var v: Int32 = 0
    if MIDIObjectGetIntegerProperty(d, "is BLE MIDI null device" as CFString, &v) == noErr && v == 1 { return d }
  }
  return nil
}
func online(_ uuid: String) -> Bool {
  for i in 0..<MIDIGetNumberOfDevices() {
    let d = MIDIGetDevice(i); var u: Unmanaged<CFString>?; var off: Int32 = 1
    guard MIDIObjectGetStringProperty(d, "BLE MIDI Device UUID" as CFString, &u) == noErr else { continue }
    if (u!.takeRetainedValue() as String) == uuid { MIDIObjectGetIntegerProperty(d, kMIDIPropertyOffline, &off); return off == 0 }
  }
  return false
}
class D: NSObject, CBCentralManagerDelegate, CBPeripheralDelegate {
  var p: CBPeripheral?
  func centralManagerDidUpdateState(_ m: CBCentralManager) {
    guard m.state == .poweredOn else { return }
    m.scanForPeripherals(withServices: [SVC])
    DispatchQueue.main.asyncAfter(deadline: .now() + 15) { if self.p == nil { print("not found:", NAME); exit(1) } }
  }
  func centralManager(_ m: CBCentralManager, didDiscover p: CBPeripheral, advertisementData a: [String: Any], rssi r: NSNumber) {
    guard (a[CBAdvertisementDataLocalNameKey] as? String ?? p.name) == NAME, self.p == nil else { return }
    m.stopScan(); self.p = p; m.connect(p)
  }
  func centralManager(_ m: CBCentralManager, didConnect p: CBPeripheral) { p.delegate = self; p.discoverServices([SVC]) }
  func centralManager(_ m: CBCentralManager, didFailToConnect p: CBPeripheral, error: Error?) { print("connect failed", error as Any); exit(1) }
  func peripheral(_ p: CBPeripheral, didDiscoverServices e: Error?) {
    guard let s = p.services?.first else { print("no MIDI service"); exit(1) }
    p.discoverCharacteristics(nil, for: s)
  }
  func peripheral(_ p: CBPeripheral, didDiscoverCharacteristicsFor s: CBService, error e: Error?) {
    guard let null = nullDevice() else { print("no BLE MIDI null device"); exit(1) }
    let uuid = p.identifier.uuidString
    let dict: [String: Any] = ["BLE MIDI Device UUID": uuid, kMIDIPropertyName as String: p.name ?? NAME,
                               "MIDI Local Peripheral": 0, "MIDI Remote Peripheral": 1]
    let st = MIDIObjectSetDictionaryProperty(null, "BLE MIDI Remote Peripheral" as CFString, dict as CFDictionary)
    print("driver connect request", uuid, "status", st); fflush(stdout)
    var tries = 0
    Timer.scheduledTimer(withTimeInterval: 0.5, repeats: true) { t in
      tries += 1
      if online(uuid) { print("CoreMIDI device online"); fflush(stdout); t.invalidate()
        m.cancelPeripheralConnection(p)
        DispatchQueue.main.asyncAfter(deadline: .now() + 1) { exit(0) } }
      else if tries > 20 { print("CoreMIDI device not online after 10 s"); exit(1) }
    }
  }
}
var client = MIDIClientRef()
MIDIClientCreateWithBlock("zetaohm-connect" as CFString, &client) { _ in }
let d = D(); let m = CBCentralManager(delegate: d, queue: nil)
RunLoop.main.run(until: Date().addingTimeInterval(30))
print("timeout"); exit(1)
