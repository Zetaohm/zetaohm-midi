"""Asks the esp32s3-usb-midi firmware for its counters ("stat" SysEx, F0 7D 73 74 61 74 F7) and
prints the answer. `uv run --with mido --with python-rtmidi host/stat.py [PORT]`."""

import sys
import time

import mido

part = sys.argv[1] if len(sys.argv) > 1 else "zetaohm-midi echo"
out_name = next((n for n in mido.get_output_names() if part in n), None)
in_name = next((n for n in mido.get_input_names() if part in n), None)
if out_name is None or in_name is None:
    sys.exit(f"no MIDI port with {part!r}")
with mido.open_output(out_name) as out, mido.open_input(in_name) as inp:
    time.sleep(0.3)
    list(inp.iter_pending())
    out.send(mido.Message("sysex", data=[0x7D] + list(b"stat")))
    end = time.time() + 3
    while time.time() < end:
        for m in inp.iter_pending():
            if m.type == "sysex" and m.data[:1] == (0x7D,):
                print(bytes(m.data[1:]).decode())
                sys.exit(0)
        time.sleep(0.01)
sys.exit("no answer")
