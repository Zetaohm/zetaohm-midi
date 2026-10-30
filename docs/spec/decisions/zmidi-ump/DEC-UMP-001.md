<a id="dec-ump-001"></a>

# DEC-UMP-001 — UMP messages are bitfield-struct types in spec field order, dispatched by match

**Status:** accepted

**Requirements:** [REQ-CV-MODE-001](../../requirements/zmidi-ump.md#req-cv-mode-001), [REQ-CV1-001](../../requirements/zmidi-ump.md#req-cv1-001), [REQ-CV1-002](../../requirements/zmidi-ump.md#req-cv1-002), [REQ-CV1-003](../../requirements/zmidi-ump.md#req-cv1-003), [REQ-CV1-004](../../requirements/zmidi-ump.md#req-cv1-004), [REQ-CV1-005](../../requirements/zmidi-ump.md#req-cv1-005), [REQ-CV1-006](../../requirements/zmidi-ump.md#req-cv1-006), [REQ-CV1-007](../../requirements/zmidi-ump.md#req-cv1-007), [REQ-CV1-008](../../requirements/zmidi-ump.md#req-cv1-008), [REQ-CV2-001](../../requirements/zmidi-ump.md#req-cv2-001), [REQ-CV2-002](../../requirements/zmidi-ump.md#req-cv2-002), [REQ-CV2-003](../../requirements/zmidi-ump.md#req-cv2-003), [REQ-CV2-004](../../requirements/zmidi-ump.md#req-cv2-004), [REQ-CV2-005](../../requirements/zmidi-ump.md#req-cv2-005), [REQ-CV2-006](../../requirements/zmidi-ump.md#req-cv2-006), [REQ-CV2-007](../../requirements/zmidi-ump.md#req-cv2-007), [REQ-CV2-008](../../requirements/zmidi-ump.md#req-cv2-008), [REQ-CV2-009](../../requirements/zmidi-ump.md#req-cv2-009), [REQ-CV2-010](../../requirements/zmidi-ump.md#req-cv2-010), [REQ-CV2-011](../../requirements/zmidi-ump.md#req-cv2-011), [REQ-CV2-012](../../requirements/zmidi-ump.md#req-cv2-012), [REQ-CV2-013](../../requirements/zmidi-ump.md#req-cv2-013), [REQ-CV2-020](../../requirements/zmidi-ump.md#req-cv2-020), [REQ-FLEX-001](../../requirements/zmidi-ump.md#req-flex-001), [REQ-FLEX-002](../../requirements/zmidi-ump.md#req-flex-002), [REQ-FLEX-003](../../requirements/zmidi-ump.md#req-flex-003), [REQ-FLEX-004](../../requirements/zmidi-ump.md#req-flex-004), [REQ-FLEX-005](../../requirements/zmidi-ump.md#req-flex-005), [REQ-FLEX-006](../../requirements/zmidi-ump.md#req-flex-006), [REQ-FLEX-007](../../requirements/zmidi-ump.md#req-flex-007), [REQ-FLEX-008](../../requirements/zmidi-ump.md#req-flex-008), [REQ-FLEX-009](../../requirements/zmidi-ump.md#req-flex-009), [REQ-FLEX-010](../../requirements/zmidi-ump.md#req-flex-010), [REQ-FLEX-011](../../requirements/zmidi-ump.md#req-flex-011), [REQ-FLEX-012](../../requirements/zmidi-ump.md#req-flex-012), [REQ-FLEX-013](../../requirements/zmidi-ump.md#req-flex-013), [REQ-FLEX-014](../../requirements/zmidi-ump.md#req-flex-014), [REQ-FLEX-015](../../requirements/zmidi-ump.md#req-flex-015), [REQ-MDS-001](../../requirements/zmidi-ump.md#req-mds-001), [REQ-MDS-002](../../requirements/zmidi-ump.md#req-mds-002), [REQ-MFR-001](../../requirements/zmidi-ump.md#req-mfr-001), [REQ-STREAM-001](../../requirements/zmidi-ump.md#req-stream-001), [REQ-STREAM-002](../../requirements/zmidi-ump.md#req-stream-002), [REQ-STREAM-003](../../requirements/zmidi-ump.md#req-stream-003), [REQ-STREAM-004](../../requirements/zmidi-ump.md#req-stream-004), [REQ-STREAM-005](../../requirements/zmidi-ump.md#req-stream-005), [REQ-STREAM-006](../../requirements/zmidi-ump.md#req-stream-006), [REQ-STREAM-007](../../requirements/zmidi-ump.md#req-stream-007), [REQ-STREAM-008](../../requirements/zmidi-ump.md#req-stream-008), [REQ-STREAM-009](../../requirements/zmidi-ump.md#req-stream-009), [REQ-STREAM-010](../../requirements/zmidi-ump.md#req-stream-010), [REQ-STREAM-011](../../requirements/zmidi-ump.md#req-stream-011), [REQ-STREAM-012](../../requirements/zmidi-ump.md#req-stream-012), [REQ-STREAM-013](../../requirements/zmidi-ump.md#req-stream-013), [REQ-STREAM-014](../../requirements/zmidi-ump.md#req-stream-014), [REQ-SX7-001](../../requirements/zmidi-ump.md#req-sx7-001), [REQ-SX7-002](../../requirements/zmidi-ump.md#req-sx7-002), [REQ-SX7-003](../../requirements/zmidi-ump.md#req-sx7-003), [REQ-SX8-001](../../requirements/zmidi-ump.md#req-sx8-001), [REQ-SX8-002](../../requirements/zmidi-ump.md#req-sx8-002), [REQ-SX8-003](../../requirements/zmidi-ump.md#req-sx8-003), [REQ-SYS-001](../../requirements/zmidi-ump.md#req-sys-001), [REQ-SYS-002](../../requirements/zmidi-ump.md#req-sys-002), [REQ-UMP-002](../../requirements/zmidi-ump.md#req-ump-002), [REQ-UMP-003](../../requirements/zmidi-ump.md#req-ump-003), [REQ-UMP-004](../../requirements/zmidi-ump.md#req-ump-004), [REQ-UMP-005](../../requirements/zmidi-ump.md#req-ump-005), [REQ-UMP-008](../../requirements/zmidi-ump.md#req-ump-008), [REQ-UMP-009](../../requirements/zmidi-ump.md#req-ump-009), [REQ-UMP-010](../../requirements/zmidi-ump.md#req-ump-010), [REQ-UMP-012](../../requirements/zmidi-ump.md#req-ump-012), [REQ-UTIL-001](../../requirements/zmidi-ump.md#req-util-001), [REQ-UTIL-002](../../requirements/zmidi-ump.md#req-util-002), [REQ-UTIL-003](../../requirements/zmidi-ump.md#req-util-003), [REQ-UTIL-004](../../requirements/zmidi-ump.md#req-util-004), [REQ-UTIL-005](../../requirements/zmidi-ump.md#req-util-005), [REQ-UTIL-006](../../requirements/zmidi-ump.md#req-util-006)

Each UMP message is one `#[bitfield(<int>, order = Msb)]` struct from the
`bitfield-struct` crate:
- The storage integer is the packet size: `u32`, `u64` or `u128`. Word 0 is
  the most significant 32 bits.
- Fields are declared top to bottom in the order of the spec diagram, so the
  struct reads like the figure. Reserved fields are private `__` fields.
- `new()` sets Message Type and Status (and Status Bank, where present) to the
  values for that message, through `#[bits(.., default = ..)]`.
- Spec value sets (Message Type, Status, Form) are Rust enums with `const fn`
  `into_bits` / `from_bits`.

A top-level `Ump` enum has one variant per message plus an `Unknown` variant
that keeps the raw words. Parsing reads the Message Type and Status from
word 0 and selects the struct with a `match`. Enum variants that carry data
are never placed inside a bitfield.

Multi-packet messages are read through iterators over `&[u32]`, written into
a caller buffer, and collected in fixed-capacity assemblers (DEC-002 at the
workspace tier).

## Context

The earlier code used a private fork of `modular-bitfield`. That crate packs
fields least-significant-bit first, so UMP fields had to be declared in
reverse or byte-swapped. The fork also added enum data variants inside
bitfields, and a failed test showed that this dispatch put a MIDI 1.0 Note On
into a Utility payload. A scratchpad trial with `bitfield-struct` 0.13
(`u128`, `order = Msb`, enum defaults) gave the spec bytes directly and built
for `thumbv7em-none-eabihf` with no `alloc`.

## Consequences

- `modular-bitfield` and its fork are removed from the crate.
- The project's bitfield rules change to this pattern.
- Field types are plain integers (`u8`, `u16`, `u32`) or `#[bits(n)]` widths.
  `with_*` panics in debug builds on an out-of-range value; `with_*_checked`
  returns an error. Public constructors that take user values use the
  checked form (REQ-UMP-006: never panic).
- One `match` arm per message is written by hand; a message that is not
  matched falls into `Unknown` (REQ-UMP-005).

## Alternatives

- `modular-bitfield` (upstream or fork): rejected. LSB-first only, and the
  needed features exist only in a private fork.
- `bitbybit`: acceptable, explicit bit ranges. Rejected because field order
  in the struct already gives the spec layout with `bitfield-struct`.
- `midi2` crate: rejected by the project owner; the UMP layer stays in house.
- Hand-written shifts and masks: rejected. More code, less readable than the
  spec-ordered struct.

## Verification

Each message has a test that compares `new().with_*(..)` output words with
the bit layout in the spec figure. A no-`alloc` build for
`thumbv7em-none-eabihf` passes.

## Source

m2-104-um

## Codegen include

text, consequences, verification
