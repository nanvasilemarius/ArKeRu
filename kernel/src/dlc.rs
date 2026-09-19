//! `DLC` — story loaded from the other end of the wire.
//!
//! # Why this and not a bigger story in the firmware
//!
//! The village and everything in it is compiled in, and that is the right place
//! for the parts the *engine* reads -- items have damage dice, roads have
//! lengths, villagers have stock lists. A chapter of prose has none of that. It
//! is text and branches, it is the part that will change most often, and every
//! kilobyte of it is a kilobyte of the 240 KB that cannot be spent on anything
//! else.
//!
//! So it lives in the **data flash** and arrives over the serial line. Six
//! kilobytes, blocks 0 to 5 -- not block 6, which is the save, and not block 7,
//! which `DF` erases.
//!
//! # XMODEM
//!
//! The transfer is XMODEM-CRC, 128-byte blocks, 1977 vintage. That is not
//! nostalgia. A story pack arriving over a UART needs exactly what XMODEM has:
//! per-block sequence numbers so a dropped block is noticed, a CRC so a flipped
//! bit is noticed, and a retry so either can be fixed without starting again.
//! Writing something ad hoc would have meant inventing all three badly, and any
//! terminal on any operating system can already send it.
//!
//! Note the CRC seed. XMODEM's CRC-16/CCITT starts at **0x0000**; the pack's own
//! integrity check is CCITT-**FALSE**, which starts at 0xFFFF and is what
//! [`crate::rpg::save`] uses. Same polynomial, different initial value, and
//! getting them the wrong way round produces a checksum that is wrong on every
//! single block.
//!
//! # The format
//!
//! A header, a table of fixed-size scene records, and one blob of text that
//! every string points into. Nothing is parsed on the board: a scene is thirty
//! bytes read straight out of flash at a computed offset, and a string is an
//! offset and a length. The readable source is compiled by `akterm --send`,
//! which is where a text format belongs -- on the machine with the memory.

use crate::screen::{self, Attr, Screen};
use crate::setup::{read_key, Key};
use crate::{i18n, kprint, kprintln, vt, Platform};
use crate::i18n::{Chars, Msg};

// -------------------------------------------------------------------- layout

/// Where a pack lives: the bottom six blocks.
pub const PACK_OFF: usize = 0;
pub const PACK_MAX: usize = 6 * 1024;

/// `DK` -- and it must not be `0xFFFF`, which is what erased flash reads.
const MAGIC: u16 = 0x4B44;
const VERSION: u8 = 1;

/// Bytes of header before the scene table.
const HEADER: usize = 12;
/// Bytes in one scene record: two strings and four choices.
const SCENE: usize = 36;
/// Choices a scene can offer.
pub const CHOICES: usize = 4;
/// Scenes a pack can hold. Twenty-four thirty-byte records is 720 bytes of the
/// six thousand, which leaves the rest for what a chapter is actually made of.
pub const MAX_SCENES: usize = 24;

/// A choice that ends the pack rather than leading anywhere.
const END: u8 = 0xff;

/// The longest body and label the player will hold in RAM at once.
///
/// The scene table is *not* read into RAM. One record is thirty bytes fetched
/// at a computed offset when it is needed, which is why a pack can have any
/// number of scenes without the player growing.
const BODY_MAX: usize = 640;
const LABEL_MAX: usize = 56;

// ------------------------------------------------------------------- xmodem

const SOH: u8 = 0x01;
const EOT: u8 = 0x04;
const ACK: u8 = 0x06;
const NAK: u8 = 0x15;
const CAN: u8 = 0x18;
/// The receiver asks for CRC mode by sending `C` rather than `NAK`.
const CRC_REQ: u8 = b'C';
const BLOCK: usize = 128;

