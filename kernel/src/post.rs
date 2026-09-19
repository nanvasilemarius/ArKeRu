//! The power-on self test screen.
//!
//! Styled after late-90s PC firmware -- bright white on blue, double rule at
//! the top, dot-filled device table, animated memory count-up. The vendor
//! strings are this project's own; it is the *look* that is being reproduced,
//! not any real vendor's firmware.
//!
//! Every label goes through [`i18n`], so the boot screen follows `LANG`.

use crate::i18n::{self, Chars, Msg, Text};
use crate::{kprint, kprintln, romfs, vt, Platform, VERSION};

const W: usize = 78;

/// The status column of the device table.
///
/// A ready-made string, or a number and its translated unit -- which cannot be
/// collapsed into one string on a machine with no allocator.
enum Detail {
    Text(Text),
    Count(usize, Msg),
}

impl Detail {
    /// Print left-aligned in `width` columns, with the surrounding spaces the
    /// old `" {:<16} "` gave.
    fn put(&self, p: &mut dyn Platform, width: usize) {
        match *self {
            Detail::Text(s) => kprint!(p, " {:<w$} ", s, w = width),
            Detail::Count(n, unit) => {
                let unit = i18n::t(unit);
                kprint!(p, " {} {}", n, unit);
                // Pad by hand. `chars()` rather than `len()`: a translation may
                // hold non-ASCII, and a byte count would misalign the column.
                let used = digits(n) + 1 + unit.len();
                for _ in used..width {
                    kprint!(p, " ");
                }
                kprint!(p, " ");
            }
        }
    }
}

fn digits(mut n: usize) -> usize {
    let mut d = 1;
    while n >= 10 {
        n /= 10;
        d += 1;
    }
    d
}

/// Run the full POST sequence. Blocks for roughly a second on the animation.
pub fn run(p: &mut dyn Platform) {
    vt::hide_cursor(p);
    kprint!(p, "{}", vt::scheme());
    vt::cls(p);

    header(p);
    identity(p);
    memory_test(p);
    devices(p);
    footer(p);

    vt::show_cursor(p);
}

fn header(p: &mut dyn Platform) {
    kprint!(p, "{}", vt::sgr(vt::BOLD));
    vt::box_open(p, W);
    vt::box_row(p, W, "ArduinoKernel BIOS");
    kprint!(p, "{}", vt::RESET);
    kprint!(p, "{}", vt::scheme());
    vt::box_row(p, W, i18n::t(Msg::PostCopyright));
    vt::box_close(p, W);
    kprintln!(p);
}

fn identity(p: &mut dyn Platform) {
    let mhz = p.cpu_mhz();
    let cpu = p.cpu_name();
    let board = p.board_name();

    kprintln!(p, " {:<18} : {} @ {} MHz", i18n::t(Msg::PostCpu), cpu, mhz);
    kprintln!(
        p,
        " {:<18} : ESP32-S3-MINI-1-N8  (Xtensa LX7, {})",
        i18n::t(Msg::PostCoproc),
        i18n::t(Msg::PostIdle)
    );
    kprintln!(p, " {:<18} : {}", i18n::t(Msg::PostBoard), board);
    kprintln!(
        p,
        " {:<18} : {}  {} 0001",
        i18n::t(Msg::PostBios),
        VERSION,
        i18n::t(Msg::PostBuild)
    );

    // Why this boot happened, on the screen you are looking at *because* it
    // happened. It was only in SETUP, which is the one place you are not
    // looking when the machine has just restarted on its own.
    let cause = crate::reset_cause_msg(p.reset_cause());
    kprintln!(
        p,
        " {:<18} : {}",
        i18n::t(Msg::SetupResetcause),
        i18n::t(cause)
    );
    kprintln!(p);
}

/// The count-up. Real firmware was actually walking RAM; this walks a counter,
/// because on a 32 KB part destroying your own stack to prove it exists is a
/// poor trade. The pacing is what sells it.
fn memory_test(p: &mut dyn Platform) {
    let total_kb = p.ram_total() / 1024;
    let steps = 16;
    let inc = total_kb.max(steps) / steps;
    let label = i18n::t(Msg::PostMemtest);

    let mut shown = 0usize;
    for _ in 0..steps {
        shown = (shown + inc).min(total_kb);
        kprint!(p, "\r {:<18} : {:>6} KB", label, shown);
        p.delay_ms(35);
    }
    kprint!(p, "\r {:<18} : {:>6} KB  ", label, total_kb);
    vt::tag(p, true);
    kprintln!(p);

    let rom_kb = p.rom_total() / 1024;
    kprintln!(p, " {:<18} : {:>6} KB", i18n::t(Msg::PostFlash), rom_kb);
    kprintln!(p);
}

fn devices(p: &mut dyn Platform) {
    kprintln!(p, " {}", i18n::t(Msg::PostDetecting));

    // The status column used to be a plain `&str`, which is why it read
    // `4 files` -- a hardcoded count *and* a hardcoded language, in the middle
    // of a screen that is otherwise fully translated. A number plus a
    // translated unit cannot be one `&str` without a formatter and a buffer,
    // and there is no allocator here, so it stays two prints and pads itself.

    let matrix = p.has_led_matrix();
    let console = p.console_name();

    // `present` distinguishes "not fitted" from "fitted but broken" -- an
    // absent LED matrix is a design choice, not a failure, so it must not
    // render as [FAIL].
    let entries: [(Msg, Detail, bool, bool); 4] = [
        (Msg::PostConsole, Detail::Text(Text::Plain(console)), true, true),
        (
            Msg::PostMatrix,
            Detail::Text(if matrix {
                i18n::t(Msg::PostPx)
            } else {
                i18n::t(Msg::PostNotfitted)
            }),
            matrix,
            matrix,
        ),
        (
            Msg::PostRomvol,
            Detail::Count(romfs::FILES.len(), Msg::MsgFiles),
            true,
            true,
        ),
        (
            Msg::PostBridge,
            Detail::Text(i18n::t(Msg::PostRelaying)),
            true,
            true,
        ),
    ];

    for (name, detail, ok, present) in entries {
        vt::dotfill(p, i18n::t(name), 38);
        detail.put(p, 16);
        if present {
            vt::tag(p, ok);
        } else {
            kprint!(p, "[ -- ]");
        }
        kprintln!(p);
        p.delay_ms(60);
    }
    kprintln!(p);
}

fn footer(p: &mut dyn Platform) {
    i18n::putln(p, Msg::PostSetup);
    kprintln!(p);
    vt::rule(p, W);
    i18n::put1(p, Msg::PostBooting, romfs::VOLUME);
    kprint!(p, " {} ", romfs::total_bytes());
    i18n::put(p, Msg::MsgBytes);
    kprint!(p, " / {} ", romfs::FILES.len());
    i18n::putln(p, Msg::MsgFiles);

    // Hand the screen back in the terminal's default colours, and repaint
    // everything below this point to match.
    //
    // A terminal stores attributes per cell, so the blue above stays blue --
    // which is the effect wanted, BIOS above the line and DOS below it. Without
    // the reset the scheme stayed set and leaked into everything that followed:
    // the shell drew on blue, and the games, which clear the screen and then
    // print their own colours against a `RESET`, came out striped blue where
    // the clear had reached and default where the text had.
    kprint!(p, "{}", vt::NORMAL);
    vt::clear_eos(p);
    kprintln!(p);
}
