# zmidi-sysex

MIDI 1.0 Universal System Exclusive messages for `no_std` and no-alloc code: the Universal header, the Manufacturer
ID and Device ID rules, Device Inquiry, General MIDI System On and Off, Device Control (Master Volume, Balance, Fine
and Coarse Tuning), the MIDI Tuning messages, MIDI Time Code (Quarter Frame generator and reader, Full Message, User
Bits), the MIDI Machine Control transport commands (Guideline Minimum Set 1, PLAY and PAUSE, with the Standard Time
Code) and Notation Information (Bar Marker, Time Signature). Each part is a Cargo feature; all are on by default.

| Feature | Part |
|---|---|
| `inquiry` | Identity Request and Identity Reply, with a responder |
| `gm` | General MIDI 1 System On, General MIDI System Off, General MIDI 2 System On |
| `device-control` | Master Volume, Master Balance, Master Fine Tuning, Master Coarse Tuning |
| `tuning` | The MIDI Tuning messages (bulk dumps, single note and scale/octave tuning) |
| `mtc` | MIDI Time Code: the Quarter Frame data byte, generator and reader, Full Message, User Bits |
| `mmc-transport` | MIDI Machine Control transport commands and responses (needs `mtc`) |
| `notation` | Bar Marker, Time Signature (Immediate and Delayed, with compound pairs) |

The Quarter Frame functions take and give the data byte of F1H, so they work with every transport.

Part of [zetaohm-midi](https://github.com/Zetaohm/zetaohm-midi). License: MIT.
