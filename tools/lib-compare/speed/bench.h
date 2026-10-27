/* Speed benches of other MIDI libraries, with the inputs of tools/speed-probe (docs/optimization-backlog.md
   OPT-029). The same plugin counts the instructions between two calls of speed_mark. */
#pragma once
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
extern uint32_t CV1_WORDS[64];    /* MIDI 1.0 Note On UMPs, notes 0 to 63, velocity 0x40 */
extern uint32_t CV2_WORDS[128];   /* the same as MIDI 2.0 Note On UMPs (D.3 velocity 0x8000) */
extern uint32_t MIXED_WORDS[96];  /* 32 x (MIDI 2.0 Note On), 32 x (MIDI 1.0 CC): 64 packets */
extern uint8_t NOTE_BYTES[192];   /* 64 MIDI 1.0 Note On messages with status */
extern uint8_t SYSEX_PAYLOAD[1020];
extern uint32_t SYSEX_WORDS[340]; /* the SysEx7 packets of SYSEX_PAYLOAD */
extern uint8_t MCODED7[1166];     /* SYSEX_PAYLOAD in Mcoded7 */
void bench(const char *name, uint32_t items, uint32_t expect, uint32_t (*f)(void));
void lib_run(void);
#ifdef __cplusplus
}
#endif
/* Hides a value from the optimizer, as core::hint::black_box. */
#define BB(x) __asm__ volatile("" : "+r"(x) :: "memory")
#define SINK(x) do { uint32_t s_ = (uint32_t)(x); __asm__ volatile("" :: "r"(s_) : "memory"); } while (0)
