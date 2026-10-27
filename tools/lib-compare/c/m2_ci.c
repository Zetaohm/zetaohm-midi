#include "common.h"
#include "midi2_proc.h"
#include "midi2_ci.h"
static midi2_proc_state proc; static midi2_ci_state ci; static bool init;
static uint8_t sx[512]; static uint8_t profiles[1][5]; static midi2_ci_property props[4];
static uint32_t wr(const uint32_t*w,uint32_t n,void*c){ for(uint32_t i=0;i<n;i++) SINK(w[i]); return n; }
static const char* get(const char*n,void*c){ return n; }
static bool set(const char*n,const char*v,void*c){ SINK(v[0]); return true; }
static void onsx(uint8_t g,const uint8_t*d,uint16_t l,void*c){ midi2_ci_process_sysex(&ci,g,d,l); }
void feature(void){
  if(!init){ midi2_proc_init(&proc,sx,sizeof sx,0,0); proc.on_sysex7=onsx;
    midi2_ci_init(&ci,INPUT[5],profiles,1,props,4); midi2_ci_set_identity(&ci,0x7D,1,1,1);
    midi2_ci_set_write_fn(&ci,wr,0);
    midi2_ci_add_property_dynamic(&ci,"X-Pattern",get,set); midi2_ci_add_property_dynamic(&ci,"X-Global",get,set); init=true; }
  uint32_t w[2]={INPUT[0],INPUT[1]};
  midi2_proc_feed(&proc,w,2);
}
