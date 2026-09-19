//! A cell-diffing screen: compose a whole 80x25 frame, send only what moved.
//!
//! # Why this exists
//!
//! Everything else in this kernel streams escape codes straight to the UART,
//! and `vt` says so in its own header — an 80x25 buffer costs RAM, and for a
//! boot screen drawn once, emitting as you go gives the same result for free.
//!
//! A game is not drawn once. `SETUP` already had to learn this: repainting the
//! whole screen on every keypress made navigation tear visibly from the top
//! down, because ~1.8 KB at 115200 baud is ~160 ms and the terminal paints
//! bytes as they arrive. `SETUP` fixed it by hand, rewriting the two rows it
//! knew had changed. That works for three menu items and does not survive
//! contact with a scene that has prose, a choice list, a status bar and a
//! marker that appears on some choices and not others.
//!
//! So: the caller draws the frame it wants, in full, into a buffer. This
//! compares that against what the terminal is already showing and sends the
//! difference.
//!
//! # The two lessons it is built on
//!
//! **Chris Sawyer**: blit only the dirty region. **John Carmack**: never draw
//! what the viewer cannot tell apart from what is already there. Both are the
//! same instruction — *find the binding constraint, then arrange never to pay
//! for what did not change*. Here the constraint is 11 520 bytes per second,
//! and the numbers it produces are stark: a full repaint is ~2.2 KB and ~190
//! ms; moving the choice highlight is ~100 bytes and under 10 ms.
//!
//! # Two buffers, 4 KB
//!
//! `front` is what the terminal shows, `back` is what it should show. A cell is
//! **two bytes** — one glyph code, one attribute — because a `char` is four and
//! 80x25 of those would be 8 KB before attributes. ASCII is stored directly and
//! the handful of box-drawing glyphs are indices into [`GLYPHS`], which works
//! precisely because the shipped Romanian is written without diacritics. That
//! was a decision about serial terminals; it pays a second time here.

use crate::i18n::Chars;
use crate::{kprint, vt, Platform};

pub const COLS: usize = 80;
pub const ROWS: usize = 25;
const CELLS: usize = COLS * ROWS;

/// Glyphs outside ASCII, addressed as `0x80 + index`.
static GLYPHS: [&str; 16] = [
    "\u{2554}", // ╔
    "\u{2557}", // ╗
    "\u{255a}", // ╚
    "\u{255d}", // ╝
    "\u{2550}", // ═
    "\u{2551}", // ║
    "\u{2500}", // ─
    "\u{2502}", // │
    // A single-line rule meeting a double border needs the mixed-weight tees.
    // `╠`/`╣` are double-into-double and leave a visible step where the rule
    // joins, which looked like a rendering bug rather than a choice.
    "\u{255f}", // ╟
    "\u{2562}", // ╢
    "\u{2588}", // █
    "\u{2591}", // ░
    "\u{2020}", // †
    "\u{00b7}", // ·
    "\u{25ba}", // ►
    "\u{2665}", // ♥
];

pub const TL: u8 = 0x80;
pub const TR: u8 = 0x81;
pub const BL: u8 = 0x82;
pub const BR: u8 = 0x83;
pub const H: u8 = 0x84;
pub const V: u8 = 0x85;
pub const LH: u8 = 0x86;
pub const LV: u8 = 0x87;
pub const TEE_L: u8 = 0x88;
pub const TEE_R: u8 = 0x89;
pub const BLOCK: u8 = 0x8a;
pub const SHADE: u8 = 0x8b;
pub const DAGGER: u8 = 0x8c;
pub const ARROW: u8 = 0x8e;
pub const HEART: u8 = 0x8f;

/// How a cell is coloured. Deliberately few: every extra attribute is another
/// SGR sequence the diff may have to emit mid-run.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Attr {
    Normal = 0,
    /// Structure -- frames, rules. Present but not competing with the prose.
    Frame = 1,
    /// Emphasis: titles, the selected choice.
    Bright = 2,
    /// Something the player should notice: the oath, the marker.
    Accent = 3,
    /// Something going wrong: a broken oath, low health.
    Warn = 4,
}

