#include "common.h"
#include "cmidi2.h"
/* The message set of the Rust job decode_lean: 8 MIDI 2.0 Channel Voice and 5 System messages,
   each field read, dispatch by the caller (cmidi2 has getters only). */
void feature(void){
  cmidi2_ump w[2]={INPUT[0],INPUT[1]};
  switch (cmidi2_ump_get_message_type(w)) {
    case 4: switch (cmidi2_ump_get_status_code(w)) {
      case 0x30: SINK(1^cmidi2_ump_get_group(w)^cmidi2_ump_get_channel(w)^cmidi2_ump_get_midi2_nrpn_msb(w)^cmidi2_ump_get_midi2_nrpn_lsb(w)^cmidi2_ump_get_midi2_nrpn_data(w)); break;
      case 0x40: SINK(2^cmidi2_ump_get_group(w)^cmidi2_ump_get_channel(w)^cmidi2_ump_get_midi2_nrpn_msb(w)^cmidi2_ump_get_midi2_nrpn_lsb(w)^cmidi2_ump_get_midi2_nrpn_data(w)); break;
      case 0xC0: SINK(3^cmidi2_ump_get_channel(w)^cmidi2_ump_get_midi2_program_program(w)^cmidi2_ump_get_midi2_program_options(w)); break;
      case 0xB0: SINK(4^cmidi2_ump_get_channel(w)^cmidi2_ump_get_midi2_cc_index(w)^cmidi2_ump_get_midi2_cc_data(w)); break;
      case 0x90: SINK(5^cmidi2_ump_get_channel(w)^cmidi2_ump_get_midi2_note_note(w)^cmidi2_ump_get_midi2_note_velocity(w)^cmidi2_ump_get_midi2_note_attribute_type(w)^cmidi2_ump_get_midi2_note_attribute_data(w)); break;
      case 0x80: SINK(6^cmidi2_ump_get_channel(w)^cmidi2_ump_get_midi2_note_note(w)); break;
      case 0x60: SINK(7^cmidi2_ump_get_channel(w)^cmidi2_ump_get_midi2_pn_pitch_bend_note(w)^cmidi2_ump_get_midi2_pitch_bend_data(w)); break;
      case 0xE0: SINK(8^cmidi2_ump_get_channel(w)^cmidi2_ump_get_midi2_pitch_bend_data(w)); break;
      default: break; } break;
    case 1: switch (cmidi2_ump_get_status_byte(w)) {
      case 0xF8: SINK(9); break; case 0xFA: SINK(10); break; case 0xFC: SINK(11); break; case 0xFB: SINK(12); break;
      case 0xF2: SINK(cmidi2_ump_get_system_message_byte2(w)^(cmidi2_ump_get_system_message_byte3(w)<<7)); break;
      default: break; } break;
    default: break; }
}
