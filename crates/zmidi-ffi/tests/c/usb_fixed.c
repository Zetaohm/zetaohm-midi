/* C test of the fixed USB MIDI function (OPT-038, REQ-FFI-USB-011 and REQ-FFI-USB-012): the host
 * generator of zmidi_usb_gen.h and the firmware call zmidi_usb_fixed_gtb_setup. Vectors: USB-MIDI
 * v2.0 Appendix B, Tables B-3 to B-20, B-21 and B-22, B-23 and B-24. usb_fixed_appendix_b.h is the
 * output of the generator for the Appendix B function; this test makes it again and compares. */
#include "zmidi_usb_gen.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "usb_fixed_appendix_b.h"

static int failures;
#define CHECK(c) do { if (!(c)) { fprintf(stderr, "%s:%d: CHECK(%s)\n", __FILE__, __LINE__, #c); failures++; } } while (0)

/* Tables B-3 to B-20 (132 bytes), first and last descriptors, and Tables B-21 and B-22. */
/* Tables B-3 to B-20 of USB-MIDI v2.0 Appendix B (the same literal as usb_desc.c). */
static const uint8_t APPENDIX_B[132] = {
  0x09, 0x04, 0x00, 0x00, 0x00, 0x01, 0x01, 0x00, 0x00, /* B-3 standard AC interface */
  0x09, 0x24, 0x01, 0x00, 0x01, 0x09, 0x00, 0x01, 0x01, /* B-4 AC header */
  0x09, 0x04, 0x01, 0x00, 0x02, 0x01, 0x03, 0x00, 0x00, /* B-5 MS interface, alt 0 */
  0x07, 0x24, 0x01, 0x00, 0x01, 0x41, 0x00,             /* B-6 MS header, bcdMSC 0x0100 */
  0x06, 0x24, 0x02, 0x01, 0x01, 0x00,                   /* B-7 Embedded MIDI IN Jack 1 */
  0x06, 0x24, 0x02, 0x02, 0x02, 0x00,                   /* B-8 External MIDI IN Jack 2 */
  0x09, 0x24, 0x03, 0x01, 0x03, 0x01, 0x02, 0x01, 0x00, /* B-9 Embedded MIDI OUT Jack 3 */
  0x09, 0x24, 0x03, 0x02, 0x04, 0x01, 0x01, 0x01, 0x00, /* B-10 External MIDI OUT Jack 4 */
  0x09, 0x05, 0x01, 0x02, 0x40, 0x00, 0x00, 0x00, 0x00, /* B-11 bulk OUT endpoint 1 */
  0x05, 0x25, 0x01, 0x01, 0x01,                         /* B-12 MS_GENERAL, Jack 1 */
  0x09, 0x05, 0x81, 0x02, 0x40, 0x00, 0x00, 0x00, 0x00, /* B-13 bulk IN endpoint 1 */
  0x05, 0x25, 0x01, 0x01, 0x03,                         /* B-14 MS_GENERAL, Jack 3 */
  0x09, 0x04, 0x01, 0x01, 0x02, 0x01, 0x03, 0x00, 0x00, /* B-15 MS interface, alt 1 */
  0x07, 0x24, 0x01, 0x00, 0x02, 0x07, 0x00,             /* B-16 MS header, bcdMSC 0x0200 */
  0x07, 0x05, 0x01, 0x02, 0x40, 0x00, 0x00,             /* B-17 bulk OUT endpoint 1 */
  0x05, 0x25, 0x02, 0x01, 0x01,                         /* B-18 MS_GENERAL_2_0, block 1 */
  0x07, 0x05, 0x81, 0x03, 0x40, 0x00, 0x01,             /* B-19 interrupt IN endpoint 1 */
  0x05, 0x25, 0x02, 0x01, 0x01,                         /* B-20 MS_GENERAL_2_0, block 1 */
};
static const uint8_t B21_B22[18] = {
  0x05, 0x26, 0x01, 0x12, 0x00,
  0x0D, 0x26, 0x02, 0x01, 0x00, 0x00, 0x01, 0x04, 0x00, 0x01, 0x00, 0x00, 0x00,
};
static const uint8_t B23[8] = {0x81, 0x06, 0x01, 0x26, 0x01, 0x00, 0x05, 0x00};
static const uint8_t B24[8] = {0x81, 0x06, 0x01, 0x26, 0x01, 0x00, 0x12, 0x00};

