/* C test of the net part of the C ABI (REQ-FFI-NET-005): a Host and a Client in memory, with no
 * socket (M2-124: Invitation, Invitation Reply, UMP Data, Ping, Bye). Each payload that one side
 * writes goes to the other side.
 *
 * With the feature net-auth (REQ-FFI-NET-009): Invitation with Authentication (M2-124 6.9) and
 * with User Authentication (6.10) with the CryptoNonces and digests of the specification
 * examples, a wrong password and an unknown Username that fail. */
#ifndef ZMIDI_NET
#define ZMIDI_NET
#endif
#ifndef ZMIDI_NET_AUTH
#define ZMIDI_NET_AUTH
#endif
#include "zmidi.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int failures;
#define CHECK(c) do { if (!(c)) { fprintf(stderr, "%s:%d: CHECK(%s)\n", __FILE__, __LINE__, #c); failures++; } } while (0)

static ZmidiNetHost host;
static ZmidiNetClient client;
static ZmidiNetAddr host_addr, client_addr;
static ZmidiNetEvent events[64];
static uint32_t words[512];
static uint8_t out[4096], payload[ZMIDI_NET_MAX_PAYLOAD];
static uint64_t now;

/* Events seen on each side, by kind, with the code and the value of the last one of each kind. */
static int host_seen[32], client_seen[32];
static uint32_t host_code[32], client_code[32];
static uint64_t host_value[32], client_value[32];
static uint32_t host_words[16];
static size_t host_word_count;
static uint32_t ping_id_seen;

/* The credentials of each side; NULL: the receive calls with no credentials. */
static ZmidiNetCredentials *host_creds, *client_creds;
/* True when a payload of the Client (or of the Host) held these bytes. */
static const uint8_t *client_expect, *host_expect;
static size_t client_expect_len, host_expect_len;
static bool client_sent_expected, host_sent_expected;
/* True for the authentication tests: pump() also runs the timers (failure delay). */
static bool step_deadlines;

static void reset_seen(void) {
  memset(host_seen, 0, sizeof host_seen); memset(client_seen, 0, sizeof client_seen);
  memset(host_code, 0, sizeof host_code); memset(client_code, 0, sizeof client_code);
  memset(host_value, 0, sizeof host_value); memset(client_value, 0, sizeof client_value);
  client_sent_expected = host_sent_expected = false;
}

static void buffers(ZmidiNetBuffers *b) {
  memset(b, 0, sizeof *b);
  b->events = events; b->events_capacity = 64;
  b->words = words; b->words_capacity = 512;
  b->out = out; b->out_capacity = sizeof out;
}

static void note(bool at_host, const ZmidiNetBuffers *b) {
  size_t i, k;
  for (i = 0; i < b->events_len; i++) {
    const ZmidiNetEvent *e = &events[i];
    (at_host ? host_seen : client_seen)[e->kind]++;
    (at_host ? host_code : client_code)[e->kind] = e->code;
    (at_host ? host_value : client_value)[e->kind] = e->value;
    if (at_host && e->kind == ZMIDI_NET_EVENT_KIND_DATA) {
      for (k = 0; k < e->word_count && host_word_count < 16; k++) host_words[host_word_count++] = words[e->word_offset + k];
    }
    if (!at_host && e->kind == ZMIDI_NET_EVENT_KIND_PING_REPLY) ping_id_seen = e->code;
  }
}

static bool contains(const uint8_t *p, size_t len, const uint8_t *x, size_t n) {
  size_t i;
  for (i = 0; x != NULL && i + n <= len; i++) {
    if (memcmp(p + i, x, n) == 0) return true;
  }
  return false;
}

static void host_rx(size_t len, ZmidiNetBuffers *b) {
  buffers(b);
  if (host_creds) {
    CHECK(zmidi_net_host_receive_auth(&host, now, &client_addr, payload, len, host_creds, b) == ZMIDI_STATUS_OK);
  } else {
    CHECK(zmidi_net_host_receive(&host, now, &client_addr, payload, len, b) == ZMIDI_STATUS_OK);
  }
  note(true, b);
}

static void client_rx(size_t len, ZmidiNetBuffers *b) {
  buffers(b);
  if (client_creds) {
    CHECK(zmidi_net_client_receive_auth(&client, now, &host_addr, payload, len, client_creds, b) == ZMIDI_STATUS_OK);
  } else {
    CHECK(zmidi_net_client_receive(&client, now, &host_addr, payload, len, b) == ZMIDI_STATUS_OK);
  }
  note(false, b);
}

/* Moves every pending payload of both sides to the other side until none is left. When nothing
 * moves and a side has a deadline, the time goes to the first deadline and both sides poll. */
