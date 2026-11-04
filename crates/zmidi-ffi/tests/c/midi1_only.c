/* REQ-FFI-ABI-004, REQ-FFI-ABI-006: the MIDI 1.0-only build (Cargo feature midi1, header macro
 * ZMIDI_MIDI1). The io and ble parts give the MIDI 1.0 Protocol only, with the smaller storage of
 * their readers; the usb part works with Alternate Setting 0. */
#define ZMIDI_MIDI1
#include "zmidi.h"

#include <stdio.h>

static int failures;

#define CHECK(cond)                                                                                \
  do {                                                                                             \
    if (!(cond)) {                                                                                 \
      printf("FAIL %s:%d: %s\n", __FILE__, __LINE__, #cond);                                       \
      failures++;                                                                                  \
    }                                                                                              \
  } while (0)

#if defined(ZMIDI_IO) || defined(ZMIDI_BLE) || defined(ZMIDI_USB)
#error "ZMIDI_MIDI1 must not select a MIDI 2.0 part"
#endif
#if !defined(ZMIDI_IO_MIDI1) || !defined(ZMIDI_BLE_MIDI1) || !defined(ZMIDI_USB_MIDI1) || !defined(ZMIDI_MESSAGES) || \
    !defined(ZMIDI_CHANNEL_MODE) || !defined(ZMIDI_CONTROLLERS) || !defined(ZMIDI_PARAMETERS)
#error "ZMIDI_MIDI1 selects all MIDI 1.0 parts"
#endif

int main(void) {
  static const uint8_t note_on[] = {0x90, 0x3C, 0x40};
  static const ZmidiUsbLinkConfig link_config = {false, 0xFFFF, 64, 1};
  ZmidiBytesToUmp io;
  ZmidiBleReader ble;
  ZmidiUsbLink usb;
  uint32_t w[ZMIDI_BYTES_TO_UMP_MAX_WORDS];
  size_t i, n = 0;

  /* The readers have no translator: smaller storage, and no MIDI 2.0 Protocol. */
  CHECK(ZMIDI_BYTES_TO_UMP_WORDS == 4 && zmidi_bytes_to_ump_size() <= sizeof io);
  CHECK(ZMIDI_BLE_READER_WORDS == 5 && zmidi_ble_reader_size() <= sizeof ble);
  CHECK(zmidi_bytes_to_ump_init(&io, 0, 2) == ZMIDI_STATUS_NOT_SUPPORTED);
  CHECK(zmidi_ble_reader_init(&ble, 0, 2) == ZMIDI_STATUS_NOT_SUPPORTED);
  CHECK(zmidi_ble_reader_init(&ble, 0, 1) == ZMIDI_STATUS_OK);

  /* M2-104 D.2: a MIDI 1.0 Note On gives a Message Type 0x2 UMP. */
  CHECK(zmidi_bytes_to_ump_init(&io, 0, 1) == ZMIDI_STATUS_OK);
  for (i = 0; i < sizeof note_on; i++) {
    CHECK(zmidi_bytes_to_ump_push(&io, note_on[i], w, ZMIDI_BYTES_TO_UMP_MAX_WORDS, &n) == ZMIDI_STATUS_OK);
  }
  CHECK(n == 1 && w[0] == 0x20903C40u);

  /* The usb part starts on Alternate Setting 0. */
  CHECK(zmidi_usb_link_init(&usb, &link_config) == ZMIDI_STATUS_OK);
  CHECK(zmidi_usb_link_setting(&usb) == 0);

  if (failures != 0) {
    printf("midi1_only: %d failures\n", failures);
    return 1;
  }
  printf("midi1_only: ok\n");
  return 0;
}
