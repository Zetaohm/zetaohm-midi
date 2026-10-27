#!/usr/bin/env python3
"""Speed check of the hot paths (docs/optimization-backlog.md, OPT-029).

Builds tools/speed-probe for thumbv7em-none-eabihf (the size-probe profile: opt-level "s", fat LTO),
runs it on the QEMU machine mps2-an386 (Cortex-M4F) with the plugin tools/speed-probe/qemu/insn_mark.c,
and gives the executed instructions of each bench and of each item (a message, a packet, a byte).
The counts are the same on each run; they change with rustc and with the library code.

    python3 scripts/speed-check.py [--no-build] [--markdown docs/speeds.md] [--profile BENCH]

--profile gives the functions that execute the instructions of one bench (the plugin counts each
translation block; a block is given to the function symbol at or below its address).

--hw LOG reads the log of examples/mk20dx-speed-bench (`cargo run --release` there, on the FLXS1):
the DWT cycles of each bench, and the cycles for each QEMU instruction (OPT-029 step 2).

Needs qemu-system-arm with plugin support (QEMU 4.2 or later), a C compiler, glib and the QEMU header
qemu-plugin.h. When no header is found (Ubuntu ships none), the script downloads the header of the
installed QEMU version from the QEMU repository on GitHub. Exit 1 when a bench is over its budget,
fails its output check or has no budget.
"""

import argparse
import os
import re
import shutil
import subprocess
import sys
import urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PROBE = os.path.join(ROOT, "tools", "speed-probe")
TARGET = "thumbv7em-none-eabihf"
ELF = os.path.join(PROBE, "target", TARGET, "release", "speed-probe")
OUT = os.path.join(PROBE, "target", "qemu")
PLUGIN_SRC = os.path.join(PROBE, "qemu", "insn_mark.c")

# Instructions of each bench (all items), default build. Set from the measurement with a margin of
# about 10 %, as the stack budgets, so that a large slowdown fails CI.
# Measured 2026-10-23 (rustc 1.98.1) after the OPT-030, OPT-036 and OPT-037 fixes, plus about 10 %, rounded up to 64.
BUDGETS = {
    "ump_parse_mixed": 7040,
    "ump_parse_all_types": 1088,
    "ump_translate_midi1_to_midi2_note": 9216,
    "ump_translate_midi1_to_midi2_words": 3712,
    "ump_translate_midi2_to_midi1_note": 26560,
    "ump_translate_midi2_to_midi1_words": 5696,
    "ump_sysex7_reassemble_1020": 40896,
    "io_bytes_to_ump_midi1_note": 24704,
    "io_bytes_to_ump_midi1_note_feed": 7424,
    "io_bytes_to_ump_midi2_note": 28608,
    "io_bytes_to_ump_midi2_note_feed": 11712,
    "io_ump_to_bytes_note_running": 2624,
    "usb_event_write_note": 11328,
    "usb_event_read_note_midi1": 11200,
    "usb_event_read_note_midi2": 25472,
    "usb_ump_words_to_bytes": 320,
    "usb_ump_bytes_to_words": 576,
    "ble_write_note": 23552,
    "ble_read_note_midi1": 22400,
    "file_clip_write_note": 21568,
    "file_clip_read_note": 20224,
    "net_client_send_note": 13184,
    "net_host_receive_note": 13184,
    "net_host3_receive_note_last_session": 13568,
    "ci_device_receive_discovery": 2432,
    "pe_get_transaction_400": 21312,
    "pe_header_parse_inquiry": 3904,
    "pe_header_parse_reply": 4928,
    "pe_mcoded7_encode_1020": 26816,
    "pe_mcoded7_decode_1020": 25728,
}


def run(cmd, **kw):
    return subprocess.run(cmd, check=True, text=True, capture_output=True, **kw)


def tool(*names):
    """The first of `names` on PATH, else in the llvm-tools of the rustc sysroot."""
    for n in names:
        p = shutil.which(n)
        if p:
            return p
    sysroot = run(["rustc", "--print", "sysroot"], cwd=PROBE).stdout.strip()
    for dirpath, _, files in os.walk(os.path.join(sysroot, "lib", "rustlib")):
        for n in names:
            if n in files:
                return os.path.join(dirpath, n)
    sys.exit(f"speed-check: none of {', '.join(names)} found (rustup component add llvm-tools)")


