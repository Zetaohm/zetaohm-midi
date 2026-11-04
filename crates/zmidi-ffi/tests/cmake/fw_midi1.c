/* A bare-metal link check of the MIDI 1.0-only build (REQ-FFI-ABI-005, M7 of DEC-022): a 16-channel
 * MIDI 1.0 receiver on DIN bytes with the mode, controller and parameter states, an echo writer,
 * and the MIDI 1.0-only io, ble and usb parts. The ELF must hold no MIDI 2.0 code. */
#include "zmidi.h"

static ZmidiMidi1Reader reader;
static ZmidiMidi1Writer writer;
static ZmidiModeState mode;
static ZmidiControllerState controllers[16];
static ZmidiParameterState parameters[16];
static uint16_t pitch_bend_sensitivity[16];
static ZmidiBytesToUmp to_ump;
static ZmidiBleReader ble;
static ZmidiUsbLink usb;
static uint8_t sysex[64];
volatile uint8_t rx;
volatile uint8_t packet[16];
volatile uint32_t sink;

static void reset_controllers(uint8_t channel) {
  uint8_t c;
  for (c = 0; c < 16; c++) {
    if (channel == 0xFF || channel == c) {
      (void)zmidi_controllers_reset(&controllers[c]);
      (void)zmidi_parameters_reset(&parameters[c]);
    }
  }
}

static void voice(const ZmidiMidi1Message *m) {
  ZmidiModeAction a;
  ZmidiControllerEvent ce;
  ZmidiParameterEvent pe;
  uint8_t ch = m->status & 0x0F;
  if (zmidi_mode_receive(&mode, m, &a) != ZMIDI_STATUS_OK) {
    return;
  }
  if (a.kind == ZMIDI_MODE_ACTION_KIND_RESET_ALL_CONTROLLERS) {
    reset_controllers(a.channel);
  }
  if (a.kind != ZMIDI_MODE_ACTION_KIND_VOICE) {
    sink = a.kind;
    return;
  }
  if (zmidi_controllers_receive(&controllers[ch], m, &ce) == ZMIDI_STATUS_OK &&
      ce.kind != ZMIDI_CONTROLLER_EVENT_KIND_NONE) {
    sink = ce.value + ce.bank;
  }
  if (zmidi_parameters_receive(&parameters[ch], m, &pe) == ZMIDI_STATUS_OK &&
      pe.change != ZMIDI_PARAMETER_CHANGE_NONE && pe.parameter.registered && pe.parameter.msb == 0 &&
      pe.parameter.lsb == 0) {
    (void)zmidi_parameter_apply(&pe, pitch_bend_sensitivity[ch], &pitch_bend_sensitivity[ch]);
  }
}

void Reset_Handler(void) {
  static const ZmidiModeConfig config = {0, 0x0F, true, 16, false};
  static const ZmidiUsbLinkConfig link = {false, 0xFFFF, 64, 1};
  ZmidiMidi1Message m[ZMIDI_MIDI1_READER_MAX_EVENTS];
  uint32_t w[ZMIDI_BYTES_TO_UMP_MAX_WORDS];
  ZmidiBleUmp b[4];
  ZmidiUsbUmp u[4];
  uint8_t out[8];
  size_t n, k, i;
  uint8_t c;
  (void)zmidi_midi1_reader_init(&reader);
  (void)zmidi_midi1_writer_init(&writer, true);
  (void)zmidi_mode_init(&mode, &config);
  for (c = 0; c < 16; c++) {
    (void)zmidi_controllers_init(&controllers[c]);
    (void)zmidi_parameters_init(&parameters[c]);
    pitch_bend_sensitivity[c] = 2 << 7;
  }
  (void)zmidi_bytes_to_ump_init(&to_ump, 0, 1);
  (void)zmidi_ble_reader_init(&ble, 0, 1);
  (void)zmidi_usb_link_init(&usb, &link);
  for (;;) {
    uint8_t byte = rx;
    if (zmidi_midi1_reader_push(&reader, byte, sysex, sizeof sysex, m, ZMIDI_MIDI1_READER_MAX_EVENTS, &n) ==
        ZMIDI_STATUS_OK) {
      for (i = 0; i < n; i++) {
        if (m[i].kind == ZMIDI_MIDI1_KIND_CHANNEL_VOICE) {
          voice(&m[i]);
        }
        if (zmidi_midi1_writer_write(&writer, &m[i], sysex, out, sizeof out, &k) == ZMIDI_STATUS_OK) {
          sink = out[0] + (uint32_t)k;
        }
      }
    }
    if (zmidi_bytes_to_ump_push(&to_ump, byte, w, ZMIDI_BYTES_TO_UMP_MAX_WORDS, &n) == ZMIDI_STATUS_OK && n > 0) {
      sink = w[0];
    }
    if (zmidi_ble_read(&ble, (const uint8_t *)packet, sizeof packet, b, 4, &n, &k) == ZMIDI_STATUS_OK && n > 0) {
      sink = b[0].words[0];
    }
    if (zmidi_usb_link_read(&usb, (const uint8_t *)packet, sizeof packet, u, 4, &n, &k) == ZMIDI_STATUS_OK && n > 0) {
      sink = u[0].words[0];
    }
  }
}
