/* REQ-FFI-SX-010: the sysex parts through the header, with the vectors of the zmidi-sysex tests: M1
 * v4.2.1 (Identity Reply, MIDI Tuning), RP-024 (GM2 System On), CA-025 (Master Volume), RP-004 (the
 * Quarter Frame example of p. 4, the Full Message), RP-013 (Example 2A, LOCATE [TARGET]). The file
 * needs only these parts (ZMIDI_MIDI1). */
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

static uint8_t out[600];
static size_t len;

static void test_sysex_base(void) {
  /* M1 Table VII and VIIa: a manufacturer message, and a Universal message with Sub-ID#2. */
  static const uint8_t mfr[] = {0xF0, 0x00, 0x20, 0x6B, 0x01, 0x02, 0xF7};
  static const uint8_t uni[] = {0x7E, 0x10, 0x06, 0x01};
  static const uint8_t id3[3] = {0x00, 0x20, 0x6B};
  static const uint8_t data[2] = {0x01, 0x02};
  ZmidiSysex s;
  bool yes = false;
  CHECK(zmidi_sysex_parse(mfr, sizeof mfr, &s) == ZMIDI_STATUS_OK);
  CHECK(s.kind == ZMIDI_SYSEX_CLASS_MANUFACTURER && memcmp(s.manufacturer, id3, 3) == 0);
  CHECK(s.data_offset == 4 && s.data_length == 2);
  CHECK(zmidi_sysex_parse(uni, sizeof uni, &s) == ZMIDI_STATUS_OK);
  CHECK(s.kind == ZMIDI_SYSEX_CLASS_NON_REAL_TIME && s.device_id == 0x10 && s.sub_id1 == 0x06);
  CHECK(s.has_sub_id2 && s.sub_id2 == 0x01 && s.data_length == 0);
  CHECK(zmidi_sysex_parse(mfr, 1, &s) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_sysex_addressed(0x7F, 0x10, &yes) == ZMIDI_STATUS_OK && yes);
  CHECK(zmidi_sysex_addressed(0x11, 0x10, &yes) == ZMIDI_STATUS_OK && !yes);
  CHECK(zmidi_sysex_write_manufacturer(id3, data, 2, out, sizeof out, &len) == ZMIDI_STATUS_OK);
  CHECK(len == sizeof mfr && memcmp(out, mfr, len) == 0);
  /* A small buffer gives the needed length; a null buffer of capacity 0 too. */
  CHECK(zmidi_sysex_write_manufacturer(id3, data, 2, out, 3, &len) == ZMIDI_STATUS_BUFFER_TOO_SMALL && len == 7);
  CHECK(zmidi_sysex_write_manufacturer(id3, data, 2, NULL, 0, &len) == ZMIDI_STATUS_BUFFER_TOO_SMALL && len == 7);
  CHECK(zmidi_sysex_write_universal(false, 0x10, 0x06, true, 0x01, NULL, 0, out, sizeof out, &len) == ZMIDI_STATUS_OK);
  CHECK(len == 6 && out[0] == 0xF0 && out[1] == 0x7E && out[4] == 0x01 && out[5] == 0xF7);
  /* Sub-ID#2 must match the family (06H has one). */
  CHECK(zmidi_sysex_write_universal(false, 0x10, 0x06, false, 0, NULL, 0, out, sizeof out, &len) ==
        ZMIDI_STATUS_INVALID_ARGUMENT);
}

