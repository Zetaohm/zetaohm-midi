# Optimization backlog

Possible optimizations to investigate later. An entry is not a decision: it records where a cost was seen, how it
was measured, and the options. When an entry changes an API or a module spec, a design decision (DEC) records the
chosen approach. A finished entry moves to "Done"; an entry that will not be done moves to "Closed (not done)".

Ids: DEC, REQ and AMB name the design decisions, the requirements and the specification ambiguities of the project.
T names a test in `docs/outstanding-tests.md`. The dates are the dates of the work.

Entry format: `### OPT-<nnn> — <short title>` with Area, Found, Measurement, Cause, Options, Constraints.

## Measurement method (RAM survey of 2026-10-17)

- Target `thumbv7em-none-eabi(hf)`, release, opt-level "s" (the settings of the FLXS1 firmware). Sizes on the host (64-bit) are
  larger and are not used for decisions.
- Type sizes: one line `const _: [(); 0] = [(); core::mem::size_of::<T>()];` for each type in a scratch crate;
  `cargo check --target thumbv7em-none-eabihf --message-format=json` gives each size in the E0308 message.
- Stack: a scratch firmware calls the main entry points with opaque inputs; a script reads
  `arm-none-eabi-objdump -d` (`push`, `vpush`, `sub sp`, `bl`/`b` targets) and gives the deepest call chain of
  each entry point. Indirect calls are not followed, so the values are lower bounds.
- Generic sizes used: `CiDevice<4,4,8>`, `PeEngine<2,256,4,2>`, `Host<1>` with the other defaults.
- Flash = text + rodata. The `.data` image of the probe statics is not included (OPT-013).

Survey result, sizes in bytes (static state):

| Type | Size | Note |
|---|---|---|
| `zmidi_net::endpoint::Host` (defaults: 4 sessions) | 10072 | was 14168 (OPT-007) |
| `zmidi_net::endpoint::Client` (defaults) | 4048 | was 5072 (OPT-007) |
| `zmidi_net::session::Session` (defaults) | 1960 | was 2984; `SendHistory<256>` is about 1040 of it (OPT-007) |
| `Host<1,1,65,1,1,16>` (smallest) | 3160 | fixed part of `Core` is about 2100 |
| `zmidi_ci_pe::ZlibMcoded7Decoder` | 10596 | plus the 32768-byte window of the caller |
| `zmidi_io::BytesToUmp` | 156 | was 4120, then 280, 152; `BytesToUmpCore` 24, `Midi1ToMidi2Group` 128 (OPT-001, OPT-011) |
| `zmidi_ump::Midi1ToMidi2` (16 Groups) | 2048 | was 4096; `Midi1ToMidi2Group` is 128 (OPT-011) |
| `zmidi_usb::MidiLink<1>` | 156 | was 412, 284; `MidiLink<16>` 652, was 4748, 2700 (OPT-011) |
| `zmidi_usb::FunctionConfig` (defaults) | 196 | 52 without Alternate Setting 0 jacks and USB MIDI 2.0 |
| `zmidi_ci_pe::PeEngine<2,256,4,2>` | 3600 | 2664 of it are two tables of `SubscriptionEntry` |
| `zmidi_ci_pe::SubscriptionEntry` | 116 | was 332 (OPT-005) |
| `zmidi_ci::CiDevice<4,4,8>` | 392 | no action |
| `zmidi_ump::Ump` | 32 | |

Deepest stack of the main entry points:

| Entry point | Stack | Largest frame |
|---|---|---|
| `zmidi_net` `Received::next` (Host) | 10896 | 7008, its own frame (OPT-003) |
| `zmidi_net` `Core::poll_entry` | 3100 | `Session::poll` 1104 |
| `zmidi_io::BytesToUmp::new` | 276 | was 4180 (OPT-001 done) |
| `zmidi_usb::MidiLink` read, write, descriptors, GTB request | 804 | 1140 when built with the host reader |
| `zmidi_usb::read_configuration` and its iterators (host) | 292 | no action |
| `zmidi_ci_pe::PeEngine::receive` | 3872 | `invalidate` 3368 (OPT-003) |
| `zmidi_ci_pe::PeEngine::new` | 3000 | its own frame (OPT-003) |
| `zmidi_ci_pe::PeEngine::write_pending` | 1724 | 1236 |
| `zmidi_io::UmpToBytes::write` | 788 | `to_midi1` 320 |
| `zmidi_ci::CiDevice::receive` | 528 | no action |
| `zmidi_ci_pe::ZlibMcoded7Decoder::new` (`zlib`) | 10612 | `ZlibInflater::new` 10596 (OPT-003) |
| `zmidi_net` `Received::next` (Host, `auth`) | 14176 | own frame and larger handshake frames |

Each crate alone, second run after OPT-001 (2026-10-17). A probe uses the main API; flash = text + rodata; each row
includes the zmidi-ump code that the crate uses, so the rows do not add up.

| Crate, feature | Flash, no LTO | Flash, LTO | Static state | Peak stack |
|---|---|---|---|---|
| zmidi-ump | 12802 | 9260 | 4232 (`Midi1ToMidi2`, `SysEx7Reassembler<128>`) | 548 |
| zmidi-ump `alloc` | adds no code | | | |
| zmidi-io | 14330 | 10070 | 282 (`BytesToUmp`, `UmpToBytes`); was 4122 | 788; was 4180 |
| zmidi-ci | 19653 | 14898 | 392 (`CiDevice<4,4,8>`) | 528 |
| zmidi-ci-pe, with zmidi-ci | 60880 | 46544 | 3992 (`PeEngine<2,256,4,2>`, `CiDevice`) | 3872 |
| zmidi-ci-pe `zlib`, with zmidi-ci | 72060 | 55800 | 47356 (adds decoder 10596, window 32768) | 10612 |
| zmidi-file | 6916 | 6532 | 0 (caller buffers) | 332 |
| zmidi-net (`Host<1>`) | 33485 | 28528 | 5072 | 10896 |
| zmidi-net `auth` (`Host<1>`) | 39669 | 32384 | 5320 | 14176 |
| zmidi-usb, device (`MidiLink<1>`, default `FunctionConfig`) | 32022 | 21512 | 608 | 804 |
| zmidi-usb, host (`read_configuration`) | 11074 | 6970 | 0 (caller slice) | 292 |
| zmidi-usb, device and host | 39642 | 27530 | 608 | 1140 |
| all, no `zlib`, no `auth`, no zmidi-usb | 73112 | 58680 | 8506 | 10896 (zmidi-net; the script misses it in this build) |
| all, no `zlib`, no `auth`, with zmidi-usb | 96704 | 77436 | 9120 | 10896 (zmidi-net; the script misses it in this build) |

## Measurement method (speed, 2026-10-23)

- `tools/speed-probe` is a firmware for the QEMU machine `mps2-an386` (Cortex-M4F), with the size-probe profile
  (`thumbv7em-none-eabihf`, opt-level "s", fat LTO, one codegen unit, panic abort). Each bench prints its name
  through semihosting and runs its work between two calls of `speed_mark`. The plugin `qemu/insn_mark.c` counts
  the executed instructions (one callback for each translation block) and prints the count at each mark.
- The number is executed Thumb-2 instructions, not cycles: QEMU has no pipeline and no flash wait states. On a
  Cortex-M4 one instruction is about 1 to 2 cycles. On the FLXS1 (MK20DX256, 72 MHz, 24 MHz flash) the DWT counts
  gave 1.35 to 2.55 cycles for each instruction, median about 1.9 (T-017, 2026-10-23).
- The counts are the same on each run and on each host (QEMU 10.1 on macOS arm64 and QEMU 8.2 on Ubuntu x86_64
  gave the same numbers on 2026-10-23). They change with rustc and with the library code only.
- Each bench checks its output (the number of words, bytes or events), so that a bench that does no work fails.

## Open

### OPT-002 — `Ump` values in queues and small buffers

- Area: `crates/zmidi-io/src/byte_stream.rs` (`Midi1Scheduler`, `Pushed`), `crates/zmidi-ump/src/translate.rs`
  (`Translated`).
- Found: 2026-10-14, in a size review for this backlog (not from a reported problem).
- Measurement (host, 64-bit): `size_of::<Ump>()` = 48 bytes; `Pushed` = 112 (2 × `Option<Ump>`);
  `Midi1Scheduler<8>` = 528 (66 bytes per entry: `Option<(u64, Ump)>`); `Translated` = 208 (4 × `Ump`).
  On `thumbv7em-none-eabihf` (2026-10-17): `Ump` = 32, `Pushed` = 72, `Midi1Scheduler<8>` = 328, `Translated` = 136.
- Cause: the `Ump` enum is as large as its largest variant plus the tag, although most queued messages are 1 or 2
  words. A queue entry could be `[u32; 4]` (16 bytes) plus the render time, or the word count plus `[u32; 4]`.
- Options:
  1. Store raw words (`[u32; 4]`, or `[u32; 2]` where the Message Types allow) in `Midi1Scheduler` and `Pushed`,
     and parse when the value is used.
  2. Make the `Ump` variants smaller (store word arrays and decode on access).
- Constraints: DEC-002; the `Ump` API is public in zmidi-ump, so option 2 changes an API (a decision).

- Status (2026-10-18): option 1 done for the stored state: `Midi1Scheduler` keeps `(u64, [u32; 4])` (the words of
  the parsed UMP; the Message Type gives their number, M2-104 Table 4): `Midi1Scheduler<8>` 328 -> 200 bytes, with
  the test `prop_scheduler_writes_what_the_writer_writes`. `Pushed` and `Translated` are return values on the
  stack, not stored state, so they stay. Option 2 is not done (public `Ump` API).

Other sizes measured at the same time, no action proposed: `SendOrder` 96, `JrReceiver<8>` 136,
`SysEx8Reassembler<16>` 32 bytes.

### OPT-004 — `PeEngine` keeps a second table of ended subscriptions

- Area: `crates/zmidi-ci-pe/src/transaction.rs` (`PeEngine::ended`, `take_ended`, `invalidate`).
- Found: 2026-10-17, RAM survey.
- Measurement: `ended: [Option<SubscriptionEntry>; S]` = 1332 bytes for S = 4, the same size as the
  `SubscriptionTable<4>` itself. `PeEngine<2,256,4,2>` = 3600, of which 2664 are the two tables.
- Cause: an ended subscription is copied out of the table into a second full-size table until the caller takes it.
- Options:
  1. Mark the entry as ended in the `SubscriptionTable` (a state flag) and free it in `take_ended`; no copy.
  2. Keep only a small record in `ended` (peer MUID and "subscribeId", about 24 bytes) if the caller needs no more.
  3. Give the ended entries as `PeEvent`s at once, with no storage.
- Constraints: REQ-PE-SUB-* behaviour must not change; `take_ended` is public API (a decision for option 2 or 3).
- Status (2026-10-18): option 1 done. `SubscriptionTable` marks an ended entry in its place (`end_peer`, `end_all`)
  and `take_ended` frees it; `PeEngine` has no `ended` table. `PeEngine<2,256,4,2>` 3600 -> 2280 bytes, and the
  `PeEngine::new` stack 3048 -> 1720 (the copy of the second table was most of the OPT-003 rest). One edge case
  changed: an ended entry keeps its place until the caller takes it, so a new subscription can get "full"
  (REQ-PE-SUB-005) until then. No ended entry is lost (REQ-PE-SUB-019); before, entries were lost when the second
  table was full.

### OPT-005 — `SubscriptionEntry` text buffers of 144 bytes

- Area: `crates/zmidi-ci-pe/src/subscription.rs` (`NAME_MAX = 36 * 4`, `resource`, `res_id`).
- Found: 2026-10-17, RAM survey.
- Measurement: `SubscriptionEntry` = 332 bytes; `resource` and `res_id` are 145 bytes each.
- Cause: the buffers hold 36 characters of 4 UTF-8 bytes. "resource" is ASCII alphanumeric with an optional "X-"
  prefix, and "resId" is ASCII in Tables 68 and 72 (AMB-M2-103-03, OPEN).
- Options:
  1. 36-byte buffers, and refuse a "start" with a non-ASCII "resource" or "resId" (status 400): 332 to about 116.
  2. Store "resId" as given but only up to 36 bytes; refuse a longer one.
- Constraints: needs a resolution of AMB-M2-103-03 first; with OPT-004 option 1, the saving is 216 bytes per entry.
- Status (2026-10-18): done in a narrow form. A stored "resource" is always ASCII and at most 36 bytes (the
  responder checks it against its own Resource table first; the initiator stores its own "start"), so only a
  received "resId" of more than 36 bytes changes: 400 (REQ-PE-SUB-007), and such a value breaks Tables 13, 68 and
  72 anyway. `NAME_MAX` 144 -> 36: `SubscriptionEntry` 332 -> 116 bytes, `PeEngine<2,256,4,2>` 2280 -> 1416.

### OPT-006 — `PeEngine` pending queue has a fixed capacity of 16

- Area: `crates/zmidi-ci-pe/src/transaction.rs` (`PENDING = 16`, `pending: [Option<Pending>; PENDING]`).
- Found: 2026-10-17, RAM survey.
- Measurement: not measured alone (private type); about 16 × 20 bytes from the fields.
- Cause: the capacity does not follow the transaction count `T`.
- Options: a const generic, or a capacity from `T` (for example `2 * T + 2`).
- Constraints: a full queue must still give the overflow behaviour that the requirements state.
- Status (2026-10-18): done as a const generic `Q` of `PeEngine` (default `DEFAULT_QUEUE` = 16, compile-time check
  `Q >= 3`). Stable Rust does not allow a length computed from `T`. 16 bytes per entry: `PeEngine<2,256,4,2,4>`
  3408 bytes, 3600 with the default. A defect found on the way, fixed: `refresh`, `wait`, `abort`, `reply_status`
  and `corrupt` queued with no check, so a full queue lost the message and the call gave `Ok`. They now give
  `QueueFull` before any change (tests `caller_requests_on_a_full_queue_give_queue_full` and
  `corrupt_on_a_full_queue_uses_the_place_of_the_removed_ack`).

### OPT-007 — `SendHistory` default of 512 words for each Network MIDI session

- Area: `crates/zmidi-net/src/stream.rs` (`SendHistory<const C: usize = 512>`), defaults of `Session`, `Host`,
  `Client`.
- Found: 2026-10-17, RAM survey.
- Measurement: `SendHistory<512>` = 2068 bytes, 69% of `Session` (2984). The default `Host` (4 sessions) = 14168.
  `Session<1,65,1,1,16,1>` = 1048.
