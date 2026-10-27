#include "common.h"
#include "umpProcessor.h"
#include "midiCIProcessor.h"
#include "midiCIMessageCreate.h"
static umpProcessor up; static midiCIProcessor ci; static bool init;
static uint8_t tx[512]; static uint32_t localMuid;
static void out(uint16_t n){ for(uint16_t i=0;i<n;i++) SINK(tx[i]); }
extern "C" void feature(void){
  if(!init){
    localMuid = INPUT[5] & 0x0FFFFFFF;
    up.setSysEx([](umpData m){
      if(m.form==0||m.form==1) ci.startSysex7(m.umpGroup, m.data[1]);
      for(int i=0;i<m.dataLength;i++) ci.processMIDICI(m.data[i]);
      if(m.form==0||m.form==3) ci.endSysex7(); });
    ci.setCheckMUID([](uint8_t g, uint32_t muid, void*){ return muid==localMuid; });
    ci.setRecvDiscovery([](MIDICI d, std::array<uint8_t,3> a, std::array<uint8_t,2> b, std::array<uint8_t,2> c, std::array<uint8_t,4> v, uint8_t s, uint16_t mx, uint8_t o){
      out(CIMessage::sendDiscoveryReply(tx, 2, localMuid, d.remoteMUID, {0x7D,0,0}, {1,0}, {1,0}, {0,0,0,1}, 0x0C, 512, 0, 0x7F)); });
    ci.setRecvProfileInquiry([](MIDICI d){ uint8_t p[5]={0x7E,0,0,1,1};
      out(CIMessage::sendProfileListResponse(tx, 2, localMuid, d.remoteMUID, 0x7F, 1, p, 0, nullptr)); });
    ci.setPECapabilities([](MIDICI d, uint8_t n, uint8_t a, uint8_t b){
      out(CIMessage::sendPECapabilityReply(tx, 2, localMuid, d.remoteMUID, 2, 0, 0)); });
    ci.setRecvPEGetInquiry([](MIDICI d, std::string req){
      static uint8_t hdr[] = "{\"status\":200}"; static uint8_t body[64];
      uint16_t n = 0; for(char ch : req) { if(n<64) body[n++] = (uint8_t)ch; }
      out(CIMessage::sendPEGetReply(tx, 2, localMuid, d.remoteMUID, d.requestId, sizeof hdr - 1, hdr, 1, 1, n, body)); });
    init=true; }
  up.processUMP(INPUT[0]);
}
