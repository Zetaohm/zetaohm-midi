#!/usr/bin/env python3
"""Size matrix of the library jobs (OPT-052, step 1).

Builds tools/size-probe once for each configuration (thumbv7em-none-eabi, the release profile of
the probe: opt-level "s", fat LTO, one codegen unit, panic abort) and gives the flash and RAM of
each job. The cost of a job is the probe with its features minus a reference build: `base` (no
feature) or `usb_core` (the usb-device core with no class), so a USB number is the class alone.

Flash = .text + .rodata + .data. RAM = .data + .bss + .uninit (static RAM). Stack = the peak stack of the
build: the deepest call chain from a probe entry function (`size_probe::<job>::run`, `init`, `on_event`,
`poll`), with the frame sizes of scripts/stack-check.py; it is the whole build, not a difference. Each job has a budget (exit 1 when
it is over) and the number of the smallest other library for the same job, measured with
tools/lib-compare on 2026-10-19 and 2026-10-20. The goal of the plan is a ratio of 1.0 for the
smallest build of each job.

    python3 scripts/size-check.py [--no-build] [--only NAME,NAME] [--markdown docs/sizes.md]

--markdown writes the published size table (OPT-052, rule 4).
"""

import argparse
import os
import shutil
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PROBE = os.path.join(ROOT, "tools", "size-probe")
TARGET = "thumbv7em-none-eabi"
ELF = os.path.join(PROBE, "target", TARGET, "release", "size-probe")