static ZmidiUsbFunction fn;

/* The Appendix B function with the run-time calls (as tests/c/usb_desc.c). */
static void appendix_b(void) {
  ZmidiUsbFunctionConfig c = {0, 1, false, 0, 0, 1, 1, 1, 1, 0};
  ZmidiUsbEndpoint out, in;
  ZmidiUsbBlock b22 = {1, 0, 0, 1, 4, 0x00, 0x0001, 0x0000};
  uint8_t alt = 0;
  memset(&out, 0, sizeof out);
  out.direction = 0;
  out.number = 1;
  out.transfer = 2;
  out.max_packet = 64;
  out.block_count = 1;
  out.block_ids[0] = 1;
  in = out;
  in.direction = 1;
  in.transfer = 3;
  in.interval = 1;
  CHECK(zmidi_usb_function_init(&fn, &c) == ZMIDI_STATUS_OK);
  CHECK(zmidi_usb_function_add_setting(&fn, 0, &alt) == ZMIDI_STATUS_OK && alt == 1);
  CHECK(zmidi_usb_function_add_endpoint(&fn, 1, &out) == ZMIDI_STATUS_OK);
  CHECK(zmidi_usb_function_add_endpoint(&fn, 1, &in) == ZMIDI_STATUS_OK);
  CHECK(zmidi_usb_function_set_blocks(&fn, 1, &b22, 1) == ZMIDI_STATUS_OK);
}

/* The whole file at `path`, NUL-terminated, or NULL. */
static char *read_file(const char *path, size_t *len) {
  FILE *f = fopen(path, "rb");
  char *buf;
  long n;
  if (f == NULL) {
    return NULL;
  }
  fseek(f, 0, SEEK_END);
  n = ftell(f);
  fseek(f, 0, SEEK_SET);
  buf = malloc((size_t)n + 1);
  if (buf != NULL && fread(buf, 1, (size_t)n, f) == (size_t)n) {
    buf[n] = 0;
    *len = (size_t)n;
  } else {
    free(buf);
    buf = NULL;
  }
  fclose(f);
  return buf;
}

static void fixed_answer(const uint8_t setup[8], uint8_t ms, size_t capacity, ZmidiStatus status,
                         ZmidiUsbGtbResponse response, size_t n) {
  uint8_t out[64];
  ZmidiUsbGtbResponse r = ZMIDI_USB_GTB_RESPONSE_ANSWERED;
  size_t written = 99;
  memset(out, 0xEE, sizeof out);
  CHECK(zmidi_usb_fixed_gtb_setup(appendix_b_gtb_sets, appendix_b_gtb_lens, appendix_b_GTB_COUNT, ms,
                                  setup, out, capacity, &r, &written) == status);
  if (status == ZMIDI_STATUS_OK) {
    CHECK(r == response && written == n);
    CHECK(memcmp(out, B21_B22, n) == 0);
  } else {
    CHECK(written == 99);
  }
  CHECK(out[n] == 0xEE); /* nothing after the answer */
}