static void pump(void) {
  int round;
  for (round = 0; round < 60; round++) {
    bool moved = false;
    ZmidiNetAddr to;
    size_t len = 0;
    ZmidiNetBuffers b;
    for (;;) {
      CHECK(zmidi_net_client_write_pending(&client, now, payload, sizeof payload, &to, &len) == ZMIDI_STATUS_OK);
      if (len == 0) break;
      if (contains(payload, len, client_expect, client_expect_len)) client_sent_expected = true;
      host_rx(len, &b);
      moved = true;
    }
    for (;;) {
      CHECK(zmidi_net_host_write_pending(&host, now, payload, sizeof payload, &to, &len) == ZMIDI_STATUS_OK);
      if (len == 0) break;
      CHECK(to.family == 4 && to.port == client_addr.port);
      if (contains(payload, len, host_expect, host_expect_len)) host_sent_expected = true;
      client_rx(len, &b);
      moved = true;
    }
    if (!moved && !step_deadlines) break;
    if (!moved) {
      uint64_t dh = 0, dc = 0, next;
      bool h = zmidi_net_host_next_deadline(&host, &dh), c = zmidi_net_client_next_deadline(&client, &dc);
      /* Only the short timers of the authentication (failure delay, repeats); no session timeout. */
      if (!h && !c) break;
      next = h && c ? (dh < dc ? dh : dc) : (h ? dh : dc);
      if (next > now + 5000) break;
      if (next > now) now = next;
      buffers(&b);
      CHECK(zmidi_net_host_poll(&host, now, &b) == ZMIDI_STATUS_OK);
      note(true, &b);
      buffers(&b);
      CHECK(zmidi_net_client_poll(&client, now, &b) == ZMIDI_STATUS_OK);
      note(false, &b);
    } else {
      now += 10;
    }
  }
}

static ZmidiNetAddr v4(uint8_t a, uint8_t b, uint8_t c, uint8_t d, uint16_t port) {
  ZmidiNetAddr x;
  memset(&x, 0, sizeof x);
  x.family = 4; x.ip[0] = a; x.ip[1] = b; x.ip[2] = c; x.ip[3] = d; x.port = port;
  return x;
}

static void test_plain(void) {
  static const uint32_t note_on[2] = {0x40903C00u, 0xC0000000u};
  ZmidiNetBuffers b;
  uint32_t id = 0;

  CHECK(zmidi_net_host_size() <= sizeof host && zmidi_net_client_size() <= sizeof client);
  /* DEC-019 rule 6: storage of exactly the *_size() bytes is enough (ASan finds a write past the
   * end of the heap block). */
  {
    ZmidiNetHost *h = (ZmidiNetHost *)malloc(zmidi_net_host_size());
    ZmidiNetClient *c = (ZmidiNetClient *)malloc(zmidi_net_client_size());
    CHECK(h != NULL && zmidi_net_host_init(h, (const uint8_t *)"zmidi Host", (const uint8_t *)"host-1") == ZMIDI_STATUS_OK);
    CHECK(c != NULL && zmidi_net_client_init(c, (const uint8_t *)"zmidi Client", (const uint8_t *)"client-1") == ZMIDI_STATUS_OK);
    free(h); free(c);
  }
  CHECK(zmidi_net_host_init(&host, (const uint8_t *)"zmidi Host", (const uint8_t *)"host-1") == ZMIDI_STATUS_OK);
  CHECK(zmidi_net_client_init(&client, (const uint8_t *)"zmidi Client", (const uint8_t *)"client-1") == ZMIDI_STATUS_OK);
  CHECK(zmidi_net_host_init(&host, (const uint8_t *)"", (const uint8_t *)"x") == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_net_host_init(&host, (const uint8_t *)"zmidi Host", (const uint8_t *)"host-1") == ZMIDI_STATUS_OK);

  /* Invitation and Invitation Reply: both sides Established, with no authentication method. */
  CHECK(zmidi_net_client_connect(&client, now, &host_addr) == ZMIDI_STATUS_OK);
  pump();
  CHECK(host_seen[ZMIDI_NET_EVENT_KIND_ESTABLISHED] == 1);
  CHECK(client_seen[ZMIDI_NET_EVENT_KIND_ESTABLISHED] == 1);
  CHECK(host_code[ZMIDI_NET_EVENT_KIND_ESTABLISHED] == 0);

  /* UMP Data from the Client arrives on the Host with the same words. */
  CHECK(zmidi_net_client_send_ump(&client, &host_addr, note_on, 2) == ZMIDI_STATUS_OK);
  pump();
  CHECK(host_seen[ZMIDI_NET_EVENT_KIND_DATA] >= 1);
  CHECK(host_word_count == 2 && host_words[0] == note_on[0] && host_words[1] == note_on[1]);

  /* A Ping and its Ping Reply. */
  CHECK(zmidi_net_client_ping(&client, now, &host_addr, &id) == ZMIDI_STATUS_OK);
  pump();
  CHECK(client_seen[ZMIDI_NET_EVENT_KIND_PING_REPLY] == 1 && ping_id_seen == id);

  /* A Bye (0x01 User terminated) from the Client: Closed on both sides. */
  buffers(&b);
  CHECK(zmidi_net_client_bye(&client, now, &host_addr, 0x01, &b) == ZMIDI_STATUS_OK);
  note(false, &b);
  pump();
  CHECK(host_seen[ZMIDI_NET_EVENT_KIND_CLOSED] == 1);
  CHECK(client_seen[ZMIDI_NET_EVENT_KIND_CLOSED] == 1);

  /* Requests to an unknown peer, bad addresses, small buffers. */
  {
    ZmidiNetAddr bad = host_addr, to;
    size_t len;
    bad.family = 5;
    CHECK(zmidi_net_client_send_ump(&client, &bad, note_on, 2) == ZMIDI_STATUS_INVALID_ARGUMENT);
    CHECK(zmidi_net_host_send_ump(&host, &client_addr, note_on, 2) == ZMIDI_STATUS_REJECTED);
    CHECK(zmidi_net_host_write_pending(&host, now, payload, 16, &to, &len) == ZMIDI_STATUS_BUFFER_TOO_SMALL);
    buffers(&b);
    b.words_capacity = 0;
    CHECK(zmidi_net_host_receive(&host, now, &client_addr, payload, 16, &b) == ZMIDI_STATUS_BUFFER_TOO_SMALL);
  }
}

