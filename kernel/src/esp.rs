//! Talking to the ESP32-S3 bridge: the wire protocol, a trace mode, and a
//! menu of commands that are known to work.
//!
//! # Why a menu
//!
//! `AT` is a raw passthrough with no validation, and the bridge is the chip
//! that presents USB. A command it does not expect can leave it unresponsive,
//! and clearing that needs the cable physically unplugged. Typing command names
//! by hand is therefore the riskiest thing this shell offers.
//!
//! [`MENU`] is the opposite: every entry has been sent to this hardware and
//! seen to answer. You cannot select something unproven, because unproven
//! things are not in the list. `AT` stays for exploration; this is for use.
//!
//! # Reply framing
//!
//! Replies are `+NAME: <bytes>|<payload>` followed by `OK`, or a bare `ERROR`.
//! `AT+WIFISCAN` answers `+WIFISCAN: 304|SSID | BSSID | RSSI | ch | security`
//! and one line per further network. `AT+GMR` answers a literal `<len>` where
//! the length should be, which appears to be a stub in the bridge firmware
//! rather than anything we are doing wrong.
//!
//! # Timing
//!
//! A fixed reply window does not fit: `AT+PREFSTAT` answers in milliseconds and
//! `AT+WIFISCAN` takes the best part of ten seconds, because it really is
//! scanning. Waiting a flat ten seconds for everything would make the shell
//! feel broken; waiting one would have lost the scan -- which is exactly what
//! happened, and made `WIFISCAN` look like a command that did not exist.
//!
//! So each command carries how long to wait for its *first* byte, and after
//! that the reply is read until a terminator or until the link falls quiet.

use crate::i18n::{self, Msg};
use crate::setup::{read_key, Key};
use crate::{kprint, kprintln, vt, Platform};

/// How a reply ended.
pub enum Outcome {
    /// Terminated with `OK`.
    Ok,
    /// Terminated with `ERROR`.
    Error,
    /// Nothing came back at all.
    Silent,
    /// Something came back, but no terminator followed it.
    Truncated,
}

/// Where a reply is written.
enum Sink {
    /// Straight to the shell, as `AT` does.
    Console,
    /// Into a fixed region of the screen, for the menu. Lines past the bottom
    /// are consumed but not drawn.
    Pane { top: usize, height: usize },
}

/// Milliseconds of silence after the first byte before giving up on a
/// terminator. Generous: the bridge streams a scan a line at a time.
const IDLE_MS: u64 = 1500;
/// Absolute ceiling, so a confused bridge cannot hold the shell forever.
const HARD_MS: u64 = 20_000;

/// How long a hand-typed `AT` waits for its first byte. Long enough for a
/// network scan, short enough that a typo does not feel like a hang.
pub const RAW_FIRST_MS: u64 = 10_000;

// ---------------------------------------------------------------- trace

/// Settings key for the link trace.
///
/// Stored as 1 for off and 2 for on, never 0: a key that was never written
/// reads back as zero, and "off" has to stay distinguishable from "no opinion"
/// -- the same reason `theme` is stored as index+1.
pub const TRACE_KEY: &str = "trace";

static TRACE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

pub fn set_trace(on: bool) {
    TRACE.store(on, core::sync::atomic::Ordering::Relaxed);
}

pub fn trace() -> bool {
    TRACE.load(core::sync::atomic::Ordering::Relaxed)
}

// ---------------------------------------------------------------- the menu

/// Commands that have been sent to this board and seen to answer.
///
/// The last field is how long to wait for the first byte. Nothing goes in here
/// on the strength of a guess -- the whole point is that selecting from this
/// list cannot reach an unproven command.
#[rustfmt::skip]
pub static MENU: &[(Msg, &str, u64)] = &[
    (Msg::EspLink,     "AT",               2_000),
    (Msg::EspVersion,  "AT+GMR",           2_000),
    (Msg::EspStore,    "AT+PREFSTAT",      2_000),
    (Msg::EspScan,     "AT+WIFISCAN",     15_000),
    (Msg::EspSocket,   "AT+BEGINCLIENT",   3_000),
    (Msg::EspRadio,    "AT+SOFTRESETWIFI", 5_000),
];

// ---------------------------------------------------------------- protocol

