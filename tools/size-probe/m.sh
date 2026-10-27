#!/bin/sh
# usage: sh m.sh name[=feats] ...   flash and RAM of each configuration (a build failure says FAILED)
cd "$(dirname "$0")"; SIZE=$(ls ~/.rustup/toolchains/*/lib/rustlib/*/bin/llvm-size | head -1); mkdir -p elf
B=target/thumbv7em-none-eabi/release/size-probe
for spec in "$@"; do name=${spec%%=*}; feats=${spec#*=}; [ "$name" = "$spec" ] && feats=$name
  rm -f $B
  if [ "$feats" = base ]; then cargo build --release -q 2>&1 | grep -m5 -A5 '^error'; else cargo build --release -q --features "$feats" 2>&1 | grep -m5 -A5 '^error'; fi
  [ -f $B ] || { echo "$name FAILED"; continue; }
  cp $B elf/$name
  $SIZE -A elf/$name | awk -v n=$name '$1==".text"{t=$2} $1==".rodata"{r=$2} $1==".data"{d=$2} $1==".bss"{b=$2} $1==".uninit"{u=$2} END{printf "%-14s flash %6d ram %5d\n", n,t+r+d,d+b+u}'
done
