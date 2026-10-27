/* C test of the ci part of the C ABI (REQ-FFI-CI-007, REQ-FFI-CI-014): two devices, A (Initiator)
 * and B (Responder), talk through record buffers [group][len lo][len hi][bytes]. M2-101 (Discovery,
 * Endpoint Information, PE Capabilities, Profile Configuration in section 7, Process Inquiry in
 * section 9) and M2-103 (Get and Set Property Data in one Chunk, status 200 and 404, ResourceList).
 * The Profile and Process Inquiry messages are compared byte for byte with M2-101 Tables 17 to 28
 * and 40 to 46 (SysEx7 payload: no F0 and F7). */
#ifndef ZMIDI_CI
#define ZMIDI_CI
#endif
#include "zmidi.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int failures;
#define CHECK(c) do { if (!(c)) { fprintf(stderr, "%s:%d: CHECK(%s)\n", __FILE__, __LINE__, #c); failures++; } } while (0)

static ZmidiCi a, b;
static uint8_t msgs[2048], replies[2048], scratch[1024];
static ZmidiCiEvent ev;

/* Gives each record of `recs` to `dev`; the replies of all go to `rep`. Gives the record count. */
static size_t deliver(ZmidiCi *dev, const uint8_t *recs, size_t n, uint8_t *rep, size_t *rep_n) {
  size_t at = 0, count = 0;
  *rep_n = 0;
  while (at + 3 <= n) {
    uint8_t g = recs[at];
    size_t len = (size_t)recs[at + 1] | ((size_t)recs[at + 2] << 8), k = 0;
    CHECK(zmidi_ci_receive(dev, g, recs + at + 3, len, 1000, rep + *rep_n, 2048 - *rep_n, &k, &ev) == ZMIDI_STATUS_OK);
    *rep_n += k;
    at += 3 + len;
    count++;
  }
  return count;
}

/* M2-101 3.2: a MUID as 4 bytes of 7 bits, LSB first. */
static void muid_bytes(uint8_t *p, uint32_t m) {
  p[0] = (uint8_t)(m & 0x7F); p[1] = (uint8_t)((m >> 7) & 0x7F);
  p[2] = (uint8_t)((m >> 14) & 0x7F); p[3] = (uint8_t)((m >> 21) & 0x7F);
}

/* M2-101 Table 3: the 13-byte header of a MIDI-CI message, Message Version 0x02. Gives 13. */
static size_t ci_header(uint8_t *p, uint8_t device_id, uint8_t sub_id2, uint32_t src, uint32_t dst) {
  p[0] = 0x7E; p[1] = device_id; p[2] = 0x0D; p[3] = sub_id2; p[4] = 0x02;
  muid_bytes(p + 5, src);
  muid_bytes(p + 9, dst);
  return 13;
}

/* True when `recs` holds one record on `group` with the `n` bytes of `want`. */
static int one_record(const uint8_t *recs, size_t recs_n, uint8_t group, const uint8_t *want, size_t n) {
  return recs_n == 3 + n && recs[0] == group && recs[1] == (uint8_t)n && recs[2] == (uint8_t)(n >> 8) &&
         memcmp(recs + 3, want, n) == 0;
}

/* M2-101 Table 33 (Message Version 0x02): a Reply to Get Property Data Chunk from `src` to `dst`,
 * with the Header Data `hdr` (or none) and the Property Data `data`. Gives the byte count. */
static size_t get_reply_chunk(uint8_t *p, uint32_t src, uint32_t dst, uint8_t rid, const char *hdr, uint16_t total,
                              uint16_t number, const char *data) {
  size_t at = ci_header(p, 0x7F, 0x35, src, dst), hn = hdr ? strlen(hdr) : 0, dn = strlen(data);
  p[at++] = rid;
  p[at++] = (uint8_t)(hn & 0x7F); p[at++] = (uint8_t)(hn >> 7);
  memcpy(p + at, hdr, hn); at += hn;
  p[at++] = (uint8_t)(total & 0x7F); p[at++] = (uint8_t)(total >> 7);
  p[at++] = (uint8_t)(number & 0x7F); p[at++] = (uint8_t)(number >> 7);
  p[at++] = (uint8_t)(dn & 0x7F); p[at++] = (uint8_t)(dn >> 7);
  memcpy(p + at, data, dn); at += dn;
  return at;
}

#define BROADCAST 0x0FFFFFFFu

static const char list[] = "[{\"resource\":\"DeviceInfo\"},{\"resource\":\"X-Volume\",\"canSet\":\"full\"}]";

static ZmidiCiConfig config(void) {
  ZmidiCiConfig c;
  memset(&c, 0, sizeof c);
  c.manufacturer[0] = 0x00; c.manufacturer[1] = 0x21; c.manufacturer[2] = 0x7F;
  c.family = 1; c.model = 2; c.revision[2] = 1;
  c.max_sysex_size = 512;
  c.group_count = 1;
  c.channels_in_use = 1;
  c.product_instance_id = (const uint8_t *)"zmidi-c-test";
  c.property_exchange = true;
  c.resource_list = (const uint8_t *)list;
  c.resource_list_len = sizeof list - 1;
  return c;
}

