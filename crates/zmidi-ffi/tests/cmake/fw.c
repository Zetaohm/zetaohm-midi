/* A bare-metal link check of the io part: a MIDI 1.0 byte stream to UMP and back. */
#include "zmidi.h"

static ZmidiBytesToUmp reader;
static ZmidiUmpToBytes writer;
volatile uint8_t rx;
volatile uint32_t sink;

void Reset_Handler(void) {
  uint32_t w[ZMIDI_BYTES_TO_UMP_MAX_WORDS];
  uint8_t b[16];
  size_t n, m;
  (void)zmidi_bytes_to_ump_init(&reader, 0, 2);
  (void)zmidi_ump_to_bytes_init(&writer, true);
  for (;;) {
    if (zmidi_bytes_to_ump_push(&reader, rx, w, ZMIDI_BYTES_TO_UMP_MAX_WORDS, &n) == ZMIDI_STATUS_OK && n > 0 &&
        zmidi_ump_to_bytes_write(&writer, w, n, b, sizeof b, &m) == ZMIDI_STATUS_OK) {
      sink = b[0] + (uint32_t)m;
    }
  }
}
