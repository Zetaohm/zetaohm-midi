#include "bench.h"

extern uint32_t _estack;
void reset(void);
__attribute__((section(".vectors"), used)) void *const VECTORS[2] = { &_estack, (void *)reset };

static int semihost(int op, const void *arg) {
  register int r0 __asm__("r0") = op;
  register const void *r1 __asm__("r1") = arg;
  __asm__ volatile("bkpt 0xAB" : "+r"(r0) : "r"(r1) : "memory");
  return r0;
}
static void put(const char *s) { semihost(0x04, s); }
static void put_u(uint32_t v) {
  char b[12]; int i = 11; b[i] = 0;
  do { b[--i] = (char)('0' + v % 10); v /= 10; } while (v);
  put(&b[i]);
}
static void quit(int ok) { semihost(0x18, (void *)(ok ? 0x20026 : 0x20024)); for (;;) {} }

/* The marker: the plugin counts the instructions before its first instruction. */
#ifdef __clang__
__attribute__((noinline, used)) void speed_mark(void)
#else
__attribute__((noipa, used)) void speed_mark(void)
#endif
{ volatile uint32_t x = 1; (void)x; }

void bench(const char *name, uint32_t items, uint32_t expect, uint32_t (*f)(void)) {
  put("BENCH "); put(name); put(" "); put_u(items); put("\n");
  speed_mark();
  uint32_t got = f();
  speed_mark();
  if (got != expect) { put("CHECK "); put(name); put(" gave "); put_u(got); put("\n"); quit(0); }
}

uint32_t CV1_WORDS[64], CV2_WORDS[128], MIXED_WORDS[96], SYSEX_WORDS[340];
uint8_t NOTE_BYTES[192], SYSEX_PAYLOAD[1020], MCODED7[1166];

static void inputs(void) {
  for (uint32_t i = 0; i < 64; i++) {
    CV1_WORDS[i] = 0x20900040u | (i << 8);
    CV2_WORDS[2 * i] = 0x40900000u | (i << 8);
    CV2_WORDS[2 * i + 1] = 0x80000000u;
    NOTE_BYTES[3 * i] = 0x90; NOTE_BYTES[3 * i + 1] = (uint8_t)i; NOTE_BYTES[3 * i + 2] = 0x40;
  }
  for (uint32_t i = 0; i < 32; i++) {
    MIXED_WORDS[3 * i] = 0x40903C00u | i; MIXED_WORDS[3 * i + 1] = 0x80000000u; MIXED_WORDS[3 * i + 2] = 0x20B00700u | i;
  }
  for (uint32_t i = 0; i < 1020; i++) SYSEX_PAYLOAD[i] = (uint8_t)(i % 128);
  for (uint32_t p = 0; p < 170; p++) {
    const uint8_t *b = &SYSEX_PAYLOAD[6 * p];
    uint32_t st = p == 0 ? 1 : p == 169 ? 3 : 2;
    SYSEX_WORDS[2 * p] = 0x30000000u | (st << 20) | (6u << 16) | ((uint32_t)b[0] << 8) | b[1];
    SYSEX_WORDS[2 * p + 1] = ((uint32_t)b[2] << 24) | ((uint32_t)b[3] << 16) | ((uint32_t)b[4] << 8) | b[5];
  }
  /* M2-103 Mcoded7: one byte of high bits (bit 6 = first byte), then up to 7 low 7-bit bytes. */
  uint32_t o = 0;
  for (uint32_t g = 0; g < 1020; g += 7) {
    uint32_t n = 1020 - g < 7 ? 1020 - g : 7, hi = 0;
    for (uint32_t k = 0; k < n; k++) hi |= (uint32_t)(SYSEX_PAYLOAD[g + k] >> 7) << (6 - k);
    MCODED7[o++] = (uint8_t)hi;
    for (uint32_t k = 0; k < n; k++) MCODED7[o++] = SYSEX_PAYLOAD[g + k] & 0x7F;
  }
}

void *__dso_handle = 0;
static uint32_t nothing(void) { return 0; }

void reset(void) {
  extern void (*__init_array_start[])(void), (*__init_array_end[])(void);
  for (void (**f)(void) = __init_array_start; f < __init_array_end; f++) (*f)();
  inputs();
  bench("calibrate", 1, 0, nothing); /* the cost of the marks and the call alone */
  lib_run();
  put("END\n");
  quit(1);
}
