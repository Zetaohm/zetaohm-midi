/* C test of a responder-only ci build (REQ-FFI-CI-009, REQ-FFI-CI-015): the staticlib has the features
 * ci-device and ci-property-exchange only, and the program selects the same blocks of the header.
 * The messages to the device are M2-101 Tables 6, 8 and 17 and the Get Property Data of M2-101 8.8
 * (SysEx7 payload: no F0 and F7). DEC-CI-001: a reply for an Initiator gives no event and no reply. */
#define ZMIDI_CI_DEVICE
#define ZMIDI_CI_PROPERTY_EXCHANGE
#include "zmidi.h"

#if defined(ZMIDI_CI_INITIATOR) || defined(ZMIDI_CI_PROFILES) || defined(ZMIDI_CI_PROCESS_INQUIRY)
#error "the responder-only selection must not turn on other ci blocks"
#endif
#ifndef ZMIDI_PE_PENDING_MAX
#error "the engine of the ci-property-exchange block is missing"
#endif

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int failures;
#define CHECK(c) do { if (!(c)) { fprintf(stderr, "%s:%d: CHECK(%s)\n", __FILE__, __LINE__, #c); failures++; } } while (0)

static ZmidiCi dev;
static uint8_t replies[ZMIDI_CI_REPLY_BUFFER], scratch[256], out[512];
static ZmidiCiEvent ev;
/* REQ-FFI-PE-005: the engine is in this build without its Initiator calls. */
static ZmidiPe engine;

/* The MUID of the Initiator in the messages below: 0x01020304 as 4 bytes of 7 bits, LSB first. */
#define PEER 0x01020304u
#define PEER_BYTES 0x04, 0x06, 0x08, 0x08

static const char list[] = "[{\"resource\":\"DeviceInfo\"}]";
static const char info[] = "{\"manufacturerId\":[0,33,127]}";

static ZmidiCiConfig config(void) {
  ZmidiCiConfig c;
  memset(&c, 0, sizeof c);
  c.manufacturer[0] = 0x00; c.manufacturer[1] = 0x21; c.manufacturer[2] = 0x7F;
  c.family = 1; c.model = 2; c.revision[2] = 1;
  c.max_sysex_size = 512;
  c.group_count = 1;
  c.channels_in_use = 1;
  c.property_exchange = true;
  c.resource_list = (const uint8_t *)list;
  c.resource_list_len = sizeof list - 1;
  return c;
}

/* Gives `msg` to the device; gives the byte count of the replies. */
static size_t receive(const uint8_t *msg, size_t len) {
  size_t n = 0;
  CHECK(zmidi_ci_receive(&dev, 0, msg, len, 1000, replies, sizeof replies, &n, &ev) == ZMIDI_STATUS_OK);
  return n;
}

