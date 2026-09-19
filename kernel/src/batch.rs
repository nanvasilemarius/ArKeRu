//! Batch file execution, with the `AUTOEXEC.BAT` semantics you would expect.
//!
//! - a leading `@` suppresses echo of that one line
//! - `ECHO OFF` / `ECHO ON` suppress echo of everything after
//! - `REM` and blank lines are skipped
//! - anything else is handed to the normal command dispatcher
//!
//! Echoed lines are printed as `A:\>COMMAND`, matching what you would have
//! seen had you typed them.

use crate::i18n::{self, Chars, Msg};
use crate::marquee::Marquee;
use crate::{cmds, kprintln, romfs, Platform, LINE_MAX};
use core::sync::atomic::{AtomicU8, Ordering};

/// The file run automatically after boot.
pub const AUTOEXEC: &str = "AUTOEXEC.BAT";

/// How deep batch files may call each other. A script that runs itself is an
/// easy mistake to make and would otherwise wedge the machine at boot with no
/// way in, since the shell never starts.
const MAX_DEPTH: u8 = 4;
static DEPTH: AtomicU8 = AtomicU8::new(0);

/// Whether a batch file is running.
///
/// The pager asks, because `AUTOEXEC.BAT` runs before there is anybody at the
/// keyboard and a `-- More --` prompt would stop the boot dead.
pub fn in_batch() -> bool {
    DEPTH.load(Ordering::Relaxed) > 0
}

/// Run `AUTOEXEC.BAT` if it exists. Missing is not an error.
pub fn autoexec(p: &mut dyn Platform, m: &mut Marquee, drive: u8) {
    if romfs::find(AUTOEXEC).is_some() {
        run(p, m, drive, AUTOEXEC);
    }
}

/// Execute a batch file from the ROM volume.
pub fn run(p: &mut dyn Platform, m: &mut Marquee, drive: u8, name: &str) -> bool {
    let Some(file) = romfs::find(name) else {
        return false;
    };

    if DEPTH.load(Ordering::Relaxed) >= MAX_DEPTH {
        i18n::putln1(p, Msg::MsgBatchdeep, name);
        return true;
    }
    DEPTH.fetch_add(1, Ordering::Relaxed);

    // Read once: the body is looked up per language, and switching language
    // mid-script would otherwise change the file being executed underneath the
    // loop -- `LANG RO` inside AUTOEXEC.BAT is a perfectly reasonable line.
    let body = file.body();

    // The body arrives a character at a time, so a line is gathered here before
    // it can be dispatched -- a command has to be whole to be split into a verb
    // and its arguments. `LINE_MAX` is the shell's own input buffer, which is
    // the right bound: a line of a batch file is a line somebody could type.
    let mut buf = [0u8; LINE_MAX];
    let mut n = 0;
    let mut echo = true;
    body.each(&mut |b| {
        if b != b'\n' {
            if n < LINE_MAX {
                buf[n] = b;
                n += 1;
            }
            return;
        }
        line(p, m, drive, &mut echo, &buf[..n]);
        n = 0;
    });
    // A last line with no newline after it is still a line.
    line(p, m, drive, &mut echo, &buf[..n]);

    DEPTH.fetch_sub(1, Ordering::Relaxed);
    true
}

/// Run one line of a batch file.
fn line(p: &mut dyn Platform, m: &mut Marquee, drive: u8, echo: &mut bool, raw: &[u8]) {
    let line = core::str::from_utf8(raw).unwrap_or("").trim();
    if line.is_empty() {
        return;
    }

    // `@` suppresses echo for this line only.
    let (quiet, line) = match line.strip_prefix('@') {
        Some(rest) => (true, rest.trim()),
        None => (false, line),
    };
    if line.is_empty() {
        return;
    }

    let (head, args) = cmds::split_first(line);

    if cmds::ieq(head, "REM") {
        return;
    }

    // ECHO ON/OFF control echoing; ECHO <anything else> prints, so it
    // falls through to the dispatcher.
    if cmds::ieq(head, "ECHO") {
        if cmds::ieq(args, "OFF") {
            *echo = false;
            return;
        }
        if cmds::ieq(args, "ON") {
            *echo = true;
            return;
        }
    }

    if *echo && !quiet {
        kprintln!(p, "{}:\\>{}", drive as char, line);
    }
    cmds::dispatch_dyn(p, m, line);
}
