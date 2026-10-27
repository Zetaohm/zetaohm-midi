#!/bin/sh
cd "$(dirname "$0")"; mkdir -p objsz
T=ext/tinyusb/src; U=ext/midi2-dev_tusb_ump
A="-mcpu=cortex-m4 -mthumb -mfloat-abi=soft -Os -ffunction-sections -fdata-sections -DNDEBUG -Itusbcfg -I$T"
arm-none-eabi-g++ $A -DCFG_UMP_MIDI2_TO_MIDI1=1 -std=gnu++17 -fno-exceptions -fno-rtti -I$U -c $U/ump_device.cpp -o objsz/tusb_ump_m2m1.o 2>&1 | grep -m3 error
arm-none-eabi-size objsz/tusb_ump_device.o objsz/tusb_ump_m2m1.o
