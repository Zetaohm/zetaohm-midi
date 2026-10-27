#!/bin/sh
# usage: sh build.sh   builds elf_<lib> for each library (QEMU mps2-an386, -Os -flto, as ../c/build.sh)
set -e
cd "$(dirname "$0")"
E=../ext; M2=$E/sauloverissimo_midi2/src; AM=$E/midi2-dev_AM_MIDI2.0Lib; NI=$E/midi2-dev_ni-midi2
F="-mcpu=cortex-m4 -mthumb -mfloat-abi=soft -Os -flto -ffunction-sections -fdata-sections -DNDEBUG -I."
L="-nostartfiles -T link.ld -Wl,--gc-sections --specs=nano.specs --specs=nosys.specs"
X="-fno-exceptions -fno-rtti -fno-threadsafe-statics -std=gnu++17"
mkdir -p out
arm-none-eabi-gcc $F -I$M2 crt.c m2.c $M2/midi2_dispatch.c $M2/midi2_conv.c $L -o out/elf_m2
arm-none-eabi-gcc $F -I$E/atsushieno_cmidi2 -Wno-unused-function crt.c cm.c $L -o out/elf_cm
arm-none-eabi-g++ $F $X -I$AM/include -x c crt.c -x c++ am.cpp $AM/src/umpProcessor.cpp $AM/src/utils.cpp $L -o out/elf_am
arm-none-eabi-g++ $F $X -I$NI/inc -x c crt.c -x c++ ni.cpp $NI/src/universal_packet.cpp $NI/src/midi1_byte_stream.cpp $NI/src/sysex.cpp $NI/src/sysex_collector.cpp $L -o out/elf_ni