/// One character on screen. Two bytes, and the layout is the point.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Cell {
    ch: u8,
    attr: u8,
}

const BLANK: Cell = Cell {
    ch: b' ',
    attr: Attr::Normal as u8,
};

/// A cell that cannot occur, so the first flush is a guaranteed full paint.
const UNKNOWN: Cell = Cell { ch: 0, attr: 0xff };

/// Unchanged cells tolerated inside one run before it is cheaper to stop and
/// re-address the cursor.
///
/// A cursor address is `ESC [ row ; col H` — eight bytes for a two-digit row
/// and column. So rewriting up to eight identical cells costs no more than
/// jumping over them, and rewriting them keeps the run going, which usually
/// saves the *next* address too.
const GAP: usize = 8;

/// Where [`Screen::wrap`] keeps its place.
///
/// Held apart from the screen so both can be borrowed at once while the text
/// streams past: the characters arrive one at a time and the word they belong
/// to is not known to be finished until a space, a newline or the end of the
/// message says so.
struct Wrap {
    x: usize,
    y: usize,
    w: usize,
    max_rows: usize,
    attr: Attr,
    row: usize,
    col: usize,
    /// The word being gathered. A box is never wider than the screen, so a
    /// word too long for this buffer is too long for any box and gets split --
    /// which is what happens to it anyway.
    word: [u8; COLS],
    n: usize,
    /// This word is the tail of one already part-drawn, so it takes no leading
    /// space and gets no second chance to fit.
    mid: bool,
    done: bool,
}

impl Wrap {
    fn push(&mut self, s: &mut Screen, b: u8) {
        if self.done {
            return;
        }
        match b {
            b'\n' => {
                self.flush(s);
                if !self.done {
                    self.row += 1;
                    self.col = 0;
                    self.mid = false;
                    self.done = self.row >= self.max_rows;
                }
            }
            b' ' => self.flush(s),
            _ => {
                self.word[self.n] = b;
                self.n += 1;
                if self.n == self.word.len() {
                    self.flush(s);
                    self.mid = true;
                }
            }
        }
    }

    /// Draw the gathered word, breaking the line before it if it will not fit.
    fn flush(&mut self, s: &mut Screen) {
        let cont = self.mid;
        self.mid = false;
        if self.done || self.n == 0 {
            self.n = 0;
            return;
        }
        // Break before a word that will not fit -- unless it would not fit on
        // a line of its own either, in which case it is split.
        if !cont && self.col > 0 && self.col + 1 + self.n > self.w {
            self.row += 1;
            self.col = 0;
            if self.row >= self.max_rows {
                self.done = true;
                self.n = 0;
                return;
            }
        }
        if !cont && self.col > 0 {
            s.put(self.x + self.col, self.y + self.row, b' ', self.attr);
            self.col += 1;
        }
        for i in 0..self.n {
            if self.col >= self.w {
                self.row += 1;
                self.col = 0;
                if self.row >= self.max_rows {
                    self.done = true;
                    self.n = 0;
                    return;
                }
            }
            s.put(
                self.x + self.col,
                self.y + self.row,
                code(self.word[i] as char),
                self.attr,
            );
            self.col += 1;
        }
        self.n = 0;
    }
}

/// Map a character to a cell code. Anything unrepresentable becomes `?` rather
/// than silently vanishing.
pub fn code(c: char) -> u8 {
    if (' '..='~').contains(&c) {
        return c as u8;
    }
    let mut i = 0;
    while i < GLYPHS.len() {
        if GLYPHS[i].chars().next() == Some(c) {
            return 0x80 + i as u8;
        }
        i += 1;
    }
    b'?'
}

