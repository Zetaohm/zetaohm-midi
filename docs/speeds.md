# Speed of the hot paths

Written by `python3 scripts/speed-check.py --markdown docs/speeds.md`. Executed Thumb-2
instructions on QEMU `mps2-an386` (Cortex-M4F), opt-level "s", fat LTO: not cycles
(docs/optimization-backlog.md, measurement method of speed). An item is one message, packet
or byte of the bench input; the bench names and inputs are in `tools/speed-probe/src/benches/`.

| Bench | Items | Instructions | For each item | Budget |
|---|---:|---:|---:|---:|
| `ump_parse_mixed` | 64 | 6,370 | 99.5 | 7,040 |
| `ump_parse_all_types` | 9 | 977 | 108.6 | 1,088 |
| `ump_translate_midi1_to_midi2_note` | 64 | 8,412 | 131.4 | 9,216 |
| `ump_translate_midi1_to_midi2_words` | 64 | 3,420 | 53.4 | 3,712 |
| `ump_translate_midi2_to_midi1_note` | 64 | 24,012 | 375.2 | 26,560 |
| `ump_translate_midi2_to_midi1_words` | 64 | 5,189 | 81.1 | 5,696 |
| `ump_sysex7_reassemble_1020` | 170 | 37,840 | 222.6 | 40,896 |
| `io_bytes_to_ump_midi1_note` | 64 | 22,446 | 350.7 | 24,704 |
| `io_bytes_to_ump_midi1_note_feed` | 64 | 7,233 | 113.0 | 7,424 |
| `io_bytes_to_ump_midi2_note` | 64 | 25,971 | 405.8 | 28,608 |
| `io_bytes_to_ump_midi2_note_feed` | 64 | 10,449 | 163.3 | 11,712 |
| `io_ump_to_bytes_note_running` | 64 | 2,385 | 37.3 | 2,624 |
| `usb_event_write_note` | 64 | 10,291 | 160.8 | 11,328 |
| `usb_event_read_note_midi1` | 64 | 10,367 | 162.0 | 11,200 |
| `usb_event_read_note_midi2` | 64 | 21,803 | 340.7 | 25,472 |
| `usb_ump_words_to_bytes` | 64 | 273 | 4.3 | 320 |
| `usb_ump_bytes_to_words` | 64 | 516 | 8.1 | 576 |
| `ble_write_note` | 64 | 21,494 | 335.8 | 23,552 |
| `ble_read_note_midi1` | 64 | 20,256 | 316.5 | 22,400 |
| `file_clip_write_note` | 64 | 19,464 | 304.1 | 21,568 |
| `file_clip_read_note` | 64 | 19,263 | 301.0 | 20,224 |
| `net_client_send_note` | 64 | 11,982 | 187.2 | 13,184 |
| `net_host_receive_note` | 64 | 11,957 | 186.8 | 13,184 |
| `net_host3_receive_note_last_session` | 64 | 12,219 | 190.9 | 13,568 |
| `ci_device_receive_discovery` | 1 | 2,165 | 2,165.0 | 2,432 |
| `pe_get_transaction_400` | 1 | 19,384 | 19,384.0 | 21,312 |
| `pe_header_parse_inquiry` | 1 | 3,492 | 3,492.0 | 3,904 |
| `pe_header_parse_reply` | 1 | 4,430 | 4,430.0 | 4,928 |
| `pe_mcoded7_encode_1020` | 1020 | 24,660 | 24.2 | 26,816 |
| `pe_mcoded7_decode_1020` | 1020 | 23,370 | 22.9 | 25,728 |