/// CRC-16/CCITT with a caller-chosen seed.
///
/// One function, two uses. XMODEM requires a seed of zero; the pack's own check
/// uses `0xFFFF`, the same CCITT-FALSE the save records use. The polynomial is
/// identical and the seed is the entire difference.
fn crc16(seed: u16, data: &[u8]) -> u16 {
    let mut crc = seed;
    for &b in data {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// Read one byte, or give up after `ms`.
fn get_by(p: &mut dyn Platform, ms: u32) -> Option<u8> {
    let deadline = p.uptime_ms() + ms as u64;
    while p.uptime_ms() < deadline {
        if let Some(b) = p.get() {
            return Some(b);
        }
    }
    None
}

/// Why a transfer stopped.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    NoStore,
    Erase,
    Write,
    /// The sender never started, or stopped mid-way.
    Timeout,
    /// The sender cancelled.
    Cancelled,
    /// More than [`PACK_MAX`] bytes arrived.
    TooBig,
    /// It transferred, but it is not a pack.
    NotAPack,
}

impl Fault {
    pub fn msg(self) -> Msg {
        match self {
            Fault::NoStore => Msg::DlcNoStore,
            Fault::Erase => Msg::DlcErase,
            Fault::Write => Msg::DlcWrite,
            Fault::Timeout => Msg::DlcTimeout,
            Fault::Cancelled => Msg::DlcCancelled,
            Fault::TooBig => Msg::DlcTooBig,
            Fault::NotAPack => Msg::DlcNotAPack,
        }
    }
}

/// Receive a pack over XMODEM-CRC and program it into the data flash.
///
/// Blocks are written to flash as they are acknowledged rather than buffered
/// whole: six kilobytes of RAM to hold a pack that is going to flash anyway is
/// six kilobytes this part does not have to spare, and the sender is already
/// waiting on an ACK that arrives when the write finishes.
fn receive(p: &mut dyn Platform) -> Result<usize, Fault> {
    let (size, block) = p.dataflash_info().ok_or(Fault::NoStore)?;
    if size < PACK_OFF + PACK_MAX {
        return Err(Fault::NoStore);
    }
    let mut off = PACK_OFF;
    while off < PACK_OFF + PACK_MAX {
        if !p.dataflash_erase(off) {
            return Err(Fault::Erase);
        }
        off += block;
    }

    let mut buf = [0u8; BLOCK];
    let mut written = 0usize;
    let mut expect = 1u8;
    // Ten `C`s at a second apart. Long enough for a person to start the sender
    // by hand, short enough that a mistyped command does not hang the shell.
    let mut invites = 10u32;

    loop {
        let head = match get_by(p, 1000) {
            Some(b) => b,
            None => {
                if invites == 0 {
                    return Err(Fault::Timeout);
                }
                invites -= 1;
                p.put(CRC_REQ);
                continue;
            }
        };
        // Once a block has arrived the sender is talking, so stop inviting.
        invites = 0;
        match head {
            SOH => {}
            EOT => {
                p.put(ACK);
                return Ok(written);
            }
            CAN => return Err(Fault::Cancelled),
            _ => continue,
        }

        let Some(n) = get_by(p, 1000) else {
            return Err(Fault::Timeout);
        };
        let Some(inv) = get_by(p, 1000) else {
            return Err(Fault::Timeout);
        };
        let mut ok = n == !inv;
        for b in buf.iter_mut() {
            match get_by(p, 1000) {
                Some(v) => *b = v,
                None => return Err(Fault::Timeout),
            }
        }
        let Some(hi) = get_by(p, 1000) else {
            return Err(Fault::Timeout);
        };
        let Some(lo) = get_by(p, 1000) else {
            return Err(Fault::Timeout);
        };
        ok &= crc16(0, &buf) == ((hi as u16) << 8 | lo as u16);

        if !ok {
            p.put(NAK);
            continue;
        }
        // A repeat of the block just taken means our ACK was lost. Acknowledge
        // it again and write nothing -- programming the same flash twice is how
        // a pack ends up as the bitwise AND of itself.
        if n == expect.wrapping_sub(1) {
            p.put(ACK);
            continue;
        }
        if n != expect {
            p.put(NAK);
            continue;
        }
        if written + BLOCK > PACK_MAX {
            p.put(CAN);
            return Err(Fault::TooBig);
        }
        if !p.dataflash_write(PACK_OFF + written, &buf) {
            p.put(CAN);
            return Err(Fault::Write);
        }
        written += BLOCK;
        expect = expect.wrapping_add(1);
        p.put(ACK);
    }
}

// -------------------------------------------------------------------- format

/// An offset and a length into the pack's text blob.
///
/// The length is a `u16`. A byte was the obvious size and it is wrong: a
/// scene of prose is three or four hundred characters, so a 255-byte ceiling
/// caps a chapter at postcards. Two bytes per string costs twenty-four bytes
/// across a whole pack.
#[derive(Clone, Copy)]
struct Str {
    off: u16,
    len: u16,
}

fn u16_at(b: &[u8], i: usize) -> u16 {
    b[i] as u16 | ((b[i + 1] as u16) << 8)
}

fn str_at(b: &[u8], i: usize) -> Str {
    Str {
        off: u16_at(b, i),
        len: u16_at(b, i + 2),
    }
}

/// A pack's header, as read once when it is opened.
#[derive(Clone, Copy)]
pub struct Pack {
    pub scenes: u8,
    pub bytes: u16,
    title: Str,
}

/// Read a string out of the pack into `out`, and hand back what it says.
///
/// Non-ASCII is refused rather than replaced. The screen's cell encoding is
/// ASCII plus a handful of box glyphs, so an accented byte would draw as a
/// question mark; better to have the packer reject it on a machine that can say
/// which line it was on.
fn text<'a>(p: &mut dyn Platform, s: Str, out: &'a mut [u8]) -> &'a str {
    let n = (s.len as usize).min(out.len());
    if n == 0 || !p.dataflash_read(PACK_OFF + s.off as usize, &mut out[..n]) {
        return "";
    }
    for b in out[..n].iter_mut() {
        if !(0x20..0x7f).contains(b) && *b != b'\n' {
            *b = b' ';
        }
    }
    core::str::from_utf8(&out[..n]).unwrap_or("")
}

