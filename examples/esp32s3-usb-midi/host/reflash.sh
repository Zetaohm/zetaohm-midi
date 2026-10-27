#!/bin/sh
# Flashes esp32s3-usb-midi with no BOOT button: sends the "boot" SysEx, waits for the ROM
# download port (USB-Serial-JTAG, 303A:1001) and runs espflash. Run from examples/esp32s3-usb-midi
# after `cargo build --release`. Argument: the ELF (default: this example). Needs espflash and uv.
set -e
elf=${1:-target/xtensa-esp32s3-none-elf/release/esp32s3-usb-midi}
port_of_jtag() {
    espflash list-ports 2>/dev/null | awk '/303A/ && $1 ~ /cu\./ {print $1; exit}'
}
port=$(port_of_jtag)
if [ -z "$port" ]; then
    uv run --quiet --with mido --with python-rtmidi host/boot.py
    i=0
    while [ -z "$port" ] && [ $i -lt 50 ]; do
        sleep 0.2
        port=$(port_of_jtag)
        i=$((i + 1))
    done
fi
[ -n "$port" ] || { echo "no USB-Serial-JTAG port (303A:1001) appeared" >&2; exit 1; }
espflash flash --chip esp32s3 --port "$port" --non-interactive "$elf"
# The boot command set FORCE_DOWNLOAD_BOOT (RTC_CNTL OPTION1, 0x6000812C bit 0), which stays set
# over a reset: clear it, then reset into the new firmware.
uvx esptool --chip esp32s3 --port "$port" --before no-reset --after hard-reset \
    write-mem 0x6000812C 0 0x1 >/dev/null