- Cause: the retransmit history holds 512 UMP words for each session. C must be 65 or more.
- Options:
  1. Smaller defaults (for example C = 128: about 530 bytes), with the large values as a documented choice.
  2. One history shared by all sessions of a Host (the same UMP data often goes to all peers).
- Constraints: REQ-NET-RTX-003 and the retransmit requirements; option 2 changes the module design.
- Status (2026-10-18): option 1 done with C = 256 (maintainer choice; N-40 changed in DEC-NET-001, REQ-NET-RTX-003).
  `Session` 2984 -> 1960, `Host<1>` 5072 -> 4048, default `Host` 14168 -> 10072 bytes. M2-124 7.2.3 gives no
  size; a Command that is no longer held gets a Retransmit Error. Option 2 is open.

### OPT-008 — `SocketAddr` (32 bytes) in the fixed queues of `zmidi-net`

- Area: `crates/zmidi-net/src/endpoint.rs` (`OutQueue`, `Backlog`, `StatelessPings`, `Entry`).
- Found: 2026-10-17, RAM survey.
- Measurement: `SocketAddr` = 32 bytes. The fixed part of `Core` is about 2100 bytes (smallest `Host` 3160 minus
  one session 1048): `OutQueue` 16 × (32 + `Out`), `Backlog` 16 × (32 + 24), `StatelessPings` 4 × 48.
- Cause: `core::net::SocketAddr` keeps room for IPv6 with flow info and scope id; the capacities 16, 16 and 4 are
  constants, not generics.
- Options:
  1. A compact address type inside the crate (IPv4 + port is 6 bytes; IPv6 + port + scope is 22) or a table of
     peer addresses that the queues refer to by index (1 byte).
  2. Const generics for the queue capacities, with the present values as defaults.
- Constraints: DEC-012 (sans-IO), public API takes `SocketAddr`; only the storage changes.
- Status (2026-10-18): measured on `thumbv7em`: `OutQueue` 648 bytes (entry 40: `SocketAddr` 32 + `Out` 8),
  `Backlog` 912 (entry 56), `StatelessPings` 200: 1760 of the 4048 bytes of `Host<1>`. Option 1 is not done: an
  exact IPv6 address (address, port, flow info, scope id) needs 28 bytes, so it saves 4 bytes for each entry; an
  index table does not fit `OutQueue`, whose replies go mostly to addresses with no session. Option 2 done: one
  const generic `O` of `Host` and `Client` for both queues (default `STATELESS_CAPACITY` = 16; `O >= 1` checked at
  compile time; `BACKLOG_CAPACITY` removed). `Host<1, 64, 256, 4, 8, 128, 4>` = 2896 bytes (4048 with the default).

### OPT-009 — Copies of the configuration in each Network MIDI session

- Area: `crates/zmidi-net/src/session.rs` (`Session::config`), `crates/zmidi-net/src/handshake.rs`
  (`Handshake::config`).
- Found: 2026-10-17, RAM survey.
- Measurement: `SessionConfig` = 104 and `HandshakeConfig` = 40 bytes in each session, and `Core` has its own copy.
- Cause: each session keeps its own copy, although all sessions of a Host use the same values.
- Options: give `&SessionConfig` to the session calls from `Core`; keep only the per-session values.
- Constraints: the `Session` API is public; a change of its methods needs a module spec change.
- Measured again (2026-10-18): each session keeps 184 bytes of configuration copies: `SessionConfig` 104, the
  `Handshake` copy 40, and one `StreamConfig` (20) each in the `Sender` and the `Receiver`.
- Status (2026-10-18), step 1 done: `Handshake`, `Sender`, `Receiver` and `DataStream` keep no configuration; the
  methods that read it take `&HandshakeConfig` or `&StreamConfig`, and the `Session` gives the parts of its own.
  Inside `handshake.rs` the steps that read it run on a private view (`Hs`, the handshake and the configuration,
  `Deref` to the handshake). `Handshake::config` is gone. `Session` 1928 -> 1840, `Host<1>` 4008 -> 3920, default
  `Host` 9912 -> 9560 bytes.
- Status (2026-10-18), step 2 done: the Endpoint keeps a crate-private `SessionCore` (the state with no
  configuration) for each entry and one `SessionConfig` for all; the methods of the core that read the
  configuration take `cfg` (passed through, no view: a `Deref` view would borrow the whole core). The public
  `Session` (configuration and core) keeps its API; `Host::session` and `sessions` give a new read-only
  `SessionRef`. `Host<1>` 3920 -> 3816, default `Host` 9560 -> 9144 bytes (184 + 8 bytes less for each session over
  both steps). The receive stack grew since step 1: OPT-014.

### OPT-010 — zlib inflate state

- Area: `crates/zmidi-ci-pe/src/zlib.rs` (`ZlibInflater`, `ZlibMcoded7Decoder`), dependency `miniz_oxide`.
- Found: 2026-10-17, RAM survey.
- Measurement: `ZlibInflater` = 10516 bytes, `ZlibMcoded7Decoder` = 10596, plus the caller window (up to 32768 for
  a stream with the largest window).
- Cause: the `miniz_oxide` decompressor keeps full Huffman fast-lookup tables. The window size is set by the sender.
- Options:
  1. A smaller inflate core (a "tinf"-style decoder keeps about 1-2 KB of tables, and is slower).
  2. Keep `miniz_oxide`, and document the RAM need of the `zlib` feature (it is off by default, DEC-PE-001).
- Constraints: DEC-PE-001 chose `miniz_oxide`; option 1 needs a new decision and the zlib tests as the check.
- Status (2026-10-18): option 2 done: the `zlib` module documents the RAM need (decoder 10596 bytes, window up to
  32768, about 21 KB of stack when a function makes and uses a decoder on its stack, because `new` moves it).
  `scripts/stack-check.py --features zlib` measures `probe_pe_zlib_decode`. Option 1 is open.
  `ZlibInflater::reset` and `ZlibMcoded7Decoder::reset` (2026-10-18) reuse a decoder in place: reset and decode
  584 bytes of stack. An `init` into a caller slot was measured and not kept: 21220 bytes, because
  `DecompressorOxide::new` builds its state by value.
- Status (2026-10-18): option 1 done (DEC-PE-004, which supersedes DEC-PE-001): an own streaming inflater in the
  private module `inflate` (puff-style canonical Huffman decoding, no alloc, no `unsafe`, no dependency).
  `ZlibInflater` 10516 -> 1072, `ZlibMcoded7Decoder` 10596 -> 1152 bytes; a decoder made and used on the stack
  2768 bytes of stack (was 21584); the probe code about 4.3 KB smaller. Differential tests against miniz_oxide
  (dev-dependency): every level 0 to 10, and changed streams (same output or both an error); run once with 2560
  and 40000 cases with no disagreement.

### OPT-011 — `BytesToUmp` keeps 256 bytes of translator state for MIDI 1.0 Protocol output

- Area: `crates/zmidi-io/src/byte_stream.rs` (`BytesToUmp`), `crates/zmidi-ump/src/translate.rs` (`ChannelState`),
  `crates/zmidi-usb/src/event_packet.rs` (`PacketReader`), `crates/zmidi-usb/src/link.rs` (`MidiLink`).
- Found: 2026-10-17, second RAM survey (open options 2 and 3 of OPT-001).
- Measurement (`thumbv7em-none-eabihf`): `BytesToUmp` = 280, of which `Option<Midi1ToMidi2Group>` = 256.
  `MidiLink<16>` = 4748, of which 4096 are the 16 translators. `LinkConfig::default()` selects
  `StreamProtocol::Midi1`, so the USB default never uses them.
- Cause: `Option<T>` reserves the full size of `T` also when it is `None`. `ChannelState` is 16 bytes, but its
  held values are 7 bits each.
- Options:
  1. Pack `ChannelState` in 8 bytes (0x80 = no value): `Midi1ToMidi2Group` 256 -> 128, `Midi1ToMidi2` 4096 -> 2048.
     No API change.
  2. A caller-owned translator that `push` borrows (`push_with(&mut Midi1ToMidi2Group, byte)`), or a const generic
     protocol parameter: MIDI 1.0 output pays nothing. `BytesToUmp` 280 -> about 24, `MidiLink<16>` 4748 -> about 650.
  3. `MidiLink`: hold the Event Packet state and the UMP state as one enum (only one mode is active at a time):
     about 50 to 60 bytes less for `MidiLink<1>`. Small; do it with option 2 only if the code stays simple.
- Constraints: D.3.3/D.3.4 results must not change (the OPT-001 differential test is the check). Option 2 changes
  the public API of zmidi-io and zmidi-usb (a module spec change and a decision).
- Status (2026-10-18): option 1 done. `ChannelState` holds each value in one byte (`Held`, 0x80 = no value) and the
  parameter and pending bits in one flags byte: 16 -> 8 bytes. `Midi1ToMidi2Group` 128, `Midi1ToMidi2` 2048,
  `BytesToUmp` 152, `PacketReader<1>` 156, `MidiLink<1>` 284, `MidiLink<16>` 2700. The differential test against
  the earlier translator (4000 cases each; 189 RPN, 173 NRPN, 245 Program Change with bank outputs) matches. Option 3
  is open.
- Status (2026-10-18): option 2 done as a mix of both forms. zmidi-io: `BytesToUmpCore` (24 bytes) with
  `push_with(byte, Option<&mut Midi1ToMidi2Group>)`; `BytesToUmp` (156) is the core plus its own translator, with the
  same API. zmidi-usb: `PacketReader<CABLES, X>` and `MidiLink<CABLES, X>` keep `X` translators (default 0); MIDI 2.0
  Protocol output needs `X >= CABLES`, else `new` gives `EventPacketError::NoTranslators` (REQ-USB-EVT-023). The
  runtime Protocol choice (U-20) stays. The adapters get the same `X`. `MidiLink<1>` 284 -> 156, `MidiLink<16>`
  2700 -> 652, `PacketReader<1>` 156 -> 28; `MidiLink<16, 16>` 2700 as before. A type parameter for the Protocol
  was not taken: it would end the runtime choice.

### OPT-012 — Each Network MIDI session keeps its handshake state after it opens

- Area: `crates/zmidi-net/src/session.rs` (`Session::hs`), `crates/zmidi-net/src/handshake.rs` (`Handshake`).
- Found: 2026-10-17, second RAM survey.
- Measurement: `Handshake<128>` = 248 bytes (472 with `auth`) in each session; `PeerInfo` is 143 of it. In the
  open state the session still calls `peer()`, `touch`, `on_command`, `cancel`, `poll` and the reply writer. Not
  examined yet: which of the deadlines, `repeating` and the `auth` data (224 bytes) these calls need after the open.
- Cause: the `Handshake` value is a field of `Session` for the whole session life.
- Options: keep `PeerInfo` in `Session` and hold the other handshake state in an enum of the session state (only
  in the handshake states), or clear it to a small value after the open. About 100 bytes per session, 330 with
  `auth`: about 400 to 1300 bytes for a 4-session Host.
- Constraints: check that Session Reset and the reply writer need no handshake field after the open
  (REQ-NET-RST-*). The `Session::handshake()` accessor is public.
- Check (2026-10-18), not done: in Established the handshake still runs (a Host replies Accepted to an
  Invitation, REQ-NET-INV-018, `handshake.rs:1110`; each Client Command calls `touch`, REQ-NET-SES-002; `cancel`
  gives the Bye), so `role`, `config`, `caps` and `peer` stay. The other fields (deadlines, `repeating`,
  `pending_seen`, `auth`: about 100 bytes, 330 with `auth`) are not used after the open, but an enum of the
  session state saves nothing: the `Session` keeps room for the largest variant, because each slot can be in a
  handshake state. Only one storage shared by these fields and the `DataStream` (used only after the open) saves
  memory: about 400 bytes for a default Host, 1.3 KB with `auth`. It changes the Session state machine (14 call
  sites); do it only with a change of the `Session` module spec and full state machine tests.
- Status (2026-10-18): done in a compact form instead. A crate-private `Deadline` (8 bytes, `u64::MAX` = none) replaces
  `Option<u64>` (16 bytes) for the three handshake deadlines, `Session::last_reset` and the Endpoint `polled`:
  `Session` 1960 -> 1928, `Host<1>` 4048 -> 4008, default `Host` 10072 -> 9912 bytes. The behavior is the same (a
  deadline at `u64::MAX` never comes). The storage shared with the `DataStream` was measured again: about 72 bytes
  for each session for a change of the public `Handshake` API; not done.

### OPT-013 — Large statics with a non-zero empty value use flash for a `.data` image

- Area: all large state types (`Host`, `PeEngine`, `CiDevice`, `BytesToUmp`, `MidiLink`, `Midi1ToMidi2`).
- Found: 2026-10-17, second RAM survey (`arm-none-eabi-nm` of the probe).
- Measurement: in the probe, `static mut X: Option<T> = None` goes to `.data`, not `.bss`: `Option<Midi1ToMidi2>`
  4096, `Option<PeEngine<..>>` 3600, `Option<Host<1>>` 5072, `Option<MidiLink<1>>` 412. Each byte of `.data` also
  uses one byte of flash for the initial image. The flash values of the survey tables are text + rodata and do not
  include this image.
- Cause: the `None` of these `Option` types uses a niche, so its bit pattern is not all zeros.
- Options: give a `const fn new()` (or `EMPTY` constant) whose value is all zero bits, so a `static` goes to
  `.bss`; document the use of `static_cell`/`MaybeUninit` (also OPT-003 option 3). A firmware that keeps the
  values in RTIC resources or in `MaybeUninit` statics does not have this cost.
- Constraints: an all-zero empty value is a layout rule; add a test that checks it (for example a `const` that
  compares the bytes), so a later field change does not break it.
- Status (2026-10-18), checked in the symbol table of a probe (`llvm-objdump -t`): `.data`:
  `Option<Host<1>>` 4048, `Option<PeEngine<..>>` 2280, `Option<CiDevice<..>>` 392, `Option<MidiLink<1>>` 284;
  `.bss`: `Option<Midi1ToMidi2>`, `MaybeUninit<Host<1>>`. The compiler sets the bit pattern of `None`, so the
  fix for these is documentation: the docs of `Host::init`, `Client::init`, `PeEngine`, `MidiLink` and `CiDevice`
  name the `MaybeUninit` / `static_cell` pattern. Found on the way: OPT-011 made `Midi1ToMidi2::new()` non-zero
  (0x80 = no value), 2048 bytes in `.data`; `Held` now stores the value + 1 (0 = no value), so it is in `.bss`
  again, with the test `empty_translator_is_all_zero_bits`.