fn glyph(ch: u8) -> &'static str {
    if ch < 0x80 {
        // Callers only ever store printable ASCII here.
        return match ch {
            0x20..=0x7e => ASCII[(ch - 0x20) as usize],
            _ => " ",
        };
    }
    GLYPHS
        .get((ch - 0x80) as usize)
        .copied()
        .unwrap_or("?")
}

/// One-character strings for printable ASCII, so a cell can be emitted with the
/// same `&str` path as a multi-byte glyph and the byte accounting stays in one
/// place.
static ASCII: [&str; 95] = [
    " ", "!", "\"", "#", "$", "%", "&", "'", "(", ")", "*", "+", ",", "-", ".", "/", "0", "1", "2",
    "3", "4", "5", "6", "7", "8", "9", ":", ";", "<", "=", ">", "?", "@", "A", "B", "C", "D", "E",
    "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R", "S", "T", "U", "V", "W", "X",
    "Y", "Z", "[", "\\", "]", "^", "_", "`", "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k",
    "l", "m", "n", "o", "p", "q", "r", "s", "t", "u", "v", "w", "x", "y", "z", "{", "|", "}", "~",
];

/// The colour for an attribute, as up to two strings.
///
/// Two rather than one because the background belongs to the theme and the
/// foreground to the attribute, and there is no allocator to join them. Under
/// `THEME MONO` the second is empty and the whole thing collapses to a reset,
/// which is exactly what mono should mean.
fn attr_sgr(a: u8) -> (&'static str, &'static str) {
    let fg = match a {
        x if x == Attr::Frame as u8 => vt::sgr(vt::FG_CYAN),
        x if x == Attr::Bright as u8 => vt::sgr(vt::FG_BRIGHT),
        x if x == Attr::Accent as u8 => vt::sgr(vt::FG_YELLOW),
        x if x == Attr::Warn as u8 => vt::sgr(vt::FG_RED),
        _ => "",
    };
    (vt::scheme(), fg)
}

fn digits(mut n: usize) -> usize {
    let mut d = 1;
    while n >= 10 {
        n /= 10;
        d += 1;
    }
    d
}

/// A composed frame, plus what the terminal is currently showing.
pub struct Screen {
    front: [Cell; CELLS],
    back: [Cell; CELLS],
    /// Cells that genuinely changed in the last flush.
    pub last_cells: usize,
    /// Bytes the last flush put on the wire.
    pub last_bytes: usize,
    pub frames: usize,
    pub total_bytes: usize,
}

impl Default for Screen {
    fn default() -> Self {
        Self::new()
    }
}

impl Screen {
    pub const fn new() -> Self {
        Self {
            front: [UNKNOWN; CELLS],
            back: [BLANK; CELLS],
            last_cells: 0,
            last_bytes: 0,
            frames: 0,
            total_bytes: 0,
        }
    }

    /// Blank the frame being composed. The front buffer is untouched, so a
    /// clear followed by a redraw of the same content sends nothing.
    pub fn clear(&mut self) {
        self.back = [BLANK; CELLS];
    }

    /// Forget what the terminal is showing, so the next flush repaints
    /// everything. Needed after anything else has written to the screen.
    pub fn invalidate(&mut self) {
        self.front = [UNKNOWN; CELLS];
    }

    pub fn put(&mut self, x: usize, y: usize, ch: u8, attr: Attr) {
        if x < COLS && y < ROWS {
            self.back[y * COLS + x] = Cell {
                ch,
                attr: attr as u8,
            };
        }
    }

    /// Write text, clipped at the right edge. Returns the columns used.
    pub fn text(&mut self, x: usize, y: usize, s: impl Chars, attr: Attr) -> usize {
        self.write(x, y, &s, attr)
    }

    fn write(&mut self, x: usize, y: usize, s: &dyn Chars, attr: Attr) -> usize {
        let mut n = 0;
        s.each(&mut |b| {
            if x + n < COLS {
                self.put(x + n, y, code(b as char), attr);
                n += 1;
            }
        });
        n
    }

