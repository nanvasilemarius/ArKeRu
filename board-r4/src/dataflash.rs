//! The RA4M1's 8 KB data flash: erase, program, and read.
//!
//! # Provenance
//!
//! None of this is in the User's Manual. §44.7, *Programming Commands*, says
//! only "The FCB controls the programming commands" and names no register in
//! 1436 pages. The interface below comes from Renesas FSP, which is public and
//! supports `EK-RA4M1` -- see `docs/RA4M1-DATA-FLASH.md` for the full account.
//!
//! # Why this cannot brick the board
//!
//! The flash sequencer is put into one of two modes by `FENTRYR`, and the mode
//! decides which array it can touch. **This module only ever writes the data
//! flash value.** The code flash constants are deliberately absent from this
//! file: they are not defined, not named, and not reachable by a bug here.
//!
//! So the worst outcome is losing 8 KB of saved data. The firmware is in a
//! different array, the board still boots, and it still reflashes.
//!
//! Table 44.12 also confirms background operation: code may execute from code
//! flash while data flash is being written, which is why -- unlike FSP, whose
//! driver also handles code flash -- none of this has to run from RAM.

#![allow(dead_code)]

use core::ptr::{read_volatile, write_volatile};

/// Flash sequencer register block.
const FACI: usize = 0x407E_C000;

const DFLCTL: *mut u8 = (FACI + 0x090) as *mut u8;
const FPMCR: *mut u8 = (FACI + 0x100) as *mut u8;
const FASR: *mut u8 = (FACI + 0x104) as *mut u8;
const FSARL: *mut u16 = (FACI + 0x108) as *mut u16;
const FSARH: *mut u16 = (FACI + 0x110) as *mut u16;
const FCR: *mut u8 = (FACI + 0x114) as *mut u8;
const FEARL: *mut u16 = (FACI + 0x118) as *mut u16;
const FEARH: *mut u32 = (FACI + 0x120) as *mut u32;
const FSTATR1: *mut u32 = (FACI + 0x12C) as *mut u32;
const FWBL0: *mut u32 = (FACI + 0x130) as *mut u32;
const FPR: *mut u32 = (FACI + 0x180) as *mut u32;
/// `FENTRYR` selects which array the sequencer may touch. 16-bit, and the top
/// byte is a key that must read `0xAA`.
///
/// Note the offset: `0x3FB2`, nowhere near the rest of the block. There is
/// also a `FENTRYR_MF4` two bytes below it at `0x3FB0` for the MF4 flash
/// variant -- the RA4M1 is MF3 (`BSP_FEATURE_FLASH_LP_VERSION == 3`), so this
/// is the correct one. Picking the wrong register would leave the sequencer
/// in read mode while the code believed it was in P/E.
const FENTRYR: *mut u16 = (FACI + 0x3FB2) as *mut u16;

/// Data flash P/E mode. **The only `FENTRYR` value this file contains.**
const FENTRYR_DF_PE: u16 = 0xAA80;
/// Read mode -- leaves P/E entirely.
const FENTRYR_READ: u16 = 0xAA00;

/// `FPMCR` data flash P/E mode.
const FPMCR_DF_PE: u8 = 0x10;
/// `FPMCR` read mode.
const FPMCR_READ: u8 = 0x08;
/// OR'd into `FPMCR` when the MCU is *not* in high-speed operating mode.
const FPMCR_LVPE: u8 = 0x40;

const FCR_WRITE: u8 = 0x81;
const FCR_ERASE: u8 = 0x84;
const FCR_CLEAR: u8 = 0x00;

/// `FSTATR1.FRDY`, bit 6: the sequencer has finished.
const FRDY: u32 = 1 << 6;

/// `FRESETR` -- forces the sequencer back to a known state after a timeout.
const FRESETR: *mut u32 = (FACI + 0x124) as *mut u32;

/// `FSTATR2` -- where the sequencer reports that a command *failed*.
///
/// Polling `FRDY` only says the sequencer stopped, not that it succeeded. Not
/// reading this is what let a bad write report success for a whole afternoon.
const FSTATR2: *const u32 = (FACI + 0x1F0) as *const u32;
/// Programming error bits, from FSP's `FLASH_LP_FSTATR2_WRITE_ERROR_BITS`.
const ERR_WRITE: u32 = 0x12;
/// Erase error bits, from `FLASH_LP_FSTATR2_ERASE_ERROR_BITS`.
const ERR_ERASE: u32 = 0x11;

/// `FISR` -- **Flash Initial Setting Register**, and the register whose absence
/// made this driver quietly wrong.
///
/// `PCKA` in bits 5:0 tells the flash sequencer how fast `FCLK` runs, and the
/// sequencer uses it to time its own program and erase pulses. Get it wrong and
/// nothing announces itself: erases work, `FRDY` comes up on schedule, every
/// command reports success -- and the *programming* is marginal. Bytes needing
/// a few bits cleared come out right; bytes needing all eight cleared come out
/// as noise.
///
/// Which is exactly what happened. A save record is mostly small numbers and
/// zero padding, and every single byte that failed was a `0x00`.
///
/// Writable only in P/E mode, which is why it is set in [`enter_pe`].
const FISR: *mut u32 = (FACI + 0x1D8) as *mut u32;