/// Write a command, normalising it the way the bridge expects.
///
/// Upper case up to the first `=` (the bridge rejects lower case, but a
/// parameter after `=` is data), exactly one leading `AT`, and a `+` inserted
/// when the caller left it out.
pub fn send(p: &mut dyn Platform, cmd: &str, traced: bool) {
    // Drain to silence first: a byte already on the wire from a previous
    // command would otherwise prefix this reply.
    let start = p.uptime_ms();
    let mut quiet = start;
    while p.uptime_ms().wrapping_sub(quiet) < 60 {
        if p.modem_get().is_some() {
            quiet = p.uptime_ms();
        }
        if p.uptime_ms().wrapping_sub(start) > 500 {
            break;
        }
    }

    // Build the line first, so the trace can show what actually goes out.
    // Printing the caller's argument instead was quietly misleading: `AT`
    // traced as an empty string and `at gmr` traced as `gmr`, when the wire
    // saw `AT` and `AT+GMR`. A debug view that does not show the wire is
    // worse than no debug view.
    let mut line = [0u8; 160];
    let n = normalise(cmd, &mut line);

    // Safe to print here: the link was just drained to silence, so nothing is
    // in flight to collide with the console write.
    if traced {
        kprint!(p, "  {}[tx] ", vt::sgr(vt::FG_CYAN));
        for i in 0..n {
            let c = line[i];
            p.put(c);
        }
        kprintln!(p, "{}", vt::RESET);
    }

    for i in 0..n {
        let c = line[i];
        p.modem_put(c);
    }
    p.modem_put(b'\r');
    p.modem_put(b'\n');
}

/// Render `cmd` the way the bridge wants it, returning the length written.
///
/// Exactly one leading `AT`, a `+` inserted when the caller left it out, and
/// the command name upper-cased as far as the first `=` -- beyond that is a
/// parameter, where case is data rather than syntax.
fn normalise(cmd: &str, out: &mut [u8; 160]) -> usize {
    let mut n = 0usize;
    let b = cmd.as_bytes();
    let starts_at = b.len() >= 2 && (b[0] | 32) == b'a' && (b[1] | 32) == b't';

    if !starts_at {
        out[0] = b'A';
        out[1] = b'T';
        n = 2;
        if !b.is_empty() && b[0] != b'+' && b[0] != b'&' {
            out[2] = b'+';
            n = 3;
        }
    }

    let fold_to = cmd.find('=').unwrap_or(cmd.len());
    for (i, c) in cmd.bytes().enumerate() {
        if n >= out.len() {
            break;
        }
        out[n] = if i < fold_to { c.to_ascii_uppercase() } else { c };
        n += 1;
    }
    n
}

/// Read one reply. `first_ms` is how long to wait before concluding that
/// nothing is coming.
fn read(p: &mut dyn Platform, first_ms: u64, sink: Sink, traced: bool) -> Outcome {
    let start = p.uptime_ms();
    let mut last = start;
    let mut got = false;
    // Gathered for the trace, reported afterwards. Nothing is printed inside
    // the loop.
    let mut first_at = 0u64;
    let mut count = 0usize;
    let mut nonprint = 0usize;

    // Enough of the current line to recognise a terminator at end-of-line,
    // which is where it has to be checked -- a payload can contain "OK".
    let mut tail = [0u8; 5];
    let mut n = 0usize;
    let mut row = 0usize;

    if let Sink::Pane { top, height } = sink {
        for r in 0..height {
            vt::at(p, top + r, 1);
            vt::clear_eol(p);
        }
        vt::at(p, top, 1);
    }

    loop {
        if let Some(b) = p.modem_get() {
            if !got {
                first_at = p.uptime_ms().wrapping_sub(start);
            }
            got = true;
            last = p.uptime_ms();
            count += 1;

            match b {
                b'\n' => {
                    let end = (n == 2 && &tail[..2] == b"OK")
                        || (n == 5 && &tail[..5] == b"ERROR")
                        || (n == 6 && &tail[..5] == b"+ERRO");
                    row += 1;
                    match sink {
                        Sink::Console => kprintln!(p),
                        Sink::Pane { top, height } => {
                            if row < height {
                                vt::at(p, top + row, 1);
                            }
                        }
                    }
                    n = 0;
                    if end {
                        let r = if tail[0] == b'O' { Outcome::Ok } else { Outcome::Error };
                        return done(p, r, first_at, count, nonprint, traced);
                    }
                }
                b'\r' => {}
                0x20..=0x7e => {
                    let visible = match sink {
                        Sink::Console => true,
                        Sink::Pane { height, .. } => row < height,
                    };
                    if visible {
                        p.put(b);
                    }
                    if n < tail.len() {
                        tail[n] = b;
                    }
                    n += 1;
                }
                _ => {
                    // Not printed: a write here costs a character time, and
                    // the reply is arriving at the same rate it would go out.
                    // The trace summary reports the count instead.
                    nonprint += 1;
                    // Any non-printable rules out a clean terminator line.
                    n = tail.len() + 1;
                }
            }
            continue;
        }

        let now = p.uptime_ms();
        if !got {
            if now.wrapping_sub(start) > first_ms {
                return done(p, Outcome::Silent, first_at, count, nonprint, traced);
            }
        } else if now.wrapping_sub(last) > IDLE_MS {
            return done(p, Outcome::Truncated, first_at, count, nonprint, traced);
        }
        if now.wrapping_sub(start) > HARD_MS {
            return done(p, Outcome::Truncated, first_at, count, nonprint, traced);
        }
    }
}

