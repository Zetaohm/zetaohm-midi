#include "common.h"
#include "midi2_msg.h"
/* MIDI 2.0 to 1.0 Protocol (midi2_msg_mt4_to_mt2: one word, RPN/NRPN and Bank dropped). */
void feature(void){ uint32_t w[2]={INPUT[8],INPUT[9]}, out; if(midi2_msg_mt4_to_mt2(w,&out)) SINK(out); }
