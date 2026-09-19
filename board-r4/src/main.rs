//! RA4M1 backend for the ArduinoKernel.
//!
//! The `kernel` crate is `#![forbid(unsafe_code)]`; every raw register access
//! in the system lives in [`ra4m1`]. That is the point of the split -- the
//! shell, the parser and the POST screen cannot contain a memory-safety bug,
//! because they are not allowed to express one.
//!
//! Flash `probe` first (see `src/bin/probe.rs`) to confirm which SCI channel
//! actually reaches your terminal, then set [`CONSOLE`] accordingly.

#![no_std]
#![no_main]

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use cortex_m_rt::{entry, exception};
use kernel::{Kernel, Platform, PortState, ResetCause};

mod dataflash;
mod modem;
mod ra4m1;
mod tft;

/// Set false if nothing is wired to the SPI header. Left on because `TFT ID`
/// is harmless with no panel attached -- it just reads back 0 or 0xFFFFFF.
const TFT_WIRED: bool = true;

/// Console channel: SCI9 on P109/P110, relayed to USB-C by the ESP32-S3.
///
/// This is what Arduino's own `Serial` uses on this board. From the core:
/// `Arduino.h` has `#define Serial _UART1_`, `SerialObj1.cpp` builds it from
/// `UART1_TX_PIN`/`UART1_RX_PIN` = 22/23, and `variant.cpp` maps 22/23 to
/// P109/P110 -- which `pinmux.inc` assigns to SCI channel 9.
///
/// Switch to `SCI2_D1D0` / `PINS_SCI2` to use the D1/D0 header pins with an
/// external USB-serial adapter instead.
const CONSOLE: ra4m1::Sci = ra4m1::SCI9_ESP;
const CONSOLE_PINS: [(usize, usize, u32); 2] = ra4m1::PINS_SCI9;

/// Modem link to the ESP32-S3: SCI1 on P501/P502, Arduino's `Serial2`.
/// The bridge firmware speaks an AT protocol here -- WiFi and a key/value
/// store in the ESP32's NVS.
const MODEM: ra4m1::Sci = ra4m1::SCI1_WIFI;
const MODEM_PINS: [(usize, usize, u32); 2] = ra4m1::PINS_SCI1;
const MODEM_BAUD: u32 = 115_200;

/// Clock the Arduino bootloader leaves configured before jumping to 0x4000.
const PCLK_HZ: u32 = 48_000_000;
const BAUD: u32 = 115_200;

const RAM_BYTES: usize = 32 * 1024;
const ROM_BYTES: usize = 240 * 1024; // 256K flash less the 16K bootloader

/// SysTick rate. The LED matrix multiplexer needs one interrupt per LED, and
/// 96 LEDs at 10 kHz gives a ~104 Hz frame rate -- flicker-free, and the same
/// rate the Arduino core's matrix timer uses.
const TICK_HZ: u32 = 10_000;
const TICKS_PER_MS: u32 = TICK_HZ / 1000;

/// Milliseconds since reset, for code that needs a clock without a Platform.
pub fn millis() -> u32 {
    MILLIS.load(Ordering::Relaxed)
}

/// Raw SysTick ticks, and milliseconds derived from them.
static TICKS: AtomicU32 = AtomicU32::new(0);
static MILLIS: AtomicU32 = AtomicU32::new(0);

/// Application base. The first 16 KB of flash belong to the bootloader.
const VECTOR_TABLE: u32 = 0x0000_4000;

