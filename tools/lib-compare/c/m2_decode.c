#include "common.h"
#include "midi2_dispatch.h"
static void non(uint8_t g,uint8_t c,uint8_t n,uint16_t v,uint8_t at,uint16_t ad,void*x){SINK(g^c^n^v^at^ad);}
static void nof(uint8_t g,uint8_t c,uint8_t n,uint16_t v,uint8_t at,uint16_t ad,void*x){SINK(g^c^n);}
static void cc(uint8_t g,uint8_t c,uint8_t i,uint32_t v,void*x){SINK(g^c^i^v);}
static void pc(uint8_t g,uint8_t c,uint8_t p,bool b,uint8_t m,uint8_t l,void*x){SINK(g^c^p^b);}
static void pb(uint8_t g,uint8_t c,uint32_t v,void*x){SINK(v^c);}
static void pnpb(uint8_t g,uint8_t c,uint8_t n,uint32_t v,void*x){SINK(v^n);}
static void nrpn(uint8_t g,uint8_t c,uint8_t b,uint8_t i,uint32_t v,void*x){SINK(g^c^b^i^v);}
static void rel(uint8_t g,uint8_t c,uint8_t b,uint8_t i,uint32_t v,void*x){SINK(g^c^b^i^v^1);}
static void sys(uint8_t g,uint8_t s,uint8_t d1,uint8_t d2,void*x){SINK(s^d1^d2);}
static midi2_dispatch dp; static bool init;
void feature(void){
  if(!init){ midi2_dispatch_init(&dp); dp.on_note_on=non; dp.on_note_off=nof; dp.on_cc=cc; dp.on_program=pc;
    dp.on_pitch_bend=pb; dp.on_per_note_pb=pnpb; dp.on_nrpn=nrpn; dp.on_rel_nrpn=rel; dp.on_system=sys; init=true; }
  uint32_t w[4]={INPUT[0],INPUT[1],INPUT[2],INPUT[3]};
  midi2_dispatch_feed(w,(uint8_t)(INPUT[4]&3)+1,&dp);
}
