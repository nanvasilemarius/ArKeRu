//! `-- More --` paging, so long output can be read without scrolling back.
//!
//! The shell is a teletype, as DOS was: output flows downwards and the screen
//! scrolls. That is fine for a two-line answer and poor for `HELP`, which is
//! about forty lines and pushes its own beginning off the top before you have
//! read any of it.
//!
//! The fix DOS used was a pager, and it is the right one here too. A
//! full-screen shell would stop the scrolling as well, but it would do so by
//! throwing away the scrollback — you could no longer look at something from
//! five commands ago, because it would not exist. Paging keeps the history and
//! removes the need to chase it.
//!
//! Paging is suppressed inside batch files. `AUTOEXEC.BAT` runs before there
//! is anyone at the keyboard, and a `-- More --` prompt there would stop the
//! boot dead.

use crate::i18n::{self, Chars, Msg};
use crate::{kprint, vt, Platform};
use core::sync::atomic::{AtomicU8, Ordering};

/// Settings key for the page height.
///
/// Stored as rows + 1, because 0 rows is a meaningful setting -- it means
/// "never page" -- and has to stay distinguishable from a key that was never
/// written.
pub const PAGE_KEY: &str = "page";

/// Rows in a standard terminal. One is kept for the prompt itself.
pub const DEFAULT_ROWS: u8 = 24;

static ROWS: AtomicU8 = AtomicU8::new(DEFAULT_ROWS);

pub fn set_rows(n: u8) {
    ROWS.store(n, Ordering::Relaxed);
}

pub fn rows() -> u8 {
    ROWS.load(Ordering::Relaxed)
}

/// Counts lines and stops for a key when the screen is full.
pub struct Pager {
    printed: usize,
    since_pause: usize,
    stopped: bool,
}

impl Default for Pager {
    fn default() -> Self {
        Self::new()
    }
}

impl Pager {
    pub fn new() -> Self {
        Self {
            printed: 0,
            since_pause: 0,
            stopped: false,
        }
    }

    /// Whether the reader has asked to stop.
    pub fn stopped(&self) -> bool {
        self.stopped
    }

    /// Call once for every line emitted. Returns `false` once the reader has
    /// quit, at which point the caller should stop producing output.
    pub fn line(&mut self, p: &mut dyn Platform) -> bool {
        if self.stopped {
            return false;
        }
        self.printed += 1;

        let limit = rows() as usize;
        // Zero disables paging entirely, and a batch file has nobody to press
        // a key.
        if limit == 0 || crate::batch::in_batch() {
            return true;
        }

        self.since_pause += 1;
        if self.since_pause + 1 < limit {
            return true;
        }

        i18n::put1(p, Msg::PagerMore, self.printed);
        let key = loop {
            if let Some(b) = p.get() {
                break b;
            }
        };

        // Erase the prompt so the paged text reads continuously rather than
        // being interrupted by a row of leftover prompts.
        kprint!(p, "\r");
        vt::clear_eol(p);

        match key {
            b'q' | b'Q' | 0x1b | 0x03 => {
                self.stopped = true;
                false
            }
            // Enter advances a single line, so you can creep through slowly.
            b'\r' | b'\n' => {
                self.since_pause = limit.saturating_sub(2);
                true
            }
            _ => {
                self.since_pause = 0;
                true
            }
        }
    }
}

/// Print a block of text through the pager, one line at a time.
///
/// Used for anything already stored as a multi-line string -- help pages, ROM
/// files -- where the caller has no natural per-line loop of its own.
pub fn put_text(p: &mut dyn Platform, pg: &mut Pager, text: impl Chars) {
    put_text_dyn(p, pg, &text);
}

fn put_text_dyn(p: &mut dyn Platform, pg: &mut Pager, text: &dyn Chars) {
    // The text arrives a character at a time, so the newline that ends a line
    // is also what triggers the pause -- there is no list of lines to walk.
    // A message ending in a newline therefore stops there, rather than
    // printing the empty piece after it as a blank line that is not really
    // in the text.
    let mut stopped = false;
    let mut last = b'\n';
    text.each(&mut |b| {
        if stopped {
            return;
        }
        if b == b'\n' {
            kprint!(p, "\n");
            stopped = !pg.line(p);
        } else {
            p.put(b);
        }
        last = b;
    });
    // A last line with no newline of its own still needs one, and still counts
    // towards the next pause.
    if !stopped && last != b'\n' {
        kprint!(p, "\n");
        pg.line(p);
    }
}
