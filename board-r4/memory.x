/* Renesas R7FA4M1AB3CFM -- Arduino UNO R4 WiFi
 *
 * Flash starts at 0x4000 rather than 0, because the first 16 KB hold the
 * Arduino/Renesas DFU bootloader. Keeping it is what lets you re-flash over
 * USB-C with a double-tap on RESET instead of needing a SWD probe.
 *
 * The bootloader has already configured the 48 MHz clock and pointed VTOR at
 * 0x4000 by the time our reset handler runs, so there is no clock bring-up.
 */
MEMORY
{
  FLASH : ORIGIN = 0x00004000, LENGTH = 240K
  RAM   : ORIGIN = 0x20000000, LENGTH = 32K
}