/* B answers the pending request; A reads the reply. */
static void answer(uint16_t status, const char *data) {
  size_t n = 0, rn = 0;
  CHECK(zmidi_ci_pe_reply(&b, status, (const uint8_t *)data, data ? strlen(data) : 0, scratch, sizeof scratch, msgs, sizeof msgs, &n) == ZMIDI_STATUS_OK);
  deliver(&a, msgs, n, replies, &rn);
}

/* M2-101 7 (Profile Configuration) and 9 (Process Inquiry) between A and B (REQ-FFI-CI-014). */
static void profiles_and_process(void) {
  ZmidiCiConfig c = config();
  ZmidiCiProfileEvent pe;
  ZmidiCiProcessEvent pi;
  ZmidiCiProfile entry;
  uint8_t want[64], ids[10];
  size_t n = 0, rn = 0, k = 0, count = 0;
  uint32_t ma, mb;
  /* A Standard Defined Profile (Table 19): Bank 0, Number 1, Version 1, Level 0x01 Minimum. */
  static const uint8_t prof[5] = {0x7E, 0x00, 0x01, 0x01, 0x01};
  static const uint8_t detail[2] = {0x01, 0x02};
  static const uint8_t psd[3] = {0x10, 0x20, 0x30};
  ZmidiCiReport requested = {0x7F, 0x01, 0x02, 0x01};
  ZmidiCiReport supported = {0x00, 0x00, 0x03, 0x00};
  ZmidiCiReport bad = {0x80, 0, 0, 0};

  c.profiles = true;
  c.process_inquiry = true;
  CHECK(zmidi_ci_init(&a, &c, 11) == ZMIDI_STATUS_OK);
  CHECK(zmidi_ci_init(&b, &c, 12) == ZMIDI_STATUS_OK);
  ma = zmidi_ci_muid(&a);
  mb = zmidi_ci_muid(&b);
  CHECK(zmidi_ci_discovery(&a, 0, 0, msgs, sizeof msgs, &n) == ZMIDI_STATUS_OK);
  deliver(&b, msgs, n, replies, &rn);
  deliver(&a, replies, rn, msgs, &n);
  CHECK(ev.kind == ZMIDI_CI_EVENT_KIND_DISCOVERED);
  /* Discovered is not a Profile or Process Inquiry event. */
  CHECK(zmidi_ci_profile_event(&a, &pe) == ZMIDI_STATUS_NO_PENDING);
  CHECK(zmidi_ci_process_event(&a, &pi) == ZMIDI_STATUS_NO_PENDING);

  /* B adds a disabled Profile on Channel 1: Profile Added Report to Broadcast (Table 20). */
  memset(&entry, 0, sizeof entry);
  entry.device_id = 0x00;
  memcpy(entry.profile, prof, 5);
  entry.max_channels = 1;
  CHECK(zmidi_ci_add_profile(&b, &entry, msgs, ZMIDI_CI_REPORT_BUFFER - 1, &n) == ZMIDI_STATUS_BUFFER_TOO_SMALL);
  CHECK(zmidi_ci_add_profile(&b, &entry, msgs, sizeof msgs, &n) == ZMIDI_STATUS_OK);
  k = ci_header(want, 0x00, 0x26, mb, BROADCAST);
  memcpy(want + k, prof, 5); k += 5;
  CHECK(one_record(msgs, n, 0, want, k));
  CHECK(zmidi_ci_add_profile(&b, &entry, msgs, sizeof msgs, &n) == ZMIDI_STATUS_REJECTED);
  entry.device_id = 0x10; /* reserved Device ID (Table 4) */
  CHECK(zmidi_ci_add_profile(&b, &entry, msgs, sizeof msgs, &n) == ZMIDI_STATUS_INVALID_ARGUMENT);

  /* Set Profile On before Profile Inquiry: rejected (M2-101 2.2). */
  CHECK(zmidi_ci_set_profile_on(&a, mb, 0, 0x00, prof, 1, 0, msgs, sizeof msgs, &n) == ZMIDI_STATUS_REJECTED);

  /* Profile Inquiry (Table 17), the automatic Reply to Profile Inquiry (Table 18). */
  CHECK(zmidi_ci_profile_inquiry(&a, mb, 0, 0x00, msgs, sizeof msgs, &n) == ZMIDI_STATUS_OK);
  k = ci_header(want, 0x00, 0x20, ma, mb);
  CHECK(one_record(msgs, n, 0, want, k));
  deliver(&b, msgs, n, replies, &rn);
  k = ci_header(want, 0x00, 0x21, mb, ma);
  want[k++] = 0; want[k++] = 0; want[k++] = 1; want[k++] = 0;
  memcpy(want + k, prof, 5); k += 5;
  CHECK(one_record(replies, rn, 0, want, k));
  deliver(&a, replies, rn, msgs, &n);
  CHECK(ev.kind == ZMIDI_CI_EVENT_KIND_PROFILE_INQUIRY_REPLY && ev.muid == mb);
  CHECK(zmidi_ci_profile_event(&a, &pe) == ZMIDI_STATUS_OK);
  CHECK(pe.device_id == 0x00 && pe.enabled_count == 0 && pe.disabled_count == 1);
  CHECK(zmidi_ci_profile_ids(&a, true, NULL, 0, &count) == ZMIDI_STATUS_OK && count == 0);
  CHECK(zmidi_ci_profile_ids(&a, false, ids, 4, &count) == ZMIDI_STATUS_BUFFER_TOO_SMALL && count == 1);
  CHECK(zmidi_ci_profile_ids(&a, false, ids, sizeof ids, &count) == ZMIDI_STATUS_OK && count == 1);
  CHECK(memcmp(ids, prof, 5) == 0);

  /* Set Profile On (Table 24): Level 0x7F, 1 Channel; the SetProfile event; B enables the Profile
   * and sends the Profile Enabled Report (Table 26). */
  CHECK(zmidi_ci_set_profile_on(&a, mb, 0, 0x00, prof, 1, 0, msgs, sizeof msgs, &n) == ZMIDI_STATUS_OK);
  k = ci_header(want, 0x00, 0x22, ma, mb);
  memcpy(want + k, prof, 4); want[k + 4] = 0x7F; k += 5;
  want[k++] = 0x01; want[k++] = 0x00;
  CHECK(one_record(msgs, n, 0, want, k));
  deliver(&b, msgs, n, replies, &rn);
  CHECK(ev.kind == ZMIDI_CI_EVENT_KIND_SET_PROFILE && ev.muid == ma && rn == 0);
  CHECK(zmidi_ci_profile_event(&b, &pe) == ZMIDI_STATUS_OK);
  CHECK(pe.on && pe.channels == 1 && pe.device_id == 0x00 && pe.group == 0 && memcmp(pe.profile, prof, 4) == 0);
  CHECK(zmidi_ci_set_profile_state(&b, pe.device_id, pe.group, pe.profile, true, pe.channels, msgs, sizeof msgs, &n) == ZMIDI_STATUS_OK);
  k = ci_header(want, 0x00, 0x24, mb, BROADCAST);
  memcpy(want + k, prof, 5); k += 5;
  want[k++] = 0x01; want[k++] = 0x00;
  CHECK(one_record(msgs, n, 0, want, k));
  deliver(&a, msgs, n, replies, &rn);
  CHECK(ev.kind == ZMIDI_CI_EVENT_KIND_PROFILE_REPORT);
  CHECK(zmidi_ci_profile_event(&a, &pe) == ZMIDI_STATUS_OK);
  CHECK(pe.report == ZMIDI_CI_PROFILE_REPORT_ENABLED && pe.channels == 1 && memcmp(pe.profile, prof, 5) == 0);

  /* Profile Details Inquiry (Table 22), Inquiry Target 0x00: B replies by itself with the Number of
   * MIDI Channels (M2-102 2.5.1): 1 in use, maximum 1. */
  CHECK(zmidi_ci_profile_details_inquiry(&a, mb, 0, 0x00, prof, 0x00, msgs, sizeof msgs, &n) == ZMIDI_STATUS_OK);
  k = ci_header(want, 0x00, 0x28, ma, mb);
  memcpy(want + k, prof, 5); k += 5;
  want[k++] = 0x00;
  CHECK(one_record(msgs, n, 0, want, k));
  deliver(&b, msgs, n, replies, &rn);
  deliver(&a, replies, rn, msgs, &n);
  CHECK(ev.kind == ZMIDI_CI_EVENT_KIND_PROFILE_DETAILS_REPLY);
  CHECK(zmidi_ci_profile_event(&a, &pe) == ZMIDI_STATUS_OK);
  CHECK(pe.target == 0 && !pe.notification && pe.data_len == 4);
  CHECK(pe.data[0] == 1 && pe.data[1] == 0 && pe.data[2] == 1 && pe.data[3] == 0);

  /* Inquiry Target 0x40 (Profile Specific): the program of B answers (Table 23). */
  CHECK(zmidi_ci_profile_details_inquiry(&a, mb, 0, 0x00, prof, 0x40, msgs, sizeof msgs, &n) == ZMIDI_STATUS_OK);
  deliver(&b, msgs, n, replies, &rn);
  CHECK(ev.kind == ZMIDI_CI_EVENT_KIND_PROFILE_DETAILS_INQUIRY && rn == 0);
  CHECK(zmidi_ci_profile_event(&b, &pe) == ZMIDI_STATUS_OK && pe.target == 0x40);
  CHECK(zmidi_ci_profile_details_reply(&b, ma, 0, pe.device_id, pe.profile, pe.target, detail, 2, msgs, sizeof msgs, &n) == ZMIDI_STATUS_OK);
  k = ci_header(want, 0x00, 0x29, mb, ma);
  memcpy(want + k, prof, 5); k += 5;
  want[k++] = 0x40; want[k++] = 0x02; want[k++] = 0x00; want[k++] = 0x01; want[k++] = 0x02;
  CHECK(one_record(msgs, n, 0, want, k));
  deliver(&a, msgs, n, replies, &rn);
  CHECK(zmidi_ci_profile_event(&a, &pe) == ZMIDI_STATUS_OK);
  CHECK(pe.target == 0x40 && !pe.notification && pe.data_len == 2 && memcmp(pe.data, detail, 2) == 0);

  /* Profile Specific Data (Table 28): a 4-byte length, LSB first. */
  CHECK(zmidi_ci_profile_specific_data(&a, mb, 0, 0x00, prof, psd, 3, msgs, sizeof msgs, &n) == ZMIDI_STATUS_OK);
  k = ci_header(want, 0x00, 0x2F, ma, mb);
  memcpy(want + k, prof, 5); k += 5;
  want[k++] = 3; want[k++] = 0; want[k++] = 0; want[k++] = 0;
  memcpy(want + k, psd, 3); k += 3;
  CHECK(one_record(msgs, n, 0, want, k));
  deliver(&b, msgs, n, replies, &rn);
  CHECK(ev.kind == ZMIDI_CI_EVENT_KIND_PROFILE_SPECIFIC_DATA);
  CHECK(zmidi_ci_profile_event(&b, &pe) == ZMIDI_STATUS_OK && pe.data_len == 3 && memcmp(pe.data, psd, 3) == 0);

  /* Set Profile Off (Table 25), then the Profile Disabled Report (Table 27). */
  CHECK(zmidi_ci_set_profile_off(&a, mb, 0, 0x00, prof, msgs, sizeof msgs, &n) == ZMIDI_STATUS_OK);
  k = ci_header(want, 0x00, 0x23, ma, mb);
  memcpy(want + k, prof, 5); k += 5;
  want[k++] = 0; want[k++] = 0;
  CHECK(one_record(msgs, n, 0, want, k));
  deliver(&b, msgs, n, replies, &rn);
  CHECK(zmidi_ci_profile_event(&b, &pe) == ZMIDI_STATUS_OK && !pe.on);
  CHECK(zmidi_ci_set_profile_state(&b, 0x00, 0, prof, false, 1, msgs, sizeof msgs, &n) == ZMIDI_STATUS_OK);
  k = ci_header(want, 0x00, 0x25, mb, BROADCAST);
  memcpy(want + k, prof, 5); k += 5;
  want[k++] = 0x01; want[k++] = 0x00;
  CHECK(one_record(msgs, n, 0, want, k));
  deliver(&a, msgs, n, replies, &rn);
  CHECK(zmidi_ci_profile_event(&a, &pe) == ZMIDI_STATUS_OK && pe.report == ZMIDI_CI_PROFILE_REPORT_DISABLED);

  /* B removes the disabled Profile: Profile Removed Report only (Table 21). */
  CHECK(zmidi_ci_remove_profile(&b, 0x00, 0, prof, msgs, sizeof msgs, &n) == ZMIDI_STATUS_OK);
  k = ci_header(want, 0x00, 0x27, mb, BROADCAST);
  memcpy(want + k, prof, 5); k += 5;
  CHECK(one_record(msgs, n, 0, want, k));
  CHECK(zmidi_ci_remove_profile(&b, 0x00, 0, prof, msgs, sizeof msgs, &n) == ZMIDI_STATUS_REJECTED);
  CHECK(zmidi_ci_set_profile_state(&b, 0x00, 0, prof, true, 1, msgs, sizeof msgs, &n) == ZMIDI_STATUS_REJECTED);

  /* Process Inquiry Capabilities (Table 40) and the automatic reply (Table 41: D0 set). */
  CHECK(zmidi_ci_pi_capabilities_inquiry(&a, mb, msgs, sizeof msgs, &n) == ZMIDI_STATUS_OK);
  k = ci_header(want, 0x7F, 0x40, ma, mb);
  CHECK(one_record(msgs, n, 0, want, k));
  deliver(&b, msgs, n, replies, &rn);
  k = ci_header(want, 0x7F, 0x41, mb, ma);
  want[k++] = 0x01;
  CHECK(one_record(replies, rn, 0, want, k));
  deliver(&a, replies, rn, msgs, &n);
  CHECK(ev.kind == ZMIDI_CI_EVENT_KIND_PI_CAPABILITIES);
  CHECK(zmidi_ci_process_event(&a, &pi) == ZMIDI_STATUS_OK && pi.midi_message_report);
  CHECK(zmidi_ci_profile_event(&a, &pe) == ZMIDI_STATUS_NO_PENDING);

  /* Inquiry: MIDI Message Report (Table 43): full report of MTC Quarter Frame, Control Change
   * and Notes. */
  CHECK(!zmidi_ci_report_open(&a, mb));
  CHECK(zmidi_ci_message_report_inquiry(&a, mb, 0, 0x7F, &bad, msgs, sizeof msgs, &n) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_ci_message_report_inquiry(&a, mb, 0, 0x7F, &requested, msgs, sizeof msgs, &n) == ZMIDI_STATUS_OK);
  k = ci_header(want, 0x7F, 0x42, ma, mb);
  want[k++] = 0x7F; want[k++] = 0x01; want[k++] = 0x00; want[k++] = 0x02; want[k++] = 0x01;
  CHECK(one_record(msgs, n, 0, want, k));
  CHECK(zmidi_ci_report_open(&a, mb));
  deliver(&b, msgs, n, replies, &rn);
  CHECK(ev.kind == ZMIDI_CI_EVENT_KIND_MESSAGE_REPORT && rn == 0);
  CHECK(zmidi_ci_process_event(&b, &pi) == ZMIDI_STATUS_OK);
  CHECK(pi.device_id == 0x7F && pi.report.data_control == 0x7F && pi.report.system == 0x01 && pi.report.channel == 0x02 && pi.report.note == 0x01);

  /* Reply to MIDI Message Report (Table 45): only the bits both requested and supported. */
  CHECK(zmidi_ci_message_report_reply(&b, ma, 0, pi.device_id, &pi.report, &supported, msgs, sizeof msgs, &n) == ZMIDI_STATUS_OK);
  k = ci_header(want, 0x7F, 0x43, mb, ma);
  want[k++] = 0x00; want[k++] = 0x00; want[k++] = 0x02; want[k++] = 0x00;
  CHECK(one_record(msgs, n, 0, want, k));
  deliver(&a, msgs, n, replies, &rn);
  CHECK(ev.kind == ZMIDI_CI_EVENT_KIND_MESSAGE_REPORT_REPLY);
  CHECK(zmidi_ci_process_event(&a, &pi) == ZMIDI_STATUS_OK && pi.report.channel == 0x02 && pi.report.system == 0);

  /* End of MIDI Message Report (Table 46) closes the report. */
  CHECK(zmidi_ci_message_report_end(&b, ma, 0, 0x7F, msgs, sizeof msgs, &n) == ZMIDI_STATUS_OK);
  k = ci_header(want, 0x7F, 0x44, mb, ma);
  CHECK(one_record(msgs, n, 0, want, k));
  deliver(&a, msgs, n, replies, &rn);
  CHECK(ev.kind == ZMIDI_CI_EVENT_KIND_MESSAGE_REPORT_END && !zmidi_ci_report_open(&a, mb));

  /* Abort: NAK (Table 15) for Sub-ID#2 0x42 with Status Code 0x20 (M2-101 9.8.1). */
  CHECK(zmidi_ci_abort_report(&b, ma, 0, 0x7F, false, msgs, sizeof msgs, &n) == ZMIDI_STATUS_OK);
  CHECK(n > 3 + 15 && msgs[3 + 3] == 0x7F && msgs[3 + 13] == 0x42 && msgs[3 + 14] == 0x20);

  /* An enabled Profile on Group 15: the Profile Added Report, then the Profile Enabled Report with
   * its Channels (Tables 20, 26). Group 16 and a channel count over 14 bits are not valid. */
  memset(&entry, 0, sizeof entry);
  entry.device_id = 0x00; entry.group = 15;
  memcpy(entry.profile, prof, 5);
  entry.enabled = true; entry.channels = 1; entry.max_channels = 1;
  CHECK(zmidi_ci_add_profile(&b, &entry, msgs, sizeof msgs, &n) == ZMIDI_STATUS_OK);
  k = ci_header(want, 0x00, 0x26, mb, BROADCAST);
  memcpy(want + k, prof, 5); k += 5;
  CHECK(n > 3 + k && msgs[0] == 15 && msgs[1] == (uint8_t)k && memcmp(msgs + 3, want, k) == 0);
  {
    size_t first = 3 + k;
    k = ci_header(want, 0x00, 0x24, mb, BROADCAST);
    memcpy(want + k, prof, 5); k += 5;
    want[k++] = 0x01; want[k++] = 0x00;
    CHECK(one_record(msgs + first, n - first, 15, want, k));
  }
  CHECK(zmidi_ci_remove_profile(&b, 0x00, 15, prof, msgs, sizeof msgs, &n) == ZMIDI_STATUS_OK);
  entry.group = 16;
  CHECK(zmidi_ci_add_profile(&b, &entry, msgs, sizeof msgs, &n) == ZMIDI_STATUS_INVALID_ARGUMENT);
  entry.group = 0; entry.channels = 0x4000;
  CHECK(zmidi_ci_add_profile(&b, &entry, msgs, sizeof msgs, &n) == ZMIDI_STATUS_INVALID_ARGUMENT);
  entry.channels = 1; entry.max_channels = 0x4000;
  CHECK(zmidi_ci_add_profile(&b, &entry, msgs, sizeof msgs, &n) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_ci_profile_inquiry(&a, mb, 16, 0x00, msgs, sizeof msgs, &n) == ZMIDI_STATUS_INVALID_ARGUMENT);

  /* Records on another Group; bad Groups, Device IDs and pointers. */
  CHECK(zmidi_ci_message_report_end(&b, ma, 5, 0x7F, msgs, sizeof msgs, &n) == ZMIDI_STATUS_OK && msgs[0] == 5);
  CHECK(zmidi_ci_message_report_end(&b, ma, 16, 0x7F, msgs, sizeof msgs, &n) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_ci_profile_inquiry(&a, mb, 0, 0x10, msgs, sizeof msgs, &n) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_ci_set_profile_on(&a, mb, 0, 0x00, NULL, 1, 0, msgs, sizeof msgs, &n) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_ci_profile_inquiry(&a, mb, 0, 0x00, msgs, 4, &n) == ZMIDI_STATUS_BUFFER_TOO_SMALL);
  CHECK(zmidi_ci_add_profile(&b, NULL, msgs, sizeof msgs, &n) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_ci_profile_event(&a, NULL) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_ci_process_event(NULL, &pi) == ZMIDI_STATUS_INVALID_ARGUMENT);
}