# name: (features, reference build, flash budget, RAM budget, other library flash, other library RAM, which)
# Budgets: the measurement plus about 2% (2026-10-20, after the DEC-018 word paths). Lower a
# budget when a change lands.
JOBS = {
    "decode_lean": ("rx_lean", "base", 784, 16, 808, None, "Rust midi2 0.11.1, CV2 and System"),
    "decode_all": ("rx_cv2", "base", 6272, 16, 2544, 208, "midi2 C99 dispatch"),
    "decode_cv2_system": ("rx_cv2_tier", "base", 3008, 16, 2544, 208, "midi2 C99 dispatch"),
    "midi2_to_midi1": ("down", "base", 6736, 16, 680, 40, "AM_MIDI2.0Lib umpToMIDI1Protocol"),
    "midi2_to_midi1_words": ("down_words", "base", 512, 16, 680, 40, "AM_MIDI2.0Lib umpToMIDI1Protocol"),
    "midi1_to_midi2": ("up", "base", 6640, 2096, None, 140, "AM_MIDI2.0Lib state (RAM)"),
    # midi1_to_midi2_lean: the typed translator with Midi1ChannelVoiceMessage::parse in place of
    # Ump::parse (all Message Types); RAM is the state of one Group.
    "midi1_to_midi2_lean": ("up_lean", "base", 1232, 136, None, 140, "AM_MIDI2.0Lib state (RAM)"),
    "midi1_to_midi2_words": ("up_words", "base", 976, 136, 212, 0, "midi2 C99 midi2_msg_mt2_to_mt4 (stateless, a subset)"),
    "din_in": ("bytes1", "base", 1072, 32, 852, 52, "midi2 C99 midi2_conv"),
    "din_in_core": ("bytes_core", "base", 2416, 32, 852, 52, "midi2 C99 midi2_conv"),
    "din_in_feed": ("bytes_feed", "base", 704, 32, 852, 52, "midi2 C99 midi2_conv"),
    "din_out_each": ("bytes_out_each", "base", 1152, 16, 1160, 104, "AM_MIDI2.0Lib umpToBytestream"),
    "din_out": ("bytes_out", "base", 1424, 16, 1160, 104, "AM_MIDI2.0Lib umpToBytestream"),
    "usb_class": ("usb2", "usb_core", 17488, 672, 3556, 666, "tusb_ump class driver (object)"),
    # ci_device: Discovery and management, no category feature (DEC-017), with a 64 B reply buffer
    # (tests/reply_len.rs of zmidi-ci); pe_engine includes the PE message framing of zmidi-ci (feature
    # property-exchange), the 448 B more of a 512 B reply buffer that a PE reply needs, and the
    # Initiator role of zmidi-ci that the engine needs (feature initiator, DEC-CI-001).
    "usb_class_fixed": ("usb2_fixed", "usb_core", 8192, 480, 3556, 666, "tusb_ump class driver (object)"),
    "usb_class_lean": ("usb2_lean", "usb_core", 4976, 384, 3556, 666, "tusb_ump class driver (object), with Alt 0"),
    # The embassy-usb adapter, with a stub driver (tools/size-probe/src/edriver.rs); emb_core is the
    # embassy-usb device with no class.
    "emb_class": ("emb2", "emb_core", 19936, 544, 3556, 666, "tusb_ump class driver (object)"),
    "emb_class_fixed": ("emb2_fixed", "emb_core", 9424, 368, 3556, 666, "tusb_ump class driver (object)"),
    "emb_class_lean": ("emb2_lean", "emb_core", 6240, 272, 3556, 666, "tusb_ump class driver (object), with Alt 0"),
    "ci_device": ("ci", "base", 6992, 696, 4615, 736, "midi2 C99 CI (with its PE Get and Set)"),
    # ci_device_initiator: ci_device with the Initiator role (feature initiator of zmidi-ci, DEC-CI-001).
    "ci_device_initiator": ("ci_init", "base", 8192, 768, 4615, 736, "midi2 C99 CI (with its PE Get and Set)"),
    "pe_engine": ("ci,pe", "ci", 26192, 3056, 1500, None, "midi2 C99 PE part (about)"),
    "pe_engine_no_subscriptions": ("ci,pe_nosub", "ci", 22608, 2576, 1500, None, "midi2 C99 PE part (about)"),
    # pe_one_chunk: 2026-10-29, about +64 B: the Responder stores the PE Capabilities of the peer, for
    # Subscription messages in both roles (DEC-CI-001).
    "pe_one_chunk": ("ci,pe_one_chunk", "ci", 5008, 752, 1500, None, "midi2 C99 PE part (about)"),
    "pe_one_chunk_flat": ("ci,pe_one_chunk_flat", "ci", 4416, 752, 1500, None, "midi2 C99 PE part (about)"),
    # ci_one_chunk: the same job as the midi2 C99 number (CI with PE Get and Set), OPT-049.
    "ci_one_chunk": ("ci,pe_one_chunk", "base", 11856, 1448, 4615, 736, "midi2 C99 CI (with its PE Get and Set)"),
    "ci_one_chunk_flat": ("ci,pe_one_chunk_flat", "base", 11264, 1448, 4615, 736, "midi2 C99 CI (with its PE Get and Set)"),
    # ci_like_c: ci_one_chunk with the categories that midi2 C99 CI answers (docs/c-comparison.md).
    "ci_like_c": ("ci,ci_c", "base", 15248, 1504, 4615, 736, "midi2 C99 CI (with its PE Get and Set)"),
    "ci_like_c_flat": ("ci,ci_c_flat", "base", 14672, 1504, 4615, 736, "midi2 C99 CI (with its PE Get and Set)"),
    "full": ("usb2,rx_lean,tx_cv2,flex,stream,bytes1,bytes_out,down,ci,pe", "base", 63792, 5392,
             None, None, "no single library does all of it"),
    # BLE-MIDI, Network MIDI 2.0 and the file formats (2026-10-27, OPT-044). No other library measured yet.
    "ble_rx": ("ble_rx", "base", 9328, 192, None, None, "not measured yet"),
    "ble_rx_words": ("ble_rx_words", "base", 3584, 192, None, None, "not measured yet"),
    "ble_tx": ("ble_tx", "base", 2976, 80, None, None, "not measured yet"),
    "net_client": ("net_client", "base", 29856, 4448, None, None, "not measured yet"),
    "net_host": ("net_host", "base", 28400, 4448, None, None, "not measured yet"),
    "smf_read": ("smf_read", "base", 1216, 16, None, None, "not measured yet"),
    "clip_read": ("clip_read", "base", 3344, 16, None, None, "not measured yet"),
    "clip_write": ("clip_write", "base", 2896, 544, None, None, "not measured yet"),
}