/* Stateless replies and IPv6 (M2-124 7.1): UMP Data from an address with no Session gets a
 * reply that the receive call writes as a record [family][16 address bytes][port BE][length BE]
 * [payload]. A multicast address is refused (M2-124 5.1: peer to peer). Two new UMP Data Commands in one packet
 * give two Data events with their word offsets. */
static void test_replies_and_ipv6(void) {
  static const uint32_t note_on[2] = {0x40903C00u, 0xC0000000u};
  /* "MIDI", UMP Data (0xFF, 1 word, Sequence Number 0), a MIDI 1.0 Channel Voice Note On. */
  static const uint8_t data[12] = {'M', 'I', 'D', 'I', 0xFF, 0x01, 0x00, 0x00, 0x20, 0x90, 0x3C, 0x40};
  ZmidiNetAddr v6, multicast;
  ZmidiNetBuffers b;
  size_t len, k;
  uint32_t id = 0;

  memset(&v6, 0, sizeof v6);
  v6.family = 6;
  v6.ip[0] = 0xFD; v6.ip[1] = 0x00; v6.ip[15] = 0x20;
  v6.port = 0xC351;
  CHECK(zmidi_net_host_init(&host, (const uint8_t *)"zmidi Host", (const uint8_t *)"host-1") == ZMIDI_STATUS_OK);
  memcpy(payload, data, sizeof data);
  buffers(&b);
  CHECK(zmidi_net_host_receive(&host, now, &v6, payload, sizeof data, &b) == ZMIDI_STATUS_OK);
  CHECK(b.out_len > ZMIDI_NET_RECORD_HEADER);
  len = ((size_t)out[19] << 8) | out[20];
  CHECK(b.out_len == ZMIDI_NET_RECORD_HEADER + len);
  CHECK(out[0] == 6 && memcmp(out + 1, v6.ip, 16) == 0 && out[17] == 0xC3 && out[18] == 0x51);
  CHECK(len >= 8 && memcmp(out + ZMIDI_NET_RECORD_HEADER, "MIDI", 4) == 0);

  multicast = v4(224, 0, 0, 251, 5673);
  CHECK(zmidi_net_client_init(&client, (const uint8_t *)"zmidi Client", (const uint8_t *)"client-1") == ZMIDI_STATUS_OK);
  CHECK(zmidi_net_client_ping(&client, now, &multicast, &id) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_net_host_send_ump(&host, &multicast, note_on, 2) == ZMIDI_STATUS_INVALID_ARGUMENT);

  /* Two sends before one delivery. */
  reset_seen();
  CHECK(zmidi_net_client_connect(&client, now, &host_addr) == ZMIDI_STATUS_OK);
  pump();
  CHECK(host_seen[ZMIDI_NET_EVENT_KIND_ESTABLISHED] == 1);
  /* The first packet is lost; the second repeats the first UMP Data Command (M2-124 7.2), so the
   * Host gets both from one packet: two Data events, the second at word offset 2. */
  host_word_count = 0;
  reset_seen();
  CHECK(zmidi_net_client_send_ump(&client, &host_addr, note_on, 2) == ZMIDI_STATUS_OK);
  CHECK(zmidi_net_client_write_pending(&client, now, payload, sizeof payload, &v6, &len) == ZMIDI_STATUS_OK && len > 0);
  CHECK(zmidi_net_client_send_ump(&client, &host_addr, note_on, 2) == ZMIDI_STATUS_OK);
  CHECK(zmidi_net_client_write_pending(&client, now, payload, sizeof payload, &v6, &len) == ZMIDI_STATUS_OK && len > 0);
  host_rx(len, &b);
  CHECK(host_seen[ZMIDI_NET_EVENT_KIND_DATA] == 2 && b.events_len >= 2);
  CHECK(events[b.events_len - 1].word_offset == 2 && events[b.events_len - 1].word_count == 2);
  CHECK(host_word_count == 4);
  for (k = 0; k < 4; k++) {
    CHECK(host_words[k] == note_on[k % 2]);
  }
}