/// Open whatever is in the flash, if it is a pack.
pub fn open(p: &mut dyn Platform) -> Option<Pack> {
    let mut head = [0u8; HEADER];
    if !p.dataflash_read(PACK_OFF, &mut head) {
        return None;
    }
    if u16_at(&head, 0) != MAGIC || head[2] != VERSION {
        return None;
    }
    let scenes = head[3];
    let stored = u16_at(&head, 4);
    let bytes = u16_at(&head, 6);
    if scenes == 0 || scenes as usize > MAX_SCENES {
        return None;
    }
    if bytes as usize > PACK_MAX || (bytes as usize) < HEADER + scenes as usize * SCENE {
        return None;
    }

    // The CRC covers everything after the header, in chunks, because the whole
    // pack will not fit in RAM and does not need to.
    let mut crc = 0xFFFFu16;
    let mut at = HEADER;
    let mut chunk = [0u8; BLOCK];
    while at < bytes as usize {
        let n = (bytes as usize - at).min(BLOCK);
        if !p.dataflash_read(PACK_OFF + at, &mut chunk[..n]) {
            return None;
        }
        crc = crc16(crc, &chunk[..n]);
        at += n;
    }
    if crc != stored {
        return None;
    }

    Some(Pack {
        scenes,
        bytes,
        title: str_at(&head, 8),
    })
}

pub fn erase(p: &mut dyn Platform) -> bool {
    let Some((_, block)) = p.dataflash_info() else {
        return false;
    };
    let mut off = PACK_OFF;
    while off < PACK_OFF + PACK_MAX {
        if !p.dataflash_erase(off) {
            return false;
        }
        off += block;
    }
    true
}