# What each job is, for the published table (docs/sizes.md).
WHAT = {
    "decode_lean": "UMP decode, the caller selects the types (MIDI 2.0 Channel Voice and System)",
    "decode_all": "`Ump::parse` with all Message Types",
    "decode_cv2_system": "`Ump::parse` with the features `channel-voice2` and `system-common` only",
    "midi2_to_midi1": "MIDI 2.0 to 1.0 Protocol, typed (`Ump::parse`, `to_midi1`)",
    "midi2_to_midi1_words": "MIDI 2.0 to 1.0 Protocol, word API (`midi2_to_midi1`)",
    "midi1_to_midi2": "MIDI 1.0 to 2.0 Protocol (`Ump::parse`, all 16 Groups)",
    "midi1_to_midi2_lean": "MIDI 1.0 to 2.0 Protocol, one Group, `Midi1ChannelVoiceMessage::parse`",
    "midi1_to_midi2_words": "MIDI 1.0 to 2.0 Protocol, word API (`Midi1ToMidi2Group::translate_words`), one Group",
    "din_in": "DIN bytes to UMP, value form (`BytesToUmp::push`, MIDI 1.0 Protocol output)",
    "din_in_core": "DIN bytes to UMP, `BytesToUmpCore::push_with`, each packet read as words",
    "din_in_feed": "DIN bytes to UMP, callback form (`BytesToUmpCore::feed`)",
    "din_out_each": "UMP to DIN bytes, callback form (`UmpToBytes::write_each`)",
    "din_out": "UMP to DIN bytes, value form (`UmpToBytes::write`)",
    "usb_class": "USB MIDI 2.0 class on usb-device, run-time `FunctionConfig`",
    "usb_class_fixed": "USB MIDI 2.0 class on usb-device, fixed function from the build script",
    "usb_class_lean": "usb_class_fixed with the feature alt0 off: empty Alternate Setting 0, no USB-MIDI 1.0 data path",
    "emb_class": "USB MIDI 2.0 class on embassy-usb, run-time `FunctionConfig`",
    "emb_class_fixed": "USB MIDI 2.0 class on embassy-usb, fixed function from the build script",
    "emb_class_lean": "emb_class_fixed with the feature alt0 off: empty Alternate Setting 0, no USB-MIDI 1.0 data path",
    "ci_device": "MIDI-CI Device, responder only: Discovery and management, no category feature",
    "ci_device_initiator": "MIDI-CI Device with the Initiator role (feature `initiator`), no category feature",
    "pe_engine": "Property Exchange engine (`zmidi-ci-pe`), on top of `ci_device`",
    "pe_engine_no_subscriptions": "Property Exchange engine with no Subscription slots (S=0)",
    "pe_one_chunk": "one-chunk Property Exchange Responder (`one_chunk::respond`)",
    "pe_one_chunk_flat": "pe_one_chunk with respond_flat: the flat Header Data reader of M2-103 7.1.1 (DEC-PE-006)",
    "ci_one_chunk": "MIDI-CI Device with the one-chunk PE Responder (the job of the C number)",
    "ci_one_chunk_flat": "ci_one_chunk with respond_flat (DEC-PE-006)",
    "ci_like_c": "ci_one_chunk with Profile Inquiry, Set Profile On/Off and Process Inquiry: the categories of midi2 C99 CI",
    "ci_like_c_flat": "ci_like_c with respond_flat (DEC-PE-006)",
    "full": "all of the UMP, DIN, USB, CI and PE jobs above in one firmware",
    "ble_rx": "BLE-MIDI packet reader (`PacketReader::read`, typed `Ump`, MIDI 2.0 Protocol output)",
    "ble_rx_words": "BLE-MIDI packet reader, word form (`PacketReader::read_words`, DEC-BLE-002)",
    "ble_tx": "BLE-MIDI packet writer (`PacketWriter::write_packet`, ATT MTU 23)",
    "net_client": "Network MIDI 2.0 Client, one session, no authentication: connect, receive, poll, send",
    "net_host": "Network MIDI 2.0 Host, one session, no authentication: receive, poll, send",
    "smf_read": "Standard MIDI File reader, all tracks (`SmfReader`, `TrackReader`)",
    "clip_read": "MIDI Clip File reader (`ClipReader`, typed `Ump`)",
    "clip_write": "MIDI Clip File writer (`ClipWriter`, 512 B buffer)",
}


# Peak stack of the other library for the same job, where measured (the C libraries put their reply
# buffers on the stack): midi2 C99 CI, main 48 B + onsx 1,144 B (ci_handle_pe_get 1,080 B), 2026-10-22.
OTHER_STACK = {"ci_device": 1192, "ci_device_initiator": 1192, "ci_one_chunk": 1192, "ci_like_c": 1192}


def peak_stack(elf):
    """The deepest call chain from a probe entry function, with the parser of stack-check.py."""
    import importlib.util
    import re
    from functools import lru_cache

    spec = importlib.util.spec_from_file_location("stack_check", os.path.join(ROOT, "scripts", "stack-check.py"))
    sc = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(sc)
    dump = subprocess.run(sc.objdump() + ["-d", "--no-show-raw-insn", "--demangle", elf],
                          capture_output=True, text=True, check=True).stdout
    frames, calls, tails = sc.parse(dump)
    active = set()

    @lru_cache(maxsize=None)
    def depth(f):
        if f in active:
            return 0
        active.add(f)
        below = max((depth(c) for c in calls.get(f, ())), default=0)
        own = frames.get(f, 0) + below
        own = max([own] + [depth(c) for c in tails.get(f, ())])
        active.discard(f)
        return own

    sys.setrecursionlimit(100000)
    entries = [f for f in frames if re.match(r"size_probe::.*::(run|init|on_event|poll)$", f)]
    return max((depth(f) for f in entries), default=0)


def tool(name):
    if shutil.which(name):
        return name
    sysroot = subprocess.run(["rustc", "--print", "sysroot"], capture_output=True, text=True).stdout.strip()
    for dirpath, _, files in os.walk(os.path.join(sysroot, "lib", "rustlib")):
        if name in files:
            return os.path.join(dirpath, name)
    sys.exit(f"size-check: no {name} (rustup component add llvm-tools)")


