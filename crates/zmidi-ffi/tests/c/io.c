/* C test of the io part of the C ABI (REQ-FFI-IO-006). Vectors: M2-104 Appendix D (D.1 MIDI 1.0
 * byte stream to MIDI 1.0 Protocol UMP, D.3 to MIDI 2.0 Protocol), M1 (running status, real time
 * between data bytes, System Exclusive). */
#ifndef ZMIDI_IO
#define ZMIDI_IO
#endif
#include "zmidi.h"

#include <stdio.h>
#include <stdlib.h>

static int failures;
#define CHECK(c) do { if (!(c)) { fprintf(stderr, "%s:%d: CHECK(%s)\n", __FILE__, __LINE__, #c); failures++; } } while (0)

static size_t push(ZmidiBytesToUmp *r, uint8_t b, uint32_t *out) {
  size_t n = 99;
  CHECK(zmidi_bytes_to_ump_push(r, b, out, ZMIDI_BYTES_TO_UMP_MAX_WORDS, &n) == ZMIDI_STATUS_OK);
  return n;
}

static void bytes_to_ump_midi1(void) {
  ZmidiBytesToUmp r;
  uint32_t w[ZMIDI_BYTES_TO_UMP_MAX_WORDS];
  CHECK(zmidi_bytes_to_ump_init(&r, 3, 1) == ZMIDI_STATUS_OK);
  /* Note On channel 1, note 60, velocity 100 on Group 3: Message Type 0x2. */
  CHECK(push(&r, 0x90, w) == 0);
  CHECK(push(&r, 0x3C, w) == 0);
  CHECK(push(&r, 0x64, w) == 1 && w[0] == 0x23903C64u);
  /* Running status (M1): the next two data bytes are a Note On too. */
  CHECK(push(&r, 0x3E, w) == 0);
  CHECK(push(&r, 0x40, w) == 1 && w[0] == 0x23903E40u);
  /* Timing Clock between data bytes: a System Real Time UMP at once (Message Type 0x1). */
  CHECK(push(&r, 0x90, w) == 0);
  CHECK(push(&r, 0x3C, w) == 0);
  CHECK(push(&r, 0xF8, w) == 1 && w[0] == 0x13F80000u);
  CHECK(push(&r, 0x64, w) == 1 && w[0] == 0x23903C64u);
}

static void bytes_to_ump_midi2(void) {
  ZmidiBytesToUmp r;
  uint32_t w[ZMIDI_BYTES_TO_UMP_MAX_WORDS];
  CHECK(zmidi_bytes_to_ump_init(&r, 0, 2) == ZMIDI_STATUS_OK);
  /* D.3: Note On to Message Type 0x4; velocity 100 upscaled to 16 bits is 0xC924 (D.3.2). */
  push(&r, 0x90, w);
  push(&r, 0x3C, w);
  CHECK(push(&r, 0x64, w) == 2);
  CHECK(w[0] == 0x40903C00u);
  CHECK(w[1] == 0xC9240000u);
}

static void bytes_to_ump_sysex(void) {
  ZmidiBytesToUmp r;
  uint32_t w[ZMIDI_BYTES_TO_UMP_MAX_WORDS];
  static const uint8_t msg[] = {0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7};
  size_t i, n = 0;
  CHECK(zmidi_bytes_to_ump_init(&r, 0, 1) == ZMIDI_STATUS_OK);
  for (i = 0; i < sizeof msg; i++) n = push(&r, msg[i], w);
  /* One SysEx7 packet, Complete in One UMP, 4 bytes (M2-104 7.7). */
  CHECK(n == 2 && w[0] == 0x30047E7Fu && w[1] == 0x06010000u);
}

static void bytes_to_ump_small_buffer(void) {
  ZmidiBytesToUmp r;
  uint32_t w[ZMIDI_BYTES_TO_UMP_MAX_WORDS];
  size_t n = 99;
  CHECK(zmidi_bytes_to_ump_init(&r, 0, 1) == ZMIDI_STATUS_OK);
  push(&r, 0x90, w);
  push(&r, 0x3C, w);
  /* Too small: the byte is not read, so the UMP comes with the next push. */
  CHECK(zmidi_bytes_to_ump_push(&r, 0x64, w, ZMIDI_BYTES_TO_UMP_MAX_WORDS - 1, &n) == ZMIDI_STATUS_BUFFER_TOO_SMALL);
  CHECK(n == 99);
  CHECK(push(&r, 0x64, w) == 1 && w[0] == 0x20903C64u);
}

