#!/bin/sh
# usage: sh measure.sh  -> prints one line per config
cd "$(dirname "$0")"
SIZE=$(ls ~/.rustup/toolchains/*/lib/rustlib/*/bin/llvm-size | head -1)
mkdir -p elf
run() { name=$1; feats=$2
  if [ -z "$feats" ]; then cargo build --release -q 2>/dev/null; else cargo build --release -q --features "$feats" 2>/dev/null || { echo "$name BUILD FAILED"; return; }; fi
  cp target/thumbv7em-none-eabi/release/size-probe elf/$name
  $SIZE -A elf/$name | awk -v n=$name '$1==".text"{t=$2} $1==".rodata"{r=$2} $1==".data"{d=$2} $1==".bss"{b=$2} $1==".uninit"{u=$2} END{printf "%-12s text %6d rodata %5d data %5d bss %5d  flash %6d ram %5d\n", n,t,r,d,b,t+r+d,d+b+u}'; }
run base ""
for f in rx_cv2 tx_cv2 translate bytes flex stream ci pe zlib sysex8 remote inject usb1 usb2; do run $f $f; done
C=usb2,rx_cv2,tx_cv2,translate,bytes,flex,stream
run core $C
run core_ci $C,ci
run core_pe $C,pe
run core_zlib $C,zlib
run core_all $C,zlib,sysex8,remote,inject
run small rx_cv2,tx_cv2,flex,stream
run small_tr rx_cv2,tx_cv2,flex,stream,translate
run small_by rx_cv2,tx_cv2,flex,stream,bytes
if [ "$1" = lean ]; then :; fi
