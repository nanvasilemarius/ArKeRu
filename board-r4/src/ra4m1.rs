//! Direct register access for the Renesas RA4M1. The only `unsafe` in the
//! project -- the `kernel` crate is `#![forbid(unsafe_code)]`.
//!
//! Addresses are from the RA4M1 Group User's Manual (R01UH0887EJ). Every
//! constant is named and commented, because a wrong value here is a silent
//! hang rather than a compile error.

#![allow(dead_code)]

use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{AtomicU16, AtomicUsize, Ordering};

// --- Register write protection ----------------------------------------------
//
// PRCR guards the clock-generation and low-power register blocks. Writes to
// MSTPCRB are silently DISCARDED while PRC1 is clear -- no fault, no hint, the
// SCI clock simply stays gated and SSR.TDRE never sets. Every MSTP change must
// be bracketed by unlock/lock.
const PRCR: *mut u16 = 0x4001_E3FE as *mut u16;
const PRCR_KEY: u16 = 0xA500; // PRKEY, required in the upper byte of any write
const PRCR_PRC1: u16 = 1 << 1; // low-power / module-stop registers

fn prcr_unlock() {
    unsafe { write_volatile(PRCR, PRCR_KEY | PRCR_PRC1) }
}

fn prcr_lock() {
    unsafe { write_volatile(PRCR, PRCR_KEY) }
}

// --- Module Stop Control Register B: clock-gates the SCI channels -----------
const MSTPCRB: *mut u32 = 0x4004_7000 as *mut u32;

/// SCI0 is MSTPB31 and the channels count downwards, so SCI9 is MSTPB22.
const fn mstpb_sci(ch: usize) -> u32 {
    1 << (31 - ch as u32)
}

// --- Reset status -----------------------------------------------------------
//
// RA4M1 User's Manual R01UH0887EJ0100, section 5.2. Neither register appears
// in the PRCR protection table (section 12.1), so clearing the flags needs no
// unlock -- unlike the module-stop registers next door, which do.

/// Reset Status Register 0: power-on and voltage-monitor flags.
const RSTSR0: *mut u8 = 0x4001_E410 as *mut u8;
/// Reset Status Register 1: watchdog, software, memory and bus flags.
const RSTSR1: *mut u16 = 0x4001_E0C0 as *mut u16;

/// Read both reset status registers and clear every flag.
///
/// The flags are sticky across resets, so this has to happen once at startup:
/// leave them set and the next boot reports the union of everything that has
/// ever happened. The manual specifies clearing by writing 0 after reading 1,
/// which is what the write-back does.
pub fn take_reset_flags() -> (u8, u16) {
    unsafe {
        let r0 = read_volatile(RSTSR0);
        let r1 = read_volatile(RSTSR1);
        write_volatile(RSTSR0, 0);
        write_volatile(RSTSR1, 0);
        (r0, r1)
    }
}

// --- Pin function select ----------------------------------------------------
/// Write-protect register guarding the PFS block.
const PWPR: *mut u8 = 0x4004_0D03 as *mut u8;
const PWPR_B0WI: u8 = 1 << 7;
const PWPR_PFSWE: u8 = 1 << 6;

const PFS_BASE: usize = 0x4004_0800;
const PORT_BASE: usize = 0x4004_0000;

const PFS_PMR: u32 = 1 << 16; // 0 = GPIO, 1 = peripheral
const PFS_ASEL: u32 = 1 << 15; // 1 = pin switched to analog
const PFS_PCR: u32 = 1 << 4; // 1 = internal pull-up enabled

/// PSEL for even-numbered SCI channels (SCI0/2/4/6/8).
pub const PSEL_SCI_EVEN: u32 = 0b00100 << 24;
/// PSEL for odd-numbered SCI channels (SCI1/3/5/7/9).
pub const PSEL_SCI_ODD: u32 = 0b00101 << 24;

/// PmnPFS is one 32-bit register per pin: base + port*0x40 + pin*4.
fn pfs(port: usize, pin: usize) -> *mut u32 {
    (PFS_BASE + port * 0x40 + pin * 4) as *mut u32
}

