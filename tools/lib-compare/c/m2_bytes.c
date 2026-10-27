#include "common.h"
#include "midi2_conv.h"
static midi2_conv_state st; static bool init;
void feature(void){
  if(!init){ midi2_conv_init(&st,0); init=true; }
  for(int i=0;i<(int)(INPUT[63]&63);i++){ if(midi2_conv_feed(&st,(uint8_t)INPUT[i])){ do { SINK(st.ump[0]^st.ump[1]); } while(midi2_conv_next(&st)); } }
}
