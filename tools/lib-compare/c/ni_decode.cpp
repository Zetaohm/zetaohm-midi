#include "common.h"
#include "midi/midi2_channel_voice_message.h"
#include "midi/system_message.h"
using namespace midi;
extern "C" void feature(void){
  universal_packet p; for(int i=0;i<4;i++) p.data[i]=INPUT[i];
  if (is_midi2_channel_voice_message(p)) { midi2_channel_voice_message_view v(p);
    switch (v.status()) {
      case 0x90: SINK(v.channel()^v.byte3()^get_midi2_note_attribute(p)^get_midi2_note_attribute_data(p)^v.data()); break;
      case 0x80: SINK(v.channel()^v.byte3()); break;
      case 0x30: case 0x50: SINK(v.channel()^v.byte3()^v.byte4()^v.data()^v.status()); break;
      case 0xB0: SINK(v.byte3()^v.data()); break;
      case 0xC0: SINK(v.data()); break;
      case 0xE0: case 0x60: SINK(v.data()^v.byte3()); break;
      default: break; } }
  else if (is_system_message(p)) { system_message_view s(p); SINK(s.status()^s.data_byte_1()^s.data_byte_2()); }
}
