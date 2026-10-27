#include "common.h"
volatile uint32_t INPUT[64];
void feature(void);
int main(void) { for (;;) { feature(); } }
void *__dso_handle = 0;
