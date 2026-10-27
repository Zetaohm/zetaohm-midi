#include "common.h"
#include "umpProcessor.h"
static umpProcessor up; static bool init;
extern "C" void feature(void){
  if(!init){ up.setCVM([](umpCVM m){ SINK(m.umpGroup^m.status^m.channel^m.note^m.value^m.index^m.bank^m.flag1); });
    up.setSystem([](umpGeneric m){ SINK(m.status^m.value); }); init=true; }
  up.processUMP(INPUT[0]);
}
