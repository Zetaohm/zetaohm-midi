"""Sends the "boot" SysEx (F0 7D 62 6F 6F 74 F7) to the esp32s3-usb-midi firmware: it restarts in
the ROM download mode. `uv run --with mido --with python-rtmidi host/boot.py [PORT]`."""

import sys
import time

import mido

part = sys.argv[1] if len(sys.argv) > 1 else "zetaohm-midi echo"
names = [n for n in mido.get_output_names() if part in n]
if not names:
    sys.exit(f"no MIDI output with {part!r}: {mido.get_output_names()}")
with mido.open_output(names[0]) as out:
    out.send(mido.Message("sysex", data=[0x7D] + list(b"boot")))
    # CoreMIDI drops a message that is still queued when the port closes.
    time.sleep(0.5)
print(f"boot command sent to {names[0]!r}")
