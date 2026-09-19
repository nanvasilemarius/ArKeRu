//! VT100/ANSI output plus the box-drawing helpers that give the POST screen its
//! late-90s BIOS look.
//!
//! Everything streams straight to the UART -- there is no framebuffer. An 80x25
//! cell buffer with attributes would cost 4 KB of the RA4M1's 32 KB, and we get
//! the same result by emitting escape sequences as we go.

use crate::i18n::{self, Chars};
use crate::{kprint, Platform};

// ---------------------------------------------------------------- attributes

pub const RESET: &str = "\x1b[0m";
pub const BOLD: &str = "\x1b[1m";

pub const FG_BLACK: &str = "\x1b[30m";
pub const FG_RED: &str = "\x1b[31m";
pub const FG_GREEN: &str = "\x1b[32m";
pub const FG_YELLOW: &str = "\x1b[33m";
pub const FG_BLUE: &str = "\x1b[34m";
pub const FG_MAGENTA: &str = "\x1b[35m";
pub const FG_CYAN: &str = "\x1b[36m";
pub const FG_WHITE: &str = "\x1b[37m";
pub const FG_BRIGHT: &str = "\x1b[97m";

pub const BG_BLACK: &str = "\x1b[40m";
pub const BG_BLUE: &str = "\x1b[44m";
pub const BG_CYAN: &str = "\x1b[46m";
pub const BG_GREY: &str = "\x1b[100m";

/// The classic setup-screen palette: bright white on blue.
pub const SCHEME: &str = "\x1b[97;44m";

// ---------------------------------------------------------------- theming
//
// White-on-blue is what PC firmware looked like, and on the SPI TFT -- where
// the kernel owns every pixel of a 320x240 panel -- it is exactly right.
//
// A terminal window is not that. The kernel paints 80x25 of a window whose
// size and colour scheme belong to the user, so a blue field ends up as a
// rectangle floating inside somebody else's theme, with a hard edge wherever
// the paint stopped. The destination genuinely differs, so the colours do too:
// the TFT keeps its blue unconditionally (it has its own palette in `tft`),
// and the terminal defaults to PLAIN.

/// How the full-screen views colour themselves on the *terminal*.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    /// The terminal's own background, with colour used only for accents:
    /// `[ OK ]` tags, the selected row, game sprites. The default.
    Plain = 0,
    /// Bright white on blue, the way firmware looked.
    Bios = 1,
    /// Positioning but no colour, for terminals where it is unwanted.
    Mono = 2,
}

/// Settings key under which the theme is persisted.
pub const THEME_KEY: &str = "theme";

pub const THEME_COUNT: usize = 3;
static THEME_NAMES: [&str; THEME_COUNT] = ["PLAIN", "BIOS", "MONO"];

static CURRENT: core::sync::atomic::AtomicU8 =
    core::sync::atomic::AtomicU8::new(Theme::Plain as u8);

pub fn set_theme(t: Theme) {
    CURRENT.store(t as u8, core::sync::atomic::Ordering::Relaxed);
}

pub fn theme() -> Theme {
    match CURRENT.load(core::sync::atomic::Ordering::Relaxed) {
        1 => Theme::Bios,
        2 => Theme::Mono,
        _ => Theme::Plain,
    }
}

pub fn theme_index() -> u8 {
    theme() as u8
}

/// Select by index, ignoring values outside the range. Used when restoring
/// from the settings store, which cannot promise a sane byte.
pub fn set_theme_index(i: u8) {
    set_theme(match i {
        1 => Theme::Bios,
        2 => Theme::Mono,
        _ => Theme::Plain,
    });
}

pub fn theme_name() -> &'static str {
    THEME_NAMES[theme_index() as usize]
}

/// Resolve `"BIOS"` and friends, case-insensitively.
pub fn theme_by_name(name: &str) -> Option<u8> {
    THEME_NAMES
        .iter()
        .position(|n| {
            n.len() == name.len()
                && n.bytes()
                    .zip(name.bytes())
                    .all(|(a, b)| a.to_ascii_uppercase() == b.to_ascii_uppercase())
        })
        .map(|i| i as u8)
}

