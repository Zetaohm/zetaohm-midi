# Outstanding tests

Tests that did not run because of a constraint: hardware, host operating system, tool or time. When a test runs,
its entry moves to "Done" with the date and the result. Entries are not deleted. Help is welcome: if you have the
hardware or the host of an open entry, please report the result in an issue.

Each entry gives: what to test, why it did not run, what it needs, and where the gap is recorded.

## Open

### T-001 — USB-MIDI 1.0 (Alternate Setting 0) on a real host
- Test: `examples/mk20dx-usb-midi` on a host with only a USB-MIDI 1.0 driver. Check that the host selects
  Alternate Setting 0 (defmt log), and that `host/echo.py` (or `amidi` on Linux) gets every message back.
- Why not run (2026-10-18): macOS 27 always selects Alternate Setting 1. The Ubuntu 22.04 Parallels VM (Linux
  5.15, USB-MIDI 1.0 driver only) needs a USB connect by hand in the Parallels menu; it was not done.
- Needs: Linux before 6.5, or Windows without Windows MIDI Services, connected to the FLXS1 board.

### T-002 — `zmidi-usb-embassy` on hardware
- Test: `examples/esp32s3-usb-midi` (`cargo run --release` through the UART connector, the USB MIDI function on
  the USB connector): enumeration, `examples/mk20dx-usb-midi/host/echo.py` in Alternate Settings 0 and 1, the
  zero-length packet (a SysEx of exactly 64 bytes of packets), and the switch between the settings.
- Why not run: no board with an `embassy-usb` driver is available. The MK20DX (FLXS1) has only a `usb-device` bus.
  On 2026-10-18 the ESP32-S3 example builds and passes clippy, but no ESP32-S3 is attached (no USB vendor 0x303A).
- Needs: an ESP32-S3 board with the OTG port on a connector (for example the ESP32-S3-DevKitC-1), or an STM32
  board (for example the STM32WB55 of DEC-003), or `examples/stm32wb55-usb-midi` (T-014).
- Recorded in: DEC-015 consequences.

### T-003 — the `zmidi-usb-embassy` endpoint layout on Windows, macOS and Linux
- Test: the DEC-015 layout (OUT 1 / IN 1 on Alternate Setting 0, OUT 2 / IN 2 on Alternate Setting 1) with the
  host drivers of the three systems. USB-MIDI v2.0 Appendix B shares the endpoints, so a host can expect that.
- Why not run: needs T-002 first.
- Needs: the T-002 board, and a Windows, a macOS and a Linux host.
- Recorded in: DEC-015 consequences.

### T-004 — USB MIDI 2.0 (Alternate Setting 1) on Windows and Linux
- Test: `examples/mk20dx-usb-midi` with Windows MIDI Services and with Linux 6.5 or later (USB MIDI 2.0 driver).
- Why not run (2026-10-18): only macOS 27 is tested.
- Needs: a Windows 11 host with Windows MIDI Services, a Linux 6.5+ host.

### T-005 — MIDI 2.0 Protocol UMPs on a real host
- Test: send MIDI 2.0 Channel Voice messages (Message Type 0x4), UMP Stream messages (Endpoint Discovery) and
  System Exclusive 8 from a host UMP API, and check the echo and the Group Terminal Block answer.
- Why not run (2026-10-18): `host/echo.py` uses the MIDI 1.0 API (mido), so the host sent only MIDI 1.0 Protocol
  UMPs. The firmware does not answer Endpoint Discovery.
- Needs: a host UMP API (CoreMIDI MIDI 2.0 API on macOS, or Windows MIDI Services).

### T-006 — `zmidi-usb-device` at high speed
- Test: a high-speed device (wMaxPacketSize 512, `BusSpeed::High`).
- Why not run: the MK20DX USB is full speed only.
- Needs: a board with a high-speed `usb-device` bus (for example `synopsys-usb-otg` with an external ULPI PHY).

### T-007 — live MIDI-CI and Property Exchange interop
- Test: `zmidi-ci` and `zmidi-ci-pe` against JUCE and the MIDI 2.0 Workbench: Discovery, the MUID collision, ACK and
  NAK, Flow Control and Notify.
- Why not run: the interop notes of DEC-010 and DEC-011 come from a source review, not from a live
  session.