/// One scene, as thirty bytes straight out of flash.
struct Scene {
    title: Str,
    body: Str,
    label: [Str; CHOICES],
    to: [u8; CHOICES],
    set: [u8; CHOICES],
    need: [u8; CHOICES],
}

fn scene(p: &mut dyn Platform, i: u8) -> Option<Scene> {
    let mut r = [0u8; SCENE];
    if !p.dataflash_read(PACK_OFF + HEADER + i as usize * SCENE, &mut r) {
        return None;
    }
    let mut sc = Scene {
        title: str_at(&r, 0),
        body: str_at(&r, 4),
        label: [Str { off: 0, len: 0 }; CHOICES],
        to: [END; CHOICES],
        set: [0; CHOICES],
        need: [0; CHOICES],
    };
    for c in 0..CHOICES {
        let b = 8 + c * 7;
        sc.label[c] = str_at(&r, b);
        sc.to[c] = r[b + 4];
        sc.set[c] = r[b + 5];
        sc.need[c] = r[b + 6];
    }
    Some(sc)
}

// -------------------------------------------------------------------- player

const TITLE_Y: usize = 1;
const BODY_Y: usize = 4;
const BODY_H: usize = 10;
const LIST_Y: usize = 16;
const STATUS_Y: usize = 23;

/// Walk the pack. `ENTER` or a digit takes a choice; `Q` leaves.
///
/// Flags are eight bits held for the length of the visit and nowhere else. A
/// chapter that remembered its own state between runs would need somewhere to
/// put it, and the one block that is left is the save's.
fn play(p: &mut dyn Platform, pack: &Pack) {
    let mut s = Screen::new();
    let mut body = [0u8; BODY_MAX];
    let mut labels = [[0u8; LABEL_MAX]; CHOICES];
    let mut flags = 0u8;
    let mut at = 0u8;
    let mut sel = 0usize;

    vt::alt_enter(p);
    vt::hide_cursor(p);
    vt::cls_normal(p);

    let mut title_buf = [0u8; 32];
    // Copied once: the pack title is on every frame and re-reading flash for it
    // sixty times a visit would be sixty erasable-memory reads for a string
    // that cannot change.
    let tn = {
        let t = text(p, pack.title, &mut title_buf);
        t.len()
    };

    while at != END && (at as usize) < pack.scenes as usize {
        let Some(sc) = scene(p, at) else { break };

        // Which choices are open. A locked one is not greyed out, it is absent:
        // showing a door you cannot open tells the player they missed something,
        // which is a different game from the one being told.
        let mut open = [0usize; CHOICES];
        let mut n = 0usize;
        for c in 0..CHOICES {
            if sc.label[c].len == 0 {
                continue;
            }
            if sc.need[c] != 0 && flags & sc.need[c] == 0 {
                continue;
            }
            open[n] = c;
            n += 1;
        }
        if sel >= n {
            sel = 0;
        }

        // Lengths are kept rather than the buffers being terminated. `text`
        // fills a prefix and leaves the rest alone, so a short label after a
        // long one would otherwise read the tail of the previous scene's.
        let mut label_len = [0usize; CHOICES];
        for (i, &c) in open.iter().take(n).enumerate() {
            label_len[i] = text(p, sc.label[c], &mut labels[i]).len();
        }

        loop {
            s.clear();
            s.border(Attr::Frame);
            s.rule(2, Attr::Frame);
            s.text(2, TITLE_Y, core::str::from_utf8(&title_buf[..tn]).unwrap_or(""), Attr::Bright);
            {
                let mut tb = [0u8; 32];
                let st = text(p, sc.title, &mut tb);
                let w = st.chars().count();
                if w > 0 {
                    s.text(screen::COLS - 2 - w, TITLE_Y, st, Attr::Accent);
                }
            }
            {
                let b = text(p, sc.body, &mut body);
                s.wrap(2, BODY_Y, 76, BODY_H, b, Attr::Normal);
            }
            s.rule(LIST_Y - 1, Attr::Frame);

            for i in 0..n {
                let y = LIST_Y + i;
                let on = i == sel;
                s.put(2, y, if on { screen::ARROW } else { b' ' }, Attr::Bright);
                s.put(4, y, b'1' + i as u8, if on { Attr::Bright } else { Attr::Frame });
                s.put(5, y, b'.', Attr::Frame);
                let l = core::str::from_utf8(&labels[i][..label_len[i]]).unwrap_or("");
                s.text(7, y, l, if on { Attr::Bright } else { Attr::Normal });
            }
            if n == 0 {
                s.text(7, LIST_Y, i18n::t(Msg::DlcTheEnd), Attr::Accent);
            }

            s.fill(1, STATUS_Y, screen::COLS - 2, 1, b' ', Attr::Normal);
            let keys = i18n::t(Msg::DlcKeys);
            let kw = keys.len();
            s.text((screen::COLS - 2).saturating_sub(kw), STATUS_Y, keys, Attr::Frame);
            s.flush(p);

            match read_key(p) {
                Key::Up if n > 0 => sel = (sel + n - 1) % n,
                Key::Down if n > 0 => sel = (sel + 1) % n,
                Key::Digit(d) if (d as usize) <= n => {
                    sel = d as usize - 1;
                    break;
                }
                Key::Enter if n > 0 => break,
                Key::Enter | Key::Esc => {
                    at = END;
                    break;
                }
                _ => {}
            }
        }

        if at == END {
            break;
        }
        let c = open[sel];
        flags |= sc.set[c];
        at = sc.to[c];
        sel = 0;
    }

    vt::show_cursor(p);
    vt::alt_leave(p);
}

