/* C test of the Property Exchange engine of the C ABI (REQ-FFI-PE-001 to REQ-FFI-PE-010): two devices,
 * A (Initiator) and B (Responder), each with a ZmidiCi and a ZmidiPe, talk through record buffers
 * [group][len lo][len hi][bytes]. M2-103: Get with many Chunks (5.3), ResourceList (9), 404 (8.6.2),
 * Set with many Chunks, Subscription start, full and end (11), timeouts (5.3, 12), MIDI-CI ACK 0x10 and
 * NAK 0x20 (12). The program gives each message to zmidi_ci_receive, then to zmidi_pe_receive. */
#ifndef ZMIDI_CI
#define ZMIDI_CI
#endif
#include "zmidi.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int failures;
#define CHECK(c) do { if (!(c)) { fprintf(stderr, "%s:%d: CHECK(%s)\n", __FILE__, __LINE__, #c); failures++; } } while (0)

static ZmidiCi a, b;
static ZmidiPe pa, pb;
static uint8_t msgs[4096], replies[4096], scratch[1024], body[4096], got[4096];
static size_t got_n;
static ZmidiCiEvent ev;
static uint64_t now = 1000000;

/* The engine events of the last delivery. */
static ZmidiPeEvent events[16];
static size_t event_n;

/* Gives each record of `recs` to `dev` and then to its engine `pe`. The automatic replies of the
 * device go to `rep`. The Property Data of each Message event is added to `got`. */
static void deliver(ZmidiCi *dev, ZmidiPe *pe, const uint8_t *recs, size_t n, uint8_t *rep, size_t *rep_n) {
  size_t at = 0;
  *rep_n = 0;
  event_n = 0;
  while (at + 3 <= n) {
    uint8_t g = recs[at];
    size_t len = (size_t)recs[at + 1] | ((size_t)recs[at + 2] << 8), k = 0;
    ZmidiPeEvent pev;
    CHECK(zmidi_ci_receive(dev, g, recs + at + 3, len, now, rep + *rep_n, sizeof replies - *rep_n, &k, &ev) == ZMIDI_STATUS_OK);
    *rep_n += k;
    CHECK(zmidi_pe_receive(pe, dev, now, scratch, sizeof scratch, &pev) == ZMIDI_STATUS_OK);
    if (pev.kind != ZMIDI_PE_EVENT_KIND_NONE && event_n < 16) {
      events[event_n++] = pev;
    }
    if (pev.kind == ZMIDI_PE_EVENT_KIND_MESSAGE) {
      CHECK(got_n + pev.data_len <= sizeof got);
      memcpy(got + got_n, pev.data, pev.data_len);
      got_n += pev.data_len;
    }
    at += 3 + len;
  }
}

/* Writes every queued control message of `pe` into `out`; gives the byte count. */
static size_t pending(ZmidiPe *pe, ZmidiCi *dev, uint8_t *out, size_t cap) {
  size_t at = 0, k = 0;
  ZmidiStatus st;
  while ((st = zmidi_pe_write_pending(pe, dev, now, out + at, cap - at, &k)) == ZMIDI_STATUS_OK) {
    at += k;
  }
  CHECK(st == ZMIDI_STATUS_NO_PENDING);
  return at;
}

/* Sends `len` bytes of `data` as the Chunks of the transaction (`peer`, `id`, `role`) into `out`;
 * gives the byte count and writes the Chunk count. */
static size_t send_all(ZmidiPe *pe, ZmidiCi *dev, uint32_t peer, uint8_t id, uint8_t role, const uint8_t *data,
                       size_t len, uint8_t *out, size_t cap, int *chunks) {
  size_t at = 0, sent = 0;
  *chunks = 0;
  do {
    size_t room = 0, part, k = 0;
    CHECK(zmidi_pe_data_room(pe, dev, peer, id, role, &room) == ZMIDI_STATUS_OK);
    part = len - sent < room ? len - sent : room;
    CHECK(zmidi_pe_send(pe, dev, peer, id, role, data + sent, part, sent + part == len, now, out + at, cap - at, &k) ==
          ZMIDI_STATUS_OK);
    sent += part;
    at += k;
    (*chunks)++;
  } while (sent < len && *chunks < 32);
  return at;
}

static ZmidiPeHeader header(const char *resource) {
  ZmidiPeHeader h;
  memset(&h, 0, sizeof h);
  h.resource = (const uint8_t *)resource;
  return h;
}

/* True when the `n` bytes at `p` contain the text `t`. */
static int has(const uint8_t *p, size_t n, const char *t) {
  size_t k = strlen(t), i;
  for (i = 0; i + k <= n; i++) {
    if (memcmp(p + i, t, k) == 0) {
      return 1;
    }
  }
  return 0;
}

/* M2-101 3.2: a MUID as 4 bytes of 7 bits, LSB first. */
static void muid_bytes(uint8_t *p, uint32_t m) {
  p[0] = (uint8_t)(m & 0x7F); p[1] = (uint8_t)((m >> 7) & 0x7F);
  p[2] = (uint8_t)((m >> 14) & 0x7F); p[3] = (uint8_t)((m >> 21) & 0x7F);
}

/* Invalidate MUID (M2-101 5.11) from `src` to Broadcast for `target`, as a record. Gives its length. */
static size_t invalidate(uint8_t *rec, uint32_t src, uint32_t target) {
  uint8_t *p = rec + 3;
  p[0] = 0x7E; p[1] = 0x7F; p[2] = 0x0D; p[3] = 0x7E; p[4] = 0x02;
  muid_bytes(p + 5, src);
  muid_bytes(p + 9, 0x0FFFFFFF);
  muid_bytes(p + 13, target);
  rec[0] = 0; rec[1] = 17; rec[2] = 0;
  return 3 + 17;
}

/* The length of the first record at `recs`. */
static size_t first_record(const uint8_t *recs) {
  return 3 + ((size_t)recs[1] | ((size_t)recs[2] << 8));
}

static ZmidiPeResource resource(const char *name) {
  ZmidiPeResource r;
  memset(&r, 0, sizeof r);
  r.resource = (const uint8_t *)name;
  r.can_get = true;
  r.encodings = 1u << ZMIDI_PE_ENCODING_ASCII;
  return r;
}

static ZmidiCiConfig config(void) {
  ZmidiCiConfig c;
  memset(&c, 0, sizeof c);
  c.manufacturer[0] = 0x00; c.manufacturer[1] = 0x21; c.manufacturer[2] = 0x7F;
  c.family = 1; c.model = 2; c.revision[2] = 1;
  c.max_sysex_size = 512;
  c.group_count = 1;
  c.channels_in_use = 1;
  c.property_exchange = true;
  return c;
}

static uint8_t big_schema[600];
/* Mcoded7 of the zlib CMF and FLG bytes 0x78 0x9C (window 32 KiB, RFC 1950 2.2). */
static const uint8_t zlib7[3] = {0x20, 0x78, 0x1C};

