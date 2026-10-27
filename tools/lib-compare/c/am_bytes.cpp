#include "common.h"
#include "bytestreamToUMP.h"
static bytestreamToUMP bs;
extern "C" void feature(void){
  for(int i=0;i<(int)(INPUT[63]&63);i++){ bs.bytestreamParse((uint8_t)INPUT[i]); while(bs.availableUMP()) SINK(bs.readUMP()); }
}
