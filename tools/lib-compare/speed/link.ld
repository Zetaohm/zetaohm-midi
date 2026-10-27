/* QEMU mps2-an386: code at 0x0, RAM at 0x20000000. QEMU loads .data at its address and the RAM
   starts as zero, so the reset code does no copy. */
MEMORY { FLASH (rx) : ORIGIN = 0x00000000, LENGTH = 4M
         RAM (rwx)  : ORIGIN = 0x20000000, LENGTH = 4M }
ENTRY(reset)
SECTIONS {
  .text : { KEEP(*(.vectors)) *(.text*) *(.rodata*)
            . = ALIGN(4); __init_array_start = .; KEEP(*(SORT(.init_array.*))) KEEP(*(.init_array)) __init_array_end = .; } > FLASH
  .ARM.exidx : { *(.ARM.exidx*) } > FLASH
  .data : { *(.data*) } > RAM
  .bss : { *(.bss*) *(COMMON) } > RAM
  . = ALIGN(8); end = .; _end = .;
  _estack = ORIGIN(RAM) + LENGTH(RAM);
}