#[entry]
fn main() -> ! {
    // Point the vector table at our image. Without this, SysTick may vector
    // into the bootloader's table, MILLIS never advances, and the first
    // delay_ms() in the POST animation spins forever.
    unsafe {
        (*cortex_m::peripheral::SCB::PTR).vtor.write(VECTOR_TABLE);
    }

    let cp = cortex_m::Peripherals::take().unwrap();

    // 1 kHz SysTick off the processor clock.
    let mut syst = cp.SYST;
    syst.set_clock_source(cortex_m::peripheral::syst::SystClkSource::Core);
    syst.set_reload(PCLK_HZ / TICK_HZ - 1);
    syst.clear_current();
    syst.enable_counter();
    syst.enable_interrupt();

    // The bootloader hands off with PRIMASK set, so SysTick was configured
    // correctly but never fired -- which is why TIME read 00:00:00.00.
    unsafe { cortex_m::interrupt::enable() };

    // A heartbeat on the built-in LED, so a dead console is distinguishable
    // from dead firmware.
    ra4m1::gpio_output(ra4m1::LED_PORT, ra4m1::LED_PIN);
    ra4m1::gpio_write(ra4m1::LED_PORT, ra4m1::LED_PIN, true);
    ra4m1::matrix_init();

    CONSOLE.init(PCLK_HZ, BAUD, &CONSOLE_PINS);
    MODEM.init(PCLK_HZ, MODEM_BAUD, &MODEM_PINS);

    // Route the console's receive event to the CPU. Three separate things, and
    // the RA4M1 stays silent if any one of them is missing: the ICU links the
    // event to a slot, the NVIC enables that slot, and the SCI raises the event
    // at all. Order matters -- the vector is already in the table, so linking
    // before enabling means a byte arriving mid-setup lands in the handler
    // rather than in whatever the empty vector used to point at.
    unsafe {
        // The bootloader's own links are still in place and point at a vector
        // table that no longer exists. Clear the lot before adding ours.
        ra4m1::icu_reset();
        ra4m1::icu_link(IRQ_CONSOLE_RX, ra4m1::EVT_SCI9_RXI);
        // Priority 0: above the SysTick that drives the LED matrix, so a
        // keystroke is never waiting on a display refresh.
        ra4m1::nvic_enable(IRQ_CONSOLE_RX, 0);
    }
    CONSOLE.enable_rx_interrupt();

    let mut plat = R4::new();
    // Before anything else touches the system registers: the flags are sticky
    // and have to be taken exactly once per boot.
    plat.latch_reset_cause();
    let mut k = Kernel::new();
    k.run(&mut plat)
}

#[exception]
fn SysTick() {
    let t = TICKS.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
    if t % TICKS_PER_MS == 0 {
        MILLIS.fetch_add(1, Ordering::Relaxed);
    }
    // Advance the matrix multiplexer by exactly one LED.
    ra4m1::matrix_step();
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    // No console guarantees here. Halt with interrupts off so the failure is
    // unambiguous under a debugger, and leave the LED on.
    cortex_m::interrupt::disable();
    loop {
        cortex_m::asm::wfi();
    }
}

/// Console receive ring, filled by the interrupt handler.
///
/// 256 bytes is about 22 ms of input at 115200 baud, which is far longer than
/// anything the foreground does between polls -- the longest is a 504 ms flash
/// erase, and nobody types during one on purpose.
const RX_CAP: usize = 256;
/// Ring for the ESP32 link. A Wi-Fi scan reply is a few hundred bytes and is
/// consumed as it arrives, so this only has to cover the gap while the console
/// blocks on its transmitter.
const MRX_CAP: usize = 256;

// ------------------------------------------------------- console receive ISR
//
// # Why this exists
//
// The SCI holds **one byte**. Until now nothing in this firmware routed a
// peripheral interrupt, so receive was polled: `Platform::get` and
// `Platform::put` were the only things that emptied `RDR`, and any stretch of
// code longer than one character time -- 87 us at 115200 -- lost input.
//
// It survived twenty versions because the shell echoes as you type, so it polls
// constantly, and because a menu discards the keys it does not want anyway. It
// broke the moment something needed a *sequence* of typed characters: laying
// out an 80x25 frame is several hundred microseconds with no I/O in it, and
// `Vlad Ursu` arrived as `VlUrsu`. The dropped bytes are visible in the save
// record dumped in 0.21.0.
//
// So the receiver is now interrupt-driven and independent of whatever the
// foreground is doing.

