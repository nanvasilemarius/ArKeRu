//! The command line itself: a fixed-buffer line editor with history, and the
//! dispatch loop.
//!
//! No allocation anywhere. The line buffer and the history ring are inline
//! arrays sized at compile time, so the shell's entire memory cost is visible
//! in `size_of::<Shell>()` and cannot grow at runtime.

use crate::{cmds, kprint, kprintln, marquee::Marquee, Platform, HISTORY, LINE_MAX};

/// Where the ANSI escape parser is between bytes.
#[derive(Clone, Copy, PartialEq)]
enum Esc {
    None,
    Saw1b,
    SawBracket,
}

pub struct Shell {
    buf: [u8; LINE_MAX],
    len: usize,

    hist: [[u8; LINE_MAX]; HISTORY],
    hist_len: [usize; HISTORY],
    /// How many slots hold a real entry (saturates at HISTORY).
    hist_count: usize,
    /// Next slot to write.
    hist_head: usize,
    /// How far back the user has arrowed. 0 = editing a fresh line.
    hist_back: usize,

    esc: Esc,
    /// First numeric parameter of the CSI sequence being parsed, or 0.
    esc_param: u8,
    /// Set after a CR so a following LF is swallowed. Terminals send CR,
    /// pipes send LF, and PowerShell here-strings send CRLF -- all must mean
    /// exactly one Enter.
    saw_cr: bool,
}

impl Default for Shell {
    fn default() -> Self {
        Self::new()
    }
}

impl Shell {
    pub const fn new() -> Self {
        Self {
            buf: [0; LINE_MAX],
            len: 0,
            hist: [[0; LINE_MAX]; HISTORY],
            hist_len: [0; HISTORY],
            hist_count: 0,
            hist_head: 0,
            hist_back: 0,
            esc: Esc::None,
            esc_param: 0,
            saw_cr: false,
        }
    }

    /// The post-boot DOS banner, then the first prompt.
    pub fn banner<P: Platform>(&mut self, p: &mut P, _drive: u8) {
        crate::i18n::putln1(p, crate::i18n::Msg::ShellVersion, crate::VERSION);
        crate::i18n::putln(p, crate::i18n::Msg::ShellCopyright);
        kprintln!(p);
        crate::i18n::putln(p, crate::i18n::Msg::ShellTypehelp);
        kprintln!(p);
        // No prompt here: AUTOEXEC.BAT runs between the banner and the first
        // prompt, exactly as it did on DOS.
    }

    pub fn prompt<P: Platform>(&mut self, p: &mut P, drive: u8) {
        kprint!(p, "{}:\\>", drive as char);
    }

    /// Consume whatever input is pending. Call this in a loop.
    pub fn poll<P: Platform>(&mut self, p: &mut P, m: &mut Marquee, drive: u8) {
        while let Some(b) = p.get() {
            self.feed(p, m, b, drive);
        }
    }

