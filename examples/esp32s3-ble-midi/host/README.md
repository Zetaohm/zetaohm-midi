# Host tools for `esp32s3-ble-midi` (macOS)

Build each tool with `swiftc -O <tool>.swift -o bin/<tool>`. Bluetooth access is asked for on the first run.

| Tool | What it does |
|---|---|
| `connect` | Connects "zetaohm-midi" and gives it to CoreMIDI, with no Audio MIDI Setup click. Run it again after each flash. |
| `gatt` | A GATT test with no CoreMIDI: the properties (0x1E), the empty read, the T-011 packets and the echo (`check.py` compares them). |
| `flood <bytes>...` | Long SysEx in full packets as fast as CoreBluetooth allows, and checks the echo. |
| `sx <bytes> sysex\|paced` | Long SysEx through CoreMIDI (`MIDISendSysex`, or 1000-byte parts every 50 ms), and checks the echo. |

`connect` does what Audio MIDI Setup does (CoreAudioKit `AMSBTLEConnectionManager`, macOS 27): after the
CoreBluetooth connection it sets the dictionary property `BLE MIDI Remote Peripheral` on the driver's null device
(`is BLE MIDI null device` = 1) to `{BLE MIDI Device UUID, name, MIDI Local Peripheral = 0, MIDI Remote Peripheral
= 1}`, and then closes its own link. This is private macOS behavior and can change.
`MIDIBluetoothDriverActivateAllConnections()` does not connect the device.

`gatt` and `flood` need the peripheral not connected to CoreMIDI (one connection only). `gatt` and `flood` make a
connection with a 15 ms interval and a data length of 251; CoreMIDI uses 7.5 ms and 130.

The macOS Bluetooth MIDI driver drops data when a client sends more than about 14 kB at once
(`mido`/`rtmidi` give a long SysEx to one `MIDISend`); `MIDISendSysex` sends it at the `maxSysExSpeed` of the
endpoint (3125 bytes/s) with no loss.
