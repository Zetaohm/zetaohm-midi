#include "common.h"
#include "midi2_msg.h"
/* MIDI 1.0 to 2.0 Protocol (midi2_msg_mt2_to_mt4: stateless, no Bank or RPN/NRPN state). */
void feature(void){ uint32_t out[2]; if(midi2_msg_mt2_to_mt4(INPUT[8],out)) SINK(out[0]^out[1]); }
