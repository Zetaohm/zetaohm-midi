import importlib.util, subprocess, sys, re
from functools import lru_cache
spec = importlib.util.spec_from_file_location("sc", __import__("os").path.join(__import__("os").path.dirname(__import__("os").path.abspath(__file__)), "..", "..", "scripts", "stack-check.py"))
sc = importlib.util.module_from_spec(spec); spec.loader.exec_module(sc)
sys.setrecursionlimit(100000)
for elf in sys.argv[1:]:
    dump = subprocess.run(sc.objdump() + ["-d", "--no-show-raw-insn", "--demangle", elf], capture_output=True, text=True, check=True).stdout
    frames, calls, tails = sc.parse(dump)
    active=set()
    @lru_cache(maxsize=None)
    def depth(f):
        if f in active: return 0, ()
        active.add(f); best=(0,())
        for c in calls.get(f,()):
            d=depth(c)
            if d[0]>best[0]: best=d
        own=(frames.get(f,0)+best[0], ((f,frames.get(f,0)),)+best[1])
        for c in tails.get(f,()):
            d=depth(c)
            if d[0]>own[0]: own=(d[0], ((f,0),)+d[1])
        active.discard(f); return own
    for f in sorted(x for x in frames if re.match(r"size_probe::.*::(run|init|on_event|poll)$", x)):
        t, ch = depth(f); big=max(ch,key=lambda x:x[1])
        print(f"{elf.split('/')[-1]:10} {t:6d}  {f:34} largest {big[1]:5d} {big[0][:60]}")
