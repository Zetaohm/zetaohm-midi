#include "common.h"
#include "cmidi2.h"
void feature(void){
  cmidi2_ump w[4]={INPUT[0],INPUT[1],INPUT[2],INPUT[3]};
  switch (cmidi2_ump_get_message_type(w)) {
    case 4: switch (cmidi2_ump_get_status_code(w)) {
      case 0x90: SINK(cmidi2_ump_get_channel(w)^cmidi2_ump_get_midi2_note_note(w)^cmidi2_ump_get_midi2_note_velocity(w)^cmidi2_ump_get_midi2_note_attribute_type(w)^cmidi2_ump_get_midi2_note_attribute_data(w)); break;
      case 0x80: SINK(cmidi2_ump_get_midi2_note_note(w)); break;
      case 0x30: case 0x50: SINK(cmidi2_ump_get_midi2_nrpn_msb(w)^cmidi2_ump_get_midi2_nrpn_lsb(w)^cmidi2_ump_get_midi2_nrpn_data(w)^cmidi2_ump_get_status_code(w)); break;
      case 0xB0: SINK(cmidi2_ump_get_midi2_cc_index(w)^cmidi2_ump_get_midi2_cc_data(w)); break;
      default: SINK(w[1]); } break;
    case 1: SINK(cmidi2_ump_get_status_byte(w)); break;
    default: break; }
}