    fn feed<P: Platform>(&mut self, p: &mut P, m: &mut Marquee, b: u8, drive: u8) {
        // --- escape sequence handling (arrow keys) ---
        match self.esc {
            Esc::Saw1b => {
                self.esc_param = 0;
                self.esc = if b == b'[' { Esc::SawBracket } else { Esc::None };
                return;
            }
            Esc::SawBracket => {
                // A CSI sequence is `ESC [` , then parameter bytes (0x30-0x3F)
                // and intermediates (0x20-0x2F), terminated by a final byte
                // (0x40-0x7E). Treating the byte straight after `[` as final
                // works for the arrows -- `ESC [ A` -- but not for anything
                // parameterised: Delete is `ESC [ 3 ~`, so the `3` was eaten
                // and the `~` arrived as text.
                if (0x20..=0x3f).contains(&b) {
                    // Remember the first parameter digit; that is enough to
                    // tell Delete (3) from Home (1), End (4) and the rest.
                    if self.esc_param == 0 && b.is_ascii_digit() {
                        self.esc_param = b - b'0';
                    }
                    return;
                }
                let param = core::mem::take(&mut self.esc_param);
                self.esc = Esc::None;
                match b {
                    b'A' => self.recall(p, 1, drive),
                    b'B' => self.recall(p, 0, drive),
                    // `ESC [ 3 ~` is Delete. The BIOS screen offers it as the
                    // way into SETUP, so honour that here.
                    b'~' if param == 3 => {
                        kprintln!(p);
                        crate::setup::run(p, m);
                        self.len = 0;
                        self.prompt(p, drive);
                    }
                    _ => {}
                }
                return;
            }
            Esc::None => {}
        }

        // Only an LF *directly* after a CR is a continuation; anything else
        // clears the pairing.
        if b != b'\n' {
            self.saw_cr = false;
        }

        match b {
            0x1b => self.esc = Esc::Saw1b,

            // Enter. Accept either terminator so CR, LF and CRLF all behave.
            b'\r' => {
                self.saw_cr = true;
                kprintln!(p);
                self.execute(p, m, drive);
            }
            b'\n' => {
                if !core::mem::replace(&mut self.saw_cr, false) {
                    kprintln!(p);
                    self.execute(p, m, drive);
                }
            }

            // Backspace / DEL
            0x08 | 0x7f => {
                if self.len > 0 {
                    self.len -= 1;
                    kprint!(p, "\x08 \x08");
                }
            }

            // Ctrl-C: abandon the line, DOS-style.
            0x03 => {
                kprintln!(p, "^C");
                self.len = 0;
                self.hist_back = 0;
                self.prompt(p, drive);
            }

            // Printable ASCII only. Anything else is ignored rather than
            // corrupting the buffer.
            0x20..=0x7e => {
                if self.len < LINE_MAX {
                    self.buf[self.len] = b;
                    self.len += 1;
                    p.put(b);
                }
            }

            _ => {}
        }
    }

    fn execute<P: Platform>(&mut self, p: &mut P, m: &mut Marquee, drive: u8) {
        let len = self.len;
        self.len = 0;
        self.hist_back = 0;

        if len > 0 {
            // Copy out first: `push_history` needs `&mut self`, which cannot
            // coexist with a `&str` borrowed from `self.buf`.
            let mut tmp = [0u8; LINE_MAX];
            tmp[..len].copy_from_slice(&self.buf[..len]);
            // Safe: the editor only ever stores printable ASCII.
            let line = core::str::from_utf8(&tmp[..len]).unwrap_or("");
            self.push_history(line);
            cmds::dispatch(p, m, line);
        }
        self.prompt(p, drive);
    }

    fn push_history(&mut self, line: &str) {
        let bytes = line.as_bytes();
        let n = bytes.len().min(LINE_MAX);
        let slot = self.hist_head;
        self.hist[slot][..n].copy_from_slice(&bytes[..n]);
        self.hist_len[slot] = n;
        self.hist_head = (self.hist_head + 1) % HISTORY;
        if self.hist_count < HISTORY {
            self.hist_count += 1;
        }
    }

    /// `dir` of 1 walks back through history, 0 walks forward.
    fn recall<P: Platform>(&mut self, p: &mut P, dir: u8, drive: u8) {
        if self.hist_count == 0 {
            return;
        }
        if dir == 1 {
            if self.hist_back < self.hist_count {
                self.hist_back += 1;
            }
        } else if self.hist_back > 0 {
            self.hist_back -= 1;
        }

        // Redraw the line in place.
        kprint!(p, "\r");
        crate::vt::clear_eol(p);
        self.prompt(p, drive);

        if self.hist_back == 0 {
            self.len = 0;
            return;
        }

        let slot = (self.hist_head + HISTORY - self.hist_back) % HISTORY;
        let n = self.hist_len[slot];
        // Copy through a temporary so we are not borrowing self.hist while
        // writing into self.buf.
        let mut tmp = [0u8; LINE_MAX];
        tmp[..n].copy_from_slice(&self.hist[slot][..n]);
        self.buf[..n].copy_from_slice(&tmp[..n]);
        self.len = n;
        for &c in &tmp[..n] {
            p.put(c);
        }
    }
}