static void test_inquiry_gm_device_control(void) {
  static const uint8_t request[] = {0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7};
  static const uint8_t reply[] = {0xF0, 0x7E, 0x10, 0x06, 0x02, 0x00, 0x20, 0x6B, 0x02,
                                  0x02, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01, 0xF7};
  static const uint8_t gm2_on[] = {0xF0, 0x7E, 0x7F, 0x09, 0x03, 0xF7};
  static const uint8_t volume[] = {0xF0, 0x7F, 0x7F, 0x04, 0x01, 0x7F, 0x7F, 0xF7};
  ZmidiIdentity id = {{0x00, 0x20, 0x6B}, 0x0102, 0x0003, {0, 0, 0, 1}};
  ZmidiIdentity got;
  ZmidiDeviceControl dc;
  bool is_reply = true;
  uint8_t dev = 0, cmd = 0;
  int16_t fine = 0;
  int8_t coarse = 0;
  /* M1 General Information: the responder answers a request to all devices with its own ID. */
  CHECK(zmidi_inquiry_read(request, sizeof request, &is_reply, &dev, &got) == ZMIDI_STATUS_OK);
  CHECK(!is_reply && dev == 0x7F);
  CHECK(zmidi_inquiry_respond(request, sizeof request, 0x10, &id, out, sizeof out, &len) == ZMIDI_STATUS_OK);
  CHECK(len == sizeof reply && memcmp(out, reply, len) == 0);
  CHECK(zmidi_inquiry_read(reply, sizeof reply, &is_reply, &dev, &got) == ZMIDI_STATUS_OK);
  CHECK(is_reply && dev == 0x10 && got.family == 0x0102 && got.member == 3 && got.revision[3] == 1);
  CHECK(zmidi_inquiry_respond(reply, sizeof reply, 0x10, &id, out, sizeof out, &len) == ZMIDI_STATUS_NO_PENDING);
  CHECK(zmidi_inquiry_respond(gm2_on, sizeof gm2_on, 0x10, &id, out, sizeof out, &len) == ZMIDI_STATUS_NO_PENDING);
  /* RP-024 4.9.1: GM2 System On. */
  CHECK(zmidi_gm_system_read(gm2_on, sizeof gm2_on, &dev, &cmd) == ZMIDI_STATUS_OK && dev == 0x7F && cmd == 3);
  CHECK(zmidi_gm_system_write(0x7F, 3, out, sizeof out, &len) == ZMIDI_STATUS_OK);
  CHECK(len == sizeof gm2_on && memcmp(out, gm2_on, len) == 0);
  CHECK(zmidi_gm_system_write(0x7F, 4, out, sizeof out, &len) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_gm_system_read(volume, sizeof volume, &dev, &cmd) == ZMIDI_STATUS_NO_PENDING);
  /* M1 Device Control: Master Volume 3FFFH; CA-025: the tuning conversions. */
  CHECK(zmidi_device_control_read(volume, sizeof volume, &dc) == ZMIDI_STATUS_OK);
  CHECK(dc.kind == ZMIDI_DEVICE_CONTROL_KIND_MASTER_VOLUME && dc.device_id == 0x7F && dc.value == 0x3FFF);
  CHECK(zmidi_device_control_write(&dc, out, sizeof out, &len) == ZMIDI_STATUS_OK);
  CHECK(len == sizeof volume && memcmp(out, volume, len) == 0);
  CHECK(zmidi_fine_tuning_offset(0x2000, &fine) == ZMIDI_STATUS_OK && fine == 0);
  CHECK(zmidi_fine_tuning_offset(0, &fine) == ZMIDI_STATUS_OK && fine == -8192);
  CHECK(zmidi_coarse_tuning_semitones(0x40, &coarse) == ZMIDI_STATUS_OK && coarse == 0);
  CHECK(zmidi_coarse_tuning_semitones(0x80, &coarse) == ZMIDI_STATUS_INVALID_ARGUMENT);
}

