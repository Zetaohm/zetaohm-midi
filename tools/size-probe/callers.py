"""Call sites of a function in a probe ELF, with the first source line in crates/ (OPT-047).

    python3 tools/size-probe/callers.py ELF REGEX

REGEX matches the demangled callee name (for example 'memmove'). Prints: count, enclosing function, source line.
"""
import collections, glob, os, re, shutil, subprocess, sys
elf, pat = sys.argv[1], re.compile(sys.argv[2])
NM = glob.glob(os.path.expanduser("~/.rustup/toolchains/stable-*/lib/rustlib/*/bin/llvm-nm"))[0]
OD = NM.replace("llvm-nm", "llvm-objdump")
SYM = os.environ.get("LLVM_SYMBOLIZER") or shutil.which("llvm-symbolizer") or "/opt/homebrew/opt/llvm/bin/llvm-symbolizer"
dis = subprocess.run([OD, "-d", "--no-show-raw-insn", "-C", elf], capture_output=True, text=True).stdout
fn = ""; sites = []
for line in dis.splitlines():
    m = re.match(r"^[0-9a-f]+ <(.*)>:", line)
    if m: fn = m.group(1); continue
    m = re.match(r"^\s*([0-9a-f]+):\s+(b|bl|b\.w)\s+0x[0-9a-f]+ <([^>+]+)>", line)
    if m and pat.search(m.group(3)) and not pat.search(fn):
        sites.append((int(m.group(1), 16), fn))
out = subprocess.run([SYM, "--inlines", "--demangle", "--obj", elf], input="\n".join(hex(a) for a, _ in sites),
                     capture_output=True, text=True).stdout.strip().split("\n\n")
rows = collections.Counter()
for (a, f), blk in zip(sites, out):
    l = blk.splitlines()
    loc = next((l[i + 1] for i in range(0, len(l) - 1, 2) if "/crates/" in l[i + 1]), l[1] if len(l) > 1 else "?")
    rows[(f[:80], loc.split("zetaohm-midi/")[-1])] += 1
for (f, loc), n in rows.most_common(): print(n, f, "|", loc)
