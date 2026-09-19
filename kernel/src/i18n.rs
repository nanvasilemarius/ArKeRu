//! Runtime language selection.
//!
//! The tables come from `i18n/*.json` via `build.rs`, so switching language is
//! a single atomic store. There is no parser and no allocation.
//!
//! The strings are not stored as text. Two languages of prose came to 91 KB,
//! which was 45% of the firmware, so `build.rs` squeezes them with byte-pair
//! encoding into 52 KB and this module expands them again on the way out. A
//! message is therefore not a `&str` and cannot be handed round as one --
//! expanding it needs somewhere to put it, and the longest is eight kilobytes,
//! a quarter of this part's RAM. So [`t`] returns a [`Text`], which is a
//! promise to spell something rather than something already spelled, and the
//! helpers that used to take `&str` take [`Chars`] instead.
//!
//! Messages may contain `{}` placeholders. Rust's `format!` needs a literal
//! format string, so a translated template cannot be handed to `write!`;
//! [`spell`] reports the holes as it goes instead, which is what lets a
//! translator move the value within the sentence.

use crate::{kprint, kprintln, Platform};
use core::sync::atomic::{AtomicU8, Ordering};

include!(concat!(env!("OUT_DIR"), "/strings.rs"));

/// Settings key under which the language index is persisted.
pub const LANG_KEY: &str = "lang";

/// Index into the tables. 0 is always English.
static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn set_lang(idx: u8) {
    if (idx as usize) < LANG_COUNT {
        CURRENT.store(idx, Ordering::Relaxed);
    }
}

pub fn lang_index() -> u8 {
    CURRENT.load(Ordering::Relaxed)
}

pub fn lang_name() -> &'static str {
    LANG_NAMES[lang_index() as usize]
}

/// Resolve a language name such as `"RO"`, case-insensitively.
pub fn lang_by_name(name: &str) -> Option<u8> {
    LANG_NAMES
        .iter()
        .position(|n| {
            n.len() == name.len()
                && n.bytes()
                    .zip(name.bytes())
                    .all(|(a, b)| a.to_ascii_uppercase() == b.to_ascii_uppercase())
        })
        .map(|i| i as u8)
}

/// The active translation of `m`.
pub fn t(m: Msg) -> Text {
    let i = lang_index() as usize * MSG_COUNT + m as usize;
    Text::Packed(&PACKED[SPANS[i] as usize..SPANS[i + 1] as usize])
}

/// Something that can be spelled out as characters.
///
/// Compressed messages cannot travel as `&str`, so the screen, the pager and
/// the terminal helpers take this instead -- and a translated message, a plain
/// literal and a runtime string all go through the same door.
pub trait Chars {
    /// Feed every character to `out`, in order.
    fn each(&self, out: &mut dyn FnMut(u8));

    /// How many characters [`each`](Chars::each) will produce.
    ///
    /// This expands the text a second time rather than storing a length beside
    /// every message; two passes over a label cost less than 2 KB of table.
    ///
    /// Counted in bytes, which is what callers laying out columns want because
    /// both tables are pure ASCII -- `build.rs` derives the alphabet from them
    /// and it came out 90 values, none above `~`. A translation that used
    /// diacritics would still build and still read correctly; its labels would
    /// just measure wide.
    fn len(&self) -> usize {
        let mut n = 0;
        self.each(&mut |_| n += 1);
        n
    }

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The same text without leading or trailing spaces.
    fn trim(self) -> Trimmed<Self>
    where
        Self: Sized,
    {
        Trimmed(self)
    }
}

/// Text with the spaces at either end left out.
///
/// The POST device labels carry the indentation the boot screen wants baked
/// into them, and SETUP lays the same labels out differently. Trailing spaces
/// are held back rather than buffered: a run of them is a count, and it is
/// only spent once a character turns up after it.
pub struct Trimmed<T>(pub T);

