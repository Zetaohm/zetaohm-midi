#include "common.h"
#include "cmidi2.h"
/* DIN bytes to UMP, MIDI 1.0 Protocol output (cmidi2_convert_midi1_to_ump; at most 255 bytes per call). */
void feature(void){
  static uint8_t in[64]; static cmidi2_ump out[32];
  for(int i=0;i<64;i++) in[i]=(uint8_t)INPUT[i];
  cmidi2_midi_conversion_context c; cmidi2_midi_conversion_context_initialize(&c);
  c.midi_protocol=CMIDI2_PROTOCOL_TYPE_MIDI1; c.midi1=in; c.midi1_num_bytes=INPUT[63]&63; c.ump=out; c.ump_num_bytes=sizeof out;
  if(cmidi2_convert_midi1_to_ump(&c)==CMIDI2_CONVERSION_RESULT_OK) SINK(out[0]^c.ump_proceeded_bytes);
}
