#include "common.h"
#include "umpToBytestream.h"
static umpToBytestream t;
extern "C" void feature(void){ for (int i = 0; i < 4; i++) t.UMPStreamParse(INPUT[i]); while (t.availableBS()) SINK(t.readBS()); }
