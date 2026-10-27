#!/bin/sh
# usage: sh build-clang.sh   builds out/elf_<lib>_clang for the C libraries with clang -Os (LLVM, as rustc),
# one translation unit each (in place of LTO), linked with arm-none-eabi-gcc (newlib-nano).
set -e
cd "$(dirname "$0")"
E=../ext; M2=$E/sauloverissimo_midi2/src
CLANG=${CLANG:-/opt/homebrew/opt/llvm/bin/clang}
SYS=$(arm-none-eabi-gcc -print-sysroot)
F="--target=thumbv7em-none-eabi -mcpu=cortex-m4 -mfloat-abi=soft -Os -ffunction-sections -fdata-sections -DNDEBUG -I. --sysroot=$SYS -isystem $SYS/include"
L="-mcpu=cortex-m4 -mthumb -mfloat-abi=soft -nostartfiles -T link.ld -Wl,--gc-sections --specs=nano.specs --specs=nosys.specs"
mkdir -p out
printf '#include "crt.c"\n#include "m2.c"\n#include "%s/midi2_dispatch.c"\n#include "%s/midi2_conv.c"\n' $M2 $M2 > out/unity_m2.c
printf '#include "crt.c"\n#include "cm.c"\n' > out/unity_cm.c
for l in m2 cm; do
  $CLANG $F -I$M2 -I$E/atsushieno_cmidi2 -Wno-unused-function -c out/unity_$l.c -o out/unity_${l}_clang.o
  arm-none-eabi-gcc $L out/unity_${l}_clang.o -o out/elf_${l}_clang
  arm-none-eabi-gcc -mcpu=cortex-m4 -mthumb -mfloat-abi=soft -Os -ffunction-sections -fdata-sections -DNDEBUG -I. -I$M2 -I$E/atsushieno_cmidi2 -Wno-unused-function \
    -c out/unity_$l.c -o out/unity_${l}_gcc.o
  arm-none-eabi-gcc $L out/unity_${l}_gcc.o -o out/elf_${l}_gccunity
done
