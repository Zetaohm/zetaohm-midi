/* C test of the ble part of the C ABI (REQ-FFI-BLE-005). Vectors: BLE-MIDI v1.0 sections 7 and 8
 * (the packets of the zmidi-ble tests). */
#ifndef ZMIDI_BLE
#define ZMIDI_BLE
#endif
#include "zmidi.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int failures;
#define CHECK(c) do { if (!(c)) { fprintf(stderr, "%s:%d: CHECK(%s)\n", __FILE__, __LINE__, #c); failures++; } } while (0)

#define HDR(t) ((uint8_t)(0x80 | ((t) >> 7)))
#define TS(t) ((uint8_t)(0x80 | ((t) & 0x7F)))

static ZmidiBleReader reader;
static ZmidiBleUmp rec[64];

static size_t read_packet(const uint8_t *p, size_t len, size_t *dropped) {
  size_t n = 99;
  CHECK(zmidi_ble_read(&reader, p, len, rec, 64, &n, dropped) == ZMIDI_STATUS_OK);
  return n;
}

static void reader_packets(void) {
  size_t dropped = 99;
  CHECK(zmidi_ble_reader_init(&reader, 0, 1) == ZMIDI_STATUS_OK);
  {
    /* Section 7: one and two full messages with timestamps. */
    const uint8_t p[] = {HDR(0x0A55), TS(0x0A55), 0x90, 0x3C, 0x64, TS(0x0A57), 0x80, 0x3C, 0x00};
    CHECK(read_packet(p, sizeof p, &dropped) == 2 && dropped == 0);
    CHECK(rec[0].timestamp_ms == 0x0A55 && rec[0].word_count == 1 && rec[0].words[0] == 0x20903C64u);
    CHECK(rec[1].timestamp_ms == 0x0A57 && rec[1].words[0] == 0x20803C00u);
  }
  {
    /* Running status with no timestamp byte. */
    const uint8_t p[] = {HDR(5), TS(5), 0x90, 0x3C, 0x64, 0x40, 0x64};
    CHECK(read_packet(p, sizeof p, &dropped) == 2);
    CHECK(rec[1].timestamp_ms == 5 && rec[1].words[0] == 0x20904064u);
  }
  {
    /* Running status with a timestamp byte. */
    const uint8_t p[] = {HDR(5), TS(5), 0x90, 0x3C, 0x64, TS(6), 0x40, 0x64};
    CHECK(read_packet(p, sizeof p, &dropped) == 2);
    CHECK(rec[1].timestamp_ms == 6 && rec[1].words[0] == 0x20904064u);
  }
  {
    /* Section 8: a SysEx message in one packet. */
    const uint8_t p[] = {HDR(1), TS(1), 0xF0, 0x01, 0x02, 0x03, TS(1), 0xF7};
    CHECK(read_packet(p, sizeof p, &dropped) == 1 && dropped == 0);
    CHECK(rec[0].word_count == 2 && rec[0].words[0] == 0x30030102u && rec[0].words[1] == 0x03000000u);
  }
  {
    /* System Real Time with its own timestamp. */
    const uint8_t p[] = {HDR(9), TS(9), 0xF8};
    CHECK(read_packet(p, sizeof p, &dropped) == 1 && rec[0].words[0] == 0x10F80000u);
  }
  {
    /* No header byte: the packet is dropped whole. */
    const uint8_t p[] = {0x10, 0x90, 0x3C, 0x64};
    CHECK(read_packet(p, sizeof p, &dropped) == 0 && dropped == 1);
  }
  {
    /* Fewer records than bytes: refused before the packet is read. */
    const uint8_t p[] = {HDR(1), TS(1), 0x90, 0x3C, 0x64};
    size_t n = 99;
    CHECK(zmidi_ble_read(&reader, p, sizeof p, rec, 4, &n, &dropped) == ZMIDI_STATUS_BUFFER_TOO_SMALL);
    CHECK(n == 99);
    CHECK(read_packet(p, sizeof p, &dropped) == 1 && rec[0].words[0] == 0x20903C64u);
  }
  {
    /* The refused packet is not read: a SysEx continuation still finds its open message. */
    const uint8_t a[] = {HDR(1), TS(1), 0xF0, 0x01, 0x02};
    const uint8_t b[] = {HDR(2), 0x03, 0x04, TS(2), 0xF7};
    size_t n = 99;
    CHECK(read_packet(a, sizeof a, &dropped) == 0);
    CHECK(zmidi_ble_read(&reader, b, sizeof b, rec, 4, &n, &dropped) == ZMIDI_STATUS_BUFFER_TOO_SMALL);
    CHECK(n == 99);
    CHECK(read_packet(b, sizeof b, &dropped) == 1 && dropped == 0);
    CHECK(rec[0].words[0] == 0x30040102u && rec[0].words[1] == 0x03040000u);
  }
  CHECK(zmidi_ble_reader_init(&reader, 16, 1) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_ble_reader_init(&reader, 0, 0) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_ble_reader_reset(NULL) == ZMIDI_STATUS_INVALID_ARGUMENT);
}

