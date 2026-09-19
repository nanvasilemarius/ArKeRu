//! ArduinoKernel -- a DOS-style command kernel with a BIOS-style POST screen.
//!
//! `no_std`, no allocator, no static mut. Everything the kernel needs lives in
//! one `Kernel` value that the board crate owns; the only heap-shaped thing is
//! a fixed-size line buffer.
//!
//! The kernel talks to hardware exclusively through [`Platform`]. That is what
//! lets the exact same code run on the RA4M1 over a UART and on a PC terminal
//! for development -- see the `host` crate.

#![no_std]
#![forbid(unsafe_code)]

pub mod batch;
pub mod cmds;
pub mod df;
pub mod dlc;
pub mod dino;
pub mod esp;
pub mod i18n;
pub mod marquee;
pub mod pager;
pub mod pins;
pub mod post;
pub mod romfs;
pub mod rpg;
pub mod screen;
pub mod setup;
pub mod shell;
pub mod tft;
pub mod ttt;
pub mod vt;

/// One GPIO port's worth of state, as four bitmaps over its 16 pins.
///
/// Bitmaps rather than a per-pin struct because that is the shape the silicon
/// already has: one register read per field instead of sixteen, and the whole
/// port fits in 8 bytes.
#[derive(Clone, Copy, Default)]
pub struct PortState {
    /// Input data: the level actually present on the pin.
    pub level: u16,
    /// Direction: 1 = output.
    pub output: u16,
    /// 1 = the pin belongs to a peripheral, not to GPIO.
    pub peripheral: u16,
    /// 1 = the pin is switched to analog.
    pub analog: u16,
}

/// Why the machine last restarted.
///
/// Firmware has always been able to tell you this and it is genuinely useful:
/// a board that reboots on its own looks identical to one you reset yourself
/// until something says which it was.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ResetCause {
    /// Power was applied. A cold start.
    PowerOn,
    /// The RESET pin was driven -- the button, or the bootloader.
    Pin,
    /// Software asked for it. `REBOOT` and `EXIT` land here.
    Software,
    /// The watchdog was not fed: the machine had stopped making progress.
    Watchdog,
    /// The independent watchdog, which runs from its own oscillator.
    IndependentWatchdog,
    /// Supply voltage fell below a monitored threshold.
    Brownout,
    /// The stack pointer left its permitted range.
    StackOverflow,
    /// SRAM parity or ECC error.
    MemoryError,
    /// A bus master or slave violated the memory protection unit.
    BusError,
}

/// Everything the kernel needs from the machine underneath it.
///
/// Byte I/O is deliberately non-blocking on the read side: the shell polls, so
/// a board implementation can service interrupts or sleep between calls without
/// the kernel knowing or caring.
pub trait Platform {
    /// Write one byte to the console.
    fn put(&mut self, b: u8);

    /// Read one byte from the console, or `None` if nothing is pending.
    fn get(&mut self) -> Option<u8>;

    /// Milliseconds since reset.
    fn uptime_ms(&mut self) -> u64;

    /// Busy-wait. Used only by the POST animation.
    fn delay_ms(&mut self, ms: u32);

    fn cpu_name(&self) -> &'static str;
    fn cpu_mhz(&self) -> u32;

    /// Total RAM in bytes.
    fn ram_total(&self) -> usize;
    /// Bytes of RAM currently spoken for (static + stack high-water estimate).
    fn ram_used(&self) -> usize;
    /// Total program flash in bytes.
    fn rom_total(&self) -> usize;

    /// Machine name shown on the POST screen.
    fn board_name(&self) -> &'static str;

