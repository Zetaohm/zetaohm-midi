<a id="dec-ble-002"></a>

# DEC-BLE-002 — PacketReader word form read_words; read is the typed form on top of it

**Status:** accepted

`PacketReader` has a word form, `read_words(packet, sink)`. It gives each UMP of a packet to the sink as
`ReceivedWords { timestamp, words }`, where `words` are the words that the zmidi-io `BytesToUmp` of the reader
makes (one UMP, 1 or 2 words), with no `Ump::parse`. Errors are the same `ReadError` values, in the same order.

`read(packet, sink)` stays, with the same signature and behaviour. It is the typed form on top of `read_words`: it
parses the words of each UMP with `Ump::parse` and gives `Received { timestamp, ump }`. Both forms share one packet
reader, so the wire rules of DEC-BLE-001 (timestamps, running status, SysEx continuation, malformed parts) apply to
both without a second copy.

## Context

OPT-044 (docs/optimization-backlog.md): the job `ble_rx` of tools/size-probe (`PacketReader::read`, MIDI 2.0
Protocol output) is 9,140 B flash, and `Ump::parse` of all Message Types is about 4,448 B of it. Every BLE-MIDI
receiver pays for it, also one that only forwards the words (to USB, to a UMP queue). DEC-018 gives word paths for
the other transports (zmidi-io `BytesToUmpCore::feed`, job `din_in_feed` 676 B).

## Consequences

A firmware that calls only `read_words` does not link `Ump::parse`. `read` costs the same as before. The adapter
zmidi-ble-trouble and examples/esp32s3-ble-midi keep `read`; they can move to `read_words` later. The C ABI and the
bindings do not change (they take words already).

## Alternatives

Replace `read` by the word form only: a breaking change of the public API for no size gain over the additive form.
A Cargo feature that turns the typed form off: more feature combinations (DEC-017) for the same result that the
linker gives when `read` is not called.

## Verification

A property test: for random packets, `read_words` followed by `Ump::parse` gives the same items and errors, in the
same order, as `read`. The existing reader tests stay unchanged. Size: a job `ble_rx_words` in tools/size-probe
with a budget in scripts/size-check.py.

## Codegen include

text, consequences, verification