/// Single-producer, single-consumer ring: the handler writes, the shell reads.
///
/// No critical section anywhere. One side only ever advances `head`, the other
/// only ever advances `tail`, and the release/acquire pair on those two indices
/// is what publishes the byte -- so the shell can never see an index move
/// before the data it refers to.
struct RxRing {
    buf: UnsafeCell<[u8; RX_CAP]>,
    head: AtomicUsize,
    tail: AtomicUsize,
}

/// Safe because of the discipline above: exactly one writer, exactly one
/// reader, and they touch disjoint indices.
unsafe impl Sync for RxRing {}

impl RxRing {
    const fn new() -> Self {
        Self {
            buf: UnsafeCell::new([0; RX_CAP]),
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
        }
    }

    /// Handler side. Drops on overflow, which is the right trade: a full ring
    /// means the shell is hundreds of keystrokes behind, and the recent ones
    /// are worth more than the stale ones.
    fn push(&self, b: u8) {
        let head = self.head.load(Ordering::Relaxed);
        let next = (head + 1) % RX_CAP;
        if next == self.tail.load(Ordering::Acquire) {
            return;
        }
        unsafe { (*self.buf.get())[head] = b };
        self.head.store(next, Ordering::Release);
    }

    /// Shell side.
    fn pop(&self) -> Option<u8> {
        let tail = self.tail.load(Ordering::Relaxed);
        if tail == self.head.load(Ordering::Acquire) {
            return None;
        }
        let b = unsafe { (*self.buf.get())[tail] };
        self.tail.store((tail + 1) % RX_CAP, Ordering::Release);
        Some(b)
    }
}

static CONSOLE_RX: RxRing = RxRing::new();

/// Times the handler has run. Purely a diagnostic; see `MEM`.
static RXI_COUNT: AtomicU32 = AtomicU32::new(0);

/// ICU slot used for the console receive event. Slot number and NVIC interrupt
/// number are the same thing on this part.
const IRQ_CONSOLE_RX: usize = 0;

/// The handler. Drains `RDR` into the ring and clears the ICU request flag.
///
/// # Safety
/// Installed in the vector table below and reached only from the NVIC.
unsafe extern "C" fn console_rxi() {
    // Clear the request *first*, then take the byte. The other order loses
    // input: a byte arriving between the read and the clear sets the flag, and
    // the clear then wipes it, so that byte sits in `RDR` with no interrupt
    // pending and the receiver stops for good.
    RXI_COUNT.fetch_add(1, Ordering::Relaxed);
    ra4m1::icu_clear(IRQ_CONSOLE_RX);
    // A loop rather than a single read: `ERI` is not linked, so if an overrun
    // ever does happen this is the only thing that will clear it.
    while let Some(b) = CONSOLE.isr_read() {
        CONSOLE_RX.push(b);
    }
}

/// Nothing else is linked, so nothing else can reach the CPU. Returning rather
/// than spinning means a stray event costs one wasted entry instead of a hung
/// machine.
unsafe extern "C" fn unlinked() {}

/// The device interrupt vector table.
///
/// `cortex-m-rt` provides the sixteen ARM exception vectors and expects the
/// device support crate to supply the rest in `.vector_table.interrupts`. There
/// is no PAC here, so this is that table -- and until now it did not exist at
/// all, which meant the words after the exception vectors were whatever `.text`
/// happened to start with. Any peripheral interrupt would have jumped into the
/// middle of a function.
#[link_section = ".vector_table.interrupts"]
#[no_mangle]
pub static __INTERRUPTS: [unsafe extern "C" fn(); ra4m1::ICU_SLOTS] = {
    let mut v: [unsafe extern "C" fn(); ra4m1::ICU_SLOTS] = [unlinked; ra4m1::ICU_SLOTS];
    v[IRQ_CONSOLE_RX] = console_rxi;
    v
};

