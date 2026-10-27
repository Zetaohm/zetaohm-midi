# lib-compare

Flash and RAM of other MIDI 2.0 libraries on the FLXS1 target (Cortex-M4, `-Os`), for comparison with
`tools/size-probe`. Results: the document "zetaohm-midi: flash and RAM against other MIDI 2.0 libraries"
(2026-10-19).

1. `sh fetch.sh` clones the libraries into `ext/` at the measured commits.
2. `sh c/run.sh` links one probe for each job (decode, byte stream, translation, CI) and prints flash and RAM.
   Subtract the `none` probe (12 B flash, 256 B RAM). The table with the Rust jobs of the same feature set is
   `docs/c-comparison.md`.
3. `sh objsz.sh` prints the size of each library object; `sh tusb.sh` and `sh m2m1.sh` the TinyUSB classes.
4. `cd rs-midi2 && cargo build --release --features lean` (or `full`, `full,allmt`) for the `midi2` crate.

Needs `arm-none-eabi-gcc` (newlib-nano) and, for `rs-midi2`, the sibling repository `firmware/mk20dx`.

## MIDI-CI interop on the host

`sh interop/run.sh` builds the midi2 C99 MIDI-CI responder for the host (`interop/m2_ci_host.c`, one message
per line on stdin, reply UMP words on stdout) and runs `interop/rs`: our `zmidi-ci` Initiator sends Discovery,
Inquiry: Endpoint Information, Profile Inquiry, Inquiry: PE Capabilities, Inquiry: Get Property Data and
Inquiry: Process Inquiry Capabilities, and reads each reply with the same Device. A secondary check of DEC-001:
a difference is a finding to read against M2-101, not a test vector.

## Speed (instructions on QEMU)

`python3 speed/run.py [--no-build]` builds `speed/out/elf_<lib>` (arm-none-eabi-gcc `-Os -flto`, QEMU
`mps2-an386`, `speed/link.ld`), runs each with the plugin of `scripts/speed-check.py` and prints the instructions
for each item next to the Rust bench of the same name (`tools/speed-probe`). The inputs are the same tables
(`speed/crt.c`), and each bench checks its output count. Notes:

- cmidi2 `cmidi2_convert_midi1_to_ump` keeps its byte indexes through a `uint8_t *` view of a `size_t`, so one call
  takes at most 255 bytes in and out. The bench gives it 4 calls of 16 notes.
- ni-midi2 `sysex7_collector` grows a `std::vector` on the heap (newlib-nano `malloc`); its count includes that.
- The 3-byte manufacturer ID is not part of the ni-midi2 SysEx data, so its check is 1017 bytes.
- The C side uses GCC 14 and the Rust side LLVM. `sh speed/build-clang.sh` builds the C libraries (midi2, cmidi2)
  with clang `-Os` too (`speed/out/elf_<lib>_clang`, one translation unit, linked with arm-none-eabi-gcc); on
  2026-10-23 it gave -22 % to +30 % against GCC (OPT-037).
- A Rust bench whose name starts with a compared name (the `_feed` forms) shows in the same table.
