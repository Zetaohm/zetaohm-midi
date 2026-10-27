// midi2-dev/ni-midi2 (C++).
#include "bench.h"
#include "midi/midi1_byte_stream.h"
#include "midi/midi1_channel_voice_message.h"
#include "midi/midi2_channel_voice_message.h"
#include "midi/sysex_collector.h"
using namespace midi;

static uint32_t count;
static uint32_t parse_mixed() {
  const uint32_t *w = MIXED_WORDS; BB(w); uint32_t n = 0;
  for (uint32_t i = 0; i < 96;) {
    universal_packet p(w[i]);
    if (is_midi2_channel_voice_message(p)) {
      p.data[1] = w[i + 1];
      midi2_channel_voice_message_view v(p);
      SINK(v.channel() ^ v.byte3() ^ get_midi2_note_attribute(p) ^ get_midi2_note_attribute_data(p) ^ v.data()); n++;
    } else if (is_midi1_channel_voice_message(p)) {
      midi1_channel_voice_message_view v(p);
      SINK(v.channel() ^ v.data_byte_1() ^ v.data_byte_2()); n++;
    }
    i += p.size();
  }
  return n;
}
static uint32_t bytes_m1() {
  midi1_byte_stream_parser parser([](universal_packet p) { SINK(p.data[0]); count++; });
  const uint8_t *b = NOTE_BYTES; BB(b); count = 0;
  for (uint32_t i = 0; i < 192; i++) parser.feed(b[i]);
  return count;
}
// The collector grows a std::vector on the heap (newlib-nano malloc).
static uint32_t sysex_reassemble() {
  sysex7_collector c([](const sysex7 &s) { count = (uint32_t)(s.data.size()); });
  const uint32_t *w = SYSEX_WORDS; BB(w); count = 0;
  for (uint32_t i = 0; i < 340; i += 2) { universal_packet p(w[i], w[i + 1]); c.feed(p); }
  return count;
}
extern "C" void lib_run(void) {
  bench("ump_parse_mixed", 64, 64, parse_mixed);
  bench("io_bytes_to_ump_midi1_note", 64, 64, bytes_m1);
  bench("ump_sysex7_reassemble_1020", 170, 1017, sysex_reassemble);
}
