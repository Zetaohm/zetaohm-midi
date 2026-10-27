// sx <bytes> <mode>: mode "sysex" = MIDISendSysex, "paced" = 1000-byte MIDISend parts every 50 ms.
import CoreMIDI
import Foundation
let n = Int(CommandLine.arguments[1])!, mode = CommandLine.arguments[2]
func ep(_ count: Int, _ get: (Int) -> MIDIEndpointRef) -> MIDIEndpointRef {
  for i in 0..<count { let e = get(i); var s: Unmanaged<CFString>?; MIDIObjectGetStringProperty(e, kMIDIPropertyDisplayName, &s)
    if (s?.takeRetainedValue() as String? ?? "").contains("zetaohm-midi Bluetooth") { return e } }
  print("no endpoint"); exit(1)
}
let dst = ep(MIDIGetNumberOfDestinations(), MIDIGetDestination), src = ep(MIDIGetNumberOfSources(), MIDIGetSource)
var client = MIDIClientRef(), inPort = MIDIPortRef(), outPort = MIDIPortRef()
MIDIClientCreateWithBlock("sx" as CFString, &client, nil)
var got = [UInt8](); let lock = NSLock()
MIDIInputPortCreateWithBlock(client, "in" as CFString, &inPort) { list, _ in
  var p = list.pointee.packet
  for _ in 0..<list.pointee.numPackets {
    let len = Int(p.length); withUnsafeBytes(of: p.data) { b in lock.lock(); got += b.prefix(len); lock.unlock() }
    p = MIDIPacketNext(&p).pointee
  }
}
MIDIPortConnectSource(inPort, src, nil); MIDIOutputPortCreate(client, "out" as CFString, &outPort)
let data: [UInt8] = [0xF0] + (0..<n).map { UInt8($0 % 128) } + [0xF7]
let buf = UnsafeMutableBufferPointer<UInt8>.allocate(capacity: data.count); _ = buf.initialize(from: data)
var req = MIDISysexSendRequest(destination: dst, data: UnsafePointer(buf.baseAddress!), bytesToSend: UInt32(data.count), complete: false, reserved: (0, 0, 0), completionProc: { _ in }, completionRefCon: nil)
let t0 = Date()
if mode == "sysex" { MIDISendSysex(&req) } else {
  var i = 0
  while i < data.count {
    let k = min(1000, data.count - i)
    let chunk = Array(data[i..<(i + k)])
    var raw = [UInt8](repeating: 0, count: 2048)
    raw.withUnsafeMutableBytes { rb in
      let l = rb.baseAddress!.assumingMemoryBound(to: MIDIPacketList.self); var q = MIDIPacketListInit(l)
      q = MIDIPacketListAdd(l, 2048, q, 0, k, chunk); MIDISend(outPort, dst, l)
    }
    i += k; usleep(50_000)
  }
}
while Date().timeIntervalSince(t0) < 40 { lock.lock(); let c = got.count; lock.unlock(); if c >= data.count { break }; usleep(20_000) }
lock.lock(); let g = got; lock.unlock()
let echo = g.filter { $0 < 0xF8 }
print("\(mode) \(n): echo \(echo.count) of \(data.count) bytes, equal \(echo == data), \(String(format: "%.2f", Date().timeIntervalSince(t0))) s")
