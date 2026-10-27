#include "common.h"
#include "cmidi2.h"
/* UMP to DIN bytes (cmidi2_convert_single_ump_to_midi1). */
void feature(void){
  cmidi2_ump w[4]={INPUT[0],INPUT[1],INPUT[2],INPUT[3]}; uint8_t d[16];
  size_t n=cmidi2_convert_single_ump_to_midi1(d,sizeof d,w); for(size_t i=0;i<n;i++) SINK(d[i]);
}
