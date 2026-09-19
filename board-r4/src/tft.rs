//! ILI9341 320x240 SPI display driver.
//!
//! **SPI is bit-banged, deliberately.** The RA4M1 has an SPI0 peripheral on
//! exactly these pins, and using it would be faster. But this driver could not
//! be tested against a panel while it was written, and bit-banging reuses the
//! GPIO primitives already proven on hardware by the LED matrix rather than
//! stacking untested SPI register setup underneath untested display init.
//! Swap in SPI0 once a panel has verified everything above it.
//!
//! Throughput is roughly 4 Mbit/s, so a full-screen fill takes ~0.3 s. Fine for
//! a status display, and the text path only repaints the cells it touches.
//!
//! Wiring (standard Arduino ILI9341 breakout):
//!
//! ```text
//!   TFT SCK  -> D13  (P102)   also LED_BUILTIN; the LED flickers on traffic
//!   TFT MOSI -> D11  (P411)
//!   TFT MISO -> D12  (P410)   only needed for ID readback
//!   TFT CS   -> D10  (P103)
//!   TFT DC   -> D9   (P303)
//!   TFT RST  -> D8   (P304)
//!   TFT VCC  -> 5V (or 3V3 -- check your breakout)
//!   TFT GND  -> GND
//! ```

use crate::ra4m1::{gpio_input, gpio_output, gpio_read, gpio_write};

// (port, pin) for each line, from variants/UNOWIFIR4/variant.cpp.
const SCK: (usize, usize) = (1, 2); // D13 / P102
const MOSI: (usize, usize) = (4, 11); // D11 / P411
const MISO: (usize, usize) = (4, 10); // D12 / P410
const CS: (usize, usize) = (1, 3); // D10 / P103
const DC: (usize, usize) = (3, 3); // D9  / P303
const RST: (usize, usize) = (3, 4); // D8  / P304

pub const WIDTH: u16 = 320;
pub const HEIGHT: u16 = 240;

// --- bit-banged SPI mode 0 --------------------------------------------------

#[inline(always)]
fn write_byte(b: u8) {
    for i in (0..8).rev() {
        gpio_write(MOSI.0, MOSI.1, b & (1 << i) != 0);
        gpio_write(SCK.0, SCK.1, true);
        gpio_write(SCK.0, SCK.1, false);
    }
}

#[inline(always)]
fn read_byte() -> u8 {
    let mut v = 0u8;
    for _ in 0..8 {
        gpio_write(SCK.0, SCK.1, true);
        v = (v << 1) | gpio_read(MISO.0, MISO.1) as u8;
        gpio_write(SCK.0, SCK.1, false);
    }
    v
}

fn cmd(c: u8) {
    gpio_write(DC.0, DC.1, false);
    gpio_write(CS.0, CS.1, false);
    write_byte(c);
    gpio_write(CS.0, CS.1, true);
}

fn data(bytes: &[u8]) {
    gpio_write(DC.0, DC.1, true);
    gpio_write(CS.0, CS.1, false);
    for &b in bytes {
        write_byte(b);
    }
    gpio_write(CS.0, CS.1, true);
}

fn delay_ms(ms: u32) {
    cortex_m::asm::delay(48_000 * ms);
}

// --- ILI9341 ----------------------------------------------------------------

/// Landscape, BGR order. 0x28 rotates the panel 90 degrees from its native
/// portrait orientation, which is what makes it 320 wide.
const MADCTL_LANDSCAPE: u8 = 0x28;

