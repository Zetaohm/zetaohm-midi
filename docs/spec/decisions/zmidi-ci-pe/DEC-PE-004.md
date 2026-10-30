<a id="dec-pe-004"></a>

# DEC-PE-004 — zlib+Mcoded7: own streaming inflate (about 1 KB state) replaces miniz_oxide

**Status:** accepted

**Supersedes:** [DEC-PE-001](../../decisions/zmidi-ci-pe/DEC-PE-001.md)

**Requirements:** [REQ-PE-DATA-002](../../requirements/zmidi-ci-pe.md#req-pe-data-002), [REQ-PE-ENC-009](../../requirements/zmidi-ci-pe.md#req-pe-enc-009), [REQ-PE-ENC-010](../../requirements/zmidi-ci-pe.md#req-pe-enc-010)

OPT-010 option 1 (2026-10-18). This decision replaces DEC-PE-001 and keeps
all its parts except the inflate core.

The encoding "zlib+Mcoded7" (M2-103-UM 6.1.6 Table 12, 6.2.2, 6.2.4) is behind a Cargo feature
`zlib`, off by default. Without the feature, the encoding gives `Unsupported`.

Inflate: an own streaming zlib (RFC 1950) and deflate (RFC 1951) decoder in the private module
`inflate`, with no alloc, no `unsafe` and no dependency. The Huffman decoding follows `puff.c` of
Mark Adler (the zlib reference decoder): canonical codes with a count and a symbol table for each
code. An over-subscribed code set is an error; an incomplete literal/length or distance code set is
an error unless it has one code of length 1; the code length code set must be complete. Each step
checks that its bits are there before it consumes any, so the data streams Chunk by Chunk. The
window is a caller buffer (a power of two, at most 32 KiB). A stream whose zlib header (CINFO)
declares a window larger than the caller buffer gives `WindowTooLarge`; a stream that is not valid
zlib (header, FDICT, block type, lengths, codes, a distance beyond the output or the window, the
Adler-32 trailer) gives `InvalidZlib`; never a panic. The Transaction module replies 413 to a
window that is too large (REQ-PE-STAT-011).

Deflate: unchanged from DEC-PE-001: the writer writes the zlib header, stored deflate blocks (RFC
1951 BTYPE 00) and the Adler-32 check into a caller buffer, with an overflow error when it is too
small. A device writes zlib only for a Resource whose "encodings" list names it.

## Context

OPT-010 (docs/optimization-backlog.md): the miniz_oxide decompressor kept full Huffman fast-lookup
tables, so `ZlibInflater` was 10516 bytes and `ZlibMcoded7Decoder` 10596, and `new` built them by
value: about 21 KB of stack for one decoder on the stack. Measured on thumbv7em-none-eabihf,
opt-level "s", 2026-10-18.

## Consequences

- `ZlibInflater` 10516 -> 1072 bytes, `ZlibMcoded7Decoder` 10596 -> 1152; a decoder made and used on
  the stack needs 2768 bytes of stack (was about 21 KB); the probe code is about 4.3 KB smaller.
  The feature needs about 34 KB of RAM with a 32 KiB window (was about 43 KB).
- The `zlib` feature has no dependency; miniz_oxide stays a dev-dependency (the reference compressor
  and decoder of the tests).
- The decoder reads one bit at a time to decode a symbol (no lookup table), so it is slower than
  miniz_oxide; Property Exchange data is small and arrives in Chunks, so the speed is not a limit.
- The public API of `ZlibInflater` and `ZlibMcoded7Decoder` does not change.

## Alternatives

- Keep miniz_oxide and document its RAM need (OPT-010 option 2): rejected; option 1 was chosen.
- A table-driven decoder (like zlib inflate_fast): faster, but its tables are several KB, which is
  the cost this decision removes.
- Another deflate crate with no alloc: none found that builds with no alloc (DEC-PE-001 survey).

## Verification

Tests in `zlib.rs`: the existing vectors, window, bad-stream, never-panic and round-trip tests; a
differential test against the reference compressor at levels 0 to 10 (stored, fixed and dynamic
blocks; data up to 100 KB, so that distances wrap) in random input and output piece sizes; a
differential test of changed streams against the reference decoder (both give the same output or
both give an error); a stream with CINFO 0 and a 256-byte window. Run once with 2560 and 40000
cases on 2026-10-18 with no disagreement. `cargo check --no-default-features --features zlib` for
thumbv7em-none-eabihf; `scripts/stack-check.py --features zlib`.

## Source

m2-103-um

## Codegen include

text, consequences, verification
