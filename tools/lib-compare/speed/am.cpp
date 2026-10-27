// midi2-dev/AM_MIDI2.0Lib (C++).
#include "bench.h"
#include "umpProcessor.h"
#include "bytestreamToUMP.h"
#include "umpToBytestream.h"
#include "umpToMIDI1Protocol.h"
#include "umpToMIDI2Protocol.h"
#include "mcoded7.h"

static uint32_t count;
static uint32_t parse_mixed() {
  umpProcessor up;
  up.setCVM([](umpCVM m) { SINK(m.umpGroup ^ m.status ^ m.channel ^ m.note ^ m.value ^ m.index ^ m.bank ^ m.flag1); count++; });
  up.setM1CVM([](umpM1CVM m) { SINK(m.umpGroup ^ m.status ^ m.channel ^ m.data1 ^ m.data2); count++; });
  const uint32_t *w = MIXED_WORDS; BB(w); count = 0;
  for (uint32_t i = 0; i < 96; i++) up.processUMP(w[i]);
  return count;
}
static uint32_t m1_to_m2() {
  umpToMIDI2Protocol t; const uint32_t *w = CV1_WORDS; BB(w); uint32_t n = 0;
  for (uint32_t i = 0; i < 64; i++) { t.UMPStreamParse(w[i]); while (t.availableUMP()) { SINK(t.readUMP()); n++; } }
  return n / 2;
}
static uint32_t m2_to_m1() {
  umpToMIDI1Protocol t; const uint32_t *w = CV2_WORDS; BB(w); uint32_t n = 0;
  for (uint32_t i = 0; i < 128; i++) { t.UMPStreamParse(w[i]); while (t.availableUMP()) { SINK(t.readUMP()); n++; } }
  return n;
}
static uint32_t bytes_m1() {
  bytestreamToUMP bs; const uint8_t *b = NOTE_BYTES; BB(b); uint32_t n = 0;
  for (uint32_t i = 0; i < 192; i++) { bs.bytestreamParse(b[i]); while (bs.availableUMP()) { SINK(bs.readUMP()); n++; } }
  return n;
}
static uint32_t ump_to_bytes() {
  umpToBytestream t; t.enableRunningStatus = true; const uint32_t *w = CV1_WORDS; BB(w); uint32_t n = 0;
  for (uint32_t i = 0; i < 64; i++) { t.UMPStreamParse(w[i]); while (t.availableBS()) { SINK(t.readBS()); n++; } }
  return n;
}
static uint8_t enc[1200], dec[1100];
static uint32_t mc7_encode() {
  mcoded7Encode e; e.reset(); const uint8_t *b = SYSEX_PAYLOAD; BB(b); uint32_t o = 0;
  for (uint32_t i = 0; i < 1020; i++) {
    e.parseByte(b[i]);
    if (e.currentPos() == 7) { for (int k = 0; k < 8; k++) enc[o++] = e.dump[k]; e.reset(); }
  }
  if (e.currentPos()) { for (int k = 0; k <= e.currentPos(); k++) enc[o++] = e.dump[k]; }
  SINK(enc[0]);
  return o;
}
static uint32_t mc7_decode() {
  mcoded7Decode d; d.reset(); const uint8_t *b = MCODED7; BB(b); uint32_t o = 0;
  for (uint32_t i = 0; i < 1166; i++) {
    d.parseS7Byte(b[i]);
    if (d.currentPos() == 7) { for (int k = 0; k < 7; k++) dec[o++] = d.dump[k]; d.reset(); }
  }
  if (d.currentPos() != 255) { for (int k = 0; k < d.currentPos(); k++) dec[o++] = d.dump[k]; }
  SINK(dec[0]);
  return o;
}
extern "C" void lib_run(void) {
  bench("ump_parse_mixed", 64, 64, parse_mixed);
  bench("ump_translate_midi1_to_midi2_note", 64, 64, m1_to_m2);
  bench("ump_translate_midi2_to_midi1_words", 64, 64, m2_to_m1);
  bench("io_bytes_to_ump_midi1_note", 64, 64, bytes_m1);
  bench("io_ump_to_bytes_note_running", 64, 129, ump_to_bytes);
  bench("pe_mcoded7_encode_1020", 1020, 1166, mc7_encode);
  bench("pe_mcoded7_decode_1020", 1020, 1020, mc7_decode);
}
