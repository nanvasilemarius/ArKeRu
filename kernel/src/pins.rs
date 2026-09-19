//! `PINS` — what every GPIO pin on the chip is currently doing.
//!
//! The board brings most of the microcontroller's pins out to through-hole
//! headers, and the mapping from a header label to a silicon pin is the sort
//! of thing you normally look up in a variant table and hope. This shows the
//! silicon directly: for each port, whether a pin belongs to GPIO or to a
//! peripheral, its direction, and the level actually present on it.
//!
//! # Why this only reads
//!
//! Reporting a pin's state cannot break anything. *Driving* one can: if
//! something on the header is already holding a pin low and the chip drives it
//! high, the only thing limiting the current is the two output stages.
//!
//! The single exception is an optional pull-up on **one pin the caller names**
//! -- `PINS WATCH 3.1` -- so that pin rests at 1 and grounding it reads 0.
//!
//! It is one pin, and named, because the blanket version broke the board. That
//! version pulled up everything that looked unclaimed: not a peripheral, not
//! an output, not in the platform's note table. The board went off USB and
//! needed a physical replug. "This firmware has not claimed it" is not "no
//! wire is attached to it" -- the note table describes what *this code*
//! configures and says nothing about what the *board* connects, and a 33 kohm
//! pull-up is quite enough to move a high-impedance control input that nothing
//! else is driving.
//!
//! # `PINS WATCH`
//!
//! Redraws the map continuously, rewriting only the cells that changed — a few
//! bytes per update rather than the ~700 the map costs. That makes it usable
//! for identifying a header pin *empirically*: touch a jumper from the pin to
//! GND and watch which cell flips. That beats trusting a table, which is the
//! same reason the LED matrix was brought up against Arduino's own heart frame.

use crate::i18n::{self, Msg};
use crate::{kprint, kprintln, vt, Platform, PortState};

/// Ports worth showing. The RA4M1's register block covers more, but they read
/// back as zero on this package and zero is indistinguishable from "sixteen
/// low inputs" — printing them would be inventing information.
const PORTS: usize = 6;

/// 1-based screen row of the P0 line.
const TOP: usize = 5;
/// 1-based column holding pin 15's character. Each pin takes three columns.
const LEFT: usize = 10;
/// Row for the `DEBUG ON` statistics line, below the assignment list.
const STATS_ROW: usize = TOP + PORTS + 12;

fn cell_col(pin: usize) -> usize {
    LEFT + (15 - pin) * 3
}

/// One character summarising a pin. `exists` is the port's pin-presence mask.
fn cell(s: &PortState, pin: usize, exists: u16) -> char {
    let m = 1u16 << pin;
    if exists & m == 0 {
        // Not on the die. Distinct from a pin that reads 0, which is the whole
        // point -- an absent pin used to be indistinguishable from a grounded
        // input, and there are a lot of them.
        return '.';
    }
    if s.analog & m != 0 {
        'a'
    } else if s.peripheral & m != 0 {
        'p'
    } else if s.output & m != 0 {
        if s.level & m != 0 {
            'H'
        } else {
            'L'
        }
    } else if s.level & m != 0 {
        '1'
    } else {
        '0'
    }
}

fn read_all(p: &mut dyn Platform) -> [PortState; PORTS] {
    let mut out = [PortState::default(); PORTS];
    for (i, slot) in out.iter_mut().enumerate() {
        if let Some(s) = p.port_state(i) {
            *slot = s;
        }
    }
    out
}

/// Header row of pin numbers, then one row per port.
fn draw_map(p: &mut dyn Platform, states: &[PortState; PORTS]) {
    vt::at(p, TOP - 1, 1);
    kprint!(p, "       ");
    for pin in (0..16).rev() {
        kprint!(p, "{:>3}", pin);
    }

    for port in 0..PORTS {
        let s = states[port];
        let exists = p.port_pins(port);
        vt::at(p, TOP + port, 1);
        kprint!(p, "  {}P{}{}   ", vt::sgr(vt::FG_BRIGHT), port, vt::RESET);
        for pin in (0..16).rev() {
            kprint!(p, "  {}", cell(&s, pin, exists));
        }
    }
}

/// Decimal digits in a small number, for costing a cursor address exactly.
fn digits(n: usize) -> usize {
    if n >= 100 {
        3
    } else if n >= 10 {
        2
    } else {
        1
    }
}

/// Rewrite only the cells whose character changed.
///
/// Returns `(cells rewritten, bytes sent)`. The byte count is computed rather
/// than measured, but exactly: each cell costs `ESC [ row ; col H` plus one
/// character, and every part of that is known.
fn draw_diff(
    p: &mut dyn Platform,
    old: &[PortState; PORTS],
    new: &[PortState; PORTS],
) -> (usize, usize) {
    let mut cells = 0;
    let mut bytes = 0;
    for port in 0..PORTS {
        let exists = p.port_pins(port);
        for pin in 0..16 {
            // Absent pins can never change, so they are never rewritten.
            if exists & (1 << pin) == 0 {
                continue;
            }
            let c = cell(&new[port], pin, exists);
            if c != cell(&old[port], pin, exists) {
                let (row, col) = (TOP + port, cell_col(pin));
                vt::at(p, row, col);
                kprint!(p, "{}", c);
                cells += 1;
                bytes += 2 + digits(row) + 1 + digits(col) + 1 + 1;
            }
        }
    }
    (cells, bytes)
}

