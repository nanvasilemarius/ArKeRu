//! Text rendering and scrolling for the 12x8 LED matrix.
//!
//! A 12x8 panel holds two 5x7 characters at a time, so anything worth saying
//! has to move. This renders a message as a strip of vertical column slices and
//! slides a 12-column window along it.
//!
//! There is no column buffer: column `i` of the strip is computed on demand
//! from `text[i / 6]`, so a 48-character message costs 48 bytes rather than the
//! 288 a rasterised strip would need. On a 32 KB part that is worth the divide.

use crate::Platform;

/// Longest message. 48 chars x 6 columns = 288 columns of scroll.
pub const TEXT_MAX: usize = 48;

/// Columns per character: 5 of glyph plus 1 of spacing.
const ADVANCE: usize = 6;
const COLS: usize = 12;
const ROWS: usize = 8;

/// Blank columns appended after the message so it scrolls fully off before
/// wrapping, instead of the tail running into the head.
const GAP: usize = COLS;

/// 5x7 font, ASCII 0x20..0x5F. Each glyph is 5 column bitmaps; within a column
/// bit 0 is the TOP row. Lowercase is folded to uppercase before lookup.
#[rustfmt::skip]
static FONT: [[u8; 5]; 64] = [
    [0x00,0x00,0x00,0x00,0x00], // ' '
    [0x00,0x00,0x5F,0x00,0x00], // !
    [0x00,0x07,0x00,0x07,0x00], // "
    [0x14,0x7F,0x14,0x7F,0x14], // #
    [0x24,0x2A,0x7F,0x2A,0x12], // $
    [0x23,0x13,0x08,0x64,0x62], // %
    [0x36,0x49,0x55,0x22,0x50], // &
    [0x00,0x05,0x03,0x00,0x00], // '
    [0x00,0x1C,0x22,0x41,0x00], // (
    [0x00,0x41,0x22,0x1C,0x00], // )
    [0x14,0x08,0x3E,0x08,0x14], // *
    [0x08,0x08,0x3E,0x08,0x08], // +
    [0x00,0x50,0x30,0x00,0x00], // ,
    [0x08,0x08,0x08,0x08,0x08], // -
    [0x00,0x60,0x60,0x00,0x00], // .
    [0x20,0x10,0x08,0x04,0x02], // /
    [0x3E,0x51,0x49,0x45,0x3E], // 0
    [0x00,0x42,0x7F,0x40,0x00], // 1
    [0x42,0x61,0x51,0x49,0x46], // 2
    [0x21,0x41,0x45,0x4B,0x31], // 3
    [0x18,0x14,0x12,0x7F,0x10], // 4
    [0x27,0x45,0x45,0x45,0x39], // 5
    [0x3C,0x4A,0x49,0x49,0x30], // 6
    [0x01,0x71,0x09,0x05,0x03], // 7
    [0x36,0x49,0x49,0x49,0x36], // 8
    [0x06,0x49,0x49,0x29,0x1E], // 9
    [0x00,0x36,0x36,0x00,0x00], // :
    [0x00,0x56,0x36,0x00,0x00], // ;
    [0x08,0x14,0x22,0x41,0x00], // <
    [0x14,0x14,0x14,0x14,0x14], // =
    [0x00,0x41,0x22,0x14,0x08], // >
    [0x02,0x01,0x51,0x09,0x06], // ?
    [0x32,0x49,0x79,0x41,0x3E], // @
    [0x7E,0x11,0x11,0x11,0x7E], // A
    [0x7F,0x49,0x49,0x49,0x36], // B
    [0x3E,0x41,0x41,0x41,0x22], // C
    [0x7F,0x41,0x41,0x22,0x1C], // D
    [0x7F,0x49,0x49,0x49,0x41], // E
    [0x7F,0x09,0x09,0x09,0x01], // F
    [0x3E,0x41,0x49,0x49,0x7A], // G
    [0x7F,0x08,0x08,0x08,0x7F], // H
    [0x00,0x41,0x7F,0x41,0x00], // I
    [0x20,0x40,0x41,0x3F,0x01], // J
    [0x7F,0x08,0x14,0x22,0x41], // K
    [0x7F,0x40,0x40,0x40,0x40], // L
    [0x7F,0x02,0x0C,0x02,0x7F], // M
    [0x7F,0x04,0x08,0x10,0x7F], // N
    [0x3E,0x41,0x41,0x41,0x3E], // O
    [0x7F,0x09,0x09,0x09,0x06], // P
    [0x3E,0x41,0x51,0x21,0x5E], // Q
    [0x7F,0x09,0x19,0x29,0x46], // R
    [0x46,0x49,0x49,0x49,0x31], // S
    [0x01,0x01,0x7F,0x01,0x01], // T
    [0x3F,0x40,0x40,0x40,0x3F], // U
    [0x1F,0x20,0x40,0x20,0x1F], // V
    [0x3F,0x40,0x38,0x40,0x3F], // W
    [0x63,0x14,0x08,0x14,0x63], // X
    [0x07,0x08,0x70,0x08,0x07], // Y
    [0x61,0x51,0x49,0x45,0x43], // Z
    [0x00,0x7F,0x41,0x41,0x00], // [
    [0x02,0x04,0x08,0x10,0x20], // backslash
    [0x00,0x41,0x41,0x7F,0x00], // ]
    [0x04,0x02,0x01,0x02,0x04], // ^
    [0x40,0x40,0x40,0x40,0x40], // _
];

