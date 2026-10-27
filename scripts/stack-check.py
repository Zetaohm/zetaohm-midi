#!/usr/bin/env python3
"""Stack check of the main entry points (docs/optimization-backlog.md, OPT-003).

Builds tools/stack-probe for thumbv7em-none-eabihf (release, opt-level "s"), reads the disassembly
and gives the deepest call chain of each `probe_*` function. A function's frame is its `push`,
`vpush` and `sub sp` bytes. `bl` is a call; `b` to another function is a tail call (the caller
frame is gone). Indirect calls are not followed, and a recursive edge is not followed, so the
values are lower bounds.

    python3 scripts/stack-check.py [--features auth,zlib] [--no-build] [--chain NAME]

Exit 1 when an entry point is over its budget (BUDGETS below; the default build only).
"""

import argparse
import os
import re
import shutil
import subprocess
import sys
from functools import lru_cache

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PROBE = os.path.join(ROOT, "tools", "stack-probe")
TARGET = "thumbv7em-none-eabihf"
LIB = os.path.join(PROBE, "target", TARGET, "release", "libstack_probe.a")

# Bytes, default features. Set from the measurement after each optimization, with a small margin,
# so that a change that brings a large stack copy back fails CI.
# Measured 2026-10-18 (rustc 1.98) after OPT-003, OPT-007 and OPT-004, plus about 10%.
BUDGETS = {
    "probe_io_bytes_to_ump_new": 256,
    "probe_net_client_init": 2560,
    "probe_net_client_new": 6528,
    "probe_net_host_init": 2560,
    "probe_net_host_new": 6528,
    "probe_net_host_poll": 2560,
    "probe_net_host_receive": 6400,
    "probe_pe_new": 1920,
    "probe_pe_receive": 4608,
    "probe_pe_write_pending": 2048,
}


def objdump():
    for tool in ("llvm-objdump", "rust-objdump", "arm-none-eabi-objdump"):
        if shutil.which(tool):
            return [tool]
    sysroot = subprocess.run(["rustc", "--print", "sysroot"], capture_output=True, text=True).stdout.strip()
    for dirpath, _, files in os.walk(os.path.join(sysroot, "lib", "rustlib")):
        if "llvm-objdump" in files:
            return [os.path.join(dirpath, "llvm-objdump")]
    sys.exit("stack-check: no objdump (rustup component add llvm-tools)")


def regs(text):
    """Number of registers in a list such as {r4, r5, r7, lr} or {d8-d11}."""
    n = 0
    for part in text.split(","):
        part = part.strip()
        m = re.match(r"([rsd])(\d+)\s*-\s*[rsd](\d+)", part)
        n += int(m.group(3)) - int(m.group(2)) + 1 if m else (1 if part else 0)
    return n


def parse(dump):
    frames, calls, tails = {}, {}, {}
    cur = None
    consts = {}
    for line in dump.splitlines():
        m = re.match(r"^[0-9a-f]+ <(.+)>:$", line)
        if m:
            cur = m.group(1)
            frames.setdefault(cur, 0)
            calls.setdefault(cur, set())
            tails.setdefault(cur, set())
            consts = {}
            continue
        if cur is None or "\t" not in line:
            continue
        # The probe is not linked: the target of a call is in its relocation line.
        m = re.match(r"^\s+[0-9a-f]+:\s+R_ARM_THM_(CALL|JUMP24|JUMP19)\s+(.+)$", line)
        if m:
            sym = m.group(2).strip()
            if sym != cur:
                (calls if m.group(1) == "CALL" else tails)[cur].add(sym)
            continue
        ins = line.split("\t", 1)[1].strip()
        op, _, args = ins.partition("\t")
        op = op.strip()
        args = args.strip()
        if not args and " " in op:
            op, _, args = op.partition(" ")
        if op in ("push", "push.w", "stmdb", "stmdb.w") and ("sp!" in args or op.startswith("push")):
            m = re.search(r"\{(.*)\}", args)
            if m:
                frames[cur] += 4 * regs(m.group(1))
        elif op in ("vpush", "vpush.64", "vpush.32"):
            m = re.search(r"\{(.*)\}", args)
            if m:
                size = 8 if "d" in m.group(1) else 4
                frames[cur] += size * regs(m.group(1))
        elif op in ("sub", "sub.w", "subw", "subs") and re.match(r"sp,", args):
            m = re.search(r"#(0x[0-9a-f]+|\d+)", args)
            if m:
                frames[cur] += int(m.group(1), 0)
            else:
                m = re.search(r"\b(r\d+|ip|lr)\s*$", args)
                if m and m.group(1) in consts:
                    frames[cur] += consts[m.group(1)]
        elif op in ("movw", "mov.w", "mov", "movs") and "#" in args:
            m = re.match(r"(r\d+|ip|lr),\s*#(0x[0-9a-f]+|\d+)", args)
            if m:
                consts[m.group(1)] = int(m.group(2), 0)
        elif op == "movt":
            m = re.match(r"(r\d+|ip|lr),\s*#(0x[0-9a-f]+|\d+)", args)
            if m:
                consts[m.group(1)] = consts.get(m.group(1), 0) + (int(m.group(2), 0) << 16)
        elif op in ("bl", "blx") or re.match(r"^b(\.w|\.n)?$", op):
            m = re.search(r"<([^>+]+)>", args)
            if not m or m.group(1) == cur:
                continue  # a local branch, or a call that a relocation line gives
            (calls if op.startswith("bl") else tails)[cur].add(m.group(1))
    return frames, calls, tails


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--features", default="")
    ap.add_argument("--no-build", action="store_true")
    ap.add_argument("--chain", help="print the deepest chain of this probe")
    a = ap.parse_args()

    if not a.no_build:
        cmd = ["cargo", "build", "--quiet", "--release", "--target", TARGET]
        if a.features:
            cmd += ["--features", a.features]
        subprocess.run(cmd, cwd=PROBE, check=True)

    dump = subprocess.run(objdump() + ["-dr", "--no-show-raw-insn", "--demangle", LIB],
                          capture_output=True, text=True, check=True).stdout
    frames, calls, tails = parse(dump)
    sys.setrecursionlimit(100000)
    active = set()

    @lru_cache(maxsize=None)
    def depth(f):
        if f in active:
            return 0, ()
        active.add(f)
        best = (0, ())
        for c in calls.get(f, ()):
            d = depth(c)
            if d[0] > best[0]:
                best = d
        own = (frames.get(f, 0) + best[0], ((f, frames.get(f, 0)),) + best[1])
        for c in tails.get(f, ()):
            d = depth(c)
            if d[0] > own[0]:
                own = (d[0], ((f, 0),) + d[1])
        active.discard(f)
        return own

    probes = sorted(f for f in frames if f.startswith("probe_"))
    if not probes:
        sys.exit("stack-check: no probe_* function in the disassembly")
    failed = []
    print(f"{'stack':>7}  {'own':>6}  {'largest frame in the chain':<40}  entry point")
    for p in probes:
        total, chain = depth(p)
        big = max(chain, key=lambda x: x[1])
        print(f"{total:7d}  {frames[p]:6d}  {big[1]:6d} {big[0][:33]:<33}  {p}")
        limit = BUDGETS.get(p)
        if limit is not None and not a.features and total > limit:
            failed.append((p, total, limit))
    if a.chain:
        for f, n in depth(a.chain)[1]:
            print(f"  {n:6d}  {f[:110]}")
    for p, total, limit in failed:
        print(f"stack-check: FAIL {p}: {total} bytes, budget {limit}", file=sys.stderr)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
