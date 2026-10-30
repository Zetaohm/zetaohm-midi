<a id="dec-usb-002"></a>

# DEC-USB-002 — Fixed function: descriptors made by a build script, beside FunctionConfig

**Status:** accepted

**Requirements:** [REQ-USB-FIX-001](../../requirements/zmidi-usb.md#req-usb-fix-001), [REQ-USB-FIX-002](../../requirements/zmidi-usb.md#req-usb-fix-002), [REQ-USB-FIX-003](../../requirements/zmidi-usb.md#req-usb-fix-003)

A USB MIDI function whose topology is known when the firmware is built uses `zmidi_usb::fixed`:
a build script (or any host program) calls `fixed::write_rust` with a normal `FunctionConfig`;
it runs every check of `write_function` and `gtb_set` and writes the Rust source of one
`FixedFunction` constant (the descriptor bytes for interface numbers 0 and 1 and no strings, the
endpoints, the Group mask of each Alternate Setting, the Group Terminal Block sets). The firmware
`include!`s it. `FixedFunction::replay` gives the descriptors to a `DescriptorSink` with the
interface numbers, the AudioControl baInterfaceNr and the string indexes that the USB stack gives;
`FixedFunction::gtb_setup` answers the Group Terminal Block request with the rules of
`handle_gtb_setup`. `FunctionConfig` stays, beside it, for a topology that the firmware makes when
it runs. Both adapters (`usb-device`, `embassy-usb`) take either through the trait
`zmidi_usb::fixed::Function` (implemented for `FunctionConfig` and for `fixed::Fixed`, a
`FixedFunction` with the ids of the USB stack): `MidiClass::new` for a `FunctionConfig`,
`MidiClass::fixed` for a `FixedFunction`, `MidiClass::with_function` for any `Function`.

## Context

OPT-016 (2026-10-20): beside the class, as a build.rs generator. The descriptor
build and checks are about 6.5 KB of the class, and the class keeps the FunctionConfig for its
endpoint and Group tables. A const fn builder was rejected: it is a second implementation of the
descriptor rules. zmidi-usb stays no_std: write_rust writes into a core::fmt::Write.

## Consequences

- The fixed path links no descriptor builder and no check: the USB class (usb2 minus usb_core in
  tools/size-probe) is 9,836 B instead of 20,320 B, and 464 B of RAM instead of 644 B.
- A firmware with a fixed function needs a build script with zmidi-usb as a build-dependency.
- The fields of FixedFunction are private (addendum of 2026-10-26); the only constructor is the
  const fn FixedFunction::new, which checks the endpoint rules of the adapters, and the source of
  write_rust calls it in a constant. Descriptor bytes are not checked again; replay never panics on
  them (it stops at a broken length).
- `MidiClass` of the `usb-device` adapter is generic over `F: Function` (default
  `FunctionConfig<'a>`): `MidiClass<'a, Bus, F, CABLES, EPS, X>` replaces
  `MidiClass<'a, Bus, CABLES, J, S, E, B, EPS, X>` (the crates are 0.1.0, not published).
- The adapters re-export `Function`, `FunctionIds` and `Fixed` from `zmidi_usb::fixed`. The
  embassy-usb adapter writes the bytes of `Function::write` into a buffer and walks them as
  before; its `State` and `MidiClass` are generic over `F: Function` (default
  `FunctionConfig<'d>`) in place of `J, S, E, B`. Its class (tools/size-probe, stub driver) is
  4,780 B with the fixed function instead of 15,264 B, and 304 B of RAM instead of 484 B.

## Verification

- zmidi-usb tests/fixed.rs: the stored fixtures are the output of write_rust; replay gives the
  bytes of write_function for other interface numbers and strings; gtb_setup gives the answers
  of handle_gtb_setup for each interface, Alternate Setting and wLength.
- zmidi-usb-device tests: a fixed class gives the configuration descriptor, the strings and the
  Group Terminal Block answer of the run-time class.
- scripts/size-check.py job usb_class_fixed.

## Codegen include

text, consequences, verification
