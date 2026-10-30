<a id="dec-pe-006"></a>

# DEC-PE-006 — Opt-in flat Header Data reader for the one-chunk Responder (respond_flat)

**Status:** accepted

**Requirements:** [REQ-PE-ONE-005](../../requirements/zmidi-ci-pe.md#req-pe-one-005)

`zmidi_ci_pe::one_chunk::respond_flat` is a second form of `respond` with the same arguments. It
reads the request Header Data with the rules of M2-103 7.1.1: no whitespace, the Resource first,
and each value a number, a boolean or a string. It checks each string as JSON (escapes decoded)
and takes a number or a literal (`true`, `false`, `null`) up to the next "," or "}" with no
check. Whitespace, an object or an array as a value, or bytes after the object give 400. For
every header that follows M2-103 7.1.1 it gives the same request as the complete JSON reader of
REQ-PE-HDR-014, so the replies are the same (REQ-PE-ONE-005).

The complete reader stays the default: `respond` (DEC-PE-005, REQ-PE-HDR-014). The choice is a
function, not a Cargo feature: a firmware links only the form that it calls.

## Context

An opt-in tier, on the condition that it complies with the specification (2026-10-23).
Compliance review: M2-103 7.1.1 (PDF page 28) puts the Header Data rules on the sender, so that
devices with little memory and processing power can read it more easily; section 4 (PDF page
17) describes a device with a simple parser that recognizes only three inquiries; M2-103 has no
rule that a Responder checks the whole JSON syntax (7.4: each reply has a status from Table 15;
400 means that the data arrived but is not correct). The check of REQ-PE-HDR-014 is a choice of this
project (DEC-PE-005), not of the specification. Measured (scripts/size-check.py, 2026-10-23):
`pe_one_chunk` 5,172 B with `respond`, `pe_one_chunk_flat` 4,516 B with `respond_flat` (-656 B
flash), static RAM 736 B in both.

## Consequences

- A request that breaks the JSON syntax only inside a number or a literal (for example "x":00)
  gets the answer of the request from `respond_flat`, and 400 from `respond`.
- A request with whitespace in the Header Data, which M2-103 forbids, gets 400 from
  `respond_flat`; `respond` takes it.
- The member closure of the two forms is the same code, so a Property rule changes in one place.

## Alternatives

- A key search for "resource" with no JSON walk, as midi2 C99 does: rejected; it can match inside
  another string value and does not see escapes.
- A Cargo feature that switches `respond` to the flat reader: rejected; features are additive,
  so one crate in the build would change the reader of every user.

## Verification

- `prop_flat_reader_equals_full_reader_on_conforming_headers`: generated headers that follow
  M2-103 7.1.1 give the same request from both readers.
- `flat_reader_refuses_whitespace_objects_and_arrays`, `flat_responder_gives_the_same_statuses`.

## Source

m2-103-um

## Codegen include

text, consequences, verification