### OPT-015 — Word paths: `Ump` round trips in `zmidi-io`, `zmidi-ump` translate and the USB link

- Area: `BytesToUmp` (`Pushed`), `UmpToBytes::write`, `to_midi1`, `zmidi-usb` Event Packet reader and writer
  (`Received::EventPacket`), `zmidi-ci-pe` `JsonWriter::number_u64` (DEC-017, OPT-052 step 3).
- Found: 2026-10-20, prototypes measured with `tools/size-probe` (`prototypes-2026-10-20.patch`, changes 2 to 5, 8).
- Measurement (flash, probe): `BytesToUmp` 5,428 -> 2,312; `UmpToBytes` 6,516 -> 2,328; `to_midi1` 7,268 -> 836;
  USB class 31,116 -> 27,488 (Alt 0 as words); PE 46,960 -> 45,968 (`u32` digits). Property tests: the word paths
  give the same output as the code now.
- Cause: words are parsed into a typed `Ump` (all of `Ump::parse` is linked) and written back; `Pushed` and
  `Translated` copy 32 B `Ump` values; `number_u64` inlines 64-bit division loops.
- Options: return words (`PushedWords`), write bytes from the words, `to_midi1_words`, `Received::EventPacket` with
  words, a `u32` number writer. The typed API stays on top of the words.
- Constraints: `Pushed` and `Received::EventPacket` are public: decide the API first.
- Status (2026-10-20): done with DEC-018; numbers in OPT-052, step 3.

### OPT-016 — USB descriptors and topology built at run time

- Area: `zmidi-usb` `FunctionConfig`, `zmidi-usb-device` (`get_configuration_descriptors`, `select`, `new`), also
  `zmidi-usb-embassy` (DEC-017 step 6).
- Found: 2026-10-20, prototype change 1 (a const descriptor walker).
- Measurement: USB class alone (`usb2` minus `usb_core`) 23,852 B; the const descriptor path saves 7,660 B; with
  OPT-015 the class is about 12,570 B against `tusb_ump` 3,538 B. `select` (1,626 B) and the class part of `init`
  (about 2 KB) still read `FunctionConfig`.
- Cause: descriptors are built and checked (`checked_len`, `validate_endpoints`) when the host asks.
- Options: a const configuration type that builds and checks the descriptors in `const fn`, beside or instead of
  `FunctionConfig`; endpoint and Group mask per Alternate Setting as constants.
- Constraints: public API of both adapters.
- Status (2026-10-20): done for zmidi-usb and the usb-device adapter (DEC-USB-002, `zmidi_usb::fixed`,
  `MidiClass::fixed`): USB class 20,320 -> 9,836 B. Done 2026-10-20 for the embassy-usb adapter too (the
  trait `Function` is now in `zmidi_usb::fixed`): embassy class 15,264 -> 4,780 B (stub driver probe).

### OPT-019 — One-chunk Property Exchange Responder tier

- Area: `zmidi-ci-pe` (a new small Responder beside `PeEngine`), DEC-017 step 6.
- Found: 2026-10-20, prototype `tools/size-probe/prototype-pe-one-chunk-2026-10-20.patch` (`zmidi_ci_pe::simple`,
  probe jobs `pe_simple`, `pe_simple_full`).
- Measurement (flash of the PE part, the job minus the CI device of 9,940 B): full engine 30,208 B; full
  engine with `S = 0` 26,096 B; one-chunk Responder with the full `Header::parse` 11,224 B; one-chunk
  Responder with a key search for "resource" and "resId" 3,656 B (midi2 C99: under 1.5 KB, plus our
  `zmidi-ci` PE framing). RAM: 256 B body buffer against 2,440 B.
- Cause: the engine keeps transactions, chunk reassembly, flow control, time-outs and subscriptions for
  every build; `Header::parse` alone pulls in about 7.5 KB (the JSON tokenizer, typed `HeaderStr`, the
  checks of every property).
- Options: a `simple` Responder (Get and Set in one Chunk, 413 for a request in more Chunks, ASCII
  only, static ResourceList), with a small header reader that still follows JSON for top-level string
  properties (whitespace, escapes); or the full `Header::parse` (11.2 KB).
- Iteration (2026-10-20): with a correct JSON reader (the slice walker of OPT-020, names as bytes, escapes
  decoded) in place of the key search, the one-chunk PE part is 5,932 B (key search 3,656 B; midi2 C99,
  which uses a key search, under 1.5 KB). Measured in the patch (job `pe_simple`).
- Status (2026-10-20): done (DEC-PE-005): `zmidi_ci_pe::one_chunk::respond`, 6,496 B
  and 256 B of RAM (job `pe_one_chunk`). The prototype missed one thing that the tests found: each Get or
  Set Chunk opens a request in the zmidi-ci device, so the Responder must end it (`pe_end`), or the request
  table fills after `REQUESTS` requests.
- Constraints: a decision first: the reply for a request in more than one Chunk (413 Payload Too Large),
  for "mutualEncoding" other than ASCII (415), and the header reader rules (M2-103 5, 6, 7.4.1). The key
  search of the prototype does not handle escapes or nested objects.

### OPT-020 — `Header::parse` walks the header with the streaming JSON reader

- Area: `zmidi-ci-pe` `header.rs` (`Header::parse`) and `json.rs` (`for_each_member`).
- Found: 2026-10-20, iteration on OPT-019 (patch `tools/size-probe/prototype-pe-one-chunk-2026-10-20.patch`).
- Measurement: a slice walker (`for_each_member_slice`, the whole header is in one buffer) in place of the
  streaming `JsonReader` saves 872 B in the full engine (29,336 B against 30,208 B) and 880 B in a
  one-chunk build with `Header::parse`. Most of the 6.9 KB of `Header::parse` is the typed header (`set`
  1,468 B, escape counting 660 B, `EncodingName::parse` 280 B, number parse about 200 B): spec rules.
- Two defects of the code now, found by the comparison test (30,000 mutated headers per header kind):
  1. A name with an invalid UTF-8 byte is cut at that byte: `"resource\xFF"` is read as "resource" and
     `"res\xFFource"` as "res". The slice walker gives an unknown name.
  2. A header that ends inside a string value gives a property error before the JSON error, so
     `{"mutualEncoding":"` gets 415 instead of 400. The slice walker gives the JSON error (400).
- Options: replace `for_each_member` in `Header::parse` by the slice walker, with the comparison property
  test (allowed difference: the JSON error comes first) and tests for the two defects.
- Constraints: none on the API; the wire change is only for malformed headers (400 instead of 415).
- Status (2026-10-20): done: `json::for_each_member_slice`, `Header::parse` uses it,
  REQ-PE-HDR-014. PE 30,208 -> 29,528 B. `link.rs` keeps the streaming walker.

### OPT-021 — DIN reader and writer copy their results by value

- Area: `zmidi-io` `BytesToUmpCore::push_with` (`Pushed`) and `UmpToBytes::write` (the `Bytes` staging buffer).
- Found: 2026-10-21, a question why DIN is larger than midi2 C99 and AM_MIDI2.0Lib; prototype
  `tools/size-probe/prototype-din-callback-2026-10-21.patch` (jobs `bytes_feed`, `bytes_out_each`).
- Measurement (probe): DIN in 2,324 B (midi2 C99 852 B); DIN out 2,348 B (AM 1,160 B). Of each, about 1,016 B
  are `memcpy`, `memcpy4` and `memclr4`, which the C probes do not link: they come from moving results by
  value (`Pushed` is 44 B with an `Option<Packet>` array, each `Packet` is zeroed, the 12 B `Bytes` buffer is
  copied into `out`). Callback forms with no result value: `feed(byte, |words| ..)` 672 B (0.8x of midi2
  C99, no builtins) and `write_each(words, |byte| ..)` 1,124 B (1.0x of AM; 174 B `memclr4` from the
  `Midi1Words` result). Property tests: the same packets as `push` (20,000 byte streams) and the same bytes
  and running-status state as `write` (30,000 packets).
- Options: add the callback forms as the core API and build `push` and `write` on them (one implementation);
  or only document them for small builds. Additive: no break.
- Status (2026-10-21): done: `BytesToUmpCore::feed` and `UmpToBytes::write_each` are
  the core; `push_with` and `write` are built on them (`#[inline(never)]`, else LTO copies the whole reader
  into each caller: +1,312 B in the full build). REQ-BS-017. Probe: `feed` 676 B, `write_each` 1,124 B;
  `din_in` 2,324 -> 2,136 B, `din_out` 2,348 -> 2,148 B, `usb_class` -204 B, `full` -164 B. Open: the
  zmidi-usb Event Packet reader and writer still use the value forms (its `PacketRead` keeps a `Pushed`).
- Status (2026-10-21, second step): the value forms reworked on the callback forms:
  `Pushed` keeps only the words (20 B, no `memcpy`), `feed_with` carries the translator, `write` is one pass
  into `out` (no staging buffer; on `BufferTooSmall` the state does not change and the content of `out` is
  not defined). Probe: `push` 2,136 -> 1,044 B (midi2 C99 852 B), `write` 2,148 -> 1,396 B (AM 1,160 B),
  `usb_class` -96 B, `full` 71,496 -> 71,336 B.
- Status (2026-10-21, third step): the zmidi-usb Event Packet reader and writer. `PacketReader::read_with` and
  `MidiLink::read_with` read with no iterator (`PacketRead::next`, 434 B, is not linked when only the callback
  forms are used); each byte still goes through `push_with`, so the DIN reader and the USB reader share one copy.
  `PacketWriter::write` cuts the bytes of `UmpToBytes::write_each` into packets one byte at a time (no 16-byte
  buffer); `UmpToBytes::write` and the USB writer both pass a `&mut dyn FnMut(u8)`, so they share one copy of
  `write_each`. Measured variants (fixed USB class / full build, before: 9,548 / 71,336 B): callback forms with
  their own closures 9,264 / 72,360 (a second copy of the reader and the writer in the full build); value forms
  9,524 / 71,140; the chosen mix 9,420 / 71,104. `usb_class` 20,020 -> 19,892 B. A `dyn` sink in `push_with`
  makes `din_in` 1,044 -> 1,988 B, so `push_with` keeps its generic closure. The `ump_usb` reader still has
  only the iterator inside `read_with`. Property tests: `read_with` against `read` (Event Packets, both
  Protocols, several transfers; the link in both modes).
- Constraints: `write` keeps its rule that nothing is written when `out` is too small; `write_each` has no
  buffer, so the rule does not apply to it. `UmpToBytes::new` becomes `const` (so a static can hold it).

### OPT-029 — Speed tests: what exists and what to add

- Area: all crates; `tools/speed-probe`, `scripts/speed-check.py`.
- Found: 2026-10-23. Only memory is measured (size and stack, both with budgets in CI). No
  benchmark, cycle count, latency or throughput test existed.
- Plan:
  1. Instruction counts of the hot paths in QEMU with budgets in CI (same pattern as the stack budgets): `Ump`
     parse, `BytesToUmp`, `UmpToBytes`, MIDI 1.0 and 2.0 translation, SysEx7 reassembly, USB Event Packets, BLE
     reader and writer, `zmidi-file`, `PeEngine::receive`, `Header::parse`, Mcoded7 and zlib, Network MIDI
     `Host::receive` with and without `auth`.
  2. DWT `CYCCNT` on the FLXS1 for the same benches (the factor from instructions to cycles).
  3. Scale tests: the cost for each call with 1 and with N transactions or sessions (finds a hidden O(n²)).
  4. USB and BLE throughput and latency on hardware; the cost of one call through the C ABI from Python, TS and Dart.
- Constraints: DEC-002 (the probe is outside the workspace, as size-probe and stack-probe). The budgets change with
  rustc (CI uses stable), so each budget has a margin of about 10 %, as in `scripts/stack-check.py`. A change in one
  part of the probe can move another bench by up to about 5 % through LTO (seen on 2026-10-23: Mcoded7 encode
  23,210 -> 24,376 with no change in its code).
- Status (2026-10-23):
  - Step 1 done: `tools/speed-probe` (26 benches: zmidi-ump, zmidi-io, zmidi-usb, zmidi-ble, zmidi-file, zmidi-net,
    zmidi-ci, zmidi-ci-pe), `scripts/speed-check.py` (budgets; `--profile BENCH` gives the hot functions;
    `--markdown docs/speeds.md`), CI step "speed check". A CI rehearsal in Docker (Ubuntu 24.04 amd64, QEMU 8.2,
    rustup 1.98.1) gave the same counts as macOS. The fixes that the benches found: OPT-030. Table: `docs/speeds.md`.
  - Step 3 started: `net_host3_receive_note_last_session` (3 sessions: +1.8 % for each message). Not done: PE with
    many transactions, `Ump` dispatch over all Message Types.
  - Step 2 done (T-017): `examples/mk20dx-speed-bench` (the same bench modules, DWT CYCCNT at 72 MHz, defmt) and
    `scripts/speed-check.py --hw LOG`. Cycles for each instruction: 1.35 (Mcoded7) to 2.55 (UMP parse), median
    about 1.9. The order of the benches is the same as in QEMU, so the QEMU budgets stay the CI gate.
  - Step 4, bindings part done for Python and TS (wall clock, not a gate; Apple M-series host, 2026-10-23):
    `bindings/python/scripts/bench.py`: a call about 0.2 us; `BytesToUmp.feed` of 64 notes 98 ns for each message.
    `bindings/ts/scripts/bench.mjs` (node): a call about 0.7 us; `feed` 1,700 ns for each message (OPT-035).
    `bindings/dart/tool/bench.dart`: a call about 0.25 us; `feed` 621 ns for each message (OPT-035). Not done: USB
    and BLE throughput and latency on hardware (T-018).
  - Not done: a bench with `zmidi-net/auth` (SHA-256 handshake) and with `zmidi-ci-pe/zlib`.
  - Other libraries (2026-10-23): `tools/lib-compare/speed` runs the same inputs through midi2 (C99),
    AM_MIDI2.0Lib, ni-midi2 and cmidi2 with the same plugin (`python3 tools/lib-compare/speed/run.py`). Findings:
    OPT-036, OPT-037.

### OPT-031 — Network MIDI `receive` copies its 580-byte event queue for each UDP payload

- Area: `crates/zmidi-net/src/endpoint.rs` (`Received::new`, `Sink`).
- Found: 2026-10-23, `net_host_receive_note` profile: 26 % of the instructions are `memcpy`.
- Measurement: `Received::new` builds `Sink` (`[Option<Event>; 10]`, 640 bytes) in a temporary and copies 582 bytes
  (`0x246`) with the byte-wise `__aeabi_memcpy` (odd offset): about 3,000 instructions for each received payload, the
  same as 64 one-word UMPs. A payload with a few UMPs is mostly this copy.
