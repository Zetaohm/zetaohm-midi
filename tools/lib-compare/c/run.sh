#!/bin/sh
cd "$(dirname "$0")"
E=../ext; M2=$E/sauloverissimo_midi2/src; AM=$E/midi2-dev_AM_MIDI2.0Lib/src; NI=$E/midi2-dev_ni-midi2/src
b() { sh build.sh "$@" 2>&1 | grep -v "ld: /opt\|warning:"; }
b none none.c
b m2_decode m2_decode.c $M2/midi2_dispatch.c
b m2_bytes m2_bytes.c $M2/midi2_conv.c
b m2_ci m2_ci.c $M2/midi2_proc.c $M2/midi2_ci.c $M2/midi2_dispatch.c
b am_decode am_decode.cpp $AM/umpProcessor.cpp $AM/utils.cpp
b am_bytes am_bytes.cpp $AM/bytestreamToUMP.cpp $AM/utils.cpp
b am_ci am_ci.cpp $AM/umpProcessor.cpp $AM/midiCIProcessor.cpp $AM/midiCIMessageCreate.cpp $AM/utils.cpp
b am_down am_down.cpp
b am_out am_out.cpp
[ -f ni_decode.cpp ] && b ni_decode ni_decode.cpp $NI/universal_packet.cpp
[ -f ni_bytes.cpp ] && b ni_bytes ni_bytes.cpp $NI/midi1_byte_stream.cpp $NI/universal_packet.cpp $NI/sysex.cpp
[ -f cm_decode.c ] && b cm_decode cm_decode.c
b cm_lean cm_lean.c
b cm_bytes cm_bytes.c
b cm_out cm_out.c
b m2_down m2_down.c
b m2_up m2_up.c
exit 0
