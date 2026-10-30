/* C test of the USB descriptor writer and the Group Terminal Block requests of the C ABI
 * (REQ-FFI-USB-006 to REQ-FFI-USB-010). Vectors: USB-MIDI v2.0 Appendix B, Tables B-3 to B-20
 * (the function), B-21 and B-22 (the Group Terminal Block set), B-23 and B-24 (the requests). */
#ifndef ZMIDI_USB
#define ZMIDI_USB
#endif
#include "zmidi.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int failures;
#define CHECK(c) do { if (!(c)) { fprintf(stderr, "%s:%d: CHECK(%s)\n", __FILE__, __LINE__, #c); failures++; } } while (0)

/* Tables B-3 to B-14 (Alternate Setting 0, the same bytes as USB-MIDI v1.0 Appendix B), then
 * Tables B-15 to B-20 (Alternate Setting 1): 132 bytes. */
static const uint8_t APPENDIX_B[132] = {
  0x09, 0x04, 0x00, 0x00, 0x00, 0x01, 0x01, 0x00, 0x00, /* B-3 standard AC interface */
  0x09, 0x24, 0x01, 0x00, 0x01, 0x09, 0x00, 0x01, 0x01, /* B-4 AC header */
  0x09, 0x04, 0x01, 0x00, 0x02, 0x01, 0x03, 0x00, 0x00, /* B-5 MS interface, alt 0 */
  0x07, 0x24, 0x01, 0x00, 0x01, 0x41, 0x00,             /* B-6 MS header, bcdMSC 0x0100 */
  0x06, 0x24, 0x02, 0x01, 0x01, 0x00,                   /* B-7 Embedded MIDI IN Jack 1 */
  0x06, 0x24, 0x02, 0x02, 0x02, 0x00,                   /* B-8 External MIDI IN Jack 2 */
  0x09, 0x24, 0x03, 0x01, 0x03, 0x01, 0x02, 0x01, 0x00, /* B-9 Embedded MIDI OUT Jack 3 */
  0x09, 0x24, 0x03, 0x02, 0x04, 0x01, 0x01, 0x01, 0x00, /* B-10 External MIDI OUT Jack 4 */
  0x09, 0x05, 0x01, 0x02, 0x40, 0x00, 0x00, 0x00, 0x00, /* B-11 bulk OUT endpoint 1 */
  0x05, 0x25, 0x01, 0x01, 0x01,                         /* B-12 MS_GENERAL, Jack 1 */
  0x09, 0x05, 0x81, 0x02, 0x40, 0x00, 0x00, 0x00, 0x00, /* B-13 bulk IN endpoint 1 */
  0x05, 0x25, 0x01, 0x01, 0x03,                         /* B-14 MS_GENERAL, Jack 3 */
  0x09, 0x04, 0x01, 0x01, 0x02, 0x01, 0x03, 0x00, 0x00, /* B-15 MS interface, alt 1 */
  0x07, 0x24, 0x01, 0x00, 0x02, 0x07, 0x00,             /* B-16 MS header, bcdMSC 0x0200 */
  0x07, 0x05, 0x01, 0x02, 0x40, 0x00, 0x00,             /* B-17 bulk OUT endpoint 1 */
  0x05, 0x25, 0x02, 0x01, 0x01,                         /* B-18 MS_GENERAL_2_0, block 1 */
  0x07, 0x05, 0x81, 0x03, 0x40, 0x00, 0x01,             /* B-19 interrupt IN endpoint 1 */
  0x05, 0x25, 0x02, 0x01, 0x01,                         /* B-20 MS_GENERAL_2_0, block 1 */
};

/* Table B-21 (header, wTotalLength 0x0012) and Table B-22 (block 1). */
static const uint8_t B21_B22[18] = {
  0x05, 0x26, 0x01, 0x12, 0x00,
  0x0D, 0x26, 0x02, 0x01, 0x00, 0x00, 0x01, 0x04, 0x00, 0x01, 0x00, 0x00, 0x00,
};
/* Table B-23: the first request (Alternate Setting 1, interface 1, wLength 5). */
static const uint8_t B23[8] = {0x81, 0x06, 0x01, 0x26, 0x01, 0x00, 0x05, 0x00};
/* Table B-24: the second request (wLength 0x12). */
static const uint8_t B24[8] = {0x81, 0x06, 0x01, 0x26, 0x01, 0x00, 0x12, 0x00};

