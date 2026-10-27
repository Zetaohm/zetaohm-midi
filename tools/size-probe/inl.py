"""Bytes of one function by inlined callee (OPT-049, step 0).

    python3 tools/size-probe/inl.py ELF SYMBOL [DEPTH]

SYMBOL is the demangled name, for example size_probe::ci::run. Each instruction of the function is given to
llvm-symbolizer --inlines; its bytes count for the chain of inlined callees (DEPTH levels below the
function, default 1). Generic arguments are removed from the names. Needs the probe ELF with debug info
(the release profile of the probe has debug = 2) and llvm-symbolizer (Homebrew llvm, or LLVM_SYMBOLIZER).
"""
import collections
import re
import subprocess
import sys
elf, name = sys.argv[1], sys.argv[2]
depth = int(sys.argv[3]) if len(sys.argv) > 3 else 1
NM = subprocess.run("ls ~/.rustup/toolchains/stable-*/lib/rustlib/*/bin/llvm-nm", shell=True, capture_output=True, text=True).stdout.split()[0]
OD = NM.replace("llvm-nm", "llvm-objdump")
import os, shutil
SYM = os.environ.get("LLVM_SYMBOLIZER") or shutil.which("llvm-symbolizer") or "/opt/homebrew/opt/llvm/bin/llvm-symbolizer"
start = size = None
for line in subprocess.run([NM, "-S", "-C", elf], capture_output=True, text=True).stdout.splitlines():
    p = line.split(None, 3)
    if len(p) == 4 and p[3] == name and p[2] in "tT":
        start, size = int(p[0], 16), int(p[1], 16); print("function", p[3][:100], size); break
dis = subprocess.run([OD, "-d", "--no-show-raw-insn", f"--start-address={start}", f"--stop-address={start+size}", elf], capture_output=True, text=True).stdout
addrs = [int(m.group(1), 16) for m in re.finditer(r"^\s*([0-9a-f]+):", dis, re.M)]
addrs.append(start + size)
inp = "\n".join(hex(a) for a in addrs[:-1])
out = subprocess.run([SYM, "--inlines", "--demangle", "--obj", elf], input=inp, capture_output=True, text=True).stdout
blocks = out.strip().split("\n\n")
acc = collections.Counter()
for i, b in enumerate(blocks):
    if i + 1 >= len(addrs): break
    n = addrs[i + 1] - addrs[i]
    funcs = [l for l in b.splitlines()[0::2]]
    funcs.reverse()  # outermost first
    key = " > ".join(re.sub(r"<.*?>|::\{closure#\d\}", "", f)[:60] for f in funcs[1:1 + depth]) or "(own)"
    acc[key] += n
for k, v in acc.most_common(40):
    print(f"{v:6d} {k}")