/// The background a full-screen view paints itself with.
pub fn scheme() -> &'static str {
    match theme() {
        Theme::Bios => SCHEME,
        // Not the empty string: returning to the terminal's own colours is the
        // whole point, and it must undo whatever was set before.
        _ => RESET,
    }
}

/// An SGR sequence, or nothing under `Mono`.
///
/// Everything that sets a colour goes through here, so switching to `Mono` is
/// one branch rather than a second code path per screen.
pub fn sgr(s: &'static str) -> &'static str {
    match theme() {
        Theme::Mono => "",
        _ => s,
    }
}

/// The shell's own baseline: the terminal's default colours.
///
/// Anything that paints a background of its own -- POST and SETUP both do --
/// must return here before handing the screen back, because a terminal keeps
/// attributes per *cell*. Leaving the blue scheme set means the next screen's
/// `cls` paints blue, while any text printed after a `RESET` lands on the
/// default background, and the result is a screen striped blue and black.
pub const NORMAL: &str = RESET;

// ---------------------------------------------------------------- box drawing

pub const TL: &str = "\u{2554}"; // ╔
pub const TR: &str = "\u{2557}"; // ╗
pub const BL: &str = "\u{255a}"; // ╚
pub const BR: &str = "\u{255d}"; // ╝
pub const H: &str = "\u{2550}"; // ═
pub const V: &str = "\u{2551}"; // ║

pub const LTL: &str = "\u{250c}"; // ┌
pub const LTR: &str = "\u{2510}"; // ┐
pub const LBL: &str = "\u{2514}"; // └
pub const LBR: &str = "\u{2518}"; // ┘
pub const LH: &str = "\u{2500}"; // ─
pub const LV: &str = "\u{2502}"; // │

pub const BLOCK: &str = "\u{2588}"; // █
pub const SHADE: &str = "\u{2591}"; // ░

/// Screen width the layout is designed for. Standard VGA text mode.
pub const COLS: usize = 80;

// ---------------------------------------------------------------- primitives

/// Clear the screen **and the scrollback**.
///
/// `ESC[2J` alone does not do what DOS's `CLS` did. On Windows Terminal and
/// conhost it scrolls the screen away rather than destroying it, so everything
/// stays one wheel-turn above and a "cleared" screen is really just a gap. Two
/// full-screen redraws in a row then read as two copies stacked vertically,
/// which is exactly what a repainting SETUP looked like.
///
/// `ESC[3J` erases the saved lines as well. Terminals that do not implement it
/// ignore it, so this costs three bytes and is never wrong.
pub fn cls(p: &mut dyn Platform) {
    kprint!(p, "\x1b[2J\x1b[3J\x1b[H");
}

/// Switch to the terminal's alternate screen buffer.
///
/// Full-screen views -- SETUP, the games, `PINS WATCH` -- draw on a scratch
/// screen and hand the real one back untouched on exit, the way `less` and
/// `vim` do. Without it, entering SETUP destroys whatever was in the shell
/// above it, and leaving cannot put it back.
///
/// Unsupported terminals ignore the sequence and behave as before.
pub fn alt_enter(p: &mut dyn Platform) {
    kprint!(p, "\x1b[?1049h");
}

/// Return to the normal screen, restoring what was on it.
pub fn alt_leave(p: &mut dyn Platform) {
    kprint!(p, "{}\x1b[?1049l", NORMAL);
}

/// Clear the screen back to the terminal's default colours.
///
/// `cls` alone paints with whatever attributes happen to be set, which is what
/// POST and SETUP want -- but every other screen wants a known starting state
/// rather than whatever the previous one left behind.
pub fn cls_normal(p: &mut dyn Platform) {
    kprint!(p, "{}", NORMAL);
    cls(p);
}

/// Erase from the cursor to the end of the screen.
///
/// Used to draw the line under POST: the BIOS screen above stays blue, and
/// everything below it is reset to the default background in one go, so the
/// shell does not inherit a half-painted backdrop.
pub fn clear_eos(p: &mut dyn Platform) {
    kprint!(p, "\x1b[J");
}

/// Move the cursor to a 1-based row/column.
pub fn at(p: &mut dyn Platform, row: usize, col: usize) {
    kprint!(p, "\x1b[{};{}H", row, col);
}

pub fn hide_cursor(p: &mut dyn Platform) {
    kprint!(p, "\x1b[?25l");
}

pub fn show_cursor(p: &mut dyn Platform) {
    kprint!(p, "\x1b[?25h");
}

/// Erase from the cursor to the end of the line.
pub fn clear_eol(p: &mut dyn Platform) {
    kprint!(p, "\x1b[K");
}

/// Confine scrolling to rows `top..=bottom`, 1-based and inclusive.
///
/// Text printed past the last row of the region scrolls only the region;
/// everything outside it stays where it is. This is how a status bar gets
/// pinned, and how a full-screen view stops the shell's teletype behaviour
/// from dragging the whole display upwards.
///
/// Setting a region also homes the cursor, and the region survives until it is
/// released -- so anything that sets one must call [`scroll_region_reset`]
/// before it gives the screen back.
pub fn scroll_region(p: &mut dyn Platform, top: usize, bottom: usize) {
    kprint!(p, "\x1b[{};{}r", top, bottom);
}

/// Release the scroll region: the whole screen scrolls again.
pub fn scroll_region_reset(p: &mut dyn Platform) {
    kprint!(p, "\x1b[r");
}

/// Repeat a string `n` times.
pub fn repeat(p: &mut dyn Platform, s: &str, n: usize) {
    for _ in 0..n {
        kprint!(p, "{}", s);
    }
}

// ---------------------------------------------------------------- composites

/// Draw a double-line box of `width` columns starting at the cursor's line.
/// `inner` is the number of blank content rows between the rules.
pub fn box_open(p: &mut dyn Platform, width: usize) {
    kprint!(p, "{}", TL);
    repeat(p, H, width.saturating_sub(2));
    kprint!(p, "{}\n", TR);
}

pub fn box_close(p: &mut dyn Platform, width: usize) {
    kprint!(p, "{}", BL);
    repeat(p, H, width.saturating_sub(2));
    kprint!(p, "{}\n", BR);
}

/// One content row of a double-line box, left-aligned and padded to `width`.
pub fn box_row(p: &mut dyn Platform, width: usize, text: impl Chars) {
    box_row_dyn(p, width, &text);
}

fn box_row_dyn(p: &mut dyn Platform, width: usize, text: &dyn Chars) {
    let inner = width.saturating_sub(4);
    let n = text.len().min(inner);
    kprint!(p, "{} ", V);
    let mut drawn = 0;
    text.each(&mut |b| {
        if drawn < inner {
            i18n::put_ch(p, b);
            drawn += 1;
        }
    });
    repeat(p, " ", inner - n);
    kprint!(p, " {}\n", V);
}

/// A single-line horizontal rule, used to separate POST sections.
pub fn rule(p: &mut dyn Platform, width: usize) {
    repeat(p, LH, width);
    kprint!(p, "\n");
}

/// Right-hand status tag such as `[ OK ]` or `[FAIL]`, coloured.
pub fn tag(p: &mut dyn Platform, ok: bool) {
    if ok {
        kprint!(p, "[ {}OK{} ]", sgr(FG_GREEN), scheme());
    } else {
        kprint!(p, "[{}FAIL{}]", sgr(FG_RED), scheme());
    }
}

/// Pad `text` with dots out to `to` columns -- the `Detecting Devices ....` look.
pub fn dotfill(p: &mut dyn Platform, text: impl Chars, to: usize) {
    dotfill_dyn(p, &text, to);
}

fn dotfill_dyn(p: &mut dyn Platform, text: &dyn Chars, to: usize) {
    let n = text.len();
    i18n::say(p, text);
    if n < to {
        kprint!(p, " ");
        repeat(p, ".", to - n - 1);
    }
}
