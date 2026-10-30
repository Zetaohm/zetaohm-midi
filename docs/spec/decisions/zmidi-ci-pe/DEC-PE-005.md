<a id="dec-pe-005"></a>

# DEC-PE-005 — One-chunk Responder: 413 for more Chunks, 415 for an encoding other than ASCII

**Status:** accepted

**Requirements:** [REQ-PE-ONE-001](../../requirements/zmidi-ci-pe.md#req-pe-one-001), [REQ-PE-ONE-002](../../requirements/zmidi-ci-pe.md#req-pe-one-002), [REQ-PE-ONE-003](../../requirements/zmidi-ci-pe.md#req-pe-one-003), [REQ-PE-ONE-004](../../requirements/zmidi-ci-pe.md#req-pe-one-004)

`zmidi_ci_pe::one_chunk::respond` is a small Property Exchange Responder beside `PeEngine`: it
answers an Inquiry: Get or Set Property Data whose request is one Chunk (Number of Chunks 1,
Chunk 1) from a `Resources` implementation of the application and a constant ResourceList. It
keeps no transaction, subscription or flow control state and uses the ASCII encoding only. The
replies (M2-103 7.4.1 Table 15):

- a request with Number of Chunks other than 1: 413 Payload Too Large at Chunk 1; the later
  Chunks of that request get no reply;
- "mutualEncoding" other than "ASCII" (the case of the letters is not used): 415;
- Header Data that is not valid JSON, no "resource" string, an empty "resource", or a
  "resource" or "resId" that is not a string or is longer than 36 characters: 400;
- a Set with "setPartial" true: 405 (only full Sets);
- a Subscription message: 405;
- else the status of the application, 200 with the Property Data of a Get.

The header is read with the slice walker of REQ-PE-HDR-014 (valid JSON, escapes decoded).
The Responder ends the zmidi-ci request of each Get or Set Chunk at once (`pe_end`), so the
request table of the device never fills.

## Context

OPT-019, step 6 of OPT-052 (2026-10-20). M2-103 Table 15
defines 413 and 415 but no rule for a device that supports only one Chunk or only ASCII. midi2
C99 does one-chunk PE with a key search for "resource" (no JSON check). The prototype measured
3,656 B with a key search and 5,932 B with a JSON-correct reader, against 25 to 30 KB for
PeEngine.

## Consequences

- A firmware with small Resources links about 6 KB of PE code instead of PeEngine (25 to 30 KB),
  and keeps a body buffer instead of the engine state.
- Its ResourceList must declare only what this Responder does: no canSubscribe, no canPaginate,
  canSet "full" or "none", ASCII only.
- Offset and limit (pagination) are not read; a Resource that needs them needs PeEngine.

## Verification

- tests/one_chunk.rs: Get and Set in one Chunk, escapes, the application status, 413 once for a
  request in more Chunks and for Number of Chunks 0, 415, 405 for setPartial and Subscription, 400
  cases, 40 requests through a request table of 4.
- tools/size-probe job pe_one_chunk and scripts/size-check.py job pe_one_chunk.

## Codegen include

text, consequences, verification