/* The random bytes that give a CryptoNonce: CryptoNonce byte = 32 + (random byte mod 95). */
static void random_for(const char *nonce, uint8_t *random) {
  size_t i;
  for (i = 0; i < 16; i++) random[i] = (uint8_t)(nonce[i] - 32);
}

static void bytes(ZmidiNetUser *u, const char *name, const char *password) {
  u->username = (const uint8_t *)name; u->username_len = strlen(name);
  u->password = (const uint8_t *)password; u->password_len = strlen(password);
}

static void ends(void) {
  ZmidiNetBuffers b;
  buffers(&b);
  (void)zmidi_net_client_bye(&client, now, &host_addr, 0x01, &b);
  pump();
}

/* M2-124 6.9 example: CryptoNonce "nUWrn*@#$hjfwnkL" and shared secret "5483". */
static void test_shared_secret(void) {
  static const char nonce[] = "nUWrn*@#$hjfwnkL";
  static const uint8_t digest[32] = {0x67, 0x6E, 0xBE, 0x82, 0x58, 0x7C, 0xEC, 0xA8, 0xF8, 0x2F, 0xC3,
                                     0x33, 0xD7, 0x87, 0x95, 0x1E, 0xEC, 0x2B, 0x00, 0xAD, 0x31, 0x61,
                                     0x3C, 0xC4, 0xDB, 0x18, 0xCF, 0x27, 0x37, 0x3A, 0xFB, 0x82};
  uint8_t random[16];
  ZmidiNetCredentials hc, cc;
  random_for(nonce, random);
  memset(&hc, 0, sizeof hc); memset(&cc, 0, sizeof cc);
  hc.shared_secret = (const uint8_t *)"5483"; hc.shared_secret_len = 4;
  hc.random = random; hc.random_len = sizeof random;
  cc.shared_secret = (const uint8_t *)"5483"; cc.shared_secret_len = 4;
  host_creds = &hc; client_creds = &cc;
  host_expect = (const uint8_t *)nonce; host_expect_len = 16;
  client_expect = digest; client_expect_len = 32;
  reset_seen();

  CHECK(zmidi_net_host_init_auth(&host, (const uint8_t *)"Host", (const uint8_t *)"P1", ZMIDI_NET_AUTH_SHARED_SECRET) == ZMIDI_STATUS_OK);
  CHECK(zmidi_net_client_init_auth(&client, (const uint8_t *)"zmidi Client", (const uint8_t *)"client-1", ZMIDI_NET_AUTH_SHARED_SECRET) == ZMIDI_STATUS_OK);
  CHECK(zmidi_net_client_connect(&client, now, &host_addr) == ZMIDI_STATUS_OK);
  pump();
  /* Invitation Reply: Authentication Required with the CryptoNonce, state 0x00 (first request). */
  CHECK(host_sent_expected);
  CHECK(client_seen[ZMIDI_NET_EVENT_KIND_AUTH_REQUESTED] == 1);
  CHECK(client_code[ZMIDI_NET_EVENT_KIND_AUTH_REQUESTED] == ZMIDI_NET_AUTH_SHARED_SECRET);
  CHECK(client_value[ZMIDI_NET_EVENT_KIND_AUTH_REQUESTED] == 0x00);
  /* Invitation with Authentication with the digest of the example, and the session. */
  CHECK(client_sent_expected);
  CHECK(host_seen[ZMIDI_NET_EVENT_KIND_ESTABLISHED] == 1 && client_seen[ZMIDI_NET_EVENT_KIND_ESTABLISHED] == 1);
  CHECK(host_code[ZMIDI_NET_EVENT_KIND_ESTABLISHED] == ZMIDI_NET_AUTH_SHARED_SECRET);
  CHECK(client_code[ZMIDI_NET_EVENT_KIND_ESTABLISHED] == ZMIDI_NET_AUTH_SHARED_SECRET);
  CHECK(host_seen[ZMIDI_NET_EVENT_KIND_AUTH_FAILED] == 0);
  CHECK(hc.random_used == 16);
  ends();
}