/// PCNTR1: PDR (direction) in bits 15:0, PODR (output data) in bits 31:16.
fn pcntr1(port: usize) -> *mut u32 {
    (PORT_BASE + port * 0x20) as *mut u32
}

/// PCNTR3: POSR (set high) in bits 15:0, PORR (set low) in bits 31:16.
/// Write-only bit setters, so no read-modify-write race with an ISR.
fn pcntr3(port: usize) -> *mut u32 {
    (PORT_BASE + port * 0x20 + 0x08) as *mut u32
}

/// Unlock the PFS block. Clearing B0WI must be a separate write from setting
/// PFSWE; the hardware rejects a single combined write.
fn pfs_unlock() {
    unsafe {
        write_volatile(PWPR, 0);
        write_volatile(PWPR, PWPR_PFSWE);
    }
}

fn pfs_lock() {
    unsafe { write_volatile(PWPR, PWPR_B0WI) }
}

/// Route a pin to a peripheral. PSEL and PMR are written separately: setting
/// both at once glitches the pad.
pub fn pin_to_peripheral(port: usize, pin: usize, psel: u32) {
    pfs_unlock();
    unsafe {
        let r = pfs(port, pin);
        write_volatile(r, psel);
        write_volatile(r, psel | PFS_PMR);
    }
    pfs_lock();
}

/// Make a pin a push-pull output. PCNTR1 is not PWPR-protected, so this needs
/// no unlock dance.
pub fn gpio_output(port: usize, pin: usize) {
    unsafe {
        let r = pcntr1(port);
        write_volatile(r, read_volatile(r) | (1 << pin));
    }
}

/// Make a pin a high-impedance input.
pub fn gpio_input(port: usize, pin: usize) {
    unsafe {
        let r = pcntr1(port);
        write_volatile(r, read_volatile(r) & !(1 << pin));
    }
}

/// Read a pin. PCNTR2 holds PIDR in bits 15:0.
pub fn gpio_read(port: usize, pin: usize) -> bool {
    unsafe { read_volatile((PORT_BASE + port * 0x20 + 0x04) as *mut u32) & (1 << pin) != 0 }
}

/// Turn the internal pull-up on or off for one pin.
///
/// PCR is bit 4 of PmnPFS, and writing PmnPFS needs the PWPR unlock; reads do
/// not. Read-modify-write so the pin's other settings survive.
///
/// The pull-up is ~33 kohm, so a pin held low by whatever it is connected to
/// draws about 100 uA against it -- nothing. What it buys is a *defined* level
/// on an unconnected input, which otherwise floats and reads arbitrarily.
pub fn set_pullup(port: usize, pin: usize, on: bool) {
    pfs_unlock();
    unsafe {
        let r = pfs(port, pin);
        let v = read_volatile(r);
        write_volatile(r, if on { v | PFS_PCR } else { v & !PFS_PCR });
    }
    pfs_lock();
}

/// Whether a pin belongs to the charlieplexed matrix.
///
/// Derived from the same masks the scan uses to park the panel, so this cannot
/// disagree with what is actually being driven.
pub fn is_matrix_pin(port: usize, pin: usize) -> bool {
    let bit = 1u32 << pin;
    (port == 0 && MATRIX_PORT0_MASK & bit != 0) || (port == 2 && MATRIX_PORT2_MASK & bit != 0)
}

