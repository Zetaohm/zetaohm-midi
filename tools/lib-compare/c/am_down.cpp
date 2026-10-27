#include "common.h"
#include "umpToMIDI1Protocol.h"
static umpToMIDI1Protocol t;
extern "C" void feature(void){ t.UMPStreamParse(INPUT[0]); while (t.availableUMP()) SINK(t.readUMP()); }