- Needs: a transport to JUCE or the Workbench (USB MIDI 2.0 device, or Network MIDI 2.0), and the host programs.

### T-008 — `zmidi-net` against another Network MIDI 2.0 implementation
- Test: `examples/net-echo` `host` with a Client of Windows MIDI Services (and other implementations), and the
  `client` with their Host: Invitation, UMP Data, FEC and Retransmit with packet loss, Ping, Bye, and
  authentication (`auth`).
- Why not run (2026-10-18): only our own Host and Client are tested together (loopback and LAN). macOS has no
  Network MIDI 2.0 implementation. The Windows 11 Parallels VM can run Windows MIDI Services, but it is not set
  up.
- Needs: Windows MIDI Services (Windows 11 VM or PC) on the same network. DNS-SD is in the example now; the
  Linux (`avahi-publish-service`) and Windows (`dns-sd -R`) registration paths are not tested.

### T-010 — the CI job of the ESP32-S3 example
- Test: the `esp32s3-examples` job of `.github/workflows/ci.yml` (esp-rs/xtensa-toolchain), which builds
  `examples/esp32s3-net-echo`, `examples/esp32s3-usb-midi` and `examples/esp32s3-ble-midi`.
- Why not run (2026-10-18): the job was written from the action documentation and nothing was pushed; its inputs
  are not checked.
- Needs: a push to GitHub.

### T-012 — `zmidi-ble` against other BLE-MIDI implementations
- Test: packets from Android MIDI and a BLE-MIDI library (for example lathoub Arduino-BLE-MIDI) read with no error,
  and packets of `PacketWriter` accepted by them: running status, SysEx in continuation packets, Real-Time inside
  SysEx, a timestampLow wrap.
- Done for Apple CoreMIDI on 2026-10-19 (see T-011 under "Done"). Not run: Android, a library implementation, and a
  timestampLow wrap and Real-Time inside SysEx from a central other than the `gatt` tool.
- Why not run: no Android device or second BLE-MIDI board at hand.
- Needs: an Android device with a BLE-MIDI app, or a second board with a BLE-MIDI library.
- Recorded in: DEC-BLE-001 (readings AMB-BLE-MIDI-01 to -11).

### T-014 — `examples/stm32wb55-usb-midi` on a P-NUCLEO-WB55
- Test: `cargo run --release` (probe-rs through the ST-LINK), the USB MIDI function on the "USB USER" connector
  (PA11/PA12): enumeration, `examples/mk20dx-usb-midi/host/echo.py` in Alternate Settings 0 and 1, the
  zero-length packet (a SysEx of exactly 64 bytes of packets), and the switch between the settings. Also check the
  48 MHz USB clock from PLL Q of `WPAN_DEFAULT` (32 MHz HSE) and that the CPU2 firmware above 512 KB of flash stays
  intact.
- Why not run (2026-10-22): no STM32WB55 board is attached. The example builds and passes clippy only.
  The test stays open until a board is available.
- Needs: a P-NUCLEO-WB55 (or another STM32WB55 board with the USB FS pins on a connector) and probe-rs.
- It is also a board for T-002 and T-003.

### T-015 — an empty Alternate Setting 0 (OPT-050 option 3, form B) on real hosts
- Test: a test build of `examples/mk20dx-usb-midi` with `CableCounts::new(0, 0, ...)` on Alternate Setting 0
  (REQ-USB-MS1-027). macOS 27: the ports show and `host/echo.py` gets every message back on Alternate Setting 1.
  Ubuntu 22.04 VM (Linux 5.15, USB-MIDI 1.0 driver only): the device binds with no port and no error. Linux 6.5+
  and Windows 11 with Windows MIDI Services: Alternate Setting 1 is selected and the echo works.
- Run in part (2026-10-23, macOS 27, pass): a test build of `examples/esp32s3-usb-midi` (Adafruit ESP32-S3
  Reverse TFT Feather, `zmidi-usb-embassy`) with an empty Alternate Setting 0 and the serial number
  "esp32s3-alt0-empty". macOS selects Alternate Setting 1 (`host/stat.py`: "setting 1"), shows the ports "Block 1"
  and "MIDI 2.0", and `host/echo.py` gets all 6 messages back. The board runs its earlier firmware again.
