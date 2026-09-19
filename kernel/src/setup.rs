//! The BIOS setup utility, reached with `DEL` or the `SETUP` command.
//!
//! The POST screen has offered "Press <DEL> to enter SETUP" since v0.1 while
//! nothing was behind it. This is that screen.
//!
//! Settings are held in the ESP32-S3's NVS, so `F10` persists them across a
//! power cycle. Changes take effect as you make them, exactly like the
//! firmware this imitates.
//!
//! # Differential rendering
//!
//! The whole screen is ~1.8 KB of text and escape codes. At 115200 baud that
//! is ~160 ms on the wire, and the terminal paints it as it arrives — so
//! redrawing everything on each keypress makes navigation visibly tear from
//! the top down. The PC draws instantly; it simply cannot draw bytes that have
//! not turned up yet.
//!
//! So the full screen is drawn once, and afterwards only the lines that
//! actually changed are rewritten: moving the selection touches two rows,
//! about 90 bytes, under a millisecond. Changing the language is the one case
//! that needs a full repaint, because every label is translated.

use crate::i18n::{self, Chars, Msg, Text};
use crate::marquee::Marquee;
use crate::{kprint, kprintln, romfs, vt, Platform};

/// NVS key for the marquee step interval, stored as milliseconds / 10 so it
/// fits the byte-sized settings store.
pub const SPEED_KEY: &str = "speed";

const ITEMS: usize = 3;
const W: usize = 78;

/// 1-based screen row of the first editable item. Everything above it (box,
/// title, blank) is fixed height, so the rows below are predictable too.
const ITEM_ROW: usize = 5;
/// The block below the items is fixed height as well -- blank, heading, six
/// information rows, blank, rule -- so the key legend lands here whatever
/// `ITEMS` is, and the acknowledgement two rows below that.
const KEYS_ROW: usize = ITEM_ROW + ITEMS + 11;
/// Row used for the "saved" acknowledgement.
const STATUS_ROW: usize = KEYS_ROW + 2;

/// A decoded keypress. Enough of a keyboard for a menu.
///
/// Shared with `esp`, which needs the same arrow-and-escape handling and has
/// no business reimplementing the timing rules that took two goes to get right.
pub(crate) enum Key {
    Up,
    Down,
    Left,
    Right,
    Enter,
    Esc,
    Save,
    /// `1` to `9`, held as the digit itself. A menu of a few items should be
    /// answerable by typing its number -- that is what DOS menus did, and it is
    /// the fallback wherever a terminal mangles the arrow keys.
    Digit(u8),
    /// Any other letter, folded to upper case. `WASD` are already arrows, `Q`
    /// is escape and `S` saves, so what reaches here is the rest of the
    /// alphabet -- which is where a screen with more verbs than a menu can put
    /// them.
    Char(u8),
    /// `+` and `-`, for a screen with a scale. Every map ever drawn has used
    /// these two, and `=` and `_` come with them because they are the
    /// unshifted keys in the same place and are what gets pressed.
    Plus,
    Minus,
    Other,
}