// ------------------------------------------------------------------- command

fn describe(p: &mut dyn Platform) {
    match open(p) {
        Some(pack) => {
            let mut buf = [0u8; 32];
            let t = text(p, pack.title, &mut buf);
            kprintln!(p, "");
            kprint!(p, "  {}{}{}", vt::sgr(vt::FG_BRIGHT), t, vt::NORMAL);
            kprintln!(p, "");
            i18n::put2(p, Msg::DlcLoaded, pack.scenes as usize, pack.bytes as usize);
            kprintln!(p, "");
        }
        None => {
            kprintln!(p, "");
            i18n::put(p, Msg::DlcEmpty);
            kprintln!(p, "");
        }
    }
}

pub fn cmd(p: &mut dyn Platform, args: &str) {
    let arg = args.trim();
    if arg.is_empty() {
        describe(p);
        return;
    }
    if arg.eq_ignore_ascii_case("PLAY") {
        match open(p) {
            Some(pack) => play(p, &pack),
            None => {
                kprintln!(p, "");
                i18n::put(p, Msg::DlcEmpty);
                kprintln!(p, "");
            }
        }
        return;
    }
    if arg.eq_ignore_ascii_case("ERASE") {
        kprintln!(p, "");
        if erase(p) {
            i18n::put(p, Msg::DlcErased);
        } else {
            i18n::put(p, Msg::DlcErase);
        }
        kprintln!(p, "");
        return;
    }
    if arg.eq_ignore_ascii_case("LOAD") {
        kprintln!(p, "");
        i18n::put(p, Msg::DlcSendNow);
        kprintln!(p, "");
        match receive(p) {
            Ok(n) => {
                // The transfer succeeding and the pack being valid are two
                // different claims, and only the second one is worth making.
                match open(p) {
                    Some(pack) => i18n::put2(p, Msg::DlcTook, n, pack.scenes as usize),
                    None => i18n::put(p, Fault::NotAPack.msg()),
                }
            }
            Err(e) => i18n::put(p, e.msg()),
        }
        kprintln!(p, "");
        return;
    }
    kprintln!(p, "");
    i18n::put(p, Msg::DlcUsage);
    kprintln!(p, "");
}