int main(void) {
  ZmidiCiConfig c = config();
  size_t n = 0;
  uint8_t m[4];
  /* Discovery (Table 6), to Broadcast: Category Supported 0x08 (PE), 512 bytes, Output Path ID 0. */
  static const uint8_t discovery[] = {0x7E, 0x7F, 0x0D, 0x70, 0x02, PEER_BYTES, 0x7F, 0x7F, 0x7F, 0x7F,
                                      0x00, 0x21, 0x7F, 0x01, 0x00, 0x02, 0x00, 0x00, 0x00, 0x01, 0x00,
                                      0x08, 0x00, 0x04, 0x00, 0x00, 0x00};
  /* Get Property Data (M2-101 8.8): Request ID 1, header {"resource":"DeviceInfo"}, 1 of 1 Chunk. */
  static const uint8_t get_head[] = {0x7E, 0x7F, 0x0D, 0x34, 0x02, PEER_BYTES};
  static const char get_header[] = "{\"resource\":\"DeviceInfo\"}";
  uint8_t get[64];
  size_t k = 0;

  CHECK(zmidi_ci_size() <= sizeof(ZmidiCi));
  CHECK(zmidi_pe_size() <= sizeof(ZmidiPe) && zmidi_pe_init(&engine, NULL, 0) == ZMIDI_STATUS_OK);
  /* A category that the build does not have: not supported. */
  c.profiles = true;
  CHECK(zmidi_ci_init(&dev, &c, 3) == ZMIDI_STATUS_NOT_SUPPORTED);
  c.profiles = false;
  c.process_inquiry = true;
  CHECK(zmidi_ci_init(&dev, &c, 3) == ZMIDI_STATUS_NOT_SUPPORTED);
  c.process_inquiry = false;
  CHECK(zmidi_ci_init(&dev, &c, 3) == ZMIDI_STATUS_OK);

  /* Discovery: a Reply to Discovery (Table 8) with the Category Supported bitmap 0x08 only. */
  n = receive(discovery, sizeof discovery);
  CHECK(ev.kind == ZMIDI_CI_EVENT_KIND_NONE);
  CHECK(n >= 3 + 30 && replies[3 + 3] == 0x71);
  CHECK(replies[3 + 9] == 0x04 && replies[3 + 10] == 0x06 && replies[3 + 11] == 0x08 && replies[3 + 12] == 0x08);
  CHECK(replies[3 + 13 + 11] == 0x08);

  /* A Reply to Discovery (Table 8) to this Device is a reply for an Initiator: no event, no reply. */
  m[0] = replies[3 + 5]; m[1] = replies[3 + 6]; m[2] = replies[3 + 7]; m[3] = replies[3 + 8];
  {
    uint8_t reply[31];
    memcpy(reply, discovery, 9);
    reply[3] = 0x71;
    reply[9] = m[0]; reply[10] = m[1]; reply[11] = m[2]; reply[12] = m[3];
    memcpy(reply + 13, discovery + 13, 16);
    reply[29] = 0x00; /* Output Path ID */
    reply[30] = 0x7F; /* Function Block: none */
    n = receive(reply, sizeof reply);
    CHECK(ev.kind == ZMIDI_CI_EVENT_KIND_NONE && n == 0);
  }

  /* Profile Inquiry (Table 17) with no Profiles in the build: a NAK (M2-101 5.11, DEC-017). */
  {
    uint8_t q[13] = {0x7E, 0x00, 0x0D, 0x20, 0x02, PEER_BYTES};
    q[9] = m[0]; q[10] = m[1]; q[11] = m[2]; q[12] = m[3];
    n = receive(q, sizeof q);
    CHECK(n > 3 + 14 && replies[3 + 3] == 0x7F && replies[3 + 13] == 0x20);
  }

  /* Get Property Data of DeviceInfo: the PeGet event and the one-chunk reply of the program. */
  memcpy(get, get_head, sizeof get_head); k = sizeof get_head;
  memcpy(get + k, m, 4); k += 4;
  get[k++] = 0x01;
  get[k++] = (uint8_t)(sizeof get_header - 1); get[k++] = 0x00;
  memcpy(get + k, get_header, sizeof get_header - 1); k += sizeof get_header - 1;
  get[k++] = 0x01; get[k++] = 0x00; get[k++] = 0x01; get[k++] = 0x00; get[k++] = 0x00; get[k++] = 0x00;
  n = receive(get, k);
  CHECK(ev.kind == ZMIDI_CI_EVENT_KIND_PE_GET && ev.muid == PEER && ev.request_id == 1 && n == 0);
  CHECK(strcmp((const char *)ev.resource, "DeviceInfo") == 0);
  CHECK(zmidi_ci_pe_reply(&dev, 200, (const uint8_t *)info, sizeof info - 1, scratch, sizeof scratch, out, sizeof out, &n) == ZMIDI_STATUS_OK);
  /* Reply to Get Property Data (0x35) with Request ID 1, header {"status":200}, the data. */
  CHECK(n > 3 + 13 && out[3 + 3] == 0x35 && out[3 + 13] == 0x01);
  CHECK(memcmp(out + 3 + 16, "{\"status\":200}", 14) == 0);
  CHECK(memcmp(out + n - (sizeof info - 1), info, sizeof info - 1) == 0);
  CHECK(zmidi_ci_pe_reply(&dev, 200, NULL, 0, scratch, sizeof scratch, out, sizeof out, &n) == ZMIDI_STATUS_NO_PENDING);

  if (failures) { fprintf(stderr, "%d failure(s)\n", failures); return EXIT_FAILURE; }
  printf("ci responder ok: state %u B, slot %u B\n", (unsigned)zmidi_ci_size(), (unsigned)sizeof(ZmidiCi));
  return EXIT_SUCCESS;
}