/// Snapshot one port for the `PINS` diagnostic: level, direction, and whether
/// each pin has been handed to a peripheral or to the ADC.
///
/// Reads only. PCNTR1 and PCNTR2 are one register each; PMR and ASEL live one
/// per pin in PmnPFS, so those cost sixteen reads. PmnPFS is readable without
/// touching PWPR -- the write protection guards writes, not reads.
///
/// Ports beyond the package are still mapped and read back as zero, which is
/// indistinguishable from "all pins low inputs". The caller decides how many
/// ports are worth showing.
pub fn port_snapshot(port: usize) -> (u16, u16, u16, u16) {
    unsafe {
        let level = (read_volatile((PORT_BASE + port * 0x20 + 0x04) as *mut u32) & 0xFFFF) as u16;
        let output = (read_volatile(pcntr1(port)) & 0xFFFF) as u16;
        let mut peripheral = 0u16;
        let mut analog = 0u16;
        for pin in 0..16 {
            let v = read_volatile(pfs(port, pin));
            if v & PFS_PMR != 0 {
                peripheral |= 1 << pin;
            }
            if v & PFS_ASEL != 0 {
                analog |= 1 << pin;
            }
        }
        (level, output, peripheral, analog)
    }
}

pub fn gpio_write(port: usize, pin: usize, high: bool) {
    unsafe {
        // POSR is the low half, PORR the high half.
        write_volatile(pcntr3(port), if high { 1 << pin } else { 1 << (pin + 16) });
    }
}

// --- SCI --------------------------------------------------------------------

const SCI_BASE: usize = 0x4007_0000;
const SCI_STRIDE: usize = 0x20;

const SMR: usize = 0x00;
const BRR: usize = 0x01;
const SCR: usize = 0x02;
const TDR: usize = 0x03;
const SSR: usize = 0x04;
const RDR: usize = 0x05;
const SCMR: usize = 0x06;
const SEMR: usize = 0x07;

const SCR_TE: u8 = 1 << 5;
const SCR_RE: u8 = 1 << 4;
/// Receive interrupt enable: raise `RXI` when `RDRF` sets.
const SCR_RIE: u8 = 1 << 6;

// --------------------------------------------------------------- ICU / NVIC
//
// The RA4M1 does not wire peripherals to NVIC lines directly. Every peripheral
// signal is an *event number*, and the ICU has 32 slots -- `IELSR0` to
// `IELSR31` -- each of which links one event to one NVIC interrupt of the same
// index. Nothing reaches the CPU until a slot is written.
//
// That indirection is why this firmware ran for twenty versions with no
// peripheral interrupt at all and nothing complaining: an unlinked event is not
// an error, it is silence.

/// `IELSR0`. Thirty-two registers, four bytes apart, section 13.2.6.
const ICU_IELSR: usize = 0x4000_6300;
/// How many ICU slots this part has. Also the length of the interrupt vector
/// table the linker expects.
pub const ICU_SLOTS: usize = 32;

/// `IELSR.IR`, bit 16: an interrupt request occurred. Write 0 to clear.
const IELSR_IR: u32 = 1 << 16;

/// Event numbers. **Measured, not read off a table.**
///
/// The User's Manual gives Table 13.4 for the ICU and Table 18.3 for the ELC.
/// They cover the same peripherals with the same names and disagree by one from
/// SCI2 onwards, and Table 13.4 straddles a page break exactly at SCI9 -- so
/// whether `SCI9_RXI` is `0xA7` or `0xA8` comes down to which side of a page
/// boundary an orphaned table row belongs to. Both readings are defensible from
/// the text and one of them silently kills the console.
///
/// The board settled it. The Arduino bootloader leaves its own ICU links in
/// place when it hands over, and dumping the slots after a keypress showed:
///
/// ```text
///   IELSR4 000A9   IELSR5 000AA   IELSR6 100A8   IELSR7 000AB
///                                        ^^^^^ IR set
/// ```
///
/// Slot 6 is linked to `0xA8` and its interrupt-status flag is set after a byte
/// arrives. That is `SCI9_RXI`, stated by the hardware.
pub const EVT_SCI9_RXI: u8 = 0xA8;

