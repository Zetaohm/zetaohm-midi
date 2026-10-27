#!/bin/sh
# Compile every library source for Cortex-M4 (-Os) and print the total of the objects.
cd "$(dirname "$0")/ext"
A="-mcpu=cortex-m4 -mthumb -mfloat-abi=soft -Os -ffunction-sections -fdata-sections"
OUT=../objsz; mkdir -p $OUT
for d in sauloverissimo_midi2 midi2-dev_AM_MIDI2.0Lib midi2-dev_ni-midi2; do
  objs=""
  for f in $(find $d/src -maxdepth 1 \( -name '*.c' -o -name '*.cpp' \)); do
    o=$OUT/$d-$(basename $f).o
    case $f in
      *.c) arm-none-eabi-gcc $A -std=c99 -I$d/src -I$d/include -I$d/inc -c $f -o $o 2>&1 | grep -m3 error ;;
      *) arm-none-eabi-g++ $A -std=c++17 -fno-exceptions -fno-rtti -I$d/include -I$d/inc -I$d/src -c $f -o $o 2>&1 | grep -m3 error ;;
    esac
    [ -f $o ] && objs="$objs $o"
  done
  echo "== $d"; arm-none-eabi-size -t $objs | sed 's|.*/||' 
done
