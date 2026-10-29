/* C test of the ump part of the C ABI (REQ-FFI-UMP-006). Vectors: M2-104 Table 4 (word counts),
 * 7.4 Figures (MIDI 2.0 Channel Voice), Appendix D.2 (MIDI 2.0 to 1.0), M2-115 Tables 5 and 8
 * (scaling, also M2-104 D.1.4). */
#ifndef ZMIDI_UMP
#define ZMIDI_UMP
#endif
#include "zmidi.h"

#include <stdio.h>
#include <stdlib.h>

static int failures;
#define CHECK(c) do { if (!(c)) { fprintf(stderr, "%s:%d: CHECK(%s)\n", __FILE__, __LINE__, #c); failures++; } } while (0)

static void word_counts(void) {
  /* Table 4: Message Types 0x0-0x2, 0x6, 0x7: 1 word; 0x3, 0x4, 0x8-0xA: 2; 0xB, 0xC: 3; 0x5, 0xD-0xF: 4. */
  static const size_t want[16] = {1, 1, 1, 2, 2, 4, 1, 1, 2, 2, 2, 3, 3, 4, 4, 4};
  uint32_t mt;
  for (mt = 0; mt < 16; mt++) CHECK(zmidi_ump_word_count(mt << 28) == want[mt]);
}

static void writers(void) {
  uint32_t w[2];
  /* 7.4.2 Note On: Group 0, Channel 0, note 60, velocity 0xC000, no attribute. */
  CHECK(zmidi_ump_midi2_note_on(0, 0, 60, 0xC000, w) == ZMIDI_STATUS_OK);
  CHECK(w[0] == 0x40903C00u && w[1] == 0xC0000000u);
  CHECK(zmidi_ump_midi2_note_off(0, 0, 60, 0x1234, w) == ZMIDI_STATUS_OK);
  CHECK(w[0] == 0x40803C00u && w[1] == 0x12340000u);
  /* 7.4.6 Control Change: Group 1, index 7, data 0xFFFFFFFF. */
  CHECK(zmidi_ump_midi2_control_change(1, 0, 7, 0xFFFFFFFFu, w) == ZMIDI_STATUS_OK);
  CHECK(w[0] == 0x41B00700u && w[1] == 0xFFFFFFFFu);
  /* 7.4.9 Program Change (Figure 64): program 5, Bank 1/2. */
  CHECK(zmidi_ump_midi2_program_change(0, 0, 5, true, 1, 2, w) == ZMIDI_STATUS_OK);
  CHECK(w[0] == 0x40C00001u && w[1] == 0x05000102u);
  /* With no Bank: Bank Valid 0 and the bank bytes 0. */
  CHECK(zmidi_ump_midi2_program_change(0, 0, 5, false, 1, 2, w) == ZMIDI_STATUS_OK);
  CHECK(w[0] == 0x40C00000u && w[1] == 0x05000000u);
  CHECK(zmidi_ump_midi2_channel_pressure(0, 3, 0x89ABCDEFu, w) == ZMIDI_STATUS_OK);
  CHECK(w[0] == 0x40D30000u && w[1] == 0x89ABCDEFu);
  /* 7.4.11 Pitch Bend: the center is 0x80000000. */
  CHECK(zmidi_ump_midi2_pitch_bend(2, 15, 0x80000000u, w) == ZMIDI_STATUS_OK);
  CHECK(w[0] == 0x42EF0000u && w[1] == 0x80000000u);
  /* Out of range and null. */
  CHECK(zmidi_ump_midi2_note_on(16, 0, 60, 1, w) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_ump_midi2_note_on(0, 16, 60, 1, w) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_ump_midi2_note_on(0, 0, 128, 1, w) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_ump_midi2_control_change(0, 0, 128, 0, w) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_ump_midi2_program_change(0, 0, 128, false, 0, 0, w) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_ump_midi2_pitch_bend(0, 0, 0, NULL) == ZMIDI_STATUS_INVALID_ARGUMENT);
}

