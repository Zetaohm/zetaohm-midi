/* sauloverissimo/midi2 (C99). */
#include "bench.h"
#include "midi2_dispatch.h"
#include "midi2_conv.h"
#include "midi2_msg.h"

static uint32_t count;
static void non(uint8_t g, uint8_t c, uint8_t n, uint16_t v, uint8_t at, uint16_t ad, void *x) { SINK(g ^ c ^ n ^ v ^ at ^ ad); count++; }
static void cc(uint8_t g, uint8_t c, uint8_t i, uint8_t v, void *x) { SINK(g ^ c ^ i ^ v); count++; }

static uint32_t parse_mixed(void) {
  midi2_dispatch dp; midi2_dispatch_init(&dp); dp.on_note_on = non; dp.on_cv1_cc = cc;
  const uint32_t *w = MIXED_WORDS; BB(w); count = 0;
  for (uint32_t i = 0; i < 96;) { uint8_t n = (w[i] >> 28) == 4 ? 2 : 1; midi2_dispatch_feed(&w[i], n, &dp); i += n; }
  return count;
}
static uint32_t m1_to_m2(void) {
  const uint32_t *w = CV1_WORDS; BB(w); uint32_t n = 0, out[2];
  for (uint32_t i = 0; i < 64; i++) if (midi2_msg_mt2_to_mt4(w[i], out)) { SINK(out[0] ^ out[1]); n++; }
  return n;
}
static uint32_t m2_to_m1(void) {
  const uint32_t *w = CV2_WORDS; BB(w); uint32_t n = 0, out;
  for (uint32_t i = 0; i < 64; i++) { uint32_t k = midi2_msg_mt4_to_mt2(&w[2 * i], &out); SINK(out); n += k; }
  return n;
}
static uint32_t bytes_m1(void) {
  midi2_conv_state st; midi2_conv_init(&st, 0);
  const uint8_t *b = NOTE_BYTES; BB(b); uint32_t n = 0;
  for (uint32_t i = 0; i < 192; i++)
    if (midi2_conv_feed(&st, b[i])) do { SINK(st.ump[0]); n++; } while (midi2_conv_next(&st));
  return n;
}
void lib_run(void) {
  bench("ump_parse_mixed", 64, 64, parse_mixed);
  bench("ump_translate_midi1_to_midi2_note", 64, 64, m1_to_m2);
  bench("ump_translate_midi2_to_midi1_words", 64, 64, m2_to_m1);
  bench("io_bytes_to_ump_midi1_note", 64, 64, bytes_m1);
}
