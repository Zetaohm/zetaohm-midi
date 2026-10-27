#include <stdint.h>
#include <stdbool.h>
extern volatile uint32_t INPUT[64];
#define SINK(x) do { __asm__ volatile("" :: "r"((uint32_t)(x)) : "memory"); } while (0)
