#!/bin/sh
# Clone the compared libraries into ext/ at the commits of the 2026-10-19 measurement.
cd "$(dirname "$0")"; mkdir -p ext
get() { d=ext/$1; url=$2; rev=$3
  [ -d $d/.git ] || git clone -q --filter=blob:none ${4:+--sparse} $url $d
  [ -n "$4" ] && git -C $d sparse-checkout set $4
  git -C $d -c advice.detachedHead=false checkout -q $rev && echo "$1 $rev"; }
get sauloverissimo_midi2     https://github.com/sauloverissimo/midi2   841f711234b29a4b9140a36b618061110440534d
get midi2-dev_AM_MIDI2.0Lib  https://github.com/midi2-dev/AM_MIDI2.0Lib 7fa1b0c59acf31fbaa55a0e45042663d9a8b7ee4
get midi2-dev_ni-midi2       https://github.com/midi2-dev/ni-midi2      81522602dec38b565dcf035ad02a092c36781ff9
get atsushieno_cmidi2        https://github.com/atsushieno/cmidi2      49d4fb19a8e79b287e472b0a6d3a0e083fe77c53
get midi2-dev_tusb_ump       https://github.com/midi2-dev/tusb_ump      74ecd5edaa490655f55fc7cfaeb6cf95a84b3fdc
get tinyusb                  https://github.com/hathach/tinyusb         2c992c00272bcba4e06cdcc7ed5a76cc9aac3278 src