pub fn init() {
    for p in [SCK, MOSI, CS, DC, RST] {
        gpio_output(p.0, p.1);
    }
    gpio_input(MISO.0, MISO.1);
    gpio_write(CS.0, CS.1, true);
    gpio_write(SCK.0, SCK.1, false);

    // Hardware reset: the panel needs a low pulse and time to come back.
    gpio_write(RST.0, RST.1, true);
    delay_ms(5);
    gpio_write(RST.0, RST.1, false);
    delay_ms(20);
    gpio_write(RST.0, RST.1, true);
    delay_ms(150);

    cmd(0x01); // software reset as well, belt and braces
    delay_ms(150);

    // Power/timing block. These are the manufacturer's recommended values;
    // they are magic numbers in every ILI9341 driver in existence.
    cmd(0xCF);
    data(&[0x00, 0xC1, 0x30]);
    cmd(0xED);
    data(&[0x64, 0x03, 0x12, 0x81]);
    cmd(0xE8);
    data(&[0x85, 0x00, 0x78]);
    cmd(0xCB);
    data(&[0x39, 0x2C, 0x00, 0x34, 0x02]);
    cmd(0xF7);
    data(&[0x20]);
    cmd(0xEA);
    data(&[0x00, 0x00]);

    cmd(0xC0);
    data(&[0x23]); // power control 1
    cmd(0xC1);
    data(&[0x10]); // power control 2
    cmd(0xC5);
    data(&[0x3E, 0x28]); // VCOM 1
    cmd(0xC7);
    data(&[0x86]); // VCOM 2

    cmd(0x36);
    data(&[MADCTL_LANDSCAPE]);
    cmd(0x3A);
    data(&[0x55]); // 16 bits per pixel, RGB565
    cmd(0xB1);
    data(&[0x00, 0x18]); // frame rate
    cmd(0xB6);
    data(&[0x08, 0x82, 0x27]); // display function

    cmd(0xF2);
    data(&[0x00]); // 3-gamma off
    cmd(0x26);
    data(&[0x01]); // gamma curve 1
    cmd(0xE0);
    data(&[
        0x0F, 0x31, 0x2B, 0x0C, 0x0E, 0x08, 0x4E, 0xF1, 0x37, 0x07, 0x10, 0x03, 0x0E, 0x09, 0x00,
    ]);
    cmd(0xE1);
    data(&[
        0x00, 0x0E, 0x14, 0x03, 0x11, 0x07, 0x31, 0xC1, 0x48, 0x08, 0x0F, 0x0C, 0x31, 0x36, 0x0F,
    ]);

    cmd(0x11); // sleep out
    delay_ms(120);
    cmd(0x29); // display on
    delay_ms(20);
}

/// Read the controller ID (command 0xD3). A correctly wired ILI9341 answers
/// `0x009341`. Anything else means the panel is absent, miswired, or not an
/// ILI9341 -- which is the whole point of having this.
pub fn read_id() -> u32 {
    gpio_write(DC.0, DC.1, false);
    gpio_write(CS.0, CS.1, false);
    write_byte(0xD3);
    gpio_write(DC.0, DC.1, true);
    let _dummy = read_byte();
    let a = read_byte();
    let b = read_byte();
    let c = read_byte();
    gpio_write(CS.0, CS.1, true);
    ((a as u32) << 16) | ((b as u32) << 8) | c as u32
}

/// Select the rectangle that subsequent pixel writes fill.
fn set_window(x: u16, y: u16, w: u16, h: u16) {
    let x1 = x + w - 1;
    let y1 = y + h - 1;
    cmd(0x2A);
    data(&[(x >> 8) as u8, x as u8, (x1 >> 8) as u8, x1 as u8]);
    cmd(0x2B);
    data(&[(y >> 8) as u8, y as u8, (y1 >> 8) as u8, y1 as u8]);
    cmd(0x2C); // memory write
}

/// Fill a rectangle with a single RGB565 colour. Clipped to the panel.
pub fn fill_rect(x: u16, y: u16, w: u16, h: u16, color: u16) {
    if x >= WIDTH || y >= HEIGHT || w == 0 || h == 0 {
        return;
    }
    let w = w.min(WIDTH - x);
    let h = h.min(HEIGHT - y);

    set_window(x, y, w, h);
    let hi = (color >> 8) as u8;
    let lo = color as u8;

    gpio_write(DC.0, DC.1, true);
    gpio_write(CS.0, CS.1, false);
    for _ in 0..(w as u32 * h as u32) {
        write_byte(hi);
        write_byte(lo);
    }
    gpio_write(CS.0, CS.1, true);
}