static void test_tuning(void) {
  static ZmidiFrequencyWord words[128];
  static ZmidiTuningNote notes[128];
  static const uint8_t name[16] = {0x7A, 0x6D, 0x69, 0x64, 0x69, 0x20, 0x74, 0x75, 0x6E, 0x69, 0x6E, 0x67, 0x20, 0x20, 0x20, 0x20};
  ZmidiTuning t;
  ZmidiTuningNote change[2];
  ZmidiTuningOffsets o;
  uint8_t b[3];
  size_t i;
  /* M1 MIDI Tuning: the frequency word 3CH 00H 00H is 261.6256 Hz (middle C). */
  ZmidiFrequencyWord w = {0x3C, 0};
  CHECK(zmidi_frequency_word_to_bytes(&w, b) == ZMIDI_STATUS_OK && b[0] == 0x3C && b[1] == 0 && b[2] == 0);
  b[2] = 0x80;
  CHECK(zmidi_frequency_word_from_bytes(b, &w) == ZMIDI_STATUS_INVALID_ARGUMENT);
  for (i = 0; i < 128; i++) {
    words[i].semitone = (uint8_t)i;
    words[i].fraction = (uint16_t)(i * 64);
  }
  CHECK(zmidi_tuning_key_dump_write(0x7F, true, 1, 2, name, words, out, sizeof out, &len) == ZMIDI_STATUS_OK);
  CHECK(zmidi_tuning_read(out, len, &t, notes, 128) == ZMIDI_STATUS_OK);
  CHECK(t.kind == ZMIDI_TUNING_KIND_KEY_DUMP && t.has_bank && t.bank == 1 && t.program == 2 && t.count == 128);
  CHECK(t.checksum_ok && memcmp(t.name, name, 16) == 0);
  CHECK(notes[100].key == 100 && notes[100].word.semitone == 100 && notes[100].word.fraction == 6400);
  /* A small note array gives the count. */
  CHECK(zmidi_tuning_read(out, len, &t, NULL, 0) == ZMIDI_STATUS_BUFFER_TOO_SMALL && t.count == 128);
  change[0].key = 60;
  change[0].word = w;
  change[1].key = 61;
  change[1].word.semitone = 0x3D;
  change[1].word.fraction = 0x2000;
  CHECK(zmidi_tuning_note_change_write(true, 0x7F, false, 0, 5, change, 2, out, sizeof out, &len) == ZMIDI_STATUS_OK);
  CHECK(zmidi_tuning_read(out, len, &t, notes, 2) == ZMIDI_STATUS_OK);
  CHECK(t.kind == ZMIDI_TUNING_KIND_NOTE_CHANGE && t.real_time && t.count == 2 && notes[1].key == 61);
  memset(&o, 0, sizeof o);
  o.two_byte = false;
  o.offsets[0] = -64;
  o.offsets[11] = 63;
  CHECK(zmidi_tuning_scale_octave_write(false, 0x7F, 0xFFFF, &o, out, sizeof out, &len) == ZMIDI_STATUS_OK);
  CHECK(zmidi_tuning_read(out, len, &t, NULL, 0) == ZMIDI_STATUS_OK);
  CHECK(t.kind == ZMIDI_TUNING_KIND_SCALE_OCTAVE && t.channels == 0xFFFF && t.offsets.offsets[0] == -64);
  o.offsets[0] = -65;
  CHECK(zmidi_tuning_scale_octave_write(false, 0x7F, 0xFFFF, &o, out, sizeof out, &len) ==
        ZMIDI_STATUS_INVALID_ARGUMENT);
}

