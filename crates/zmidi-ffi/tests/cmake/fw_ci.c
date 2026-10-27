/* A bare-metal link check of the ci part: a MIDI-CI Responder with one-chunk Property Exchange. */
#include "zmidi.h"

static ZmidiCi dev;
static uint8_t replies[ZMIDI_CI_REPLY_BUFFER];
static uint8_t scratch[256];
volatile uint8_t rx[512];
volatile size_t rx_len;
volatile uint32_t sink;

static const char list[] = "[{\"resource\":\"DeviceInfo\"}]";
static const char info[] = "{\"manufacturerId\":[0,33,127]}";

void Reset_Handler(void) {
  ZmidiCiConfig c = {{0x00, 0x21, 0x7F}, 1, 2, {0, 0, 1, 0}, 512, false, 0, 0, 1, 1, (const uint8_t *)0,
                     false, true, false, false, (const uint8_t *)list, sizeof list - 1};
  ZmidiCiEvent ev;
  size_t n;
  (void)zmidi_ci_init(&dev, &c, sink);
  for (;;) {
    if (zmidi_ci_receive(&dev, 0, (const uint8_t *)rx, rx_len, 0, replies, sizeof replies, &n, &ev) != ZMIDI_STATUS_OK) continue;
    if (ev.kind == ZMIDI_CI_EVENT_KIND_PE_GET) {
      (void)zmidi_ci_pe_reply(&dev, 200, (const uint8_t *)info, sizeof info - 1, scratch, sizeof scratch, replies, sizeof replies, &n);
    }
    sink = replies[0] + (uint32_t)n;
  }
}
