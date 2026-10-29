# Changelog

All notable changes to the zetaohm-midi crates and packages. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/). The versions follow
[Semantic Versioning](https://semver.org/), with the rule for 0.x versions: any 0.x release can
change the API. All crates and packages have the same version.

## [Unreleased]

### Changed

- `zmidi-file`: `SmfWriter::escape` refuses a channel status byte (0x80 to 0xEF) with `BadValue`. Channel
  messages are MIDI events in a Standard MIDI File (RP-001).

### Fixed

- `zmidi-ci`, `zmidi-ci-pe`: a responder-only build (feature `initiator` off) can send Subscription updates
  again. The Device keeps the PE Capabilities from the Inquiry of the peer, and `pe_request` accepts a
  Subscription message in both roles (M2-101 8.11). (#1)
- `zmidi-ci-pe`: the reply path of the Property Exchange engine uses about 730 B less stack (`on_reply` no
  longer copies the transaction). (#1)
- `zmidi-ci-pe`: an Inquiry: Get or Set Property Data with no "mutualEncoding" is "ASCII" (M2-103 6.1.6).
  For a Resource whose "encodings" do not list "ASCII", the responder now replies 415. Before, the
  inquiry went to the application, and a reply in another encoding was refused, so the transaction
  stayed open until its timeout.

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