/// Read one key, decoding CSI escape sequences.
pub(crate) fn read_key(p: &mut dyn Platform) -> Key {
    let b = loop {
        if let Some(b) = p.get() {
            break b;
        }
    };

    if b != 0x1b {
        return match b {
            b'\r' | b'\n' => Key::Enter,
            b'q' | b'Q' | 0x03 => Key::Esc,
            // F10 is `ESC [ 2 1 ~`, awkward on some terminals; S saves too.
            b's' | b'S' => Key::Save,
            b'w' | b'W' => Key::Up,
            b'z' | b'Z' => Key::Down,
            b'a' | b'A' => Key::Left,
            b'd' | b'D' => Key::Right,
            b'1'..=b'9' => Key::Digit(b - b'0'),
            b'+' | b'=' => Key::Plus,
            b'-' | b'_' => Key::Minus,
            b'a'..=b'z' | b'A'..=b'Z' => Key::Char(b.to_ascii_uppercase()),
            _ => Key::Other,
        };
    }

    // Possible escape sequence: wait for a following byte.
    //
    // The window has to exceed however long the host takes to deliver the rest
    // of the sequence, which is not the ~87 us the baud rate suggests: a
    // terminal that writes and flushes per byte turns each one into its own
    // USB transaction, costing milliseconds. 250 ms is comfortably longer than
    // that and comfortably shorter than a human pressing ESC then another key,
    // which is the only thing this has to tell apart.
    let deadline = p.uptime_ms() + 250;
    let mut next = None;
    while p.uptime_ms() < deadline {
        if let Some(b) = p.get() {
            next = Some(b);
            break;
        }
    }
    let Some(b2) = next else { return Key::Esc };
    if b2 != b'[' {
        return Key::Other;
    }

    // Consume parameters, then act on the final byte.
    let mut param = 0u8;
    loop {
        let Some(b) = p.get() else { continue };
        if (0x20..=0x3f).contains(&b) {
            if param == 0 && b.is_ascii_digit() {
                param = b - b'0';
            }
            continue;
        }
        return match b {
            b'A' => Key::Up,
            b'B' => Key::Down,
            b'C' => Key::Right,
            b'D' => Key::Left,
            // F10 arrives as `ESC [ 2 1 ~`; the first digit is enough here.
            b'~' if param == 2 => Key::Save,
            _ => Key::Other,
        };
    }
}

/// Rewrite one item's row in place. ~90 bytes rather than a full screen.
fn draw_item(p: &mut dyn Platform, m: &Marquee, i: usize, sel: usize) {
    vt::at(p, ITEM_ROW + i, 1);
    vt::clear_eol(p);
    kprint!(p, "{}", vt::scheme());

    kprint!(p, " {} ", if i == sel { ">" } else { " " });
    if i == sel {
        kprint!(p, "{}", vt::sgr(vt::FG_YELLOW));
    }
    match i {
        0 => {
            vt::dotfill(p, i18n::t(Msg::SetupLanguage), 34);
            kprint!(p, "  {}", i18n::lang_name());
        }
        1 => {
            vt::dotfill(p, i18n::t(Msg::SetupSpeed), 34);
            kprint!(p, "  {} ms", m.speed());
        }
        _ => {
            vt::dotfill(p, i18n::t(Msg::SetupTheme), 34);
            kprint!(p, "  {}", vt::theme_name());
        }
    }
    kprint!(p, "{}", vt::scheme());
}

fn draw_status(p: &mut dyn Platform, ok: Option<bool>) {
    vt::at(p, STATUS_ROW, 1);
    vt::clear_eol(p);
    kprint!(p, "{}", vt::scheme());
    match ok {
        Some(true) => i18n::put(p, Msg::MsgSaved),
        Some(false) => i18n::put(p, Msg::MsgNotsaved),
        None => {}
    }
}

/// Paint the whole screen. Needed on entry, and again whenever the language
/// changes because every label is translated.
fn draw_full(p: &mut dyn Platform, m: &Marquee, sel: usize) {
    kprint!(p, "{}", vt::scheme());
    vt::cls(p);
    kprint!(p, "{}", vt::sgr(vt::BOLD));
    vt::box_open(p, W);
    vt::box_row(p, W, i18n::t(Msg::SetupTitle));
    kprint!(p, "{}{}", vt::RESET, vt::scheme());
    vt::box_close(p, W);
    kprintln!(p);

    for i in 0..ITEMS {
        draw_item(p, m, i, sel);
        kprintln!(p);
    }

    // --- read-only information ---
    kprintln!(p);
    kprintln!(p, " {}", i18n::t(Msg::SetupInfo));
    let (cpu, mhz, ram, rom) = (p.cpu_name(), p.cpu_mhz(), p.ram_total(), p.rom_total());
    let matrix = p.has_led_matrix();
    let console = p.console_name();

    kprint!(p, "   ");
    vt::dotfill(p, i18n::t(Msg::PostCpu), 32);
    kprintln!(p, "  {} @ {} MHz", cpu, mhz);
    kprint!(p, "   ");
    vt::dotfill(p, i18n::t(Msg::PostConsole).trim(), 32);
    kprintln!(p, "  {}", console);
    kprint!(p, "   ");
    vt::dotfill(p, i18n::t(Msg::PostMemtest), 32);
    kprint!(p, "  {} ", ram);
    i18n::putln(p, Msg::MsgBytes);
    kprint!(p, "   ");
    vt::dotfill(p, i18n::t(Msg::PostFlash), 32);
    kprint!(p, "  {} ", rom);
    i18n::putln(p, Msg::MsgBytes);
    kprint!(p, "   ");
    vt::dotfill(p, i18n::t(Msg::PostMatrix).trim(), 32);
    kprintln!(
        p,
        "  {}",
        if matrix { Text::Plain("96 px") } else { i18n::t(Msg::PostNotfitted) }
    );
    kprint!(p, "   ");
    vt::dotfill(p, i18n::t(Msg::PostRomvol).trim(), 32);
    kprint!(p, "  {} ", romfs::total_bytes());
    i18n::putln(p, Msg::MsgBytes);
    kprint!(p, "   ");
    vt::dotfill(p, i18n::t(Msg::SetupResetcause), 32);
    let cause = crate::reset_cause_msg(p.reset_cause());
    kprintln!(p, "  {}", i18n::t(cause));

    kprintln!(p);
    vt::rule(p, W);
    i18n::putln(p, Msg::SetupKeys);
}

