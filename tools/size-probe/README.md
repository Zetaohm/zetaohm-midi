# size-probe

One FLXS1 MIDI 2.0 job for each Cargo feature, built with the FLXS1 release profile (opt-level "s",
fat LTO, one codegen unit, panic abort) for `thumbv7em-none-eabi`. The cost of a feature is this binary
with the feature minus the binary with no feature (`base`).

- `sh measure.sh` builds each configuration into `elf/` and prints flash and RAM.
- `python3 stack.py elf/<name>...` prints the deepest stack chain of each `run`/`init`/`poll` entry.

No device crate is needed: `memory.x` is the MK20DX256 (FLXS1) map and `src/bus.rs` is a stub
`usb-device` bus, so CI can build the probe. The feature `usb_core` builds the `usb-device` core with
no class; a class costs `usb1` or `usb2` minus `usb_core`. `scripts/size-check.py` builds the job
matrix of OPT-052 and checks the budgets.
- `sh m.sh name=feat1,feat2 ...` builds one configuration for each argument and prints its flash and RAM.
- `python3 inl.py elf/NAME SYMBOL [DEPTH]` gives the bytes of one function by inlined callee (for example
  `size_probe::ci::run`), with `llvm-symbolizer --inlines`. It shows where the code of a large function comes
  from when LTO puts other functions into it.
- `sh types.sh` gives the RAM (`size_of` on `thumbv7em`) of the MIDI-CI types, with no category feature and
  with all features (crate `types/`, outside the workspace).

`prototypes-2026-10-20.patch` holds the size prototypes of the improvement plan (word paths in
zmidi-ump, zmidi-io and zmidi-usb, a const descriptor path in zmidi-usb-device, compile-time CI
gates, a u32 JSON number writer) and the probe features that measure them (`bytes1w`,
`bytes_outw`, `downw`, `up1`, `bytes1c`, `usb2c`). They are experiments, not reviewed code:
apply them on a scratch copy (`git apply`), never on a working branch. The patch fits the tree before step 3
of the plan (DEC-018): the word paths are now in the crates, so it no longer applies to the working tree.

The features `down_words` (`midi2_to_midi1`) and `bytes_core` (`BytesToUmpCore` with no translator) measure the
word APIs of DEC-018.
