<a id="dec-pe-001"></a>

# DEC-PE-001 — zlib+Mcoded7: Cargo feature zlib, miniz_oxide inflate core, stored-block writer

**Status:** superseded

**Superseded by:** [DEC-PE-004](../../decisions/zmidi-ci-pe/DEC-PE-004.md)

The encoding "zlib+Mcoded7" (M2-103-UM 6.1.6 Table 12, 6.2.2,
6.2.4) is behind a Cargo feature `zlib`, off by default. Without the feature, the encoding gives
`Unsupported`.

Inflate: the dependency miniz_oxide with default-features = false (no `with-alloc`), and only its
inflate core (`miniz_oxide::inflate::core::decompress`). The data is streamed Chunk by Chunk. The
32 KiB window is a caller buffer. A stream whose zlib header (CINFO) declares a window larger than
the caller buffer, or a stream that is not valid zlib, gives an error, never a panic; the
Transaction module replies 413 to a window that is too large (REQ-PE-STAT-011).

Deflate: miniz_oxide has no compressor with no alloc (its `deflate` module needs `with-alloc`). The
writer writes the zlib header, stored deflate blocks (RFC 1951 BTYPE 00) and the Adler-32 check,
into a caller buffer, with an overflow error when it is too small. A device must write zlib only for
a Resource whose "encodings" list names it.

## Context

OQ-PE-001. DEC-007 named the no_std core of miniz_oxide, but in miniz_oxide 0.9.1 only inflate works
with no alloc (src/lib.rs, `deflate` is gated on `with-alloc`). DEC-002 forbids alloc in the core API.

## Consequences

A new optional dependency (miniz_oxide, inflate only). The writer does not make the data smaller;
a compressor with no alloc (fixed Huffman, LZ77 in a caller buffer) can replace the stored-block
writer later with no API change. The inflate state is a fixed-size struct of about 11 KB plus the
caller's 32 KiB window.

## Alternatives

Another deflate crate with no alloc: none found that builds with no alloc (zlib-rs needs an
allocator). Our own compressor now: more code to test for little gain on 7-bit transport. Other
implementations all allocate: JUCE bundled zlib, MIDI 2.0 Workbench pako, ktmidi java.util.zip
(the ambiguity register, survey of 2026-10-15).

## Verification

Tests: inflate of a zlib stream made by a reference compressor (std-only dev test), Chunk by
Chunk; window larger than the buffer gives the error; bad Adler-32 gives the error; the stored-block
writer output inflates to the input; `cargo check --no-default-features --features zlib` for
thumbv7em-none-eabihf.

## Source

m2-103-um

## Codegen include

text, consequences, verification