static void bytes_to_ump_invalid(void) {
  ZmidiBytesToUmp r;
  uint32_t w[ZMIDI_BYTES_TO_UMP_MAX_WORDS];
  size_t n;
  CHECK(zmidi_bytes_to_ump_init(&r, 16, 1) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_bytes_to_ump_init(&r, 0, 3) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_bytes_to_ump_init(NULL, 0, 1) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_bytes_to_ump_init(&r, 0, 1) == ZMIDI_STATUS_OK);
  CHECK(push(&r, 0x90, w) == 0);
  CHECK(zmidi_bytes_to_ump_push(NULL, 0x3C, w, 4, &n) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_bytes_to_ump_push(&r, 0x3C, NULL, 4, &n) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_bytes_to_ump_push(&r, 0x3C, w, 4, NULL) == ZMIDI_STATUS_INVALID_ARGUMENT);
  /* REQ-FFI-ABI-002: the null calls changed no state: the Note On goes on as before. */
  CHECK(push(&r, 0x3C, w) == 0);
  CHECK(push(&r, 0x64, w) == 1 && w[0] == 0x20903C64u);
}

static size_t write1(ZmidiUmpToBytes *wr, uint32_t word, uint8_t *out, size_t cap, ZmidiStatus *st) {
  size_t n = 99;
  *st = zmidi_ump_to_bytes_write(wr, &word, 1, out, cap, &n);
  return n;
}

static void ump_to_bytes(void) {
  ZmidiUmpToBytes wr;
  uint8_t b[16];
  ZmidiStatus st;
  size_t n;
  CHECK(zmidi_ump_to_bytes_init(&wr, true) == ZMIDI_STATUS_OK);
  n = write1(&wr, 0x20903C64u, b, sizeof b, &st);
  CHECK(st == ZMIDI_STATUS_OK && n == 3 && b[0] == 0x90 && b[1] == 0x3C && b[2] == 0x64);
  /* Running status: no status byte for the same status. */
  n = write1(&wr, 0x20903E40u, b, sizeof b, &st);
  CHECK(st == ZMIDI_STATUS_OK && n == 2 && b[0] == 0x3E && b[1] == 0x40);
  /* After a refresh the status byte comes again (M1 Appendix A-2). */
  CHECK(zmidi_ump_to_bytes_refresh_status(&wr) == ZMIDI_STATUS_OK);
  n = write1(&wr, 0x20903E40u, b, sizeof b, &st);
  CHECK(st == ZMIDI_STATUS_OK && n == 3 && b[0] == 0x90);
  /* Too small: no state change, so a retry with a larger buffer gives the same bytes. */
  CHECK(zmidi_ump_to_bytes_refresh_status(&wr) == ZMIDI_STATUS_OK);
  n = write1(&wr, 0x20903C64u, b, 2, &st);
  CHECK(st == ZMIDI_STATUS_BUFFER_TOO_SMALL);
  n = write1(&wr, 0x20903C64u, b, 3, &st);
  CHECK(st == ZMIDI_STATUS_OK && n == 3 && b[0] == 0x90);
  /* Running status off: each message has its status byte (REQ-FFI-IO-003). */
  CHECK(zmidi_ump_to_bytes_init(&wr, false) == ZMIDI_STATUS_OK);
  n = write1(&wr, 0x20903C64u, b, sizeof b, &st);
  CHECK(st == ZMIDI_STATUS_OK && n == 3 && b[0] == 0x90 && b[1] == 0x3C && b[2] == 0x64);
  n = write1(&wr, 0x20903E40u, b, sizeof b, &st);
  CHECK(st == ZMIDI_STATUS_OK && n == 3 && b[0] == 0x90 && b[1] == 0x3E && b[2] == 0x40);
  /* Null pointers. */
  CHECK(zmidi_ump_to_bytes_init(NULL, false) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_ump_to_bytes_refresh_status(NULL) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_ump_to_bytes_write(&wr, NULL, 1, b, sizeof b, &n) == ZMIDI_STATUS_INVALID_ARGUMENT);
}