/// Wipe every ICU link.
///
/// The bootloader hands over with slots 1 to 7 still linked to its own events,
/// pointing at a vector table that is no longer installed. They are harmless
/// only because none of them is enabled in the NVIC -- one `nvic_enable` on the
/// wrong index and a leftover event vectors into whatever this image happens to
/// have at that offset.
///
/// So the ICU is cleared before anything is linked deliberately. Writing zero
/// both disables the slot and clears its `IR` flag.
///
/// # Safety
/// Must happen before any interrupt this firmware relies on is linked, and
/// after the bootloader has finished with them -- which is to say, early in
/// `main`.
pub unsafe fn icu_reset() {
    for slot in 0..ICU_SLOTS {
        write_volatile((ICU_IELSR + slot * 4) as *mut u32, 0);
    }
}

/// Link `event` to ICU slot `slot`, which is NVIC interrupt number `slot`.
///
/// # Safety
/// The caller must have installed a handler at that vector first. An event
/// linked to a slot whose vector is garbage is a hard fault on the next byte.
pub unsafe fn icu_link(slot: usize, event: u8) {
    if slot < ICU_SLOTS {
        write_volatile((ICU_IELSR + slot * 4) as *mut u32, event as u32);
    }
}

/// Clear the request flag for a slot. Must happen inside the handler, or the
/// interrupt re-enters immediately and the machine makes no further progress.
pub fn icu_clear(slot: usize) {
    if slot < ICU_SLOTS {
        let r = (ICU_IELSR + slot * 4) as *mut u32;
        unsafe {
            let v = read_volatile(r);
            write_volatile(r, v & !IELSR_IR);
            // Read back, so the write has retired before the handler returns.
            // Otherwise the flag can still be set when the NVIC re-evaluates
            // and the same interrupt fires a second time for one event.
            let _ = read_volatile(r);
        }
    }
}

/// Read a slot's link register back, to confirm the write took.
pub fn ielsr_read(slot: usize) -> u32 {
    if slot < ICU_SLOTS {
        unsafe { read_volatile((ICU_IELSR + slot * 4) as *const u32) }
    } else {
        0
    }
}

/// NVIC registers, from the ARMv7-M architecture rather than the RA4M1 manual.
const NVIC_ISER: *mut u32 = 0xE000_E100 as *mut u32;
const NVIC_ICER: *mut u32 = 0xE000_E180 as *mut u32;
const NVIC_IPR: *mut u8 = 0xE000_E400 as *mut u8;

/// Enable an interrupt and give it a priority. Lower numbers win.
///
/// # Safety
/// As [`icu_link`]: the vector must already be valid.
pub unsafe fn nvic_enable(irq: usize, priority: u8) {
    if irq >= ICU_SLOTS {
        return;
    }
    write_volatile(NVIC_IPR.add(irq), priority);
    write_volatile(NVIC_ISER.add(irq / 32), 1 << (irq % 32));
}

/// Which interrupts are enabled, and which are pending. A pending-but-not-taken
/// interrupt means the event arrived and something is masking it; nothing
/// pending means the event never reached the NVIC at all.
pub fn nvic_iser_read() -> u32 {
    unsafe { read_volatile(NVIC_ISER as *const u32) }
}

pub fn nvic_ispr_read() -> u32 {
    unsafe { read_volatile(0xE000_E200 as *const u32) }
}

/// Turn an interrupt off again. Used by nothing yet; present because enabling
/// something with no way to disable it is how a diagnostic becomes impossible.
pub unsafe fn nvic_disable(irq: usize) {
    if irq < ICU_SLOTS {
        write_volatile(NVIC_ICER.add(irq / 32), 1 << (irq % 32));
    }
}

const SSR_TDRE: u8 = 1 << 7; // transmit data register empty
const SSR_RDRF: u8 = 1 << 6; // receive data register full
const SSR_ORER: u8 = 1 << 5; // overrun
const SSR_FER: u8 = 1 << 4; // framing
const SSR_PER: u8 = 1 << 3; // parity
const SSR_ERRORS: u8 = SSR_ORER | SSR_FER | SSR_PER;

/// One SCI channel in asynchronous 8N1 mode.
#[derive(Clone, Copy)]
pub struct Sci {
    ch: usize,
}

impl Sci {
    pub const fn new(ch: usize) -> Self {
        Self { ch }
    }