int main(void) {
  uint8_t desc[256];
  size_t written = 0, len = 0, glen = 0, i;
  ZmidiUsbSettingEndpoint e;
  char golden_path[1024];
  char *golden, *made;
  FILE *tmp;

  /* REQ-FFI-USB-012: the generated arrays are the bytes of the run-time writer and of Appendix B. */
  appendix_b();
  CHECK(zmidi_usb_function_write(&fn, desc, sizeof desc, &written) == ZMIDI_STATUS_OK);
  CHECK(written == sizeof appendix_b_descriptors);
  CHECK(memcmp(desc, appendix_b_descriptors, written) == 0);
  CHECK(sizeof appendix_b_descriptors == sizeof APPENDIX_B);
  CHECK(memcmp(appendix_b_descriptors, APPENDIX_B, sizeof APPENDIX_B) == 0);
  CHECK(appendix_b_GTB_COUNT == 1 && appendix_b_gtb_lens[0] == sizeof B21_B22);
  CHECK(memcmp(appendix_b_gtb_sets[0], B21_B22, sizeof B21_B22) == 0);
  CHECK(appendix_b_ENDPOINT_COUNT == zmidi_usb_function_endpoint_count(&fn));
  for (i = 0; i < appendix_b_ENDPOINT_COUNT; i++) {
    CHECK(zmidi_usb_function_endpoint(&fn, i, &e) == ZMIDI_STATUS_OK);
    CHECK(memcmp(&e, &appendix_b_endpoints[i], sizeof e) == 0);
  }
  CHECK(appendix_b_ALTERNATE_SETTINGS == 2);
  CHECK(appendix_b_group_masks_out[0] == 0xFFFF && appendix_b_group_masks_out[1] == 0x0001);
  CHECK(appendix_b_group_masks_in[0] == 0xFFFF && appendix_b_group_masks_in[1] == 0x0001);

  /* The generator writes the checked-in header again, byte for byte. */
  tmp = tmpfile();
  CHECK(tmp != NULL);
  if (tmp != NULL) {
    CHECK(zmidi_usb_function_print_c(&fn, tmp, "appendix_b") == ZMIDI_STATUS_OK);
    len = (size_t)ftell(tmp);
    made = malloc(len + 1);
    rewind(tmp);
    CHECK(made != NULL && fread(made, 1, len, tmp) == len);
    fclose(tmp);
    strncpy(golden_path, __FILE__, sizeof golden_path - 1);
    golden_path[sizeof golden_path - 1] = 0;
    {
      char *slash = strrchr(golden_path, '/');
      strcpy(slash ? slash + 1 : golden_path, "usb_fixed_appendix_b.h");
    }
    golden = read_file(golden_path, &glen);
    CHECK(golden != NULL);
    CHECK(made != NULL && golden != NULL && glen == len && memcmp(made, golden, len) == 0);
    free(made);
    free(golden);
  }
  /* A broken rule gives its status, and nothing else; null arguments are refused. */
  CHECK(zmidi_usb_function_print_c(NULL, stdout, "x") == ZMIDI_STATUS_INVALID_ARGUMENT);
  {
    ZmidiUsbFunctionConfig bad = {1, 1, false, 0, 0, 1, 1, 1, 1, 0}; /* the same interface twice */
    ZmidiUsbFunction g;
    tmp = tmpfile();
    CHECK(zmidi_usb_function_init(&g, &bad) == ZMIDI_STATUS_OK);
    if (tmp != NULL) {
      CHECK(zmidi_usb_function_print_c(&g, tmp, "bad") == ZMIDI_STATUS_REJECTED);
      CHECK(ftell(tmp) == 0);
      fclose(tmp);
    }
  }

  /* REQ-FFI-USB-011: the firmware side, with the generated sets. The same answers as
   * zmidi_usb_gtb_setup in tests/c/usb_desc.c. */
  fixed_answer(B23, 1, 64, ZMIDI_STATUS_OK, ZMIDI_USB_GTB_RESPONSE_ANSWERED, 5);
  fixed_answer(B24, 1, 64, ZMIDI_STATUS_OK, ZMIDI_USB_GTB_RESPONSE_ANSWERED, 18);
  fixed_answer(B24, 1, 17, ZMIDI_STATUS_BUFFER_TOO_SMALL, ZMIDI_USB_GTB_RESPONSE_ANSWERED, 0);
  {
    uint8_t s[8];
    memcpy(s, B24, 8);
    s[6] = 0x40; /* wLength above wTotalLength: the whole set only */
    fixed_answer(s, 1, 64, ZMIDI_STATUS_OK, ZMIDI_USB_GTB_RESPONSE_ANSWERED, 18);
    fixed_answer(B24, 2, 64, ZMIDI_STATUS_OK, ZMIDI_USB_GTB_RESPONSE_STALL, 0); /* another interface */
    memcpy(s, B24, 8);
    s[5] = 0x01; /* wIndex 0x0101: the 16-bit compare */
    fixed_answer(s, 1, 64, ZMIDI_STATUS_OK, ZMIDI_USB_GTB_RESPONSE_STALL, 0);
    memcpy(s, B24, 8);
    s[2] = 0x00; /* Alternate Setting 0 */
    fixed_answer(s, 1, 64, ZMIDI_STATUS_OK, ZMIDI_USB_GTB_RESPONSE_STALL, 0);
    s[2] = 0x02; /* no set for Alternate Setting 2 */
    fixed_answer(s, 1, 64, ZMIDI_STATUS_OK, ZMIDI_USB_GTB_RESPONSE_STALL, 0);
  }
  {
    static const uint8_t device[8] = {0x80, 0x06, 0x00, 0x01, 0x00, 0x00, 0x12, 0x00};
    static const uint8_t other[8] = {0x21, 0x06, 0x01, 0x26, 0x01, 0x00, 0x12, 0x00};
    fixed_answer(device, 1, 64, ZMIDI_STATUS_OK, ZMIDI_USB_GTB_RESPONSE_NOT_HANDLED, 0);
    fixed_answer(other, 1, 64, ZMIDI_STATUS_OK, ZMIDI_USB_GTB_RESPONSE_NOT_HANDLED, 0);
  }
  {
    /* No set at all: every Group Terminal Block request stalls. Bad arguments. */
    uint8_t out[64];
    ZmidiUsbGtbResponse r;
    size_t n = 0;
    const uint8_t *three[3] = {B21_B22, B21_B22, B21_B22};
    size_t lens[3] = {18, 18, 18};
    const uint8_t *hole[1] = {NULL};
    CHECK(zmidi_usb_fixed_gtb_setup(NULL, NULL, 0, 1, B24, out, sizeof out, &r, &n) == ZMIDI_STATUS_OK);
    CHECK(r == ZMIDI_USB_GTB_RESPONSE_STALL && n == 0);
    CHECK(zmidi_usb_fixed_gtb_setup(three, lens, 3, 1, B24, out, sizeof out, &r, &n) == ZMIDI_STATUS_INVALID_ARGUMENT);
    CHECK(zmidi_usb_fixed_gtb_setup(hole, lens, 1, 1, B24, out, sizeof out, &r, &n) == ZMIDI_STATUS_INVALID_ARGUMENT);
    CHECK(zmidi_usb_fixed_gtb_setup(NULL, lens, 1, 1, B24, out, sizeof out, &r, &n) == ZMIDI_STATUS_INVALID_ARGUMENT);
    CHECK(zmidi_usb_fixed_gtb_setup(three, lens, 2, 1, NULL, out, sizeof out, &r, &n) == ZMIDI_STATUS_INVALID_ARGUMENT);
    CHECK(zmidi_usb_fixed_gtb_setup(three, lens, 2, 1, B24, NULL, sizeof out, &r, &n) == ZMIDI_STATUS_INVALID_ARGUMENT);
    CHECK(zmidi_usb_fixed_gtb_setup(three, lens, 2, 1, B24, out, sizeof out, NULL, &n) == ZMIDI_STATUS_INVALID_ARGUMENT);
    CHECK(zmidi_usb_fixed_gtb_setup(three, lens, 2, 1, B24, out, sizeof out, &r, NULL) == ZMIDI_STATUS_INVALID_ARGUMENT);
    /* Two sets: Alternate Setting 2 answers from the second set. */
    {
      uint8_t s[8];
      memcpy(s, B24, 8);
      s[2] = 0x02;
      CHECK(zmidi_usb_fixed_gtb_setup(three, lens, 2, 1, s, out, sizeof out, &r, &n) == ZMIDI_STATUS_OK);
      CHECK(r == ZMIDI_USB_GTB_RESPONSE_ANSWERED && n == 18);
    }
  }

  if (failures == 0) {
    printf("usb_fixed: all checks passed\n");
  }
  return failures ? 1 : 0;
}
