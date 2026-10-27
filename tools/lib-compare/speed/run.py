#!/usr/bin/env python3
"""Instructions of other MIDI libraries on QEMU mps2-an386, next to tools/speed-probe (OPT-029).

    python3 tools/lib-compare/speed/run.py [--no-build]

Builds elf_<lib> with build.sh (arm-none-eabi-gcc -Os -flto), runs each with the plugin of
scripts/speed-check.py and prints the instructions for each item, with the Rust number of the same bench.
"""
import importlib.util, os, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(HERE)))
spec = importlib.util.spec_from_file_location("speed_check", os.path.join(ROOT, "scripts", "speed-check.py"))
sc = importlib.util.module_from_spec(spec); spec.loader.exec_module(sc)
LIBS = [("m2", "midi2 (C99)"), ("am", "AM_MIDI2.0Lib"), ("ni", "ni-midi2"), ("cm", "cmidi2")]


def measure(elf, so, qemu):
    arg = f"{so},mark={sc.mark_address(elf):#x}"
    p = subprocess.run([qemu, "-M", "mps2-an386", "-nographic", "-semihosting-config", "enable=on,target=native",
                        "-kernel", elf, "-plugin", arg, "-d", "plugin"], text=True, capture_output=True, timeout=600)
    lines = (p.stdout + p.stderr).splitlines()
    benches = [(l.split()[1], int(l.split()[2])) for l in lines if l.startswith("BENCH ")]
    marks = [int(l.split()[1]) for l in lines if l.startswith("MARK ")]
    if p.returncode != 0 or "END" not in lines or len(marks) != 2 * len(benches):
        print("\n".join(lines), file=sys.stderr); sys.exit(f"run.py: {elf} failed")
    d = [marks[2 * i + 1] - marks[2 * i] for i in range(len(benches))]
    return {n: (d[i] - d[0]) / items for i, (n, items) in enumerate(benches) if i}


def main():
    if "--no-build" not in sys.argv:
        subprocess.run(["sh", os.path.join(HERE, "build.sh")], check=True)
    qemu = sc.tool("qemu-system-arm"); so = sc.build_plugin(qemu)
    rust = {n: i_ / items for n, items, i_ in sc.measure("--no-build" not in sys.argv)}
    other = {lib: measure(os.path.join(HERE, "out", f"elf_{lib}"), so, qemu) for lib, _ in LIBS}
    # A Rust bench also shows when its name starts with a compared name (for example the _feed forms).
    compared = {n for o in other.values() for n in o}
    names = [n for n in rust if any(n.startswith(c) for c in compared)]
    names += sorted({n for o in other.values() for n in o} - set(names))
    print(f"{'bench (instructions for each item)':40}{'zetaohm':>9}" + "".join(f"{t:>15}" for _, t in LIBS))
    for n in names:
        r = f"{rust[n]:9.1f}" if n in rust else f"{'-':>9}"
        print(f"{n:40}{r}" + "".join(f"{other[l][n]:15.1f}" if n in other[l] else f"{'-':>15}" for l, _ in LIBS))


main()