    /// Console channel description for the POST device table, e.g.
    /// "SCI9 P109/P110". Kept out of `post.rs` so the boot screen cannot drift
    /// out of sync with the hardware it is describing.
    fn console_name(&self) -> &'static str {
        "UART"
    }

    /// Reset the machine. On the host harness this just exits.
    fn reboot(&mut self) -> !;

    /// Why the machine last restarted, if the platform can tell.
    ///
    /// Latched at startup rather than read on demand: the hardware flags are
    /// sticky across resets and have to be read and cleared once, early, or
    /// every subsequent boot reports the union of everything that ever
    /// happened.
    fn reset_cause(&self) -> Option<ResetCause> {
        None
    }

    /// Optional: draw one frame on a 12x8 LED matrix, row-major, 12 bits per
    /// row packed into the low bits of each `u16`. Boards without one ignore it.
    fn led_matrix(&mut self, _rows: &[u16; 8]) {}

    /// Whether this platform actually has an LED matrix wired up.
    fn has_led_matrix(&self) -> bool {
        false
    }

    // --- SPI TFT ------------------------------------------------------------
    //
    // Deliberately a single rectangle fill rather than a framebuffer: 320x240
    // RGB565 is 150 KB, and this part has 32 KB. The kernel composes glyphs
    // and patterns out of rectangles in `tft`.

    /// Whether a TFT is wired up at all (independent of whether one answered).
    fn tft_present(&self) -> bool {
        false
    }

    /// Reset and initialise the panel. Returns the controller ID, or 0 if
    /// nothing answered -- an ILI9341 reports `0x009341`.
    fn tft_init(&mut self) -> u32 {
        0
    }

    /// Fill a rectangle with an RGB565 colour.
    fn tft_rect(&mut self, _x: u16, _y: u16, _w: u16, _h: u16, _color: u16) {}

    // --- ESP32-S3 modem link -----------------------------------------------
    //
    // A second UART to the ESP32-S3, separate from the console. The stock
    // bridge firmware speaks an AT-style protocol on it: WiFi, and a key/value
    // store backed by the ESP32's own NVS. That store is how settings persist
    // without ever writing RA4M1 flash.

    /// Whether a modem UART is wired up.
    fn modem_present(&self) -> bool {
        false
    }

    fn modem_put(&mut self, _b: u8) {}

    fn modem_get(&mut self) -> Option<u8> {
        None
    }

    // --- GPIO inspection -----------------------------------------------------
    //
    // Read-only on purpose. Reporting what a pin is doing cannot break
    // anything; driving one that is already being driven from the other end
    // is bus contention, and this is a diagnostic, not a bench supply.

    /// How many GPIO ports this platform can report on. 0 means none.
    fn port_count(&self) -> usize {
        0
    }

    /// Which of a port's sixteen pins physically exist, as a bitmask.
    ///
    /// Ports are sixteen bits wide in the register map but not on the die:
    /// this part has no `P009`, nothing between `P207` and `P211`, and `P3`
    /// stops at `P307`. Without this the map draws cells for pins that cannot
    /// exist, and a permanent `0` is indistinguishable from a grounded input.
    fn port_pins(&self, _port: usize) -> u16 {
        0xFFFF
    }

    /// Snapshot one port, or `None` if it does not exist here.
    fn port_state(&mut self, _port: usize) -> Option<PortState> {
        None
    }

    /// What a pin is wired to on this board, if anything known. Kept on the
    /// platform because it is a fact about the hardware, not about the shell.
    fn pin_note(&self, _port: usize, _pin: usize) -> Option<&'static str> {
        None
    }

    /// Enable or disable the internal pull-up on **one named pin**.
    ///
    /// Deliberately one pin at a time, and never automatic.
    ///
    /// An earlier version applied pull-ups to every pin that looked free --
    /// not a peripheral, not an output, not named in [`Platform::pin_note`] --
    /// so that an unconnected input would read 1 and grounding it would read
    /// 0. That took the board off USB entirely and needed a physical replug.
    ///
    /// The mistake was treating "this firmware has not claimed it" as "nothing
    /// is connected to it". Those are different sets: `pin_note` knows what
    /// *this code* configures, and knows nothing about what the *board*
    /// wires -- the ESP32's control lines, the USB mux, anything on a net the
    /// firmware never touches. A 33 kohm pull-up is weak, but on a
    /// high-impedance control input with nothing else driving it, weak is
    /// entirely enough to change the level.
    ///
    /// So the caller names a pin and owns the consequence.
    fn pin_pullup(&mut self, _port: usize, _pin: usize, _on: bool) -> bool {
        false
    }

    // --- data flash ----------------------------------------------------------
    //
    // Byte offsets rather than addresses, so the kernel never handles a raw
    // pointer and cannot name a location outside the array.

    /// `(size, erase block size)` if this platform has data flash.
    fn dataflash_info(&self) -> Option<(usize, usize)> {
        None
    }

    fn dataflash_read(&mut self, _off: usize, _out: &mut [u8]) -> bool {
        false
    }

    /// Erase one block. `off` must be block-aligned.
    fn dataflash_erase(&mut self, _off: usize) -> bool {
        false
    }

    /// Program bytes into already-erased space.
    fn dataflash_write(&mut self, _off: usize, _data: &[u8]) -> bool {
        false
    }

    /// How many times the console receive interrupt has run.
    ///
    /// A diagnostic, and a pointed one: wiring an interrupt on the RA4M1 takes
    /// three independent steps that all fail silently, so "is it actually
    /// firing?" is otherwise unanswerable from the outside. Zero here with a
    /// working console means the polled fallback is carrying it.
    /// `[count, IELSR0, NVIC ISER0, NVIC ISPR0, SCI SCR]`.
    ///
    /// Wiring an interrupt on this part takes three independent steps that all
    /// fail silently, so the only way to tell them apart is to read back what
    /// each one actually did.
    fn rx_irq_debug(&self) -> [u32; 12] {
        [0; 12]
    }

    /// Read a persisted `u8` setting. `None` if absent or unavailable.
    fn settings_get_u8(&mut self, _key: &str) -> Option<u8> {
        None
    }

    /// Persist a `u8` setting. Returns whether it was stored.
    fn settings_set_u8(&mut self, _key: &str, _val: u8) -> bool {
        false
    }
}

