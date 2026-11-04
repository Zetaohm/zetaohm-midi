# zetaohm-midi for Python

[![PyPI](https://img.shields.io/pypi/v/zetaohm-midi.svg)](https://pypi.org/project/zetaohm-midi/)
[![Python](https://img.shields.io/pypi/pyversions/zetaohm-midi.svg)](https://pypi.org/project/zetaohm-midi/)

MIDI 2.0 for Python: UMP, MIDI-CI, Network MIDI 2.0, BLE-MIDI, USB-MIDI and MIDI Clip Files. A
typed package over the Rust crates of [zetaohm-midi](../../README.md).

## Features

- UMP: `parse`, builders such as `midi2_note_on`, MIDI 2.0 → 1.0 translation, value scaling.
- MIDI 1.0 byte stream ↔ UMP: `BytesToUmp`, `UmpToBytes`.
- MIDI 1.0 on bytes: `Midi1Reader` and `Midi1Writer` give and take `Midi1Message` (System Exclusive
  whole); the receiver states `ModeState` (Modes 1 to 4), `ControllerState` (14-bit controllers, bank and
  program, Reset All Controllers as RP-015 gives) and `ParameterState` (RPN and NRPN, the RP-018 steps),
  and their writers such as `bank_program_messages` and `value_messages`.
- MIDI-CI: `CiDevice`, with Profiles, Property Exchange and Process Inquiry.
- Network MIDI 2.0: `NetHost` and `NetClient`, with authentication.
- USB-MIDI: `UsbLink`. BLE-MIDI: `BleReader` and `BleWriter`.
- USB descriptors: `UsbFunction` (with `UsbAlt0`, `UsbSetting`, `UsbEndpoint` and `UsbBlock`) takes
  the full description of a USB MIDI function and gives its descriptors (USB-MIDI v2.0 Appendix B
  order), its endpoints, the Group Terminal masks for `UsbLink`, and the Group Terminal Block requests
  (device side `answer_setup`, host side `gtb_request`, `gtb_total_length`, `gtb_blocks`). A broken
  rule raises `RejectedError` with the text of the rule.
- MIDI Clip File: `read_clip` and `ClipWriter`.
- Sans-IO, as the Rust crates: you own the sockets, the USB device and the BLE connection.
- Full type stubs (`py.typed`). Python 3.9 or later; one wheel per platform (stable ABI).

## Install

The package is not yet on PyPI. Build it from the repository with
[uv](https://docs.astral.sh/uv/) and a Rust toolchain:

```sh
cd bindings/python
uv sync
```

## Example

```python
import zetaohm_midi as zm

# Build and parse a MIDI 2.0 Note On.
words = zm.midi2_note_on(group=0, channel=0, note=60, velocity=0xC000)
ump = zm.parse(words)
print(hex(ump.message_type), ump.group, ump.channel, [hex(w) for w in ump.words])

# MIDI 1.0 bytes to MIDI 2.0 Protocol UMPs, and back to bytes.
din_in = zm.BytesToUmp(group=0, protocol=2)
print(din_in.feed(bytes([0x90, 0x3C, 0x64])))  # == [[0x40903C00, 0xC9240000]]

din_out = zm.UmpToBytes(running_status=True)
print(din_out.write([0x20903C64]).hex())  # 903c64

# MIDI 1.0 receiver: typed messages, the controller state and Pitch Bend Sensitivity (RPN 0).
reader, controllers, parameters = zm.Midi1Reader(), zm.ControllerState(), zm.ParameterState()
bend_range = 2 << 7  # 2 semitones, 0 cents
for m in reader.feed(bytes([0xB0, 0x65, 0x00, 0x64, 0x00, 0x06, 0x0C, 0xB0, 0x01, 0x40])):
    if m.kind == "channel_voice":
        print(controllers.receive(m))
        if (e := parameters.receive(m)) is not None and e.parameter == zm.Parameter(0, 0):
            bend_range = e.apply(bend_range)
print(bend_range >> 7)  # 12 semitones
```

## Errors

Every error is a `zetaohm_midi.MidiError`:

| Exception | When |
|---|---|
| `InvalidValueError` | An argument is out of range (for example a Group above 15). |
| `MalformedError` | Received data breaks the specification. |
| `RejectedError` | The operation is not possible in the current state. |

## Development

```sh
uv sync --locked
uv run pytest
uv run python -m pyright
uv run python -m ruff check
```

## License

Licensed under the [MIT license](LICENSE). Copyright (c) 2025-2026 Zetaohm LLC.
