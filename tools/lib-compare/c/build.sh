#!/bin/sh
# usage: sh build.sh <name> <sources...>   prints flash (text+data) and RAM (data+bss) of the linked probe
cd "$(dirname "$0")"
E=../ext; N=$1; shift
INC="-I. -I$E/sauloverissimo_midi2/src -I$E/midi2-dev_AM_MIDI2.0Lib/include -I$E/midi2-dev_ni-midi2/inc -I$E/atsushieno_cmidi2"
F="-mcpu=cortex-m4 -mthumb -mfloat-abi=soft -Os -flto -ffunction-sections -fdata-sections -DNDEBUG"
arm-none-eabi-g++ $F $INC -fno-exceptions -fno-rtti -std=gnu++17 -x c base.c -x none "$@" \
  -nostartfiles -Wl,--gc-sections -Wl,-e,main --specs=nano.specs --specs=nosys.specs -o elf_$N 2>&1 | grep -v "warning\|note:\|in function\|^\s*|" | grep -m8 . 
arm-none-eabi-size -A elf_$N 2>/dev/null | awk -v n=$N '$1==".text"{t=$2} $1==".rodata"{r=$2} $1==".data"{d=$2} $1==".bss"{b=$2} END{printf "%-14s text %6d rodata %5d data %4d bss %5d  flash %6d ram %5d\n",n,t,r,d,b,t+r+d,d+b}'