- Why the rest did not run (2026-10-23): the FLXS1 board was in use for other work. No Windows or Linux 6.5+ host
  is available. The Ubuntu 22.04 VM does not run with the Parallels version at hand.
- Needs: a Windows 11 host, a Linux 6.5+ host, and the Ubuntu 22.04 VM with the board connected (USB-MIDI 1.0
  host: the device must bind with no port and no error).
- Recorded in: OPT-050 option 3.

### T-018 — USB and BLE throughput and latency on hardware (OPT-029 step 4)
- Test: messages for each second and round-trip latency of the USB echo (FLXS1, `examples/mk20dx-usb-midi`) and of
  the BLE echo (ESP32-S3), from a host script.
- Why not run (2026-10-23): no host-side script for throughput and latency yet; T-017 measured the CPU cost only.
- Needs: a host script (macOS CoreMIDI) with timestamps, the FLXS1 and an ESP32-S3.
- Recorded in: docs/optimization-backlog.md OPT-029.

## Done

### T-019 — the fault paths of the `zmidi-ble-trouble` loops (done 2026-10-25, host tests, pass)
- Test: `Receiver::next` when the GATT reply to a write fails (for example a link that drops during a write), and
  `Sender::send` when the packet pool of trouble-host has no free packet (the notify retries of `RETRY_DELAY`).
- Why not run (2026-10-25): T-016 ran the normal paths only; the notify retries stayed 0, and no GATT reply failed.
  The adapter's async loop has no host test (it needs a BLE controller).
- Needs: the ESP32-S3 Feather and a firmware that makes the faults (a small packet pool, a disconnect in a write), or
  a host test with a fake `trouble-host` controller.
- Spike (2026-10-25): firmware builds with a smaller trouble-host packet pool
  (`TROUBLE_HOST_DEFAULT_PACKET_POOL_SIZE` 4 and 2, default 16) do not reach these paths. With 4, `gatt` and `flood`
  pass and the notify retries stay 0. With 2, the host drops a received PDU first (`host rx errors` 1, counted by
  `host_rx_errors`): the lost SysEx part gives 200 read errors and a short `gatt` echo, but `flood` of 64000 bytes
  still passes; the notify retries stay 0 and no GATT reply fails. A host test needs a fake HCI controller of our own
  (connection, ATT PDUs, completed packets): the mock controller of trouble-host 0.7 is private and has `todo!()`
  bodies. Estimate: 300 to 500 lines.
- Done (2026-10-25) as host tests: the adapter takes the notification and the end of a write as small seams
  (`Sender::send_with`, `Receiver::finish`), so `src/tests.rs` makes them fail with no BLE controller:
  `receiver_decodes_a_packet_when_the_reply_fails`, `sender_retries_when_the_pool_is_full` (the same packets, two
  waits for each packet) and `sender_stops_on_another_error` (the UMPs not taken stay). `send` and `next` call the
  same code with trouble-host.
- Recorded in: T-016 (done).

### T-016 — `examples/esp32s3-ble-midi` with `zmidi-ble-trouble` (done 2026-10-25, pass)
- Board: Adafruit ESP32-S3 Reverse TFT Feather, macOS 27, esp toolchain 1.95, trouble-host 0.7, tools in
  `examples/esp32s3-ble-midi/host`.
- GATT (`gatt`, `flood`): properties 0x1E, a read gives 0 bytes, ATT_MTU 251 (packets of 248 bytes), connection
  interval 15 ms, data length 251, 2M PHY; the T-011 packets come back the same (363 of 363 bytes), and SysEx of
  4000 to 64000 bytes (64000 in 2.6 s, about 24.5 kB/s).
- CoreMIDI (`connect`, `echo.py`, `sx`): interval 7.5 ms, data length 130; the 6 `echo.py` messages, and SysEx of
  4000 to 64000 bytes through `MIDISendSysex` (64000 in 20.7 s) and 16000 bytes paced come back the same.
- Status line: dropped 0, too long 0, notify retries 0, host rx errors 0; errors 1 is the incomplete Song Select of
  the `gatt` packets (the same as T-011).
- Fault found (macOS, not the adapter): with the BLE address of the trouble-host 0.5.1 build, macOS used its cached
  GATT handles: the read gave the old value (7 bytes) and the notify request failed with "The handle is invalid". The
  example now has a new address. The GATT service of trouble-host 0.7 has no Service Changed characteristic.