    pub fn fill(&mut self, x: usize, y: usize, w: usize, h: usize, ch: u8, attr: Attr) {
        for row in y..(y + h) {
            for col in x..(x + w) {
                self.put(col, row, ch, attr);
            }
        }
    }

    /// Word-wrap `text` into a box, treating `\n` as a hard break. Returns the
    /// number of rows used, so a caller can place what follows.
    ///
    /// The text arrives a character at a time and cannot be rewound, so words
    /// are gathered in [`Wrap`] until something ends them. That is the only
    /// lookahead wrapping needs: whether the word about to be drawn fits on
    /// the line it is standing on.
    pub fn wrap(
        &mut self,
        x: usize,
        y: usize,
        w: usize,
        max_rows: usize,
        text: impl Chars,
        attr: Attr,
    ) -> usize {
        self.wrap_dyn(x, y, w, max_rows, &text, attr)
    }

    #[allow(clippy::too_many_arguments)]
    fn wrap_dyn(
        &mut self,
        x: usize,
        y: usize,
        w: usize,
        max_rows: usize,
        text: &dyn Chars,
        attr: Attr,
    ) -> usize {
        let mut st = Wrap {
            x,
            y,
            w,
            max_rows,
            attr,
            row: 0,
            col: 0,
            word: [0; COLS],
            n: 0,
            mid: false,
            done: false,
        };
        text.each(&mut |b| st.push(self, b));
        st.flush(self);
        if !st.done {
            // The last line has no newline to close it, but it still counts.
            st.row += 1;
        }
        st.row
    }

    /// A double-line border around the whole screen.
    pub fn border(&mut self, attr: Attr) {
        self.put(0, 0, TL, attr);
        self.put(COLS - 1, 0, TR, attr);
        self.put(0, ROWS - 1, BL, attr);
        self.put(COLS - 1, ROWS - 1, BR, attr);
        for x in 1..(COLS - 1) {
            self.put(x, 0, H, attr);
            self.put(x, ROWS - 1, H, attr);
        }
        for y in 1..(ROWS - 1) {
            self.put(0, y, V, attr);
            self.put(COLS - 1, y, V, attr);
        }
    }

    /// A horizontal rule across the frame, with tees where it meets the border.
    pub fn rule(&mut self, y: usize, attr: Attr) {
        self.put(0, y, TEE_L, attr);
        self.put(COLS - 1, y, TEE_R, attr);
        for x in 1..(COLS - 1) {
            self.put(x, y, LH, attr);
        }
    }

    /// Send the difference between `back` and `front`.
    ///
    /// Returns `(cells changed, bytes sent)`. The byte figure is counted as it
    /// is emitted rather than estimated, because a renderer whose own cost
    /// report is a guess proves nothing — the same rule `PINS WATCH` follows.
    pub fn flush(&mut self, p: &mut dyn Platform) -> (usize, usize) {
        let mut cells = 0usize;
        let mut bytes = 0usize;
        // 0xff cannot be a real attribute, so the first run always emits one.
        let mut cur_attr = 0xffu8;

        for row in 0..ROWS {
            let base = row * COLS;
            let mut col = 0usize;
            while col < COLS {
                if self.back[base + col] == self.front[base + col] {
                    col += 1;
                    continue;
                }

                // Extend the run while changes keep coming, tolerating short
                // stretches of unchanged cells.
                let start = col;
                let mut end = col + 1;
                let mut gap = 0usize;
                let mut scan = col + 1;
                while scan < COLS {
                    if self.back[base + scan] != self.front[base + scan] {
                        end = scan + 1;
                        gap = 0;
                    } else {
                        gap += 1;
                        if gap > GAP {
                            break;
                        }
                    }
                    scan += 1;
                }

                kprint!(p, "\x1b[{};{}H", row + 1, start + 1);
                bytes += 4 + digits(row + 1) + digits(start + 1);

                for i in start..end {
                    let c = self.back[base + i];
                    if c.attr != cur_attr {
                        let (bg, fg) = attr_sgr(c.attr);
                        kprint!(p, "{}{}", bg, fg);
                        bytes += bg.len() + fg.len();
                        cur_attr = c.attr;
                    }
                    let g = glyph(c.ch);
                    kprint!(p, "{}", g);
                    bytes += g.len();
                    if self.front[base + i] != c {
                        cells += 1;
                    }
                    self.front[base + i] = c;
                }
                col = end;
            }
        }

        if bytes > 0 {
            kprint!(p, "{}", vt::NORMAL);
            bytes += vt::NORMAL.len();
        }

        self.last_cells = cells;
        self.last_bytes = bytes;
        self.frames += 1;
        self.total_bytes += bytes;
        (cells, bytes)
    }

