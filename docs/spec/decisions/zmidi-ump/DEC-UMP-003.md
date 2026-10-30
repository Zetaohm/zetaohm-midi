<a id="dec-ump-003"></a>

# DEC-UMP-003 — Property-based tests with proptest, in addition to specification vectors

**Status:** accepted

`zmidi-ump` tests use two kinds of test:

1. Specification vectors (primary, DEC-001): fixed words from the figures and
   tables of the specification, one test for each rule.
2. Property tests with `proptest` (secondary): a rule that holds for every
   input is checked against generated inputs. Properties used:
   - round trip: for every valid field value, decoding the encoded words
     gives the same message;
   - reserved bits: any pattern of reserved bits decodes to the same value as
     the word with them cleared;
   - robustness: parse and write never panic on any word slice or buffer
     length, and an unknown packet keeps its words unchanged;
   - range checks: a checked constructor accepts exactly the range the
     specification gives.

`proptest` is a dev-dependency only. Property tests sit in the module test
module, next to the vector tests, and are named `prop_<rule>`. Each one is a
plain `#[test] fn` that uses the closure form `proptest!(|(x in ..)| { .. })`,
so `specdev-analyzer report tests` finds it and `verified_by` can name it
(the analyzer does not see tests inside a `proptest! { }` block).

## Context

The vector tests check selected values. The field ranges are large (16-bit and
20-bit values, 2^32 word patterns), and a rule such as "reserved bits are
ignored" must hold for every pattern. A hand-written random loop existed
(`ump_parse_and_write_never_panic`, now replaced) but had no shrinking to a
minimal case.

## Consequences

- `proptest` is in `[dev-dependencies]`; the library and the bare-metal gate
  (`cargo check --lib --target thumbv7em-none-eabihf`) do not build it.
- A property test never replaces a specification vector. A requirement's
  `verified_by` names a vector test first.
- A failing property shrinks to a minimal input; that input is added as a
  fixed regression vector.

## Alternatives

- `quickcheck`: rejected. Smaller strategy library and weaker shrinking for
  structured inputs.
- Fuzzing (`cargo-fuzz`, `bolero`): useful later for the transport parsers;
  it needs a nightly toolchain or a separate target and does not run in
  `cargo test`.
- Only hand-written random loops: rejected. No shrinking, and each loop
  repeats generator code.

## Verification

`cargo test -p zmidi-ump` runs the `prop_` tests; the bare-metal gate passes
without building `proptest`.

## Codegen include

text, consequences