static void test_mtc(void) {
  /* RP-004 p. 4: 01:37:52:16 at 30 frames per second. */
  static const uint8_t pieces[8] = {0x00, 0x11, 0x24, 0x33, 0x45, 0x52, 0x61, 0x76};
  static const uint8_t full[] = {0xF0, 0x7F, 0x7F, 0x01, 0x01, 0x61, 37, 52, 16, 0xF7};
  ZmidiMtcTime t = {1, 37, 52, 16, 3}, u;
  ZmidiMtcGenerator g;
  ZmidiMtcReader r;
  ZmidiMtcEvent e;
  ZmidiMtcReaderInfo info;
  ZmidiMtcUserBits bits = {{1, 2, 3, 4, 5, 6, 7, 8}, 2}, got;
  uint8_t q[8], b = 0, dev = 0;
  uint32_t count = 0;
  bool valid = false;
  size_t i;
  CHECK(sizeof(ZmidiMtcGenerator) >= zmidi_mtc_generator_size());
  CHECK(sizeof(ZmidiMtcReader) >= zmidi_mtc_reader_size());
  CHECK(zmidi_mtc_quarter_frames(&t, q) == ZMIDI_STATUS_OK && memcmp(q, pieces, 8) == 0);
  /* RP-013 p. 10: 00:22:00:02 drop-frame is 00:21:58:22 non-drop; 00:22:00:00 is skipped. */
  u.hours = 0;
  u.minutes = 22;
  u.seconds = 0;
  u.frames = 0;
  u.rate = 2;
  CHECK(zmidi_mtc_time_is_valid(&u, &valid) == ZMIDI_STATUS_OK && !valid);
  CHECK(zmidi_mtc_next_valid(&u, &u) == ZMIDI_STATUS_OK && u.frames == 2);
  CHECK(zmidi_mtc_to_frame_count(&u, &count) == ZMIDI_STATUS_OK);
  CHECK(zmidi_mtc_from_frame_count(count, 3, &u) == ZMIDI_STATUS_OK);
  CHECK(u.minutes == 21 && u.seconds == 58 && u.frames == 22);
  CHECK(zmidi_mtc_add_frames(&u, -1, &u) == ZMIDI_STATUS_OK && u.frames == 21);
  /* The generator gives the pieces, the reader assembles them: current time +2 frames. */
  CHECK(zmidi_mtc_generator_init(&g, &t, false) == ZMIDI_STATUS_OK);
  CHECK(zmidi_mtc_reader_init(&r) == ZMIDI_STATUS_OK);
  for (i = 0; i < 8; i++) {
    CHECK(zmidi_mtc_generator_next(&g, &b) == ZMIDI_STATUS_OK && b == pieces[i]);
    CHECK(zmidi_mtc_reader_receive(&r, b, &e) == ZMIDI_STATUS_OK && e.piece == i);
  }
  CHECK(e.has_sequence && !e.reverse && !e.discontinuity);
  CHECK(e.raw.minutes == 37 && e.raw.frames == 16 && e.current.frames == 18);
  CHECK(zmidi_mtc_reader_info(&r, &info) == ZMIDI_STATUS_OK && info.locked && info.running && info.has_time);
  CHECK(zmidi_mtc_generator_set_running(&g, false) == ZMIDI_STATUS_OK);
  CHECK(zmidi_mtc_generator_next(&g, &b) == ZMIDI_STATUS_NO_PENDING);
  CHECK(zmidi_mtc_reader_receive(&r, 0x80, &e) == ZMIDI_STATUS_INVALID_ARGUMENT);
  /* RP-004 p. 5: the Full Message stops the reader with its time. */
  CHECK(zmidi_mtc_full_write(0x7F, &t, out, sizeof out, &len) == ZMIDI_STATUS_OK);
  CHECK(len == sizeof full && memcmp(out, full, len) == 0);
  CHECK(zmidi_mtc_full_read(full, sizeof full, &dev, &u) == ZMIDI_STATUS_OK && dev == 0x7F && u.seconds == 52);
  CHECK(zmidi_mtc_reader_full_message(&r, &u) == ZMIDI_STATUS_OK);
  CHECK(zmidi_mtc_reader_info(&r, &info) == ZMIDI_STATUS_OK && !info.running && info.time.frames == 16);
  /* RP-004 p. 6: User Bits. */
  CHECK(zmidi_mtc_user_bits_write(0x7F, &bits, out, sizeof out, &len) == ZMIDI_STATUS_OK && len == 15);
  CHECK(zmidi_mtc_user_bits_read(out, len, &dev, &got) == ZMIDI_STATUS_OK && got.groups[7] == 8 && got.flags == 2);
  bits.flags = 4;
  CHECK(zmidi_mtc_user_bits_write(0x7F, &bits, out, sizeof out, &len) == ZMIDI_STATUS_INVALID_ARGUMENT);
}