static ZmidiBleTimedUmp timed(uint64_t ms, uint32_t w0, uint32_t w1, uint8_t wc) {
  ZmidiBleTimedUmp u;
  memset(&u, 0, sizeof u);
  u.millis = ms;
  u.word_count = wc;
  u.words[0] = w0;
  u.words[1] = w1;
  return u;
}

static void writer_packets(void) {
  ZmidiBleWriter w;
  uint8_t out[64];
  size_t len = 99, taken = 99;
  ZmidiBleTimedUmp u[2];
  CHECK(zmidi_ble_writer_init(&w, 22) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_ble_writer_init(&w, 23) == ZMIDI_STATUS_OK);
  CHECK(zmidi_ble_writer_packet_len(&w) == 20);
  {
    /* Section 7: one full message. */
    const uint8_t want[] = {HDR(0x0A55), TS(0x0A55), 0x90, 0x3C, 0x64};
    u[0] = timed(0x0A55, 0x20903C64u, 0, 1);
    CHECK(zmidi_ble_write_packet(&w, u, 1, out, sizeof out, &len, &taken) == ZMIDI_STATUS_OK);
    CHECK(len == sizeof want && memcmp(out, want, len) == 0 && taken == 1);
  }
  {
    /* Same time: running status with no status byte and no timestamp byte. */
    const uint8_t want[] = {HDR(5), TS(5), 0x90, 0x3C, 0x64, 0x40, 0x64};
    u[0] = timed(5, 0x20903C64u, 0, 1);
    u[1] = timed(5, 0x20904064u, 0, 1);
    CHECK(zmidi_ble_writer_reset(&w) == ZMIDI_STATUS_OK);
    CHECK(zmidi_ble_write_packet(&w, u, 2, out, sizeof out, &len, &taken) == ZMIDI_STATUS_OK);
    CHECK(len == sizeof want && memcmp(out, want, len) == 0 && taken == 2);
  }
  /* A buffer shorter than the packet length: no state change, so the same packet follows. */
  CHECK(zmidi_ble_writer_reset(&w) == ZMIDI_STATUS_OK);
  CHECK(zmidi_ble_write_packet(&w, u, 1, out, 19, &len, &taken) == ZMIDI_STATUS_BUFFER_TOO_SMALL);
  {
    const uint8_t want[] = {HDR(5), TS(5), 0x90, 0x3C, 0x64};
    CHECK(zmidi_ble_write_packet(&w, u, 1, out, sizeof out, &len, &taken) == ZMIDI_STATUS_OK);
    CHECK(len == sizeof want && memcmp(out, want, len) == 0 && taken == 1);
  }
  {
    /* Running status off: the second message has its timestamp and status bytes. */
    const uint8_t want[] = {HDR(5), TS(5), 0x90, 0x3C, 0x64, TS(5), 0x90, 0x40, 0x64};
    CHECK(zmidi_ble_writer_set_running_status(&w, false) == ZMIDI_STATUS_OK);
    CHECK(zmidi_ble_write_packet(&w, u, 2, out, sizeof out, &len, &taken) == ZMIDI_STATUS_OK);
    CHECK(len == sizeof want && memcmp(out, want, len) == 0 && taken == 2);
    CHECK(zmidi_ble_writer_set_running_status(&w, true) == ZMIDI_STATUS_OK);
  }
  /* A valid ATT_MTU change: packets of ATT_MTU minus 3 bytes. */
  CHECK(zmidi_ble_writer_set_att_mtu(&w, 30) == ZMIDI_STATUS_OK);
  CHECK(zmidi_ble_writer_packet_len(&w) == 27);
  CHECK(zmidi_ble_writer_set_att_mtu(&w, 23) == ZMIDI_STATUS_OK);
  /* Null pointers, nothing to send. */
  CHECK(zmidi_ble_write_packet(&w, NULL, 1, out, sizeof out, &len, &taken) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_ble_write_packet(&w, NULL, 0, out, sizeof out, &len, &taken) == ZMIDI_STATUS_OK && len == 0 && taken == 0);
  CHECK(!zmidi_ble_writer_has_held(&w));
  CHECK(zmidi_ble_writer_set_att_mtu(&w, 10) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_ble_writer_set_running_status(NULL, true) == ZMIDI_STATUS_INVALID_ARGUMENT);
}

