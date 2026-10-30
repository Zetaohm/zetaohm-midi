<a id="dec-ci-001"></a>

# DEC-CI-001 — Responder-only build: feature initiator; a reply for an Initiator is ignored

**Status:** accepted

**Requirements:** [REQ-CI-PEF-044](../../requirements/zmidi-ci.md#req-ci-pef-044), [REQ-CI-PEF-045](../../requirements/zmidi-ci.md#req-ci-pef-045), [REQ-CI-TIER-002](../../requirements/zmidi-ci.md#req-ci-tier-002)

A Cargo feature `initiator` (in `full`, so on by default) holds the Initiator role of `zmidi-ci`: the readers
of the replies that only an Initiator receives (Reply to Discovery, Reply to Endpoint Information, Reply to
Profile Inquiry, Reply to PE Capabilities, Reply to Process Inquiry Capabilities, Reply to MIDI Message
Report, End of MIDI Message Report; with `profiles`, also Reply to Profile Details Inquiry and the Profile
Enabled, Disabled, Added and Removed Reports: addendum of 2026-10-26), their events, and the Discovery window.

With `initiator` off, the Device ignores such a reply: no event and no NAK (answer (a) of OQ-CI-001,
2026-10-22). Discovery, Invalidate MUID, ACK and NAK are read in both roles, and the peer table
stays: a Responder needs the Receivable Maximum SysEx Message Size and the Message Version of the Initiator
from its Discovery (DEC-011 D-11), and an Initiator can send a NAK 0x20 Terminate (Table 16).

## Context

OPT-049 step 8 (the size goal of DEC-017). M2-101 does not say what a Device that never starts
Transactions does with a reply (AMB-M2-101-27): 5.11 (PDF page 35) uses a NAK as the response to any message that a
Device does not understand, and Appendix F (PDF page 74) asks only to respond to Discovery and Invalidate MUID
and to be able to send a NAK.

## Consequences

- REQ-CI-TIER-002 states the feature; the Initiator calls (discovery, endpoint_info_inquiry, profile_inquiry,
  pe_capabilities_inquiry, pi_capabilities_inquiry, message_report_inquiry) need `initiator`.
- A reply is not a request, so silence cannot start a NAK loop; midi2 C99 (a responder only) does the same by
  default.
- Expected saving about 400 to 600 B flash, no RAM. The feature matrix covers the build with it off.

## Alternatives

- (b) NAK 0x01 Not Supported, the literal reading of 5.11: an Initiator can report an error for a valid reply.
- (c) An event with the raw Sub-ID#2 and body: more code, and no known use.

## Verification

A test with the feature off: each reply kind gives no event and no reply; Discovery, Invalidate MUID, ACK and
NAK behave as with the feature on. Probe job of a responder-only build in scripts/size-check.py.

## Source

m2-101-um

## Codegen include

text, consequences, verification
