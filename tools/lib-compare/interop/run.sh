#!/bin/sh
# Our MIDI-CI Initiator (zmidi-ci) against the midi2 C99 responder on the host (OPT-049,
# step 10). Needs cc and the sources from tools/lib-compare/fetch.sh. usage: sh tools/lib-compare/interop/run.sh
set -e
cd "$(dirname "$0")"
S=../ext/sauloverissimo_midi2/src
mkdir -p rs/target
cc -std=c99 -O1 -Wall -I$S m2_ci_host.c $S/midi2_ci.c $S/midi2_proc.c -o rs/target/m2_ci_host
cd rs && cargo run -q -- target/m2_ci_host
