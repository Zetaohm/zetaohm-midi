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
    CHECK(read_packet(p, sizeof p, &dropped) == 1 && rec[0].words[0] == 0x20903C64u);
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
  /* A buffer shorter than the packet length, null pointers, nothing to send. */
  CHECK(zmidi_ble_write_packet(&w, u, 1, out, 19, &len, &taken) == ZMIDI_STATUS_BUFFER_TOO_SMALL);
  CHECK(zmidi_ble_write_packet(&w, NULL, 1, out, sizeof out, &len, &taken) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_ble_write_packet(&w, NULL, 0, out, sizeof out, &len, &taken) == ZMIDI_STATUS_OK && len == 0 && taken == 0);
  CHECK(!zmidi_ble_writer_has_held(&w));
  CHECK(zmidi_ble_writer_set_att_mtu(&w, 10) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_ble_writer_set_running_status(NULL, true) == ZMIDI_STATUS_INVALID_ARGUMENT);
}

static void round_trip_sysex(void) {
  /* SysEx of 16 bytes in three SysEx7 packets (start, continue, end), written into packets of
   * ATT_MTU 23 and read back. */
  ZmidiBleWriter w;
  ZmidiBleTimedUmp u[3];
  uint8_t out[64];
  size_t len, taken, n, dropped, total = 0, i = 0, got = 0, guard;
  uint32_t back[3][2];
  u[0] = timed(1, 0x30160001u, 0x02030405u, 2);
  u[1] = timed(1, 0x30260607u, 0x08090A0Bu, 2);
  u[2] = timed(1, 0x30360C0Du, 0x0E0F1011u, 2);
  CHECK(zmidi_ble_writer_init(&w, 23) == ZMIDI_STATUS_OK);
  CHECK(zmidi_ble_reader_init(&reader, 0, 1) == ZMIDI_STATUS_OK);
  for (guard = 0; guard < 16; guard++) {
    CHECK(zmidi_ble_write_packet(&w, u + total, 3 - total, out, sizeof out, &len, &taken) == ZMIDI_STATUS_OK);
    total += taken;
    if (len == 0) break;
    n = read_packet(out, len, &dropped);
    CHECK(dropped == 0);
    for (i = 0; i < n && got < 3; i++, got++) {
      back[got][0] = rec[i].words[0];
      back[got][1] = rec[i].words[1];
    }
  }
  CHECK(total == 3 && got == 3);
  for (i = 0; i < 3 && i < got; i++) CHECK(back[i][0] == u[i].words[0] && back[i][1] == u[i].words[1]);
}

static void sizes(void) {
  CHECK(zmidi_ble_reader_size() <= sizeof(ZmidiBleReader));
  CHECK(zmidi_ble_writer_size() <= sizeof(ZmidiBleWriter));
}

int main(void) {
  reader_packets();
  writer_packets();
  round_trip_sysex();
  sizes();
  if (failures) { fprintf(stderr, "%d failure(s)\n", failures); return EXIT_FAILURE; }
  printf("ble ok\n");
  return EXIT_SUCCESS;
}