int main(void) {
  ZmidiCiConfig c = config();
  size_t n = 0, rn = 0;
  uint8_t id = 0;
  static const char info[] = "{\"manufacturerId\":[0,33,127],\"familyId\":[1,0]}";
  static const char vol[] = "{\"value\":100}";

  CHECK(zmidi_ci_size() <= sizeof(ZmidiCi));
  /* DEC-019 rule 6: storage of exactly zmidi_ci_size() bytes is enough (ASan finds a write past
   * the end of the heap block). */
  {
    ZmidiCi *heap = (ZmidiCi *)malloc(zmidi_ci_size());
    CHECK(heap != NULL && zmidi_ci_init(heap, &c, 3) == ZMIDI_STATUS_OK && zmidi_ci_muid(heap) != 0);
    free(heap);
  }
  CHECK(zmidi_ci_init(&a, &c, 1) == ZMIDI_STATUS_OK);
  CHECK(zmidi_ci_init(&b, &c, 2) == ZMIDI_STATUS_OK);
  CHECK(zmidi_ci_muid(&a) != zmidi_ci_muid(&b));

  /* A PE request before PE Capabilities, to a peer A does not know: rejected (M2-101 8.4). */
  CHECK(zmidi_ci_pe_get(&a, zmidi_ci_muid(&b), (const uint8_t *)"{}", 2, msgs, sizeof msgs, &n, &id) == ZMIDI_STATUS_REJECTED);

  /* Discovery: B replies by itself; A gets Discovered. */
  CHECK(zmidi_ci_discovery(&a, 0, 0, msgs, sizeof msgs, &n) == ZMIDI_STATUS_OK);
  CHECK(deliver(&b, msgs, n, replies, &rn) == 1 && rn > 3);
  deliver(&a, replies, rn, msgs, &n);
  CHECK(ev.kind == ZMIDI_CI_EVENT_KIND_DISCOVERED && ev.muid == zmidi_ci_muid(&b));

  /* Inquiry: Endpoint Information. */
  CHECK(zmidi_ci_endpoint_info_inquiry(&a, zmidi_ci_muid(&b), msgs, sizeof msgs, &n) == ZMIDI_STATUS_OK);
  deliver(&b, msgs, n, replies, &rn);
  CHECK(rn > 3);
  deliver(&a, replies, rn, msgs, &n);
  CHECK(ev.kind == ZMIDI_CI_EVENT_KIND_ENDPOINT_INFO);

  /* PE Capabilities, then a Get of DeviceInfo answered by the program. */
  CHECK(zmidi_ci_pe_capabilities_inquiry(&a, zmidi_ci_muid(&b), msgs, sizeof msgs, &n) == ZMIDI_STATUS_OK);
  deliver(&b, msgs, n, replies, &rn);
  deliver(&a, replies, rn, msgs, &n);
  CHECK(ev.kind == ZMIDI_CI_EVENT_KIND_PE_CAPABILITIES);
  CHECK(zmidi_ci_pe_get(&a, zmidi_ci_muid(&b), (const uint8_t *)"{\"resource\":\"DeviceInfo\"}", 25, msgs, sizeof msgs, &n, &id) == ZMIDI_STATUS_OK);
  deliver(&b, msgs, n, replies, &rn);
  CHECK(ev.kind == ZMIDI_CI_EVENT_KIND_PE_GET && ev.request_id == id && ev.muid == zmidi_ci_muid(&a));
  CHECK(strcmp((const char *)ev.resource, "DeviceInfo") == 0 && ev.res_id[0] == 0);
  answer(200, info);
  CHECK(ev.kind == ZMIDI_CI_EVENT_KIND_PE_GET_REPLY && ev.request_id == id);
  CHECK(ev.header_len == 14 && memcmp(ev.header, "{\"status\":200}", 14) == 0);
  CHECK(ev.data_len == sizeof info - 1 && memcmp(ev.data, info, ev.data_len) == 0);
  /* No pending request now. */
  CHECK(zmidi_ci_pe_reply(&b, 200, NULL, 0, scratch, sizeof scratch, msgs, sizeof msgs, &n) == ZMIDI_STATUS_NO_PENDING);

  /* A Get of ResourceList: the library answers with the list of the configuration. */
  CHECK(zmidi_ci_pe_get(&a, zmidi_ci_muid(&b), (const uint8_t *)"{\"resource\":\"ResourceList\"}", 27, msgs, sizeof msgs, &n, &id) == ZMIDI_STATUS_OK);
  deliver(&b, msgs, n, replies, &rn);
  CHECK(ev.kind == ZMIDI_CI_EVENT_KIND_PE_GET);
  answer(404, NULL);
  CHECK(ev.data_len == sizeof list - 1 && memcmp(ev.data, list, ev.data_len) == 0);

  /* A Get of an unknown resource: 404 from the program. */
  CHECK(zmidi_ci_pe_get(&a, zmidi_ci_muid(&b), (const uint8_t *)"{\"resource\":\"X-None\"}", 21, msgs, sizeof msgs, &n, &id) == ZMIDI_STATUS_OK);
  deliver(&b, msgs, n, replies, &rn);
  answer(404, NULL);
  CHECK(ev.kind == ZMIDI_CI_EVENT_KIND_PE_GET_REPLY && ev.header_len == 14 && memcmp(ev.header, "{\"status\":404}", 14) == 0);

  /* A Set of X-Volume: the program reads the Property Data of the event and answers 200. */
  CHECK(zmidi_ci_pe_set(&a, zmidi_ci_muid(&b), (const uint8_t *)"{\"resource\":\"X-Volume\"}", 23, (const uint8_t *)vol, sizeof vol - 1, msgs, sizeof msgs, &n, &id) == ZMIDI_STATUS_OK);
  deliver(&b, msgs, n, replies, &rn);
  CHECK(ev.kind == ZMIDI_CI_EVENT_KIND_PE_SET && strcmp((const char *)ev.resource, "X-Volume") == 0);
  CHECK(ev.data_len == sizeof vol - 1 && memcmp(ev.data, vol, ev.data_len) == 0);
  answer(200, NULL);
  CHECK(ev.kind == ZMIDI_CI_EVENT_KIND_PE_SET_REPLY && memcmp(ev.header, "{\"status\":200}", 14) == 0);

  /* REQ-FFI-CI-005: both buffers of the one-chunk reply need 2 x max(data, ResourceList) + 64 bytes;
   * a smaller one changes nothing. The reply of 300 bytes is one Chunk of more than 255 bytes: its
   * record header has the high byte of the length. */
  {
    static uint8_t big[300];
    const size_t need = 2 * sizeof big + 64;
    memset(big, 'x', sizeof big);
    big[0] = '"';
    big[sizeof big - 1] = '"';
    /* The fifth request of A: the replies above freed the 4 Request IDs (REQ-FFI-CI-021). */
    CHECK(zmidi_ci_pe_get(&a, zmidi_ci_muid(&b), (const uint8_t *)"{\"resource\":\"DeviceInfo\"}", 25, msgs, sizeof msgs, &n, &id) == ZMIDI_STATUS_OK);
    deliver(&b, msgs, n, replies, &rn);
    CHECK(zmidi_ci_pe_reply(&b, 200, big, sizeof big, scratch, need - 1, msgs, sizeof msgs, &n) == ZMIDI_STATUS_BUFFER_TOO_SMALL);
    CHECK(zmidi_ci_pe_reply(&b, 200, big, sizeof big, scratch, need, msgs, need - 1, &n) == ZMIDI_STATUS_BUFFER_TOO_SMALL);
    CHECK(zmidi_ci_pe_reply(&b, 200, big, sizeof big, scratch, need, msgs, need, &n) == ZMIDI_STATUS_OK);
    CHECK(n > 3 + 255 && msgs[2] == 1 && n == 3 + ((size_t)msgs[1] | ((size_t)msgs[2] << 8)));
    deliver(&a, msgs, n, replies, &rn);
    CHECK(ev.kind == ZMIDI_CI_EVENT_KIND_PE_GET_REPLY && ev.data_len == sizeof big && memcmp(ev.data, big, sizeof big) == 0);
  }

  /* REQ-FFI-CI-021: the last Chunk of a reply frees the Request ID of A, at the next
   * zmidi_ci_receive (M2-101 Table 29). The device has 4 Request IDs, and 6 requests go. */
  {
    const uint32_t peer = zmidi_ci_muid(&b);
    uint8_t first = 0, second = 0, ids[4];
    int i;
    for (i = 0; i < 6; i++) {
      CHECK(zmidi_ci_pe_get(&a, peer, (const uint8_t *)"{\"resource\":\"DeviceInfo\"}", 25, msgs, sizeof msgs, &n, &id) == ZMIDI_STATUS_OK);
      deliver(&b, msgs, n, replies, &rn);
      answer(200, info);
      CHECK(ev.kind == ZMIDI_CI_EVENT_KIND_PE_GET_REPLY && ev.request_id == id);
    }
    /* The event of the reply stays valid: until the next receive, a new request gets another ID. */
    CHECK(zmidi_ci_pe_get(&a, peer, (const uint8_t *)"{\"resource\":\"DeviceInfo\"}", 25, msgs, sizeof msgs, &n, &first) == ZMIDI_STATUS_OK);
    deliver(&b, msgs, n, replies, &rn);
    answer(200, info);
    CHECK(ev.kind == ZMIDI_CI_EVENT_KIND_PE_GET_REPLY && ev.request_id == first);
    CHECK(zmidi_ci_pe_get(&a, peer, (const uint8_t *)"{\"resource\":\"DeviceInfo\"}", 25, msgs, sizeof msgs, &n, &second) == ZMIDI_STATUS_OK);
    CHECK(second != first);
    deliver(&b, msgs, n, replies, &rn);
    answer(200, info);
    CHECK(ev.kind == ZMIDI_CI_EVENT_KIND_PE_GET_REPLY && ev.request_id == second);

    /* REQ-FFI-CI-022: a request with no reply keeps its ID until zmidi_ci_pe_end. The ID of the
     * last reply goes too: a later receive does not free it again. */
    CHECK(zmidi_ci_pe_end(&a, peer, second) == ZMIDI_STATUS_OK);
    for (i = 0; i < 4; i++)
      CHECK(zmidi_ci_pe_get(&a, peer, (const uint8_t *)"{}", 2, msgs, sizeof msgs, &n, &ids[i]) == ZMIDI_STATUS_OK);
    CHECK(ids[0] == second);
    CHECK(zmidi_ci_endpoint_info_inquiry(&a, peer, msgs, sizeof msgs, &n) == ZMIDI_STATUS_OK);
    deliver(&b, msgs, n, replies, &rn);
    deliver(&a, replies, rn, msgs, &n);
    CHECK(ev.kind == ZMIDI_CI_EVENT_KIND_ENDPOINT_INFO);
    CHECK(zmidi_ci_pe_get(&a, peer, (const uint8_t *)"{}", 2, msgs, sizeof msgs, &n, &id) == ZMIDI_STATUS_REJECTED);
    CHECK(zmidi_ci_pe_end(&a, peer, ids[2]) == ZMIDI_STATUS_OK);
    CHECK(zmidi_ci_pe_get(&a, peer, (const uint8_t *)"{}", 2, msgs, sizeof msgs, &n, &id) == ZMIDI_STATUS_OK && id == ids[2]);
    CHECK(zmidi_ci_pe_end(&a, peer, 0x80) == ZMIDI_STATUS_INVALID_ARGUMENT);
    CHECK(zmidi_ci_pe_end(&a, 0x10000000u, 0) == ZMIDI_STATUS_INVALID_ARGUMENT);
    CHECK(zmidi_ci_pe_end(NULL, peer, 0) == ZMIDI_STATUS_INVALID_ARGUMENT);
    for (i = 0; i < 4; i++)
      CHECK(zmidi_ci_pe_end(&a, peer, ids[i]) == ZMIDI_STATUS_OK);

    /* A reply of 2 Chunks (M2-101 Table 29): only the last Chunk frees the ID. */
    CHECK(zmidi_ci_pe_get(&a, peer, (const uint8_t *)"{}", 2, msgs, sizeof msgs, &n, &first) == ZMIDI_STATUS_OK);
    n = get_reply_chunk(msgs, peer, zmidi_ci_muid(&a), first, "{\"status\":200}", 2, 1, "{\"a\":");
    CHECK(zmidi_ci_receive(&a, 0, msgs, n, 1000, replies, sizeof replies, &rn, &ev) == ZMIDI_STATUS_OK);
    CHECK(ev.kind == ZMIDI_CI_EVENT_KIND_PE_GET_REPLY && ev.chunk_number == 1 && ev.chunk_total == 2);
    n = get_reply_chunk(msgs, peer, zmidi_ci_muid(&a), first, NULL, 2, 2, "1}");
    CHECK(zmidi_ci_receive(&a, 0, msgs, n, 1000, replies, sizeof replies, &rn, &ev) == ZMIDI_STATUS_OK);
    CHECK(ev.kind == ZMIDI_CI_EVENT_KIND_PE_GET_REPLY && ev.chunk_number == 2 && ev.request_id == first);
    CHECK(zmidi_ci_pe_get(&a, peer, (const uint8_t *)"{}", 2, msgs, sizeof msgs, &n, &second) == ZMIDI_STATUS_OK);
    CHECK(second != first);
    CHECK(zmidi_ci_pe_end(&a, peer, second) == ZMIDI_STATUS_OK);
  }

  /* Small buffers and null pointers. */
  CHECK(zmidi_ci_receive(&b, 0, msgs, n, 0, replies, ZMIDI_CI_REPLY_BUFFER - 1, &rn, &ev) == ZMIDI_STATUS_BUFFER_TOO_SMALL);
  CHECK(zmidi_ci_init(NULL, &c, 1) == ZMIDI_STATUS_INVALID_ARGUMENT);
  c.max_sysex_size = 100; /* below 512 with PE (M2-101 5.2) */
  CHECK(zmidi_ci_init(&a, &c, 1) == ZMIDI_STATUS_INVALID_ARGUMENT);
  CHECK(zmidi_ci_poll(&b, 5000000, &ev) == ZMIDI_STATUS_OK);

  profiles_and_process();

  if (failures) { fprintf(stderr, "%d failure(s)\n", failures); return EXIT_FAILURE; }
  printf("ci ok: state %u B, slot %u B\n", (unsigned)zmidi_ci_size(), (unsigned)sizeof(ZmidiCi));
  return EXIT_SUCCESS;
}
