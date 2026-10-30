# Changelog

All notable changes to the zetaohm-midi crates and packages. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/). The versions follow
[Semantic Versioning](https://semver.org/), with the rule for 0.x versions: any 0.x release can
change the API. All crates and packages have the same version.

## [Unreleased]

### Added

- `zmidi-ump`: feature `nfc` (with `alloc`): `text::is_nfc` and `text::to_nfc` for UMP text in
  Normalization Form C (M2-104 1.8).
- `zmidi-ump`: `FlexTextPackets::project_name` and `FlexTextPackets::midi_clip_name`, to the Address
  Group (M2-104 7.5.9.1).

### Changed

- `zmidi-ci`: `Discovery::write` and `DiscoveryReply::write` refuse a Receivable Maximum SysEx Message
  Size below 512 when the categories have Profile Configuration or Property Exchange (M2-101 5.5.3,
  Appendix F).
- `zmidi-ci`: `PeChunk::write` refuses an Inquiry: Get Property Data or a Reply to Set Property Data
  that is not Chunk 1 of 1 with no Property Data (M2-101 Tables 33, 36).
- `zmidi-ci`: `ReportWriter` leaves out the inactive notes for Message Data Control 0x01 (M2-101
  Table 44); the application still gives only the non-default controller values.
- `zmidi-ci-pe`: the ResourceList writer leaves out a "mediaTypes" list equal to the default
  `["application/json"]`, as for the other default values (M2-103 14.3), unless `write_defaults` is
  set.
- `zmidi-file`: `SmfWriter::escape` refuses a channel status byte (0x80 to 0xEF) with `BadValue`. Channel
  messages are MIDI events in a Standard MIDI File (RP-001).

### Fixed

- `zmidi-ci-pe`: `Link::write` refuses an empty "selectedProperty" (M2-103 Table 72), as the reader
  does.
- `zmidi-ci-pe`: the ResourceList reader marks a Column "title" as truncated only when it was cut, not
  when it fills the buffer exactly.
- `zmidi-usb`: after a USB reset or a configuration change, `MidiLink` writes packets of the
  wMaxPacketSize of Alternate Setting 0 again, not of the last USB MIDI 2.0 setting.
- `zmidi-ffi`: `zmidi_usb_link_read` counts the ignored last 1 to 3 bytes of a transfer as an error, as
  zmidi-usb reports them.
- `zmidi-ci`, `zmidi-ci-pe`: a responder-only build (feature `initiator` off) can send Subscription updates
  again. The Device keeps the PE Capabilities from the Inquiry of the peer, and `pe_request` accepts a
  Subscription message in both roles (M2-101 8.11). (#1)
- `zmidi-ci-pe`: the reply path of the Property Exchange engine uses about 730 B less stack (`on_reply` no
  longer copies the transaction). (#1)
- `zmidi-ci-pe`: an Inquiry: Get or Set Property Data with no "mutualEncoding" is "ASCII" (M2-103 6.1.6).
  For a Resource whose "encodings" do not list "ASCII", the responder now replies 415. Before, the
  inquiry went to the application, and a reply in another encoding was refused, so the transaction
  stayed open until its timeout.
- `zmidi-ci-pe`: the Partial SET reader and writer refuse a key with a "~" that is not "~0" or "~1"
  (RFC 6901), with `InvalidKey`.
- `zmidi-net`: a Client that gets Invitation Reply: Authentication Required or User Authentication
  Required with a reserved Authentication State and the nonce that it answered already treats it as
  a repeat of the first request (M2-124 Tables 15, 16), as for state 0x00. Before, it asked the
  caller for the credentials again.

## [0.1.0-alpha.1]

First public release.

### Added

- `zmidi-ump`: Universal MIDI Packet messages of MIDI 2.0 (M2-104-UM), with MIDI 1.0 and MIDI 2.0
  translation (Appendix D).
- `zmidi-io`: MIDI 1.0 byte streams to and from UMP, UMP Endpoint, Jitter Reduction and Group routing.
- `zmidi-ci`: MIDI Capability Inquiry (M2-101-UM): Discovery, Profile Configuration, Process Inquiry
  and the Property Exchange message framing.
- `zmidi-ci-pe`: MIDI-CI Property Exchange (M2-103-UM): the engine, the one-chunk Responder and the
  optional `zlib+Mcoded7` encoding.
- `zmidi-file`: MIDI Clip File (M2-116-U) and Standard MIDI File 1.0 reader and writer.
- `zmidi-net`: Network MIDI 2.0 over UDP (M2-124-UM), sans-IO, with optional authentication.
- `zmidi-usb`: USB-MIDI 2.0 and 1.0 codecs and descriptors, sans-IO; the adapters
  `zmidi-usb-device` (`usb-device` 0.3) and `zmidi-usb-embassy` (`embassy-usb` 0.6).
- `zmidi-ble`: BLE-MIDI packets and GATT data, sans-IO; the adapter `zmidi-ble-trouble`
  (`trouble-host` 0.7).
- `zmidi`: one crate that re-exports the parts above, each as a Cargo feature.
- `zmidi-ffi`: the C ABI (`zmidi.h`), and the packages `zetaohm-midi` for Python (PyPI), TypeScript
  (npm, WASM) and Dart (pub.dev).

[Unreleased]: https://github.com/Zetaohm/zetaohm-midi/compare/v0.1.0-alpha.1...HEAD
[0.1.0-alpha.1]: https://github.com/Zetaohm/zetaohm-midi/releases/tag/v0.1.0-alpha.1