/* The block of Table B-22: ID 1, bidirectional, Group Terminal 1, iBlockItem 4, bMIDIProtocol
 * 0x00, wMaxInputBandwidth 0x0001. */
static const ZmidiUsbBlock B22 = {1, 0, 0, 1, 4, 0x00, 0x0001, 0x0000};

static ZmidiUsbFunction fn;

static ZmidiUsbEndpoint endpoint(uint8_t direction, uint8_t transfer, uint8_t interval) {
  ZmidiUsbEndpoint e;
  memset(&e, 0, sizeof e);
  e.direction = direction;
  e.number = 1;
  e.transfer = transfer;
  e.interval = interval;
  e.max_packet = 64;
  e.block_count = 1;
  e.block_ids[0] = 1;
  return e;
}

/* The Appendix B function: AudioControl interface 0, MIDI Streaming interface 1, one cable in
 * each direction, Alternate Setting 1 with bulk OUT 1 and interrupt IN 1 on the block of B-22. */
static void appendix_b(void) {
  ZmidiUsbFunctionConfig c = {0, 1, false, 0, 0, 1, 1, 1, 1, 0};
  ZmidiUsbEndpoint out = endpoint(0, 2, 0), in = endpoint(1, 3, 1);
  uint8_t alt = 0;
  CHECK(zmidi_usb_function_init(&fn, &c) == ZMIDI_STATUS_OK);
  CHECK(zmidi_usb_function_add_setting(&fn, 0, &alt) == ZMIDI_STATUS_OK && alt == 1);
  CHECK(zmidi_usb_function_add_endpoint(&fn, 1, &out) == ZMIDI_STATUS_OK);
  CHECK(zmidi_usb_function_add_endpoint(&fn, 1, &in) == ZMIDI_STATUS_OK);
  CHECK(zmidi_usb_function_set_blocks(&fn, 1, &B22, 1) == ZMIDI_STATUS_OK);
}

static void setup_answer(const uint8_t setup[8], size_t capacity, ZmidiStatus status,
                         ZmidiUsbGtbResponse response, size_t n) {
  uint8_t out[64];
  ZmidiUsbGtbResponse r = ZMIDI_USB_GTB_RESPONSE_ANSWERED;
  size_t written = 99;
  memset(out, 0xEE, sizeof out);
  CHECK(zmidi_usb_gtb_setup(&fn, setup, out, capacity, &r, &written) == status);
  if (status == ZMIDI_STATUS_OK) {
    CHECK(r == response && written == n);
    CHECK(memcmp(out, B21_B22, n) == 0);
  } else {
    CHECK(written == 99);
  }
  CHECK(out[n] == 0xEE); /* nothing after the answer */
}