/// System Clock Division Control Register. `FCK` in bits 30:28 and `ICK` in
/// bits 26:24, each a power-of-two divider of the same source.
const SCKDIVCR: *const u32 = 0x4001_E020 as *const u32;

/// `FCLK` in megahertz.
///
/// Derived rather than hardcoded. `ICLK` is 48 MHz on this board and both
/// clocks divide the same source, so the source is `48 << ICK` and `FCLK` is
/// that shifted down by `FCK`. A future clock change therefore fixes the flash
/// timing on its own instead of corrupting writes in a way nobody would connect
/// to the clock.
fn fclk_mhz() -> u32 {
    let d = unsafe { read_volatile(SCKDIVCR) };
    let fck = (d >> 28) & 0x7;
    let ick = (d >> 24) & 0x7;
    ((48u32 << ick) >> fck).clamp(1, 32)
}

/// Worst-case data flash block erase, from FSP's
/// `FLASH_LP_MAX_ERASE_DF_BLOCK_TIME_US`. **504 ms.**
///
/// This is not a detail to approximate. A first attempt used a fixed loop
/// count worth roughly 20 ms, which would have aborted every erase a
/// twenty-fifth of the way through and then written mode registers while the
/// sequencer was still running.
const ERASE_US: u32 = 600_000;

/// Worst-case per-byte program time, from `FLASH_LP_MAX_WRITE_DF_TIME_US`
/// (886 us), with margin.
const WRITE_US: u32 = 2_000;

/// Operating Power Control Register. `OPCM` in bits 1:0; zero means high-speed
/// mode, which decides whether `FPMCR` needs the low-voltage bit.
const OPCCR: *mut u8 = 0x4001_E0A0 as *mut u8;

/// Where the data flash is mapped for reading.
pub const BASE: u32 = 0x4010_0000;

/// Where the *sequencer* addresses the same bytes.
///
/// The data flash has two address spaces. Reads come from `0x40100000`, but
/// `FSAR` and `FEAR` take `0xFE000000`, and nothing in the register names or
/// the User's Manual hints at it -- FSP calls it "conversion to the P/E
/// address from the read address" in a one-line comment.
///
/// Getting this wrong is quiet rather than loud. An erase aimed at the read
/// address reports success and does nothing, which looked exactly like a
/// working erase for as long as the block happened to be blank already.
pub const PE_BASE: u32 = 0xFE00_0000;
/// 8 KB.
pub const SIZE: usize = 8 * 1024;
/// Erase granularity. Writes are per byte; erases are per block.
pub const BLOCK: usize = 1024;

/// Cycles for a microsecond delay at 48 MHz, rounded up generously. These
/// waits are a handful of microseconds and being late costs nothing.
fn delay_us(us: u32) {
    cortex_m::asm::delay(us * 48 + 96);
}

/// Poll `FRDY` until it matches `want`, for at most `timeout_us`.
///
/// Polls once per microsecond rather than counting loop iterations, so the
/// bound means what it says regardless of how the compiler arranges the loop.
/// Bounded at all because a wedged sequencer must not hang the shell.
fn wait_frdy(want: bool, timeout_us: u32) -> bool {
    for _ in 0..timeout_us {
        let ready = unsafe { read_volatile(FSTATR1) } & FRDY != 0;
        if ready == want {
            return true;
        }
        delay_us(1);
    }
    false
}

/// Force the sequencer back to a known state. Used only after a timeout, when
/// continuing to poke mode registers would be worse than starting over.
fn reset_sequencer() {
    unsafe {
        write_volatile(FRESETR, 1);
        write_volatile(FRESETR, 0);
    }
}

/// Write `FPMCR`, which is not an ordinary register.
///
/// It takes three writes -- the value, its complement, then the value again --
/// after unlocking with `FPR`. Nothing about the register's name suggests
/// this; it comes from `r_flash_lp_write_fpmcr` in FSP, and it is the single
/// detail that made guessing at this interface hopeless.
fn write_fpmcr(value: u8) -> bool {
    unsafe {
        write_volatile(FPR, 0xA5);
        write_volatile(FPMCR, value);
        write_volatile(FPMCR, !value);
        write_volatile(FPMCR, value);
        read_volatile(FPMCR) == value
    }
}