/// Run the setup screen. Returns when the user leaves.
pub fn run(p: &mut dyn Platform, m: &mut Marquee) {
    let mut sel = 0usize;
    // Scratch screen: the shell above is handed back exactly as it was.
    vt::alt_enter(p);
    vt::hide_cursor(p);
    draw_full(p, m, sel);

    loop {
        match read_key(p) {
            k @ (Key::Up | Key::Down) => {
                let prev = sel;
                sel = match k {
                    Key::Up => (sel + ITEMS - 1) % ITEMS,
                    _ => (sel + 1) % ITEMS,
                };
                // Only the two affected rows change.
                draw_item(p, m, prev, sel);
                draw_item(p, m, sel, sel);
                draw_status(p, None);
            }
            k @ (Key::Left | Key::Right) => {
                adjust(m, sel, matches!(k, Key::Right));
                // Language retranslates every label and theme recolours every
                // cell; both need the whole screen. Speed changes one row.
                if sel == 1 {
                    draw_item(p, m, sel, sel);
                } else {
                    draw_full(p, m, sel);
                }
                draw_status(p, None);
            }
            Key::Save => {
                let lang_ok = p.settings_set_u8(i18n::LANG_KEY, i18n::lang_index());
                let speed_ok = p.settings_set_u8(SPEED_KEY, (m.speed() / 10).clamp(1, 200) as u8);
                // Stored as index+1: a key that was never written reads back as
                // zero, and zero has to stay distinguishable from PLAIN.
                let theme_ok = p.settings_set_u8(vt::THEME_KEY, vt::theme_index() + 1);
                draw_status(p, Some(lang_ok && speed_ok && theme_ok));
            }
            Key::Esc => break,
            // Enter is `esp`'s "run this"; here there is nothing to run, and
            // people press it out of habit after changing a value.
            _ => {}
        }
    }

    vt::show_cursor(p);
    vt::alt_leave(p);
}

/// Change the selected setting. Effects apply immediately, as firmware setup
/// screens do; `F10` is what makes them survive a power cycle.
fn adjust(m: &mut Marquee, sel: usize, forward: bool) {
    match sel {
        0 => {
            let n = i18n::LANG_COUNT as u8;
            let cur = i18n::lang_index();
            let next = if forward {
                (cur + 1) % n
            } else {
                (cur + n - 1) % n
            };
            i18n::set_lang(next);
        }
        1 => {
            let step = 10i32;
            let cur = m.speed() as i32;
            let next = if forward { cur + step } else { cur - step };
            m.set_speed(next.clamp(10, 2000) as u32);
        }
        _ => {
            let n = vt::THEME_COUNT as u8;
            let cur = vt::theme_index();
            let next = if forward {
                (cur + 1) % n
            } else {
                (cur + n - 1) % n
            };
            vt::set_theme_index(next);
        }
    }
}
