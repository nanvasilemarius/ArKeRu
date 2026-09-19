//! Text and test patterns for an SPI TFT panel.
//!
//! The kernel owns presentation; the board crate only exposes a rectangle
//! fill. Every glyph pixel becomes one `tft_rect` call of `scale x scale`, so
//! there is no framebuffer and no pixel buffer — on a 32 KB part a 320x240
//! RGB565 framebuffer would need 150 KB, which is not a rounding error away
//! from fitting.
//!
//! The font is the same 5x7 table the LED matrix marquee uses.

use crate::marquee::{glyph_col, GLYPH_H, GLYPH_W};
use crate::Platform;

pub const WIDTH: u16 = 320;
pub const HEIGHT: u16 = 240;

// RGB565.
pub const BLACK: u16 = 0x0000;
pub const WHITE: u16 = 0xFFFF;
pub const RED: u16 = 0xF800;
pub const GREEN: u16 = 0x07E0;
pub const BLUE: u16 = 0x001F;
pub const CYAN: u16 = 0x07FF;
pub const MAGENTA: u16 = 0xF81F;
pub const YELLOW: u16 = 0xFFE0;
pub const GREY: u16 = 0x8410;
/// The POST screen's blue, so the panel matches the serial console.
pub const POST_BG: u16 = 0x0010;

/// Advance per character, in font pixels (glyph plus one column of spacing).
const ADVANCE: usize = GLYPH_W + 1;

/// Characters that fit on a line at the given scale.
pub fn cols(scale: u16) -> usize {
    WIDTH as usize / (ADVANCE * scale as usize)
}

/// Text rows that fit at the given scale.
pub fn rows(scale: u16) -> usize {
    HEIGHT as usize / ((GLYPH_H + 1) * scale as usize)
}

/// Draw one character with its top-left corner at (x, y).
///
/// `bg` of `None` leaves untouched pixels alone, which is much faster but
/// leaves whatever was there before showing through.
pub fn glyph<P: Platform + ?Sized>(
    p: &mut P,
    x: u16,
    y: u16,
    ch: u8,
    scale: u16,
    fg: u16,
    bg: Option<u16>,
) {
    let s = scale.max(1);
    for c in 0..GLYPH_W {
        let bits = glyph_col(ch, c);
        for r in 0..GLYPH_H {
            let on = bits & (1 << r) != 0;
            let color = if on {
                fg
            } else {
                match bg {
                    Some(b) => b,
                    None => continue,
                }
            };
            p.tft_rect(x + c as u16 * s, y + r as u16 * s, s, s, color);
        }
    }
    // The spacing column, so consecutive characters do not run together.
    if let Some(b) = bg {
        p.tft_rect(x + GLYPH_W as u16 * s, y, s, GLYPH_H as u16 * s, b);
    }
}

/// Draw a string starting at (x, y), clipped at the right edge.
pub fn text<P: Platform + ?Sized>(
    p: &mut P,
    x: u16,
    y: u16,
    s: &str,
    scale: u16,
    fg: u16,
    bg: Option<u16>,
) {
    let sc = scale.max(1);
    let step = (ADVANCE as u16) * sc;
    let mut cx = x;
    for b in s.bytes() {
        if cx + step > WIDTH {
            break;
        }
        glyph(p, cx, y, b, sc, fg, bg);
        cx += step;
    }
}

/// Centre a string horizontally on the given row.
pub fn text_centred<P: Platform + ?Sized>(
    p: &mut P,
    y: u16,
    s: &str,
    scale: u16,
    fg: u16,
    bg: Option<u16>,
) {
    let sc = scale.max(1);
    let w = s.len() as u16 * (ADVANCE as u16) * sc;
    let x = if w >= WIDTH { 0 } else { (WIDTH - w) / 2 };
    text(p, x, y, s, sc, fg, bg);
}

pub fn clear<P: Platform + ?Sized>(p: &mut P, color: u16) {
    p.tft_rect(0, 0, WIDTH, HEIGHT, color);
}

/// Eight vertical colour bars plus a label.
///
/// Deliberately diagnostic: wrong bar order means the panel is in BGR rather
/// than RGB mode, and bars in the wrong place mean the rotation is off.
pub fn test_pattern<P: Platform + ?Sized>(p: &mut P) {
    const BARS: [u16; 8] = [WHITE, YELLOW, CYAN, GREEN, MAGENTA, RED, BLUE, GREY];
    let w = WIDTH / BARS.len() as u16;
    for (i, c) in BARS.iter().enumerate() {
        p.tft_rect(i as u16 * w, 0, w, HEIGHT - 40, *c);
    }
    p.tft_rect(0, HEIGHT - 40, WIDTH, 40, BLACK);
    text(p, 4, HEIGHT - 28, "WHT YEL CYN GRN MAG RED BLU GRY", 1, WHITE, None);
}

/// A boot banner matching the serial POST screen.
pub fn banner<P: Platform + ?Sized>(p: &mut P) {
    let board = p.board_name();
    let cpu = p.cpu_name();
    let mhz = p.cpu_mhz();

    clear(p, POST_BG);
    p.tft_rect(0, 0, WIDTH, 28, BLUE);
    text(p, 6, 8, "ARDUINOKERNEL BIOS", 2, WHITE, None);

    text(p, 6, 44, board, 1, CYAN, None);
    text(p, 6, 60, cpu, 1, WHITE, None);

    // Small helper so the MHz value can be drawn without allocating.
    let mut buf = [0u8; 16];
    let n = fmt_u32(&mut buf, mhz);
    text(p, 6, 76, "CPU MHZ:", 1, GREY, None);
    text(p, 60, 76, core::str::from_utf8(&buf[..n]).unwrap_or("?"), 1, GREEN, None);

    text(p, 6, 104, "A:\\>", 2, GREEN, None);
}

/// Decimal formatting without `core::fmt`, for the few numbers drawn here.
fn fmt_u32(buf: &mut [u8; 16], mut v: u32) -> usize {
    if v == 0 {
        buf[0] = b'0';
        return 1;
    }
    let mut tmp = [0u8; 10];
    let mut i = 0;
    while v > 0 {
        tmp[i] = b'0' + (v % 10) as u8;
        v /= 10;
        i += 1;
    }
    for j in 0..i {
        buf[j] = tmp[i - 1 - j];
    }
    i
}