    fn reg(&self, off: usize) -> *mut u8 {
        (SCI_BASE + self.ch * SCI_STRIDE + off) as *mut u8
    }

    /// Asynchronous bit rate divisor, from the RA4M1 manual:
    ///
    /// ```text
    ///   BRR = PCLK / (32 * baud) - 1      (CKS=0, ABCS=0, BGDM=0)
    /// ```
    ///
    /// At 48 MHz and 115200 this is 12, giving an actual 115385 baud -- 0.16%
    /// error, well inside the ~2% a UART tolerates.
    const fn brr_for(pclk: u32, baud: u32) -> u8 {
        let v = pclk / (32 * baud);
        if v == 0 {
            0
        } else {
            (v - 1) as u8
        }
    }

    /// Bring the channel up on the given pins. `pins` is `(port, pin, psel)`
    /// for TX and RX; both get the same PSEL.
    pub fn init(&self, pclk: u32, baud: u32, pins: &[(usize, usize, u32)]) {
        // Ungate the clock. A 0 bit in MSTP means "running". This write is
        // ignored unless PRCR.PRC1 is set first.
        prcr_unlock();
        unsafe {
            write_volatile(MSTPCRB, read_volatile(MSTPCRB) & !mstpb_sci(self.ch));
        }
        prcr_lock();

        for &(port, pin, psel) in pins {
            pin_to_peripheral(port, pin, psel);
        }

        unsafe {
            write_volatile(self.reg(SCR), 0); // TX/RX off while configuring
            write_volatile(self.reg(SMR), 0); // async, 8-bit, no parity, 1 stop
            write_volatile(self.reg(SCMR), 0xF2); // reset value: LSB first
            write_volatile(self.reg(SEMR), 0); // no double-speed, no filter
            write_volatile(self.reg(BRR), Self::brr_for(pclk, baud));

            // One bit time must elapse before enabling. ~8.7 us at 115200;
            // this is comfortably longer at 48 MHz.
            for _ in 0..2000 {
                cortex_m::asm::nop();
            }

            write_volatile(self.reg(SCR), SCR_TE | SCR_RE);
        }
    }

    /// Single-byte transmit, bounded.
    ///
    /// Returns false if the channel never reported TDRE. An unbounded spin here
    /// means one misconfigured UART wedges the entire system -- which is
    /// exactly the failure that made the first probe look like dead firmware.
    #[must_use]
    pub fn put(&self, b: u8) -> bool {
        // ~100k iterations is milliseconds at 48 MHz, far longer than one
        // character time at any baud we use.
        for _ in 0..100_000 {
            unsafe {
                if read_volatile(self.reg(SSR)) & SSR_TDRE != 0 {
                    write_volatile(self.reg(TDR), b);
                    return true;
                }
            }
        }
        false
    }

    /// Returns false as soon as any byte times out.
    pub fn write(&self, s: &[u8]) -> bool {
        for &b in s {
            if !self.put(b) {
                return false;
            }
        }
        true
    }

    /// Turn on the receive interrupt for this channel.
    ///
    /// Only meaningful once the event is linked and the vector is installed --
    /// see [`icu_link`]. Setting `RIE` on its own does nothing at all, which is
    /// the same quiet failure the ICU has everywhere.
    pub fn enable_rx_interrupt(&self) {
        unsafe {
            let scr = read_volatile(self.reg(SCR));
            write_volatile(self.reg(SCR), scr | SCR_RIE);
        }
    }

    /// Read `RDR` and clear any error latch, for use **inside an interrupt
    /// handler**.
    ///
    /// Separate from [`Sci::get`] because the two must never both be live on
    /// one channel: whichever reads `RDR` consumes the byte, and a byte
    /// consumed by a poll that the handler then does not see is a byte lost.
    pub fn isr_read(&self) -> Option<u8> {
        unsafe {
            let ssr = read_volatile(self.reg(SSR));
            if ssr & SSR_ERRORS != 0 {
                // The overrun that this whole change exists to prevent. Clear
                // it and take the byte anyway -- `RDR` still holds a good one,
                // and dropping it as well would make a bad situation worse.
                write_volatile(self.reg(SSR), ssr & !SSR_ERRORS);
            }
            if ssr & SSR_RDRF != 0 {
                Some(read_volatile(self.reg(RDR)))
            } else {
                None
            }
        }
    }

