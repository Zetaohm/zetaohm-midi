/* STM32WB55RG (RM0434 memory map): 1 MB flash at 0x08000000, SRAM1 192 KB at 0x20000000.
 * The upper flash holds the CPU2 firmware (FUS and the wireless stack) above the secure start
 * address (SFSA option byte), so the application uses the lower 512 KB only. SRAM2a and SRAM2b
 * (shared with CPU2) are not used. */
MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 512K
  RAM   : ORIGIN = 0x20000000, LENGTH = 192K
}