static void parse(void) {
  ZmidiUmpInfo info;
  static const uint32_t note_on[2] = {0x43923C00u, 0xC0000000u};
  static const uint32_t clock[1] = {0x13F80000u};
  CHECK(zmidi_ump_parse(note_on, 2, &info) == ZMIDI_STATUS_OK);
  CHECK(info.message_type == 0x4 && info.word_count == 2 && info.decoded);
  CHECK(info.has_group && info.group == 3 && info.has_status && info.status == 0x9);
  CHECK(info.has_channel && info.channel == 2);
  CHECK(info.words[0] == note_on[0] && info.words[1] == note_on[1] && info.words[2] == 0);
  CHECK(zmidi_ump_parse(clock, 1, &info) == ZMIDI_STATUS_OK);
  CHECK(info.message_type == 0x1 && info.status == 0xF8 && info.group == 3 && !info.has_channel);
  CHECK(info.has_group && info.has_status && info.decoded);
  /* Utility NOOP: no Group (M2-104 2.1.2), Status 0. */
  {
    static const uint32_t noop[1] = {0x00000000u};
    static const uint32_t reserved[1] = {0x6A000000u};
    CHECK(zmidi_ump_parse(noop, 1, &info) == ZMIDI_STATUS_OK);
    CHECK(info.message_type == 0x0 && !info.has_group && info.has_status && info.status == 0);
    CHECK(info.decoded && !info.has_channel);
    /* A reserved Message Type: not decoded, no status. */
    CHECK(zmidi_ump_parse(reserved, 1, &info) == ZMIDI_STATUS_OK);
    CHECK(info.message_type == 0x6 && info.word_count == 1 && !info.decoded);
    CHECK(!info.has_status && !info.has_channel);
    /* M2-104 2.1.2: a reserved Message Type may be defined with or without a Group. */
    CHECK(!info.has_group);
  }
  /* Too few words for Message Type 0x4, and null pointers. */
  CHECK(zmidi_ump_parse(note_on, 1, &info) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_ump_parse(NULL, 1, &info) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_ump_parse(note_on, 2, NULL) == ZMIDI_STATUS_INVALID_ARGUMENT);
}

static void translate(void) {
  uint32_t w[2], out[4];
  size_t n = 99;
  /* D.2: Note On velocity 0xC000 gives MIDI 1.0 velocity 0x60. */
  CHECK(zmidi_ump_midi2_note_on(0, 0, 60, 0xC000, w) == ZMIDI_STATUS_OK);
  CHECK(zmidi_ump_midi2_to_midi1(w, 2, out, 4, &n) == ZMIDI_STATUS_OK);
  CHECK(n == 1 && out[0] == 0x20903C60u);
  /* D.2: a velocity that downscales to 0 gives velocity 1 (a Note On with velocity 0 is a Note Off). */
  CHECK(zmidi_ump_midi2_note_on(0, 0, 60, 0x0100, w) == ZMIDI_STATUS_OK);
  CHECK(zmidi_ump_midi2_to_midi1(w, 2, out, 4, &n) == ZMIDI_STATUS_OK);
  CHECK(n == 1 && out[0] == 0x20903C01u);
  /* D.2: Control Change data 0xFFFFFFFF gives value 0x7F. */
  CHECK(zmidi_ump_midi2_control_change(0, 0, 7, 0xFFFFFFFFu, w) == ZMIDI_STATUS_OK);
  CHECK(zmidi_ump_midi2_to_midi1(w, 2, out, 4, &n) == ZMIDI_STATUS_OK);
  CHECK(n == 1 && out[0] == 0x20B0077Fu);
  /* D.2: Program Change with Bank gives Bank Select MSB, LSB, then Program Change. */
  CHECK(zmidi_ump_midi2_program_change(0, 0, 5, true, 1, 2, w) == ZMIDI_STATUS_OK);
  CHECK(zmidi_ump_midi2_to_midi1(w, 2, out, 4, &n) == ZMIDI_STATUS_OK);
  CHECK(n == 3 && out[0] == 0x20B00001u && out[1] == 0x20B02002u && out[2] == 0x20C00500u);
  /* D.2: Pitch Bend center 0x80000000 gives 0x2000 (LSB 0x00, MSB 0x40). */
  CHECK(zmidi_ump_midi2_pitch_bend(0, 0, 0x80000000u, w) == ZMIDI_STATUS_OK);
  CHECK(zmidi_ump_midi2_to_midi1(w, 2, out, 4, &n) == ZMIDI_STATUS_OK);
  CHECK(n == 1 && out[0] == 0x20E00040u);
  /* D.2: Registered Controller (RPN) 0/1 with data 0x80000000 gives 4 Control Changes. */
  {
    static const uint32_t rpn[2] = {0x40200001u, 0x80000000u};
    CHECK(zmidi_ump_midi2_to_midi1(rpn, 2, out, 4, &n) == ZMIDI_STATUS_OK);
    CHECK(n == 4 && out[0] == 0x20B06500u && out[1] == 0x20B06401u);
    CHECK(out[2] == 0x20B00640u && out[3] == 0x20B02600u);
  }
  /* REQ-FFI-UMP-004: Per-Note Management has no MIDI 1.0 form: zero words. */
  {
    static const uint32_t pnm[2] = {0x40F03C00u, 0};
    n = 99;
    CHECK(zmidi_ump_midi2_to_midi1(pnm, 2, out, 4, &n) == ZMIDI_STATUS_OK);
    CHECK(n == 0);
  }
  /* Not a MIDI 2.0 Channel Voice packet. */
  out[0] = 0x20903C64u;
  CHECK(zmidi_ump_midi2_to_midi1(out, 1, w, 2, &n) == ZMIDI_STATUS_NOT_SUPPORTED);
  /* A buffer that is too small. */
  CHECK(zmidi_ump_midi2_note_on(0, 0, 60, 0xC000, w) == ZMIDI_STATUS_OK);
  CHECK(zmidi_ump_midi2_to_midi1(w, 2, out, 0, &n) == ZMIDI_STATUS_BUFFER_TOO_SMALL);
}