/* REQ-FFI-PE-001, -009, REQ-FFI-ABI-002: storage, init checks, null pointers. */
static void init_checks(const ZmidiPeResource *table, size_t count) {
  ZmidiPeResource bad[ZMIDI_PE_RESOURCES_MAX + 1];
  ZmidiPe *heap, moved;
  ZmidiPeEvent pev;
  size_t i, n = 0;
  uint16_t st = 0;

  CHECK(zmidi_pe_size() <= sizeof(ZmidiPe));
  CHECK(zmidi_abi_version() == ((uint32_t)ZMIDI_ABI_MAJOR << 16 | ZMIDI_ABI_MINOR) && ZMIDI_ABI_MINOR >= 3);
  /* DEC-019 rule 6: storage of exactly zmidi_pe_size() bytes is enough (ASan). */
  heap = (ZmidiPe *)malloc(zmidi_pe_size());
  CHECK(heap != NULL && zmidi_pe_init(heap, table, count) == ZMIDI_STATUS_OK);
  free(heap);

  CHECK(zmidi_pe_init(NULL, table, count) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_pe_init(&pb, NULL, 1) == ZMIDI_STATUS_INVALID_ARGUMENT);
  for (i = 0; i <= ZMIDI_PE_RESOURCES_MAX; i++) {
    bad[i] = resource("DeviceInfo");
  }
  CHECK(zmidi_pe_init(&pb, bad, ZMIDI_PE_RESOURCES_MAX + 1) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_pe_init(&pb, bad, ZMIDI_PE_RESOURCES_MAX) == ZMIDI_STATUS_OK);
  bad[0] = resource("Bad-Name"); /* M2-103 8.1: letters and digits only */
  CHECK(zmidi_pe_init(&pb, bad, 1) == ZMIDI_STATUS_INVALID_ARGUMENT);
  bad[0] = resource("X-");
  CHECK(zmidi_pe_init(&pb, bad, 1) == ZMIDI_STATUS_INVALID_ARGUMENT);
  bad[0] = resource("ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789A"); /* 37 characters */
  CHECK(zmidi_pe_init(&pb, bad, 1) == ZMIDI_STATUS_INVALID_ARGUMENT);
  bad[0] = resource("DeviceInfo");
  bad[0].encodings = 0;
  CHECK(zmidi_pe_init(&pb, bad, 1) == ZMIDI_STATUS_INVALID_ARGUMENT);
  bad[0] = resource("DeviceInfo");
  bad[0].can_set = 3;
  CHECK(zmidi_pe_init(&pb, bad, 1) == ZMIDI_STATUS_INVALID_ARGUMENT);
  bad[0] = resource("DeviceInfo");
  bad[0].media_type = (const uint8_t *)"json";
  CHECK(zmidi_pe_init(&pb, bad, 1) == ZMIDI_STATUS_INVALID_ARGUMENT);
  bad[0].media_type = (const uint8_t *)"application/octet-stream";
  CHECK(zmidi_pe_init(&pb, bad, 1) == ZMIDI_STATUS_OK);
  bad[0] = resource("DeviceInfo");
  bad[0].schema_len = 4; /* null schema with a length */
  CHECK(zmidi_pe_init(&pb, bad, 1) == ZMIDI_STATUS_INVALID_ARGUMENT);

  bad[0] = resource("DeviceInfo");
  bad[0].encodings = 8; /* no fourth encoding */
  CHECK(zmidi_pe_init(&pb, bad, 1) == ZMIDI_STATUS_INVALID_ARGUMENT);
  bad[0] = resource("DeviceInfo");
  bad[0].schema = big_schema; /* more than 512 bytes of ResourceList JSON */
  bad[0].schema_len = sizeof big_schema;
  CHECK(zmidi_pe_init(&pb, bad, 1) == ZMIDI_STATUS_INVALID_ARGUMENT);
  /* The entry fields reach the ResourceList (M2-103 Table 14, REQ-PE-LIST-009). */
  bad[0] = resource("X-One");
  bad[0].can_set = 1;
  bad[0].encodings = 1u << ZMIDI_PE_ENCODING_MCODED7;
  bad[0].media_type = (const uint8_t *)"application/octet-stream";
  bad[0].write_defaults = true;
  CHECK(zmidi_pe_init(&pb, bad, 1) == ZMIDI_STATUS_OK);
  CHECK(zmidi_pe_resource_list(&pb, body, sizeof body, &n) == ZMIDI_STATUS_OK);
  CHECK(has(body, n, "\"canGet\":true") && has(body, n, "\"canSet\":\"full\""));
  CHECK(has(body, n, "\"encodings\":[\"Mcoded7\"]"));
  CHECK(has(body, n, "\"mediaTypes\":[\"application/octet-stream\"]"));
  CHECK(zmidi_pe_init(&pb, table, count) == ZMIDI_STATUS_OK);
  /* A copied storage is refused: the engine points into its own storage. */
  memcpy(&moved, &pb, sizeof moved);
  CHECK(zmidi_pe_resource_list(&moved, body, sizeof body, &n) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_pe_last_error(&moved, &st) == ZMIDI_PE_ERROR_NONE);

  CHECK(zmidi_pe_receive(NULL, &b, now, scratch, sizeof scratch, &pev) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_pe_receive(&pb, NULL, now, scratch, sizeof scratch, &pev) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_pe_receive(&pb, &b, now, NULL, 4, &pev) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_pe_receive(&pb, &b, now, scratch, sizeof scratch, NULL) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_pe_poll(&pb, &b, now, NULL) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_pe_write_pending(&pb, &b, now, NULL, 0, &n) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_pe_write_pending(&pb, &b, now, msgs, sizeof msgs, NULL) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_pe_write_pending(&pb, &b, now, msgs, ZMIDI_PE_PENDING_MAX - 1, &n) == ZMIDI_STATUS_BUFFER_TOO_SMALL);
  CHECK(zmidi_pe_write_pending(&pb, &b, now, msgs, sizeof msgs, &n) == ZMIDI_STATUS_NO_PENDING);
  CHECK(zmidi_pe_resource_list(&pb, NULL, 4, &n) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_pe_subscription(&pb, 0, NULL) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_pe_take_ended(&pb, NULL) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_pe_data_room(&pb, &b, 1, 0, 0, NULL) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_pe_update(&pb, &b, 0, ZMIDI_PE_COMMAND_FULL, 0, 0, now, NULL) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_pe_last_error(NULL, NULL) == ZMIDI_PE_ERROR_NONE);
  /* No event in the ZmidiCi: no engine event. */
  CHECK(zmidi_pe_receive(&pb, &b, now, scratch, sizeof scratch, &pev) == ZMIDI_STATUS_OK);
  CHECK(pev.kind == ZMIDI_PE_EVENT_KIND_NONE);
}

