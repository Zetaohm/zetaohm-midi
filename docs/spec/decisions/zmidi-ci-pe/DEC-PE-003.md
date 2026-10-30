<a id="dec-pe-003"></a>

# DEC-PE-003 — zmidi-ci-pe readings: items 1, 5-11, 15 and AMB-M2-103-61

**Status:** accepted

**Requirements:** [REQ-PE-DATA-003](../../requirements/zmidi-ci-pe.md#req-pe-data-003), [REQ-PE-ENC-005](../../requirements/zmidi-ci-pe.md#req-pe-enc-005), [REQ-PE-ENC-006](../../requirements/zmidi-ci-pe.md#req-pe-enc-006), [REQ-PE-ENC-008](../../requirements/zmidi-ci-pe.md#req-pe-enc-008), [REQ-PE-END-008](../../requirements/zmidi-ci-pe.md#req-pe-end-008), [REQ-PE-END-009](../../requirements/zmidi-ci-pe.md#req-pe-end-009), [REQ-PE-END-011](../../requirements/zmidi-ci-pe.md#req-pe-end-011), [REQ-PE-HDR-002](../../requirements/zmidi-ci-pe.md#req-pe-hdr-002), [REQ-PE-HDR-003](../../requirements/zmidi-ci-pe.md#req-pe-hdr-003), [REQ-PE-HDR-006](../../requirements/zmidi-ci-pe.md#req-pe-hdr-006), [REQ-PE-JSON-006](../../requirements/zmidi-ci-pe.md#req-pe-json-006), [REQ-PE-JSON-007](../../requirements/zmidi-ci-pe.md#req-pe-json-007), [REQ-PE-LINK-001](../../requirements/zmidi-ci-pe.md#req-pe-link-001), [REQ-PE-LINK-002](../../requirements/zmidi-ci-pe.md#req-pe-link-002), [REQ-PE-LINK-003](../../requirements/zmidi-ci-pe.md#req-pe-link-003), [REQ-PE-LINK-004](../../requirements/zmidi-ci-pe.md#req-pe-link-004), [REQ-PE-LINK-005](../../requirements/zmidi-ci-pe.md#req-pe-link-005), [REQ-PE-LINK-006](../../requirements/zmidi-ci-pe.md#req-pe-link-006), [REQ-PE-LINK-007](../../requirements/zmidi-ci-pe.md#req-pe-link-007), [REQ-PE-LIST-001](../../requirements/zmidi-ci-pe.md#req-pe-list-001), [REQ-PE-LIST-003](../../requirements/zmidi-ci-pe.md#req-pe-list-003), [REQ-PE-LIST-004](../../requirements/zmidi-ci-pe.md#req-pe-list-004), [REQ-PE-LIST-005](../../requirements/zmidi-ci-pe.md#req-pe-list-005), [REQ-PE-LIST-006](../../requirements/zmidi-ci-pe.md#req-pe-list-006), [REQ-PE-LIST-007](../../requirements/zmidi-ci-pe.md#req-pe-list-007), [REQ-PE-LIST-008](../../requirements/zmidi-ci-pe.md#req-pe-list-008), [REQ-PE-LIST-009](../../requirements/zmidi-ci-pe.md#req-pe-list-009), [REQ-PE-LIST-010](../../requirements/zmidi-ci-pe.md#req-pe-list-010), [REQ-PE-LIST-011](../../requirements/zmidi-ci-pe.md#req-pe-list-011), [REQ-PE-RES-002](../../requirements/zmidi-ci-pe.md#req-pe-res-002), [REQ-PE-STAT-005](../../requirements/zmidi-ci-pe.md#req-pe-stat-005), [REQ-PE-STAT-007](../../requirements/zmidi-ci-pe.md#req-pe-stat-007), [REQ-PE-STAT-012](../../requirements/zmidi-ci-pe.md#req-pe-stat-012), [REQ-PE-SUB-001](../../requirements/zmidi-ci-pe.md#req-pe-sub-001), [REQ-PE-SUB-004](../../requirements/zmidi-ci-pe.md#req-pe-sub-004), [REQ-PE-SUB-005](../../requirements/zmidi-ci-pe.md#req-pe-sub-005), [REQ-PE-TXN-004](../../requirements/zmidi-ci-pe.md#req-pe-txn-004), [REQ-PE-TXN-007](../../requirements/zmidi-ci-pe.md#req-pe-txn-007)

The readings of the ambiguity register, "Decisions needed",
"Open: zmidi-ci-pe requirement choices", items 1, 5, 6, 7, 8, 9, 10, 11 and 15, as written in the
requirements that name this decision, and the reading of AMB-M2-103-61.

- Item 1: the Resource-content helpers (Link, ResourceList) stay in zmidi-ci-pe, with no extra Cargo
  feature.
- Item 5: "subscribeId" parser 1 to 16 characters, writer 8.
- Item 6: the header reader gives an error only for a known Property with a nested or null value,
  and skips unknown Properties of any type.
- Item 7: long text is cut and marked ("message", "title", "confirmMsg", Column "title").
- Item 8: encoding names are compared with no case; the "Mcoded7" JSON order is escape, Mcoded7,
  Chunks; non-JSON 7-bit data can use "ASCII".
- Item 9: the Mcoded7 decoder ignores a last group of 1 byte and the unused bits; an unpaired
  surrogate becomes U+FFFD.
- Item 10: the JSON writer writes "\u" in lower case; the reader accepts both cases.
- Item 11: 200 for an applied Set, 201 accepted as success; "totalCount" only with status 200-299.
- Item 15: "endedBy" always written; the entry keeps the "start" Properties; "" JSON Pointer
  refused; "canSet":"patch" read as "none"; "resource" writer accepts "X-"; "selectAction" is an
  unknown role; ResourceList ignores "offset"/"limit".
- AMB-M2-103-61: an ACK 0x10 wait counts until the next Chunk.

## Context

M2-103-UM v1.2 does not state these values. Each had a recommendation below 90% confidence; the
library survey of 2026-10-15 (JUCE, MIDI 2.0 Workbench, ktmidi) is in the ambiguity register.

## Source

m2-103-um
