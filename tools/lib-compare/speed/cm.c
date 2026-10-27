/* atsushieno/cmidi2 (C, one header). */
#include "bench.h"
#include "cmidi2.h"

static uint32_t parse_mixed(void) {
  cmidi2_ump *w = MIXED_WORDS; BB(w); uint32_t n = 0;
  for (uint32_t i = 0; i < 96;) {
    cmidi2_ump *u = &w[i];
    switch (cmidi2_ump_get_message_type(u)) {
      case 4: SINK(cmidi2_ump_get_channel(u) ^ cmidi2_ump_get_midi2_note_note(u) ^ cmidi2_ump_get_midi2_note_velocity(u)
                   ^ cmidi2_ump_get_midi2_note_attribute_type(u) ^ cmidi2_ump_get_midi2_note_attribute_data(u)); n++; break;
      case 2: SINK(cmidi2_ump_get_channel(u) ^ cmidi2_ump_get_midi1_cc_index(u) ^ cmidi2_ump_get_midi1_cc_data(u)); n++; break;
      default: break;
    }
    i += cmidi2_ump_get_num_bytes(u[0]) / 4;
  }
  return n;
}
/* cmidi2_convert_midi1_to_ump keeps its indexes through a uint8_t view of a size_t, so one call takes
   at most 255 bytes in and out: 4 calls of 16 notes. The context is set up once, as a stream would. */
static uint32_t to_ump(enum cmidi2_ci_protocol_values protocol, uint32_t per) {
  static cmidi2_ump out[32];
  cmidi2_midi_conversion_context c; cmidi2_midi_conversion_context_initialize(&c);
  uint8_t *b = NOTE_BYTES; BB(b); uint32_t n = 0;
  c.midi_protocol = protocol; c.ump = out; c.ump_num_bytes = sizeof out;
  for (uint32_t k = 0; k < 4; k++) {
    c.midi1 = b + 48 * k; c.midi1_num_bytes = 48; c.midi1_proceeded_bytes = 0; c.ump_proceeded_bytes = 0;
    if (cmidi2_convert_midi1_to_ump(&c) != CMIDI2_CONVERSION_RESULT_OK) return 0;
    SINK(out[0]);
    n += (uint32_t)(c.ump_proceeded_bytes / per);
  }
  return n;
}
static uint32_t bytes_m1(void) { return to_ump(CMIDI2_PROTOCOL_TYPE_MIDI1, 4); }
static uint32_t bytes_m2(void) { return to_ump(CMIDI2_PROTOCOL_TYPE_MIDI2, 8); }
/* No running status in cmidi2: 3 bytes for each message. */
static uint32_t ump_to_bytes(void) {
  cmidi2_ump *w = CV1_WORDS; BB(w); uint8_t d[4]; uint32_t n = 0;
  for (uint32_t i = 0; i < 64; i++) { n += (uint32_t)cmidi2_convert_single_ump_to_midi1(d, sizeof d, &w[i]); SINK(d[0] ^ d[1] ^ d[2]); }
  return n;
}
void lib_run(void) {
  bench("ump_parse_mixed", 64, 64, parse_mixed);
  bench("io_bytes_to_ump_midi1_note", 64, 64, bytes_m1);
  bench("io_bytes_to_ump_midi2_note", 64, 64, bytes_m2);
  bench("io_ump_to_bytes_note", 64, 192, ump_to_bytes);
}
