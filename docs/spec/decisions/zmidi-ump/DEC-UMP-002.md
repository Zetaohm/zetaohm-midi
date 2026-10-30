<a id="dec-ump-002"></a>

# DEC-UMP-002 — Typed message API: read-only headers, checked constructors, normalizing decode

**Status:** accepted

**Requirements:** [REQ-CV-MODE-001](../../requirements/zmidi-ump.md#req-cv-mode-001), [REQ-CV1-001](../../requirements/zmidi-ump.md#req-cv1-001), [REQ-CV1-002](../../requirements/zmidi-ump.md#req-cv1-002), [REQ-CV1-003](../../requirements/zmidi-ump.md#req-cv1-003), [REQ-CV1-004](../../requirements/zmidi-ump.md#req-cv1-004), [REQ-CV1-005](../../requirements/zmidi-ump.md#req-cv1-005), [REQ-CV1-006](../../requirements/zmidi-ump.md#req-cv1-006), [REQ-CV1-007](../../requirements/zmidi-ump.md#req-cv1-007), [REQ-CV1-008](../../requirements/zmidi-ump.md#req-cv1-008), [REQ-CV2-001](../../requirements/zmidi-ump.md#req-cv2-001), [REQ-CV2-002](../../requirements/zmidi-ump.md#req-cv2-002), [REQ-CV2-003](../../requirements/zmidi-ump.md#req-cv2-003), [REQ-CV2-004](../../requirements/zmidi-ump.md#req-cv2-004), [REQ-CV2-005](../../requirements/zmidi-ump.md#req-cv2-005), [REQ-CV2-006](../../requirements/zmidi-ump.md#req-cv2-006), [REQ-CV2-007](../../requirements/zmidi-ump.md#req-cv2-007), [REQ-CV2-008](../../requirements/zmidi-ump.md#req-cv2-008), [REQ-CV2-009](../../requirements/zmidi-ump.md#req-cv2-009), [REQ-CV2-010](../../requirements/zmidi-ump.md#req-cv2-010), [REQ-CV2-011](../../requirements/zmidi-ump.md#req-cv2-011), [REQ-CV2-012](../../requirements/zmidi-ump.md#req-cv2-012), [REQ-CV2-013](../../requirements/zmidi-ump.md#req-cv2-013), [REQ-CV2-020](../../requirements/zmidi-ump.md#req-cv2-020), [REQ-FLEX-001](../../requirements/zmidi-ump.md#req-flex-001), [REQ-FLEX-002](../../requirements/zmidi-ump.md#req-flex-002), [REQ-FLEX-003](../../requirements/zmidi-ump.md#req-flex-003), [REQ-FLEX-004](../../requirements/zmidi-ump.md#req-flex-004), [REQ-FLEX-005](../../requirements/zmidi-ump.md#req-flex-005), [REQ-FLEX-006](../../requirements/zmidi-ump.md#req-flex-006), [REQ-FLEX-007](../../requirements/zmidi-ump.md#req-flex-007), [REQ-FLEX-008](../../requirements/zmidi-ump.md#req-flex-008), [REQ-FLEX-009](../../requirements/zmidi-ump.md#req-flex-009), [REQ-FLEX-010](../../requirements/zmidi-ump.md#req-flex-010), [REQ-FLEX-011](../../requirements/zmidi-ump.md#req-flex-011), [REQ-FLEX-012](../../requirements/zmidi-ump.md#req-flex-012), [REQ-FLEX-013](../../requirements/zmidi-ump.md#req-flex-013), [REQ-FLEX-014](../../requirements/zmidi-ump.md#req-flex-014), [REQ-FLEX-015](../../requirements/zmidi-ump.md#req-flex-015), [REQ-MDS-001](../../requirements/zmidi-ump.md#req-mds-001), [REQ-MDS-002](../../requirements/zmidi-ump.md#req-mds-002), [REQ-MFR-001](../../requirements/zmidi-ump.md#req-mfr-001), [REQ-STREAM-001](../../requirements/zmidi-ump.md#req-stream-001), [REQ-STREAM-002](../../requirements/zmidi-ump.md#req-stream-002), [REQ-STREAM-003](../../requirements/zmidi-ump.md#req-stream-003), [REQ-STREAM-004](../../requirements/zmidi-ump.md#req-stream-004), [REQ-STREAM-005](../../requirements/zmidi-ump.md#req-stream-005), [REQ-STREAM-006](../../requirements/zmidi-ump.md#req-stream-006), [REQ-STREAM-007](../../requirements/zmidi-ump.md#req-stream-007), [REQ-STREAM-008](../../requirements/zmidi-ump.md#req-stream-008), [REQ-STREAM-009](../../requirements/zmidi-ump.md#req-stream-009), [REQ-STREAM-010](../../requirements/zmidi-ump.md#req-stream-010), [REQ-STREAM-011](../../requirements/zmidi-ump.md#req-stream-011), [REQ-STREAM-012](../../requirements/zmidi-ump.md#req-stream-012), [REQ-STREAM-013](../../requirements/zmidi-ump.md#req-stream-013), [REQ-STREAM-014](../../requirements/zmidi-ump.md#req-stream-014), [REQ-SX7-001](../../requirements/zmidi-ump.md#req-sx7-001), [REQ-SX7-002](../../requirements/zmidi-ump.md#req-sx7-002), [REQ-SX7-003](../../requirements/zmidi-ump.md#req-sx7-003), [REQ-SX8-001](../../requirements/zmidi-ump.md#req-sx8-001), [REQ-SX8-002](../../requirements/zmidi-ump.md#req-sx8-002), [REQ-SX8-003](../../requirements/zmidi-ump.md#req-sx8-003), [REQ-SYS-001](../../requirements/zmidi-ump.md#req-sys-001), [REQ-SYS-002](../../requirements/zmidi-ump.md#req-sys-002), [REQ-UMP-003](../../requirements/zmidi-ump.md#req-ump-003), [REQ-UMP-004](../../requirements/zmidi-ump.md#req-ump-004), [REQ-UMP-006](../../requirements/zmidi-ump.md#req-ump-006), [REQ-UMP-007](../../requirements/zmidi-ump.md#req-ump-007), [REQ-UMP-013](../../requirements/zmidi-ump.md#req-ump-013), [REQ-UTIL-004](../../requirements/zmidi-ump.md#req-util-004), [REQ-UTIL-005](../../requirements/zmidi-ump.md#req-util-005)

API choices for typed message values (for example `JrClock`). The
specification rules they implement are cited in `source_ref` and are not
repeated here.

- Message Type, Status and Status Bank fields are read-only
  (`access = RO`). `new()` sets them; no public setter changes them.
- A message type with a field whose valid range is smaller than its bit
  width has no `new()` and no `Default` (`new = false`, `default = false`).
  Its only public constructor is a checked `try_new` that returns
  `UmpError::OutOfRange`.
- `UmpWords::from_words` checks Message Type and Status (and Status Bank) and
  returns `UmpError::UnexpectedMessage` on a mismatch.
- `UmpWords::from_words` clears reserved bits, so a typed value never holds
  them. `Ump::Unknown` keeps its raw words unchanged. Code that forwards
  packets without interpreting them uses raw words, not typed values.
- Message structs use `conversion = false`, so the macro does not generate a
  public `from_bits` or `into_bits`; callers get the words with `to_words`.
  Inside the crate a value is built from raw bits only after the checks
  above. A typed value comes only from `new`, `try_new` or `from_words`.

## Context

`bitfield-struct` generates a public `new()`, `Default` and a setter for each
public field, keeps reserved bits in `from_bits`, compares them in the
derived `PartialEq`, and writes them again in `into_bits` (confirmed by a
scratchpad test). Without these choices a typed value could hold a reserved
value, a changed Status, or non-zero reserved bits.

## Consequences

- Each message type defines a const mask of its reserved bits for
  `from_words`.
- A public `from_bits` would skip every check above, so message structs do
  not have one.
- A decode-then-encode cycle does not keep reserved bits; raw words do.

## Alternatives

- Keep reserved bits in typed values: rejected. Equality and re-encoding
  would then depend on them.
- Public setters with debug-only panics (the `bitfield-struct` default):
  rejected. A release build would store a truncated value silently.

## Verification

Tests: a packet with non-zero reserved bits decodes equal to the cleared
packet; `try_new` rejects each out-of-range value; `from_words` on another
message's words returns `UnexpectedMessage`.

## Source

m2-104-um

## Codegen include

text, consequences, verification
