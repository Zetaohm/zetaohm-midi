<a id="dec-pe-002"></a>

# DEC-PE-002 — Transaction readings: statuses, Chunk gaps, timeouts, retransmit, Flow Control

**Status:** accepted

**Requirements:** [REQ-PE-END-009](../../requirements/zmidi-ci-pe.md#req-pe-end-009), [REQ-PE-END-011](../../requirements/zmidi-ci-pe.md#req-pe-end-011), [REQ-PE-END-012](../../requirements/zmidi-ci-pe.md#req-pe-end-012), [REQ-PE-END-016](../../requirements/zmidi-ci-pe.md#req-pe-end-016), [REQ-PE-FLOW-005](../../requirements/zmidi-ci-pe.md#req-pe-flow-005), [REQ-PE-FLOW-007](../../requirements/zmidi-ci-pe.md#req-pe-flow-007), [REQ-PE-FLOW-008](../../requirements/zmidi-ci-pe.md#req-pe-flow-008), [REQ-PE-FLOW-011](../../requirements/zmidi-ci-pe.md#req-pe-flow-011), [REQ-PE-FLOW-012](../../requirements/zmidi-ci-pe.md#req-pe-flow-012), [REQ-PE-FLOW-013](../../requirements/zmidi-ci-pe.md#req-pe-flow-013), [REQ-PE-FLOW-014](../../requirements/zmidi-ci-pe.md#req-pe-flow-014), [REQ-PE-RES-001](../../requirements/zmidi-ci-pe.md#req-pe-res-001), [REQ-PE-STAT-008](../../requirements/zmidi-ci-pe.md#req-pe-stat-008), [REQ-PE-STAT-011](../../requirements/zmidi-ci-pe.md#req-pe-stat-011), [REQ-PE-STAT-012](../../requirements/zmidi-ci-pe.md#req-pe-stat-012), [REQ-PE-SUB-007](../../requirements/zmidi-ci-pe.md#req-pe-sub-007), [REQ-PE-SUB-015](../../requirements/zmidi-ci-pe.md#req-pe-sub-015), [REQ-PE-TXN-003](../../requirements/zmidi-ci-pe.md#req-pe-txn-003)

The readings of the ambiguity register, "Decisions needed",
items 3, 4, 12, 13 and 14, as written in the requirements that name this decision.

- Item 3, statuses that M2-103 does not give: 405 for a Set that "canSet" refuses (TXN-003); 405 for
  a "start" with no "canSubscribe", 343 when the subscription table is full (SUB-007); 400 for an
  unknown "subscribeId", 200 for "end" (SUB-015); 400 for no "resId" (RES-001); 413 for a refused
  zlib window (ENC-009, STAT-011); 415 for an unknown "mutualEncoding" (STAT-012).
- Item 4, Chunk gap with no Flow Control: NAK 0x21 at once and the transaction closes; 342 for a
  final Chunk 0x0000 with data missing and for decode errors (STAT-008). A Number of Chunks of 0
  (unknown total) is not a gap.
- Item 12, timeouts: the responder closes a Set with no next Chunk at the deadline (END-009); a Flow
  Control receiver waits 12 s (FLOW-014); Notify 100 adds 3 s (END-012); 3 s counted from the last
  Chunk, an ACK or NAK does not restart it (END-011).
- Item 13, retransmit: 3 retransmits after the first send; resend from "Last successful" + 1, else
  NAK 0x20 (FLOW-012, FLOW-013); NAK 0x12 for a gap with Flow Control, "Last successful" 0 when no
  Chunk came (FLOW-011).
- Item 14, Flow Control interop: an early 406/407 reply after Chunk 1 stops the initiator (FLOW-007,
  FLOW-008); a NAK 0x00 that answers an ACK does not end a transaction (END-016, FLOW-005); the JUCE
  NAK carries Original Transaction Sub-ID#2 0x7D.

## Context

M2-103-UM v1.2 does not state these values. The review of 2026-10-15 gave each a recommendation
below 90% confidence; the library survey (JUCE, MIDI 2.0 Workbench, ktmidi) found no conflicting
practice: JUCE and ktmidi have no timeouts, retransmit or gap checks; Workbench agrees on NAK 0x21
for a reply gap and on Notify 100 + 3 s.

## Consequences

The Transaction engine is stricter than the other implementations on Chunk gaps and deadlines.
JUCE answers every ACK with NAK 0x00 and ignores "flowControl", so the engine must keep a transaction
open after that NAK.

## Source

m2-103-um
