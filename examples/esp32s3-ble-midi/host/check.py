"""Compares the echo in the output of `gatt` with the written packets (BLE-MIDI v1.0 section 7:
header and timestamp bytes removed, running status expanded, Clock and the incomplete Song
Select left out, as the firmware does)."""
import sys
def strip(lines):
    out=[]
    for l in lines:
        b=[int(x,16) for x in l.split()]
        ts=False  # True right after a timestamp byte
        for x in b[1:]:
            if x>=0x80 and not ts: ts=True; continue
            ts=False; out.append(x)
    return out
L=open(sys.argv[1]).read().splitlines()
w=[l[7:] for l in L if l.startswith("write: ")]; n=[l[8:] for l in L if l.startswith("notify: ")]
sent=[x for x in strip(w) if x not in (0xF8,)]
# expected echo: drop Clock, the incomplete F3, expand running status
def expand(bs):
    out=[];st=0;need={0x8:2,0x9:2,0xA:2,0xB:2,0xC:1,0xD:1,0xE:2}
    i=0
    while i<len(bs):
        x=bs[i]
        if x==0xF3 and (i+1>=len(bs) or bs[i+1]>=0x80): i+=1; continue
        if x in (0xF0,0xF7) or (x<0x80 and st==0xF0): out.append(x); st=0xF0 if x==0xF0 else (0 if x==0xF7 else st); i+=1; continue
        if x>=0x80: st=x; i+=1; continue
        k=need[st>>4]; out+= [st]+bs[i:i+k]; i+=k
    return out
e,g=expand(sent),expand(strip(n))
print("sent",len(e),"echo",len(g),"equal",e==g)
if e!=g:
    for i,(a,b) in enumerate(zip(e,g)):
        if a!=b: print("first difference at",i,hex(a),hex(b)); break
