/* Host side of the MIDI-CI interop check (OPT-049, step 10): the midi2 C99 MIDI-CI
 * responder (tools/lib-compare/ext/sauloverissimo_midi2) on the host. It reads one MIDI-CI message per
 * line as hex bytes (no F0/F7), gives it to midi2_ci_process_sysex on Group 0, and prints each UMP word
 * of the replies as "W xxxxxxxx", then "END". A line "MUID" prints the responder MUID as "MUID xxxxxxx". */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "midi2_ci.h"

static midi2_ci_state ci;
static uint8_t profiles[2][5];
static midi2_ci_property props[4];

static uint32_t wr(const uint32_t *w, uint32_t n, void *c) {
    (void)c;
    for (uint32_t i = 0; i < n; i++) printf("W %08x\n", w[i]);
    return n;
}
static uint32_t rng(void *c) { (void)c; return 0x0345678u; }
static const char *get(const char *n, void *c) { (void)n; (void)c; return "[1,2,3]"; }
static bool set(const char *n, const char *v, void *c) { (void)n; (void)v; (void)c; return true; }

int main(void) {
    midi2_ci_init(&ci, 0x0123456u, profiles, 2, props, 4);
    midi2_ci_set_identity(&ci, 0x7D0000u, /* one-byte ID 7D: bytes 7D 00 00 (M2-101 5.5.1) */ 0x0102, 0x0304, 0x01020304);
    midi2_ci_set_write_fn(&ci, wr, 0);
    midi2_ci_set_rng(&ci, rng, 0);
    midi2_ci_set_nak_on_unknown(&ci, true);
    const uint8_t gm[5] = {0x7E, 0x21, 0x00, 0x01, 0x01};
    midi2_ci_add_profile(&ci, gm);
    midi2_ci_add_property_dynamic(&ci, "X-Pattern", get, set);
    char line[4096];
    uint8_t msg[1024];
    while (fgets(line, sizeof line, stdin)) {
        if (strncmp(line, "MUID", 4) == 0) {
            printf("MUID %07x\nEND\n", ci.muid);
        } else {
            uint16_t n = 0;
            for (char *p = strtok(line, " \n"); p && n < sizeof msg; p = strtok(NULL, " \n"))
                msg[n++] = (uint8_t)strtoul(p, NULL, 16);
            midi2_ci_process_sysex(&ci, 0, msg, n);
            printf("END\n");
        }
        fflush(stdout);
    }
    return 0;
}
