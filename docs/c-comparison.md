# Flash and RAM against the C MIDI 2.0 libraries (2026-10-26, rows 6 to 12 updated the same day)

Each row is one job with the same feature set on both sides. A C library that does only part of the job is in
parentheses, with the reason in the notes. "-" means that the library has no code for the job.

## Flash (bytes)

| # | Job | zetaohm-midi | midi2 C99 | AM_MIDI2.0Lib | ni-midi2 | cmidi2 | TinyUSB / tusb_ump | Ratio to the smallest C |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| 1 | UMP receive: decode MIDI 2.0 Channel Voice and System, dispatch to the application | 2,944 | 2,544 | 5,056 | 7,512 | - | - | 1.16x |
| 2 | UMP field getters, the application dispatches (13 message types) | 756 | - | - | - | 304 | - | 2.49x |
| 3 | DIN in: MIDI 1.0 bytes to UMP (MIDI 1.0 Protocol), with SysEx | 676 | 852 | 832 | 8,960 | 804 | - | 0.81x |
| 4 | DIN out: UMP to MIDI 1.0 bytes (Message Types 1, 2, 3, 4) | 936 | - | 1,160 | - | 554 | - | 1.69x |
| 5 | MIDI 2.0 to 1.0 Protocol, M2-104 D.2 (RPN, NRPN and Bank as CC sequences) | 488 | (228) | 680 | - | - | - | 0.72x |
| 6 | MIDI 1.0 to 2.0 Protocol, M2-104 D.3 (Bank and RPN/NRPN state), one Group | 932 | (212) | - | - | - | - | - |
| 7 | USB MIDI 2.0 class with Alt 0 USB-MIDI 1.0 translation (class only, no USB core) | 8,140 | - | - | - | - | 3,556 | 2.29x |
| 8 | USB MIDI 2.0 class with no Alt 0 translation (class only, no USB core) | 4,872 | - | - | - | - | 3,439 | 1.42x |
| 9 | MIDI-CI responder: Discovery, Invalidate MUID, Profiles, Process Inquiry, PE Get and Set in one Chunk | 15,500 | 4,615 | (14,534) | - | - | - | 3.36x |
| 10 | Row 9 with no Profiles and no Process Inquiry | 11,768 | - | - | - | - | - | - |
| 11 | MIDI-CI Discovery and management only (no category) | 6,940 | - | - | - | - | - | - |
| 12 | PE engine: transactions, Chunks, Flow Control, subscriptions, JSON headers | 26,200 | - | - | - | - | - | - |

## Static RAM (bytes)

| # | Job | zetaohm-midi | midi2 C99 | AM_MIDI2.0Lib | ni-midi2 | cmidi2 | TinyUSB / tusb_ump | Ratio to the smallest C |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| 1 | UMP receive and dispatch | 0 | 208 | 564 | 608 | - | - | 0.0x |
| 2 | UMP field getters | 0 | - | - | - | 0 | - | equal |
| 3 | DIN in | 24 | 52 | 140 | 688 | 0 | - | (see note 3) |
| 4 | DIN out | 4 | - | 104 | - | 0 | - | (see note 4) |
| 5 | MIDI 2.0 to 1.0 Protocol | 0 | (0) | 40 | - | - | - | 0.0x |
| 6 | MIDI 1.0 to 2.0 Protocol | 128 | (0) | - | - | - | - | - |
| 7 | USB class with Alt 0 translation | 464 | - | - | - | - | 666 | 0.70x |
| 8 | USB class with no Alt 0 translation | 364 | - | - | - | - | 316 | 1.15x |
| 9 | MIDI-CI responder with PE Get and Set | 1,472 | 736 | (2,084) | - | - | - | 2.00x |
| 10 | Row 9 with no Profiles and no Process Inquiry | 1,416 | - | - | - | - | - | - |
| 11 | MIDI-CI Discovery and management only | 680 | - | - | - | - | - | - |
| 12 | PE engine | 2,992 | - | - | - | - | - | - |

## Which job of each side

| # | zetaohm-midi (`scripts/size-check.py`) | C probe (`tools/lib-compare/c`, `tusb.sh`) |
|---|---|---|
| 1 | `decode_cv2_system` | `m2_decode`, `am_decode`, `ni_decode` |
| 2 | `decode_lean` | `cm_lean` |
| 3 | `din_in_feed` (`din_in`, the value form: 1,032 B) | `m2_bytes`, `am_bytes`, `ni_bytes`, `cm_bytes` |
| 4 | `din_out_each` (`din_out`, the value form: 1,276 B) | `am_out`, `cm_out` |
| 5 | `midi2_to_midi1_words` | `am_down`, `m2_down` |
| 6 | `midi1_to_midi2_words` (`midi1_to_midi2_lean`, with the typed parse: 1,200 B) | `m2_up` |
| 7 | `usb_class_fixed` (embassy-usb: `emb_class_fixed`, 9,256 B) | tusb_ump `ump_device.cpp` object |
| 8 | `usb_class_lean` (embassy-usb: `emb_class_lean`, 6,116 B) | TinyUSB `class/midi/midi2_device.c` object |
| 9 | `ci_like_c` (`ci_like_c_flat`, with the flat Header Data reader: 14,912 B) | `m2_ci`, `am_ci` |
| 10 | `ci_one_chunk` (`ci_one_chunk_flat`: 11,184 B) | - |
| 11 | `ci_device` | - |
| 12 | `pe_engine` | - |

