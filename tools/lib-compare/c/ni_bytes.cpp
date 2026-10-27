#include "common.h"
#include "midi/midi1_byte_stream.h"
using namespace midi;
static midi1_byte_stream_parser *bp;
extern "C" void feature(void){
  static midi1_byte_stream_parser parser([](universal_packet p){ SINK(p.data[0]^p.data[1]); }, [](const sysex7& s){ SINK(s.data.size()); });
  for(int i=0;i<(int)(INPUT[63]&63);i++) parser.feed((uint8_t)INPUT[i]);
}