/* M2-124 6.10 example: CryptoNonce "XI|~=NNRVaD;XCPL", Username "Rosa", password "RPBqBno". */
static void test_user(void) {
  static const char nonce[] = "XI|~=NNRVaD;XCPL";
  static const uint8_t digest[32] = {0x3A, 0x2D, 0x5E, 0xBE, 0xEF, 0x92, 0xE8, 0x46, 0x35, 0x35, 0xC2,
                                     0x7F, 0xB4, 0xB7, 0xB3, 0xE2, 0xD1, 0xAC, 0xF5, 0x7A, 0x84, 0x4C,
                                     0x6D, 0x08, 0x08, 0xE0, 0x41, 0xC1, 0x02, 0xB7, 0xFF, 0x1A};
  uint8_t random[16];
  ZmidiNetUser table[2], me;
  ZmidiNetCredentials hc, cc;
  random_for(nonce, random);
  bytes(&table[0], "Ana", "other");
  bytes(&table[1], "Rosa", "RPBqBno");
  bytes(&me, "Rosa", "RPBqBno");
  memset(&hc, 0, sizeof hc); memset(&cc, 0, sizeof cc);
  hc.users = table; hc.user_count = 2;
  hc.random = random; hc.random_len = sizeof random;
  cc.users = &me; cc.user_count = 1;
  host_creds = &hc; client_creds = &cc;
  host_expect = (const uint8_t *)nonce; host_expect_len = 16;
  client_expect = digest; client_expect_len = 32;
  reset_seen();

  CHECK(zmidi_net_host_init_auth(&host, (const uint8_t *)"Host", (const uint8_t *)"P1", ZMIDI_NET_AUTH_USER) == ZMIDI_STATUS_OK);
  CHECK(zmidi_net_client_init_auth(&client, (const uint8_t *)"zmidi Client", (const uint8_t *)"client-1", ZMIDI_NET_AUTH_SHARED_SECRET | ZMIDI_NET_AUTH_USER) == ZMIDI_STATUS_OK);
  CHECK(zmidi_net_client_connect(&client, now, &host_addr) == ZMIDI_STATUS_OK);
  pump();
  CHECK(host_sent_expected);
  CHECK(client_seen[ZMIDI_NET_EVENT_KIND_AUTH_REQUESTED] == 1);
  CHECK(client_code[ZMIDI_NET_EVENT_KIND_AUTH_REQUESTED] == ZMIDI_NET_AUTH_USER);
  CHECK(client_sent_expected);
  CHECK(host_seen[ZMIDI_NET_EVENT_KIND_ESTABLISHED] == 1 && client_seen[ZMIDI_NET_EVENT_KIND_ESTABLISHED] == 1);
  CHECK(host_code[ZMIDI_NET_EVENT_KIND_ESTABLISHED] == ZMIDI_NET_AUTH_USER);
  CHECK(client_code[ZMIDI_NET_EVENT_KIND_ESTABLISHED] == ZMIDI_NET_AUTH_USER);
  ends();
}

/* A wrong password (or an unknown Username): AuthFailed on the Host, the Client sees the
 * Authentication State of the failure, and after the failure limit (5) the Host ends the
 * Invitation with Bye 0x43 (or 0x44). No session. */