def qemu_version(qemu):
    out = run([qemu, "--version"]).stdout
    m = re.search(r"version (\d+\.\d+\.\d+)", out)
    if not m:
        sys.exit(f"speed-check: no version in {out!r}")
    return m.group(1)


def plugin_header(version):
    """The directory of qemu-plugin.h: an installed copy, else a download for `version`."""
    for d in (os.environ.get("QEMU_PLUGIN_INCLUDE"), "/opt/homebrew/include", "/usr/local/include",
              "/usr/include", "/usr/include/qemu"):
        if d and os.path.exists(os.path.join(d, "qemu-plugin.h")):
            return d
    d = os.path.join(OUT, f"include-{version}")
    h = os.path.join(d, "qemu-plugin.h")
    if not os.path.exists(h):
        os.makedirs(d, exist_ok=True)
        url = f"https://raw.githubusercontent.com/qemu/qemu/v{version}/include/qemu/qemu-plugin.h"
        print(f"speed-check: downloading {url}", file=sys.stderr)
        with urllib.request.urlopen(url, timeout=60) as r, open(h, "wb") as f:
            f.write(r.read())
    return d


def build_plugin(qemu):
    os.makedirs(OUT, exist_ok=True)
    so = os.path.join(OUT, "insn_mark.so")
    inc = plugin_header(qemu_version(qemu))
    glib = run(["pkg-config", "--cflags", "glib-2.0"]).stdout.split()
    cmd = [tool("cc", "gcc", "clang"), "-shared", "-fPIC", "-O2", "-Wall", f"-I{inc}", *glib,
           "-o", so, PLUGIN_SRC]
    if sys.platform == "darwin":
        cmd.insert(1, "-undefined")
        cmd.insert(2, "dynamic_lookup")
    run(cmd)
    return so


def mark_address(elf):
    nm = tool("llvm-nm", "rust-nm", "arm-none-eabi-nm")
    for line in run([nm, elf]).stdout.splitlines():
        parts = line.split()
        if len(parts) == 3 and parts[2] == "speed_mark":
            return int(parts[0], 16)
    sys.exit("speed-check: no speed_mark symbol")


def bench_names():
    """The bench names in run order, from the sources (the probe prints the same names)."""
    names = ["calibrate"]
    src = os.path.join(PROBE, "src", "benches")
    order = re.findall(r"^\s+(\w+)::run\(\);", open(os.path.join(src, "mod.rs")).read(), re.M)
    for mod in order:
        names += re.findall(r'bench\(\s*"(\w+)"', open(os.path.join(src, mod + ".rs")).read())
    return names


def functions(elf):
    """Sorted (address, demangled name) of the code symbols."""
    nm = tool("llvm-nm", "rust-nm", "arm-none-eabi-nm")
    out = run([nm, "-n", "-C", "--defined-only", elf]).stdout
    syms = []
    for line in out.splitlines():
        parts = line.split(" ", 2)
        if len(parts) == 3 and parts[1] in "tTW":
            syms.append((int(parts[0], 16) & ~1, parts[2]))
    return syms


def print_profile(lines, elf, name):
    import bisect
    syms = functions(elf)
    addrs = [a for a, _ in syms]
    per = {}
    for line in lines:
        if line.startswith("PROF "):
            _, a, n = line.split()
            i = bisect.bisect_right(addrs, int(a, 16)) - 1
            f = syms[i][1] if i >= 0 else a
            per[f] = per.get(f, 0) + int(n)
    total = sum(per.values()) or 1
    print(f"\nprofile of {name}: {total} instructions (with the marks)")
    for f, n in sorted(per.items(), key=lambda x: -x[1])[:20]:
        print(f"{n:9} {100 * n / total:5.1f}%  {f[:110]}")


