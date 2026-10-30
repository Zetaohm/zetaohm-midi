/* C test of the usb part of the C ABI (REQ-FFI-USB-005). Vectors: USB-MIDI 1.0 (Event Packets:
 * CIN 0x9 Note On, CIN 0x4 and 0x7 SysEx) at Alternate Setting 0, and USB MIDI 2.0 (UMP words,
 * least significant byte first) at Alternate Setting 1. */
#ifndef ZMIDI_USB
#define ZMIDI_USB
#endif
#include "zmidi.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int failures;
#define CHECK(c) do { if (!(c)) { fprintf(stderr, "%s:%d: CHECK(%s)\n", __FILE__, __LINE__, #c); failures++; } } while (0)

static ZmidiUsbLink link;
static ZmidiUsbUmp rec[32];

static ZmidiUsbUmp ump(uint32_t w0, uint32_t w1, uint8_t n) {
  ZmidiUsbUmp u;
  memset(&u, 0, sizeof u);
  u.word_count = n;
  u.words[0] = w0;
  u.words[1] = w1;
  return u;
}

int main(void) {
  ZmidiUsbLinkConfig c = {false, 0xFFFF, 64, 1};
  size_t n = 99, errors = 99, len = 0, taken = 0;
  uint8_t out[64];
  ZmidiUsbUmp u[2];

  CHECK(zmidi_usb_link_size() <= sizeof link);
  /* DEC-019 rule 6: storage of exactly zmidi_usb_link_size() bytes is enough (ASan). */
  {
    ZmidiUsbLink *h = (ZmidiUsbLink *)malloc(zmidi_usb_link_size());
    CHECK(h != NULL && zmidi_usb_link_init(h, &c) == ZMIDI_STATUS_OK);
    free(h);
  }
  CHECK(zmidi_usb_link_init(&link, &c) == ZMIDI_STATUS_OK);
  CHECK(zmidi_usb_link_setting(&link) == 0 && !zmidi_usb_link_is_ump(&link));

  {
    /* Alternate Setting 0: Note On on cable 0 (CIN 0x9) gives a MIDI 1.0 Protocol UMP on Group 0. */
    static const uint8_t ep[] = {0x09, 0x90, 0x3C, 0x64};
    CHECK(zmidi_usb_link_read(&link, ep, sizeof ep, rec, 32, &n, &errors) == ZMIDI_STATUS_OK);
    CHECK(n == 1 && errors == 0 && rec[0].word_count == 1 && rec[0].words[0] == 0x20903C64u);
  }
  {
    /* SysEx F0 7E 7F 06 01 F7 in two Event Packets (CIN 0x4, then CIN 0x7 with 3 bytes). */
    static const uint8_t ep[] = {0x04, 0xF0, 0x7E, 0x7F, 0x07, 0x06, 0x01, 0xF7};
    CHECK(zmidi_usb_link_read(&link, ep, sizeof ep, rec, 32, &n, &errors) == ZMIDI_STATUS_OK);
    CHECK(n == 1 && rec[0].word_count == 2 && rec[0].words[0] == 0x30047E7Fu && rec[0].words[1] == 0x06010000u);
  }
  {
    /* The writer at Alternate Setting 0: the same Event Packets. */
    static const uint8_t want[] = {0x09, 0x90, 0x3C, 0x64};
    u[0] = ump(0x20903C64u, 0, 1);
    CHECK(zmidi_usb_link_write_packet(&link, u, 1, out, sizeof out, &len, &taken) == ZMIDI_STATUS_OK);
    CHECK(len == 4 && taken == 1 && memcmp(out, want, 4) == 0);
    u[0] = ump(0x30047E7Fu, 0x06010000u, 2);
    CHECK(zmidi_usb_link_write_packet(&link, u, 1, out, sizeof out, &len, &taken) == ZMIDI_STATUS_OK);
    CHECK(len == 8 && out[0] == 0x04 && out[1] == 0xF0 && out[4] == 0x07 && out[7] == 0xF7);
  }

  /* Alternate Setting 1: UMP words, least significant byte first. */
  CHECK(zmidi_usb_link_set_alternate_setting(&link, 1, 0x0001, 64) == ZMIDI_STATUS_OK);
  CHECK(zmidi_usb_link_setting(&link) == 1 && zmidi_usb_link_is_ump(&link));
  {
    static const uint8_t data[] = {0x00, 0x3C, 0x90, 0x40, 0x00, 0x00, 0x00, 0xC0};
    CHECK(zmidi_usb_link_read(&link, data, sizeof data, rec, 32, &n, &errors) == ZMIDI_STATUS_OK);
    CHECK(n == 1 && rec[0].word_count == 2 && rec[0].words[0] == 0x40903C00u && rec[0].words[1] == 0xC0000000u);
    u[0] = ump(0x40903C00u, 0xC0000000u, 2);
    CHECK(zmidi_usb_link_write_packet(&link, u, 1, out, sizeof out, &len, &taken) == ZMIDI_STATUS_OK);
    CHECK(len == 8 && memcmp(out, data, 8) == 0);
  }

  /* The Group Terminal mask of the setting (0x0001): zmidi-usb gives a UMP on another Group
   * too, as a UMP with no Group Terminal (the record has no Group Terminal field). */
  CHECK(zmidi_usb_link_packet_size(&link) == 64);
  {
    static const uint8_t g2[] = {0x00, 0x3C, 0x92, 0x42, 0x00, 0x00, 0x00, 0xC0};
    CHECK(zmidi_usb_link_read(&link, g2, sizeof g2, rec, 32, &n, &errors) == ZMIDI_STATUS_OK);
    CHECK(n == 1 && errors == 0 && rec[0].words[0] == 0x42923C00u);
  }
  {
    /* REQ-FFI-USB-003: the errors of zmidi-usb are counted: 9 bytes give one UMP and one error
     * for the last byte (REQ-USB-UMP-006). */
    static const uint8_t odd[] = {0x00, 0x3C, 0x90, 0x40, 0x00, 0x00, 0x00, 0xC0, 0x55};
    CHECK(zmidi_usb_link_read(&link, odd, sizeof odd, rec, 32, &n, &errors) == ZMIDI_STATUS_OK);
    CHECK(n == 1 && errors == 1);
  }
  /* A configuration change goes back to Alternate Setting 0. */
  CHECK(zmidi_usb_link_configuration_change(&link) == ZMIDI_STATUS_OK);
  CHECK(zmidi_usb_link_setting(&link) == 0 && !zmidi_usb_link_is_ump(&link));
  /* A smaller wMaxPacketSize is the packet size of the setting. */
  CHECK(zmidi_usb_link_set_alternate_setting(&link, 1, 0xFFFF, 32) == ZMIDI_STATUS_OK);
  CHECK(zmidi_usb_link_packet_size(&link) == 32);

  /* A USB reset goes back to Alternate Setting 0. */
  CHECK(zmidi_usb_link_usb_reset(&link) == ZMIDI_STATUS_OK && zmidi_usb_link_setting(&link) == 0);
  CHECK(zmidi_usb_link_packet_size(&link) == 64);
  {
    /* The reader holds the part of a SysEx message until its end comes. */
    static const uint8_t start[] = {0x04, 0xF0, 0x7E, 0x7F};
    static const uint8_t end[] = {0x07, 0x06, 0x01, 0xF7};
    CHECK(!zmidi_usb_link_holds_part(&link));
    CHECK(zmidi_usb_link_read(&link, start, sizeof start, rec, 32, &n, &errors) == ZMIDI_STATUS_OK && n == 0);
    CHECK(zmidi_usb_link_holds_part(&link));
    CHECK(zmidi_usb_link_read(&link, end, sizeof end, rec, 32, &n, &errors) == ZMIDI_STATUS_OK && n == 1);
    CHECK(!zmidi_usb_link_holds_part(&link));
  }

  /* REQ-FFI-USB-003: a SysEx start with no room is refused before it is read. */
  {
    static const uint8_t start[] = {0x04, 0xF0, 0x7E, 0x7F};
    n = 99;
    CHECK(zmidi_usb_link_read(&link, start, sizeof start, rec, 0, &n, &errors) == ZMIDI_STATUS_BUFFER_TOO_SMALL);
    CHECK(n == 99 && !zmidi_usb_link_holds_part(&link));
  }
  /* Small buffers and bad values. */
  {
    static const uint8_t ep[] = {0x09, 0x90, 0x3C, 0x64, 0x09, 0x80, 0x3C, 0x00};
    CHECK(zmidi_usb_link_read(&link, ep, sizeof ep, rec, 2, &n, &errors) == ZMIDI_STATUS_BUFFER_TOO_SMALL);
    CHECK(zmidi_usb_link_read(&link, ep, sizeof ep, rec, 3, &n, &errors) == ZMIDI_STATUS_OK && n == 2);
    u[0] = ump(0x20903C64u, 0, 1);
    CHECK(zmidi_usb_link_write_packet(&link, u, 1, out, 3, &len, &taken) == ZMIDI_STATUS_BUFFER_TOO_SMALL);
    /* One byte below the packet size: refused with no state change; the retry gives the bytes. */
    CHECK(zmidi_usb_link_write_packet(&link, u, 1, out, 63, &len, &taken) == ZMIDI_STATUS_BUFFER_TOO_SMALL);
    CHECK(zmidi_usb_link_write_packet(&link, u, 1, out, 64, &len, &taken) == ZMIDI_STATUS_OK);
    CHECK(len == 4 && taken == 1 && out[0] == 0x09 && out[1] == 0x90 && out[2] == 0x3C && out[3] == 0x64);
  }
  {
    /* REQ-FFI-USB-001: the Event Packet reader with the MIDI 2.0 Protocol: a Note On is a MIDI 2.0
     * Note On, velocity 0x64 upscaled to 16 bits (M2-115): 0xC924. */
    ZmidiUsbLinkConfig m2 = {false, 0xFFFF, 64, 2};
    static const uint8_t ep[] = {0x09, 0x90, 0x3C, 0x64};
    CHECK(zmidi_usb_link_init(&link, &m2) == ZMIDI_STATUS_OK);
    CHECK(zmidi_usb_link_read(&link, ep, sizeof ep, rec, 32, &n, &errors) == ZMIDI_STATUS_OK);
    CHECK(n == 1 && rec[0].word_count == 2 && rec[0].words[0] == 0x40903C00u && rec[0].words[1] == 0xC9240000u);
    m2.protocol = 3;
    CHECK(zmidi_usb_link_init(&link, &m2) == ZMIDI_STATUS_INVALID_ARGUMENT);
  }
  {
    /* A packet size of 8: a SysEx of 6 data bytes (3 Event Packets) is split, and the writer
     * has a pending part between the two packets. */
    ZmidiUsbLinkConfig small = {false, 0xFFFF, 8, 1};
    CHECK(zmidi_usb_link_init(&link, &small) == ZMIDI_STATUS_OK);
    u[0] = ump(0x30067E7Fu, 0x06010203u, 2);
    CHECK(zmidi_usb_link_write_packet(&link, u, 1, out, 8, &len, &taken) == ZMIDI_STATUS_OK);
    CHECK(len == 8 && taken == 1 && zmidi_usb_link_pending(&link));
    CHECK(zmidi_usb_link_write_packet(&link, u, 0, out, 8, &len, &taken) == ZMIDI_STATUS_OK);
    CHECK(len == 4 && taken == 0 && out[0] == 0x06 && !zmidi_usb_link_pending(&link));
    CHECK(zmidi_usb_link_init(&link, &c) == ZMIDI_STATUS_OK);
    c.max_packet = 2;
    CHECK(zmidi_usb_link_init(&link, &c) == ZMIDI_STATUS_INVALID_ARGUMENT);
    c.max_packet = 64;
    c.protocol = 7;
    CHECK(zmidi_usb_link_init(&link, &c) == ZMIDI_STATUS_INVALID_ARGUMENT);
  }

  if (failures) { fprintf(stderr, "%d failure(s)\n", failures); return EXIT_FAILURE; }
  printf("usb ok\n");
  return EXIT_SUCCESS;
}