    /// Read `SCR` back, to confirm `RIE` actually stuck.
    pub fn scr_read(&self) -> u8 {
        unsafe { read_volatile(self.reg(SCR)) }
    }

    /// Non-blocking single-byte receive, by polling.
    pub fn get(&self) -> Option<u8> {
        unsafe {
            let ssr = read_volatile(self.reg(SSR));

            // Clear any error latch, or reception stalls forever.
            if ssr & SSR_ERRORS != 0 {
                write_volatile(self.reg(SSR), ssr & !SSR_ERRORS);
                return None;
            }

            if ssr & SSR_RDRF != 0 {
                Some(read_volatile(self.reg(RDR))) // reading RDR clears RDRF
            } else {
                None
            }
        }
    }
}

// --- Board wiring, from ArduinoCore-renesas variants/UNOWIFIR4 ---------------

/// D1/D0 header pins. P302 = TXD2, P301 = RXD2.
pub const SCI2_D1D0: Sci = Sci::new(2);
pub const PINS_SCI2: [(usize, usize, u32); 2] =
    [(3, 2, PSEL_SCI_EVEN), (3, 1, PSEL_SCI_EVEN)];

/// ESP32-S3 UART0. P109 = TXD9, P110 = RXD9. Odd channel, so PSEL differs.
pub const SCI9_ESP: Sci = Sci::new(9);
pub const PINS_SCI9: [(usize, usize, u32); 2] =
    [(1, 9, PSEL_SCI_ODD), (1, 10, PSEL_SCI_ODD)];

/// ESP32-S3 "WIFI" link. P501 = TXD1, P502 = RXD1.
pub const SCI1_WIFI: Sci = Sci::new(1);
pub const PINS_SCI1: [(usize, usize, u32); 2] =
    [(5, 1, PSEL_SCI_ODD), (5, 2, PSEL_SCI_ODD)];

/// LED_BUILTIN (Arduino D13) is P102.
pub const LED_PORT: usize = 1;
pub const LED_PIN: usize = 2;

// --- 12x8 charlieplexed LED matrix ------------------------------------------
//
// 96 LEDs driven from 11 pins. Charlieplexing works because each LED is wired
// between an ordered *pair* of pins: to light one, drive its source pin high,
// its sink pin low, and leave the other nine high-impedance so no other LED
// sees a forward voltage. Only one LED is lit at a time; persistence of vision
// does the rest.
//
// Tables transcribed from the Arduino core's `Arduino_LED_Matrix.h`
// (`pin_zero_index = 28`, so its indices are Arduino pins 28..38).

/// Matrix pin index (0..10) -> (port, pin).
const MATRIX_PINS: [(usize, usize); 11] = [
    (0, 3),   // 0  = D28 = P003
    (0, 4),   // 1  = D29 = P004
    (0, 11),  // 2  = D30 = P011
    (0, 12),  // 3  = D31 = P012
    (0, 13),  // 4  = D32 = P013
    (0, 15),  // 5  = D33 = P015
    (2, 4),   // 6  = D34 = P204
    (2, 5),   // 7  = D35 = P205
    (2, 6),   // 8  = D36 = P206
    (2, 12),  // 9  = D37 = P212
    (2, 13),  // 10 = D38 = P213
];

/// PDR bit masks for the two ports the matrix occupies. Nothing else in this
/// project uses PORT0 or PORT2, so the scan can clear them wholesale.
const MATRIX_PORT0_MASK: u32 = (1 << 3) | (1 << 4) | (1 << 11) | (1 << 12) | (1 << 13) | (1 << 15);
const MATRIX_PORT2_MASK: u32 = (1 << 4) | (1 << 5) | (1 << 6) | (1 << 12) | (1 << 13);