/// A `core::fmt::Write` adapter over a [`Platform`], translating `\n` to `\r\n`
/// so output is correct on a raw serial terminal.
pub struct Con<'p, P: Platform + ?Sized>(pub &'p mut P);

impl<P: Platform + ?Sized> core::fmt::Write for Con<'_, P> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for b in s.bytes() {
            if b == b'\n' {
                self.0.put(b'\r');
            }
            self.0.put(b);
        }
        Ok(())
    }
}

/// Convenience: `kprint!(plat, "x = {}", x)`.
///
/// The `&mut *` reborrow matters: without it the macro would move the caller's
/// `&mut` into `Con` and the next call would not compile.
#[macro_export]
macro_rules! kprint {
    ($p:expr, $($arg:tt)*) => {{
        use core::fmt::Write as _;
        let _ = write!($crate::Con(&mut *$p), $($arg)*);
    }};
}

/// Convenience: `kprintln!(plat, "line")`.
#[macro_export]
macro_rules! kprintln {
    ($p:expr) => {{ $crate::kprint!($p, "\n"); }};
    ($p:expr, $($arg:tt)*) => {{
        use core::fmt::Write as _;
        let _ = writeln!($crate::Con(&mut *$p), $($arg)*);
    }};
}

/// The translated name of a reset cause, or "unknown" where the platform
/// cannot tell -- the host harness, for one.
pub fn reset_cause_msg(c: Option<ResetCause>) -> i18n::Msg {
    use i18n::Msg;
    match c {
        Some(ResetCause::PowerOn) => Msg::ResetPoweron,
        Some(ResetCause::Pin) => Msg::ResetPin,
        Some(ResetCause::Software) => Msg::ResetSoftware,
        Some(ResetCause::Watchdog) => Msg::ResetWdt,
        Some(ResetCause::IndependentWatchdog) => Msg::ResetIwdt,
        Some(ResetCause::Brownout) => Msg::ResetBrownout,
        Some(ResetCause::StackOverflow) => Msg::ResetStack,
        Some(ResetCause::MemoryError) => Msg::ResetMemory,
        Some(ResetCause::BusError) => Msg::ResetBus,
        None => Msg::ResetUnknown,
    }
}

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Maximum command line length, in bytes. DOS used 127; so do we.
pub const LINE_MAX: usize = 127;
/// Number of recalled command lines kept for the up/down arrows.
pub const HISTORY: usize = 8;

/// The whole machine state. One of these lives in the board crate's `main`.
pub struct Kernel {
    pub shell: shell::Shell,
    /// Drives scrolling text on the LED matrix, independently of the shell.
    pub marquee: marquee::Marquee,
    /// Current drive letter shown in the prompt. Cosmetic; there is one volume.
    pub drive: u8,
}

impl Default for Kernel {
    fn default() -> Self {
        Self::new()
    }
}

impl Kernel {
    pub const fn new() -> Self {
        Self {
            shell: shell::Shell::new(),
            marquee: marquee::Marquee::new(),
            drive: b'A',
        }
    }

    /// Run the POST screen, then the shell. Never returns under normal use.
    ///
    /// The shell and the marquee are polled in the same loop: neither blocks,
    /// so scrolling text keeps animating while you type.
    pub fn run<P: Platform>(&mut self, p: &mut P) -> ! {
        // Restore the saved language first, so even the POST screen comes up
        // in it. A missing or unreachable store just leaves the default.
        if let Some(idx) = p.settings_get_u8(i18n::LANG_KEY) {
            i18n::set_lang(idx);
        }
        // Stored as ms/10 so it fits the byte-sized settings store.
        //
        // A key that was never written reads back as 0, which is not a valid
        // interval -- feeding it to set_speed clamped to the 10 ms floor and
        // made every fresh board scroll at maximum speed. Zero means absent.
        match p.settings_get_u8(setup::SPEED_KEY) {
            Some(s) if s > 0 => self.marquee.set_speed(s as u32 * 10),
            _ => {}
        }
        // Stored as index+1 for the same reason: 0 means "never written", and
        // the default has to be reachable as a deliberate choice too.
        match p.settings_get_u8(vt::THEME_KEY) {
            Some(t) if t > 0 => vt::set_theme_index(t - 1),
            _ => {}
        }
        // 1 means off, 2 means on. Zero is "never written", not "off".
        match p.settings_get_u8(esp::TRACE_KEY) {
            Some(t) if t > 0 => esp::set_trace(t == 2),
            _ => {}
        }
        // Rows + 1, because zero rows means "never page" and has to stay
        // distinguishable from a key that was never written.
        match p.settings_get_u8(pager::PAGE_KEY) {
            Some(n) if n > 0 => pager::set_rows(n - 1),
            _ => {}
        }
        post::run(p);
        self.shell.banner(p, self.drive);
        batch::autoexec(p, &mut self.marquee, self.drive);
        self.shell.prompt(p, self.drive);
        loop {
            self.shell.poll(p, &mut self.marquee, self.drive);
            self.marquee.tick(p);
        }
    }
}
