"""Functions inlined at more than one call site, with their bytes (OPT-045).

    python3 tools/size-probe/dup_inline.py ELF [FILTER] [N]

Each instruction of .text is given to llvm-symbolizer --inlines. For each inlined function (a frame that is not
the outermost one) the script adds the bytes and counts the distinct inline sites (caller file:line:column of
the call). A function with many sites and many bytes is a candidate for #[inline(never)]. FILTER is a regular
expression on the function name (default: midi_). Generic arguments are removed from the names.
"""
import collections, glob, os, re, shutil, subprocess, sys
elf = sys.argv[1]
flt = re.compile(sys.argv[2] if len(sys.argv) > 2 else r"midi_")
top = int(sys.argv[3]) if len(sys.argv) > 3 else 30
NM = glob.glob(os.path.expanduser("~/.rustup/toolchains/stable-*/lib/rustlib/*/bin/llvm-nm"))[0]
OD = NM.replace("llvm-nm", "llvm-objdump")
SYM = os.environ.get("LLVM_SYMBOLIZER") or shutil.which("llvm-symbolizer") or "/opt/homebrew/opt/llvm/bin/llvm-symbolizer"
dis = subprocess.run([OD, "-d", "--no-show-raw-insn", "-j", ".text", elf], capture_output=True, text=True).stdout
addrs = [int(m.group(1), 16) for m in re.finditer(r"^\s*([0-9a-f]+):\s", dis, re.M)]
sizes = [b - a for a, b in zip(addrs, addrs[1:])] + [2]
out = subprocess.run([SYM, "--inlines", "--demangle", "--obj", elf], input="\n".join(hex(a) for a in addrs),
                     capture_output=True, text=True).stdout.strip().split("\n\n")
def clean(n):
    while True:  # generic arguments only (a '<' after a name), not the '<Type as Trait>' form
        m = re.sub(r"(?<=[\w>])<[^<>]*>", "", n)
        if m == n: return n
        n = m
by = collections.defaultdict(int); sites = collections.defaultdict(set)
for blk, sz in zip(out, sizes):
    l = blk.splitlines()
    frames = [(l[i], l[i + 1]) for i in range(0, len(l) - 1, 2)]  # innermost first
    seen = set()
    for k in range(len(frames) - 1):  # every frame but the outermost is inlined
        name = clean(frames[k][0])
        if name in seen or not flt.search(name): continue
        seen.add(name)
        by[name] += sz
        sites[name].add(frames[k + 1][1].split("zetaohm-midi/")[-1])  # the call line in the caller
rows = sorted(((by[n], len(sites[n]), n) for n in by if len(sites[n]) > 1), reverse=True)[:top]
for b, s, n in rows: print(f"{b:6d} {s:3d}  {n[:110]}")
