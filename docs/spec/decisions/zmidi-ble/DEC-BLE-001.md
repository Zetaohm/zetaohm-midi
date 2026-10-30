<a id="dec-ble-001"></a>

# DEC-BLE-001 — zmidi-ble readings of BLE-MIDI v1.0 (AMB-BLE-MIDI-01 to -11) and the codec API

**Status:** accepted

**Requirements:** [REQ-BLE-GATT-003](../../requirements/zmidi-ble.md#req-ble-gatt-003), [REQ-BLE-MSG-003](../../requirements/zmidi-ble.md#req-ble-msg-003), [REQ-BLE-MSG-004](../../requirements/zmidi-ble.md#req-ble-msg-004), [REQ-BLE-MSG-005](../../requirements/zmidi-ble.md#req-ble-msg-005), [REQ-BLE-MSG-006](../../requirements/zmidi-ble.md#req-ble-msg-006), [REQ-BLE-MSG-009](../../requirements/zmidi-ble.md#req-ble-msg-009), [REQ-BLE-PKT-002](../../requirements/zmidi-ble.md#req-ble-pkt-002), [REQ-BLE-PKT-004](../../requirements/zmidi-ble.md#req-ble-pkt-004), [REQ-BLE-PKT-005](../../requirements/zmidi-ble.md#req-ble-pkt-005), [REQ-BLE-RD-001](../../requirements/zmidi-ble.md#req-ble-rd-001), [REQ-BLE-SYX-006](../../requirements/zmidi-ble.md#req-ble-syx-006), [REQ-BLE-TS-003](../../requirements/zmidi-ble.md#req-ble-ts-003), [REQ-BLE-TS-005](../../requirements/zmidi-ble.md#req-ble-ts-005), [REQ-BLE-UMP-001](../../requirements/zmidi-ble.md#req-ble-ump-001), [REQ-BLE-UMP-002](../../requirements/zmidi-ble.md#req-ble-ump-002), [REQ-BLE-UMP-003](../../requirements/zmidi-ble.md#req-ble-ump-003)

Readings of BLE-MIDI v1.0 for the `zmidi-ble` codec (AMB-BLE-MIDI-01 to -11):

- Running Status (01, 02): inside one packet the reader keeps the running status of the most recent full Channel
  Voice or Mode message across System Common, System Real-Time and System Exclusive messages; the end of the packet
  clears it. The reader writes the status byte again for each message, so `zmidi-io` gets whole messages only. The
  writer (when running status is on, the default) omits a status byte only after a channel message with the same
  status in the same packet and with no system message between them.
- GATT properties (03): read, write, write without response and notify: 0x1E (Bluetooth Core Vol 3 Part G
  3.3.1.1 (Bluetooth Core section from memory; the project has no copy of the Bluetooth Core document, not checked)).
- Header bit 6 (04): the writer writes 0; the reader ignores it.
- Packet span (05): the writer ends a packet before a message whose time is 128 ms or more after the first message
  of the packet, so at most one wrap of timestampLow happens in a packet.
- Timestamps (06): non-decreasing. The writer takes a time in milliseconds for each UMP from the caller, uses its
  low 13 bits, and uses the preceding time for a time that is earlier.
- Continuation packets (07): a UMP that ends in a SysEx continuation packet gets the most recent timestamp of its
  SysEx message.
- Open SysEx (08): when the writer gets a message that is not SysEx7 and not System Real-Time while a SysEx
  message is open, it first ends the SysEx (timestamp byte and F7) and drops the later Continue and End data of that
  SysEx message.
- Malformed packets (09): the reader drops a packet with no header byte whole; it drops a data byte with no running
  status, a message cut by the end of the packet and a timestamp byte with no message, reports each as an error and
  reads the rest of the packet.
- Undefined status bytes (11): the reader passes them to `zmidi-io` (ignored there); the writer never writes them.

API shape:
- `PacketReader` reads one packet (`&[u8]`) and gives each UMP with its 13-bit timestamp, or an error, to a sink
  closure (no alloc), as zmidi-usb `MidiLink::read_with` does. It wraps a zmidi-io `BytesToUmp` with the caller's
  Group and Protocol (default Group 0, MIDI 1.0 Protocol).
- `PacketWriter` writes one packet from a `Peekable` iterator of UMPs with their times into a caller buffer, as
  zmidi-usb `MidiLink::write_packet` does. It takes the ATT_MTU (23 or more) and writes at most ATT_MTU - 3 bytes. It
  holds the bytes of at most one UMP between two packets (a SysEx that continues, or a message that did not fit).

## Context

Item 03: the lathoub Arduino-BLE-MIDI library for ESP32 sets read, write, write without response and notify
(src/hardware/BLEMIDI_ESP32.h, read 2026-10-19). The API follows the pattern of zmidi-usb (DEC-014, DEC-015).

## Consequences

- The reader gives UMPs on one Group; the writer ignores the Group (BLE-MIDI has one stream, Preface: no virtual
  cables). The caller routes by Group (zmidi-io `route`).
- A BLE host adapter only moves packets; the codec has all BLE-MIDI rules (DEC-004, DEC-016).
- The times of the reader are 13-bit sender times. The correlation with the receiver clock is outside the
  specification (section 9) and outside the codec.

## Source

ble-midi, m1, m2-104-um

## Codegen include

text, consequences
