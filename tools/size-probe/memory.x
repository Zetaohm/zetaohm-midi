/* MK20DX256 (FLXS1): 256 KiB flash, 64 KiB RAM. The probe needs no device crate. */
MEMORY
{
  FLASH : ORIGIN = 0x00000000, LENGTH = 256K
  RAM : ORIGIN = 0x1FFF8000, LENGTH = 64K
}