def measure(features, build):
    if build:
        cmd = ["cargo", "build", "--quiet", "--release", "--target", TARGET]
        if features != "base":
            cmd += ["--features", features]
        subprocess.run(cmd, cwd=PROBE, check=True)
    out = subprocess.run([tool("llvm-size"), "-A", ELF], capture_output=True, text=True, check=True).stdout
    sec = {}
    for line in out.splitlines():
        parts = line.split()
        if len(parts) >= 2 and parts[0].startswith(".") and parts[1].isdigit():
            sec[parts[0]] = int(parts[1])
    flash = sec.get(".text", 0) + sec.get(".rodata", 0) + sec.get(".data", 0)
    ram = sec.get(".data", 0) + sec.get(".bss", 0) + sec.get(".uninit", 0)
    return flash, ram, peak_stack(ELF)


def ratio(ours, other):
    return f"{ours / other:5.1f}x" if other else "    -"


HEADER = """# Flash and RAM of each job

Generated by `python3 scripts/size-check.py --markdown docs/sizes.md`; do not edit by hand. CI checks
each number against its budget in `scripts/size-check.py` (OPT-052, rule 4). The probe
features of each job are in `JOBS` in that script.

Method: `tools/size-probe` for `thumbv7em-none-eabi` (Cortex-M4), release profile of the probe
(opt-level "s", fat LTO, one codegen unit, panic abort). A job is the probe with its features minus a
reference build: `base` (no feature) or, for the USB class jobs, the USB device core with no class
(`usb_core` for usb-device 0.3, `emb_core` for embassy-usb 0.6, both with a stub driver). Flash is
`.text + .rodata + .data`, RAM is `.data + .bss + .uninit` (static RAM, buffers that the probe keeps in
statics included). Stack is the peak stack of the whole job build (the deepest call chain from a probe entry
function, frame sizes as in `scripts/stack-check.py`), not a difference. The C libraries keep some buffers on
the stack (midi2 C99 CI: about 1 KB for a PE reply), so compare static RAM and stack together.

The other library is the smallest C or Rust library for the same job, measured with
`tools/lib-compare` (same compiler flags for size, same target). Ratio = this library / other library;
the goal is 1.0x for the smallest build of each job (OPT-052).

| Job | What | Flash (B) | Other (B) | Ratio | RAM (B) | Other (B) | Ratio | Stack (B) | Other (B) | Other library |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---|
"""


def write_markdown(path, rows):
    def num(v):
        return f"{v:,}" if v is not None else "-"

    def rat(ours, other):
        return f"{ours / other:.1f}x" if other else "-"

    lines = [HEADER]
    for name, features, ref, flash, ram, st, of, orr, ost, which in rows:
        lines.append(
            f"| `{name}` | {WHAT.get(name, features)} | {num(flash)} | {num(of)} | {rat(flash, of)} "
            f"| {num(ram)} | {num(orr)} | {rat(ram, orr)} | {num(st)} | {num(ost)} | {which} |\n"
        )
    with open(os.path.join(ROOT, path) if not os.path.isabs(path) else path, "w") as f:
        f.write("".join(lines))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--no-build", action="store_true", help="measure the last build only (one job)")
    ap.add_argument("--only", help="comma-separated job names")
    ap.add_argument("--markdown", help="write the size table as Markdown to this file")
    a = ap.parse_args()
    names = a.only.split(",") if a.only else list(JOBS)
    cache = {}

    def get(features):
        if features not in cache:
            cache[features] = measure(features, not a.no_build)
        return cache[features]

    print(f"{'job':<20} {'flash':>7} {'budget':>7} {'other':>6} {'ratio':>6}  {'RAM':>6} {'budget':>6} {'other':>5} {'ratio':>6}  {'stack':>6} {'other':>5}  other library")
    failed = []
    rows = []
    for name in names:
        features, ref, fb, rb, of, orr, which = JOBS[name]
        f1, r1, st = get(features)
        f0, r0, _ = get(ref)
        flash, ram = f1 - f0, r1 - r0
        ost = OTHER_STACK.get(name)
        rows.append((name, features, ref, flash, ram, st, of, orr, ost, which))
        print(f"{name:<20} {flash:7d} {fb:7d} {of or '-':>6} {ratio(flash, of):>6}  {ram:6d} {rb:6d} {orr or '-':>5} {ratio(ram, orr):>6}  {st:6d} {ost or '-':>5}  {which}")
        if flash > fb:
            failed.append(f"{name}: flash {flash} B, budget {fb} B")
        if ram > rb:
            failed.append(f"{name}: RAM {ram} B, budget {rb} B")
    for f in failed:
        print(f"size-check: FAIL {f}", file=sys.stderr)
    if a.markdown:
        write_markdown(a.markdown, rows)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