def measure(build, profile=None):
    if build:
        subprocess.run(["cargo", "build", "--release", "-q"], cwd=PROBE, check=True)
    qemu = tool("qemu-system-arm")
    so = build_plugin(qemu)
    mark = mark_address(ELF)
    arg = f"{so},mark={mark:#x}"
    if profile:
        names = bench_names()
        if profile not in names:
            sys.exit(f"speed-check: no bench {profile}; benches: {' '.join(names)}")
        arg += f",profile={names.index(profile)}"
    p = subprocess.run(
        [qemu, "-M", "mps2-an386", "-nographic", "-semihosting-config", "enable=on,target=native",
         "-kernel", ELF, "-plugin", arg, "-d", "plugin"],
        text=True, capture_output=True, timeout=600)
    lines = (p.stdout + p.stderr).splitlines()
    benches, marks, failed = [], [], []
    for line in lines:
        if line.startswith("BENCH "):
            _, name, items = line.split()
            benches.append((name, int(items)))
        elif line.startswith("MARK "):
            marks.append(int(line.split()[1]))
        elif line.startswith(("CHECK ", "PANIC ")):
            failed.append(line)
    if p.returncode != 0 or failed or "END" not in lines:
        print("\n".join(lines), file=sys.stderr)
        sys.exit("speed-check: the probe failed")
    if len(marks) != 2 * len(benches):
        sys.exit(f"speed-check: {len(benches)} benches but {len(marks)} marks")
    if profile:
        print_profile(lines, ELF, profile)
    deltas = [marks[2 * i + 1] - marks[2 * i] for i in range(len(benches))]
    calibrate = deltas[0]
    return [(name, items, d - calibrate) for (name, items), d in list(zip(benches, deltas))[1:]]


def write_markdown(path, rows):
    with open(path, "w") as f:
        f.write("# Speed of the hot paths\n\n")
        f.write("Written by `python3 scripts/speed-check.py --markdown docs/speeds.md`. Executed Thumb-2\n")
        f.write("instructions on QEMU `mps2-an386` (Cortex-M4F), opt-level \"s\", fat LTO: not cycles\n")
        f.write("(docs/optimization-backlog.md, measurement method of speed). An item is one message, packet\n")
        f.write("or byte of the bench input; the bench names and inputs are in `tools/speed-probe/src/benches/`.\n\n")
        f.write("| Bench | Items | Instructions | For each item | Budget |\n|---|---:|---:|---:|---:|\n")
        for name, items, n in rows:
            b = BUDGETS.get(name)
            f.write(f"| `{name}` | {items} | {n:,} | {n / items:,.1f} | {f'{b:,}' if b else '-'} |\n")


def hardware(log, rows):
    """Prints the cycles of each bench in `log` beside the QEMU instructions of `rows`."""
    cycles = {}
    for line in open(log, errors="replace"):
        m = re.search(r"BENCH (\w+) (\d+) (\d+)", line)
        if m:
            cycles[m.group(1)] = (int(m.group(2)), int(m.group(3)))
    if "calibrate" not in cycles:
        sys.exit(f"speed-check: no BENCH calibrate line in {log}")
    cal = cycles["calibrate"][1]
    print(f"\n{'bench':42} {'cycles':>9} {'/item':>9} {'instr':>9} {'cyc/instr':>9}")
    for name, items, n in rows:
        if name not in cycles:
            print(f"{name:42} not in the log")
            continue
        c = cycles[name][1] - cal
        print(f"{name:42} {c:9} {c / items:9.1f} {n:9} {c / n if n else 0:9.2f}")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--no-build", action="store_true", help="measure the last build")
    ap.add_argument("--markdown", help="write the table as Markdown to this file")
    ap.add_argument("--profile", metavar="BENCH", help="give the hot functions of one bench")
    ap.add_argument("--hw", metavar="LOG", help="compare with the cycles in a log of examples/mk20dx-speed-bench")
    args = ap.parse_args()
    rows = measure(not args.no_build, args.profile)
    if args.hw:
        hardware(args.hw, rows)
    bad = 0
    print(f"{'bench':42} {'items':>6} {'instr':>9} {'/item':>9} {'budget':>9}")
    for name, items, n in rows:
        b = BUDGETS.get(name)
        flag = ""
        if b is None:
            flag, bad = "  NO BUDGET", bad + 1
        elif n > b:
            flag, bad = "  OVER", bad + 1
        print(f"{name:42} {items:6} {n:9} {n / items:9.1f} {b if b else '-':>9}{flag}")
    for name in sorted(set(BUDGETS) - {r[0] for r in rows}):
        print(f"{name:42} missing from the probe  MISSING")
        bad += 1
    if args.markdown:
        write_markdown(args.markdown, rows)
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
