/* REQ-FFI-MIDI1-008: the messages, channel-mode, controllers, parameters, sync and active-sensing parts
 * through the header, with vectors from M1 v4.2.1, RP-015 and RP-018. The file needs only these parts
 * (ZMIDI_MIDI1). */
#ifndef ZMIDI_MIDI1
#define ZMIDI_MIDI1
#endif
#include "zmidi.h"

#include <stdio.h>
#include <string.h>

static int failures;

#define CHECK(cond)                                                                                \
  do {                                                                                             \
    if (!(cond)) {                                                                                 \
      printf("FAIL %s:%d: %s\n", __FILE__, __LINE__, #cond);                                       \
      failures++;                                                                                  \
    }                                                                                              \
  } while (0)

static ZmidiMidi1Reader reader;
static uint8_t sysex[8];

/* Reads `n` bytes and writes the messages into `out` (at most `cap`); gives their count. */
static size_t feed(const uint8_t *bytes, size_t n, ZmidiMidi1Message *out, size_t cap) {
  size_t i, j, k = 0, total = 0;
  ZmidiMidi1Message m[ZMIDI_MIDI1_READER_MAX_EVENTS];
  for (i = 0; i < n; i++) {
    CHECK(zmidi_midi1_reader_push(&reader, bytes[i], sysex, sizeof sysex, m, ZMIDI_MIDI1_READER_MAX_EVENTS, &k) ==
          ZMIDI_STATUS_OK);
    for (j = 0; j < k && total < cap; j++) {
      out[total++] = m[j];
    }
  }
  return total;
}

static ZmidiMidi1Message cv(uint8_t status, uint8_t d1, uint8_t d2) {
  ZmidiMidi1Message m;
  m.kind = ZMIDI_MIDI1_KIND_CHANNEL_VOICE;
  m.status = status;
  m.data1 = d1;
  m.data2 = d2;
  m.length = 0;
  return m;
}

static void test_reader_and_writer(void) {
  /* M1 p. 5: running status; p. 30: a System Real Time byte inside a System Exclusive message. */
  static const uint8_t in[] = {0x90, 0x3C, 0x40, 0x3E, 0x40, 0xF0, 0x7E, 0x7F, 0xF8, 0x06, 0x01, 0xF7};
  static const uint8_t sx[] = {0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7};
  static const uint8_t long_sx[] = {0xF0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 0xF7};
  ZmidiMidi1Message m[8];
  ZmidiMidi1Writer writer;
  uint8_t out[16];
  size_t n, k;
  CHECK(zmidi_midi1_reader_init(&reader) == ZMIDI_STATUS_OK);
  n = feed(in, sizeof in, m, 8);
  CHECK(n == 4);
  CHECK(m[0].kind == ZMIDI_MIDI1_KIND_CHANNEL_VOICE && m[0].status == 0x90 && m[0].data1 == 0x3C && m[0].data2 == 0x40);
  CHECK(m[1].kind == ZMIDI_MIDI1_KIND_CHANNEL_VOICE && m[1].status == 0x90 && m[1].data1 == 0x3E);
  CHECK(m[2].kind == ZMIDI_MIDI1_KIND_SYSTEM && m[2].status == 0xF8);
  CHECK(m[3].kind == ZMIDI_MIDI1_KIND_SYS_EX && m[3].length == sizeof sx && memcmp(sysex, sx, sizeof sx) == 0);
  /* A System Exclusive message longer than the buffer: its full length. */
  n = feed(long_sx, sizeof long_sx, m, 8);
  CHECK(n == 1 && m[0].kind == ZMIDI_MIDI1_KIND_SYS_EX_TOO_LONG && m[0].length == sizeof long_sx);
  /* AMB-M1-13: the reset of the program clears running status. */
  n = feed(in, 1, m, 8);
  CHECK(zmidi_midi1_reader_reset(&reader) == ZMIDI_STATUS_OK);
  n = feed(in + 1, 2, m, 8);
  CHECK(n == 0);
  /* A small array: no byte is read. */
  CHECK(zmidi_midi1_reader_push(&reader, 0xF8, sysex, sizeof sysex, m, 1, &n) == ZMIDI_STATUS_BUFFER_TOO_SMALL);

  /* The writer with running status: 90 3C 40, then 3E 40. */
  CHECK(zmidi_midi1_writer_init(&writer, true) == ZMIDI_STATUS_OK);
  m[0] = cv(0x90, 0x3C, 0x40);
  CHECK(zmidi_midi1_writer_write(&writer, &m[0], NULL, out, sizeof out, &k) == ZMIDI_STATUS_OK && k == 3);
  CHECK(out[0] == 0x90 && out[1] == 0x3C && out[2] == 0x40);
  m[0] = cv(0x90, 0x3E, 0x40);
  CHECK(zmidi_midi1_writer_write(&writer, &m[0], NULL, out, sizeof out, &k) == ZMIDI_STATUS_OK && k == 2);
  CHECK(out[0] == 0x3E && out[1] == 0x40);
  CHECK(zmidi_midi1_writer_refresh_status(&writer) == ZMIDI_STATUS_OK);
  CHECK(zmidi_midi1_writer_write(&writer, &m[0], NULL, out, 2, &k) == ZMIDI_STATUS_BUFFER_TOO_SMALL);
  CHECK(zmidi_midi1_writer_write(&writer, &m[0], NULL, out, sizeof out, &k) == ZMIDI_STATUS_OK && k == 3);
  /* A System Exclusive message from the caller array; one with a bad end is invalid. */
  m[0].kind = ZMIDI_MIDI1_KIND_SYS_EX;
  m[0].status = 0xF0;
  m[0].length = sizeof sx;
  CHECK(zmidi_midi1_writer_write(&writer, &m[0], sx, out, sizeof out, &k) == ZMIDI_STATUS_OK && k == sizeof sx);
  CHECK(memcmp(out, sx, sizeof sx) == 0);
  m[0].length = sizeof sx - 1;
  CHECK(zmidi_midi1_writer_write(&writer, &m[0], sx, out, sizeof out, &k) == ZMIDI_STATUS_INVALID_ARGUMENT);
  /* A Channel Voice message with a data byte above 7FH is invalid. */
  m[0] = cv(0x90, 0x80, 0x40);
  CHECK(zmidi_midi1_writer_write(&writer, &m[0], NULL, out, sizeof out, &k) == ZMIDI_STATUS_INVALID_ARGUMENT);
}

static void test_mode(void) {
  /* p. 20 to 26: Omni Off on the Basic Channel selects Mode 3; All Notes Off acts on the instrument. */
  ZmidiModeConfig config = {0, 0x0F, true, 16, false};
  ZmidiModeState mode;
  ZmidiModeAction a;
  ZmidiModeInfo info;
  ZmidiMidi1Message m;
  CHECK(zmidi_mode_init(&mode, &config) == ZMIDI_STATUS_OK);
  CHECK(zmidi_mode_info(&mode, &info) == ZMIDI_STATUS_OK && info.mode == 1 && info.local);
  m = cv(0xB0, 0x7C, 0x00);
  CHECK(zmidi_mode_receive(&mode, &m, &a) == ZMIDI_STATUS_OK);
  CHECK(a.kind == ZMIDI_MODE_ACTION_KIND_MODE_CHANGE && a.changed && !a.alternate);
  CHECK(zmidi_mode_info(&mode, &info) == ZMIDI_STATUS_OK && info.mode == 3);
  m = cv(0xB0, 0x7B, 0x00);
  CHECK(zmidi_mode_receive(&mode, &m, &a) == ZMIDI_STATUS_OK);
  CHECK(a.kind == ZMIDI_MODE_ACTION_KIND_ALL_NOTES_OFF && a.channel == 0xFF);
  /* Mode 3: a voice message on another channel is not for the instrument. */
  m = cv(0x91, 0x3C, 0x40);
  CHECK(zmidi_mode_receive(&mode, &m, &a) == ZMIDI_STATUS_OK && a.kind == ZMIDI_MODE_ACTION_KIND_IGNORE);
  m = cv(0x90, 0x3C, 0x40);
  CHECK(zmidi_mode_receive(&mode, &m, &a) == ZMIDI_STATUS_OK && a.kind == ZMIDI_MODE_ACTION_KIND_VOICE &&
        a.voice == 0xFF);
  /* Local Control off (p. 24); the System Reset action gives Mode 1 and Local On again. */
  m = cv(0xB0, 0x7A, 0x00);
  CHECK(zmidi_mode_receive(&mode, &m, &a) == ZMIDI_STATUS_OK && a.kind == ZMIDI_MODE_ACTION_KIND_LOCAL_CONTROL &&
        !a.on);
  CHECK(zmidi_mode_system_reset(&mode) == ZMIDI_STATUS_OK);
  CHECK(zmidi_mode_info(&mode, &info) == ZMIDI_STATUS_OK && info.mode == 1 && info.local);
  /* The front panel; invalid values. */
  CHECK(zmidi_mode_set_basic_channel(&mode, 16) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_mode_set_basic_channel(&mode, 4) == ZMIDI_STATUS_OK);
  CHECK(zmidi_mode_set_omni(&mode, false) == ZMIDI_STATUS_OK);
  CHECK(zmidi_mode_set_mono(&mode, 4) == ZMIDI_STATUS_OK);
  CHECK(zmidi_mode_info(&mode, &info) == ZMIDI_STATUS_OK && info.mode == 4 && info.basic_channel == 4 && info.m == 4);
  CHECK(zmidi_mode_set_poly(&mode) == ZMIDI_STATUS_OK);
  CHECK(zmidi_mode_set_local(&mode, false) == ZMIDI_STATUS_OK);
  CHECK(zmidi_mode_info(&mode, &info) == ZMIDI_STATUS_OK && info.mode == 3 && !info.local);
  config.basic_channel = 16;
  CHECK(zmidi_mode_init(&mode, &config) == ZMIDI_STATUS_INVALID_ARGUMENT);
  m.kind = ZMIDI_MIDI1_KIND_SYSTEM;
  CHECK(zmidi_mode_receive(&mode, &m, &a) == ZMIDI_STATUS_INVALID_ARGUMENT);
}

static void test_controllers(void) {
  ZmidiControllerState s;
  ZmidiControllerEvent e;
  ZmidiMidi1Message m, w[3];
  bool has;
  uint16_t bank;
  uint8_t v;
  size_t n;
  CHECK(zmidi_controllers_init(&s) == ZMIDI_STATUS_OK);
  /* p. 12: an MSB, then an LSB alone. */
  m = cv(0xB0, 0x01, 0x12);
  CHECK(zmidi_controllers_receive(&s, &m, &e) == ZMIDI_STATUS_OK && e.kind == ZMIDI_CONTROLLER_EVENT_KIND_CONTROLLER14 &&
        e.controller == 1 && e.value == 0x12 << 7);
  m = cv(0xB0, 0x21, 0x34);
  CHECK(zmidi_controllers_receive(&s, &m, &e) == ZMIDI_STATUS_OK && e.value == (0x12 << 7 | 0x34));
  /* p. 17, Example 2: B0 54 3C, 90 40 40 glides from 60; a later Note On has no source. */
  m = cv(0xB0, 0x54, 0x3C);
  CHECK(zmidi_controllers_receive(&s, &m, &e) == ZMIDI_STATUS_OK && e.kind == ZMIDI_CONTROLLER_EVENT_KIND_NONE);
  m = cv(0x90, 0x40, 0x40);
  CHECK(zmidi_controllers_receive(&s, &m, &e) == ZMIDI_STATUS_OK && e.kind == ZMIDI_CONTROLLER_EVENT_KIND_PORTAMENTO &&
        e.source == 0x3C);
  CHECK(zmidi_controllers_receive(&s, &m, &e) == ZMIDI_STATUS_OK && e.kind == ZMIDI_CONTROLLER_EVENT_KIND_NONE);
  /* pp. 13, 14: Bank Select MSB and LSB, then Program Change with the bank. */
  CHECK(zmidi_bank_program_messages(0, 0x0102, 0x11, w, 3, &n) == ZMIDI_STATUS_OK && n == 3);
  CHECK(w[0].status == 0xB0 && w[0].data1 == 0x00 && w[0].data2 == 0x02);
  CHECK(w[1].status == 0xB0 && w[1].data1 == 0x20 && w[1].data2 == 0x02);
  CHECK(w[2].status == 0xC0 && w[2].data1 == 0x11 && w[2].data2 == 0);
  CHECK(zmidi_controllers_receive(&s, &w[0], &e) == ZMIDI_STATUS_OK && e.kind == ZMIDI_CONTROLLER_EVENT_KIND_NONE);
  CHECK(zmidi_controllers_receive(&s, &w[1], &e) == ZMIDI_STATUS_OK);
  CHECK(zmidi_controllers_receive(&s, &w[2], &e) == ZMIDI_STATUS_OK && e.kind == ZMIDI_CONTROLLER_EVENT_KIND_PROGRAM &&
        e.program == 0x11 && e.has_bank && e.bank == 0x0102);
  /* RP-015: Reset All Controllers keeps the bank; Expression to 127, Balance kept. */
  m = cv(0xB0, 0x79, 0x00);
  CHECK(zmidi_controllers_receive(&s, &m, &e) == ZMIDI_STATUS_OK &&
        e.kind == ZMIDI_CONTROLLER_EVENT_KIND_RESET_ALL_CONTROLLERS);
  CHECK(zmidi_controllers_bank(&s, &has, &bank) == ZMIDI_STATUS_OK && has && bank == 0x0102);
  CHECK(zmidi_controller_reset_value(11, &v) == ZMIDI_STATUS_OK && v == 127);
  CHECK(zmidi_controller_reset_value(64, &v) == ZMIDI_STATUS_OK && v == 0);
  CHECK(zmidi_controller_reset_value(8, &v) == ZMIDI_STATUS_OK && v == ZMIDI_CONTROLLER_KEEP);
  CHECK(zmidi_controllers_reset(&s) == ZMIDI_STATUS_OK);
  /* The 14-bit writer: MSB then LSB. */
  CHECK(zmidi_controller14_messages(5, 1, 0x1234, w, 2, &n) == ZMIDI_STATUS_OK && n == 2);
  CHECK(w[0].status == 0xB5 && w[0].data1 == 0x01 && w[0].data2 == 0x24);
  CHECK(w[1].status == 0xB5 && w[1].data1 == 0x21 && w[1].data2 == 0x34);
  CHECK(zmidi_controller14_messages(5, 32, 0, w, 2, &n) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_controller14_messages(5, 1, 0, w, 1, &n) == ZMIDI_STATUS_BUFFER_TOO_SMALL);
}

static void test_parameters(void) {
  /* RP-018 example: B0 65 00, B0 64 00, B0 60 00, B0 60 00, B0 65 7F, B0 64 7F. */
  static const uint8_t rp18[][2] = {{0x65, 0x00}, {0x64, 0x00}, {0x60, 0x00}, {0x60, 0x00}, {0x65, 0x7F}, {0x64, 0x7F}};
  ZmidiParameterState s;
  ZmidiParameterEvent e;
  ZmidiParameter p;
  ZmidiMidi1Message m, w[4];
  uint16_t pbs = 2 << 7;
  bool selected, rpn, nrpn;
  size_t i, n;
  CHECK(zmidi_parameters_init(&s) == ZMIDI_STATUS_OK);
  CHECK(zmidi_parameters_reception(&s, &rpn, &nrpn) == ZMIDI_STATUS_OK && rpn && !nrpn);
  for (i = 0; i < sizeof rp18 / sizeof rp18[0]; i++) {
    m = cv(0xB0, rp18[i][0], rp18[i][1]);
    CHECK(zmidi_parameters_receive(&s, &m, &e) == ZMIDI_STATUS_OK);
    if (e.change != ZMIDI_PARAMETER_CHANGE_NONE) {
      CHECK(e.change == ZMIDI_PARAMETER_CHANGE_INCREMENT && e.parameter.registered && e.parameter.msb == 0 &&
            e.parameter.lsb == 0);
      CHECK(zmidi_parameter_apply(&e, pbs, &pbs) == ZMIDI_STATUS_OK);
    }
  }
  CHECK(pbs == (2 << 7 | 2));
  CHECK(zmidi_parameters_selection(&s, &selected, &p) == ZMIDI_STATUS_OK && !selected);
  /* p. 50: Bn 64 03 65 00 06 tt selects Tuning Program tt. */
  m = cv(0xB0, 0x64, 0x03);
  CHECK(zmidi_parameters_receive(&s, &m, &e) == ZMIDI_STATUS_OK && e.change == ZMIDI_PARAMETER_CHANGE_NONE);
  m = cv(0xB0, 0x65, 0x00);
  CHECK(zmidi_parameters_receive(&s, &m, &e) == ZMIDI_STATUS_OK);
  m = cv(0xB0, 0x06, 0x05);
  CHECK(zmidi_parameters_receive(&s, &m, &e) == ZMIDI_STATUS_OK && e.change == ZMIDI_PARAMETER_CHANGE_DATA_ENTRY_MSB &&
        e.value == 5 && e.parameter.registered && e.parameter.lsb == 3);
  /* RP-015: Reset All Controllers selects no parameter. */
  m = cv(0xB0, 0x79, 0x00);
  CHECK(zmidi_parameters_receive(&s, &m, &e) == ZMIDI_STATUS_OK && e.change == ZMIDI_PARAMETER_CHANGE_NONE);
  CHECK(zmidi_parameters_selection(&s, &selected, &p) == ZMIDI_STATUS_OK && !selected);
  /* NRPN reception goes on: both bytes again. */
  CHECK(zmidi_parameters_set_reception(&s, true, true) == ZMIDI_STATUS_OK);
  CHECK(zmidi_parameters_reception(&s, &rpn, &nrpn) == ZMIDI_STATUS_OK && rpn && nrpn);
  /* The writer: number MSB, LSB, then Data Entry MSB and LSB (p. 17). */
  p.registered = true;
  p.msb = 0;
  p.lsb = 0;
  CHECK(zmidi_parameter_value_messages(3, &p, 2 << 7 | 50, w, 4, &n) == ZMIDI_STATUS_OK && n == 4);
  CHECK(w[0].status == 0xB3 && w[0].data1 == 0x65 && w[0].data2 == 0);
  CHECK(w[1].data1 == 0x64 && w[2].data1 == 0x06 && w[2].data2 == 2 && w[3].data1 == 0x26 && w[3].data2 == 50);
  CHECK(zmidi_parameter_select_messages(3, &p, w, 2, &n) == ZMIDI_STATUS_OK && n == 2);
  CHECK(zmidi_parameter_change_message(3, ZMIDI_PARAMETER_CHANGE_DECREMENT, 0, &w[0]) == ZMIDI_STATUS_OK &&
        w[0].data1 == 0x61 && w[0].data2 == 0);
  CHECK(zmidi_parameter_change_message(3, ZMIDI_PARAMETER_CHANGE_NONE, 0, &w[0]) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_parameter_value_messages(3, &p, 0x4000, w, 4, &n) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_parameter_value_messages(3, &p, 0, w, 3, &n) == ZMIDI_STATUS_BUFFER_TOO_SMALL);
}

static ZmidiMidi1Message sys(uint8_t status, uint8_t d1, uint8_t d2) {
  ZmidiMidi1Message m = cv(status, d1, d2);
  m.kind = ZMIDI_MIDI1_KIND_SYSTEM;
  return m;
}

static void test_sync(void) {
  /* M1 p. 28: Song Position Pointer 4 (24 clocks), Continue and three Timing Clocks give the 27th
   * clock; p. 31: no advance while stopped; p. 32: Start while playing is ignored. */
  ZmidiSyncReceiver s;
  ZmidiSyncEvent e;
  ZmidiSyncInfo info;
  ZmidiMidi1Message m;
  int i;
  CHECK(zmidi_sync_init(&s) == ZMIDI_STATUS_OK);
  m = sys(0xF2, 4, 0);
  CHECK(zmidi_sync_receive(&s, &m, &e) == ZMIDI_STATUS_OK && e.kind == ZMIDI_SYNC_EVENT_KIND_LOCATE && e.position == 24);
  m = sys(0xFB, 0, 0);
  CHECK(zmidi_sync_receive(&s, &m, &e) == ZMIDI_STATUS_OK && e.kind == ZMIDI_SYNC_EVENT_KIND_CONTINUE);
  m = sys(0xF8, 0, 0);
  for (i = 0; i < 3; i++) {
    CHECK(zmidi_sync_receive(&s, &m, &e) == ZMIDI_STATUS_OK && e.kind == ZMIDI_SYNC_EVENT_KIND_CLOCK &&
          e.position == (uint32_t)(24 + i));
  }
  CHECK(zmidi_sync_info(&s, &info) == ZMIDI_STATUS_OK && info.position == 27 && info.run == 2 && info.sync_mode);
  CHECK(zmidi_sync_command(&s, ZMIDI_SYNC_COMMAND_START, &e) == ZMIDI_STATUS_OK && e.kind == ZMIDI_SYNC_EVENT_KIND_NONE);
  CHECK(zmidi_sync_command(&s, ZMIDI_SYNC_COMMAND_STOP, &e) == ZMIDI_STATUS_OK && e.kind == ZMIDI_SYNC_EVENT_KIND_STOP);
  CHECK(zmidi_sync_receive(&s, &m, &e) == ZMIDI_STATUS_OK && e.kind == ZMIDI_SYNC_EVENT_KIND_NONE);
  /* Song Select out of MIDI sync mode is ignored; the System Reset action stops at position 0. */
  CHECK(zmidi_sync_set_mode(&s, false) == ZMIDI_STATUS_OK);
  m = sys(0xF3, 5, 0);
  CHECK(zmidi_sync_receive(&s, &m, &e) == ZMIDI_STATUS_OK && e.kind == ZMIDI_SYNC_EVENT_KIND_NONE);
  CHECK(zmidi_sync_system_reset(&s) == ZMIDI_STATUS_OK);
  CHECK(zmidi_sync_info(&s, &info) == ZMIDI_STATUS_OK && info.position == 0 && info.run == 0 && !info.sync_mode);
  CHECK(zmidi_sync_command(&s, 9, &e) == ZMIDI_STATUS_INVALID_ARGUMENT);
  m = cv(0x90, 0x3C, 0x40);
  CHECK(zmidi_sync_receive(&s, &m, &e) == ZMIDI_STATUS_INVALID_ARGUMENT);
}

static void test_active_sensing(void) {
  /* M1 p. 32: no timeout before the first FEH; after it, more than 330 ms with no byte gives one
   * timeout; the sender asks for FEH after 270 ms with no byte. */
  ZmidiActiveSensingMonitor m;
  ZmidiActiveSensingSender s;
  bool timeout, armed, send;
  CHECK(zmidi_active_sensing_monitor_init(&m, ZMIDI_ACTIVE_SENSING_TIMEOUT_US) == ZMIDI_STATUS_OK);
  CHECK(zmidi_active_sensing_monitor_poll(&m, 10000000u, &timeout, &armed) == ZMIDI_STATUS_OK && !timeout && !armed);
  CHECK(zmidi_active_sensing_monitor_receive_byte(&m, 0xFE, 0) == ZMIDI_STATUS_OK);
  CHECK(zmidi_active_sensing_monitor_receive_byte(&m, 0x90, 300000u) == ZMIDI_STATUS_OK);
  CHECK(zmidi_active_sensing_monitor_poll(&m, 630000u, &timeout, &armed) == ZMIDI_STATUS_OK && !timeout && armed);
  CHECK(zmidi_active_sensing_monitor_poll(&m, 630001u, &timeout, &armed) == ZMIDI_STATUS_OK && timeout && !armed);
  CHECK(zmidi_active_sensing_monitor_poll(&m, 2000000u, &timeout, &armed) == ZMIDI_STATUS_OK && !timeout);
  CHECK(zmidi_active_sensing_monitor_receive(&m, true, 3000000u) == ZMIDI_STATUS_OK);
  CHECK(zmidi_active_sensing_monitor_poll(&m, 3000001u, &timeout, &armed) == ZMIDI_STATUS_OK && armed);
  CHECK(zmidi_active_sensing_monitor_is_armed(&m, &armed) == ZMIDI_STATUS_OK && armed);
  CHECK(zmidi_active_sensing_sender_init(&s, ZMIDI_ACTIVE_SENSING_INTERVAL_US) == ZMIDI_STATUS_OK);
  CHECK(zmidi_active_sensing_sender_poll(&s, 269999u, &send) == ZMIDI_STATUS_OK && !send);
  CHECK(zmidi_active_sensing_sender_poll(&s, 270000u, &send) == ZMIDI_STATUS_OK && send);
  CHECK(zmidi_active_sensing_sender_sent(&s, 400000u) == ZMIDI_STATUS_OK);
  CHECK(zmidi_active_sensing_sender_poll(&s, 669999u, &send) == ZMIDI_STATUS_OK && !send);
}

/* One MPE input: a parameter receiver for each channel gives the RPN events to the MPE receiver. */
static ZmidiParameterState mpe_parameters[16];

static ZmidiMpeEvent mpe_feed(ZmidiMpeReceiver *r, uint8_t status, uint8_t d1, uint8_t d2) {
  ZmidiMidi1Message m = cv(status, d1, d2);
  ZmidiParameterEvent pe;
  ZmidiMpeEvent e;
  e.kind = 99;
  CHECK(zmidi_parameters_receive(&mpe_parameters[status & 0x0F], &m, &pe) == ZMIDI_STATUS_OK);
  CHECK(zmidi_mpe_receive(r, &m, &pe, &e) == ZMIDI_STATUS_OK);
  return e;
}

static ZmidiMpeEvent mpe_mcm(ZmidiMpeReceiver *r, uint8_t channel, uint8_t members) {
  (void)mpe_feed(r, 0xB0 | channel, 0x65, 0);
  (void)mpe_feed(r, 0xB0 | channel, 0x64, 6);
  return mpe_feed(r, 0xB0 | channel, 0x06, members);
}

static void test_mpe(void) {
  /* M1-100 Appendix B.5 with the corrected data byte (AMB-M1-100-04), RPN 0 (2.2.5), Appendix C.4,
   * Table 4 with the corrected Pitch Bend (AMB-M1-100-08) and the note allocator (A.3). */
  static const uint8_t table4[11] = {0xE2, 0x55, 0x40, 0xB2, 0x4A, 0x40, 0xD2, 0x00, 0x92, 0x3C, 0x38};
  ZmidiMpeReceiver r;
  ZmidiMpeNoteAllocator a;
  ZmidiMpeEvent e;
  ZmidiMpeInfo info;
  ZmidiMpeChannel ch;
  ZmidiMpeLayout layout;
  ZmidiMidi1Message out[48];
  uint8_t bytes[16];
  size_t n, k, i;
  uint16_t value;
  uint8_t c;
  uint32_t role, zone, cls;
  bool found;
  for (c = 0; c < 16; c++) {
    CHECK(zmidi_parameters_init(&mpe_parameters[c]) == ZMIDI_STATUS_OK);
  }
  CHECK(zmidi_mpe_receiver_init(&r) == ZMIDI_STATUS_OK);
  e = mpe_mcm(&r, 0, 7);
  CHECK(e.kind == ZMIDI_MPE_EVENT_KIND_LAYOUT && e.reset == 0x00FF);
  e = mpe_mcm(&r, 15, 11);
  CHECK(e.kind == ZMIDI_MPE_EVENT_KIND_LAYOUT && e.reset == 0xFFF0);
  CHECK(zmidi_mpe_info(&r, &info) == ZMIDI_STATUS_OK && info.layout.lower == 3 && info.layout.upper == 11);
  CHECK(info.mode[ZMIDI_MPE_ZONE_UPPER] == 3 && info.manager[ZMIDI_MPE_ZONE_UPPER].semitones == 2 &&
        info.member[ZMIDI_MPE_ZONE_UPPER].semitones == 48);
  CHECK(zmidi_mpe_layout_role(info.layout.lower, info.layout.upper, 4, &role, &zone) == ZMIDI_STATUS_OK && role == ZMIDI_MPE_ROLE_MEMBER &&
        zone == ZMIDI_MPE_ZONE_UPPER);
  /* RPN 0 on one Member Channel sets the Member sensitivity of the Zone. */
  (void)mpe_feed(&r, 0xB5, 0x65, 0);
  (void)mpe_feed(&r, 0xB5, 0x64, 0);
  e = mpe_feed(&r, 0xB5, 0x06, 24);
  CHECK(e.kind == ZMIDI_MPE_EVENT_KIND_SENSITIVITY && e.zone == ZMIDI_MPE_ZONE_UPPER && !e.manager);
  CHECK(zmidi_mpe_info(&r, &info) == ZMIDI_STATUS_OK && info.member[ZMIDI_MPE_ZONE_UPPER].semitones == 24);
  /* Pitch Bend on a Member Channel: 0x41 << 7 | 0x2B at 24 semitones is +0.5 semitone. */
  e = mpe_feed(&r, 0xE6, 0x2B, 0x41);
  CHECK(e.kind == ZMIDI_MPE_EVENT_KIND_ZONE && e.zone == ZMIDI_MPE_ZONE_UPPER && !e.manager);
  CHECK(zmidi_mpe_channel(&r, 6, &ch) == ZMIDI_STATUS_OK && ch.in_zone && ch.pitch_bend == 8363 && ch.brightness == 64);
  CHECK(ch.pitch_offset >= 4950 && ch.pitch_offset <= 5050);
  e = mpe_feed(&r, 0xA6, 0x3C, 0x40);
  CHECK(e.kind == ZMIDI_MPE_EVENT_KIND_IGNORED);
  CHECK(zmidi_mpe_set_layout(&r, 15, 0, &value) == ZMIDI_STATUS_OK && value == 0xFFFF);
  /* C.4: +7 semitones at 48 gives 9387. */
  CHECK(zmidi_mpe_pitch_bend_value(70000, 48, 0, &value) == ZMIDI_STATUS_OK && value == 9387);
  /* Table 4 (corrected): a quarter tone up at 48 semitones. */
  CHECK(zmidi_mpe_pitch_bend_value(5000, 48, 0, &value) == ZMIDI_STATUS_OK);
  CHECK(zmidi_mpe_note_messages(2, 0x3C, 0x38, value, 0x40, 0, out, 4, &n) == ZMIDI_STATUS_OK && n == 4);
  for (i = 0, k = 0; i < n; i++) {
    bytes[k++] = out[i].status;
    bytes[k++] = out[i].data1;
    if ((out[i].status & 0xF0) != 0xD0) {
      bytes[k++] = out[i].data2;
    }
  }
  CHECK(k == 11 && memcmp(bytes, table4, 11) == 0);
  CHECK(zmidi_mpe_note_messages(2, 0x3C, 0, value, 0x40, 0, out, 4, &n) == ZMIDI_STATUS_INVALID_ARGUMENT);
  /* The MCM writer (Example B.1) and the RPN 0 writer of a Zone. */
  CHECK(zmidi_mpe_mcm_messages(ZMIDI_MPE_ZONE_LOWER, 15, out, 3, &n) == ZMIDI_STATUS_OK && n == 3 &&
        out[2].status == 0xB0 && out[2].data1 == 0x06 && out[2].data2 == 0x0F);
  CHECK(zmidi_mpe_layout_new(7, 7, &layout) == ZMIDI_STATUS_OK);
  CHECK(zmidi_mpe_zone_sensitivity_messages(layout.lower, layout.upper, ZMIDI_MPE_ZONE_UPPER, 2, 48, out, 48, &n) == ZMIDI_STATUS_OK &&
        n == 24 && out[2].status == 0xBF && out[5].status == 0xB8);
  CHECK(zmidi_mpe_zone_sensitivity_messages(layout.lower, layout.upper, ZMIDI_MPE_ZONE_UPPER, 2, 48, out, 23, &n) ==
        ZMIDI_STATUS_BUFFER_TOO_SMALL);
  CHECK(zmidi_mpe_layout_new(7, 8, &layout) == ZMIDI_STATUS_INVALID_ARGUMENT);
  /* Appendix E: All Sound Off on the Manager Channel is P (AMB-M1-100-12). */
  CHECK(zmidi_mpe_class(ZMIDI_MPE_MESSAGE_ALL_SOUND_OFF, true, false, false, &cls) == ZMIDI_STATUS_OK &&
        cls == ZMIDI_MPE_CLASS_PROHIBITED);
  /* A.3: an unoccupied channel first, then the oldest last Note Off. */
  CHECK(zmidi_mpe_note_allocator_init(&a) == ZMIDI_STATUS_OK);
  for (i = 0; i < 3; i++) {
    CHECK(zmidi_mpe_note_on(&a, 3, 0, ZMIDI_MPE_ZONE_LOWER, (uint8_t)(60 + i), &found, &c) == ZMIDI_STATUS_OK &&
          found && c == i + 1);
  }
  CHECK(zmidi_mpe_note_off(&a, 2) == ZMIDI_STATUS_OK && zmidi_mpe_note_off(&a, 1) == ZMIDI_STATUS_OK);
  CHECK(zmidi_mpe_note_on(&a, 3, 0, ZMIDI_MPE_ZONE_LOWER, 70, &found, &c) == ZMIDI_STATUS_OK && found && c == 2);
  CHECK(zmidi_mpe_note_on(&a, 3, 0, ZMIDI_MPE_ZONE_UPPER, 70, &found, &c) == ZMIDI_STATUS_OK && !found);
}

static void test_storage(void) {
  CHECK(zmidi_midi1_reader_size() <= sizeof(ZmidiMidi1Reader));
  CHECK(zmidi_midi1_writer_size() <= sizeof(ZmidiMidi1Writer));
  CHECK(zmidi_mode_state_size() <= sizeof(ZmidiModeState));
  CHECK(zmidi_controller_state_size() <= sizeof(ZmidiControllerState));
  CHECK(zmidi_parameter_state_size() <= sizeof(ZmidiParameterState));
  CHECK(zmidi_sync_receiver_size() <= sizeof(ZmidiSyncReceiver));
  CHECK(zmidi_active_sensing_monitor_size() <= sizeof(ZmidiActiveSensingMonitor));
  CHECK(zmidi_active_sensing_sender_size() <= sizeof(ZmidiActiveSensingSender));
  CHECK(zmidi_mpe_receiver_size() <= sizeof(ZmidiMpeReceiver));
  CHECK(zmidi_mpe_note_allocator_size() <= sizeof(ZmidiMpeNoteAllocator));
  CHECK(zmidi_abi_version() == (ZMIDI_ABI_MAJOR << 16 | ZMIDI_ABI_MINOR) && ZMIDI_ABI_MINOR >= 10);
}

int main(void) {
  test_reader_and_writer();
  test_mode();
  test_controllers();
  test_parameters();
  test_sync();
  test_active_sensing();
  test_mpe();
  test_storage();
  if (failures != 0) {
    printf("midi1: %d failures\n", failures);
    return 1;
  }
  printf("midi1: ok\n");
  return 0;
}
