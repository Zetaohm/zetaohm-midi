/* A bare-metal link check of the fixed USB MIDI function (OPT-038): the descriptors and the Group
 * Terminal Block set of USB-MIDI v2.0 Appendix B as generated arrays (tests/c/usb_fixed_appendix_b.h,
 * from zmidi_usb_gen.h), and the request handler zmidi_usb_fixed_gtb_setup. */
#ifndef ZMIDI_USB
#define ZMIDI_USB
#endif
#include "zmidi.h"

#include "usb_fixed_appendix_b.h"

volatile uint8_t rx[8];
volatile uint32_t sink;
static uint8_t answer[64];

void Reset_Handler(void) {
  ZmidiUsbGtbResponse r;
  size_t n = 0;
  /* A USB stack sends the descriptors from the array. */
  sink = appendix_b_descriptors[rx[0] % sizeof appendix_b_descriptors] + appendix_b_endpoints[0].address;
  for (;;) {
    uint8_t s[8];
    int i;
    for (i = 0; i < 8; i++) {
      s[i] = rx[i];
    }
    if (zmidi_usb_fixed_gtb_setup(appendix_b_gtb_sets, appendix_b_gtb_lens, appendix_b_GTB_COUNT, 1, s,
                                  answer, sizeof answer, &r, &n) == ZMIDI_STATUS_OK) {
      sink = (uint32_t)r + (uint32_t)n + answer[0];
    }
  }
}