int main(void) {
  uint8_t out[256];
  size_t len = 0, written = 0, i;
  uint8_t alt = 0;
  uint16_t total = 0;
  ZmidiUsbSettingEndpoint e;
  ZmidiUsbBlock b;

  CHECK(zmidi_usb_function_size() <= sizeof fn);

  /* REQ-FFI-USB-006: the function writer gives the bytes of Appendix B. */
  appendix_b();
  CHECK(zmidi_usb_function_len(&fn, &len) == ZMIDI_STATUS_OK && len == sizeof APPENDIX_B);
  memset(out, 0xEE, sizeof out);
  CHECK(zmidi_usb_function_write(&fn, out, sizeof out, &written) == ZMIDI_STATUS_OK);
  CHECK(written == sizeof APPENDIX_B && memcmp(out, APPENDIX_B, written) == 0);
  CHECK(out[written] == 0xEE);
  memset(out, 0xEE, sizeof out);
  written = 99;
  CHECK(zmidi_usb_function_write(&fn, out, sizeof APPENDIX_B - 1, &written) == ZMIDI_STATUS_BUFFER_TOO_SMALL);
  CHECK(written == 99 && out[0] == 0xEE);

  /* The endpoints in descriptor order: OUT 1 and IN 1 of Alternate Setting 0, then of 1. */
  CHECK(zmidi_usb_function_endpoint_count(&fn) == 4);
  CHECK(zmidi_usb_function_endpoint(&fn, 0, &e) == ZMIDI_STATUS_OK);
  CHECK(e.alternate_setting == 0 && e.address == 0x01 && e.transfer == 2 && e.max_packet == 64);
  CHECK(zmidi_usb_function_endpoint(&fn, 1, &e) == ZMIDI_STATUS_OK && e.alternate_setting == 0 && e.address == 0x81);
  CHECK(zmidi_usb_function_endpoint(&fn, 2, &e) == ZMIDI_STATUS_OK && e.alternate_setting == 1 && e.address == 0x01);
  CHECK(e.transfer == 2 && e.interval == 0);
  CHECK(zmidi_usb_function_endpoint(&fn, 3, &e) == ZMIDI_STATUS_OK && e.alternate_setting == 1 && e.address == 0x81);
  CHECK(e.transfer == 3 && e.interval == 1 && e.max_packet == 64);
  CHECK(zmidi_usb_function_endpoint(&fn, 4, &e) == ZMIDI_STATUS_INVALID_ARGUMENT);

  /* The Group mask of each direction for the link (REQ-USB-GTB-014). */
  CHECK(zmidi_usb_function_group_mask(&fn, 0, 0) == 0xFFFF);
  CHECK(zmidi_usb_function_group_mask(&fn, 1, 0) == 0x0001);
  CHECK(zmidi_usb_function_group_mask(&fn, 1, 1) == 0x0001);
  CHECK(zmidi_usb_function_group_mask(&fn, 2, 0) == 0);
  CHECK(zmidi_usb_function_group_mask(&fn, 1, 2) == 0);
  CHECK(zmidi_usb_function_group_mask(NULL, 0, 0) == 0);

  /* REQ-FFI-USB-007: the Group Terminal Block set of Tables B-21 and B-22. */
  CHECK(zmidi_usb_gtb_set_len(&fn, 1, &len) == ZMIDI_STATUS_OK && len == sizeof B21_B22);
  CHECK(zmidi_usb_gtb_set_write(&fn, 1, out, sizeof out, &written) == ZMIDI_STATUS_OK);
  CHECK(written == sizeof B21_B22 && memcmp(out, B21_B22, written) == 0);
  CHECK(zmidi_usb_gtb_set_write(&fn, 1, out, sizeof B21_B22 - 1, &written) == ZMIDI_STATUS_BUFFER_TOO_SMALL);
  CHECK(zmidi_usb_gtb_set_len(&fn, 0, &len) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_usb_gtb_set_len(&fn, 2, &len) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_usb_gtb_set_write(&fn, 0, out, sizeof out, &written) == ZMIDI_STATUS_INVALID_ARGUMENT);

  /* REQ-FFI-USB-008: the device side of the request (section 6). */
  setup_answer(B23, 64, ZMIDI_STATUS_OK, ZMIDI_USB_GTB_RESPONSE_ANSWERED, 5);
  setup_answer(B24, 64, ZMIDI_STATUS_OK, ZMIDI_USB_GTB_RESPONSE_ANSWERED, 18);
  setup_answer(B24, 17, ZMIDI_STATUS_BUFFER_TOO_SMALL, ZMIDI_USB_GTB_RESPONSE_ANSWERED, 0);
  {
    /* A wLength above wTotalLength gives the whole set only. */
    uint8_t s[8];
    memcpy(s, B24, 8);
    s[6] = 0x40;
    setup_answer(s, 64, ZMIDI_STATUS_OK, ZMIDI_USB_GTB_RESPONSE_ANSWERED, 18);
    /* Another interface (wIndex 2), Alternate Setting 0 and Alternate Setting 2: stall. */
    memcpy(s, B24, 8);
    s[4] = 0x02;
    setup_answer(s, 64, ZMIDI_STATUS_OK, ZMIDI_USB_GTB_RESPONSE_STALL, 0);
    memcpy(s, B24, 8);
    s[5] = 0x01; /* wIndex 0x0101: the 16-bit compare */
    setup_answer(s, 64, ZMIDI_STATUS_OK, ZMIDI_USB_GTB_RESPONSE_STALL, 0);
    memcpy(s, B24, 8);
    s[2] = 0x00;
    setup_answer(s, 64, ZMIDI_STATUS_OK, ZMIDI_USB_GTB_RESPONSE_STALL, 0);
    s[2] = 0x02;
    setup_answer(s, 64, ZMIDI_STATUS_OK, ZMIDI_USB_GTB_RESPONSE_STALL, 0);
    /* GET_DESCRIPTOR of the device descriptor and a class request: not this request. */
    {
      static const uint8_t device[8] = {0x80, 0x06, 0x00, 0x01, 0x00, 0x00, 0x12, 0x00};
      static const uint8_t other[8] = {0x21, 0x06, 0x01, 0x26, 0x01, 0x00, 0x12, 0x00};
      setup_answer(device, 64, ZMIDI_STATUS_OK, ZMIDI_USB_GTB_RESPONSE_NOT_HANDLED, 0);
      setup_answer(other, 64, ZMIDI_STATUS_OK, ZMIDI_USB_GTB_RESPONSE_NOT_HANDLED, 0);
    }
  }

  /* REQ-FFI-USB-009: the host side. The requests of Tables B-23 and B-24, the wTotalLength of the
   * header-only answer, and the block of Table B-22. */
  memset(out, 0, sizeof out);
  CHECK(zmidi_usb_gtb_request(1, 1, 5, out) == ZMIDI_STATUS_OK && memcmp(out, B23, 8) == 0);
  CHECK(zmidi_usb_gtb_request(1, 1, 0x12, out) == ZMIDI_STATUS_OK && memcmp(out, B24, 8) == 0);
  CHECK(zmidi_usb_gtb_request(1, 1, 5, NULL) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_usb_gtb_total_length(B21_B22, 5, &total) == ZMIDI_STATUS_OK && total == 0x12);
  CHECK(zmidi_usb_gtb_total_length(B21_B22, 4, &total) == ZMIDI_STATUS_REJECTED);
  CHECK(zmidi_usb_gtb_total_length(B23, 8, &total) == ZMIDI_STATUS_REJECTED); /* not a header */
  CHECK(zmidi_usb_gtb_total_length(NULL, 0, &total) == ZMIDI_STATUS_REJECTED);
  memset(&b, 0, sizeof b);
  CHECK(zmidi_usb_gtb_block(B21_B22, sizeof B21_B22, 0, &b) == ZMIDI_STATUS_OK);
  CHECK(memcmp(&b, &B22, sizeof b) == 0);
  CHECK(zmidi_usb_gtb_block(B21_B22, sizeof B21_B22, 1, &b) == ZMIDI_STATUS_NO_PENDING);
  /* A short answer (the header only) has no block. */
  CHECK(zmidi_usb_gtb_block(B21_B22, 5, 0, &b) == ZMIDI_STATUS_NO_PENDING);
  CHECK(zmidi_usb_gtb_block(NULL, 4, 0, &b) == ZMIDI_STATUS_INVALID_ARGUMENT);

  /* REQ-FFI-USB-010: the rules and the capacities. */
  CHECK(zmidi_usb_function_add_setting(&fn, 7, &alt) == ZMIDI_STATUS_OK && alt == 2);
  CHECK(zmidi_usb_function_add_setting(&fn, 0, &alt) == ZMIDI_STATUS_REJECTED && alt == 2);
  /* Alternate Setting 2 has no endpoint: the writers refuse the function (REQ-USB-GEN-008). */
  CHECK(zmidi_usb_function_len(&fn, &len) == ZMIDI_STATUS_REJECTED);
  /* REQ-FFI-USB-006: nothing is written. */
  memset(out, 0xEE, sizeof out);
  written = 99;
  CHECK(zmidi_usb_function_write(&fn, out, sizeof out, &written) == ZMIDI_STATUS_REJECTED);
  CHECK(written == 99 && out[0] == 0xEE && out[sizeof out - 1] == 0xEE);
  {
    /* Four MIDI OUT endpoints, each on its own block: a block is named by one endpoint of a
     * direction only (REQ-USB-GTB-013). */
    ZmidiUsbEndpoint ep = endpoint(0, 2, 0);
    ZmidiUsbBlock four[ZMIDI_USB_MAX_ENDPOINTS];
    for (i = 0; i < ZMIDI_USB_MAX_ENDPOINTS; i++) {
      four[i] = B22;
      four[i].id = (uint8_t)(i + 1);
      four[i].first_group = (uint8_t)i;
      four[i].kind = 1; /* input only: a bidirectional block needs a MIDI IN endpoint too */
      ep.number = (uint8_t)(i + 2);
      ep.block_ids[0] = (uint8_t)(i + 1);
      CHECK(zmidi_usb_function_add_endpoint(&fn, 2, &ep) == ZMIDI_STATUS_OK);
    }
    CHECK(zmidi_usb_function_add_endpoint(&fn, 2, &ep) == ZMIDI_STATUS_REJECTED);
    CHECK(zmidi_usb_function_len(&fn, &len) == ZMIDI_STATUS_REJECTED); /* blocks 2 to 4 missing */
    CHECK(zmidi_usb_function_set_blocks(&fn, 2, four, ZMIDI_USB_MAX_ENDPOINTS) == ZMIDI_STATUS_OK);
    CHECK(zmidi_usb_function_group_mask(&fn, 2, 0) == 0x000F);
    CHECK(zmidi_usb_function_group_mask(&fn, 2, 1) == 0);
    CHECK(zmidi_usb_function_len(&fn, &len) == ZMIDI_STATUS_OK);
    /* iInterface 7 of Alternate Setting 2. */
    CHECK(zmidi_usb_function_write(&fn, out, sizeof out, &written) == ZMIDI_STATUS_OK && written == len);
    CHECK(out[sizeof APPENDIX_B + 3] == 2 && out[sizeof APPENDIX_B + 8] == 7);
    ep.direction = 2;
    CHECK(zmidi_usb_function_add_endpoint(&fn, 1, &ep) == ZMIDI_STATUS_INVALID_ARGUMENT);
    ep.direction = 0;
    ep.transfer = 1; /* isochronous */
    CHECK(zmidi_usb_function_add_endpoint(&fn, 1, &ep) == ZMIDI_STATUS_INVALID_ARGUMENT);
    ep.transfer = 2;
    ep.block_count = 17;
    CHECK(zmidi_usb_function_add_endpoint(&fn, 1, &ep) == ZMIDI_STATUS_INVALID_ARGUMENT);
    ep.block_count = 1;
    ep.block_ids[0] = 1;
    CHECK(zmidi_usb_function_add_endpoint(&fn, 0, &ep) == ZMIDI_STATUS_INVALID_ARGUMENT);
    CHECK(zmidi_usb_function_add_endpoint(&fn, 3, &ep) == ZMIDI_STATUS_INVALID_ARGUMENT);
    CHECK(zmidi_usb_function_add_endpoint(&fn, 1, NULL) == ZMIDI_STATUS_INVALID_ARGUMENT);
    CHECK(zmidi_usb_function_add_endpoint(NULL, 1, &ep) == ZMIDI_STATUS_INVALID_ARGUMENT);
  }
  {
    /* A block with no Group Terminal (nNumGroupTrm 0): the writers refuse it (REQ-USB-GTB-006). */
    ZmidiUsbBlock blocks[ZMIDI_USB_MAX_BLOCKS + 1];
    ZmidiUsbBlock bad = B22;
    bad.num_groups = 0;
    CHECK(zmidi_usb_function_set_blocks(&fn, 1, &bad, 1) == ZMIDI_STATUS_OK);
    CHECK(zmidi_usb_function_len(&fn, &len) == ZMIDI_STATUS_REJECTED);
    memset(out, 0xEE, sizeof out);
    written = 99;
    CHECK(zmidi_usb_gtb_set_write(&fn, 1, out, sizeof out, &written) == ZMIDI_STATUS_REJECTED);
    CHECK(written == 99 && out[0] == 0xEE && out[sizeof out - 1] == 0xEE);
    setup_answer(B24, 64, ZMIDI_STATUS_REJECTED, ZMIDI_USB_GTB_RESPONSE_ANSWERED, 0);
    /* 16 blocks, one for each Group, fit; 17 do not. */
    for (i = 0; i <= ZMIDI_USB_MAX_BLOCKS; i++) {
      blocks[i] = B22;
      blocks[i].id = (uint8_t)(i + 1);
      blocks[i].first_group = (uint8_t)i;
    }
    CHECK(zmidi_usb_function_set_blocks(&fn, 1, blocks, ZMIDI_USB_MAX_BLOCKS + 1) == ZMIDI_STATUS_REJECTED);
    CHECK(zmidi_usb_function_len(&fn, &len) == ZMIDI_STATUS_REJECTED); /* the bad block stays */
    CHECK(zmidi_usb_function_set_blocks(&fn, 1, blocks, ZMIDI_USB_MAX_BLOCKS) == ZMIDI_STATUS_OK);
    CHECK(zmidi_usb_gtb_set_len(&fn, 1, &len) == ZMIDI_STATUS_OK && len == 5 + 13 * ZMIDI_USB_MAX_BLOCKS);
    /* The endpoints of Alternate Setting 1 name block 1 only (REQ-USB-GTB-013). */
    CHECK(zmidi_usb_function_len(&fn, &len) == ZMIDI_STATUS_REJECTED);
    CHECK(zmidi_usb_function_set_blocks(&fn, 1, NULL, 1) == ZMIDI_STATUS_INVALID_ARGUMENT);
    CHECK(zmidi_usb_function_set_blocks(&fn, 0, blocks, 1) == ZMIDI_STATUS_INVALID_ARGUMENT);
    /* No block: the endpoints name a block that does not exist (REQ-USB-GTB-013). */
    CHECK(zmidi_usb_function_set_blocks(&fn, 1, NULL, 0) == ZMIDI_STATUS_OK);
    CHECK(zmidi_usb_function_len(&fn, &len) == ZMIDI_STATUS_REJECTED);
  }
  {
    /* The same AudioControl and MIDI Streaming interface number; 17 cables. */
    ZmidiUsbFunctionConfig c = {1, 1, false, 0, 0, 1, 1, 1, 1, 0};
    CHECK(zmidi_usb_function_init(&fn, &c) == ZMIDI_STATUS_OK);
    CHECK(zmidi_usb_function_len(&fn, &len) == ZMIDI_STATUS_REJECTED);
    c.ac_interface = 0;
    c.n_out = 17;
    CHECK(zmidi_usb_function_init(&fn, &c) == ZMIDI_STATUS_OK);
    CHECK(zmidi_usb_function_len(&fn, &len) == ZMIDI_STATUS_REJECTED);
    /* Alternate Setting 0 only, with strings 5 (AudioControl) and 6 (Alternate Setting 0): the
     * 92 bytes of USB-MIDI v1.0 Appendix B with the iInterface values. */
    c.n_out = 1;
    c.ac_string = 5;
    c.alt0_string = 6;
    CHECK(zmidi_usb_function_init(&fn, &c) == ZMIDI_STATUS_OK);
    CHECK(zmidi_usb_function_write(&fn, out, sizeof out, &written) == ZMIDI_STATUS_OK && written == 92);
    CHECK(out[8] == 5 && out[26] == 6 && memcmp(out + 27, APPENDIX_B + 27, 92 - 27) == 0);
    CHECK(zmidi_usb_gtb_set_len(&fn, 1, &len) == ZMIDI_STATUS_INVALID_ARGUMENT);
    setup_answer(B24, 64, ZMIDI_STATUS_OK, ZMIDI_USB_GTB_RESPONSE_STALL, 0);
    /* No cable and no USB MIDI 2.0 setting: no port (REQ-USB-MS1-027). */
    c.n_out = 0;
    c.n_in = 0;
    CHECK(zmidi_usb_function_init(&fn, &c) == ZMIDI_STATUS_OK);
    CHECK(zmidi_usb_function_len(&fn, &len) == ZMIDI_STATUS_REJECTED);
    CHECK(zmidi_usb_function_init(&fn, NULL) == ZMIDI_STATUS_INVALID_ARGUMENT);
    CHECK(zmidi_usb_function_init(NULL, &c) == ZMIDI_STATUS_INVALID_ARGUMENT);
  }

  {
    /* 16 bidirectional blocks, one for each Group, on one MIDI OUT and one MIDI IN endpoint that
     * name all 16 (the most block IDs of an endpoint). */
    ZmidiUsbFunctionConfig c = {0, 1, false, 0, 0, 1, 1, 1, 1, 0};
    ZmidiUsbEndpoint o = endpoint(0, 2, 0), n = endpoint(1, 2, 0);
    ZmidiUsbBlock blocks[ZMIDI_USB_MAX_BLOCKS];
    for (i = 0; i < ZMIDI_USB_MAX_BLOCKS; i++) {
      blocks[i] = B22;
      blocks[i].id = (uint8_t)(i + 1);
      blocks[i].first_group = (uint8_t)i;
      o.block_ids[i] = n.block_ids[i] = (uint8_t)(i + 1);
    }
    o.block_count = n.block_count = ZMIDI_USB_MAX_BLOCK_IDS;
    CHECK(zmidi_usb_function_init(&fn, &c) == ZMIDI_STATUS_OK);
    CHECK(zmidi_usb_function_add_setting(&fn, 0, &alt) == ZMIDI_STATUS_OK && alt == 1);
    CHECK(zmidi_usb_function_add_endpoint(&fn, 1, &o) == ZMIDI_STATUS_OK);
    CHECK(zmidi_usb_function_add_endpoint(&fn, 1, &n) == ZMIDI_STATUS_OK);
    CHECK(zmidi_usb_function_set_blocks(&fn, 1, blocks, ZMIDI_USB_MAX_BLOCKS) == ZMIDI_STATUS_OK);
    CHECK(zmidi_usb_function_len(&fn, &len) == ZMIDI_STATUS_OK);
    CHECK(zmidi_usb_function_group_mask(&fn, 1, 0) == 0xFFFF && zmidi_usb_function_group_mask(&fn, 1, 1) == 0xFFFF);
    CHECK(zmidi_usb_gtb_set_write(&fn, 1, out, sizeof out, &written) == ZMIDI_STATUS_OK && written == 5 + 13 * 16);
    CHECK(zmidi_usb_gtb_block(out, written, 15, &b) == ZMIDI_STATUS_OK && b.id == 16 && b.first_group == 15);
    CHECK(zmidi_usb_gtb_block(out, written, 16, &b) == ZMIDI_STATUS_NO_PENDING);
  }

  /* The default function (choices U-06, U-09): Appendix B with bulk IN 1 and the default block
   * (ID 1, bidirectional, Group 1, bMIDIProtocol 0x11). High speed gives wMaxPacketSize 512. */
  CHECK(zmidi_usb_function_init_default(&fn, 0, 1, false) == ZMIDI_STATUS_OK);
  CHECK(zmidi_usb_function_write(&fn, out, sizeof out, &written) == ZMIDI_STATUS_OK && written == sizeof APPENDIX_B);
  CHECK(memcmp(out, APPENDIX_B, 123) == 0); /* to bEndpointAddress of B-19 */
  CHECK(out[123] == 0x02 && out[126] == 0x00 && memcmp(out + 127, APPENDIX_B + 127, 5) == 0);
  CHECK(zmidi_usb_gtb_set_write(&fn, 1, out, sizeof out, &written) == ZMIDI_STATUS_OK && written == 18);
  CHECK(zmidi_usb_gtb_block(out, written, 0, &b) == ZMIDI_STATUS_OK);
  CHECK(b.id == 1 && b.kind == 0 && b.first_group == 0 && b.num_groups == 1 && b.protocol == 0x11);
  CHECK(zmidi_usb_function_init_default(&fn, 0, 1, true) == ZMIDI_STATUS_OK);
  CHECK(zmidi_usb_function_endpoint(&fn, 3, &e) == ZMIDI_STATUS_OK && e.max_packet == 512);
  CHECK(zmidi_usb_function_init_default(&fn, 1, 1, false) == ZMIDI_STATUS_OK);
  CHECK(zmidi_usb_function_len(&fn, &len) == ZMIDI_STATUS_REJECTED); /* the same interface number */
  CHECK(zmidi_usb_function_init_default(NULL, 0, 1, false) == ZMIDI_STATUS_INVALID_ARGUMENT);

  /* Null pointers. */
  CHECK(zmidi_usb_function_len(NULL, &len) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_usb_function_len(&fn, NULL) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_usb_function_write(&fn, NULL, 0, &written) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_usb_function_endpoint_count(NULL) == 0);
  CHECK(zmidi_usb_function_endpoint(&fn, 0, NULL) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_usb_function_add_setting(&fn, 0, NULL) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_usb_gtb_setup(&fn, NULL, out, sizeof out, NULL, &written) == ZMIDI_STATUS_INVALID_ARGUMENT);

  if (failures) {
    fprintf(stderr, "usb_desc: %d failures\n", failures);
    return EXIT_FAILURE;
  }
  puts("usb_desc: ok");
  return EXIT_SUCCESS;
}
