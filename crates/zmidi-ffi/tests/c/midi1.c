/* REQ-FFI-MIDI1-008: the messages, channel-mode, controllers and parameters parts through the header,
 * with vectors from M1 v4.2.1, RP-015 and RP-018. The file needs only these parts (ZMIDI_MIDI1). */
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

static void test_storage(void) {
  CHECK(zmidi_midi1_reader_size() <= sizeof(ZmidiMidi1Reader));
  CHECK(zmidi_midi1_writer_size() <= sizeof(ZmidiMidi1Writer));
  CHECK(zmidi_mode_state_size() <= sizeof(ZmidiModeState));
  CHECK(zmidi_controller_state_size() <= sizeof(ZmidiControllerState));
  CHECK(zmidi_parameter_state_size() <= sizeof(ZmidiParameterState));
  CHECK(zmidi_abi_version() == (ZMIDI_ABI_MAJOR << 16 | ZMIDI_ABI_MINOR) && ZMIDI_ABI_MINOR >= 8);
}

int main(void) {
  test_reader_and_writer();
  test_mode();
  test_controllers();
  test_parameters();
  test_storage();
  if (failures != 0) {
    printf("midi1: %d failures\n", failures);
    return 1;
  }
  printf("midi1: ok\n");
  return 0;
}
