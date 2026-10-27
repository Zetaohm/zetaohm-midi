#!/bin/sh
cd "$(dirname "$0")"; mkdir -p objsz
T=ext/tinyusb/src; U=ext/midi2-dev_tusb_ump
A="-mcpu=cortex-m4 -mthumb -mfloat-abi=soft -Os -ffunction-sections -fdata-sections -DNDEBUG -Itusbcfg -I$T"
arm-none-eabi-gcc $A -std=gnu11 -c $T/class/midi/midi2_device.c -o objsz/tusb_midi2_device.o 2>&1 | grep -m5 error
arm-none-eabi-gcc $A -std=gnu11 -c $T/class/midi/midi_device.c -o objsz/tusb_midi_device.o 2>&1 | grep -m5 error
arm-none-eabi-g++ $A -std=gnu++17 -fno-exceptions -fno-rtti -I$U -c $U/ump_device.cpp -o objsz/tusb_ump_device.o 2>&1 | grep -m5 error
arm-none-eabi-size objsz/tusb_midi2_device.o objsz/tusb_midi_device.o objsz/tusb_ump_device.o 2>/dev/null