/// LED index -> (source pin index, sink pin index). LED `i` is at
/// row `i / 12`, column `i % 12`, counting from the top-left.
const LED_PAIRS: [(u8, u8); 96] = [
    (7, 3), (3, 7), (7, 4), (4, 7), (3, 4), (4, 3),
    (7, 8), (8, 7), (3, 8), (8, 3), (4, 8), (8, 4),
    (7, 0), (0, 7), (3, 0), (0, 3), (4, 0), (0, 4),
    (8, 0), (0, 8), (7, 6), (6, 7), (3, 6), (6, 3),
    (4, 6), (6, 4), (8, 6), (6, 8), (0, 6), (6, 0),
    (7, 5), (5, 7), (3, 5), (5, 3), (4, 5), (5, 4),
    (8, 5), (5, 8), (0, 5), (5, 0), (6, 5), (5, 6),
    (7, 1), (1, 7), (3, 1), (1, 3), (4, 1), (1, 4),
    (8, 1), (1, 8), (0, 1), (1, 0), (6, 1), (1, 6),
    (5, 1), (1, 5), (7, 2), (2, 7), (3, 2), (2, 3),
    (4, 2), (2, 4), (8, 2), (2, 8), (0, 2), (2, 0),
    (6, 2), (2, 6), (5, 2), (2, 5), (1, 2), (2, 1),
    (7, 10), (10, 7), (3, 10), (10, 3), (4, 10), (10, 4),
    (8, 10), (10, 8), (0, 10), (10, 0), (6, 10), (10, 6),
    (5, 10), (10, 5), (1, 10), (10, 1), (2, 10), (10, 2),
    (7, 9), (9, 7), (3, 9), (9, 3), (4, 9), (9, 4),
];

/// Current frame: 8 rows of 12 bits, bit 11 = leftmost column.
/// Written from the shell, read from the scan interrupt.
static FRAME: [AtomicU16; 8] = [const { AtomicU16::new(0) }; 8];
static SCAN: AtomicUsize = AtomicUsize::new(0);

/// Park every matrix pin high-impedance. Must happen between LEDs, or the
/// previous pair keeps conducting and the display ghosts.
fn matrix_all_hiz() {
    unsafe {
        let r0 = pcntr1(0);
        write_volatile(r0, read_volatile(r0) & !MATRIX_PORT0_MASK);
        let r2 = pcntr1(2);
        write_volatile(r2, read_volatile(r2) & !MATRIX_PORT2_MASK);
    }
}

/// Drive one matrix pin as a push-pull output at the given level.
fn matrix_drive(idx: u8, high: bool) {
    let (port, pin) = MATRIX_PINS[idx as usize];
    unsafe {
        let r = pcntr1(port);
        let mut v = read_volatile(r);
        v |= 1 << pin; // PDR: output
        if high {
            v |= 1 << (pin + 16); // PODR: high
        } else {
            v &= !(1 << (pin + 16)); // PODR: low
        }
        write_volatile(r, v);
    }
}

pub fn matrix_init() {
    matrix_all_hiz();
}

/// Replace the displayed frame. Cheap; the scan picks it up on the next tick.
pub fn matrix_set(rows: &[u16; 8]) {
    for (i, r) in rows.iter().enumerate() {
        FRAME[i].store(*r & 0x0FFF, Ordering::Relaxed);
    }
}

/// Advance the multiplexer by one LED. Call at ~10 kHz: 96 LEDs then give a
/// ~104 Hz frame rate, which is flicker-free.
///
/// Kept branch-light because it runs in an interrupt.
pub fn matrix_step() {
    let i = SCAN.fetch_add(1, Ordering::Relaxed) % 96;
    let row = i / 12;
    let col = i % 12;

    matrix_all_hiz();

    if FRAME[row].load(Ordering::Relaxed) & (1 << (11 - col)) != 0 {
        let (src, sink) = LED_PAIRS[i];
        matrix_drive(src, true);
        matrix_drive(sink, false);
    }
}
