//! Minimal-dependency bring-up probe. Flash this before the kernel.
//!
//! Deliberately depends on as little as possible, because its job is to tell
//! you *which* assumption is wrong:
//!
//!   * no SysTick    -- delays are cycle-counting busy loops, so a bad VTOR
//!                      cannot stop the LED
//!   * no `Peripherals::take()` -- its `.unwrap()` could panic into a silent
//!                      halt before anything visible happens
//!   * VTOR set explicitly -- never assume the bootloader did it
//!   * LED blinks FIRST, before any UART or clock work
//!
//! Pattern on the L LED (D13 = P102), repeating:
//!
//!   3 slow blinks   -- "I am alive", emitted before touching any peripheral
//!   pause
//!   ~10 fast flicks -- reached the UART stage
//!   pause
//!   N slow blinks   -- N of the 3 SCIs reported their transmitter ready
//!
//! If you see the first 3 blinks but N = 0, the SCI bring-up is wrong.
//! If you see nothing at all, the image is not executing.

#![no_std]
#![no_main]

/// The probe routes no interrupts at all -- that is most of the point of it.
/// But `cortex-m-rt` is built with the `device` feature now, because the
/// kernel binary supplies its own vector table, and the feature applies to
/// every binary in the crate. So this one has to say "nothing here" out loud.
#[link_section = ".vector_table.interrupts"]
#[no_mangle]
pub static __INTERRUPTS: [unsafe extern "C" fn(); 32] = [nothing; 32];

unsafe extern "C" fn nothing() {}

use cortex_m_rt::entry;

#[path = "../ra4m1.rs"]
mod ra4m1;

const PCLK_HZ: u32 = 48_000_000;
const BAUD: u32 = 115_200;

/// Application base. The first 16 KB of flash belong to the bootloader.
const VECTOR_TABLE: u32 = 0x0000_4000;

#[entry]
fn main() -> ! {
    // Point the vector table at our image. The Arduino bootloader is widely
    // said to do this already; setting it costs one store and removes the
    // assumption entirely.
    unsafe {
        (*cortex_m::peripheral::SCB::PTR).vtor.write(VECTOR_TABLE);
    }

    // Prove execution before anything can go wrong.
    ra4m1::gpio_output(ra4m1::LED_PORT, ra4m1::LED_PIN);
    for _ in 0..3 {
        blink(400);
    }
    delay_cycles(PCLK_HZ); // 1 s gap

    ra4m1::SCI2_D1D0.init(PCLK_HZ, BAUD, &ra4m1::PINS_SCI2);
    ra4m1::SCI9_ESP.init(PCLK_HZ, BAUD, &ra4m1::PINS_SCI9);
    ra4m1::SCI1_WIFI.init(PCLK_HZ, BAUD, &ra4m1::PINS_SCI1);

    let mut round: u32 = 0;
    loop {
        let mut alive = 0u32;

        if banner(&ra4m1::SCI2_D1D0, b"[SCI2 D1/D0    ]", round) {
            alive += 1;
        }
        if banner(&ra4m1::SCI9_ESP, b"[SCI9 ESP-UART0]", round) {
            alive += 1;
        }
        if banner(&ra4m1::SCI1_WIFI, b"[SCI1 ESP-WIFI ]", round) {
            alive += 1;
        }

        // Reached the UART stage.
        for _ in 0..10 {
            blink(60);
        }
        delay_cycles(PCLK_HZ * 7 / 10);

        // How many channels answered.
        for _ in 0..alive {
            blink(400);
        }
        delay_cycles(PCLK_HZ * 3 / 2);

        round = round.wrapping_add(1);
    }
}

fn banner(sci: &ra4m1::Sci, tag: &[u8], round: u32) -> bool {
    sci.write(tag) && sci.write(b" ArduinoKernel probe #") && put_num(sci, round) && sci.write(b"\r\n")
}

/// Decimal print without `core::fmt`, to keep the probe tiny.
fn put_num(sci: &ra4m1::Sci, mut n: u32) -> bool {
    let mut buf = [0u8; 10];
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    sci.write(&buf[i..])
}

/// `ms` on, `ms` off. Cycle-counted, so it works with interrupts dead.
fn blink(ms: u32) {
    ra4m1::gpio_write(ra4m1::LED_PORT, ra4m1::LED_PIN, true);
    delay_cycles(PCLK_HZ / 1000 * ms);
    ra4m1::gpio_write(ra4m1::LED_PORT, ra4m1::LED_PIN, false);
    delay_cycles(PCLK_HZ / 1000 * ms);
}

fn delay_cycles(n: u32) {
    cortex_m::asm::delay(n);
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    // Panic must still be visible: fast permanent strobe rather than a halt.
    loop {
        ra4m1::gpio_write(ra4m1::LED_PORT, ra4m1::LED_PIN, true);
        cortex_m::asm::delay(PCLK_HZ / 40);
        ra4m1::gpio_write(ra4m1::LED_PORT, ra4m1::LED_PIN, false);
        cortex_m::asm::delay(PCLK_HZ / 40);
    }
}
