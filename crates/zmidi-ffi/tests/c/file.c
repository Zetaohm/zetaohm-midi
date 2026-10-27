/* C test of the file part of the C ABI (REQ-FFI-FILE-004). Vectors: M2-116 MIDI Clip File (the
 * minimal clip of the zmidi-file tests: File Header "SMF2CLIP", Delta Clockstamp 0 + DCTPQ, Start of
 * Clip, End of Clip). Words are stored most significant byte first. */
#ifndef ZMIDI_FILE
#define ZMIDI_FILE
#endif
#include "zmidi.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int failures;
#define CHECK(c) do { if (!(c)) { fprintf(stderr, "%s:%d: CHECK(%s)\n", __FILE__, __LINE__, #c); failures++; } } while (0)

static uint32_t word_at(const uint8_t *b) {
  return (uint32_t)b[0] << 24 | (uint32_t)b[1] << 16 | (uint32_t)b[2] << 8 | b[3];
}

static ZmidiClipConfig config(uint16_t tpq) {
  ZmidiClipConfig c;
  memset(&c, 0, sizeof c);
  c.ticks_per_quarter_note = tpq;
  return c;
}

static void minimal_clip(void) {
  static const uint32_t want[] = {0x00400000u, 0x00300060u, 0x00400000u, 0xF0200000u, 0, 0, 0,
                                  0x00400000u, 0xF0210000u, 0, 0, 0};
  uint8_t buf[128];
  ZmidiClipWriter w;
  ZmidiClipConfig c = config(96);
  ZmidiClipError e = ZMIDI_CLIP_ERROR_WRONG_PHASE;
  size_t len = 0, i;
  CHECK(zmidi_clip_writer_init(&w, buf, sizeof buf, &c, &e) == ZMIDI_STATUS_OK && e == ZMIDI_CLIP_ERROR_NONE);
  CHECK(zmidi_clip_writer_start(&w, 0, false, 0, 0, &e) == ZMIDI_STATUS_OK);
  CHECK(zmidi_clip_writer_finish(&w, 0, &len, &e) == ZMIDI_STATUS_OK);
  CHECK(len == 8 + 4 * (sizeof want / sizeof want[0]));
  CHECK(memcmp(buf, "SMF2CLIP", 8) == 0);
  for (i = 0; i < sizeof want / sizeof want[0]; i++) CHECK(word_at(buf + 8 + 4 * i) == want[i]);
  /* A second finish is in the wrong phase. */
  CHECK(zmidi_clip_writer_finish(&w, 0, &len, &e) == ZMIDI_STATUS_REJECTED && e == ZMIDI_CLIP_ERROR_WRONG_PHASE);
}

static void write_and_read_back(void) {
  static const uint32_t note_on[2] = {0x40903C00u, 0xFFFF0000u};
  static const uint32_t note_off[2] = {0x40803C00u, 0x00000000u};
  uint8_t buf[512];
  ZmidiClipWriter w;
  ZmidiClipReader r;
  ZmidiClipProfileOn p;
  ZmidiClipConfig c = config(96);
  ZmidiClipError e;
  ZmidiClipEvent ev;
  bool has = false;
  uint16_t tpq = 0;
  size_t len = 0, events = 0, seq_notes = 0;
  memset(&p, 0, sizeof p);
  p.profile_id[0] = 0x7E;
  p.profile_id[2] = 0x01;
  p.destination = 1;
  c.has_tempo = true;
  c.tempo_10ns = 50000000u;
  c.has_time_signature = true;
  c.numerator = 4;
  c.denominator = 2;
  c.thirty_second_notes = 8;
  c.profiles = &p;
  c.profile_count = 1;
  CHECK(zmidi_clip_writer_init(&w, buf, sizeof buf, &c, &e) == ZMIDI_STATUS_OK);
  CHECK(zmidi_clip_writer_start(&w, 0, false, 0, 0, &e) == ZMIDI_STATUS_OK);
  CHECK(zmidi_clip_writer_event(&w, 0, note_on, 2, &e) == ZMIDI_STATUS_OK);
  CHECK(zmidi_clip_writer_event(&w, 96, note_off, 2, &e) == ZMIDI_STATUS_OK);
  /* Ticks never decrease (events are in presentation order). */
  CHECK(zmidi_clip_writer_event(&w, 10, note_on, 2, &e) == ZMIDI_STATUS_REJECTED && e == ZMIDI_CLIP_ERROR_OUT_OF_ORDER);
  /* Not one whole UMP. */
  CHECK(zmidi_clip_writer_event(&w, 96, note_on, 1, &e) == ZMIDI_STATUS_INVALID_ARGUMENT && e == ZMIDI_CLIP_ERROR_INVALID_MESSAGE);
  CHECK(zmidi_clip_writer_finish(&w, 192, &len, &e) == ZMIDI_STATUS_OK);
  CHECK(len == zmidi_clip_writer_len(&w));

  CHECK(zmidi_clip_reader_init(&r, buf, len, &e) == ZMIDI_STATUS_OK);
  for (;;) {
    CHECK(zmidi_clip_reader_next(&r, &ev, &has, &e) == ZMIDI_STATUS_OK);
    if (!has) break;
    events++;
    if (ev.section == 1 && (ev.words[0] == note_on[0] || ev.words[0] == note_off[0])) {
      CHECK(ev.word_count == 2);
      CHECK(ev.ticks == (ev.words[0] == note_on[0] ? 0u : 96u));
      seq_notes++;
    }
    if (events > 64) break;
  }
  CHECK(seq_notes == 2);
  CHECK(zmidi_clip_reader_ticks_per_quarter_note(&r, &tpq) && tpq == 96);
}

static void bad_input(void) {
  static const uint8_t not_a_clip[16] = {0x53, 0x4D, 0x46, 0x31, 0x43, 0x4C, 0x49, 0x50}; /* "SMF1CLIP" */
  uint8_t buf[16];
  ZmidiClipReader r;
  ZmidiClipWriter w;
  ZmidiClipConfig c = config(0);
  ZmidiClipError e = ZMIDI_CLIP_ERROR_NONE;
  CHECK(zmidi_clip_reader_init(&r, not_a_clip, sizeof not_a_clip, &e) == ZMIDI_STATUS_REJECTED && e == ZMIDI_CLIP_ERROR_BAD_HEADER);
  /* A TPQ of 0 is not a valid configuration. */
  CHECK(zmidi_clip_writer_init(&w, buf, sizeof buf, &c, &e) == ZMIDI_STATUS_INVALID_ARGUMENT && e == ZMIDI_CLIP_ERROR_INVALID_CONFIG);
  /* A buffer too small for the header. */
  c = config(96);
  CHECK(zmidi_clip_writer_init(&w, buf, 4, &c, &e) == ZMIDI_STATUS_BUFFER_TOO_SMALL && e == ZMIDI_CLIP_ERROR_BUFFER_TOO_SMALL);
  CHECK(zmidi_clip_reader_init(NULL, buf, sizeof buf, NULL) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_clip_reader_size() <= sizeof(ZmidiClipReader));
  CHECK(zmidi_clip_writer_size() <= sizeof(ZmidiClipWriter));
}

int main(void) {
  minimal_clip();
  write_and_read_back();
  bad_input();
  if (failures) { fprintf(stderr, "%d failure(s)\n", failures); return EXIT_FAILURE; }
  printf("file ok\n");
  return EXIT_SUCCESS;
}