- Tried and not kept (2026-10-23): build `Received` first and fill the queue in place; `#[inline(always)]` on
  `Received::new` and `receive`: -290 instructions only, and LTO moved another bench by +1,158.
- Options: 1. `MaybeUninit` slots in `Sink` (unsafe; only the slots below `write` are read). 2. Fewer slots, or a
  smaller `Event` (the data delivery is the largest variant). 3. The queue in `Core` with events that do not borrow
  the payload (API change). Options 1 and 3 need a decision (unsafe in a core crate; public API).
- Status (2026-10-26): option 1 closed (no `unsafe` in the library). Options 2 and 3 stay open.

### OPT-032 — `to_midi1` gives `[Ump; 4]` by value

- Area: `crates/zmidi-ump/src/translate.rs` (`Translated`).
- Found: 2026-10-23: `ump_translate_midi2_to_midi1_note` 403 instructions for each note, 64 % of them `memcpy8`
  and `memclr8` (the 4 `Ump` slots, about 130 bytes, filled with `FILL` and returned). The words path
  `midi2_to_midi1` needs 109 (`ump_translate_midi2_to_midi1_words`).
- Options: `Translated` as words with `Ump` made on read (`as_slice() -> &[Ump]` is public API, so a change of the
  API); or document `midi2_to_midi1` as the fast path (DEC-018 already gives it).

### OPT-034 — Small copies and clears through `compiler_builtins`

- Area: all crates (Cortex-M targets).
- Found: 2026-10-23: in many benches, `memcpy`, `memclr` and `memcpy4/8` are 15 to 60 % of the instructions
  (`docs/speeds.md`, `scripts/speed-check.py --profile`). The generic `memcpy` of `compiler_builtins` costs about
  5 instructions for each byte when the pointers are not aligned; `__aeabi_memclr4` about 40 for 20 bytes.
- Options: find the values of 16 bytes or more that are made and then moved (as OPT-003 did for the stack), and build
  them in place; keep values below 16 bytes where possible (LLVM then uses stores, not a call). Other cases seen:
  `PeEngine` Get transaction (`__aeabi_memclr4` 17 %, `memcpy` 14 %), `ci_device_receive_discovery` (`memcpy` 22 %).
- Spike (2026-10-26) on `pe_get_transaction_400` (26,822 instructions) and `ci_device_receive_discovery` (2,159):
  1. The bench cleared a 600-byte `scratch` array in each `rx` and a new `Out` (8 x 600 bytes) for each Chunk:
     2,328 instructions of the bench, not of the engine. A static `scratch` and one `Out`: 23,620.
  2. `PeEngine::rx` copied the whole `Txn<H>` (H + 72 bytes) out of its slot and back for each Chunk. In place
     (a small copy of the 4 key fields, `push` on the `pending` field only): -874 instructions, flash
     `pe_engine` -196 B. All `zmidi-ci-pe` tests pass.
  3. `zmidi_ci::message::Header::parse` checked each byte of the whole message with an early exit (`any`). An OR of
     4-byte words with no early exit: `pe_get_transaction_400` -1,912 (-8.1 %), `ci_device_receive_discovery`
     2,159 -> 2,055 (-4.8 %); flash +40 to +44 B (`ci_device`, `ci_one_chunk`, `full`). All `zmidi-ci` tests pass.
  - Left: `compiler_builtins::mem::memcpy` (15 %, the Chunk data in `Emit::bytes`, a real copy) and 116-byte
    `memcpy4` copies in `on_inquiry` and `on_reply` (8 %, not found yet; probably `Header` by value).
- Done for the PE and CI paths (2026-10-26, with items 1 to 3 above): `Header::parse` of zmidi-ci-pe copied the
  128-byte name table `KNOWN` for each Property name (`into_iter` on an array; now by reference); `on_reply` built
  the `PeEvent` again to set `started` (now in place); `Emit::bytes` of zmidi-ci checked each byte with an early
  exit (now `high_bit`, the 4-byte check of item 3, one function for both; property test
  `prop_high_bit_matches_each_byte_check`). The copy of the Chunk data itself costs about 1.4 instructions for each
  byte: a copy loop that LLVM cannot turn into `memcpy` saved 230 more and cost up to +200 B flash, not kept.
  `pe_get_transaction_400` 26,822 -> 19,370 (-28 %, of which 2,328 is the bench fix), `pe_header_parse_inquiry`
  4,039 -> 3,491, `pe_header_parse_reply` 4,946 -> 4,429, `ci_device_receive_discovery` no change. Flash:
  `ci_device` 7,192 -> 7,276, `ci_one_chunk` 12,244 -> 12,328, `pe_engine` 26,712 -> 26,340, `full`
  65,408 -> 65,120. Static RAM and stack do not change.
  - Still open: the `Header` (116 B) moves from `on_inquiry` into `rx` and `Message.header` (a change of the
    public `Message`); the small value moves of `CiDevice::receive` (cause B of OPT-049, not now);
    the bench still clears a new 4,800-byte `Out` before the first Chunk (about 2,200 instructions of the bench).

### OPT-035 — The C ABI reads a MIDI 1.0 byte stream one byte for each call

- Area: `crates/zmidi-ffi` (`zmidi_bytes_to_ump_push`), `bindings/ts/src/io.ts`, `bindings/dart/lib/src/io.dart`.
- Found: 2026-10-23, `bindings/ts/scripts/bench.mjs`: `BytesToUmp.feed` of 64 notes needs 1,700 ns for each message
  (one WASM call and one scratch allocation for each byte); Dart (`bindings/dart/tool/bench.dart`, one FFI call and
  one arena for each byte) needs 621 ns; Python (PyO3, no C ABI) needs 98 ns.
- Options: a C ABI call that reads many bytes into a caller word buffer (for example
  `zmidi_bytes_to_ump_feed(reader, bytes, len, out, cap, &count)`, with the bytes that it did not read on Overflow);
  TS and Dart `feed` then make one call. It adds to the C ABI (`ZMIDI_ABI_MINOR`), so it needs a requirement and a
  decision. The same holds for `UmpToBytes` (one call for each UMP).
- Done 2026-10-25 for `BytesToUmp` (DEC-019 fifth addendum, ABI 0.7, REQ-FFI-IO-007): `zmidi_bytes_to_ump_feed`.
  `feed` of 64 notes: TypeScript 1,736 to 59 ns for each message, Dart 596 to 198 ns; `feed` of one note: TypeScript
  1,765 to 1,173 ns, Dart 609 to 557 ns. Open: `UmpToBytes` stays one call for each UMP (the public API takes one
  UMP), and Dart splits the words with one FFI call for each UMP (`zmidi_ump_word_count`).
- 2026-10-26, the per-call cost (a batch `UmpToBytes` call was the question): a TS bench with shared buffers gave
  81 ns for each UMP against 768 ns for `write`, so the cost was the scratch allocations of each call, not the C
  call. No batch API: the hot paths (`BytesToUmp`, `UmpToBytes`, `BleReader`, `BleWriter`, `UsbLink`) keep one
  scratch block for their call arguments (TS `Blocks.carve`, Dart `Scratch`). TS: `push` 600 to 279 ns, `write` 850
  to 301 ns, `feed` of one note 1,765 to 414 ns, `feed` of 64 notes 59 to 46 ns for each message. Dart: `push` 191
  to 143 ns, `write` 295 to 219 ns, `feed` of one note 529 to 438 ns (the rest is Dart list work).
- 2026-10-26, the other classes: TS `NetHost.receive` of one UMP Data payload (8 notes, 40 bytes) costs 3,326 ns;
  its two blocks for each call (the address and the payload) cost 559 ns (17 %). The large buffers (events, words,
  replies) are kept already. At the rate of UDP packets (about 1,000 for each second) the saving is about 0.5 ms for
  each second, so `NetHost`, `NetClient`, `CiDevice` and `PeEngine` keep their blocks for each call. Not done.

### OPT-036 — `BytesToUmp::push` costs 3.6 times the callback path

- Area: `crates/zmidi-io/src/byte_stream.rs` (`BytesToUmp::push`, `Pushed`).
- Found: 2026-10-23, `tools/lib-compare/speed` (other libraries, same inputs).
- Measurement (instructions for each MIDI 1.0 Note On, 3 bytes): `push` 407.7, `BytesToUmpCore::feed` (callback)
  114.0. Other libraries: cmidi2 49.4, AM_MIDI2.0Lib 120.2, midi2 (C99) 142.2, ni-midi2 169.2.
- Cause: each byte returns a `Pushed` (3 words and 2 counters) by value: `__aeabi_memclr4` and `memcpy` for each
  byte (18 % of the bench), `Pushed::next` (17 %) and `Packet::from_words` (8 %). The callback path has none.
