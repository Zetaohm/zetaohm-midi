"""Symbol size diff of two probe ELFs (OPT-049, OPT-050).

    python3 tools/size-probe/symdiff.py <llvm-nm> <elfA> <elfB> [N]

Sums the code and data symbols of each ELF by demangled name and prints the N largest differences B - A
(default 25), and the total. llvm-nm: ls ~/.rustup/toolchains/*/lib/rustlib/*/bin/llvm-nm.
"""
import subprocess
import sys

nm = sys.argv[1]


def syms(elf):
    d = {}
    for line in subprocess.run([nm, "-S", "-C", elf], capture_output=True, text=True).stdout.splitlines():
        p = line.split(None, 3)
        if len(p) == 4 and p[2] in "tTrRdD":
            d[p[3]] = d.get(p[3], 0) + int(p[1], 16)
    return d


a, b = syms(sys.argv[2]), syms(sys.argv[3])
diff = sorted(((b.get(k, 0) - a.get(k, 0), k) for k in set(a) | set(b)), reverse=True)
diff = [x for x in diff if x[0]]
print("total", sum(x[0] for x in diff))
for d, k in diff[: int(sys.argv[4]) if len(sys.argv) > 4 else 25]:
    print(f"{d:6d} {k[:130]}")