/// Emit the trace summary, if tracing, and hand the outcome back.
fn done(p: &mut dyn Platform, r: Outcome, first_at: u64, count: usize, nonprint: usize, traced: bool) -> Outcome {
    if traced {
        kprintln!(p);
        kprintln!(
            p,
            "  {}[rx] {} bytes, first after {} ms, {} non-printable{}",
            vt::sgr(vt::FG_CYAN),
            count,
            first_at,
            nonprint,
            vt::RESET
        );
    }
    r
}

/// Send `cmd` and print the reply on the console. Used by `AT`.
pub fn converse(p: &mut dyn Platform, cmd: &str, first_ms: u64) -> Outcome {
    if trace() {
        kprintln!(p);
    }
    // `send` traces the normalised line itself, before writing anything, so
    // the console write cannot collide with a reply in flight.
    send(p, cmd, trace());
    kprintln!(p);
    let r = read(p, first_ms, Sink::Console, trace());
    match r {
        Outcome::Silent => i18n::putln(p, Msg::EspNoreply),
        Outcome::Truncated => i18n::putln(p, Msg::EspTruncated),
        _ => {}
    }
    kprintln!(p);
    r
}

// ---------------------------------------------------------------- selector

const ITEM_ROW: usize = 5;
const PANE_TOP: usize = ITEM_ROW + 8;
const PANE_H: usize = 9;
const RULE_ROW: usize = PANE_TOP + PANE_H;
const KEYS_ROW: usize = RULE_ROW + 1;
const W: usize = 78;

fn draw_item(p: &mut dyn Platform, i: usize, sel: usize) {
    let (label, cmd, _) = MENU[i];
    vt::at(p, ITEM_ROW + i, 1);
    vt::clear_eol(p);
    kprint!(p, "{}", vt::scheme());
    kprint!(p, " {} ", if i == sel { ">" } else { " " });
    if i == sel {
        kprint!(p, "{}", vt::sgr(vt::FG_YELLOW));
    }
    vt::dotfill(p, i18n::t(label), 38);
    kprint!(p, "  {}{}", cmd, vt::scheme());
}

fn draw_frame(p: &mut dyn Platform, sel: usize) {
    kprint!(p, "{}", vt::scheme());
    vt::cls(p);
    kprint!(p, "{}", vt::sgr(vt::BOLD));
    vt::box_open(p, W);
    vt::box_row(p, W, i18n::t(Msg::EspTitle));
    kprint!(p, "{}{}", vt::RESET, vt::scheme());
    vt::box_close(p, W);

    for i in 0..MENU.len() {
        draw_item(p, i, sel);
    }

    vt::at(p, PANE_TOP - 1, 1);
    vt::rule(p, W);
    vt::at(p, RULE_ROW, 1);
    vt::rule(p, W);
    vt::at(p, KEYS_ROW, 1);
    i18n::put(p, Msg::EspKeys);
}

/// The selector. Returns when the user leaves.
pub fn menu(p: &mut dyn Platform) {
    if !p.modem_present() {
        i18n::putln(p, Msg::MsgNomodem);
        return;
    }

    vt::alt_enter(p);
    vt::hide_cursor(p);
    let mut sel = 0usize;
    draw_frame(p, sel);

    loop {
        match read_key(p) {
            k @ (Key::Up | Key::Down) => {
                let prev = sel;
                sel = match k {
                    Key::Up => (sel + MENU.len() - 1) % MENU.len(),
                    _ => (sel + 1) % MENU.len(),
                };
                draw_item(p, prev, sel);
                draw_item(p, sel, sel);
            }
            Key::Enter => {
                let (_, cmd, first) = MENU[sel];
                // On the status line, not in the pane: the pane is cleared as
                // the reply starts arriving, and a scan takes six seconds --
                // long enough that silence looks like a hang.
                vt::at(p, RULE_ROW, 1);
                vt::clear_eol(p);
                kprint!(p, "{}", vt::scheme());
                i18n::put1(p, Msg::EspRunning, cmd);

                // Never traced: the trace writes at the cursor and this
                // view owns the whole screen. A stray [tx] line would land
                // in the middle of the menu.
                send(p, cmd, false);
                let r = read(p, first, Sink::Pane { top: PANE_TOP, height: PANE_H }, false);

                vt::at(p, RULE_ROW, 1);
                vt::clear_eol(p);
                kprint!(p, "{}", vt::scheme());
                match r {
                    Outcome::Ok => i18n::put(p, Msg::EspOk),
                    Outcome::Error => i18n::put(p, Msg::EspError),
                    Outcome::Silent => i18n::put(p, Msg::EspNoreply),
                    Outcome::Truncated => i18n::put(p, Msg::EspTruncated),
                }
            }
            Key::Esc => break,
            _ => {}
        }
    }

    vt::show_cursor(p);
    vt::alt_leave(p);
}
