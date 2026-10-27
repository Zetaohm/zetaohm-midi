# /// script
# requires-python = ">=3.10"
# dependencies = ["mido", "python-rtmidi"]
# ///
"""Host side of mk20dx-usb-midi: wait for the device ports, send MIDI 1.0 messages, and check
that each one comes back.

    uv run host/echo.py [--port NAME] [--timeout SECONDS]

The host MIDI 1.0 API (CoreMIDI, ALSA, WinMM) is used, so the same check runs when the host
selects Alternate Setting 0 (USB-MIDI 1.0) or Alternate Setting 1 (USB MIDI 2.0): the operating
system translates between MIDI 1.0 messages and UMPs.
"""

import argparse
import sys
import time

import mido

MESSAGES = [
    mido.Message("note_on", channel=0, note=60, velocity=100),
    mido.Message("note_off", channel=0, note=60, velocity=0),
    mido.Message("control_change", channel=5, control=7, value=99),
    mido.Message("program_change", channel=9, program=12),
    mido.Message("pitchwheel", channel=15, pitch=-1234),
    mido.Message("sysex", data=[0x7D, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]),
]


def find(names, part):
    return [n for n in names if part in n]


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", default="zetaohm-midi")
    ap.add_argument("--timeout", type=float, default=30.0)
    args = ap.parse_args()

    # mido makes a new rtmidi client on each call, so a CoreMIDI client sees a device that
    # appears after the script starts (see mk20dx-testsuite host/usb_midi_send.py).
    deadline = time.monotonic() + args.timeout
    while True:
        outs = find(mido.get_output_names(), args.port)
        ins = find(mido.get_input_names(), args.port)
        if outs and ins:
            break
        if time.monotonic() > deadline:
            print(f"no port with {args.port!r}: outputs {mido.get_output_names()}, "
                  f"inputs {mido.get_input_names()}", file=sys.stderr)
            return 1
        time.sleep(0.2)
    print(f"output {outs[0]!r}, input {ins[0]!r}")

    failed = 0
    with mido.open_input(ins[0]) as inp, mido.open_output(outs[0]) as out:
        time.sleep(0.5)
        for _ in inp.iter_pending():
            pass
        for msg in MESSAGES:
            out.send(msg)
            got = None
            end = time.monotonic() + 1.0
            while time.monotonic() < end and got is None:
                for m in inp.iter_pending():
                    got = m
                    break
                time.sleep(0.002)
            ok = got is not None and got.bytes() == msg.bytes()
            failed += not ok
            print(f"{'ok  ' if ok else 'FAIL'} sent {msg.hex()}  got {got.hex() if got else None}")
    print("all messages came back" if not failed else f"{failed} of {len(MESSAGES)} failed")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