struct R4 {
    /// Ring for the ESP32 link, for the same reason the console has one: the
    /// SCI holds a single byte, and anything that blocks for longer than one
    /// character time loses the next.
    ///
    /// The console's own `put` is exactly such a blocker. Echoing a reply back
    /// to the terminal transmits at the same rate the bridge is sending, so
    /// printing and receiving are neck and neck -- and any extra output at all
    /// (a CRLF expansion, a trace line) pushes it over and drops bytes.
    /// `+PREFSTAT: 1892` came back as `+2`.
    mrx: [u8; MRX_CAP],
    mhead: usize,
    mtail: usize,
    modem: modem::Modem,
    /// Latched at startup from RSTSR0/RSTSR1, which are then cleared. Sticky
    /// hardware flags have to be taken exactly once or every later boot
    /// reports everything that has ever happened.
    reset: Option<ResetCause>,
}

impl R4 {
    const fn new() -> Self {
        Self {
            mrx: [0; MRX_CAP],
            mhead: 0,
            mtail: 0,
            modem: modem::Modem::new(MODEM),
            reset: None,
        }
    }

    /// Decode RSTSR0/RSTSR1 into a single cause, following the determination
    /// flow in section 5.3.8: RSTSR1 and the upper voltage monitors are
    /// specific, so they win; then voltage monitor 0; then power-on; and an
    /// otherwise-clean set of flags means the RESET pin.
    ///
    /// Several flags can be set at once -- the manual says so explicitly -- so
    /// the order here is most-alarming-first rather than bit order. A stack
    /// overflow that also tripped the watchdog should report the overflow.
    fn latch_reset_cause(&mut self) {
        let (r0, r1) = ra4m1::take_reset_flags();
        self.reset = Some(if r1 & (1 << 12) != 0 {
            ResetCause::StackOverflow
        } else if r1 & ((1 << 9) | (1 << 8)) != 0 {
            ResetCause::MemoryError
        } else if r1 & ((1 << 11) | (1 << 10)) != 0 {
            ResetCause::BusError
        } else if r1 & (1 << 0) != 0 {
            ResetCause::IndependentWatchdog
        } else if r1 & (1 << 1) != 0 {
            ResetCause::Watchdog
        } else if r1 & (1 << 2) != 0 {
            ResetCause::Software
        } else if r0 & ((1 << 3) | (1 << 2) | (1 << 1)) != 0 {
            ResetCause::Brownout
        } else if r0 & (1 << 0) != 0 {
            ResetCause::PowerOn
        } else {
            ResetCause::Pin
        });
    }

    /// The same, for the ESP32 link.
    fn drain_modem(&mut self) {
        while let Some(b) = MODEM.get() {
            let next = (self.mhead + 1) % MRX_CAP;
            if next == self.mtail {
                return;
            }
            self.mrx[self.mhead] = b;
            self.mhead = next;
        }
    }
}

impl Platform for R4 {
    fn put(&mut self, b: u8) {
        // The console no longer needs draining here -- its handler runs whether
        // or not this function is called, which is the whole point. The modem
        // is still polled and still has the same one-byte window.
        self.drain_modem();
        // Bounded internally; a wedged UART must not hang the kernel.
        let _ = CONSOLE.put(b);
    }

    fn get(&mut self) -> Option<u8> {
        if let Some(b) = CONSOLE_RX.pop() {
            return Some(b);
        }
        // Fallback poll, and deliberately kept even though the handler makes it
        // redundant. Wiring an interrupt on this part takes three independent
        // steps -- ICU link, NVIC enable, peripheral enable -- and every one of
        // them fails *silently*: an event that never arrives is indistinguishable
        // from a quiet line. Getting the event number wrong once already made the
        // console take no input at all.
        //
        // With this here, the worst a wiring mistake can do is put the receiver
        // back to how it behaved before, rather than switching it off.
        //
        // Masked, so the handler and this cannot both read `RDR` -- whichever
        // reads it consumes the byte, and a byte taken by one and expected by
        // the other is a byte lost.
        cortex_m::interrupt::free(|_| {
            if let Some(b) = CONSOLE.isr_read() {
                CONSOLE_RX.push(b);
            }
        });
        CONSOLE_RX.pop()
    }