/* REQ-FFI-BLE-001: the Group and the protocol of the reader. */
static void reader_group_and_protocol(void) {
  const uint8_t p[] = {HDR(1), TS(1), 0x90, 0x3C, 0x64};
  size_t dropped = 99;
  CHECK(zmidi_ble_reader_init(&reader, 5, 2) == ZMIDI_STATUS_OK);
  CHECK(read_packet(p, sizeof p, &dropped) == 1 && dropped == 0);
  /* MIDI 2.0 Note On on Group 5; velocity 0x64 upscaled to 16 bits (M2-115): 0xC924. */
  CHECK(rec[0].word_count == 2 && rec[0].words[0] == 0x45903C00u && rec[0].words[1] == 0xC9240000u);
  CHECK(zmidi_ble_reader_init(&reader, 16, 1) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_ble_reader_init(&reader, 0, 3) == ZMIDI_STATUS_INVALID_ARGUMENT);
}

static void round_trip_sysex(void) {
  /* SysEx of 16 bytes in three SysEx7 packets (start, continue, end), written into packets of
   * ATT_MTU 23 and read back. */
  ZmidiBleWriter w;
  ZmidiBleTimedUmp u[3];
  uint8_t out[64];
  size_t len, taken, n, dropped, total = 0, i = 0, got = 0, guard;
  uint32_t back[3][2];
  uint8_t second[8];
  size_t second_len = 0, packets = 0;
  u[0] = timed(1, 0x30160001u, 0x02030405u, 2);
  u[1] = timed(1, 0x30260607u, 0x08090A0Bu, 2);
  u[2] = timed(1, 0x30360C0Du, 0x0E0F1011u, 2);
  CHECK(zmidi_ble_writer_init(&w, 23) == ZMIDI_STATUS_OK);
  CHECK(zmidi_ble_reader_init(&reader, 0, 1) == ZMIDI_STATUS_OK);
  for (guard = 0; guard < 16; guard++) {
    CHECK(zmidi_ble_write_packet(&w, u + total, 3 - total, out, sizeof out, &len, &taken) == ZMIDI_STATUS_OK);
    total += taken;
    if (len == 0) break;
    packets++;
    /* The first packet holds bytes for the next one. */
    if (packets == 1) CHECK(zmidi_ble_writer_has_held(&w));
    if (packets == 2 && len <= sizeof second) {
      memcpy(second, out, len);
      second_len = len;
    }
    n = read_packet(out, len, &dropped);
    CHECK(dropped == 0);
    for (i = 0; i < n && got < 3; i++, got++) {
      back[got][0] = rec[i].words[0];
      back[got][1] = rec[i].words[1];
    }
  }
  CHECK(total == 3 && got == 3);
  CHECK(!zmidi_ble_writer_has_held(&w));
  {
    /* The second packet starts with the held byte 0x11, then the end of the SysEx. */
    const uint8_t want[] = {HDR(1), 0x11, TS(1), 0xF7};
    CHECK(packets == 2 && second_len == sizeof want && memcmp(second, want, sizeof want) == 0);
  }
  for (i = 0; i < 3 && i < got; i++) CHECK(back[i][0] == u[i].words[0] && back[i][1] == u[i].words[1]);
}

static void sizes(void) {
  CHECK(zmidi_ble_reader_size() <= sizeof(ZmidiBleReader));
  CHECK(zmidi_ble_writer_size() <= sizeof(ZmidiBleWriter));
  /* DEC-019 rule 6: storage of exactly the *_size() bytes is enough (ASan finds a write past the
   * end of the heap block). */
  {
    ZmidiBleReader *r = (ZmidiBleReader *)malloc(zmidi_ble_reader_size());
    ZmidiBleWriter *w = (ZmidiBleWriter *)malloc(zmidi_ble_writer_size());
    CHECK(r != NULL && zmidi_ble_reader_init(r, 0, 1) == ZMIDI_STATUS_OK);
    CHECK(w != NULL && zmidi_ble_writer_init(w, 23) == ZMIDI_STATUS_OK);
    free(r);
    free(w);
  }
}

int main(void) {
  reader_packets();
  writer_packets();
  reader_group_and_protocol();
  round_trip_sysex();
  sizes();
  if (failures) { fprintf(stderr, "%d failure(s)\n", failures); return EXIT_FAILURE; }
  printf("ble ok\n");
  return EXIT_SUCCESS;
}