impl<T: Chars> Chars for Trimmed<T> {
    fn each(&self, out: &mut dyn FnMut(u8)) {
        let mut started = false;
        let mut held = 0usize;
        self.0.each(&mut |b| {
            if b == b' ' {
                held += usize::from(started);
                return;
            }
            for _ in 0..held {
                out(b' ');
            }
            held = 0;
            started = true;
            out(b);
        });
    }
}

impl Chars for str {
    fn each(&self, out: &mut dyn FnMut(u8)) {
        for b in self.bytes() {
            out(b);
        }
    }

    fn len(&self) -> usize {
        str::len(self)
    }
}

impl<T: Chars + ?Sized> Chars for &T {
    fn each(&self, out: &mut dyn FnMut(u8)) {
        (**self).each(out);
    }

    fn len(&self) -> usize {
        (**self).len()
    }
}

/// Text that has not been spelled out yet.
///
/// `Copy`, and the size of a slice: holding one costs nothing, and nothing
/// happens until something asks for the characters.
#[derive(Clone, Copy)]
pub enum Text {
    /// A span of [`PACKED`], to be expanded through the codebook.
    Packed(&'static [u8]),
    /// Already plain -- a name out of the world tables, or a literal. Lets a
    /// `match` produce translated text on one arm and a proper noun on another.
    Plain(&'static str),
}

impl From<&'static str> for Text {
    fn from(s: &'static str) -> Self {
        Text::Plain(s)
    }
}

impl Chars for Text {
    fn each(&self, out: &mut dyn FnMut(u8)) {
        match self {
            Text::Packed(packed) => expand(packed, out),
            Text::Plain(s) => s.each(out),
        }
    }
}

/// Expand packed text, one character at a time.
///
/// A message is a string of symbols. Anything below [`NLIT`] is a literal and
/// spells itself through [`ALPHABET`]; anything above is a code standing for a
/// pair of symbols, which may be codes in turn -- so one byte can expand to a
/// whole word. Expanding one means walking down the left of that pair while
/// the right-hand halves wait on a stack.
///
/// The stack is [`DEPTH`] deep, which is not a guess: `build.rs` runs this
/// same algorithm over the same table at build time and reports how deep it
/// actually got. Since the runtime only ever decodes those entries, that
/// figure is exact, which is what makes indexing the stack safe to do without
/// a bounds check to fall back on.
///
/// Nothing is buffered. The characters go straight into whatever is consuming
/// them -- a screen cell, the serial port -- so the eight-kilobyte help page
/// never exists all at once anywhere.
fn expand(packed: &[u8], out: &mut dyn FnMut(u8)) {
    let mut stack = [0u8; DEPTH];
    let mut sp = 0usize;
    let mut at = 0usize;
    loop {
        let mut sym = if sp > 0 {
            sp -= 1;
            stack[sp]
        } else if at < packed.len() {
            at += 1;
            packed[at - 1]
        } else {
            return;
        };
        while sym >= NLIT {
            let [a, b] = BOOK[(sym - NLIT) as usize];
            stack[sp] = b;
            sp += 1;
            sym = a;
        }
        out(ALPHABET[sym as usize]);
    }
}

/// A piece of a message being spelled out.
pub enum Piece {
    /// One character of the message itself.
    Ch(u8),
    /// The `n`th `{}` placeholder, counting from zero.
    Hole(usize),
}

/// Spell `text` out, reporting `{}` placeholders instead of printing them.
///
/// What fills a hole is not the message's business -- it might be a number
/// going to the console or a column of digits drawn on a composed screen -- so
/// the reader is handed the hole and decides. Finding the placeholder used to
/// be `str::find`, which needed the whole message in one piece; on a stream it
/// is a one-character lookahead instead.
pub fn spell(text: impl Chars, out: &mut dyn FnMut(Piece)) {
    spell_dyn(&text, out);
}

fn spell_dyn(text: &dyn Chars, out: &mut dyn FnMut(Piece)) {
    let mut open = false;
    let mut holes = 0usize;
    text.each(&mut |b| {
        if open {
            open = false;
            if b == b'}' {
                out(Piece::Hole(holes));
                holes += 1;
                return;
            }
            out(Piece::Ch(b'{'));
        }
        if b == b'{' {
            open = true;
        } else {
            out(Piece::Ch(b));
        }
    });
    if open {
        out(Piece::Ch(b'{'));
    }
}

/// Write one character to the console, giving a bare newline the carriage
/// return a terminal expects.
///
/// [`crate::Con`] does this for formatted output, and everything used to reach
/// the terminal through it. Anything streaming bytes straight past it has to
/// do the same or the output comes out as a staircase, so the rule lives here
/// once rather than being repeated at every caller that now writes bytes.
pub fn put_ch(p: &mut dyn Platform, b: u8) {
    if b == b'\n' {
        p.put(b'\r');
    }
    p.put(b);
}

/// Print any text to the console.
pub fn say(p: &mut dyn Platform, text: impl Chars) {
    say_dyn(p, &text);
}

fn say_dyn(p: &mut dyn Platform, text: &dyn Chars) {
    text.each(&mut |b| put_ch(p, b));
}

/// Print `m`, no placeholder.
pub fn put(p: &mut dyn Platform, m: Msg) {
    say(p, t(m));
}

pub fn putln(p: &mut dyn Platform, m: Msg) {
    put(p, m);
    kprintln!(p);
}

/// Print `m` with its single `{}` replaced by `arg`.
pub fn put1<T: core::fmt::Display>(p: &mut dyn Platform, m: Msg, arg: T) {
    spell(t(m), &mut |piece| match piece {
        Piece::Ch(b) => put_ch(p, b),
        Piece::Hole(_) => kprint!(p, "{}", arg),
    });
}

pub fn putln1<T: core::fmt::Display>(p: &mut dyn Platform, m: Msg, arg: T) {
    put1(p, m, arg);
    kprintln!(p);
}

/// Print `m` with its two `{}` replaced, left to right.
pub fn put2<A: core::fmt::Display, B: core::fmt::Display>(
    p: &mut dyn Platform,
    m: Msg,
    first: A,
    second: B,
) {
    spell(t(m), &mut |piece| match piece {
        Piece::Ch(b) => put_ch(p, b),
        Piece::Hole(0) => kprint!(p, "{}", first),
        Piece::Hole(_) => kprint!(p, "{}", second),
    });
}

impl core::fmt::Display for Text {
    /// Honours width, fill and alignment, which the POST screen's `{:<18}`
    /// columns rely on.
    ///
    /// `Formatter::pad` would do this, but it wants the whole string at once,
    /// which is the one thing a streamed message cannot hand it. Precision is
    /// not supported because nothing formats a message with one.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        use core::fmt::Write as _;
        let pad = f.width().unwrap_or(0).saturating_sub(self.len());
        let (before, after) = match f.align() {
            Some(core::fmt::Alignment::Right) => (pad, 0),
            Some(core::fmt::Alignment::Center) => (pad / 2, pad - pad / 2),
            // Text aligns left by default, as `&str` does.
            _ => (0, pad),
        };
        for _ in 0..before {
            f.write_char(f.fill())?;
        }

        // Chunked so the formatter is not called per character. A chunk is
        // flushed only on a character boundary: the alphabet is derived from
        // the tables, so a translation with diacritics in it would put
        // multi-byte characters through here.
        let mut buf = [0u8; 32];
        let mut n = 0;
        let mut err = Ok(());
        self.each(&mut |b| {
            if err.is_err() {
                return;
            }
            if n + 4 > buf.len() && b & 0xc0 != 0x80 {
                err = flush(f, &buf[..n]);
                n = 0;
            }
            buf[n] = b;
            n += 1;
        });
        err?;
        flush(f, &buf[..n])?;

        for _ in 0..after {
            f.write_char(f.fill())?;
        }
        Ok(())
    }
}

fn flush(f: &mut core::fmt::Formatter<'_>, bytes: &[u8]) -> core::fmt::Result {
    f.write_str(core::str::from_utf8(bytes).unwrap_or("?"))
}