static void test_failure(const char *name, const char *password, uint32_t failed_code, uint8_t state, uint8_t bye) {
  static uint8_t random[64];
  ZmidiNetUser table[1], me;
  ZmidiNetCredentials hc, cc;
  size_t i;
  for (i = 0; i < sizeof random; i++) random[i] = (uint8_t)(i * 7 + 3);
  bytes(&table[0], "Rosa", "RPBqBno");
  bytes(&me, name, password);
  memset(&hc, 0, sizeof hc); memset(&cc, 0, sizeof cc);
  hc.users = table; hc.user_count = 1;
  hc.random = random; hc.random_len = sizeof random;
  cc.users = &me; cc.user_count = 1;
  host_creds = &hc; client_creds = &cc;
  host_expect = client_expect = NULL;
  reset_seen();

  CHECK(zmidi_net_host_init_auth(&host, (const uint8_t *)"Host", (const uint8_t *)"P1", ZMIDI_NET_AUTH_USER) == ZMIDI_STATUS_OK);
  CHECK(zmidi_net_client_init_auth(&client, (const uint8_t *)"zmidi Client", (const uint8_t *)"client-1", ZMIDI_NET_AUTH_USER) == ZMIDI_STATUS_OK);
  CHECK(zmidi_net_client_connect(&client, now, &host_addr) == ZMIDI_STATUS_OK);
  pump();
  CHECK(host_seen[ZMIDI_NET_EVENT_KIND_AUTH_FAILED] == 5);
  CHECK(host_code[ZMIDI_NET_EVENT_KIND_AUTH_FAILED] == failed_code);
  CHECK(host_value[ZMIDI_NET_EVENT_KIND_AUTH_FAILED] == 5);
  CHECK(client_seen[ZMIDI_NET_EVENT_KIND_AUTH_REQUESTED] >= 2);
  CHECK(client_value[ZMIDI_NET_EVENT_KIND_AUTH_REQUESTED] == state);
  CHECK(host_seen[ZMIDI_NET_EVENT_KIND_ESTABLISHED] == 0 && client_seen[ZMIDI_NET_EVENT_KIND_ESTABLISHED] == 0);
  CHECK(client_seen[ZMIDI_NET_EVENT_KIND_CLOSED] == 1);
  CHECK(client_code[ZMIDI_NET_EVENT_KIND_CLOSED] == 0 && client_value[ZMIDI_NET_EVENT_KIND_CLOSED] == bye);
  CHECK(hc.random_used == 16);
}

static void test_auth_arguments(void) {
  ZmidiNetCredentials c;
  ZmidiNetBuffers b;
  uint8_t random[16] = {0};
  CHECK(zmidi_net_host_init_auth(&host, (const uint8_t *)"Host", (const uint8_t *)"P1", 0) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_net_host_init_auth(&host, (const uint8_t *)"Host", (const uint8_t *)"P1", 4) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_net_client_init_auth(&client, (const uint8_t *)"C", (const uint8_t *)"c", 0) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_net_client_init_auth(&client, (const uint8_t *)"", (const uint8_t *)"c", 1) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_net_host_init_auth(&host, (const uint8_t *)"Host", (const uint8_t *)"P1", 3) == ZMIDI_STATUS_OK);
  memset(&c, 0, sizeof c);
  c.shared_secret_len = 4; /* a null pointer with a length */
  buffers(&b);
  CHECK(zmidi_net_host_receive_auth(&host, now, &client_addr, payload, 0, &c, &b) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_net_host_receive_auth(&host, now, &client_addr, payload, 0, NULL, &b) == ZMIDI_STATUS_INVALID_ARGUMENT);
  /* A Host with no random bytes can not ask for authentication: CredentialsError code 0. */
  {
    static const char nonce[] = "nUWrn*@#$hjfwnkL";
    ZmidiNetCredentials hc, cc;
    memset(&hc, 0, sizeof hc); memset(&cc, 0, sizeof cc);
    random_for(nonce, random);
    hc.shared_secret = (const uint8_t *)"5483"; hc.shared_secret_len = 4;
    hc.random = random; hc.random_len = sizeof random; hc.random_used = 16; /* all used */
    cc.shared_secret = (const uint8_t *)"5483"; cc.shared_secret_len = 4;
    host_creds = &hc; client_creds = &cc;
    host_expect = client_expect = NULL;
    reset_seen();
    CHECK(zmidi_net_client_init_auth(&client, (const uint8_t *)"zmidi Client", (const uint8_t *)"client-1", ZMIDI_NET_AUTH_SHARED_SECRET) == ZMIDI_STATUS_OK);
    CHECK(zmidi_net_client_connect(&client, now, &host_addr) == ZMIDI_STATUS_OK);
    pump();
    CHECK(host_seen[ZMIDI_NET_EVENT_KIND_CREDENTIALS_ERROR] >= 1 && host_code[ZMIDI_NET_EVENT_KIND_CREDENTIALS_ERROR] == 0);
    CHECK(host_seen[ZMIDI_NET_EVENT_KIND_ESTABLISHED] == 0);
  }
}

/* Sets up a Host with `host_methods` and a Client with `client_methods`, connects, and runs the
 * handshake to its end. */