- Not run: `Receiver::next` when a GATT reply fails, and `Sender::send` with a full packet pool (notify retries
  stayed 0): T-019.

### T-017 — cycle counts of the speed benches on hardware (done 2026-10-23, pass)
- Board: FLXS1 (MK20DX256, J-Link), `examples/mk20dx-speed-bench` (72 MHz core, 24 MHz flash), the
  code of 2026-10-23, rustc 1.98.1. All 27 benches ran and their output checks passed.
- Result: cycles / QEMU instructions = 1.35 to 2.55 (median about 1.9). Low: Mcoded7 (1.35, tight loops in the
  flash cache). High: UMP parse (2.5), BLE read (2.3), DIN parse (2.3). Example: DIN note 939 cycles = 13 us.
- `python3 scripts/speed-check.py --no-build --hw LOG` gives the table (exit 0).

### T-013 — USB echo on hardware after the DEC-018 word paths (done 2026-10-20, pass)
- Board: FLXS1 (MK20DX256, J-Link), firmware `examples/mk20dx-usb-midi` of 2026-10-20, macOS 27
  (Alternate Setting 1).
- `uv run host/echo.py` gets all 6 messages back with the same bytes: Note On, Note Off, Control Change, Program
  Change, Pitch Bend and a SysEx of 9 data bytes.
- Not in this run: Alternate Setting 0 (USB-MIDI 1.0). That part stays in T-001.

### T-009 — `examples/esp32s3-net-echo` on an ESP32-S3 (done 2026-10-19, pass)
- Board: Adafruit ESP32-S3 Reverse TFT Feather, a 2.4 GHz WPA2 network, macOS 27.
- DHCP gives an address; `dns-sd -B _midi2._udp` and `dns-sd -L` find "zetaohm-midi esp32s3" at
  `zetaohm-esp32s3.local.:5673` with the TXT records; `client --browse` finds the Host and echoes 16 to 510 UMPs
  (8 to 255 note pairs); two Clients at the same time get 400 UMPs each; a Client that stops with no Bye is closed by
  the Host after about 15 s (`Timeout`, Bye reason 4, `ByeTimeout`).
- Faults found and fixed: the firmware echoed at most 64 UMPs of one UDP payload and dropped the rest with no log
  (now 368, the most one 1472-byte payload can carry, and a drop counter), and it ignored a full send queue (now it
  sends the pending data and tries again); the `net-echo` client stopped with `Stream(Full)` at more than 256 words
  and left the session open (now it waits for room).
- Seen once in 8 tries: `client --browse` found no Host. The client sends one mDNS query and waits 2 s; RFC 6762
  section 5.2 lets a querier send the query again.

### T-011 — `examples/esp32s3-ble-midi` on an ESP32-S3 (done 2026-10-19, pass)
- Board: Adafruit ESP32-S3 Reverse TFT Feather (ESP32-S3 rev 0.1, 4 MB flash), macOS 27, tools in
  `examples/esp32s3-ble-midi/host`.
- GATT (`gatt`, `flood`): properties 0x1E, a read gives 0 bytes, ATT_MTU 247, connection interval 15 ms, data
  length 251, 2M PHY. Echo the same for running status, CC, Pitch Bend, Program Change, a SysEx in 2 packets with
  a Clock inside, and SysEx of 4000 to 64000 bytes (25 kB/s each way); an incomplete Song Select gives 1 read error.
- CoreMIDI (`connect`, `echo.py`, `sx`): interval 7.5 ms, data length 130; the 6 `echo.py` messages, 200 fast notes,
  and SysEx up to 64000 bytes through `MIDISendSysex` come back the same; 0 drops, 0 host receive errors.
- Faults found and fixed in the example (not in `zmidi-ble`): a full batch lost one UMP (also in
  `esp32s3-usb-midi`); with a data length of 27 the echo was slower than the input; back pressure in the GATT task
  filled the `trouble-host` 0.5.1 GATT queue, which drops a PDU with no log.
- Limit found outside the project: the macOS Bluetooth MIDI driver drops data when a client sends more than about
  14 kB at once (`mido`/`rtmidi` send a 16000-byte SysEx as 13830 bytes).