/// One column of a glyph, bit 0 = top row. Shared with the TFT text renderer
/// so there is only ever one font in flash.
pub fn glyph_col(ch: u8, col: usize) -> u8 {
    if col >= 5 {
        return 0;
    }
    let mut c = ch;
    if c.is_ascii_lowercase() {
        c -= 32;
    }
    if !(0x20..0x60).contains(&c) {
        c = b'?';
    }
    FONT[(c - 0x20) as usize][col]
}

/// Glyph width in pixels, excluding the inter-character gap.
pub const GLYPH_W: usize = 5;
/// Glyph height in pixels.
pub const GLYPH_H: usize = 7;

#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    /// Nothing driven from here; LED ON/OFF/HEART own the panel.
    Off,
    /// Message held still, showing the first 12 columns.
    Still,
    Left,
    Right,
    Up,
    Down,
}

pub struct Marquee {
    text: [u8; TEXT_MAX],
    len: usize,
    mode: Mode,
    /// Scroll offset: columns for Left/Right, rows for Up/Down.
    pos: i32,
    interval_ms: u32,
    next_ms: u64,
}

impl Default for Marquee {
    fn default() -> Self {
        Self::new()
    }
}

impl Marquee {
    pub const fn new() -> Self {
        Self {
            text: [b' '; TEXT_MAX],
            len: 0,
            mode: Mode::Off,
            pos: 0,
            interval_ms: 90,
            next_ms: 0,
        }
    }

    pub fn is_active(&self) -> bool {
        self.mode != Mode::Off
    }

    pub fn stop(&mut self) {
        self.mode = Mode::Off;
    }

    pub fn set_speed(&mut self, ms: u32) {
        self.interval_ms = ms.clamp(10, 2000);
    }

    pub fn speed(&self) -> u32 {
        self.interval_ms
    }

    /// Load a message and start it in `mode`. Returns the number of characters
    /// actually stored, which is capped at [`TEXT_MAX`].
    pub fn show(&mut self, s: &str, mode: Mode) -> usize {
        self.len = 0;
        for b in s.bytes() {
            if self.len == TEXT_MAX {
                break;
            }
            self.text[self.len] = b;
            self.len += 1;
        }
        self.mode = mode;
        self.pos = 0;
        self.next_ms = 0; // draw immediately
        self.len
    }

    /// Total columns in the scroll strip, including the trailing gap.
    fn strip_cols(&self) -> i32 {
        (self.len * ADVANCE + GAP) as i32
    }

    /// One column slice of the strip. Bit 0 is the top row.
    fn column(&self, i: i32) -> u8 {
        if i < 0 {
            return 0;
        }
        let i = i as usize;
        let ch_idx = i / ADVANCE;
        if ch_idx >= self.len {
            return 0; // inside the trailing gap
        }
        let within = i % ADVANCE;
        if within == 5 {
            return 0; // inter-character spacing
        }
        let mut c = self.text[ch_idx];
        if c.is_ascii_lowercase() {
            c -= 32; // fold to uppercase
        }
        if !(0x20..0x60).contains(&c) {
            c = b'?';
        }
        FONT[(c - 0x20) as usize][within]
    }

    /// Advance the animation if enough time has passed, and push a frame.
    pub fn tick<P: Platform + ?Sized>(&mut self, p: &mut P) {
        if self.mode == Mode::Off || !p.has_led_matrix() {
            return;
        }
        let now = p.uptime_ms();
        if self.mode != Mode::Still && now < self.next_ms {
            return;
        }
        self.next_ms = now + self.interval_ms as u64;

        let frame = self.render();
        p.led_matrix(&frame);

        match self.mode {
            Mode::Left => {
                self.pos += 1;
                if self.pos >= self.strip_cols() {
                    self.pos = 0;
                }
            }
            Mode::Right => {
                self.pos -= 1;
                if self.pos < 0 {
                    self.pos = self.strip_cols() - 1;
                }
            }
            Mode::Up => self.pos = (self.pos + 1) % ROWS as i32,
            Mode::Down => self.pos = (self.pos + ROWS as i32 - 1) % ROWS as i32,
            // Still redraws the same frame; harmless and keeps the panel lit
            // if something else overwrote it.
            Mode::Still | Mode::Off => {}
        }
    }

    /// Build the 8x12 frame for the current position.
    fn render(&self) -> [u16; 8] {
        let mut frame = [0u16; ROWS];
        let strip = self.strip_cols();
        let vertical = matches!(self.mode, Mode::Up | Mode::Down);

        for c in 0..COLS {
            // Horizontal modes slide the window; vertical modes hold it at 0.
            let src = if vertical {
                c as i32
            } else {
                let mut s = self.pos + c as i32;
                if strip > 0 {
                    s %= strip;
                }
                s
            };
            let bits = self.column(src);

            for r in 0..ROWS {
                // For vertical scrolling, read the row the glyph has moved to.
                let src_row = if vertical {
                    (r as i32 + self.pos).rem_euclid(ROWS as i32) as usize
                } else {
                    r
                };
                if src_row < 7 && bits & (1 << src_row) != 0 {
                    frame[r] |= 1 << (11 - c);
                }
            }
        }
        frame
    }
}