static void handshake(uint8_t host_methods, uint8_t client_methods, ZmidiNetCredentials *hc, ZmidiNetCredentials *cc) {
  host_creds = hc; client_creds = cc;
  host_expect = client_expect = NULL;
  reset_seen();
  CHECK(zmidi_net_host_init_auth(&host, (const uint8_t *)"Host", (const uint8_t *)"P1", host_methods) == ZMIDI_STATUS_OK);
  CHECK(zmidi_net_client_init_auth(&client, (const uint8_t *)"zmidi Client", (const uint8_t *)"client-1", client_methods) == ZMIDI_STATUS_OK);
  CHECK(zmidi_net_client_connect(&client, now, &host_addr) == ZMIDI_STATUS_OK);
  pump();
}

/* Both methods on the Host (methods 3): a Client with both Capabilities bits gets the user method
 * (N-44), a Client with D0 only gets the shared secret method (REQ-FFI-NET-006). */
static void test_both_methods(void) {
  static uint8_t random[64];
  ZmidiNetUser table[1], me;
  ZmidiNetCredentials hc, cc;
  size_t i;
  for (i = 0; i < sizeof random; i++) random[i] = (uint8_t)(i * 11 + 5);
  bytes(&table[0], "Rosa", "RPBqBno");
  bytes(&me, "Rosa", "RPBqBno");
  memset(&hc, 0, sizeof hc); memset(&cc, 0, sizeof cc);
  hc.shared_secret = (const uint8_t *)"5483"; hc.shared_secret_len = 4;
  hc.users = table; hc.user_count = 1;
  hc.random = random; hc.random_len = sizeof random;
  cc.shared_secret = (const uint8_t *)"5483"; cc.shared_secret_len = 4;
  cc.users = &me; cc.user_count = 1;

  handshake(3, ZMIDI_NET_AUTH_SHARED_SECRET | ZMIDI_NET_AUTH_USER, &hc, &cc);
  CHECK(client_code[ZMIDI_NET_EVENT_KIND_AUTH_REQUESTED] == ZMIDI_NET_AUTH_USER);
  CHECK(host_seen[ZMIDI_NET_EVENT_KIND_ESTABLISHED] == 1 && host_code[ZMIDI_NET_EVENT_KIND_ESTABLISHED] == ZMIDI_NET_AUTH_USER);
  ends();

  hc.random_used = 0;
  handshake(3, ZMIDI_NET_AUTH_SHARED_SECRET, &hc, &cc);
  CHECK(client_code[ZMIDI_NET_EVENT_KIND_AUTH_REQUESTED] == ZMIDI_NET_AUTH_SHARED_SECRET);
  CHECK(host_seen[ZMIDI_NET_EVENT_KIND_ESTABLISHED] == 1 && host_code[ZMIDI_NET_EVENT_KIND_ESTABLISHED] == ZMIDI_NET_AUTH_SHARED_SECRET);
  ends();
}

/* The code of each CredentialsError (REQ-FFI-NET-008): 1 no secret (Host), 2 empty secret, 3 empty
 * password, 4 Username not valid (Client), 5 Username too long (Client). */