## Notes on the feature match

1. Row 1: the C libraries decode with a dispatch to callbacks (midi2 C99, AM) or with typed views (ni-midi2).
   Their RAM is the table of callbacks and the parser state (midi2 C99, AM), or newlib and C++ runtime data that
   the code links (ni-midi2). The Rust decoder returns a value and keeps no state.
2. Row 2: cmidi2 has only inline field getters (no dispatch). The probe `cm_lean` reads the same 13 message types
   and fields as `decode_lean`. The Rust `parse` also checks the reserved bits and the Status values of each type.
3. Row 3: cmidi2 converts a whole caller buffer in one call and keeps no state between calls (no running status or
   SysEx across two calls); its probe has 192 B of caller buffers, not counted. ni-midi2 collects SysEx in a
   `std::vector` on the heap (newlib-nano `malloc` is in its flash).
4. Row 4: cmidi2 writes one UMP into a caller buffer with no running status and no state; the Rust writer
   keeps the running status (4 B).
5. Row 5: midi2 C99 `midi2_msg_mt4_to_mt2` gives one word and drops RPN, NRPN, Bank and the Per-Note messages, so
   it does less than M2-104 D.2. AM writes the CC sequences, as the Rust code does.
6. Row 6: midi2 C99 `midi2_msg_mt2_to_mt4` is stateless: it has no Bank Select or RPN/NRPN state (M2-104 D.3 needs
   it). AM has no UMP to UMP form (only bytes to MIDI 2.0 UMP).
7. Rows 7 and 8: the C numbers are object files (`-Os`, not linked, no `--gc-sections`), with no USB
   descriptors (the application gives them) and no USB core. The Rust numbers are the class minus the usb-device
   core with no class, and include the descriptors and the Group Terminal Block reply. tusb_ump translates USB-MIDI
   1.0 to and from UMP in Alt 0, as row 7 does. TinyUSB `midi2_device` sends raw USB-MIDI 1.0 packets in Alt 0 (the
   application writes them) and answers the UMP Stream discovery messages itself; the Rust row 8 has an empty Alt 0
   and no UMP Stream code (that is `zmidi-ump` stream, not in this job).
8. Row 9: midi2 C99 answers Discovery, Invalidate MUID, Profile Inquiry, Set Profile On/Off, PE Capabilities, PE
   Get, PE Set, Process Inquiry Capabilities and MIDI Message Report, finds "resource" with a key search and does
   not check the JSON, the destination MUID, the Device ID or the Message Version. It has no Endpoint Information.
   The Rust job answers the same categories (no Profile added, as in the C probe) and also Endpoint Information,
   with the conformance checks of DEC-001 and DEC-011. As a responder only (DEC-CI-001), it ignores the Profile
   messages that only an Initiator uses (Reply to Profile Details Inquiry, Profile reports); midi2 C99 has no
   handler for them either. AM is in parentheses: its probe includes its UMP processor
   (about 5 KB, the only way into its CI code), and the probe answers PE Get only.
9. Rows 10 to 12: no C library has a job with the same features. A full PE engine (transactions in more Chunks,
   Flow Control, time-outs, subscriptions) exists only in the Rust library.

## Method

- Target `thumbv7em-none-eabi` (Cortex-M4, the FLXS1), soft float. Rust: opt-level "s", fat LTO, one codegen unit,
  panic abort (`tools/size-probe`). C: `arm-none-eabi-gcc` 14.3, `-Os -flto -ffunction-sections -fdata-sections`,
  `--gc-sections`, newlib-nano (`tools/lib-compare/c/build.sh`).
- Flash = `.text + .rodata + .data`. RAM = `.data + .bss` (static RAM, buffers of the probe included).
- Each value is the probe with the job minus the probe with no job: the Rust `base` (or the USB core with no
  class), the C `none` (12 B flash, 256 B RAM of the shared `INPUT` array).
- The state of each job is a zeroed static (`.bss`) on both sides: the Rust probes keep large state in
  `MaybeUninit` (OPT-013), as a firmware does, so no `.data` image counts as flash.
- Row 9 links the `memcpy` of `compiler_builtins` (686 B); the C job links the newlib-nano `memcpy` (28 B, a byte
  loop), so 658 B of that difference is this one function. Rows 3 and 4 link no `memcpy`; in rows 7 and 8 the USB core
  of the reference build already links it.
- The C libraries are at the commits of `tools/lib-compare/fetch.sh`. Run `sh tools/lib-compare/c/run.sh`,
  `sh tools/lib-compare/tusb.sh` and `python3 scripts/size-check.py` to measure again.
- Stack is not in these tables: `docs/sizes.md` gives the Rust peak stack of each job; of the C jobs only midi2
  C99 CI is measured (about 1,192 B, its PE reply buffers are on the stack).