static void test_mmc(void) {
  /* RP-013 Example 2A (p. 76): 01:02:03:06 at 30 frames, status form: 61 02 03 26 00. */
  static const uint8_t tc[5] = {0x61, 0x02, 0x03, 0x26, 0x00};
  /* STOP, then LOCATE [TARGET] 01:02:03:04.05 (p. 28). */
  static const uint8_t cmds[] = {0xF0, 0x7F, 0x7F, 0x06, 0x01, 0x44, 0x06, 0x01, 0x61, 0x02, 0x03, 0x04, 0x05, 0xF7};
  ZmidiTimeCode c, d;
  ZmidiMmcMessage m;
  ZmidiMmcCommand cmd[2];
  ZmidiMmcField f;
  uint8_t b[5], s = 0;
  size_t at = 0;
  const uint8_t *str;
  CHECK(zmidi_time_code_from_bytes(tc, &c) == ZMIDI_STATUS_OK);
  CHECK(c.time.hours == 1 && c.time.rate == 3 && c.time.frames == 6 && c.status_form && !c.no_time_code);
  CHECK(zmidi_time_code_to_bytes(&c, b) == ZMIDI_STATUS_OK && memcmp(b, tc, 5) == 0);
  /* p. 32 MOVE: {st} to {ff} gives subframes 0; {ff} to {st} gives n = 1. */
  CHECK(zmidi_time_code_to_subframes_form(&c, &d) == ZMIDI_STATUS_OK && !d.status_form && d.subframes == 0);
  CHECK(zmidi_time_code_to_status_form(&d, true, &d) == ZMIDI_STATUS_OK && d.status_form && d.no_time_code);
  CHECK(d.video_field1 && !d.estimated);
  CHECK(zmidi_mmc_short_name(0x06, &s) == ZMIDI_STATUS_OK && s == 0x26);
  CHECK(zmidi_mmc_full_name(0x26, &s) == ZMIDI_STATUS_OK && s == 0x06);
  CHECK(zmidi_mmc_full_name(0x20, &s) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_mmc_read(cmds, sizeof cmds, &m) == ZMIDI_STATUS_OK && !m.response && m.device_id == 0x7F);
  CHECK(m.string_offset == 4 && m.string_length == 9);
  str = cmds + m.string_offset;
  CHECK(zmidi_mmc_next_command(str, m.string_length, &at, &cmd[0]) == ZMIDI_STATUS_OK);
  CHECK(cmd[0].kind == ZMIDI_MMC_COMMAND_KIND_STOP && at == 1);
  CHECK(zmidi_mmc_next_command(str, m.string_length, &at, &cmd[1]) == ZMIDI_STATUS_OK);
  CHECK(cmd[1].kind == ZMIDI_MMC_COMMAND_KIND_LOCATE_TARGET && cmd[1].target.time.seconds == 3);
  CHECK(cmd[1].target.subframes == 5 && !cmd[1].target.status_form);
  CHECK(zmidi_mmc_next_command(str, m.string_length, &at, &cmd[0]) == ZMIDI_STATUS_NO_PENDING);
  /* Written back from the two commands. */
  cmd[0].kind = ZMIDI_MMC_COMMAND_KIND_STOP;
  CHECK(zmidi_mmc_write_commands(0x7F, cmd, 2, NULL, 0, out, sizeof out, &len) == ZMIDI_STATUS_OK);
  CHECK(len == sizeof cmds && memcmp(out, cmds, len) == 0);
  /* A string cut short: the error, the offset at the end. */
  at = 0;
  CHECK(zmidi_mmc_next_command(str, 4, &at, &cmd[0]) == ZMIDI_STATUS_OK);
  CHECK(zmidi_mmc_next_command(str, 4, &at, &cmd[0]) == ZMIDI_STATUS_INVALID_ARGUMENT && at == 4);
  /* A response: SELECTED TIME CODE {st} from device 01H; 7FH is not a source (AMB-RP-013-04). */
  memset(&f, 0, sizeof f);
  f.kind = ZMIDI_MMC_FIELD_KIND_TIME_CODE;
  f.field = 0x01;
  f.code = c;
  CHECK(zmidi_mmc_write_responses(0x01, &f, 1, NULL, 0, out, sizeof out, &len) == ZMIDI_STATUS_OK && len == 11);
  CHECK(out[3] == 0x07 && out[4] == 0x01 && memcmp(out + 5, tc, 5) == 0);
  CHECK(zmidi_mmc_write_responses(0x7F, &f, 1, NULL, 0, out, sizeof out, &len) == ZMIDI_STATUS_INVALID_ARGUMENT);
  f.code.negative = true;
  CHECK(zmidi_mmc_write_fields(&f, 1, NULL, 0, out, sizeof out, &len) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_mmc_write_responses(0x01, &f, 1, NULL, 0, out, sizeof out, &len) == ZMIDI_STATUS_INVALID_ARGUMENT);
  f.code.negative = false;
  CHECK(zmidi_mmc_write_fields(&f, 1, NULL, 0, out, sizeof out, &len) == ZMIDI_STATUS_OK && len == 6);
  at = 0;
  CHECK(zmidi_mmc_next_field(out, len, &at, &f) == ZMIDI_STATUS_OK && f.kind == ZMIDI_MMC_FIELD_KIND_TIME_CODE);
  CHECK(f.field == 0x01 && f.data_offset == 1 && f.data_length == 5 && at == 6);
}

int main(void) {
  test_sysex_base();
  test_inquiry_gm_device_control();
  test_tuning();
  test_mtc();
  test_mmc();
  if (failures) {
    printf("%d failures\n", failures);
    return 1;
  }
  printf("ok\n");
  return 0;
}