/* REQ-FFI-IO-007 (OPT-035): many bytes in one call. The MIDI 2.0 Note On of M2-104 Appendix D,
 * twice with running status, then a Clock between data bytes. */
static void bytes_to_ump_feed(void) {
  static const uint8_t in[] = {0x90, 0x3C, 0x64, 0x3E, 0xF8, 0x64};
  ZmidiBytesToUmp r;
  uint32_t w[16];
  size_t read = 0, n = 0;
  CHECK(zmidi_bytes_to_ump_init(&r, 0, 1) == ZMIDI_STATUS_OK);
  CHECK(zmidi_bytes_to_ump_feed(&r, in, sizeof in, w, 16, &read, &n) == ZMIDI_STATUS_OK);
  CHECK(read == sizeof in && n == 3);
  CHECK(w[0] == 0x20903C64u && w[1] == 0x10F80000u && w[2] == 0x20903E64u);
  /* A small buffer: the call stops before a byte that could not fit, and the rest follows. */
  CHECK(zmidi_bytes_to_ump_init(&r, 0, 1) == ZMIDI_STATUS_OK);
  CHECK(zmidi_bytes_to_ump_feed(&r, in, sizeof in, w, ZMIDI_BYTES_TO_UMP_MAX_WORDS, &read, &n) == ZMIDI_STATUS_OK);
  CHECK(read == 3 && n == 1 && w[0] == 0x20903C64u);
  CHECK(zmidi_bytes_to_ump_feed(&r, in + read, sizeof in - read, w, ZMIDI_BYTES_TO_UMP_MAX_WORDS, &read, &n) == ZMIDI_STATUS_OK);
  CHECK(read == 2 && n == 1 && w[0] == 0x10F80000u);
  /* Too small, no bytes, null pointers. */
  CHECK(zmidi_bytes_to_ump_feed(&r, in, sizeof in, w, ZMIDI_BYTES_TO_UMP_MAX_WORDS - 1, &read, &n) == ZMIDI_STATUS_BUFFER_TOO_SMALL);
  CHECK(zmidi_bytes_to_ump_feed(&r, NULL, 0, w, 16, &read, &n) == ZMIDI_STATUS_OK && read == 0 && n == 0);
  CHECK(zmidi_bytes_to_ump_feed(&r, NULL, 1, w, 16, &read, &n) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_bytes_to_ump_feed(NULL, in, 1, w, 16, &read, &n) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_bytes_to_ump_feed(&r, in, 1, NULL, 16, &read, &n) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_bytes_to_ump_feed(&r, in, 1, w, 16, NULL, &n) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_bytes_to_ump_feed(&r, in, 1, w, 16, &read, NULL) == ZMIDI_STATUS_INVALID_ARGUMENT);
}

static void sizes_and_version(void) {
  void *p;
  CHECK(zmidi_bytes_to_ump_size() > 0 && zmidi_bytes_to_ump_size() <= sizeof(ZmidiBytesToUmp));
  CHECK(zmidi_ump_to_bytes_size() > 0 && zmidi_ump_to_bytes_size() <= sizeof(ZmidiUmpToBytes));
  /* REQ-FFI-IO-005: storage of exactly the given size is enough (AddressSanitizer checks it). */
  p = malloc(zmidi_bytes_to_ump_size());
  CHECK(p != NULL && zmidi_bytes_to_ump_init((ZmidiBytesToUmp *)p, 0, 2) == ZMIDI_STATUS_OK);
  free(p);
  p = malloc(zmidi_ump_to_bytes_size());
  CHECK(p != NULL && zmidi_ump_to_bytes_init((ZmidiUmpToBytes *)p, true) == ZMIDI_STATUS_OK);
  free(p);
  CHECK(zmidi_abi_version() == ((uint32_t)ZMIDI_ABI_MAJOR << 16 | ZMIDI_ABI_MINOR));
}

int main(void) {
  bytes_to_ump_midi1();
  bytes_to_ump_midi2();
  bytes_to_ump_sysex();
  bytes_to_ump_small_buffer();
  bytes_to_ump_invalid();
  bytes_to_ump_feed();
  ump_to_bytes();
  sizes_and_version();
  if (failures) { fprintf(stderr, "%d failure(s)\n", failures); return EXIT_FAILURE; }
  printf("io ok\n");
  return EXIT_SUCCESS;
}