- Status (2026-10-23): option 2 done. New `BytesToUmp::feed` (the words of each UMP to a caller
  function, with the reader's translator; test `prop_reader_feed_gives_the_packets_of_push`, REQ-BS-017). The
  BLE `PacketReader` and the C ABI `zmidi_bytes_to_ump_push` use it. Results: `ble_read_note_midi1` 632.6 ->
  317.7 (-50 %); new benches `io_bytes_to_ump_midi1_note_feed` 105.0 and `io_bytes_to_ump_midi2_note_feed` 281.3
  (`push`: 350.7 and 557.8). No size job changed. The TS binding stays at about 1,650 ns for each message: the cost
  of each call is larger (OPT-035).
- Open: (1) `push` keeps its cost; `zmidi-py` (`io.rs`) and `zmidi-usb` `EventPacketReader` (`push_with`) still use
  it; (3) a smaller `Pushed` (a count and an index into the reader state). The MIDI 2.0 path (281.3, cmidi2 71.4)
  goes through typed values: OPT-037 item 2.

### OPT-037 — UMP translation costs 2 to 4 times midi2 (C99)

- Area: `crates/zmidi-ump/src/translate.rs` (`Midi1ToMidi2Group`, `midi2_to_midi1`).
- Found: 2026-10-23, `tools/lib-compare/speed`.
- Measurement (instructions for each Note On): MIDI 1.0 to 2.0: zetaohm 107.4, midi2 51.1, AM 164.5. MIDI 2.0 to
  1.0 (words): zetaohm 109.1, midi2 26.1, AM 107.3. The midi2 functions are `static inline` in a header and
  translate one message without state; `Midi1ToMidi2` keeps RPN/NRPN and Bank state (D.3) for each channel.
- Compiler: the C libraries built with clang 20 `-Os` (LLVM, as rustc; `tools/lib-compare/speed/build-clang.sh`)
  give -22 % to +30 % against GCC 14. The compiler does not explain a gap of 2 to 4 times.
- Cause (profile): `midi2_to_midi1` cleared its 20-byte `Midi1Words` with a `__aeabi_memclr4` call (30 % of the
  bench). `Midi1ToMidi2Group::translate`: `memcpy4` 24 % and `memclr4` 15 % (the typed `Ump` in and out, OPT-032).
- Status (2026-10-23):
  1. Done: `midi2_to_midi1` builds the whole array in each case (no memclr). 108.1 -> 80.1 (-26 %);
     `ump_translate_midi2_to_midi1_note` (typed, it uses the same function) 403.2 -> 377.2. Flash:
     `midi2_to_midi1_words` 704 -> 488 B (stack 84 -> 64 B), `din_out_each` 1,124 -> 936 B, `din_out` 1,396 ->
     1,276 B, the USB classes -36 B. Test: `prop_midi2_to_midi1_equals_typed_translation` (20,000 cases).
  2. Done (2026-10-24, new public API; REQ-XLATE-026):
     `Midi1ToMidi2Group::translate_words(u32) -> Option<[u32; 2]>`. `BytesToUmpCore::feed_with` (and through it
     the USB reader) uses it. The typed `translate` and `flush` are now on top of it, with
     `Midi2ChannelVoiceMessage::from_translated` (crate-private, no checks); the typed code stays as the test
     reference. Test: `prop_translate_words_equals_typed_translate` (20,000 cases: words, typed on top, flush).
     Instructions for each message, before -> after: `io_bytes_to_ump_midi2_note_feed` 281.3 -> 166.3 (-41 %),
     `io_bytes_to_ump_midi2_note` 557.8 -> 405.8, `usb_event_read_note_midi2` 453.6 -> 361.7 (-20 %),
     `ump_translate_midi1_to_midi2_note` (typed) 108.4 -> 132.5 (+22 %). New bench
     `ump_translate_midi1_to_midi2_words`: 52.4 (midi2 C99 51.1 with no RPN, NRPN or Bank state). Flash: `midi1_to_midi2_lean` 2,492 ->
     1,200 B (-52 %, stack 196 -> 116 B), `midi1_to_midi2` 6,792 -> 6,472 B. Other jobs do not change.
     - The typed cost: the outer function maps the opcode to the enum variant (about 30 instructions) and
       `translate_words` is not inlined. With `#[inline]` the typed path is 115.4, but the words paths get slower
       (feed 182.3, USB 377.7), so it is not inlined.
     - The other choice: keep the typed copy. Then the typed path stays 102 to 108, the lean flash stays 2,492 B,
       and a program that uses the two forms links the two copies.
     - The MIDI 1.0 reader core (`io_bytes_to_ump_midi1_note_feed` about 105) is now the larger part of the feed
       path. It is faster than the other byte readers (AM 120.2, midi2 C99 142.2, ni-midi2 169.2); cmidi2 (49.4)
       converts a whole buffer in one call, so its number is not for each byte.

### OPT-042 — MIDI-CI Profiles and Process Inquiry cost 4,948 B in a responder

- Area: `crates/zmidi-ci` (`CiDevice::message`, `Received::write_next`, `profile.rs`, `process.rs`), features
  `profiles` and `process-inquiry`.
- Found: 2026-10-26, `docs/c-comparison.md` row 9 (job `ci_like_c` against `ci_one_chunk`).
- Measurement: `ci_like_c` 17,276 B against `ci_one_chunk` 12,328 B (+4,948 B flash, +56 B RAM), with no Profile
  added. Symbol diff: `CiDevice::message` +2,374 B (the Profile and Process Inquiry arms), `write_next` and the
  reply writers about +1,300 B (`ProfileDetailsReply::write` 282 B, `PiCapabilities::write` 108 B),
  `ProfileId::same_profile` 186 B, `read_id` 170 B, `read_id_count` 98 B, `ProfileTable::find` 88 B. midi2 C99 does
  the same categories with PE Get and Set in 4,615 B in all.
- Options: not analysed yet. Look first at `message` (the parse of each Profile message to a struct before the
  reply) and at the Profile replies written one field at a time.
- Constraints: DEC-017 wire rules (0/0 Profile Inquiry reply, NAKs); DEC-011; no `unsafe`.
- Status (2026-10-26), step 1 done: `InquiryReplies::write_next` writes both Profile lists with one function
  (`write_table_list`, property test `prop_inquiry_replies_match_generic_writer` against the generic writer):
  -284 B; `peer_max` not inlined (it was inlined at each NAK): `ci_like_c` -488 B, `ci_device` -240 B,
  `ci_one_chunk` -432 B, no speed change. The probe keeps the CI Device in `MaybeUninit` (-184 B on each CI job,
  a measurement fix). Tried and not kept: one NAK builder for `message` (+272 B).
- Status (2026-10-26), step 2 done: DEC-CI-001 addendum: with `profiles` on and `initiator` off,
  Reply to Profile Details Inquiry and the Profile Enabled, Disabled, Added and Removed Reports are ignored and
  their arms are not linked: `ci_like_c` 16,384 -> 15,500 B. Test `profile_messages_for_an_initiator_are_ignored`.
  Left in `ci_like_c` against midi2 C99: the conformance checks, Endpoint Information, `memcpy` (658 B).

### OPT-045 — CI and PE code is 8 to 13 % smaller with less inlining

- Area: `crates/zmidi-ci`, `crates/zmidi-ci-pe` (and the probe profile, opt-level "s").
- Found: 2026-10-26, a build of the probe jobs with opt-level "z" (flash, job minus its reference; direct builds give the same numbers):

  | Job | `s` (now) | `z` |
  |---|---:|---:|
  | `ci_device` | 6,940 | 6,028 |
  | `ci` + `pe` engine | 33,140 | 30,064 |
  | `ci_one_chunk` | 11,768 | 10,800 |
  | `decode_all` | 6,192 | 5,164 |
  | `usb_class_fixed` | 8,140 | 8,456 |
  | `emb_class_fixed` | 9,256 | 9,672 |
  | `midi2_to_midi1_words` | 488 | 832 |

  A first sweep also gave numbers for `-C llvm-args=--inline-threshold=5` and `=25`; direct builds did not give the
  same numbers again (`decode_all` 6,232 B at 25, not 4,736 B), so they are not used. In `decode_all` the `z` build
  only removes `memcpy`, `memcpy4` and `memclr4` (1,016 B), which a real firmware links anyway: no real gain there.
- Cause: with `s`, functions with many call sites are inlined at each one. `tools/size-probe/dup_inline.py ELF
  [FILTER]` lists the inlined functions with more than one inline site and their bytes.
- Done (2026-10-26): `#[inline(never)]` with a comment on five functions, each measured alone, then together:
  `HeaderStr::chars`, `HeaderStr::eq_str`, `PeEngine::find` (zmidi-ci-pe), `Emit::finish`, `ProfileId::to_bytes`
  (zmidi-ci). `scripts/size-check.py`:

  | Job | Flash before (B) | Flash after (B) | Static RAM | Stack before (B) | Stack after (B) |
  |---|---:|---:|---|---:|---:|
  | `ci_device` | 6,940 | 6,844 | no change | 408 | 408 |
  | `ci_one_chunk` | 11,768 | 11,620 | no change | 536 | 536 |
  | `ci_like_c` | 15,500 | 14,948 | no change | 560 | 560 |
  | `pe_one_chunk` | 4,828 | 4,776 | no change | 536 | 536 |
  | `pe_engine` | 26,200 | 25,672 | no change | 1,336 | 1,368 |
  | `full` | 63,320 | 62,692 | no change | 2,424 | 2,424 |

  `scripts/stack-check.py` `probe_pe_receive` 2,764 B (no change). `scripts/speed-check.py`: all in budget;
  `ci_device_receive_discovery` 2,165 -> 2,183, `pe_get_transaction_400` 19,384 -> 19,559 (+0.9 %). Tests of
  `zmidi-ci`, `zmidi-ci-pe` and `zmidi-ump` pass. Budgets lowered.
- Done (2026-10-27), zmidi-net: `Events::push`, `Sink::push`, `ReplyQueue::push`, `OutQueue::push`, `PeerInfo::new`,
  `Handshake::clear` not inlined. `net_client` 31,984 -> 30,232 B, `net_host` 30,552 -> 28,904 B (new jobs); static
  RAM no change; `scripts/stack-check.py` `probe_net_host_receive` 6,168 -> 6,256 B (budget 6,400),
  `probe_net_host_poll` no change. Speed: `net_client_send_note` 11,727 -> 11,982 (+2.2 %), `net_host_receive_note`
  11,953 -> 9,892 (-17 %). Tests of `zmidi-net` (all features) pass. Not kept: `Outcome::event` (-104 B, but
  `probe_net_host_poll` +600 B stack: it takes and gives the `Outcome` by value, over the budget);
  `ReceiveWindow::missing` (+124 to +252 B).
- Done (2026-10-27), zmidi-usb: `MidiLink::report` not inlined: `usb_class` 17,300 -> 17,136 B, `usb_class_fixed`
  8,140 -> 8,020 B, `emb_class` 19,624 -> 19,544 B, `emb_class_fixed` -28 B, `usb_class_lean` -20 B, `emb_class_lean`
  +20 B. Static RAM and stack: no change. Speed: `usb_event_read_note_midi1` 10,302 -> 10,367 (+0.6 %). Not kept:
  `DescWriter::bytes` (+548 B), `ClipWriter::put_word` (+92 B), `ClipWriter::bytes` (+196 B).
- Tried and not kept: `PeEngine::push` and `push_to` (-188 B flash, but `probe_pe_receive` stack +176 B: RAM is
  scarcer than flash); `SysEx7Reassembler::append` (-100 to -160 B, but `io_bytes_to_ump_midi1_note_feed` +6 %, over
  its speed budget, and `ump_sysex7_reassemble_1020` +3.6 %); `PeEngine::nak` (+80 B), `terminate` (+116 B),
  `CiDevice::peer` and `discovery_reply` (mixed), `Muid::from_bytes`, `Emit::fail`, `Emit::raw`, `Scan::peek` (mixed
  or larger).
- Left: the `z` numbers are still 600 to 2,400 B below the `s` numbers in CI and PE; the rest is not in functions
  with many call sites. Option: give the `z` numbers in `docs/sizes.md` as a second column, and tell users in the
  README that a CI or PE firmware is smaller with `z` (and a USB firmware with `s`).

### OPT-046 — USB class construction uses about 1 KB of stack in `init`

- Area: `crates/zmidi-usb-device` (`MidiClass::new`, `MidiClass::fixed`), `crates/zmidi-usb-embassy`.
- Found: 2026-10-26, `tools/size-probe/stack.py`.
- Measurement: frame of the probe `usb::init`: 168 B with `usb_core`, 1,152 B with `usb_class_lean`, 1,448 B
  with `usb_class_fixed`; `emb::init` 1,576 B. This is the peak stack of these jobs in `docs/sizes.md`. The static
  `MidiClass` is 364 to 464 B, so the value is copied about three times (`Result<Self>`, `unwrap`, the tuple
  write).
- Cause: the OPT-003 pattern (a constructor gives `Result<Self, _>` by value).
- Options: an `init(slot: &mut Option<Self>, ..)` form as `Host::init` of zmidi-net (OPT-003 option 3), or a check
  function before a constructor with no `Result`. Low priority: `init` runs once, at start, when the stack is
  empty, so the peak of a real firmware is often in the run loop.
- Constraints: public API (a decision); no `unsafe`.

### OPT-048 — MIDI Clip File reader and writer link the full `Ump::parse`

- Area: `crates/zmidi-file/src/clip_writer.rs:431` (`check_message`), `clip_reader.rs` (`ClipEvent { ump: Ump }`).
- Found: 2026-10-27, jobs `clip_write` (4,812 B) and `clip_read` (4,552 B). In `clip_write`, `check_message` is
  2,600 B: it parses each message to a typed `Ump` only to check its length and to reject the Delta Clockstamps,
  Start and End of Clip and PE SysEx. `smf_read` is 1,192 B (no `Ump`).
- Options: 1. `check_message` on words: the Message Type gives the word count (`MessageType::word_count`), the
  status of word 0 gives the reserved messages, the SysEx7 bytes give the PE check; `FlexDataMessage::parse` only
  for Message Type 0xD, because `event` uses the typed Set Tempo and Set Time Signature. Review (2026-10-27):
  `Ump::parse` fails only with `Truncated`; a packet that its family parser rejects becomes `Ump::Unknown` and is
  accepted. So the word check must accept the same edge cases (for example a Delta Clockstamp or a SysEx7 packet
  that its parser rejects is `Unknown` now, and is written). A property test of the old and the new check on random
  words gives the proof.
- Review 2 (2026-10-27): `ClipWriter::ump` writes the parsed `Ump` again (`Ump::write`), not the input words. A test
  of 2,000,000 random packets: the round trip changes the words of most known messages (MT 0x0: 38,414 of 39,023;
  MT 0x3: 13,808 of 13,874; MT 0x4: 73,107 of 116,681): reserved bits and unused SysEx bytes become zero, for example
  `39130840 1E16AE7C` -> `39130840 1E000000`. So option 1 changes the file bytes for such input (the writer would
  keep the caller's reserved bits). It needs a decision: keep the normalization (no saving) or write the words as
  given (DEC-UMP-001: writers leave reserved bits zero; M2-116 readers must ignore them).
- Decision (2026-10-27): keep the normalization; option 1 is closed (DEC-UMP-001).
- Done (2026-10-27): option 3 (also OPT-039). `zmidi-file` asks `zmidi-ump` for `utility`, `ump-stream`, `sysex7` and
  `flex-data` only (the families of its own Clip rules); a new feature `full` (on by default, DEC-017) gives the
  others. With `full` off, the Clip reader gives a message of another family as `Ump::Unknown`. A `zmidi-ump`
  dev-dependency with `full` keeps the Clip tests on every family in the feature matrix. `zmidi-ffi` (feature
  `file`) turns on `zmidi-file/full`, so the C ABI output does not change; `zmidi-py` and the speed probes ask for
  `zmidi-ump/full` already. Flash (jobs build `zmidi-file` with no default feature): `clip_read` 4,552 -> 3,272 B,
  `clip_write` 4,812 -> 2,836 B, `smf_read` no change; the `decode_cv2_system` tier with `smf_read` 7,744 -> 6,788
  B. Tests of `zmidi-file` (with and without default features) and `zmidi-ffi` (`file`) pass; `ffi-check` passes.
  Option 2 (a word form of `ClipReader`) stays open.
  2. A word form of `ClipReader` (as OPT-044 for `zmidi-ble`), with `ClipEvent` as the typed form on top.
  3. OPT-039: `zmidi-file` still asks for `zmidi-ump/full`.
- Constraints: DEC-001 (M2-116 rules), DEC-018 (word paths), DEC-021; option 2 adds public API.

## Done

### OPT-052 — Size tiers: full MIDI 2.0 by default, opt-out features for small builds (done 2026-10-22)

- Area: all crates; `tools/size-probe`, `scripts/size-check.py`, `scripts/feature-matrix.sh`; DEC-017.
- Goal: with all features on, the library gives full MIDI 2.0. Each feature can be turned off, so that a small build
  uses 1.0x the flash and RAM of the C libraries for the same job (midi2 C99, AM_MIDI2.0Lib, `tusb_ump`).
- Rules:
  1. Default on, opt out: each crate has `default = ["full"]`; `--no-default-features` gives the smallest build
     (DEC-002).
  2. Cargo features only at central dispatch points (`Ump::parse`, the CI message handler, the PE header parser,
     the USB configuration). A word API or a const configuration often removes more than a feature.
  3. A feature that is off still obeys the specification (DEC-001): for example a CI category that is off answers
     with a NAK.
  4. Size is a published, checked number: a size matrix in CI with a budget for each job. The published table is
     `docs/sizes.md` (`python3 scripts/size-check.py --markdown docs/sizes.md`).
- Steps: 1. the size matrix and budgets (`scripts/size-check.py`, a probe with no device crate); 2. the feature
  policy (DEC-017); 3. word paths (OPT-015, DEC-018); 4. the feature-matrix check with `cargo-hack`
  (`scripts/feature-matrix.sh`); 5. dispatch features, one crate at a time; 6. the hard targets: MIDI-CI reply
  writers (OPT-022, OPT-049), the one-chunk PE Responder (OPT-019, OPT-051), the const USB configuration (OPT-016,
  OPT-050).
- Status (2026-10-22): steps 1 to 6 done. The numbers of each job against the C libraries are in `docs/sizes.md`.

### OPT-049 — MIDI-CI size plan (done 2026-10-22; steps 6, 7 and 9 not done)

- Area: `zmidi-ci` (`CiDevice`) and the one-chunk PE Responder of `zmidi-ci-pe`; jobs `ci_device` and
  `ci_one_chunk` (the job of the midi2 C99 number) in `scripts/size-check.py`. Part of OPT-052; it takes the
  options of OPT-022.
- Causes: A. one `CiDevice` for both roles (DEC-009): about 1,300 B. B. four steps for each message (parse,
  `Event`, a `Reply` queue, `write_next`): about 2,500 B. C. replies through a `Writer` with a `Result` for each
  byte: about 1,790 B, and a 512 B reply buffer. D. the conformance checks (they stay). E. fields of categories that
  are off: 80 B of RAM. F. `memcpy` and `memclr8` from values moved by value: up to 860 B.
- Steps:
  0. Tools: the job `ci_one_chunk`, `tools/size-probe/inl.py` (the bytes of a function by inlined callee) and
     `tools/size-probe/types.sh` (the RAM of the CI types). Done.
  1. Byte tests for each reply writer from the M2-101 tables (`crates/zmidi-ci/tests/reply_bytes.rs`). Done.
  2. Cause E: the category fields behind their features (DEC-017 addendum). Done.
  3. Cause F: remove the copies by value. Done.
  4. Cause C: the message writers use a crate-private writer `Emit` that keeps the first error; the NAK of the
     Device has no text, so `Nak::write_without_text` links no text escape code. Done.
  5. A prototype of a one-pass responder (no library change). `crates/zmidi-ci/tests/reply_len.rs` shows that
     each automatic reply fits in `CiDevice::MAX_REPLY_LEN`. Done.
  6, 7 and 9. A one-pass responder API, and the PE Responder on it: not done (about 300 B of flash and 116 B of
     stack, for a second public API).
  8. Cause A: the feature `initiator`, a responder-only build (DEC-CI-001). Done.
  10. Close: the budgets, `docs/sizes.md`, and an interop check of our Initiator against the midi2 C99 responder
      on the host (`tools/lib-compare/interop`, a secondary check, DEC-001). Done.
- Result: OPT-022.

### OPT-051 — One-chunk PE Responder size plan (done 2026-10-23)

- Area: `zmidi_ci_pe::one_chunk` (DEC-PE-005), `zmidi_ci_pe::json`; job `pe_one_chunk`. Part of OPT-052.
- Done: the string scanner decodes into a slice with one body for names, values and skips; the whitespace scanner
  and `Emit::u14` are not inlined; `status_header` fills a constant template; the three string Properties use one
  decoder call. `pe_one_chunk` 6,412 -> 5,172 B flash. The reply bytes do not change.
- Option 1, done (DEC-PE-006, REQ-PE-ONE-005): a flat Header Data reader as an opt-in tier,
  `one_chunk::respond_flat` (4,516 B flash). M2-103 7.1.1 puts the Header Data rules on the sender for devices with
  limited memory, so the tier complies. A property test shows the same request as the complete reader for each
  conforming header. A firmware links only the form that it calls.
- Options left: a one-Chunk reply writer in `zmidi-ci` (about 300 to 400 B, estimate); a constant for the
  one-chunk reply length (for the RAM number of a real firmware).

### OPT-050 — USB class size plan (done 2026-10-23; option 2 is OPT-027)

- Area: `zmidi-usb` and the adapters `zmidi-usb-device` and `zmidi-usb-embassy`; jobs `usb_class_fixed`,
  `emb_class_fixed`, `usb_class_lean`, `emb_class_lean`. Part of OPT-052. Reference: the `tusb_ump` class driver
  (3,538 B flash, 648 B RAM, object size).
- Measurement defects found: the embassy probe called `run` before the data path (with the stub bus `run` never
  yields, so the data path was dead code and not measured); the usb-device probe kept the class in a static whose
  empty value is not all zero bits (OPT-013).
- Steps done (2026-10-22): the Event Packet reader uses the callback form of `zmidi-io` (no `Pushed` value is
  built); the run-time state of `zmidi-usb-embassy` is a `repr(u8)` enum with `Empty = 0`, so a static goes to
  `.bss`.
- Option 1, done (2026-10-22, DEC-014 addendum): a direct Alternate Setting 0 writer. `PacketWriter::write` maps
  each UMP straight to its Event Packets; the earlier writer (UmpToBytes and a byte packetizer) stays in the tests
  as the oracle of `prop_writer_matches_byte_stream_writer`.
- Option 2: a smaller class setup. It is OPT-027.
- Option 3, done (2026-10-23, form B; DEC-014 addendum, REQ-USB-GEN-014, REQ-USB-MS1-027): the feature `alt0`
  (on by default). With it off, Alternate Setting 0 is a USB-MIDI 1.0 setting with no Jack and no endpoint, and
  Alternate Setting 1 is the USB MIDI 2.0 setting. Form A (one setting, USB MIDI 2.0 only) was rejected: a USB 2.0
  host always starts on Alternate Setting 0, and Linux needs two settings for its MIDI 2.0 driver. Host tests: T-015.
- Result: `usb_class_fixed` 8,264 B, `usb_class_lean` 4,976 B, `emb_class_lean` 6,116 B (`docs/sizes.md`).

### OPT-047 — `zmidi-net` links `memmove` (1,452 B) for queues of a few entries (done 2026-10-27)

- Area: `crates/zmidi-net/src/endpoint.rs:276`, `:299`, `:312` (`OutQueue::pop`, `StatelessPings`), `session.rs:212`,
  `:225`, `:479` (`OpenPings`, `ReplyQueue::remove`), `stream.rs:867` (`RetransmitQueue::pop`).
- Found: 2026-10-27, job `net_client`: `compiler_builtins::mem::memmove` 1,428 B, `__aeabi_memmove4` and `8` 24 B.
  `tools/size-probe/callers.py ELF memmove` gives the 7 call sites above, all `copy_within` that remove the first or
  the i-th entry of a small fixed queue. No other job of the probe links `memmove` (OPT-040 removed it from PE).
- Tried and not kept (2026-10-27): a shift with `swap` for each element: LLVM still calls `memmove` for the
  48-byte `(SocketAddr, u32, u64)` entries; flash +212 B. A copy loop over the whole array with a bound test in
  the loop (`if k + gap < len`): one site still calls `memmove`, flash +608 B. LLVM finds each copy loop again, so a
  fix must not move entries (option 1, or a sequence number in each entry and an empty slot for a removed one).
- Options: 1. Ring buffers (a head index, as `OutQueue` has for `Out` already; no shift). 2. Smaller entries: an
  index into the session table in place of a `SocketAddr` (OPT-008), then a shift of a few words is inlined.
  Estimate 1,300 to 1,450 B for a firmware that links nothing else with `memmove`.
- Constraints: no `unsafe`; the order of the queues stays (REQ-NET-PING-003, REQ-NET-PING-007).
- Done (2026-10-27): option 1. `stream::Ring<T, N>` (crate-private): a ring with a head index for the
  five queues (`OutQueue`, `StatelessPings`, `OpenPings`, `ReplyQueue`, `RetransmitQueue`); a removal in the middle
  builds a new ring of the kept entries (copies between two separate values: `memcpy`, which every build links).
  An in-place shift with wrapped indices was tried first: LLVM still found the copy loop (`memmove`). Public API: no
  change. Property test `prop_ring_matches_a_vec_model`; all `zmidi-net` tests pass.
- Fix (2026-10-27): the first form (`head` and `len` as `usize`) made the Host 24 B larger, and
  `scripts/ffi-check.sh` failed on wasm32: `size_of::<HostSlot>() <= size_of::<ZmidiNetHost>()` (zmidi-ffi
  `net.rs:97`, the fixed C ABI storage). Now `head` and `len` are `u16` (N <= 65,535, checked at compile time): the
  ring is not larger than the old array with a `usize` length. `ffi-check` passes. Final numbers against the state
  before OPT-047: flash `net_client` 30,232 -> 29,376 B, `net_host` 28,904 -> 27,872 B (no `memmove`); static RAM no
  change (4,328 B); stack `probe_net_host_new` 5,704 -> 4,808 B, `probe_net_client_new` 5,680 -> 4,776 B,
  `probe_net_host_receive` 6,256 -> 6,232 B. Speed: no change.

### OPT-044 — `zmidi-ble` receive links the full `Ump::parse` (9,280 B) (done 2026-10-27)

- Area: `crates/zmidi-ble/src/reader.rs` (`PacketReader::read`, `Received { ump: Ump }`); `Cargo.toml` asks for
  `zmidi-ump/full`.
- Found: 2026-10-26. No size job measured `zmidi-ble`, `zmidi-net` or `zmidi-file`. Now jobs `ble_rx`, `ble_tx`,
  `net_client`, `net_host`, `smf_read`, `clip_read`, `clip_write` (2026-10-27, budgets in `scripts/size-check.py`).
  First numbers (scratch probe, same profile):

  | Job | Flash (B) | Static RAM (B) |
  |---|---:|---:|
  | `PacketReader::read`, MIDI 2.0 Protocol output | 9,280 | 164 |
  | `PacketWriter::write_packet`, ATT MTU 23 | 2,584 | 56 |
  | both | 11,156 | 224 |

- Cause: `read` gives a typed `Ump`, so `Ump::parse` (4,448 B, all Message Types) and `flex_data::decode` link
  for every BLE receiver. `PacketReader::feed` is 1,002 B and `BytesToUmpCore::feed_with` 788 B. The DIN path
  has the callback word form for this (`din_in_feed` 676 B, DEC-018).
- Options: 1. A word form `PacketReader::read_words(packet, |timestamp, &[u32]|)`; `read` stays as the typed
  convenience over it. Estimate: receive about 3,500 to 4,500 B (not measured). 2. Size jobs: done (2026-10-27); `ble_rx` 9,140 B, `ble_tx` 2,916 B in
  `docs/sizes.md`.
- Constraints: option 1 adds public API (a decision, DEC-016, DEC-018). The `zmidi-ble-trouble` adapter
  and `examples/esp32s3-ble-midi` use `read`.
- Done (2026-10-27): option 1, DEC-BLE-002. `PacketReader::read_words` gives `ReceivedWords
  { timestamp, words }`; `read` is the typed form on top of it. Job `ble_rx_words` 3,508 B flash against `ble_rx`
  9,128 B (-5,620 B, -62 %); static RAM 168 B and stack 88 B, no change. Property test
  `prop_read_words_matches_read` (same items, errors and order as `read`, across packets). `ble_read_note_midi1`
  20,454 -> 20,387. The adapter and the ESP32-S3 example still use `read`.

### OPT-043 — `ItemList::as_slice` bound check stops the optimizer in the run-time USB class (done 2026-10-26)

- Area: `crates/zmidi-usb/src/ms2.rs:168` and `:173` (`as_slice`, `as_mut_slice` of the fixed-capacity list).
- Found: 2026-10-26, panic-site count of the probe job `full` (65 sites; 11 of them are this one line, inlined).
- Measurement (experiment, undone): `&self.items[..self.len.min(N)]` in both functions. Flash (probe build minus
  its reference): `usb_class` 18,652 -> 17,300 B (-1,352 B), `emb_class` -156 B, `usb_class_fixed` -8 B,
  `usb_class_lean` and `emb_class_fixed` 0 B, `full` -1,372 B. Static RAM and stack: no change. `cargo test -p
  zmidi-usb -p zmidi-ump`: all pass. The same change in `Midi1Words`, `SysEx7Data`, `FlexData`, `Sx8Data`, the UMP
  Stream text and `Text<N>` of zmidi-ci-pe gave 0 to +20 B: not useful there.
- Cause: the compiler cannot prove `len <= N`, so each inlined copy keeps a `slice_index_fail` branch. The branch
  blocks more folds in `MidiClass::select`, `usb::init`, `GroupTerminalBlock::to_bytes` and `endpoints`.
- Options: 1. Keep the `.min(N)` in `ms2.rs` (no `unsafe`, no API change, same result because the invariant holds).
  2. Look for the same pattern in the other jobs with `tools/size-probe/panics.py` (panic call sites with source line).
- Constraints: none (no API change, DEC-002 holds).
- Done (2026-10-26): option 1. `scripts/size-check.py`: `usb_class` 18,652 -> 17,300 B, `emb_class` 19,780 ->
  19,624 B, `full` 64,688 -> 63,320 B; the other jobs do not change. Budgets lowered. Static RAM: no change. Peak stack
  `usb_class` 2,448 -> 2,424 B, `emb_class` 2,624 -> 2,632 B. Tests of `zmidi-usb` and the adapters pass.

### OPT-033 — USB-MIDI 1.0 writer clears a 64-byte packet buffer for each UMP

- Area: `crates/zmidi-usb/src/event_packet.rs` (`PacketWriter::write`, `Packets::new`).
- Found: 2026-10-23: `usb_event_write_note` 183 instructions for each note, 22 % of them `__aeabi_memclr4`.
- Cause: the packets of one UMP go to a buffer first, so that `Overflow` writes nothing (REQ-USB-EVT-021).
- Options: count the packets first and write into `out`; a smaller buffer (12 packets is the most for one UMP in
  the unparsed mode). Measure the flash cost with `size-check.py` (`usb_class*`).
- Done (2026-10-26): `write` writes the packets straight into `out`
  when `out` has room for the most bytes of one UMP (20 parsed, 52 unparsed); a smaller `out` goes through a
  52-byte buffer in a function that is not inlined, so REQ-USB-EVT-021 holds. All 274 `zmidi-usb` tests pass.
  `usb_event_write_note` 182.2 -> 160.8 instructions for each note (-12 %); flash `usb_class_fixed` +16 B,
  `usb_class` +16 B, `emb_class_fixed` +64 B, `emb_class` +60 B.
  New property test `prop_writer_small_out_matches_large_out` (the same bytes and cable state from both paths, no
  bytes and no state change on an overflow; it finds a planted defect: no state restore).

### OPT-017 — MIDI-CI handler links every category

- Area: `zmidi-ci` `CiDevice::message` and `receive` (DEC-017 step 5).
- Found: 2026-10-20, prototype change 6.
- Measurement: CI device 16,864 -> 13,128 B with a gate on each Profile and Process Inquiry arm; a gate on the
  category check alone saves 24 B. `ProfileDetailsReply::write` (492 B) stays linked through the Profile Inquiry
  replies of `receive`.
- Cause: `Categories` and `PiCapabilities` are run-time values; the compiler cannot prove the arms are dead.
- Options: the features `profiles`, `process-inquiry`, `initiator` (DEC-017) on each arm and on the inquiry reply
  path; reply writers that copy byte templates (not measured).
- Constraints: DEC-017 wire rules (0/0 Profile Inquiry reply, NAKs, Discovery category bits); DEC-011.
- Status (2026-10-26): done by OPT-049 steps 2 and 8 (DEC-017 addendum of 2026-10-22, DEC-CI-001):
  the features `profiles`, `process-inquiry`, `property-exchange` and `initiator` gate each arm and each category
  call. `ci_device` (no category feature, responder only) is 7,192 B, against 16,864 B before the prototype.

### OPT-018 — PE defaults for small devices (`H`, `Q`)

- Area: `zmidi-ci-pe` `PeEngine<T, H, S, P, Q>`.
- Found: 2026-10-20, prototype change 9.
- Measurement: `PeEngine<2, 256, 4, 2, 16>` -> `<2, 128, 4, 2, 4>`: RAM -448 B, flash -592 B (the `.data` image,
  OPT-013).
- Options: document small-device values; a smaller default `Q`.
- Constraints: `Q >= 3`; a 128 B header buffer is not tested with real hosts.
- Status (2026-10-26): done for `Q`: the `PeEngine` docs give `Q = 4` for a firmware that writes the queue after
  each call (-192 B RAM; no flash effect now that a static engine is in `.bss`, OPT-040). `H = 128` is not
  documented, because no test with a real host exists.

### OPT-040 — PE engine flash: `memmove` and a `str` slice panic path (done 2026-10-26)

- Area: `crates/zmidi-ci-pe/src/json.rs` (`JsonWriter::escaped_body`), `crates/zmidi-ci-pe/src/header.rs`
  (`HeaderChars::next`), `tools/size-probe/src/main.rs` (`ENG`).
- Found: 2026-10-26, symbol diff of the probe (`tools/size-probe/symdiff.py`, jobs `ci` and `ci,pe`).
- Done (flash of `pe_engine`, measured with `scripts/size-check.py`; static RAM and stack do not change):
  1. `escaped_body` wrote the quotes and then removed them with `copy_within`. That one call linked
     `compiler_builtins::mem::memmove` (1,428 B). Now `escaped_text` writes the text with no quotes: -1,496 B.
  2. `HeaderChars::next` cut the text with `&text[c.len_utf8()..]`, which linked `core::str::slice_error_fail_rt`
     (282 B). Now `chars().as_str()`: -336 B.
  3. The probe kept the engine in `static mut ENG: Option<_> = None`, a 1,416 B `.data` image counted as flash
     (OPT-013). Now `MaybeUninit`, as the `PeEngine` docs give to firmware: -1,416 B (a probe change only).
  - `pe_engine` 30,052 -> 26,712 B, `pe_engine_no_subscriptions` 25,936 -> 23,060 B, `full` 68,764 -> 65,408 B.
    The one-chunk jobs do not change (they link neither path). Wire bytes do not change.
- Measured and not kept: a constant template for the status reply in `PeEngine::write_one` (as `one_chunk`
  `status_header`): +352 B, because `Header::write` stays linked for the other replies. `on_inquiry` (2,594 B):
  1,416 B is `on_subscription` in about 9 parts of 20 to 440 B (spec rules); the `S = 0` tier already removes it.

### OPT-030 — Speed fixes found by the speed probe (done 2026-10-23)

- Area: `crates/zmidi-ump/src/sysex7.rs`, `crates/zmidi-ci-pe/src/encoding.rs`, `crates/zmidi-io/src/byte_stream.rs`.
- Found: 2026-10-23, `scripts/speed-check.py --profile`.
- Done (instructions; QEMU mps2-an386, before -> after):
  - `SysEx7::parse` and `SysEx7::data`: a 64-bit shift by a variable amount for each byte (a call or a long
    sequence on a 32-bit target). Now one shift for the mask and the 32-bit halves with constant shifts.
    `ump_sysex7_reassemble_1020`: 395.6 -> 226.6 for each packet (-43 %). `decode_all` flash +64 B (6,128 -> 6,192).
  - `Mcoded7Decoder::decode`: three passes over the input, a variable shift and a bounds check for each byte. Now the
    header byte count from a formula and one pass with the state in locals. 32.2 -> 22.9 for each byte (-29 %).
    New test `prop_decoder_needs_exactly_the_data_bytes` (REQ-PE-ENC-008).
  - `Pushed` (the value of `BytesToUmp::push`): 20 bytes cleared with `__aeabi_memclr4` for each byte (about 40
    instructions). Now 16 bytes (`[u32; 3]`: one byte completes at most 3 words, the second UMP is 1 word), cleared
    with two stores. `io_bytes_to_ump_midi1_note` 500.7 -> 407.7 for each message (-19 %), `ble_read_note_midi1`
    727.6 -> 632.6 (-13 %). `din_in` flash 1,044 -> 1,032 B, stack 80 -> 72 B. New test
    `one_push_gives_sysex_end_and_tune_request` (REQ-BS-006).

### OPT-022 — MIDI-CI Device: where the 9,576 B go (done 2026-10-22)

- Area: `zmidi-ci` `CiDevice` (job `ci_device` in `scripts/size-check.py`: Discovery and management, no category
  feature, `CiDevice<2, 1, 4>`, `max_sysex_size` 512).
- Found: 2026-10-22, the "MIDI-CI reply writers from byte templates" item of OPT-052. Tool: an
  inlined-callee split of `size_probe::ci::run` with `llvm-symbolizer --inlines` over each instruction.
- Measurement (probe, flash): 9,576 B. The C reference (midi2 C99, 4,615 B) includes a one-chunk PE Responder;
  the same job here is `ci_device` + `pe_one_chunk` = 16,072 B (3.5x).
  - Message handling inlined into the probe loop: about 2,950 B (`dispatch` 2,044 B, `rejected_header` 248 B,
    the Group map and the time window).
  - Reply writers: about 1,790 B (`Nak::write` 564 B with the text escape, `DiscoveryReply::write` 512 B,
    `EndpointInfoReply::write` 302 B, `Header::write` 184 B, `InvalidateMuid::write` 156 B, `patch_u14` 70 B).
    Each byte goes through a checked `Writer` call with a `Result`.
  - Message readers: about 1,340 B. Of these, about 690 B read replies that only an Initiator needs
    (`DiscoveryReply::read` 150 B, `Ack::read` 160 B, `Nak::read` 252 B, `EndpointInfoReply::read` 106 B,
    `read_ack_nak` 54 B).
  - Peer table: about 600 B (`change_muid` 322 B, `store_peer` 140 B, `remove_peer` 136 B).
  - SysEx7 reassembly and packets in the probe: about 850 B (`SysEx7Reassembler::push` 542 B, `send` 190 B,
    `SysEx7::data` 118 B). The C probe has its own SysEx7 code in its number.
  - `memcpy` 686 B and `memclr8` 174 B (the C probe links neither).
- Options:
  1. Responder-only build: a feature `initiator` (open in the DEC-017 addendum of zmidi-ci) removes the reply
     readers and the peer table: about 1,000 to 1,300 B. Needs a decision on what a responder does with an
     incoming ACK or NAK (M2-101 5.11, 5.12).
  2. Reply writers with one length check: compute the reply length, check `out` once, then write with no
     `Result` per byte; check the 7-bit fields of the configuration once in `CiDevice::new`. A NAK with no text
     skips `escape_text`. About 500 to 800 B. The reply bytes stay the same (the existing byte tests of
     M2-101 Tables 6 to 9 and 12 check them).
  3. Find the callers of `memcpy` (probably `Event` and reply values moved by value, as in OPT-021): up to
     686 B.
- Plan: OPT-049 (2026-10-22) takes these options and the architecture causes A to F, step by step.
- Estimate: options 1 to 3 give about 6,300 to 7,000 B for `ci_device`, still about 1.4x of the C number
  without its PE part. Confidence of 1.0x for `ci_device` + `pe_one_chunk`: 30%.
- Status (2026-10-22): done through OPT-049 (steps 0 to 5, 8 and 10; steps 6, 7 and 9 not
  now). Options 1 and 2 done; option 3 found only two avoidable copies (the 686 B `memcpy` body is
  a fixed cost of any firmware). Result:

  | Job | Flash before (B) | Flash now (B) | Static RAM before (B) | Static RAM now (B) | Stack now (B) |
  |---|---:|---:|---:|---:|---:|
  | `ci_device` (now responder only) | 9,576 | 7,320 | 1,288 | 680 | 408 |
  | `ci_device_initiator` | 9,576 | 8,640 | 1,288 | 752 | 408 |
  | `ci_one_chunk` | 16,072 | 13,732 | 1,544 | 1,416 | 728 |
  | midi2 C99 CI with PE Get/Set | 4,615 | | 736 | | about 1,192 |

### OPT-023 — C ABI: size jobs in size-check.py, and the second memcpy

- Area: `crates/zmidi-ffi` (DEC-019), `scripts/size-check.py`, `scripts/ffi-check.sh`.
- Found: 2026-10-22, with the module Io of `zmidi-ffi`.
- Measurement: `scripts/ffi-check.sh` links `crates/zmidi-ffi/tests/cmake/fw.c` (the io part: bytes to MIDI 2.0
  Protocol UMP and UMP to bytes, `PROFILE ffi`) for Cortex-M4F with `arm-none-eabi-gcc -Os --gc-sections`:
  flash 5504 B, RAM 208 B, startup code none. The Rust jobs of the same work are `din_in` 1,044 + `midi1_to_midi2_lean`
  2,492 + `din_out` 1,396 = 4,932 B; so the C ABI adds about 570 B. Of that, `compiler_builtins` `memcpy` is 662 B
  (the C program has the memcpy of its libc too). With the workspace `release` profile (no LTO, opt-level 3) the same
  link was 13,226 B; hence `[profile.ffi]`.
- Also (2026-10-22): the same `fw.c` (io calls only) is 6,046 B when the staticlib also has the feature `ump`
  (`zmidi-ump/full`), 542 B more: a zmidi-ump family feature changes code that the io path links. Find which one
  (`cargo bloat` or `nm` of the two links).
- Also (2026-10-22): `fw_ci.c` (a CI Responder with one-chunk PE) is 25,826 B flash and 1,968 B RAM; the Rust job
  `ci_one_chunk` is 13,732 B. The feature `ci` of `zmidi-ffi` takes `zmidi-ci/full` (Initiator, Profiles, Process
  Inquiry) and its Initiator functions are roots. Option: C features that select the zmidi-ci categories and the
  Initiator role (for example `ci-initiator`, `ci-profiles`), as DEC-017 does for Rust.
- Status (2026-10-22): done for the ci part. The C features are `ci-device`, `ci-initiator`, `ci-profiles`,
  `ci-property-exchange` and `ci-process-inquiry`, and `ci` is all of them (REQ-FFI-CI-009, DEC-019 addendum).
  `fw_ci.c` with `ci-device` and `ci-property-exchange` is 17,488 B flash and 1,968 B RAM, against 24,434 B with
  `ci` (-6,870 B). The Rust job `ci_one_chunk` is 13,732 B. The rest of the difference is the C ABI and `memcpy`
  (options 1 and 2 stay open). `scripts/ffi-check.sh` prints the two links.
- Cause: the C link is an absolute size, not a difference to a base build, so it is not in `docs/sizes.md`. Rust
  links its own `memcpy` from `compiler_builtins` into the staticlib.
- Options:
  1. A C job type in `scripts/size-check.py`: link `fw.c` once with no part (base) and once for each part, and
     give the difference, so the C ABI rows go into `docs/sizes.md` with budgets.
  2. Let the C libc give `memcpy`: check if `compiler_builtins` with the `mem` feature off (the default on
     thumbv7em is on) is possible for the staticlib, or if the linker can use the libc copy.
- Constraints: DEC-002, DEC-019; the C tests and `ffi-check.sh` stay the check.

### OPT-024 — `zmidi-ble-trouble` Receiver copies each packet

- Area: `crates/zmidi-ble-trouble`, RAM.
- Found: 2026-10-22, with the adapter.
- Measurement: N bytes for each `Receiver<N>` (248 B in `examples/esp32s3-ble-midi`).
- Cause: the Receiver copies the packet so that the GATT reply goes first (T-011: a full GATT queue drops a PDU).
- Options:
  1. Decode before the reply (the T-011 order fault comes back).
  2. Decode from the PDU after the reply, if a later `trouble-host` lets the data live after `accept`.

### OPT-025 — `zmidi-ffi` net-auth code size

- Area: `crates/zmidi-ffi` (`net-auth`), `crates/zmidi-net` (`auth`).
- Found: 2026-10-22, with the C ABI net authentication.
- Measurement: a Cortex-M4F Host probe (receive, poll, write; `-Os`, gc-sections): +14,520 B flash and +1,072 B RAM.
  WASM `cdylib`: +23,371 B.
- Cause: SHA-256 (`sha2`), the authentication handshake in `zmidi-net`, and an authentication buffer in each session
  (Username, 128 B).
- Options: a smaller Username capacity for the C ABI; a Host without the user method.

### OPT-039 — `zmidi-file` turns on every `zmidi-ump` family

- Area: `crates/zmidi-file/Cargo.toml` (`zmidi-ump` with `features = ["full"]`).
- Found: 2026-10-25, in the check of the SMF feature question (DEC-021). Probe (`thumbv7em-none-eabi`, opt-level
  "s", panic abort; scratch crate that calls `Ump::parse` with `zmidi-ump` `channel-voice1` only): base 1,172 B
  flash; with `zmidi-file` linked and nothing of it called 1,692 B (+520 B) with fat LTO, and 2,776 to 7,016 B
  (+4,240 B) with no LTO (16 codegen units). The binary has no `zmidi_file` symbol in both builds.
- Cause: Cargo feature unification. `zmidi-file` asks for `zmidi-ump/full`, so `Ump::parse` of the whole firmware
  decodes every family (Flex Data, UMP Stream, MIDI 2.0 Channel Voice parsers). The unused code of `zmidi-file`
  itself costs 0 B.
- Options: `zmidi-file` asks only for the families it parses (`utility`, `ump-stream`; the Clip reader gives any
  other message as `Ump::Unknown` when its family is off) and forwards a `full` feature, on by default (DEC-017).
  The SMF modules need no `zmidi-ump` family (channel messages as MT 0x2 words).
- Constraints: DEC-017 (default on, opt out); the Clip reader tests need `full`.

### OPT-038 — the USB descriptor writer of the C ABI on bare metal

- Area: `crates/zmidi-ffi/src/usb.rs` (`zmidi_usb_function_*`, `zmidi_usb_gtb_setup`; DEC-019 third addendum of
  2026-10-23).
- Found: 2026-10-24. Measurement (Cortex-M4F, `ffi` profile, `-Os`, `--gc-sections`, a probe that calls
  `zmidi_usb_function_init_default` and `zmidi_usb_function_write`): 7,792 B flash, 992 B RAM (the
  `ZmidiUsbFunction` storage is 716 B on 32-bit targets). With `zmidi_usb_gtb_setup` also: 8,434 B flash (+642 B),
  1,056 B RAM.
- Cause: the run-time configuration (`FunctionConfig` with 2 settings, 4 endpoints and 16 blocks for each setting)
  and every writer check of `zmidi-usb`.
- Options: a fixed function for C (the descriptor bytes and the Group Terminal Block sets from `fixed::write_rust`
  or a host tool, as C arrays, and a C call over `FixedFunction::gtb_setup`); smaller capacities behind a C feature.
  Measure the fixed path first.
- Measured 2026-10-25 (same flags). The run-time path by symbol: `zmidi_usb_function_write` 2,034 B (the writer,
  inlined), `GtbList::validate_endpoints` 1,094 B, `FunctionConfig::checked_len` 990 B, `memcpy` 662 B (the 716 B
  `FunctionConfig` moves by value), `zmidi_usb_function_init_default` 474 B, `zmidi_usb_gtb_setup` 368 B; the rule
  checks are about 2.6 KB of the 7.8 KB. The fixed path (a probe: the descriptors and one Group Terminal Block set as
  C arrays, and a C call over `FixedFunction::gtb_setup`): 1,114 B flash, 76 B RAM, of which the call is 178 B,
  `memcpy` 662 B and the arrays 154 B. Against 8,434 B with the request handler: about -7.3 KB (-87 %).
- Done 2026-10-25 (DEC-019 fourth addendum, ABI 0.6): `zmidi_usb_fixed_gtb_setup` and the host generator
  `zmidi_usb_gen.h`; the probe `fw_usb_fixed` (Appendix B) is 1,268 B flash and 76 B RAM, with a budget of 1,600 B in
  `scripts/ffi-check.sh`. Open: the smaller capacities behind a C feature for a run-time function (not measured).

### OPT-027 — USB class setup code of the adapters

- Area: `crates/zmidi-usb-device` (`get_configuration_descriptors`, `with_function`, `select`), the same parts of
  `crates/zmidi-usb-embassy`.
- Found: 2026-10-22 (OPT-050 option 2).
- Measurement (`usb_class_fixed`): `get_configuration_descriptors` 970 B (`endpoint_ex` once for each direction,
  322 B), `init` and `with_function` 944 B, `select` 382 B, `control_in` 214 B. Experiment E1 (no run-time check of a
  fixed function): -80 B.
- Options: no run-time check of a fixed function; `select` from the fixed tables; fewer endpoint lookups in the
  descriptor sink. `endpoint_ex` must stay (it sets `bNumEndpoints`). Estimate: 80 to 250 B, confidence 60%.
- Not done now (2026-10-23): 1 to 5 % of the class; a second experiment first.
- Status (2026-10-26), done in part (DEC-USB-002 addendum): FixedFunction has private fields and a const fn
  `new` that checks the adapter endpoint rules at compile time; `Function::CHECKED` lets the usb-device adapter
  skip its run-time checks for a Fixed function: `usb_class_fixed` 8,244 -> 8,140 B, `usb_class_lean`
  4,976 -> 4,872 B, the run-time class unchanged (with a run-time `checked` argument it was +124 B; `alloc_into`
  takes the type `F`). Measured and not kept: a table of the endpoints of each setting for `select` (+24 B;
  embassy +40 B). Left: the descriptor replay into usb-device (970 B, `endpoint_ex` twice for In and Out).

### OPT-028 — two UMP to MIDI 1.0 writers in a firmware with USB and DIN output

- Area: `crates/zmidi-usb/src/event_packet.rs` (`PacketWriter`, direct writer since 2026-10-22) and
  `crates/zmidi-io` (`UmpToBytes`).
- Found: 2026-10-22, job `full`: +108 B after the direct Alternate Setting 0 writer (DEC-014 addendum); the old
  writer shared `UmpToBytes::write` with the DIN output.
- Options: a firmware with both outputs could use one writer; for example the direct writer gives bytes to a
  callback, and `UmpToBytes` uses it. Measure first; the saving is at most about 100 B.

### OPT-026 — Password copies stay in the stack after Network MIDI 2.0 authentication (done 2026-10-23)

- Status (2026-10-23): fixed in `zmidi-net`. The cause was `Digest::finalize`, which takes the state by value: the
  move left the last block (CryptoNonce, Username, password) in the caller frame. `finalize_into_reset` works in
  place. A native stack scan finds no copy; the `bindings/ts` 6.10 test scans the full WASM memory (REQ-NET-AUTH-011).

- Area: `crates/zmidi-net` (`auth`), `crates/zmidi-ffi` (`net-auth`), the bindings. Security and memory.
- Found: 2026-10-22, by a test of `bindings/ts` (M2-124 6.10 example).
- Measurement: after `zmidi_net_*_receive_auth`, 3 or 4 copies of the password bytes stay in the Rust stack area
  of the WASM memory (offsets about 1,040,404 to 1,043,768; the package heap starts at 1,114,112). The packages
  clear their own copies; they cannot clear the Rust stack.
- Cause (not confirmed): `sha2` clears its hash state on drop (feature `zeroize`), but moves of the state and the
  local message schedule of the SHA-256 compress function leave copies in stack frames.
- Options:
  1. A spike: find the frames that keep the bytes (WASM memory dump after each step).
  2. Clear the stack area after each authentication call (a stack-scrub helper in `zmidi-ffi`), or hash through a
     state that lives at one fixed address.
  3. State the limit in the security documentation (the secret is in process memory anyway).
- Constraints: DEC-002, DEC-013, DEC-019.

### OPT-001 — `BytesToUmp` memory use (done 2026-10-17)

- Area: `crates/zmidi-io/src/byte_stream.rs` (`BytesToUmp`), `crates/zmidi-ump/src/translate.rs` (`Midi1ToMidi2`).
- Found: 2026-10-14, after the ByteStream module.
- Measurement: `core::mem::size_of::<BytesToUmp>()` = 4128 bytes on the host (64-bit); `Midi1Scheduler<8>` = 528
  bytes. On `thumbv7em-none-eabihf` (2026-10-17): `BytesToUmp` = 4120, `Midi1ToMidi2` = 4096 (`ChannelState` = 16
  bytes, 256 of them). `BytesToUmp::new` also uses a 4096-byte stack frame (OPT-003).
- Cause: `BytesToUmp` holds an `Option<Midi1ToMidi2>`, and `Midi1ToMidi2` stores `[[ChannelState; 16]; 16]`, the
  RPN/NRPN and Bank Select state for all 16 Groups and 16 Channels (D.3.3, D.3.4). One `BytesToUmp` uses only one
  Group, so 15/16 of that state is never used. The space is reserved also for MIDI 1.0 Protocol output, because
  `Option` keeps the full size.
- Options:
  1. A one-Group translator in zmidi-ump (for example a const generic Group count, or a `Midi1ToMidi2Group` type
     with `[ChannelState; 16]`): about 1/16 of the size.
  2. Make the translator a caller-owned value that `BytesToUmp::push` borrows, so several ports can share one
     16-Group translator and MIDI 1.0 output pays nothing.
  3. Make `ChannelState` smaller (pack the held CC values and flags). Each held value is 7 bits, so 0x80 can
     mean "none": 8 bytes instead of 16. Options 1 and 3 together give 128 bytes for one Group.
- Constraints: DEC-002 (no alloc), D.3.3/D.3.4 translation results must not change; the existing zmidi-ump
  translation tests are the check.
- Done: option 1. zmidi-ump `Midi1ToMidi2Group` holds `[ChannelState; 16]` for one Group; `Midi1ToMidi2` is now
  `[Midi1ToMidi2Group; 16]` and its results are the same (all translate tests, and the property test
  `prop_group_translator_matches_midi1_to_midi2`; a differential test in a scratch crate against the earlier
  code gave the same words for 4000 translator sequences and 4000 `BytesToUmp` byte streams). zmidi-io
  `BytesToUmp` uses `Midi1ToMidi2Group`. The public API
  of zmidi-io and zmidi-usb is the same.
- Result on `thumbv7em-none-eabihf`, release, opt-level "s" (2026-10-17): `Midi1ToMidi2Group` = 256,
  `BytesToUmp` = 280 (was 4120), `PacketReader<1>` = 284 (was 4124), `MidiLink<1>` = 412 (was 4192),
  `MidiLink<16>` = 4748 (was 66112). `BytesToUmp::new` frame = 276 bytes (push 20 + `sub sp` 256, was 4096); a
  caller that writes the result to a slot adds about 296. Options 2 and 3 are not done (option 3 gives 128 bytes
  for one Group).

## Closed (not done)

### OPT-041 — `core::str::from_utf8` in the PE engine (624 B)

- Area: `crates/zmidi-ci-pe` (`header.rs`: `decode_into`, `HeaderChars::next`, `classify`; `json.rs`: names,
  `num_text`, `text`; `subscription.rs`: `Text::as_str`, the subscribeId writer).
- Found: 2026-10-26, symbol list of the probe (`usb2_fixed,ci,pe,bytes1,bytes_out`).
- Measurement: `core::str::converts::from_utf8` 368 B and its 256 B width table (`.Lanon...`, read only by
  `from_utf8`). Prototype: a crate-private check (`utf8_valid_len`, about 30 lines; `char::from_u32` rejects
  surrogates and values over U+10FFFF) and `from_utf8_unchecked` on the checked bytes, at the 9 call sites:
  `ci,pe` -480 B, the probe set above -476 B. A property test against `core::str::from_utf8` (200,000 byte
  strings, 50,000 strings) and all `zmidi-ci-pe` tests pass.
- Cause: `from_utf8` is fast on long text (word steps, the table); PE text is at most a few hundred bytes.
- Options: the prototype; or decode to `char` with no `&str` where the caller needs only characters
  (`HeaderChars::next`), which needs no `unsafe` but keeps `from_utf8` for the other sites.
- Constraints: the prototype adds `unsafe` (two `from_utf8_unchecked` calls) to a crate with none now; the
  property test must stay. Measured and not kept on the way: constant Property names in `Header::write` with no
  checks and no escapes (`name_const`): +104 B.
- Status (2026-10-26): closed, not done, by the project rule: no `unsafe` in the library when it can be avoided,
  and 480 B is not worth it.

### OPT-014 — Host receive stack grew by about 440 bytes with OPT-009 step 1

- Area: `crates/zmidi-net` (`Received::next` and the code inlined into it: `Core::process`, `Core::open`,
  `SessionCore::receive`, `Handshake` steps through the `Hs` view).
- Found: 2026-10-18, `scripts/stack-check.py` at each change: host receive 5744 bytes -> 6240 (OPT-009
  step 1) -> 6184 now; the frame of `Received::next` 2816 -> 3240. Still in the budget (6400).
- Cause (found with the `alloca` list of the LLVM IR, `--emit=llvm-ir`): `Core::open`, inlined into `Received::next`,
  builds the empty `DataStream` in a stack temporary (1048 bytes: the zeroed send history) and copies it into the
  table slot. Before step 1 the stream took the runtime `StreamConfig` and LLVM built it in place; with no runtime
  value it is a constant aggregate, which LLVM materializes in a temporary. The functions inlined into
  `Received::next` are the same before and after (`-C remark=inline`). Tried: `#[inline]` on the `Handshake`
  wrappers (no change), `#[inline(never)]` on `SessionCore::receive` (6904) and on `DataStream::checked` (6272),
  the stream built inside the `SessionCore` literal (no change); kept: a struct literal in `DataStream::checked`
  and `#[inline(always)]` on the empty constructors (6160 now).
- Trade: OPT-009 saves 192 bytes of static RAM for each session, so a Host with 2 or more sessions has a lower peak;
  a 1-session Host or Client is about 250 bytes higher at the peak.
- Options: initialize the send history in the slot (`MaybeUninit` and `ptr::write_bytes`: `unsafe`, against the
  no-`unsafe` style of the crate); or keep a freed entry in its slot and reset it in place (only the first use of a
  slot builds a value).
- Status (2026-10-26): closed, not done: the only option needs `unsafe` (the project rule: no `unsafe` in the
  library when it can be avoided). The stack stays in its budget.

### OPT-003 — Large values built on the stack and then copied

- Area: `crates/zmidi-net/src/endpoint.rs` (`Core::open`, `Host::new`, `Client::new`),
  `crates/zmidi-ci-pe/src/transaction.rs` (`PeEngine::new`, `PeEngine::invalidate`),
  `crates/zmidi-ci-pe/src/subscription.rs` (`accept_start`, `add_started`), `crates/zmidi-io/src/byte_stream.rs`
  (`BytesToUmp::new`).
- Found: 2026-10-17, RAM survey (method above).
- Measurement: `Received::next` (Host) has a 7008-byte frame and a 10896-byte deepest stack. `Core::open` is inlined
  in it: `Session::host` writes a `Result<Session>` (about 3 KB) to the stack, then `memcpy` moves it into
  `Some(Entry { .. })`; the `Opening` argument (712) is a third value. `PeEngine::invalidate`: 3368 (a local
  `[Option<SubscriptionEntry>; S]`). `PeEngine::new`: 3000. `BytesToUmp::new`: 4096. `accept_start`, `add_started`:
  about 700 each (a `SubscriptionEntry` by value). A probe that put `Host<1>` (5072) in a static had a 10176-byte
  frame: `Host::new` gives the whole value by value, and the default `Host` is 14168 bytes.
  `ZlibInflater::new`: 10596 (the `miniz_oxide` decompressor by value). With `auth`, `Received::next` needs 14176.
- Cause: a constructor that returns `Self` (or `Result<Self, _>`) and a move into a table slot. Rust does not
  promise to build the value in place, and with opt-level "s" it does not.
- Options:
  1. Build in the slot: write the empty value into the table first (`self.entries[i] = Some(Entry { .. })` with a
     reset value), then call the setup on `&mut` in place (`Session::reset_host(&mut self, ..)`).
  2. `const fn new` where the arguments allow it, so a `static` gets the value at compile time (no stack, no copy).
     `Host::new` checks its configuration: split into a `const fn` that builds and a separate `check`.
  3. An in-place initializer for the large public types: `fn init(slot: &mut MaybeUninit<Self>, ..)` or
     `fn reset(&mut self, ..)`, for use with `static_cell` and RTIC resources.
  4. `PeEngine::invalidate`: give each removed entry to the `ended` storage at once (no local array); see OPT-004.
- Constraints: DEC-002. Options 2 and 3 add public API (a decision). Option 1 changes no API. Add a stack check
  to CI so that a later change does not bring the copies back.
- Status (2026-10-17): the `BytesToUmp::new` part is done with OPT-001 (frame 4096 -> 276 bytes).
- Status (2026-10-18), measured with `scripts/stack-check.py` (CI step, budgets in the script):
  - zmidi-net option 1: `Core::open` and `Core::connect` build the session in the table slot from a checked
    configuration (`Session::host_checked`, no `Result`). Host `receive` + `Received::next` 9200 -> 5792 bytes
    (largest frame 6264 -> 2856); with `auth` 8704. A `const` empty `Session` (5384 bytes) was not kept: it puts a
    2984-byte image in flash.
  - zmidi-net option 3: `Host::init` and `Client::init` write into a caller slot: 2248 and 2232 bytes. `Host::new`
    and `Client::new` (check, then build with no `Result`) 12424 -> 6968 and 12408 -> 6936.
  - zmidi-ci-pe option 4: `invalidate` gives each removed entry to `ended` at once: `PeEngine::receive` 4584 -> 4220
    (largest frame 3376 -> 1720).
  - Open: `PeEngine::new` stays 3048 (1720 after OPT-004 on 2026-10-18). The compiler builds the `[None; N]` arrays on the stack and copies them, also
    into a caller slot and with `core::array::from_fn` (worse: 4704). A fix needs `MaybeUninit` (unsafe) or less
    state: OPT-004 option 1 removes the 1332-byte `ended` table. `ZlibInflater::new` is not measured yet (`zlib`).
- Status (2026-10-26): closed. The parts with no `unsafe` are done; the rest (`PeEngine::new`, the `[None; N]` arrays
  built on the stack) needs `MaybeUninit` (`unsafe`), which the library does not use.
