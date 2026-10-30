<a id="dec-ump-004"></a>

# DEC-UMP-004 — Group and Channel are newtypes: 0-15 storage, checked constructors, number() 1-16

**Status:** accepted

**Requirements:** [REQ-UMP-008](../../requirements/zmidi-ump.md#req-ump-008), [REQ-UMP-012](../../requirements/zmidi-ump.md#req-ump-012)

The crate represents the Group field and the Channel field with two newtypes,
`Group` and `Channel`, in the `Packet` module. Each one:

- stores the 4-bit field value 0 to 15 (the value in the packet);
- has a checked constructor `new(index)` that accepts 0 to 15 and returns
  `UmpError::OutOfRange` for other values, and `from_number(n)` that
  accepts the user-facing number 1 to 16;
- gives `index()` (0 to 15) and `number()` (1 to 16);
- has const `into_bits` / `from_bits`, so a bitfield field can have the type
  directly (`#[bits(4)] group: Group`). `from_bits` takes the low 4 bits and
  is total, because all 16 values are valid.

Every message type with a Group or Channel field uses these types in its
accessors and constructors. Groupless messages (Message Type 0x0 and 0xF) have
no Group (REQ-UMP-009).

## Context

REQ-UMP-008 needs the 4-bit value 0 to 15 in the packet and the number 1 to
16 where a group is shown to users. REQ-UMP-012 needs Group and Channel to
stay separate. With plain `u8` values, a caller can swap Group and Channel,
or give a 1-based number where the packet needs a 0-based value, and the
compiler does not see the error.

## Consequences

- A Group and a Channel cannot be swapped by mistake: the types differ.
- No message constructor needs a range check for Group or Channel; the
  newtype has already checked it.
- `index()` and `number()` make the 0-based and 1-based forms explicit at
  each use.

## Alternatives

- Plain `u8` fields with `#[bits(4)]`: rejected. No type separation, and
  out-of-range values panic in debug builds or are truncated in release
  builds.
- One shared `U4` type for both: rejected. It keeps the range check but
  loses the separation that REQ-UMP-012 needs.
- Store the 1-based number: rejected. The packet holds 0 to 15, so every read
  and write would need a conversion.

## Verification

Tests: `new` accepts 0 to 15 and rejects 16 and above; `from_number` accepts
1 to 16 and rejects 0 and 17; `number() == index() + 1`; `from_bits`
ignores bits above bit 3; a compile-fail doctest shows that a `Channel`
cannot be passed where a `Group` is expected.

## Source

m2-104-um

## Codegen include

text, consequences, verification