int main(void) {
  ZmidiCiConfig c = config();
  ZmidiPeResource table[7];
  ZmidiPeHeader h;
  ZmidiPeSubscription sub;
  ZmidiPeEvent pev;
  size_t n = 0, rn = 0, i, list_n = 0;
  uint8_t id = 0, id2 = 0;
  bool missing = false;
  int chunks = 0;
  uint32_t ma, mb;
  uint16_t st = 0;
  int32_t index = -1;

  table[0] = resource("DeviceInfo");
  table[1] = resource("X-Big");
  table[1].can_set = 1;
  table[2] = resource("X-Sub");
  table[2].can_subscribe = true;
  table[3] = resource("X-Mc");
  table[3].encodings = (1u << ZMIDI_PE_ENCODING_ASCII) | (1u << ZMIDI_PE_ENCODING_MCODED7);
  table[3].schema = (const uint8_t *)"{\"type\":\"object\"}";
  table[3].schema_len = 17;
  table[4] = resource("X-ABCDEFGHIJKLMNOPQRSTUVWXYZ01234567"); /* 36 characters (M2-103 8.1) */
  table[5] = resource("X-Z");
  table[5].can_set = 1;
  table[5].encodings = (1u << ZMIDI_PE_ENCODING_ASCII) | (1u << ZMIDI_PE_ENCODING_ZLIB_MCODED7);
  table[6] = resource("X-Fc");
  table[6].can_set = 1;
  table[6].fc_on_set = true;

  CHECK(zmidi_ci_init(&a, &c, 1) == ZMIDI_STATUS_OK);
  CHECK(zmidi_ci_init(&b, &c, 2) == ZMIDI_STATUS_OK);
  init_checks(table, 7);
  CHECK(zmidi_pe_init(&pa, NULL, 0) == ZMIDI_STATUS_OK);
  ma = zmidi_ci_muid(&a);
  mb = zmidi_ci_muid(&b);

  /* Before Discovery and PE Capabilities: the device refuses (M2-101 8.4). */
  h = header("DeviceInfo");
  CHECK(zmidi_pe_request(&pa, &a, mb, 0, &h, NULL, now, &id, &missing) == ZMIDI_STATUS_REJECTED);
  CHECK(zmidi_pe_last_error(&pa, NULL) == ZMIDI_PE_ERROR_DEVICE);

  /* Discovery and PE Capabilities through the ci calls; the engines see no PE event. */
  CHECK(zmidi_ci_discovery(&a, 0, 0, msgs, sizeof msgs, &n) == ZMIDI_STATUS_OK);
  deliver(&b, &pb, msgs, n, replies, &rn);
  deliver(&a, &pa, replies, rn, msgs, &n);
  CHECK(ev.kind == ZMIDI_CI_EVENT_KIND_DISCOVERED && event_n == 0);
  CHECK(zmidi_ci_pe_capabilities_inquiry(&a, mb, msgs, sizeof msgs, &n) == ZMIDI_STATUS_OK);
  deliver(&b, &pb, msgs, n, replies, &rn);
  deliver(&a, &pa, replies, rn, msgs, &n);
  CHECK(ev.kind == ZMIDI_CI_EVENT_KIND_PE_CAPABILITIES && event_n == 0);

  /* REQ-FFI-PE-005: the ResourceList of B first (M2-103 9). */
  CHECK(zmidi_pe_request_resource_list(&pa, &a, mb, now, msgs, sizeof msgs, &n, &id) == ZMIDI_STATUS_OK);
  CHECK(n > 3 && msgs[0] == 0 && msgs[3 + 3] == 0x34);
  got_n = 0;
  deliver(&b, &pb, msgs, n, replies, &rn);
  CHECK(event_n == 1 && events[0].kind == ZMIDI_PE_EVENT_KIND_MESSAGE);
  CHECK(events[0].role == ZMIDI_PE_ROLE_RESPONDER && events[0].txn == ZMIDI_PE_TXN_KIND_GET);
  CHECK(events[0].muid == ma && events[0].request_id == id && events[0].end == ZMIDI_PE_CHUNK_END_COMPLETE);
  CHECK(strcmp((const char *)events[0].resource, "ResourceList") == 0 && events[0].resource_index == -1);
  CHECK(events[0].header_len > 0 && memcmp(events[0].header, "{\"resource\":\"ResourceList\"}", events[0].header_len) == 0);
  /* REQ-FFI-PE-004: B writes its ResourceList and replies with it. */
  CHECK(zmidi_pe_resource_list(&pb, body, 8, &list_n) == ZMIDI_STATUS_BUFFER_TOO_SMALL);
  CHECK(zmidi_pe_resource_list(&pb, body, sizeof body, &list_n) == ZMIDI_STATUS_OK);
  CHECK(list_n > 2 && body[0] == '[' && body[list_n - 1] == ']');
  body[list_n] = 0;
  CHECK(strstr((const char *)body, "\"resource\":\"DeviceInfo\"") != NULL);
  CHECK(strstr((const char *)body, "\"resource\":\"X-Sub\",\"canSubscribe\":true") != NULL);
  CHECK(strstr((const char *)body, "\"schema\":{\"type\":\"object\"}") != NULL);
  CHECK(zmidi_pe_reply(&pb, &b, ma, id, 200, ZMIDI_PE_ENCODING_ASCII, NULL, -1, -1, list_n, now) == ZMIDI_STATUS_OK);
  n = send_all(&pb, &b, ma, id, 1, body, list_n, msgs, sizeof msgs, &chunks);
  got_n = 0;
  deliver(&a, &pa, msgs, n, replies, &rn);
  CHECK(event_n == (size_t)chunks && events[event_n - 1].end == ZMIDI_PE_CHUNK_END_COMPLETE);
  CHECK(events[0].status == 200 && events[0].role == ZMIDI_PE_ROLE_INITIATOR);
  CHECK(got_n == list_n && memcmp(got, body, list_n) == 0);

  /* REQ-FFI-PE-002, -006, -010: a Get of DeviceInfo with a reply of many Chunks (M2-103 5.3). */
  h = header("DeviceInfo");
  CHECK(zmidi_pe_request(&pa, &a, mb, 0, &h, NULL, now, &id, &missing) == ZMIDI_STATUS_OK);
  CHECK(!missing); /* A has the ResourceList of B now */
  n = send_all(&pa, &a, mb, id, 0, NULL, 0, msgs, sizeof msgs, &chunks);
  CHECK(chunks == 1);
  deliver(&b, &pb, msgs, n, replies, &rn);
  CHECK(event_n == 1 && events[0].resource_index == 0 && strcmp((const char *)events[0].resource, "DeviceInfo") == 0);
  body[0] = '"';
  for (i = 1; i < 1199; i++) {
    body[i] = (uint8_t)('a' + i % 26);
  }
  body[1199] = '"';
  CHECK(zmidi_pe_reply(&pb, &b, ma, id, 200, ZMIDI_PE_ENCODING_ASCII, NULL, -1, -1, 1200, now) == ZMIDI_STATUS_OK);
  /* A second reply start is refused: the transaction sends now. */
  CHECK(zmidi_pe_reply(&pb, &b, ma, id, 200, ZMIDI_PE_ENCODING_ASCII, NULL, -1, -1, 1200, now) == ZMIDI_STATUS_REJECTED);
  CHECK(zmidi_pe_last_error(&pb, NULL) == ZMIDI_PE_ERROR_STATE);
  /* A Chunk larger than the room: buffer too small, nothing changes. */
  CHECK(zmidi_pe_send(&pb, &b, ma, id, 1, body, 1200, false, now, msgs, sizeof msgs, &n) == ZMIDI_STATUS_BUFFER_TOO_SMALL);
  CHECK(zmidi_pe_last_error(&pb, NULL) == ZMIDI_PE_ERROR_SIZE);
  CHECK(zmidi_pe_send(&pb, &b, ma, id, 1, body, 10, false, now, msgs, 8, &n) == ZMIDI_STATUS_BUFFER_TOO_SMALL);
  n = send_all(&pb, &b, ma, id, 1, body, 1200, msgs, sizeof msgs, &chunks);
  CHECK(chunks == 3);
  got_n = 0;
  deliver(&a, &pa, msgs, n, replies, &rn);
  CHECK(event_n == 3);
  CHECK(events[0].end == ZMIDI_PE_CHUNK_END_MORE && events[1].end == ZMIDI_PE_CHUNK_END_MORE);
  CHECK(events[2].end == ZMIDI_PE_CHUNK_END_COMPLETE);
  CHECK(events[0].header_len > 0 && events[1].header_len == 0 && events[0].status == 200);
  CHECK(got_n == 1200 && memcmp(got, body, 1200) == 0);
  /* The same ZmidiCi event is not given to the engine again. */
  CHECK(zmidi_pe_receive(&pa, &a, now, scratch, sizeof scratch, &pev) == ZMIDI_STATUS_OK);
  CHECK(pev.kind == ZMIDI_PE_EVENT_KIND_NONE);
  /* The transaction is closed: a send gives rejected. */
  CHECK(zmidi_pe_send(&pb, &b, ma, id, 1, NULL, 0, true, now, msgs, sizeof msgs, &n) == ZMIDI_STATUS_REJECTED);
  CHECK(zmidi_pe_last_error(&pb, NULL) == ZMIDI_PE_ERROR_UNKNOWN_TRANSACTION);

  /* A scratch buffer smaller than the Property Data of a Chunk: buffer too small, and the event
   * stays in the ZmidiCi for a second call. */
  h = header("DeviceInfo");
  CHECK(zmidi_pe_request(&pa, &a, mb, 0, &h, NULL, now, &id, &missing) == ZMIDI_STATUS_OK);
  n = send_all(&pa, &a, mb, id, 0, NULL, 0, msgs, sizeof msgs, &chunks);
  deliver(&b, &pb, msgs, n, replies, &rn);
  CHECK(zmidi_pe_reply(&pb, &b, ma, id, 200, ZMIDI_PE_ENCODING_ASCII, NULL, -1, -1, 100, now) == ZMIDI_STATUS_OK);
  n = send_all(&pb, &b, ma, id, 1, body, 100, msgs, sizeof msgs, &chunks);
  CHECK(zmidi_ci_receive(&a, 0, msgs + 3, n - 3, now, replies, sizeof replies, &rn, &ev) == ZMIDI_STATUS_OK);
  CHECK(zmidi_pe_receive(&pa, &a, now, scratch, 10, &pev) == ZMIDI_STATUS_BUFFER_TOO_SMALL);
  CHECK(pev.kind == ZMIDI_PE_EVENT_KIND_NONE && zmidi_pe_last_error(&pa, NULL) == ZMIDI_PE_ERROR_SIZE);
  CHECK(zmidi_pe_receive(&pa, &a, now, scratch, sizeof scratch, &pev) == ZMIDI_STATUS_OK);
  CHECK(pev.kind == ZMIDI_PE_EVENT_KIND_MESSAGE && pev.data_len == 100 && pev.end == ZMIDI_PE_CHUNK_END_COMPLETE);

  /* 404 for an unknown Resource (M2-103 8.6.2): the engine of B queues the status reply. */
  h = header("X-None");
  CHECK(zmidi_pe_request(&pa, &a, mb, 0, &h, NULL, now, &id, &missing) == ZMIDI_STATUS_OK);
  n = send_all(&pa, &a, mb, id, 0, NULL, 0, msgs, sizeof msgs, &chunks);
  deliver(&b, &pb, msgs, n, replies, &rn);
  CHECK(event_n == 0);
  n = pending(&pb, &b, msgs, sizeof msgs);
  CHECK(n > 3);
  deliver(&a, &pa, msgs, n, replies, &rn);
  CHECK(event_n == 1 && events[0].status == 404 && events[0].end == ZMIDI_PE_CHUNK_END_COMPLETE);

  /* REQ-FFI-CI-021: a request of the engine between zmidi_ci_receive and zmidi_pe_receive of the
   * last Chunk of a reply gets another Request ID; the engine then closes only the old request. */
  {
    uint8_t id1 = 0, id2 = 0;
    ZmidiPeEvent pev;
    h = header("X-None");
    CHECK(zmidi_pe_request(&pa, &a, mb, 0, &h, NULL, now, &id1, &missing) == ZMIDI_STATUS_OK);
    n = send_all(&pa, &a, mb, id1, 0, NULL, 0, msgs, sizeof msgs, &chunks);
    deliver(&b, &pb, msgs, n, replies, &rn);
    n = pending(&pb, &b, msgs, sizeof msgs);
    CHECK(n == first_record(msgs));
    CHECK(zmidi_ci_receive(&a, 0, msgs + 3, n - 3, now, replies, sizeof replies, &rn, &ev) == ZMIDI_STATUS_OK);
    CHECK(zmidi_pe_request(&pa, &a, mb, 0, &h, NULL, now, &id2, &missing) == ZMIDI_STATUS_OK && id2 != id1);
    CHECK(zmidi_pe_receive(&pa, &a, now, scratch, sizeof scratch, &pev) == ZMIDI_STATUS_OK);
    CHECK(pev.kind == ZMIDI_PE_EVENT_KIND_MESSAGE && pev.request_id == id1 && pev.status == 404);
    n = send_all(&pa, &a, mb, id2, 0, NULL, 0, msgs, sizeof msgs, &chunks);
    deliver(&b, &pb, msgs, n, replies, &rn);
    n = pending(&pb, &b, msgs, sizeof msgs);
    deliver(&a, &pa, msgs, n, replies, &rn);
    CHECK(event_n == 1 && events[0].request_id == id2 && events[0].status == 404);
  }

  /* REQ-FFI-PE-008: the capabilities of the peer refuse a Set of a Resource with canSet "none". */
  h = header("DeviceInfo");
  h.length = 2;
  CHECK(zmidi_pe_request(&pa, &a, mb, 1, &h, &table[0], now, &id, &missing) == ZMIDI_STATUS_REJECTED);
  CHECK(zmidi_pe_last_error(&pa, &st) == ZMIDI_PE_ERROR_REFUSED && st == 405);
  /* Values out of range. */
  CHECK(zmidi_pe_request(&pa, &a, mb, 2, &h, NULL, now, &id, &missing) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_pe_request(&pa, &a, 0x10000000u, 0, &h, NULL, now, &id, &missing) == ZMIDI_STATUS_INVALID_ARGUMENT);
  h.encoding = 3;
  CHECK(zmidi_pe_request(&pa, &a, mb, 0, &h, NULL, now, &id, &missing) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_pe_request(&pa, &a, mb, 0, NULL, NULL, now, &id, &missing) == ZMIDI_STATUS_INVALID_ARGUMENT);

  /* A Set of 700 bytes in two Chunks to X-Big; B replies 200 with no data. */
  h = header("X-Big");
  h.length = 700;
  CHECK(zmidi_pe_request(&pa, &a, mb, 1, &h, &table[1], now, &id, &missing) == ZMIDI_STATUS_OK);
  n = send_all(&pa, &a, mb, id, 0, body, 700, msgs, sizeof msgs, &chunks);
  CHECK(chunks == 2);
  got_n = 0;
  deliver(&b, &pb, msgs, n, replies, &rn);
  CHECK(event_n == 2 && events[0].txn == ZMIDI_PE_TXN_KIND_SET && events[0].resource_index == 1);
  CHECK(events[1].end == ZMIDI_PE_CHUNK_END_COMPLETE && got_n == 700 && memcmp(got, body, 700) == 0);
  CHECK(zmidi_pe_reply(&pb, &b, ma, id, 200, ZMIDI_PE_ENCODING_ASCII, NULL, -1, -1, 0, now) == ZMIDI_STATUS_OK);
  n = send_all(&pb, &b, ma, id, 1, NULL, 0, msgs, sizeof msgs, &chunks);
  deliver(&a, &pa, msgs, n, replies, &rn);
  CHECK(event_n == 1 && events[0].txn == ZMIDI_PE_TXN_KIND_SET && events[0].status == 200);

  /* REQ-FFI-PE-004, -005, -007: a Subscription "start" to X-Sub, a "full" update, an "end". */
  h = header("X-Sub");
  CHECK(zmidi_pe_subscribe(&pa, &a, mb, &h, &table[2], now, &id, &missing) == ZMIDI_STATUS_OK);
  n = send_all(&pa, &a, mb, id, 0, NULL, 0, msgs, sizeof msgs, &chunks);
  deliver(&b, &pb, msgs, n, replies, &rn);
  for (i = 0; i < event_n; i++) {
    if (events[i].kind == ZMIDI_PE_EVENT_KIND_SUBSCRIPTION_STARTED) {
      index = events[i].subscription;
    }
  }
  CHECK(index >= 0);
  CHECK(zmidi_pe_subscription(&pb, (size_t)index, &sub) == ZMIDI_STATUS_OK);
  CHECK(sub.muid == ma && sub.role == ZMIDI_PE_ROLE_RESPONDER && strcmp((const char *)sub.resource, "X-Sub") == 0);
  CHECK(sub.subscribe_id[0] != 0);
  n = pending(&pb, &b, msgs, sizeof msgs);
  deliver(&a, &pa, msgs, n, replies, &rn);
  CHECK(event_n == 1 && events[0].txn == ZMIDI_PE_TXN_KIND_SUBSCRIPTION && events[0].status == 200);
  CHECK(events[0].started >= 0 && events[0].start_error == ZMIDI_PE_SUBSCRIPTION_ERROR_NONE);
  CHECK(strcmp((const char *)events[0].subscribe_id, (const char *)sub.subscribe_id) == 0);
  CHECK(zmidi_pe_subscription(&pa, (size_t)events[0].started, &sub) == ZMIDI_STATUS_OK);
  CHECK(sub.muid == mb && sub.role == ZMIDI_PE_ROLE_INITIATOR && strcmp((const char *)sub.resource, "X-Sub") == 0);
  CHECK(zmidi_pe_subscription(&pa, 3, &sub) == ZMIDI_STATUS_NO_PENDING);
  CHECK(zmidi_pe_subscription(&pa, 99, &sub) == ZMIDI_STATUS_NO_PENDING);
  /* B sends a "full" update with 20 bytes; A gets the command and its subscription entry. */
  CHECK(zmidi_pe_update(&pb, &b, (size_t)index, ZMIDI_PE_COMMAND_START, 0, 20, now, &id2) == ZMIDI_STATUS_INVALID_ARGUMENT);
  /* B sends Subscription messages as initiator of each one: it needs the PE Capabilities of A first
   * (M2-101 8.4, the Number of Simultaneous Requests). */
  CHECK(zmidi_pe_update(&pb, &b, (size_t)index, ZMIDI_PE_COMMAND_FULL, 0, 20, now, &id2) == ZMIDI_STATUS_REJECTED);
  CHECK(zmidi_pe_last_error(&pb, NULL) == ZMIDI_PE_ERROR_DEVICE);
  CHECK(zmidi_ci_discovery(&b, 0, 0, msgs, sizeof msgs, &n) == ZMIDI_STATUS_OK);
  deliver(&a, &pa, msgs, n, replies, &rn);
  deliver(&b, &pb, replies, rn, msgs, &n);
  CHECK(zmidi_ci_pe_capabilities_inquiry(&b, ma, msgs, sizeof msgs, &n) == ZMIDI_STATUS_OK);
  deliver(&a, &pa, msgs, n, replies, &rn);
  deliver(&b, &pb, replies, rn, msgs, &n);
  CHECK(ev.kind == ZMIDI_CI_EVENT_KIND_PE_CAPABILITIES);
  CHECK(zmidi_pe_update(&pb, &b, (size_t)index, ZMIDI_PE_COMMAND_FULL, 0, 20, now, &id2) == ZMIDI_STATUS_OK);
  n = send_all(&pb, &b, ma, id2, 0, body, 20, msgs, sizeof msgs, &chunks);
  got_n = 0;
  deliver(&a, &pa, msgs, n, replies, &rn);
  CHECK(event_n == 1 && events[0].command == ZMIDI_PE_COMMAND_FULL && events[0].role == ZMIDI_PE_ROLE_RESPONDER);
  CHECK(events[0].subscription >= 0 && got_n == 20 && memcmp(got, body, 20) == 0);
  /* A answers the update with 200 (queued by its engine). */
  n = pending(&pa, &a, msgs, sizeof msgs);
  deliver(&b, &pb, msgs, n, replies, &rn);
  CHECK(event_n == 1 && events[0].status == 200 && events[0].role == ZMIDI_PE_ROLE_INITIATOR);
  /* REQ-FFI-PE-005: refresh queues a Get of X-Sub. */
  CHECK(zmidi_pe_refresh(&pa, 99) == ZMIDI_STATUS_REJECTED);
  CHECK(zmidi_pe_refresh(&pa, (size_t)events[0].subscription >= 4 ? 0 : 0) == ZMIDI_STATUS_OK);
  n = pending(&pa, &a, msgs, sizeof msgs);
  deliver(&b, &pb, msgs, n, replies, &rn);
  CHECK(event_n == 1 && events[0].txn == ZMIDI_PE_TXN_KIND_GET && strcmp((const char *)events[0].resource, "X-Sub") == 0);
  CHECK(zmidi_pe_reply(&pb, &b, ma, events[0].request_id, 200, 0, NULL, -1, -1, 2, now) == ZMIDI_STATUS_OK);
  n = send_all(&pb, &b, ma, events[0].request_id, 1, (const uint8_t *)"{}", 2, msgs, sizeof msgs, &chunks);
  deliver(&a, &pa, msgs, n, replies, &rn);
  CHECK(event_n == 1 && events[0].subscription == 0 && events[0].status == 200);
  /* A ends the subscription; B gets SubscriptionEnded with the entry. */
  CHECK(zmidi_pe_unsubscribe(&pa, &a, 0, now, &id) == ZMIDI_STATUS_OK);
  CHECK(zmidi_pe_subscription(&pa, 0, &sub) == ZMIDI_STATUS_NO_PENDING);
  n = send_all(&pa, &a, mb, id, 0, NULL, 0, msgs, sizeof msgs, &chunks);
  deliver(&b, &pb, msgs, n, replies, &rn);
  CHECK(event_n >= 1 && events[event_n - 1].kind == ZMIDI_PE_EVENT_KIND_SUBSCRIPTION_ENDED);
  CHECK(events[event_n - 1].muid == ma && strcmp((const char *)events[event_n - 1].resource, "X-Sub") == 0);
  CHECK(zmidi_pe_subscription(&pb, (size_t)index, &sub) == ZMIDI_STATUS_NO_PENDING);
  n = pending(&pb, &b, msgs, sizeof msgs);
  deliver(&a, &pa, msgs, n, replies, &rn);
  CHECK(zmidi_pe_take_ended(&pa, &sub) == ZMIDI_STATUS_NO_PENDING);

  /* REQ-FFI-PE-006: wait (ACK 0x10) and abort (NAK 0x20). B waits on a Get, then ends it. */
  h = header("DeviceInfo");
  CHECK(zmidi_pe_request(&pa, &a, mb, 0, &h, NULL, now, &id, &missing) == ZMIDI_STATUS_OK);
  n = send_all(&pa, &a, mb, id, 0, NULL, 0, msgs, sizeof msgs, &chunks);
  deliver(&b, &pb, msgs, n, replies, &rn);
  CHECK(zmidi_pe_wait(&pb, ma, id, 1, 0) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_pe_last_error(&pb, NULL) == ZMIDI_PE_ERROR_INVALID_WAIT);
  CHECK(zmidi_pe_wait(&pb, ma, id, 1, 12701) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_pe_wait(&pb, ma, id, 2, 500) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_pe_wait(&pb, ma, id, 1, 500) == ZMIDI_STATUS_OK);
  n = pending(&pb, &b, msgs, sizeof msgs);
  CHECK(n > 3 && msgs[3 + 3] == 0x7D); /* MIDI-CI ACK */
  deliver(&a, &pa, msgs, n, replies, &rn);
  CHECK(zmidi_pe_corrupt(&pb, ma, id, 1) == ZMIDI_STATUS_REJECTED); /* no Flow Control */
  CHECK(zmidi_pe_abort(&pb, &b, ma, id, 1) == ZMIDI_STATUS_OK);
  CHECK(zmidi_pe_abort(&pb, &b, ma, id, 1) == ZMIDI_STATUS_REJECTED);
  n = pending(&pb, &b, msgs, sizeof msgs);
  CHECK(n > 3 && msgs[3 + 3] == 0x7F); /* MIDI-CI NAK */
  deliver(&a, &pa, msgs, n, replies, &rn);
  CHECK(event_n == 1 && events[0].kind == ZMIDI_PE_EVENT_KIND_CLOSED && events[0].reason == ZMIDI_PE_CLOSE_REASON_TERMINATED);
  CHECK(events[0].request_id == id && events[0].muid == mb);

  /* REQ-FFI-PE-003: a timeout (M2-103 12: 3 seconds with no reply). A gets Closed with Timeout and
   * queues NAK 0x42 to B. */
  h = header("DeviceInfo");
  CHECK(zmidi_pe_request(&pa, &a, mb, 0, &h, NULL, now, &id, &missing) == ZMIDI_STATUS_OK);
  n = send_all(&pa, &a, mb, id, 0, NULL, 0, msgs, sizeof msgs, &chunks);
  CHECK(zmidi_pe_poll(&pa, &a, now + 1000000, &pev) == ZMIDI_STATUS_OK && pev.kind == ZMIDI_PE_EVENT_KIND_NONE);
  CHECK(zmidi_pe_poll(&pa, &a, now + 3001000 + 1000, &pev) == ZMIDI_STATUS_OK);
  CHECK(pev.kind == ZMIDI_PE_EVENT_KIND_CLOSED && pev.reason == ZMIDI_PE_CLOSE_REASON_TIMEOUT && pev.request_id == id);
  CHECK(zmidi_pe_poll(&pa, &a, now + 3001000 + 1000, &pev) == ZMIDI_STATUS_OK && pev.kind == ZMIDI_PE_EVENT_KIND_NONE);
  n = pending(&pa, &a, msgs, sizeof msgs);
  CHECK(n > 3 && msgs[3 + 3] == 0x7F && msgs[3 + 13] == 0x34 && msgs[3 + 14] == 0x42); /* NAK of a Get, Status Code 0x42 (M2-101 Table 16) */

  /* A reply after the deadline: rejected, and the transaction closes (M2-103 12). */
  h = header("DeviceInfo");
  CHECK(zmidi_pe_request(&pa, &a, mb, 0, &h, NULL, now, &id, &missing) == ZMIDI_STATUS_OK);
  n = send_all(&pa, &a, mb, id, 0, NULL, 0, msgs, sizeof msgs, &chunks);
  deliver(&b, &pb, msgs, n, replies, &rn);
  CHECK(zmidi_pe_reply(&pb, &b, ma, id, 200, 0, NULL, -1, -1, 2, now + 4000000) == ZMIDI_STATUS_REJECTED);
  CHECK(zmidi_pe_last_error(&pb, NULL) == ZMIDI_PE_ERROR_LATE);
  CHECK(zmidi_pe_reply_status(&pb, &b, ma, id, 404, now) == ZMIDI_STATUS_REJECTED);
  CHECK(zmidi_pe_last_error(&pb, NULL) == ZMIDI_PE_ERROR_UNKNOWN_TRANSACTION);

  /* reply_status: B answers 413 with no data; A gets the status. */
  h = header("DeviceInfo");
  CHECK(zmidi_pe_request(&pa, &a, mb, 0, &h, NULL, now, &id2, &missing) == ZMIDI_STATUS_OK);
  n = send_all(&pa, &a, mb, id2, 0, NULL, 0, msgs, sizeof msgs, &chunks);
  deliver(&b, &pb, msgs, n, replies, &rn);
  CHECK(zmidi_pe_reply_status(&pb, &b, ma, id2, 413, now) == ZMIDI_STATUS_OK);
  n = pending(&pb, &b, msgs, sizeof msgs);
  deliver(&a, &pa, msgs, n, replies, &rn);
  CHECK(event_n == 1 && events[0].status == 413);
  /* The transaction of the late reply above is still open in A: it times out. */
  CHECK(zmidi_pe_poll(&pa, &a, now + 4000000, &pev) == ZMIDI_STATUS_OK && pev.kind == ZMIDI_PE_EVENT_KIND_CLOSED);
  n = pending(&pa, &a, msgs, sizeof msgs);

  /* REQ-FFI-PE-008: the reason of a refusal is kept until the next engine call. */
  CHECK(zmidi_pe_refresh(&pa, 99) == ZMIDI_STATUS_REJECTED);
  CHECK(zmidi_pe_last_error(&pa, NULL) == ZMIDI_PE_ERROR_UNKNOWN_TRANSACTION);
  CHECK(zmidi_pe_resource_list(&pa, body, sizeof body, &n) == ZMIDI_STATUS_OK);
  CHECK(zmidi_pe_last_error(&pa, NULL) == ZMIDI_PE_ERROR_NONE);
  h = header("DeviceInfo");
  h.length = 2;
  CHECK(zmidi_pe_request(&pa, &a, mb, 1, &h, &table[0], now, &id, &missing) == ZMIDI_STATUS_REJECTED);
  CHECK(zmidi_pe_refresh(&pa, 99) == ZMIDI_STATUS_REJECTED);
  st = 7;
  CHECK(zmidi_pe_last_error(&pa, &st) == ZMIDI_PE_ERROR_UNKNOWN_TRANSACTION && st == 0);
  /* totalCount and cacheTime are 32-bit values or -1. */
  CHECK(zmidi_pe_reply(&pb, &b, ma, 1, 200, 0, NULL, 0x100000000LL, -1, 2, now) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_pe_reply(&pb, &b, ma, 1, 200, 0, NULL, -1, -2, 2, now) == ZMIDI_STATUS_INVALID_ARGUMENT);

  /* REQ-FFI-PE-010: a Get in Mcoded7 (M2-103 8.6.1, "mutualEncoding") of a 36-character Resource. */
  h = header("X-Mc");
  h.encoding = ZMIDI_PE_ENCODING_MCODED7;
  CHECK(zmidi_pe_request(&pa, &a, mb, 0, &h, NULL, now, &id, &missing) == ZMIDI_STATUS_OK);
  n = send_all(&pa, &a, mb, id, 0, NULL, 0, msgs, sizeof msgs, &chunks);
  CHECK(has(msgs, n, "\"mutualEncoding\":\"Mcoded7\""));
  deliver(&b, &pb, msgs, n, replies, &rn);
  CHECK(event_n == 1 && events[0].encoding == ZMIDI_PE_ENCODING_MCODED7 && events[0].resource_index == 3);
  CHECK(zmidi_pe_reply_status(&pb, &b, ma, id, 404, now) == ZMIDI_STATUS_OK);
  deliver(&a, &pa, msgs, pending(&pb, &b, msgs, sizeof msgs), replies, &rn);
  h = header("X-ABCDEFGHIJKLMNOPQRSTUVWXYZ01234567");
  CHECK(zmidi_pe_request(&pa, &a, mb, 0, &h, NULL, now, &id, &missing) == ZMIDI_STATUS_OK);
  n = send_all(&pa, &a, mb, id, 0, NULL, 0, msgs, sizeof msgs, &chunks);
  deliver(&b, &pb, msgs, n, replies, &rn);
  CHECK(event_n == 1 && events[0].resource_index == 4 && strlen((const char *)events[0].resource) == 36);
  CHECK(zmidi_pe_reply_status(&pb, &b, ma, id, 404, now) == ZMIDI_STATUS_OK);
  deliver(&a, &pa, msgs, pending(&pb, &b, msgs, sizeof msgs), replies, &rn);

  /* The reader takes a "resId" of up to 36 characters of any kind (M2-103 8.2; the writer of this
   * library sends letters, digits and "_" only, REQ-PE-HDR-007). A peer sends "a" and 18 x "\\u00e9"
   * (M2-103 6.1.1: SysEx data has 7 bits): 37 bytes of UTF-8 after the decode. The event gives it cut at a character boundary, with its NUL in the 37 bytes
   * of ZMIDI_PE_ID_LEN. The Get is M2-101 Table 33 with Request ID 0x55, one Chunk, no data. */
  {
    uint8_t get[256], *p = get + 3;
    char hd[200];
    size_t k, hl, at;
    strcpy(hd, "{\"resource\":\"DeviceInfo\",\"resId\":\"a");
    hl = strlen(hd);
    for (k = 0; k < 18; k++) {
      memcpy(hd + hl, "\\u00e9", 6);
      hl += 6;
    }
    hd[hl++] = '"';
    hd[hl++] = '}';
    p[0] = 0x7E; p[1] = 0x7F; p[2] = 0x0D; p[3] = 0x34; p[4] = 0x02;
    muid_bytes(p + 5, ma);
    muid_bytes(p + 9, mb);
    p[13] = 0x55;
    p[14] = (uint8_t)(hl & 0x7F); p[15] = (uint8_t)(hl >> 7);
    memcpy(p + 16, hd, hl);
    at = 16 + hl;
    p[at] = 1; p[at + 1] = 0; p[at + 2] = 1; p[at + 3] = 0; p[at + 4] = 0; p[at + 5] = 0;
    get[0] = 0; get[1] = (uint8_t)(at + 6); get[2] = 0;
    deliver(&b, &pb, get, 3 + at + 6, replies, &rn);
    CHECK(event_n == 1 && events[0].request_id == 0x55 && memchr(events[0].res_id, 0, ZMIDI_PE_ID_LEN) != NULL);
    CHECK(strlen((const char *)events[0].res_id) == 35 && events[0].res_id[0] == 'a');
    for (k = 0; k < 17; k++) {
      CHECK(events[0].res_id[1 + 2 * k] == 0xC3 && events[0].res_id[2 + 2 * k] == 0xA9);
    }
    CHECK(zmidi_pe_reply_status(&pb, &b, ma, 0x55, 404, now) == ZMIDI_STATUS_OK);
    n = pending(&pb, &b, msgs, sizeof msgs);
  }

  /* The request Properties reach the header: "offset", "limit", "setPartial", and the encoding of
   * a "start" (M2-103 8.5, 8.6.1, 11). */
  h = header("DeviceInfo");
  h.has_offset = true;
  h.offset = 5;
  h.limit = 10;
  CHECK(zmidi_pe_request(&pa, &a, mb, 0, &h, NULL, now, &id, &missing) == ZMIDI_STATUS_OK);
  n = send_all(&pa, &a, mb, id, 0, NULL, 0, msgs, sizeof msgs, &chunks);
  CHECK(has(msgs, n, "\"offset\":5") && has(msgs, n, "\"limit\":10"));
  CHECK(zmidi_pe_abort(&pa, &a, mb, id, 0) == ZMIDI_STATUS_OK);
  n = pending(&pa, &a, msgs, sizeof msgs);
  /* The NAK 0x20 of an Initiator has Status Data 0x01 (M2-101 Table 16, M2-103 12). */
  CHECK(n > 3 && msgs[3 + 14] == 0x20 && msgs[3 + 15] == 0x01);
  h = header("X-Big");
  h.set_partial = true;
  h.length = 2;
  CHECK(zmidi_pe_request(&pa, &a, mb, 1, &h, NULL, now, &id, &missing) == ZMIDI_STATUS_OK);
  n = send_all(&pa, &a, mb, id, 0, (const uint8_t *)"{}", 2, msgs, sizeof msgs, &chunks);
  CHECK(has(msgs, n, "\"setPartial\":true"));
  CHECK(zmidi_pe_abort(&pa, &a, mb, id, 0) == ZMIDI_STATUS_OK);
  n = pending(&pa, &a, msgs, sizeof msgs);
  h = header("X-Sub");
  h.encoding = ZMIDI_PE_ENCODING_MCODED7;
  CHECK(zmidi_pe_subscribe(&pa, &a, mb, &h, NULL, now, &id, &missing) == ZMIDI_STATUS_OK);
  n = send_all(&pa, &a, mb, id, 0, NULL, 0, msgs, sizeof msgs, &chunks);
  CHECK(has(msgs, n, "\"mutualEncoding\":\"Mcoded7\""));
  CHECK(zmidi_pe_abort(&pa, &a, mb, id, 0) == ZMIDI_STATUS_OK);
  n = pending(&pa, &a, msgs, sizeof msgs);

  /* REQ-PE-SUB-006: a "start" for a Resource with canSubscribe false gets 405 and no subscription. */
  h = header("DeviceInfo");
  CHECK(zmidi_pe_subscribe(&pa, &a, mb, &h, NULL, now, &id, &missing) == ZMIDI_STATUS_OK);
  n = send_all(&pa, &a, mb, id, 0, NULL, 0, msgs, sizeof msgs, &chunks);
  deliver(&b, &pb, msgs, n, replies, &rn);
  deliver(&a, &pa, msgs, pending(&pb, &b, msgs, sizeof msgs), replies, &rn);
  CHECK(event_n == 1 && events[0].status == 405 && events[0].started == -1);
  CHECK(events[0].start_error == ZMIDI_PE_SUBSCRIPTION_ERROR_NOT_SUCCESS);

  /* REQ-FFI-PE-010: data that this Device can not take closes the transaction (REQ-PE-STAT-011,
   * -013): A asks for a Get in zlib+Mcoded7, and the zlib window of the reply is over the 0 bytes of
   * the engine. A zlib Set to B is refused before a transaction (415). */
  h = header("X-Z");
  h.encoding = ZMIDI_PE_ENCODING_ZLIB_MCODED7;
  h.length = 3;
  CHECK(zmidi_pe_request(&pa, &a, mb, 1, &h, &table[5], now, &id, &missing) == ZMIDI_STATUS_OK);
  deliver(&b, &pb, msgs, send_all(&pa, &a, mb, id, 0, zlib7, sizeof zlib7, msgs, sizeof msgs, &chunks), replies, &rn);
  CHECK(event_n == 0);
  deliver(&a, &pa, msgs, pending(&pb, &b, msgs, sizeof msgs), replies, &rn);
  CHECK(event_n == 1 && events[0].status == 415);
  h = header("X-Z");
  h.encoding = ZMIDI_PE_ENCODING_ZLIB_MCODED7;
  CHECK(zmidi_pe_request(&pa, &a, mb, 0, &h, &table[5], now, &id, &missing) == ZMIDI_STATUS_OK);
  deliver(&b, &pb, msgs, send_all(&pa, &a, mb, id, 0, NULL, 0, msgs, sizeof msgs, &chunks), replies, &rn);
  CHECK(event_n == 1 && events[0].encoding == ZMIDI_PE_ENCODING_ZLIB_MCODED7);
  CHECK(zmidi_pe_reply(&pb, &b, ma, id, 200, ZMIDI_PE_ENCODING_ZLIB_MCODED7, NULL, -1, -1, sizeof zlib7, now) == ZMIDI_STATUS_OK);
  deliver(&a, &pa, msgs, send_all(&pb, &b, ma, id, 1, zlib7, sizeof zlib7, msgs, sizeof msgs, &chunks), replies, &rn);
  CHECK(event_n == 1 && events[0].kind == ZMIDI_PE_EVENT_KIND_CLOSED);
  CHECK(events[0].reason == ZMIDI_PE_CLOSE_REASON_REFUSED && events[0].status == 413);
  n = pending(&pa, &a, msgs, sizeof msgs);
  CHECK(n > 3 && msgs[3 + 14] == 0x20 && msgs[3 + 15] == 0x01); /* NAK 0x20 of the Initiator */

  /* A NAK other than 0x12, 0x20 and 0x42 closes the transaction with its code and data (M2-101 Table 16). */
  h = header("DeviceInfo");
  CHECK(zmidi_pe_request(&pa, &a, mb, 0, &h, NULL, now, &id, &missing) == ZMIDI_STATUS_OK);
  n = send_all(&pa, &a, mb, id, 0, NULL, 0, msgs, sizeof msgs, &chunks);
  deliver(&b, &pb, msgs, n, replies, &rn);
  CHECK(zmidi_pe_abort(&pb, &b, ma, id, 1) == ZMIDI_STATUS_OK);
  n = pending(&pb, &b, msgs, sizeof msgs);
  CHECK(n > 3 && msgs[3 + 14] == 0x20 && msgs[3 + 15] == 0x00); /* a Responder: Status Data 0x00 */
  msgs[3 + 14] = 0x43; /* Busy, retry after 0x05 x 100 ms */
  msgs[3 + 15] = 0x05;
  deliver(&a, &pa, msgs, n, replies, &rn);
  CHECK(event_n == 1 && events[0].reason == ZMIDI_PE_CLOSE_REASON_NAK && events[0].nak_code == 0x43 && events[0].nak_data == 0x05);

  /* Flow Control (M2-103 7.4, REQ-PE-FLOW-013): a Chunk with no ACK in 3 seconds is sent again. */
  h = header("X-Fc");
  h.length = 700;
  CHECK(zmidi_pe_request(&pa, &a, mb, 1, &h, &table[6], now, &id, &missing) == ZMIDI_STATUS_OK);
  {
    size_t room = 0;
    CHECK(zmidi_pe_data_room(&pa, &a, mb, id, 0, &room) == ZMIDI_STATUS_OK && room < 700);
    CHECK(zmidi_pe_send(&pa, &a, mb, id, 0, big_schema, room, false, now, msgs, sizeof msgs, &n) == ZMIDI_STATUS_OK);
    CHECK(has(msgs, n, "\"flowControl\":true"));
  }
  CHECK(zmidi_pe_poll(&pa, &a, now + 3002000, &pev) == ZMIDI_STATUS_OK);
  CHECK(pev.kind == ZMIDI_PE_EVENT_KIND_RESEND && pev.chunk == 1 && pev.request_id == id);
  CHECK(zmidi_pe_abort(&pa, &a, mb, id, 0) == ZMIDI_STATUS_OK);
  n = pending(&pa, &a, msgs, sizeof msgs);

  /* A reply of unknown length has 0 in Number of Chunks in Message until its last Chunk (M2-101
   * Table 33); the same ZmidiCi event is not given to the engine twice. */
  for (i = 0; i < 2; i++) {
    size_t first;
    h = header("DeviceInfo");
    CHECK(zmidi_pe_request(&pa, &a, mb, 0, &h, NULL, now, &id, &missing) == ZMIDI_STATUS_OK);
    deliver(&b, &pb, msgs, send_all(&pa, &a, mb, id, 0, NULL, 0, msgs, sizeof msgs, &chunks), replies, &rn);
    CHECK(zmidi_pe_reply(&pb, &b, ma, id, 200, 0, NULL, -1, -1, i == 0 ? 1200 : SIZE_MAX, now) == ZMIDI_STATUS_OK);
    n = send_all(&pb, &b, ma, id, 1, body, 1200, msgs, sizeof msgs, &chunks);
    first = first_record(msgs);
    got_n = 0;
    deliver(&a, &pa, msgs, first, replies, &rn);
    CHECK(ev.chunk_total == (i == 0 ? 3 : 0) && ev.chunk_number == 1);
    CHECK(zmidi_pe_receive(&pa, &a, now, scratch, sizeof scratch, &pev) == ZMIDI_STATUS_OK && pev.kind == ZMIDI_PE_EVENT_KIND_NONE);
    deliver(&a, &pa, msgs + first, n - first, replies, &rn);
    CHECK(event_n == 2 && events[1].end == ZMIDI_PE_CHUNK_END_COMPLETE && got_n == 1200);
    CHECK(pending(&pa, &a, msgs, sizeof msgs) == 0);
  }

  /* Invalidate MUID (M2-101 5.11): of the peer A at B, then of B itself (a new MUID). The
   * subscriptions of A wait in take_ended (REQ-PE-SUB-019). */
  h = header("X-Sub");
  CHECK(zmidi_pe_subscribe(&pa, &a, mb, &h, &table[2], now, &id, &missing) == ZMIDI_STATUS_OK);
  deliver(&b, &pb, msgs, send_all(&pa, &a, mb, id, 0, NULL, 0, msgs, sizeof msgs, &chunks), replies, &rn);
  deliver(&a, &pa, msgs, pending(&pb, &b, msgs, sizeof msgs), replies, &rn);
  deliver(&b, &pb, msgs, invalidate(msgs, 0x0123456, ma), replies, &rn);
  CHECK(event_n == 1 && events[0].kind == ZMIDI_PE_EVENT_KIND_INVALIDATED && events[0].muid == ma && events[0].ended == 1);
  CHECK(zmidi_pe_take_ended(&pb, &sub) == ZMIDI_STATUS_OK && sub.muid == ma && sub.role == ZMIDI_PE_ROLE_RESPONDER);
  CHECK(zmidi_pe_take_ended(&pb, &sub) == ZMIDI_STATUS_NO_PENDING);
  deliver(&b, &pb, msgs, invalidate(msgs, 0x0123456, mb), replies, &rn);
  CHECK(event_n == 1 && events[0].kind == ZMIDI_PE_EVENT_KIND_INVALIDATED && events[0].muid == mb);
  CHECK(zmidi_ci_muid(&b) != mb);

  if (failures != 0) {
    fprintf(stderr, "pe: %d failures\n", failures);
    return 1;
  }
  printf("pe: ok\n");
  return 0;
}