/// Power up the data flash controller.
///
/// Required before *reading* as well as before programming: with `DFLCTL`
/// clear, the array is not driven and every byte reads back as `0x00`. That is
/// a convincing-looking wrong answer -- an erased block should read `0xFF`, so
/// zeros mean "not powered", not "empty". FSP does this once in its open path;
/// here it is idempotent and called from each entry point.
fn enable() {
    unsafe {
        if read_volatile(DFLCTL) != 1 {
            write_volatile(DFLCTL, 1);
            // tDSTOP before the first access.
            delay_us(6);
        }
    }
}

/// Enter data flash program/erase mode.
fn enter_pe() -> bool {
    enable();
    unsafe {
        write_volatile(FENTRYR, FENTRYR_DF_PE);
    }
    // tDSTOP.
    delay_us(6);

    // The low-voltage programming bit is required unless the MCU is in
    // high-speed mode. Read it rather than assume: this board runs at 48 MHz,
    // which needs high-speed mode, but a future clock change would silently
    // corrupt writes if this were hardcoded.
    let high_speed = unsafe { read_volatile(OPCCR) } & 0x03 == 0;
    let mode = if high_speed {
        FPMCR_DF_PE
    } else {
        FPMCR_DF_PE | FPMCR_LVPE
    };
    if !write_fpmcr(mode) {
        return false;
    }

    // Tell the sequencer how fast its clock runs, or its program pulses are
    // mistimed and programming comes out marginal rather than broken. Writable
    // only in P/E mode, so it belongs here and not at start-up.
    unsafe {
        let fisr = read_volatile(FISR);
        let pcka = (fclk_mhz() - 1) & 0x1F;
        write_volatile(FISR, (fisr & !0x3F) | pcka);
    }
    true
}

/// Return to read mode. Must happen before the data flash can be read again.
fn exit_pe() -> bool {
    if !write_fpmcr(FPMCR_READ) {
        return false;
    }
    // tMS.
    delay_us(6);
    unsafe { write_volatile(FENTRYR, FENTRYR_READ) };

    for _ in 0..200_000 {
        if unsafe { read_volatile(FENTRYR) } == 0 {
            return true;
        }
    }
    false
}

/// Run one command to completion: wait for ready, clear `FCR`, wait for the
/// flag to drop again. `timeout_us` is the worst case for this command.
fn finish(timeout_us: u32, error_bits: u32) -> bool {
    if !wait_frdy(true, timeout_us) {
        reset_sequencer();
        return false;
    }
    // Ready is not the same as succeeded. FSP checks this after every command,
    // and omitting it here is what let a marginal write report a clean one.
    if unsafe { read_volatile(FSTATR2) } & error_bits != 0 {
        reset_sequencer();
        return false;
    }
    unsafe { write_volatile(FCR, FCR_CLEAR) };
    if !wait_frdy(false, timeout_us) {
        reset_sequencer();
        return false;
    }
    true
}

fn in_range(off: usize, len: usize) -> bool {
    len > 0 && off < SIZE && off + len <= SIZE
}

/// Read from the data flash. Plain memory reads -- it is mapped, and in read
/// mode no sequencer involvement is needed.
pub fn read(off: usize, out: &mut [u8]) -> bool {
    if !in_range(off, out.len()) {
        return false;
    }
    enable();
    for (i, b) in out.iter_mut().enumerate() {
        *b = unsafe { read_volatile((BASE as usize + off + i) as *const u8) };
    }
    true
}

/// Erase one 1 KB block. `off` must be block-aligned.
pub fn erase(off: usize) -> bool {
    if !in_range(off, BLOCK) || off % BLOCK != 0 {
        return false;
    }
    if !enter_pe() {
        return false;
    }

    let start = PE_BASE + off as u32;
    let end = start + BLOCK as u32 - 1;
    unsafe {
        write_volatile(FASR, 0); // user area
        write_volatile(FSARH, (start >> 16) as u16);
        write_volatile(FSARL, start as u16);
        write_volatile(FEARH, end >> 16);
        write_volatile(FEARL, end as u16);
        write_volatile(FCR, FCR_ERASE);
    }

    let ok = finish(ERASE_US, ERR_ERASE);
    exit_pe() && ok
}

/// Program bytes. The target must already be erased: flash can only clear
/// bits, so writing over data without erasing yields the AND of the two.
pub fn write(off: usize, data: &[u8]) -> bool {
    if !in_range(off, data.len()) {
        return false;
    }
    if !enter_pe() {
        return false;
    }

    let mut ok = true;
    unsafe { write_volatile(FASR, 0) };
    for (i, &b) in data.iter().enumerate() {
        let addr = PE_BASE + (off + i) as u32;
        unsafe {
            write_volatile(FSARH, (addr >> 16) as u16);
            write_volatile(FSARL, addr as u16);
            // Data flash programs one byte at a time; only the low bits of the
            // write buffer are used.
            write_volatile(FWBL0, b as u32);
            write_volatile(FCR, FCR_WRITE);
        }
        if !finish(WRITE_US, ERR_WRITE) {
            ok = false;
            break;
        }
    }

    exit_pe() && ok
}