    fn uptime_ms(&mut self) -> u64 {
        MILLIS.load(Ordering::Relaxed) as u64
    }

    /// Cycle-counted rather than SysTick-driven.
    ///
    /// Waiting on the MILLIS counter here would make every delay depend on the
    /// interrupt actually firing -- and if it does not, the POST animation
    /// wedges before printing a single character, which looks exactly like
    /// dead firmware. A busy loop cannot fail that way.
    fn delay_ms(&mut self, ms: u32) {
        // The console needs no draining any more -- its handler fires straight
        // through this loop, which is exactly the case that used to lose the
        // `DEL` you pressed during the POST animation. The modem still does.
        self.drain_modem();
        cortex_m::asm::delay(PCLK_HZ / 1000 * ms);
        self.drain_modem();
    }

    fn rx_irq_debug(&self) -> [u32; 12] {
        let mut out = [0u32; 12];
        out[0] = RXI_COUNT.load(Ordering::Relaxed);
        out[1] = ra4m1::nvic_ispr_read();
        out[2] = CONSOLE.scr_read() as u32;
        for (i, slot) in out.iter_mut().skip(3).enumerate() {
            *slot = ra4m1::ielsr_read(i);
        }
        out
    }

    fn cpu_name(&self) -> &'static str {
        "Renesas RA4M1 Cortex-M4F"
    }

    fn cpu_mhz(&self) -> u32 {
        PCLK_HZ / 1_000_000
    }

    fn ram_total(&self) -> usize {
        RAM_BYTES
    }

    fn ram_used(&self) -> usize {
        // Statics plus a nominal stack reservation. An exact figure would come
        // from the linker's __ebss/_stack_start symbols; this is deliberately
        // an estimate rather than a number that looks more precise than it is.
        core::mem::size_of::<Kernel>() + 2048
    }

    fn rom_total(&self) -> usize {
        ROM_BYTES
    }

    fn board_name(&self) -> &'static str {
        "Arduino UNO R4 WiFi (ABX00087)"
    }

    fn console_name(&self) -> &'static str {
        "SCI9 115200 8N1"
    }

    fn reboot(&mut self) -> ! {
        cortex_m::peripheral::SCB::sys_reset()
    }

    fn has_led_matrix(&self) -> bool {
        true
    }

    /// Hand the frame to the multiplexer; the SysTick ISR scans it out.
    fn led_matrix(&mut self, rows: &[u16; 8]) {
        ra4m1::matrix_set(rows);
    }

    fn tft_present(&self) -> bool {
        TFT_WIRED
    }

    fn tft_init(&mut self) -> u32 {
        tft::init();
        tft::read_id()
    }

    fn tft_rect(&mut self, x: u16, y: u16, w: u16, h: u16, color: u16) {
        tft::fill_rect(x, y, w, h, color);
    }

    fn modem_present(&self) -> bool {
        true
    }

    fn modem_put(&mut self, b: u8) {
        let _ = MODEM.put(b);
    }

    fn modem_get(&mut self) -> Option<u8> {
        self.drain_modem();
        if self.mtail == self.mhead {
            return None;
        }
        let b = self.mrx[self.mtail];
        self.mtail = (self.mtail + 1) % MRX_CAP;
        Some(b)
    }

    fn reset_cause(&self) -> Option<ResetCause> {
        self.reset
    }

    fn dataflash_info(&self) -> Option<(usize, usize)> {
        Some((dataflash::SIZE, dataflash::BLOCK))
    }

    fn dataflash_read(&mut self, off: usize, out: &mut [u8]) -> bool {
        dataflash::read(off, out)
    }

    fn dataflash_erase(&mut self, off: usize) -> bool {
        dataflash::erase(off)
    }

    fn dataflash_write(&mut self, off: usize, data: &[u8]) -> bool {
        dataflash::write(off, data)
    }

    fn port_count(&self) -> usize {
        6
    }

    /// Which pins exist, from the PmnPFS address list in section 19.2.5 of the
    /// User's Manual. The gaps are real silicon: there is no P009, nothing
    /// between P207 and P211, P3 stops at P307 and P5 at P505.
    ///
    /// This is the family superset. The 64-pin package this board uses bonds
    /// out fewer still, so a pin shown here may yet be absent -- but every pin
    /// *not* shown is definitely absent, which is the half that was misleading.
    fn port_pins(&self, port: usize) -> u16 {
        match port {
            0 => 0xFDFF, // P000-P008, P010-P015
            1 => 0xFFFF, // P100-P115
            2 => 0xF07F, // P200-P206, P212-P215
            3 => 0x00FF, // P300-P307
            4 => 0xFFFF, // P400-P415
            5 => 0x003F, // P500-P505
            _ => 0,
        }
    }

    fn port_state(&mut self, port: usize) -> Option<PortState> {
        if port >= 6 {
            return None;
        }
        let (level, output, peripheral, analog) = ra4m1::port_snapshot(port);
        Some(PortState {
            level,
            output,
            peripheral,
            analog,
        })
    }

    /// Only what this firmware actually configures, taken from the constants
    /// that configure it -- so the table cannot drift away from the wiring.
    /// The Arduino header labels are deliberately absent: they come from a
    /// variant file rather than from anything this code proves, and `PINS
    /// WATCH` identifies a header pin properly by grounding it.
    fn pin_note(&self, port: usize, pin: usize) -> Option<&'static str> {
        match (port, pin) {
            (1, 9) => Some("console TX  SCI9"),
            (1, 10) => Some("console RX  SCI9"),
            (5, 1) => Some("ESP32 TX    SCI1"),
            (5, 2) => Some("ESP32 RX    SCI1"),
            (3, 2) => Some("header TX   SCI2"),
            (3, 1) => Some("header RX   SCI2"),
            (ra4m1::LED_PORT, ra4m1::LED_PIN) => Some("built-in LED"),
            _ if ra4m1::is_matrix_pin(port, pin) => Some("LED matrix"),
            _ => None,
        }
    }

    /// Pull up one named pin, refusing the ones actually in use.
    ///
    /// "In use" means in use *now*: switched to a peripheral, or driving as an
    /// output. Plus the matrix, which the scan interrupt flips between input
    /// and output thousands of times a second and would therefore look idle
    /// whenever it was sampled.
    ///
    /// Refusing on [`Platform::pin_note`] instead was the first attempt, and it
    /// was backwards. That refused `P3.01`/`P3.02` -- the D0/D1 header pins,
    /// which have a note only because a `SCI2` constant names them, and which
    /// sit as idle GPIO because nothing ever initialises that channel. They
    /// are the *best* pins to test with, because their destination is known:
    /// two holes in the header. Meanwhile every pin the firmware has never
    /// heard of sailed through. The note table describes what this code
    /// configures; it is not a map of the board.
    fn pin_pullup(&mut self, port: usize, pin: usize, on: bool) -> bool {
        if port >= 6 || pin >= 16 || ra4m1::is_matrix_pin(port, pin) {
            return false;
        }
        let (_, output, peripheral, _) = ra4m1::port_snapshot(port);
        let m = 1u16 << pin;
        if output & m != 0 || peripheral & m != 0 {
            return false;
        }
        ra4m1::set_pullup(port, pin, on);
        true
    }

    fn settings_get_u8(&mut self, key: &str) -> Option<u8> {
        self.modem.get_u8(key)
    }

    fn settings_set_u8(&mut self, key: &str, val: u8) -> bool {
        self.modem.set_u8(key, val)
    }
}