    /// What a naive full repaint of the composed frame would have cost.
    ///
    /// Counted the same way the real flush counts — cursor addressing and SGR
    /// included — because charging the differential path for addressing while
    /// letting the full redraw have it free would flatter the comparison. That
    /// mistake was available in `PINS WATCH` and avoided there too.
    pub fn full_cost(&self) -> usize {
        let mut bytes = 0usize;
        let mut cur_attr = 0xffu8;
        for row in 0..ROWS {
            bytes += 4 + digits(row + 1) + 1;
            for col in 0..COLS {
                let c = self.back[row * COLS + col];
                if c.attr != cur_attr {
                    let (bg, fg) = attr_sgr(c.attr);
                    bytes += bg.len() + fg.len();
                    cur_attr = c.attr;
                }
                bytes += glyph(c.ch).len();
            }
        }
        bytes
    }
}

// ---------------------------------------------------------------- viewport

/// A window onto a world bigger than the screen.
///
/// # Why a viewport needs a scale and not just a scroll offset
///
/// Everything else here draws in cells, because everything else fits. A map
/// does not: Vadul Alb's streets are four to twelve minutes long and the road
/// to the bridge is a hundred and ten, so one picture cannot be both. Scrolling
/// alone would let you see the far end of the bridge road at village
/// magnification, which is forty minutes of empty field per screen and no way
/// to tell where you are going.
///
/// So the window has a **scale**, and the caller draws in world units and lets
/// this decide where — and whether — they land.
///
/// # A row is worth two columns
///
/// A terminal cell is about twice as tall as it is wide. A map that spends the
/// same world distance on a row as on a column comes out squashed to half
/// height, and every angle in it is a lie. So [`View::per`] is the world
/// distance across a *column* and a row is worth twice it. `combat` reached the
/// same conclusion from the other direction and draws its grid two columns to
/// the square.
///
/// # Level of detail is a consequence, not a setting
///
/// [`View::cells`] is the whole mechanism: it answers how wide a thing is in
/// the only unit that matters, which is the one labels are measured in. A
/// village narrower than the word "village" cannot be drawn as nine places with
/// nine names, and the honest response is to draw fewer things rather than to
/// overlap them. What the tiers are is the caller's business — this only
/// supplies the measurement they turn on.
#[derive(Clone, Copy)]
pub struct View {
    /// Top-left cell of the window.
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
    /// The world coordinate sitting at the middle of the window.
    pub cx: i32,
    pub cy: i32,
    /// World units to a cell column. A row is worth `2 * per`; see above.
    pub per: i32,
}

impl View {
    /// Where a world point lands in cells, whether or not that is on screen.
    ///
    /// `div_euclid` rather than `/`: plain division truncates towards zero, so
    /// the cell straddling the centre would be twice the width of every other
    /// one and the map would sit half a cell off to one side of it.
    pub fn cell_of(&self, w: (i32, i32)) -> (i32, i32) {
        (
            (w.0 - self.cx).div_euclid(self.per) + (self.w / 2) as i32,
            (w.1 - self.cy).div_euclid(self.per * 2) + (self.h / 2) as i32,
        )
    }