static void scaling(void) {
  /* M2-115 Table 5: Min-Center-Max upscale 7 to 16 bits. */
  static const uint32_t up[][2] = {{0x00, 0x0000}, {0x05, 0x0A00}, {0x1E, 0x3C00}, {0x20, 0x4000}, {0x40, 0x8000},
                                   {0x46, 0x8C30}, {0x60, 0xC104}, {0x78, 0xF1C7}, {0x7F, 0xFFFF}};
  /* M2-115 Table 8 (M2-104 D.1.4): downscale 16 to 7 bits. */
  static const uint32_t down[][2] = {{0x1400, 0x0A}, {0x8000, 0x40}, {0xAEBA, 0x57}, {0xFFFF, 0x7F}};
  size_t i;
  uint32_t v;
  for (i = 0; i < sizeof up / sizeof up[0]; i++) {
    CHECK(zmidi_ump_upscale(up[i][0], 7, 16, &v) == ZMIDI_STATUS_OK && v == up[i][1]);
  }
  for (i = 0; i < sizeof down / sizeof down[0]; i++) {
    CHECK(zmidi_ump_downscale(down[i][0], 16, 7, &v) == ZMIDI_STATUS_OK && v == down[i][1]);
  }
  CHECK(zmidi_ump_upscale(0, 7, 7, &v) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_ump_upscale(0x80, 7, 16, &v) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_ump_upscale(0, 7, 33, &v) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_ump_upscale(0, 7, 16, NULL) == ZMIDI_STATUS_INVALID_ARGUMENT);
  /* Downscale: the target width must be smaller, and the value must fit the source width. */
  CHECK(zmidi_ump_downscale(0, 7, 7, &v) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_ump_downscale(0, 7, 16, &v) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_ump_downscale(0x10000, 16, 7, &v) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_ump_downscale(0, 16, 7, NULL) == ZMIDI_STATUS_INVALID_ARGUMENT);
}

int main(void) {
  word_counts();
  writers();
  parse();
  translate();
  scaling();
  if (failures) { fprintf(stderr, "%d failure(s)\n", failures); return EXIT_FAILURE; }
  printf("ump ok\n");
  return EXIT_SUCCESS;
}