static void test_credentials_codes(void) {
  static uint8_t random[64];
  static char long_name[1001];
  ZmidiNetUser table[1], me;
  ZmidiNetCredentials hc, cc;
  size_t i;
  for (i = 0; i < sizeof random; i++) random[i] = (uint8_t)(i * 13 + 1);
  memset(long_name, 'a', sizeof long_name - 1);
  bytes(&table[0], "Rosa", "RPBqBno");

  /* 1: the Host has no shared secret to verify the digest. */
  memset(&hc, 0, sizeof hc); memset(&cc, 0, sizeof cc);
  hc.random = random; hc.random_len = sizeof random;
  cc.shared_secret = (const uint8_t *)"5483"; cc.shared_secret_len = 4;
  handshake(ZMIDI_NET_AUTH_SHARED_SECRET, ZMIDI_NET_AUTH_SHARED_SECRET, &hc, &cc);
  CHECK(host_seen[ZMIDI_NET_EVENT_KIND_CREDENTIALS_ERROR] >= 1 && host_code[ZMIDI_NET_EVENT_KIND_CREDENTIALS_ERROR] == 1);
  CHECK(host_seen[ZMIDI_NET_EVENT_KIND_ESTABLISHED] == 0);

  /* 2: the Client has an empty shared secret (a pointer with length 0). */
  hc.shared_secret = (const uint8_t *)"5483"; hc.shared_secret_len = 4; hc.random_used = 0;
  cc.shared_secret = (const uint8_t *)""; cc.shared_secret_len = 0;
  handshake(ZMIDI_NET_AUTH_SHARED_SECRET, ZMIDI_NET_AUTH_SHARED_SECRET, &hc, &cc);
  CHECK(client_seen[ZMIDI_NET_EVENT_KIND_CREDENTIALS_ERROR] >= 1 && client_code[ZMIDI_NET_EVENT_KIND_CREDENTIALS_ERROR] == 2);

  /* 3, 4, 5: the user of the Client has an empty password, an empty Username, a long Username. */
  memset(&hc, 0, sizeof hc);
  hc.users = table; hc.user_count = 1; hc.random = random; hc.random_len = sizeof random;
  memset(&cc, 0, sizeof cc);
  cc.users = &me; cc.user_count = 1;
  bytes(&me, "Rosa", "");
  handshake(ZMIDI_NET_AUTH_USER, ZMIDI_NET_AUTH_USER, &hc, &cc);
  CHECK(client_seen[ZMIDI_NET_EVENT_KIND_CREDENTIALS_ERROR] >= 1 && client_code[ZMIDI_NET_EVENT_KIND_CREDENTIALS_ERROR] == 3);
  bytes(&me, "", "RPBqBno");
  hc.random_used = 0;
  handshake(ZMIDI_NET_AUTH_USER, ZMIDI_NET_AUTH_USER, &hc, &cc);
  CHECK(client_seen[ZMIDI_NET_EVENT_KIND_CREDENTIALS_ERROR] >= 1 && client_code[ZMIDI_NET_EVENT_KIND_CREDENTIALS_ERROR] == 4);
  bytes(&me, long_name, "RPBqBno");
  hc.random_used = 0;
  handshake(ZMIDI_NET_AUTH_USER, ZMIDI_NET_AUTH_USER, &hc, &cc);
  CHECK(client_seen[ZMIDI_NET_EVENT_KIND_CREDENTIALS_ERROR] >= 1 && client_code[ZMIDI_NET_EVENT_KIND_CREDENTIALS_ERROR] == 5);
  CHECK(client_seen[ZMIDI_NET_EVENT_KIND_ESTABLISHED] == 0);
}

/* A null pointer with a length above 0 in the users, the random bytes or a user gives invalid
 * argument (REQ-FFI-NET-007). */
static void test_null_with_length(void) {
  ZmidiNetCredentials c;
  ZmidiNetUser u;
  ZmidiNetBuffers b;
  CHECK(zmidi_net_host_init_auth(&host, (const uint8_t *)"Host", (const uint8_t *)"P1", 3) == ZMIDI_STATUS_OK);
  memset(&c, 0, sizeof c);
  c.random_len = 16;
  buffers(&b);
  CHECK(zmidi_net_host_receive_auth(&host, now, &client_addr, payload, 0, &c, &b) == ZMIDI_STATUS_INVALID_ARGUMENT);
  memset(&c, 0, sizeof c);
  c.user_count = 1;
  CHECK(zmidi_net_host_receive_auth(&host, now, &client_addr, payload, 0, &c, &b) == ZMIDI_STATUS_INVALID_ARGUMENT);
  memset(&u, 0, sizeof u);
  u.password = (const uint8_t *)"x"; u.password_len = 1; u.username_len = 4;
  memset(&c, 0, sizeof c);
  c.users = &u; c.user_count = 1;
  CHECK(zmidi_net_host_receive_auth(&host, now, &client_addr, payload, 0, &c, &b) == ZMIDI_STATUS_INVALID_ARGUMENT);
  u.username = (const uint8_t *)"Rosa"; u.password = NULL;
  CHECK(zmidi_net_host_receive_auth(&host, now, &client_addr, payload, 0, &c, &b) == ZMIDI_STATUS_INVALID_ARGUMENT);
}

int main(void) {
  host_addr = v4(192, 168, 1, 10, 5673);
  client_addr = v4(192, 168, 1, 20, 50000);

  test_plain();
  test_replies_and_ipv6();
  step_deadlines = true;
  test_shared_secret();
  test_user();
  test_failure("Rosa", "wrong", ZMIDI_NET_AUTH_USER, 0x01, 0x43);
  test_failure("Nobody", "RPBqBno", ZMIDI_NET_AUTH_USER | ZMIDI_NET_AUTH_USERNAME_NOT_FOUND, 0x02, 0x44);
  test_auth_arguments();
  test_both_methods();
  test_credentials_codes();
  test_null_with_length();

  if (failures) { fprintf(stderr, "%d failure(s)\n", failures); return EXIT_FAILURE; }
  printf("net ok: host %u B, client %u B\n", (unsigned)zmidi_net_host_size(), (unsigned)zmidi_net_client_size());
  return EXIT_SUCCESS;
}