/// Roughly what a full repaint costs, for the comparison the statistics line
/// is really making.
///
/// Counts the same things the differential figure does -- cursor addressing
/// and colour codes included -- or the comparison would flatter the diff by
/// charging it for addressing the full redraw got for free. Approximate
/// because `THEME MONO` drops the colour codes.
const FULL_MAP_BYTES: usize = 61 + PORTS * 70;

/// The pins this board has already spoken for, three to a line.
/// `row` of 0 means "print where the cursor already is" -- the snapshot flows,
/// the live view places it.
fn draw_notes(p: &mut dyn Platform, row: usize) {
    if row > 0 {
        vt::at(p, row, 1);
    }
    i18n::putln(p, Msg::PinsKnown);
    kprintln!(p);

    let mut n = 0usize;
    for port in 0..PORTS {
        for pin in 0..16 {
            let Some(note) = p.pin_note(port, pin) else {
                continue;
            };
            if n % 3 == 0 {
                kprint!(p, "   ");
            }
            kprint!(p, "P{}.{:02} {:<20}", port, pin, note);
            n += 1;
            if n % 3 == 0 {
                kprintln!(p);
            }
        }
    }
    if n % 3 != 0 {
        kprintln!(p);
    }
}

/// `PINS`, `PINS WATCH`, `PINS WATCH <port>.<pin>`.
pub fn show(p: &mut dyn Platform, watch: bool, pull: Option<(usize, usize)>) {
    if p.port_count() == 0 {
        i18n::putln(p, Msg::MsgNopins);
        return;
    }

    let mut states = read_all(p);

    if !watch {
        // A snapshot is ordinary shell output: it flows after the prompt and
        // leaves the history above it alone. Only the live view earns the
        // whole screen.
        kprintln!(p);
        i18n::putln(p, Msg::PinsTitle);
        kprintln!(p);
        kprint!(p, "       ");
        for pin in (0..16).rev() {
            kprint!(p, "{:>3}", pin);
        }
        kprintln!(p);
        for port in 0..PORTS {
            let s = states[port];
            let exists = p.port_pins(port);
            kprint!(p, "  {}P{}{}   ", vt::sgr(vt::FG_BRIGHT), port, vt::RESET);
            for pin in (0..16).rev() {
                kprint!(p, "  {}", cell(&s, pin, exists));
            }
            kprintln!(p);
        }
        kprintln!(p);
        i18n::putln(p, Msg::PinsLegend1);
        i18n::putln(p, Msg::PinsLegend2);
        kprintln!(p);
        draw_notes(p, 0);
        kprintln!(p);
        return;
    }

    // Live view: scratch screen, absolute positioning, nothing scrolls.
    vt::alt_enter(p);
    vt::cls_normal(p);
    vt::hide_cursor(p);
    vt::at(p, 2, 1);
    i18n::put(p, Msg::PinsTitle);

    draw_map(p, &states);

    let legend = TOP + PORTS + 1;
    vt::at(p, legend, 1);
    i18n::putln(p, Msg::PinsLegend1);
    i18n::putln(p, Msg::PinsLegend2);

    draw_notes(p, legend + 3);

    // An optional single pin gets a pull-up for the duration, so it rests at 1
    // and grounding it is an unambiguous 0 rather than a coin toss. One pin,
    // named by the caller, and put back on the way out.
    if let Some((port, pin)) = pull {
        if p.pin_pullup(port, pin, true) {
            states = read_all(p);
            draw_map(p, &states);
        }
    }

    vt::at(p, legend + 2, 1);
    i18n::put(p, Msg::PinsWatching);

    // `DEBUG ON` turns this view into an instrument: it reports what each
    // update actually costs on the wire, against what a full repaint would.
    // That is the whole claim behind the differential renderer, and it should
    // be checkable rather than asserted.
    let stats = crate::esp::trace();
    let started = p.uptime_ms();
    let mut ticks = 0usize;
    let mut total = 0usize;

    // Poll rather than spin flat out: the map is 96 register reads and the
    // eye cannot use more than about ten updates a second anyway.
    let mut next = p.uptime_ms();
    loop {
        if p.get().is_some() {
            break;
        }
        let now = p.uptime_ms();
        if now < next {
            continue;
        }
        next = now + 100;

        let fresh = read_all(p);
        let (cells, bytes) = draw_diff(p, &states, &fresh);
        states = fresh;

        if stats {
            ticks += 1;
            total += bytes;
            let secs = (now.wrapping_sub(started) / 1000).max(1);
            vt::at(p, STATS_ROW, 1);
            vt::clear_eol(p);
            kprint!(p, "{}", vt::sgr(vt::FG_CYAN));
            i18n::put(p, Msg::PinsStats);
            kprintln!(
                p,
                "tick {}  changed {}  sent {} B  total {} B  {} B/s  (full map ~{} B){}",
                ticks,
                cells,
                bytes,
                total,
                total as u64 / secs,
                FULL_MAP_BYTES,
                vt::RESET
            );
        }
    }

    if let Some((port, pin)) = pull {
        p.pin_pullup(port, pin, false);
    }

    vt::show_cursor(p);
    vt::alt_leave(p);
}
