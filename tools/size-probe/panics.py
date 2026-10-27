"""Panic call sites of a probe ELF (OPT-043): kind | enclosing function | first source line in crates/.

    python3 tools/size-probe/panics.py ELF | sort | uniq -c | sort -rn
"""
import re, subprocess, sys, glob, os, collections
elf = sys.argv[1]
NM = glob.glob(os.path.expanduser("~/.rustup/toolchains/stable-*/lib/rustlib/*/bin/llvm-nm"))[0]
OD = NM.replace("llvm-nm", "llvm-objdump")
SYM = "/opt/homebrew/opt/llvm/bin/llvm-symbolizer"
dis = subprocess.run([OD, "-d", "--no-show-raw-insn", "-C", elf], capture_output=True, text=True).stdout
fn = None; sites = []
pat = re.compile(r"^\s*([0-9a-f]+):\s+(bl|b|b\.w|bne\.w|beq\.w|bhi\.w|blo\.w|bhs\.w|bls\.w|bgt\.w|blt\.w|bge\.w|ble\.w|bcs\.w|bcc\.w|b[a-z]+)\s+0x[0-9a-f]+ <([^>+]+)")
for line in dis.splitlines():
    m = re.match(r"^[0-9a-f]+ <(.*)>:", line)
    if m: fn = m.group(1); continue
    m = pat.match(line)
    if m and re.search(r"panic|index_fail|len_mismatch|expect_failed|unwrap_failed|already_borrowed", m.group(3)) and not re.search(r"panic|index_fail|len_mismatch|expect_failed|unwrap_failed|already_borrowed", fn or ""):
        sites.append((int(m.group(1), 16), fn, m.group(3).split("::")[-1]))
out = subprocess.run([SYM, "--inlines", "--demangle", "--obj", elf], input="\n".join(hex(a) for a, _, _ in sites), capture_output=True, text=True).stdout.strip().split("\n\n")
rows = []
for (a, f, k), blk in zip(sites, out):
    lines = blk.splitlines()
    loc = lines[1] if len(lines) > 1 else "?"
    # first frame in our crates
    ours = next((lines[i+1] for i in range(0, len(lines)-1, 2) if "/crates/" in lines[i+1]), loc)
    rows.append((k, f[:70], ours.split("zetaohm-midi/")[-1]))
for r in rows: print(*r, sep=" | ")
print(len(rows), "sites", file=sys.stderr)