    /// Where a world point lands, or `None` if it is outside the window.
    pub fn at(&self, w: (i32, i32)) -> Option<(usize, usize)> {
        let (col, row) = self.cell_of(w);
        if col < 0 || row < 0 || col >= self.w as i32 || row >= self.h as i32 {
            return None;
        }
        Some((self.x + col as usize, self.y + row as usize))
    }

    /// How many cell columns a world distance covers.
    ///
    /// Every level-of-detail decision starts here.
    pub fn cells(&self, world: i32) -> usize {
        (world.abs() / self.per) as usize
    }

    /// The world distance across the whole window, and down it.
    pub fn span(&self) -> (i32, i32) {
        (self.w as i32 * self.per, self.h as i32 * self.per * 2)
    }

    fn plot(&self, s: &mut Screen, col: i32, row: i32, ch: u8, attr: Attr) {
        if col >= 0 && row >= 0 && col < self.w as i32 && row < self.h as i32 {
            s.put(self.x + col as usize, self.y + row as usize, ch, attr);
        }
    }

    /// One character at a world point, if it is in the window.
    pub fn mark(&self, s: &mut Screen, w: (i32, i32), ch: u8, attr: Attr) {
        let (col, row) = self.cell_of(w);
        self.plot(s, col, row, ch, attr);
    }

    /// A straight line between two world points, clipped to the window.
    ///
    /// Bresenham over cell coordinates, plotting only what is inside. Walking
    /// the off-screen part rather than clipping the endpoints first is the
    /// cheaper mistake: the longest road in the world is a hundred and ten
    /// minutes, which at the sharpest zoom is a few hundred steps of integer
    /// arithmetic, and a clipper is a page of code that has to be right at
    /// every corner.
    pub fn line(&self, s: &mut Screen, a: (i32, i32), b: (i32, i32), ch: u8, attr: Attr) {
        let (x0, y0) = self.cell_of(a);
        let (x1, y1) = self.cell_of(b);
        let dx = (x1 - x0).abs();
        let dy = -(y1 - y0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        let (mut x, mut y) = (x0, y0);
        loop {
            self.plot(s, x, y, ch, attr);
            if x == x1 && y == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x += sx;
            }
            if e2 <= dx {
                err += dx;
                y += sy;
            }
        }
    }

    /// Which character to draw a line in, given where it is going.
    ///
    /// A map of dots reads as a scatter of dots. Choosing the rule the line
    /// mostly follows -- and allowing for a row being worth two columns before
    /// deciding -- makes a street look like a street.
    pub fn stroke(&self, a: (i32, i32), b: (i32, i32)) -> u8 {
        let dx = (b.0 - a.0).abs();
        let dy = (b.1 - a.1).abs() / 2;
        if dy * 3 < dx {
            LH
        } else if dx * 3 < dy {
            LV
        } else if (b.0 - a.0) * (b.1 - a.1) > 0 {
            b'\\'
        } else {
            b'/'
        }
    }

    /// Text near a world point, clipped to the window rather than the screen.
    ///
    /// `off` is cells from the point itself, so a label can clear the marker it
    /// belongs to -- or sit on the row above it when both sides of its own row
    /// are already spoken for. Returns the columns actually drawn, which is
    /// zero when there is nowhere for them to go.
    pub fn label(
        &self,
        s: &mut Screen,
        w: (i32, i32),
        off: (i32, i32),
        text: impl Chars,
        attr: Attr,
    ) -> usize {
        let (col, cell_row) = self.cell_of(w);
        let row = cell_row + off.1;
        if row < 0 || row >= self.h as i32 {
            return 0;
        }
        let mut at = col + off.0;
        let mut n = 0;
        text.each(&mut |b| {
            if at >= 0 && at < self.w as i32 {
                s.put(
                    self.x + at as usize,
                    self.y + row as usize,
                    code(b as char),
                    attr,
                );
                n += 1;
            }
            at += 1;
        });
        n
    }
}
