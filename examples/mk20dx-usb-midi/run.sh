#!/bin/sh
# Cargo runner: flash the ELF with probe-rs and show the defmt log.
set -e
if [ -n "${PROBE_SELECTOR:-}" ]; then
    exec probe-rs run --chip MK20DX256xxx7 --probe "$PROBE_SELECTOR" --connect-under-reset "$1"
fi
exec probe-rs run --chip MK20DX256xxx7 --connect-under-reset "$1"
