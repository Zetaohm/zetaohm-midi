<a id="dec-ump-006"></a>

# DEC-UMP-006 — MDS Number of Valid Bytes counts the Header and each Payload UMP, no pad bytes

**Status:** accepted

**Requirements:** [REQ-MDS-001](../../requirements/zmidi-ump.md#req-mds-001), [REQ-MDS-002](../../requirements/zmidi-ump.md#req-mds-002)

The Number of Valid Bytes in This Chunk field of a Mixed Data Set Header
(M2-104-UM 7.9, Figure 86) counts bytes of the UMPs of the Chunk: the 16 bytes
of the Header, and for each Payload UMP its 2 prefix bytes (mt, group, status,
MDS ID) and its data bytes. Pad bytes at the end of the last Payload are not
counted. A Chunk with L data bytes has the value 16 + 2 * ceil(L / 14) + L. A
Header-only Chunk has the value 16 (7.9.1). The largest Chunk holds 57329 data
bytes (value 65535).

A reader gets the number of Payload UMPs as ceil((value - 16) / 16) and the
number of data bytes as value - 16 - 2 * (number of Payload UMPs). A value
below 16 is not valid, and so is a value where (value - 16) mod 16 = 1: the
last Payload UMP would hold less than its 2 prefix bytes.

## Context

OQ-UMP-002. 7.9 says the field "contains the size of this Mixed Data Set
Message Chunk in bytes including the header", that the number of Payload UMPs
follows from it, and that the sender pads the last Payload when the value "is
not an integer multiple of 16". 7.9.1 gives 16 for a Header-only Chunk. The
text is the same in v1.0 (2020) and v1.1.2 (2023). Only this reading agrees
with all three rules. Review on 2026-10-14 of implementations:
ni-midi2 v1.12.0 (Native Instruments, midi2-dev, 2026-07) writes
16 + 2 * packets + data and its collector decodes the inverse, with test
vectors. The MIDI 2.0 Workbench (sendFileOverMDS, translations.js) and
cmidi2/ktmidi count the data bytes only, without the Header. Windows MIDI
Services, CoreMIDI and AM_MIDI2.0Lib take the value from the caller. The user
accepted this reading on 2026-10-14.

## Consequences

- MdsChunkPackets computes the value; MdsHeader::try_new rejects a value below
  16 (REQ-MDS-001, REQ-MDS-002).
- The crate interoperates with ni-midi2. A peer that counts data bytes only
  (MIDI 2.0 Workbench, cmidi2) gives a value that is 16 + 2 * packets too
  small by this rule; a Chunk from such a peer reads as fewer data bytes.
- If the MIDI Association publishes a clarification that disagrees, a new
  decision supersedes this one.

## Alternatives

- Data bytes only, with the Header (16 + L): no implementation found; the pad
  rule would need "multiple of 14".
- Data bytes only, without the Header (L): MIDI 2.0 Workbench and cmidi2.
  Rejected because it contradicts "including the header" and the value 16 of
  7.9.1 (DEC-001: the specification wins).

## Verification

Tests mds_chunk_packets_write_one_chunk and mds_chunks_match_ni_midi2_vectors
(the five ni-midi2 vectors, word for word), and prop_data128_writers_round_trip.

## Source

m2-104-um

## Codegen include

text, consequences
