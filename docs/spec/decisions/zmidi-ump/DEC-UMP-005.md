<a id="dec-ump-005"></a>

# DEC-UMP-005 — UMP text is caller-supplied NFC; an optional nfc feature checks and normalizes

**Status:** accepted

**Requirements:** [REQ-TEXT-001](../../requirements/zmidi-ump.md#req-text-001)

The core API does not normalize text. Every writer of a UMP text field (UMP
Stream names, Flex Data text, Lyrics, Ruby and the other text messages) takes
a `&str`. Its documentation says that the caller gives text in Unicode
Normalization Form C (M2-104-UM 1.8, REQ-TEXT-001). The writer checks only
what needs no Unicode tables: valid UTF-8 (given by `&str`), the field byte
limits, and no Byte Order Mark (REQ-TEXT-002). Readers give the text as
received.

An optional, non-default `nfc` feature turns on `alloc` and gives two helpers:
a check that a string is NFC, and a function that gives the NFC form of a
string. The feature uses a maintained Unicode normalization crate with
`default-features = false`.

## Context

OQ-UMP-001. M2-104-UM 1.8 says that UMP text is UTF-8 in NFC. DEC-002 says the
core API does not use `alloc`. Review on 2026-10-14: `icu_normalizer` 2.3
(built for `thumbv7em-none-eabihf` with only `is_normalized` and
`normalize_to`) fails with "no global memory allocator found", and
`unicode-normalization` 0.1.25 needs `alloc` through `tinyvec`. No maintained
NFC crate works without `alloc`. ASCII text is always NFC, and many device and
product strings are ASCII (Product Instance Id and BCP 47 language tags are
ASCII by definition).

## Consequences

- REQ-TEXT-001 is met by the caller in the core API and by the `nfc` helpers
  when `alloc` is available. The writer docs must state the NFC rule.
- Bare-metal builds without `alloc` carry no Unicode tables.
- A caller that writes non-NFC text into the core API gives non-conforming
  output; the crate does not detect it without the `nfc` feature.

## Alternatives

1. Normalize in the core writer: rejected, because no maintained NFC crate
   builds without `alloc` (DEC-002).
3. Check with an NFC_Quick_Check table in the core: rejected, because the
   crate would then keep its own Unicode table, and the check gives "maybe"
   for some text, which needs full normalization to decide.

## Verification

The bare-metal check (`cargo check --no-default-features --target
thumbv7em-none-eabihf`) builds without the `nfc` feature. With `nfc`, tests
show that the check rejects a decomposed string (e.g. "e" + U+0301) and that
normalization gives the composed form (U+00E9).

## Source

m2-104-um

## Codegen include

text, consequences, verification
