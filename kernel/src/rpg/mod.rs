//! `RPG` -- **Vadul Alb**. A village, a hero with no memory of arriving, and
//! the road north.
//!
//! # What is here
//!
//! - a hero with twelve stats and derived values borrowed from D&D 5e,
//!   Fallout and Skyrim ([`hero`])
//! - the village, its fourteen named people and the seven things that will
//!   fight you ([`world`])
//! - seven streets and three roads out, each with a name and a length
//!   ([`road`])
//! - a bag with weight in it, and eight people who will trade ([`item`],
//!   [`bag`])
//! - four people who will travel with you, two at a time, each with one
//!   order you give them and one thing they want ([`party`])
//! - six ways to play, from one that never punishes you to one with
//!   thirst and stat-eating poison in it ([`rules`])
//! - a village that learns who you are and forgets again, on Ebbinghaus's
//!   curve ([`mind`])
//! - a battle on a twelve-by-eight grid, which is the LED panel's exact
//!   geometry ([`combat`])
//! - a save in the RA4M1's own data flash ([`save`])
//!
//! Everything draws through [`crate::screen`], which composes a whole 80x25
//! frame and sends only the cells that moved.

pub mod bag;
pub mod clock;
pub mod combat;
pub mod hero;
pub mod item;
pub mod map;
pub mod mind;
pub mod party;
pub mod road;
pub mod rules;
pub mod save;
pub mod world;

use crate::i18n::{self, Chars, Msg, Piece, Text};
use crate::screen::{self, Attr, Screen};
use crate::setup::{read_key, Key};
use crate::{vt, Platform};
use clock::{cost, Clock, Light};
use hero::{Hero, Rng, EYES, STATS, STAT_NAME};
use item::Slot;
use mind::Village;
use party::Party;
use road::{Road, ROADS};
use world::{Role, NPCS, PLACES};

// ------------------------------------------------------------------- layout

const TITLE_Y: usize = 1;
const TEXT_Y: usize = 3;
const TEXT_H: usize = 7;
const TEXT_X: usize = 2;
const TEXT_W: usize = 76;
const MID_Y: usize = 10;
const LIST_Y: usize = 11;
/// Rows 11 to 22, which is every row between the rule and the status bar. The
/// busiest location -- the square, with five ways out, two people, your brother
/// and the four standing verbs -- comes to exactly twelve.
const LIST_MAX: usize = 12;
/// Company, bag, sheet, save and quit are on every village screen, so the
/// lists that grow with the location have to leave room for them. The square
/// -- five roads, two people -- comes to exactly twelve with them.
const RESERVED: usize = 5;
const STATUS_Y: usize = 23;

/// Write a decimal into the frame. Returns the columns used.
///
/// There is no allocator and `Screen` takes cells, not formatted strings.
pub(crate) fn num(s: &mut Screen, x: usize, y: usize, mut v: usize, attr: Attr) -> usize {
    let mut buf = [b'0'; 10];
    let mut n = 0usize;
    if v == 0 {
        n = 1;
    } else {
        while v > 0 && n < buf.len() {
            buf[n] = b'0' + (v % 10) as u8;
            v /= 10;
            n += 1;
        }
    }
    for i in 0..n {
        s.put(x + i, y, buf[n - 1 - i], attr);
    }
    n
}

/// A signed decimal, always with its sign -- `+2`, `-1`, `+0`.
///
/// D&D modifiers are read as signed quantities and a bare `2` beside a stat
/// invites being read as the stat.
fn snum(s: &mut Screen, x: usize, y: usize, v: i32, attr: Attr) -> usize {
    s.put(x, y, if v < 0 { b'-' } else { b'+' }, attr);
    1 + num(s, x + 1, y, v.unsigned_abs() as usize, attr)
}

/// Grams as kilograms to one decimal -- `4.3`. Returns the columns used.
///
/// Items are weighed in grams so an arrow is not free, and read in kilograms
/// because that is what a person carrying them would say. The conversion lives
/// here rather than in the catalogue so there is exactly one place that decides
/// how many decimals a weight has.
pub(crate) fn kg(s: &mut Screen, x: usize, y: usize, grams: u32, attr: Attr) -> usize {
    let mut w = num(s, x, y, (grams / 1000) as usize, attr);
    s.put(x + w, y, b'.', attr);
    w += 1;
    w + num(s, x + w, y, ((grams % 1000) / 100) as usize, attr)
}

fn chrome(s: &mut Screen, title: impl Chars, subtitle: impl Chars) {
    chrome_dyn(s, &title, &subtitle);
}

fn chrome_dyn(s: &mut Screen, title: &dyn Chars, subtitle: &dyn Chars) {
    s.border(Attr::Frame);
    s.rule(2, Attr::Frame);
    s.text(2, TITLE_Y, title, Attr::Bright);
    let w = subtitle.len();
    if w > 0 {
        s.text(screen::COLS - 2 - w, TITLE_Y, subtitle, Attr::Accent);
    }
}

fn keys_line(s: &mut Screen, y: usize, after: usize, m: Msg) {
    let keys = i18n::t(m);
    let w = keys.len();
    let x = (screen::COLS - 2).saturating_sub(w);
    // Dropped rather than squeezed when the left-hand side has grown into the
    // space -- `hungry` appearing pushed the hint through the border. The keys
    // are on every other screen and in HELP; the status is not.
    if x > after + 1 {
        s.text(x, y, keys, Attr::Frame);
    }
}

/// The bottom bar: who you are, how you are, and what you are carrying.
fn status(s: &mut Screen, h: &Hero) {
    s.fill(1, STATUS_Y, screen::COLS - 2, 1, b' ', Attr::Normal);
    let mut x = 2;
    x += s.text(x, STATUS_Y, h.name_str(), Attr::Bright);
    x += 2;
    x += s.text(x, STATUS_Y, i18n::t(Msg::RpgDLevel), Attr::Normal);
    x += 1;
    x += num(s, x, STATUS_Y, h.level as usize, Attr::Accent);
    x += 3;
    s.put(x, STATUS_Y, screen::HEART, Attr::Warn);
    x += 2;
    x += num(s, x, STATUS_Y, h.hp as usize, Attr::Normal);
    x += s.text(x, STATUS_Y, "/", Attr::Frame);
    x += num(s, x, STATUS_Y, h.hp_max() as usize, Attr::Frame);
    x += 3;
    x += num(s, x, STATUS_Y, h.coin as usize, Attr::Accent);
    x += 1;
    x += s.text(x, STATUS_Y, i18n::t(Msg::RpgDCoin), Attr::Normal);
    if h.starving() {
        x += 3;
        x += s.text(x, STATUS_Y, i18n::t(Msg::RpgHungry), Attr::Warn);
    }
    if h.parched() {
        x += 3;
        x += s.text(x, STATUS_Y, i18n::t(Msg::RpgThirsty), Attr::Warn);
    }
    if h.poisoned() {
        x += 3;
        x += s.text(x, STATUS_Y, i18n::t(Msg::RpgDPoison), Attr::Warn);
        x += 1;
        x += num(s, x, STATUS_Y, h.poison as usize, Attr::Warn);
    }
    keys_line(s, STATUS_Y, x, Msg::RpgKeys);
}

/// The date, the time and the light, right-aligned on the title row.
///
/// On the title row rather than the status bar because it belongs to the world
/// rather than to the character -- and because the status bar was already full.
fn draw_clock(s: &mut Screen, c: Clock, place: u8) {
    // Right to left, so the block ends flush and nothing has to be measured
    // twice.
    //
    // The light is the light *here*, not the light the hour implies: standing
    // in the forest at four in the afternoon and reading `dusk` is the game
    // telling you something it would otherwise only tell you by refusing.
    let light = i18n::t(light_at(place, c).msg());
    let day = i18n::t(c.weekday_msg());
    let month = c.month_name();
    let width = day.len()
        + 2
        + month.chars().count()
        + 1
        + 2
        + 3
        + 5
        + 2
        + light.len();
    let y = TITLE_Y;
    let mut x = (screen::COLS - 2).saturating_sub(width);
    // Spaces are written rather than skipped. Skipping leaves whatever the
    // cell already held, which on a rule row is the rule -- `Brumar-4`.
    x += s.text(x, y, day, Attr::Frame);
    x += s.text(x, y, ", ", Attr::Frame);
    x += s.text(x, y, month, Attr::Accent);
    x += s.text(x, y, " ", Attr::Normal);
    x += num(s, x, y, c.day_of_month() as usize, Attr::Accent);
    x += s.text(x, y, "   ", Attr::Normal);
    if c.hour() < 10 {
        x += s.text(x, y, "0", Attr::Bright);
    }
    x += num(s, x, y, c.hour() as usize, Attr::Bright);
    x += s.text(x, y, ":", Attr::Bright);
    if c.minute() < 10 {
        x += s.text(x, y, "0", Attr::Bright);
    }
    x += num(s, x, y, c.minute() as usize, Attr::Bright);
    x += s.text(x, y, "  ", Attr::Normal);
    s.text(x, y, light, Attr::Frame);
}

/// Where a choice leads and how far: the road you would take, and its length.
///
/// A second column rather than more words in the first. "Hanul Cerbului" is the
/// answer to *where*; "Ulita Mare, 8 min" is the answer to *how*, and a player
/// deciding whether there is time before dark needs both without reading a
/// sentence.
const VIA_X: usize = 32;
const FAR_X: usize = 56;

/// One line of a choice list.
#[derive(Clone, Copy)]
struct Line {
    text: Text,
    attr: Attr,
    /// The road taken to get there, and how many minutes it is.
    via: Option<(Text, u32)>,
}

impl Line {
    fn plain(text: impl Into<Text>, attr: Attr) -> Self {
        Self {
            text: text.into(),
            attr,
            via: None,
        }
    }
}

/// Minutes as a person would say them -- `45 min`, `1 h`, `1 h 50`.
fn duration(s: &mut Screen, x: usize, y: usize, minutes: u32, attr: Attr) -> usize {
    let (h, m) = road::split(minutes);
    if h == 0 {
        let w = num(s, x, y, m as usize, attr);
        return w + s.text(x + w, y, " min", attr);
    }
    let mut w = num(s, x, y, h as usize, attr);
    w += s.text(x + w, y, " h", attr);
    if m > 0 {
        w += s.text(x + w, y, " ", attr);
        w += num(s, x + w, y, m as usize, attr);
    }
    w
}

/// A numbered, arrow-navigable list. The one widget the whole game uses.
///
/// Returns the chosen index, or `None` if the player backed out.
fn choose(
    p: &mut dyn Platform,
    s: &mut Screen,
    items: &[Line],
    sel: &mut usize,
    redraw: &mut dyn FnMut(&mut Screen),
) -> Option<usize> {
    match choose_or(p, s, items, sel, redraw, b"") {
        Pick::Item(i) => Some(i),
        Pick::Hot(_) | Pick::Quit => None,
    }
}

/// What a list came back with.
enum Pick {
    Item(usize),
    /// One of the letters the caller said it would also answer to.
    Hot(u8),
    Quit,
}

/// As [`choose`], but also returning any of `hot` that is pressed.
///
/// The village list is twelve entries long and twelve is what fits, so a verb
/// that is not a place to walk to and not a person to talk to has nowhere to go
/// in it. `MAP` is one of those -- it costs no time and changes nothing, so it
/// was never really an item on a list of things to do.
fn choose_or(
    p: &mut dyn Platform,
    s: &mut Screen,
    items: &[Line],
    sel: &mut usize,
    redraw: &mut dyn FnMut(&mut Screen),
    hot: &[u8],
) -> Pick {
    let n = items.len().min(LIST_MAX);
    if n == 0 {
        return Pick::Quit;
    }
    loop {
        redraw(s);
        for (i, line) in items.iter().take(n).enumerate() {
            let y = LIST_Y + i;
            let on = i == *sel;
            let attr = line.attr;
            s.put(2, y, if on { screen::ARROW } else { b' ' }, Attr::Bright);
            if i < 9 {
                s.put(4, y, b'1' + i as u8, if on { Attr::Bright } else { attr });
                s.put(5, y, b'.', if on { Attr::Bright } else { attr });
            }
            s.text(7, y, line.text, if on { Attr::Bright } else { attr });
            if let Some((road, minutes)) = line.via {
                let dim = if on { Attr::Accent } else { Attr::Frame };
                s.text(VIA_X, y, road, dim);
                // Zero means the column is being used for a place rather than a
                // journey -- where a companion is waiting -- and `0 min` beside
                // a name reads as a distance of nothing rather than as no
                // distance at all.
                if minutes > 0 {
                    duration(s, FAR_X, y, minutes, dim);
                }
            }
        }
        s.flush(p);

        match read_key(p) {
            Key::Up => *sel = (*sel + n - 1) % n,
            Key::Down => *sel = (*sel + 1) % n,
            Key::Digit(d) if (d as usize) <= n => return Pick::Item(d as usize - 1),
            Key::Enter => return Pick::Item(*sel),
            Key::Esc => return Pick::Quit,
            Key::Char(c) if hot.contains(&c) => return Pick::Hot(c),
            _ => {}
        }
    }
}

// -------------------------------------------------------------- name entry

/// Read the hero's name. Raw bytes rather than [`read_key`], because a name
/// contains the letters that decoder spends on menu shortcuts.
///
/// # Why this one screen does not recompose
///
/// Every other screen here rebuilds its whole frame each time round the loop,
/// which is what the diffing renderer is for and costs nothing on the wire.
/// This one cannot afford to, because of the receiver underneath it.
///
/// **The console has one byte of hardware buffer and is polled, not
/// interrupt-driven.** `Platform::get` and `Platform::put` are the only things
/// that move a byte out of the SCI's receive register, so any stretch of code
/// longer than one character time -- 87 us at 115200 baud -- can lose input.
/// Rebuilding the frame is a few hundred microseconds of laying out cells with
/// no I/O in it at all, which is several character times, and typing at speed
/// into it dropped letters: `Vlad` arrived as `Vl`.
///
/// So the fixed parts are drawn once, and a keystroke touches the twelve cells
/// of the field. Every pending byte is also consumed before redrawing, so a
/// burst -- a fast typist, or a paste -- collapses into one update instead of
/// one per character.
///
/// The real fix is to route the SCI receive interrupt through the ICU and fill
/// the ring from a handler. Nothing in this firmware uses a peripheral
/// interrupt yet, so that is a change worth making deliberately rather than at
/// the end of an afternoon.
fn ask_name(p: &mut dyn Platform, s: &mut Screen, h: &mut Hero) -> bool {
    const FIELD_Y: usize = LIST_Y + 1;

    s.clear();
    chrome(s, i18n::t(Msg::RpgTitle), i18n::t(Msg::RpgNameTitle));
    s.wrap(TEXT_X, TEXT_Y, TEXT_W, TEXT_H, i18n::t(Msg::RpgNameAsk), Attr::Normal);
    s.rule(MID_Y, Attr::Frame);
    let x = 4 + s.text(4, FIELD_Y, i18n::t(Msg::RpgNameLabel), Attr::Accent) + 1;
    s.fill(1, STATUS_Y, screen::COLS - 2, 1, b' ', Attr::Normal);
    keys_line(s, STATUS_Y, 1, Msg::RpgNameKeys);

    let mut buf = [0u8; hero::NAME_MAX];
    let mut len = 0usize;

    loop {
        for i in 0..hero::NAME_MAX {
            let (ch, attr) = if i < len {
                (buf[i], Attr::Bright)
            } else {
                (b'_', Attr::Frame)
            };
            s.put(x + i, FIELD_Y, ch, attr);
        }
        s.flush(p);

        // Block for the first byte, then take whatever else is already here.
        let mut b = loop {
            if let Some(b) = p.get() {
                break b;
            }
        };
        loop {
            match b {
                b'\r' | b'\n' if len > 0 => {
                    h.set_name(&buf[..len]);
                    return true;
                }
                0x08 | 0x7f => len = len.saturating_sub(1),
                0x1b | 0x03 => return false,
                // Printable ASCII only. The cell encoding in `screen` is ASCII
                // plus a handful of box glyphs, and the LED font is ASCII, so
                // an accented name would draw as question marks on both. Better
                // to decline the character than to accept it and mangle it.
                0x20..=0x7e if len < hero::NAME_MAX => {
                    buf[len] = b;
                    len += 1;
                }
                _ => {}
            }
            match p.get() {
                Some(next) => b = next,
                None => break,
            }
        }
    }
}

// ------------------------------------------------------------ character sheet

fn sheet(p: &mut dyn Platform, s: &mut Screen, h: &Hero, party: &Party) -> Key {
    s.clear();
    chrome(s, i18n::t(Msg::RpgTitle), i18n::t(Msg::RpgSheetTitle));

    let mut y = 3;
    // The sheet is shown before the name is asked for, so the roll can be
    // judged on the numbers. An empty first field reads as a rendering fault
    // rather than as a question that has not been put yet.
    let named = 2 + if h.name_len == 0 {
        s.text(2, y, i18n::t(Msg::RpgUnnamed), Attr::Bright)
    } else {
        s.text(2, y, h.name_str(), Attr::Bright)
    };
    let mut x = named + 2;
    x += s.text(x, y, i18n::t(Msg::RpgDLevel), Attr::Normal) + 1;
    x += num(s, x, y, h.level as usize, Attr::Accent) + 2;
    x += s.text(x, y, i18n::t(Msg::RpgDXp), Attr::Normal) + 1;
    x += num(s, x, y, h.xp as usize, Attr::Accent);
    x += s.text(x, y, "/", Attr::Frame);
    num(s, x, y, h.xp_needed() as usize, Attr::Frame);

    y += 1;
    x = 2 + s.text(2, y, i18n::t(Msg::RpgDAge), Attr::Normal) + 1;
    x += num(s, x, y, h.age as usize, Attr::Normal) + 3;
    x += s.text(x, y, i18n::t(Msg::RpgDHeight), Attr::Normal) + 1;
    x += num(s, x, y, 100 + h.height as usize, Attr::Normal);
    x += s.text(x, y, " cm", Attr::Normal) + 3;
    x += s.text(x, y, i18n::t(Msg::RpgDWeight), Attr::Normal) + 1;
    x += num(s, x, y, h.weight as usize, Attr::Normal);
    x += s.text(x, y, " kg", Attr::Normal) + 3;
    x += s.text(x, y, i18n::t(Msg::RpgDEyes), Attr::Normal) + 1;
    s.text(x, y, i18n::t(EYES[h.eyes as usize]), Attr::Accent);

    // Twelve stats in two columns of six, each with its D&D modifier beside it
    // -- the modifier is what every roll actually uses, so showing the score
    // alone would be showing the input and hiding the number that matters.
    for i in 0..STATS {
        let col = if i < 6 { 2usize } else { 40 };
        let row = 6 + (i % 6);
        let name = i18n::t(STAT_NAME[i]);
        let mut cx = col + s.text(col, row, name, Attr::Normal);
        while cx < col + 20 {
            s.put(cx, row, b'.', Attr::Frame);
            cx += 1;
        }
        cx += 1;
        cx += num(s, cx, row, h.stats[i] as usize, Attr::Bright);
        cx += 2;
        snum(s, cx, row, h.modifier(i), Attr::Accent);
    }

    // Eight derived values in two columns of four. Attack and load earn their
    // place because both now move with what is in the bag: the sheet has to be
    // where you find out that the mail shirt cost you a square of movement.
    s.rule(12, Attr::Frame);
    let derived: [(Msg, usize); 8] = [
        (Msg::RpgDHp, h.hp_max() as usize),
        (Msg::RpgDAc, h.armour() as usize),
        (Msg::RpgDAttack, 0),
        (Msg::RpgDSpeed, h.speed() as usize),
        (Msg::RpgDCarry, h.carry_kg() as usize),
        (Msg::RpgDPerc, h.perception() as usize),
        (Msg::RpgDStam, h.stamina() as usize),
        (Msg::RpgDLoad, 0),
    ];
    for (i, (m, v)) in derived.iter().enumerate() {
        let col = if i < 4 { 2usize } else { 40 };
        let row = 13 + (i % 4);
        let mut cx = col + s.text(col, row, i18n::t(*m), Attr::Normal);
        while cx < col + 20 {
            s.put(cx, row, b'.', Attr::Frame);
            cx += 1;
        }
        cx += 1;
        match *m {
            Msg::RpgDLoad => {
                let attr = if h.over_laden() { Attr::Warn } else { Attr::Bright };
                kg(s, cx, row, h.load_g(), attr);
            }
            // Signed, because a weak hero's attack bonus really is negative and
            // clamping it to zero would flatter the sheet into a lie.
            Msg::RpgDAttack => {
                snum(s, cx, row, h.strike().1, Attr::Bright);
            }
            _ => {
                num(s, cx, row, *v, Attr::Bright);
            }
        }
    }

    // What is in hand and on the body, named. A player who has to open the bag
    // to find out what they are holding is a player who will walk up the road
    // holding a torch.
    let mut cx = 2 + s.text(2, 17, i18n::t(Msg::RpgDGear), Attr::Normal) + 1;
    let mut worn = 0usize;
    for slot in [Slot::Weapon, Slot::Body, Slot::Hand] {
        let held = h.gear.in_slot(slot);
        if item::valid(held) {
            cx += s.text(cx, 17, i18n::t(item::name(held)), Attr::Accent) + 2;
            worn += 1;
        }
    }
    if worn == 0 {
        s.text(cx, 17, i18n::t(Msg::RpgNothing), Attr::Frame);
    }

    // Which of the six, on the one screen that is a record of the character
    // rather than of the moment. It cannot be changed, so it belongs beside
    // the things that also cannot.
    let mut rx = 2 + s.text(2, 18, i18n::t(Msg::RpgDRules), Attr::Normal) + 1;
    rx += s.text(rx, 18, i18n::t(h.rules().name), Attr::Accent) + 3;
    if h.oath > 0 {
        let mut cx = rx + s.text(rx, 18, i18n::t(Msg::RpgDOath), Attr::Normal) + 1;
        cx += s.text(cx, 18, i18n::t(OATH_NAME[h.oath as usize - 1]), Attr::Accent) + 2;
        s.text(cx, 18, i18n::t(oath_state_msg(h.oath_state)), Attr::Warn);
    }

    s.rule(19, Attr::Frame);

    // The party, because a companion is part of the character now rather than
    // an inventory item.
    let mut cy = 20;
    for (kind, m) in party.travelling() {
        let mut cx = 2 + s.text(2, cy, party::Member::spec(kind).name, Attr::Accent) + 2;
        cx += s.text(cx, cy, i18n::t(Msg::RpgDHp), Attr::Normal) + 1;
        cx += num(s, cx, cy, m.hp as usize, Attr::Bright);
        cx += s.text(cx, cy, "/", Attr::Frame);
        cx += num(s, cx, cy, m.hp_max(kind) as usize, Attr::Frame) + 3;
        cx += s.text(cx, cy, i18n::t(Msg::RpgDMorale), Attr::Normal) + 1;
        cx += num(s, cx, cy, m.morale as usize, Attr::Bright) + 3;
        // What you last told them to do. It is the only order in the game and
        // the sheet is where you check it without opening four screens.
        cx += s.text(cx, cy, i18n::t(party::STANCE_NAME[m.stance as usize]), Attr::Frame) + 3;
        if m.hunger >= party::HUNGER_HUNGRY {
            s.text(cx, cy, i18n::t(Msg::RpgHungry), Attr::Warn);
        }
        cy += 1;
    }
    s.fill(1, STATUS_Y, screen::COLS - 2, 1, b' ', Attr::Normal);
    keys_line(s, STATUS_Y, 1, Msg::RpgSheetKeys);
    s.flush(p);
    read_key(p)
}

// ------------------------------------------------------------- point buy

/// Spend the point pool across the twelve stats.
///
/// Up and down pick a stat, left and right move it. The remaining pool is on
/// screen at all times, because the decision being made is not "is 14 good" but
/// "is 14 here worth 11 somewhere else".
fn allocate(p: &mut dyn Platform, s: &mut Screen, h: &mut Hero) -> bool {
    let mut sel = 0usize;
    loop {
        s.clear();
        chrome(s, i18n::t(Msg::RpgTitle), i18n::t(Msg::RpgBuyTitle));

        let mut x = 2 + s.text(2, TEXT_Y, i18n::t(Msg::RpgBuyPoints), Attr::Normal) + 1;
        x += num(s, x, TEXT_Y, h.points_left() as usize, Attr::Bright);
        x += 2;
        x += s.text(x, TEXT_Y, "/", Attr::Frame);
        num(s, x, TEXT_Y, hero::POINTS as usize, Attr::Frame);
        s.wrap(2, TEXT_Y + 1, TEXT_W, 2, i18n::t(Msg::RpgBuyHint), Attr::Frame);

        for i in 0..STATS {
            let y = 6 + i;
            let on = i == sel;
            s.put(2, y, if on { screen::ARROW } else { b' ' }, Attr::Bright);
            let attr = if on { Attr::Bright } else { Attr::Normal };
            let name = i18n::t(STAT_NAME[i]);
            let mut cx = 4 + s.text(4, y, name, attr);
            while cx < 26 {
                s.put(cx, y, b'.', Attr::Frame);
                cx += 1;
            }
            cx += 1;
            cx += num(s, cx, y, h.stats[i] as usize, if on { Attr::Accent } else { attr });
            cx += 2;
            snum(s, cx, y, h.modifier(i), Attr::Accent);
        }

        s.fill(1, STATUS_Y, screen::COLS - 2, 1, b' ', Attr::Normal);
        keys_line(s, STATUS_Y, 1, Msg::RpgBuyKeys);
        s.flush(p);

        match read_key(p) {
            Key::Up => sel = (sel + STATS - 1) % STATS,
            Key::Down => sel = (sel + 1) % STATS,
            Key::Right => {
                h.adjust(sel, true);
            }
            Key::Left => {
                h.adjust(sel, false);
            }
            // Accepting with points unspent is allowed. Refusing would turn a
            // budget into a chore, and somebody who wants an ordinary person
            // should be able to have one.
            Key::Enter => return true,
            Key::Esc => return false,
            _ => {}
        }
    }
}

/// Which of the six ways to play. Asked first, before anything else about the
/// character, because it is the only choice here that cannot be changed later.
///
/// Each line carries its own sentence. A difficulty whose effect you find out
/// by playing it is a coin toss, and the difference between `Very Hard` and
/// `Realistic` in particular is not a difference of degree.
fn difficulty(p: &mut dyn Platform, s: &mut Screen, h: &mut Hero) -> bool {
    let mut sel = h.rules as usize;
    loop {
        s.clear();
        chrome(s, i18n::t(Msg::RpgTitle), i18n::t(Msg::RpgDiffTitle));
        s.wrap(TEXT_X, TEXT_Y, TEXT_W, 2, i18n::t(Msg::RpgDiffBody), Attr::Normal);
        s.rule(5, Attr::Frame);

        // Six lines from row 6, then ten rows for the description. Realistic
        // has the most to say and needs every one of them: it is the setting
        // that turns systems on rather than moving numbers.
        for i in 0..rules::LEVELS as usize {
            let y = 6 + i;
            let on = i == sel;
            s.put(2, y, if on { screen::ARROW } else { b' ' }, Attr::Bright);
            let attr = if on { Attr::Bright } else { Attr::Normal };
            s.put(4, y, b'1' + i as u8, if on { Attr::Bright } else { Attr::Frame });
            s.put(5, y, b'.', Attr::Frame);
            s.text(7, y, i18n::t(rules::RULES[i].name), attr);
        }

        s.rule(12, Attr::Frame);
        s.wrap(TEXT_X, 13, TEXT_W, 10, i18n::t(rules::RULES[sel].about), Attr::Accent);

        s.fill(1, STATUS_Y, screen::COLS - 2, 1, b' ', Attr::Normal);
        keys_line(s, STATUS_Y, 1, Msg::RpgBuyKeys);
        s.flush(p);

        match read_key(p) {
            Key::Up => sel = (sel + rules::LEVELS as usize - 1) % rules::LEVELS as usize,
            Key::Down => sel = (sel + 1) % rules::LEVELS as usize,
            Key::Digit(d) if (d as usize) <= rules::LEVELS as usize => sel = d as usize - 1,
            Key::Enter => {
                h.rules = sel as u8;
                return true;
            }
            Key::Esc => return false,
            _ => {}
        }
    }
}

/// Gender, age and eyes. Three fields, one screen, before any numbers.
fn identity(p: &mut dyn Platform, s: &mut Screen, h: &mut Hero) -> bool {
    let mut sel = 0usize;
    loop {
        s.clear();
        chrome(s, i18n::t(Msg::RpgTitle), i18n::t(Msg::RpgWhoTitle));
        s.wrap(TEXT_X, TEXT_Y, TEXT_W, TEXT_H, i18n::t(Msg::RpgWhoBody), Attr::Normal);
        s.rule(MID_Y, Attr::Frame);

        let rows: [(Msg, usize); 3] = [
            (Msg::RpgDGender, 0),
            (Msg::RpgDAge, 1),
            (Msg::RpgDEyes, 2),
        ];
        for (i, (label, _)) in rows.iter().enumerate() {
            let y = LIST_Y + i;
            let on = i == sel;
            s.put(2, y, if on { screen::ARROW } else { b' ' }, Attr::Bright);
            let attr = if on { Attr::Bright } else { Attr::Normal };
            let mut cx = 4 + s.text(4, y, i18n::t(*label), attr);
            while cx < 22 {
                s.put(cx, y, b'.', Attr::Frame);
                cx += 1;
            }
            cx += 1;
            match i {
                0 => {
                    let g = if h.gender == hero::GENDER_HE {
                        Msg::RpgGenderHe
                    } else {
                        Msg::RpgGenderShe
                    };
                    s.text(cx, y, i18n::t(g), Attr::Accent);
                }
                1 => {
                    num(s, cx, y, h.age as usize, Attr::Accent);
                }
                _ => {
                    s.text(cx, y, i18n::t(EYES[h.eyes as usize]), Attr::Accent);
                }
            }
        }

        s.fill(1, STATUS_Y, screen::COLS - 2, 1, b' ', Attr::Normal);
        keys_line(s, STATUS_Y, 1, Msg::RpgBuyKeys);
        s.flush(p);

        let step = match read_key(p) {
            Key::Up => {
                sel = (sel + 2) % 3;
                0
            }
            Key::Down => {
                sel = (sel + 1) % 3;
                0
            }
            Key::Right => 1,
            Key::Left => -1,
            Key::Enter => return true,
            Key::Esc => return false,
            _ => 0,
        };
        if step != 0 {
            match sel {
                0 => h.gender ^= 1,
                1 => h.age = (h.age as i32 + step).clamp(16, 60) as u8,
                _ => {
                    let n = EYES.len() as i32;
                    h.eyes = ((h.eyes as i32 + step + n) % n) as u8;
                }
            }
        }
    }
}

// -------------------------------------------------------------------- oath
//
// Kept from the previous slice because the mechanic was worth keeping and
// costs two bytes: an oath is a rule the engine checks, not a stat. It is now
// sworn to Capitan Kondos in the training yard rather than at a burning
// chapter house, which is a better place for it -- you swear it to somebody
// who will be there afterwards to see whether you kept it.

static OATH_NAME: [Msg; 4] = [
    Msg::RpgOathShield,
    Msg::RpgOathTruth,
    Msg::RpgOathHand,
    Msg::RpgOathBlood,
];
static OATH_VOW: [Msg; 4] = [
    Msg::RpgVowShield,
    Msg::RpgVowTruth,
    Msg::RpgVowHand,
    Msg::RpgVowBlood,
];

fn oath_state_msg(state: u8) -> Msg {
    match state {
        0 => Msg::RpgStateKept,
        1 => Msg::RpgStateStrained,
        _ => Msg::RpgStateBroken,
    }
}

fn swear(p: &mut dyn Platform, s: &mut Screen, h: &mut Hero) {
    let mut sel = 0usize;
    let items: [Line; 4] = [
        Line::plain(i18n::t(OATH_VOW[0]), Attr::Normal),
        Line::plain(i18n::t(OATH_VOW[1]), Attr::Normal),
        Line::plain(i18n::t(OATH_VOW[2]), Attr::Normal),
        Line::plain(i18n::t(OATH_VOW[3]), Attr::Normal),
    ];
    let mut redraw = |s: &mut Screen| {
        s.clear();
        chrome(s, i18n::t(Msg::RpgTitle), i18n::t(Msg::RpgSwearTitle));
        s.wrap(TEXT_X, TEXT_Y, TEXT_W, TEXT_H, i18n::t(Msg::RpgSwearBody), Attr::Normal);
        s.rule(MID_Y, Attr::Frame);
        s.fill(1, STATUS_Y, screen::COLS - 2, 1, b' ', Attr::Normal);
        keys_line(s, STATUS_Y, 1, Msg::RpgKeys);
    };
    if let Some(i) = choose(p, s, &items, &mut sel, &mut redraw) {
        h.oath = i as u8 + 1;
        h.oath_state = 0;
    }
}

// ------------------------------------------------------------------ notices

/// A single line of consequence, held until a key is pressed.
fn notice(p: &mut dyn Platform, s: &mut Screen, h: &Hero, title: Msg, body: Msg, n: Option<usize>) {
    s.clear();
    chrome(s, i18n::t(Msg::RpgTitle), i18n::t(title));
    let text = i18n::t(body);
    // Whether there is a hole to fill has to be asked before anything is drawn,
    // and a message can only be read forwards, so it is asked by reading it
    // once with nothing on the other end. Cheaper than it sounds, and it keeps
    // the number in `Attr::Bright` -- which wrapping the whole line would lose.
    let mut holed = false;
    i18n::spell(text, &mut |piece| holed |= matches!(piece, Piece::Hole(_)));
    match (holed, n) {
        (true, Some(v)) => {
            let mut x = TEXT_X;
            i18n::spell(text, &mut |piece| match piece {
                Piece::Ch(c) => {
                    if x < screen::COLS {
                        s.put(x, TEXT_Y, screen::code(c as char), Attr::Normal);
                        x += 1;
                    }
                }
                Piece::Hole(_) => x += num(s, x, TEXT_Y, v, Attr::Bright),
            });
        }
        _ => {
            s.wrap(TEXT_X, TEXT_Y, TEXT_W, TEXT_H, text, Attr::Normal);
        }
    }
    s.rule(MID_Y, Attr::Frame);
    s.text(4, LIST_Y + 1, i18n::t(Msg::RpgAnyKey), Attr::Accent);
    status(s, h);
    s.flush(p);
    let _ = read_key(p);
}

// --------------------------------------------------------------- companions

/// Whether the hero has done whatever this person wants doing first.
fn wanted(h: &Hero, want: party::Want) -> bool {
    match want {
        party::Want::Nothing => true,
        party::Want::Deed(bit) => h.flags[0] & bit != 0,
        // The oath has existed since 0.20.0 and until now nothing read it.
        party::Want::Oath => h.oath > 0,
    }
}

/// Which companion, if any, this villager is.
fn companion_of(npc: u8) -> Option<usize> {
    party::KINDS.iter().position(|k| k.npc == npc)
}

/// Ask somebody to come along. `J` from their conversation.
fn ask_along(p: &mut dyn Platform, s: &mut Screen, h: &mut Hero, party: &mut Party, kind: usize) {
    let spec = party::Member::spec(kind);
    if party.full() {
        notice(p, s, h, Msg::RpgAskTitle, Msg::RpgPartyFull, None);
        return;
    }
    if !wanted(h, spec.want) {
        // Their own words, not a refusal message. The reason they will not come
        // is a thing about them, and it is also the instruction for how to fix
        // it -- Costache says what is taking the sheep, and that is the task.
        notice(p, s, h, Msg::RpgAskTitle, spec.refuse, None);
        return;
    }
    party.members[kind].recruit(kind);
    notice(p, s, h, Msg::RpgAskTitle, spec.join, None);
}

/// What a line on the companion screen does.
#[derive(Clone, Copy)]
enum Deal {
    Stance(u8),
    Wait,
    Follow,
    Accept,
    Back,
}

/// What they say, which depends on how they are rather than on who they are.
fn companion_line(h: &Hero, m: &party::Member, kind: usize) -> Msg {
    let spec = party::Member::spec(kind);
    if m.hunger >= party::HUNGER_HUNGRY {
        Msg::RpgSayHungry
    } else if !m.will_engage() {
        Msg::RpgSayShaken
    } else if m.quest == party::QUEST_DONE {
        spec.thanks
    } else if m.quest == party::QUEST_ASKED || m.morale >= party::MORALE_ASKS {
        spec.ask
    } else if kind == party::C_BROTHER {
        // He is the only one who has a word for you that changes with who you
        // decided to be.
        if h.gender == hero::GENDER_HE {
            Msg::RpgSayBrotherHe
        } else {
            Msg::RpgSayBrotherShe
        }
    } else {
        spec.line
    }
}

/// Health, morale, hunger and stance on one row.
fn companion_state(s: &mut Screen, m: &party::Member, kind: usize, y: usize) {
    let spec = party::Member::spec(kind);
    let mut x = 2;
    s.put(x, y, screen::HEART, Attr::Warn);
    x += 2;
    x += num(s, x, y, m.hp as usize, Attr::Normal);
    x += s.text(x, y, "/", Attr::Frame);
    x += num(s, x, y, m.hp_max(kind) as usize, Attr::Frame) + 3;
    x += s.text(x, y, i18n::t(Msg::RpgDMorale), Attr::Normal) + 1;
    x += num(s, x, y, m.morale as usize, Attr::Bright) + 3;
    if m.hunger >= party::HUNGER_HUNGRY {
        x += s.text(x, y, i18n::t(Msg::RpgHungry), Attr::Warn) + 3;
    }
    if m.present {
        x += s.text(x, y, i18n::t(Msg::RpgDStance), Attr::Normal) + 1;
        x += s.text(x, y, i18n::t(party::STANCE_NAME[m.stance as usize]), Attr::Accent) + 3;
    }
    // Where they want taking, once they have asked and you have agreed.
    if m.quest == party::QUEST_ASKED {
        x += s.text(x, y, i18n::t(Msg::RpgDWants), Attr::Normal) + 1;
        s.text(x, y, PLACES[spec.quest_at as usize].name, Attr::Warn);
    }
}

/// Who they are, how they are, and the one thing you tell them to do.
fn companion(
    p: &mut dyn Platform,
    s: &mut Screen,
    h: &mut Hero,
    party: &mut Party,
    kind: usize,
    c: Clock,
) {
    let mut sel = 0usize;
    loop {
        let m = party.members[kind];
        let spec = party::Member::spec(kind);

        let mut acts = [Deal::Back; LIST_MAX];
        let mut items = [Line::plain("", Attr::Normal); LIST_MAX];
        let mut n = 0usize;

        if m.present {
            // The stances, with the current one lit. Three lines rather than a
            // toggle, because "hold" and "guard" fail in opposite directions and
            // a player choosing between them should see both.
            for st in 0..party::STANCES {
                acts[n] = Deal::Stance(st);
                items[n] = Line::plain(
                    i18n::t(party::STANCE_NAME[st as usize]),
                    if st == m.stance {
                        Attr::Bright
                    } else {
                        Attr::Normal
                    },
                );
                n += 1;
            }
            if m.quest == party::QUEST_NONE && m.morale >= party::MORALE_ASKS {
                acts[n] = Deal::Accept;
                items[n] = Line::plain(i18n::t(Msg::RpgActAccept), Attr::Accent);
                n += 1;
            }
            acts[n] = Deal::Wait;
            items[n] = Line::plain(i18n::t(Msg::RpgActWait), Attr::Warn);
            n += 1;
        } else if m.at == h.place {
            // Only where they are standing. Whistling somebody up from two
            // hours away would make "wait here" free, and the point of leaving
            // a person somewhere is that you have to go back for them.
            acts[n] = Deal::Follow;
            items[n] = Line::plain(i18n::t(Msg::RpgActFollow), Attr::Bright);
            n += 1;
        }
        acts[n] = Deal::Back;
        items[n] = Line::plain(i18n::t(Msg::RpgActBack), Attr::Frame);
        n += 1;

        if sel >= n {
            sel = 0;
        }

        let text = i18n::t(companion_line(h, &m, kind));
        let hero_snapshot = *h;
        let mut redraw = |s: &mut Screen| {
            s.clear();
            // No subtitle: the clock owns the right-hand end of the title row,
            // and a role written there is a role the clock half-erases.
            chrome(s, spec.name, "");
            draw_clock(s, c, hero_snapshot.place);
            s.wrap(TEXT_X, TEXT_Y, TEXT_W, TEXT_H - 1, text, Attr::Normal);
            companion_state(s, &m, kind, MID_Y - 1);
            s.rule(MID_Y, Attr::Frame);
            status(s, &hero_snapshot);
        };

        let Some(i) = choose(p, s, &items[..n], &mut sel, &mut redraw) else {
            return;
        };

        match acts[i] {
            Deal::Stance(st) => party.members[kind].stance = st,
            Deal::Accept => {
                party.members[kind].quest = party::QUEST_ASKED;
                s.invalidate();
                // Agreeing while already standing where they wanted taking
                // would otherwise leave the task open until you walked away and
                // came back, which is a rule nobody would guess.
                let here = arrived_for(party, h.place);
                let body = if here.is_some() { spec.thanks } else { spec.ask };
                notice(p, s, h, Msg::RpgAskTitle, body, None);
                s.invalidate();
            }
            // Left where you are standing, not sent home. Somebody told to wait
            // at the bridge waits at the bridge, which is a thing you can get
            // wrong and then have to walk back for.
            Deal::Wait => {
                party.members[kind].present = false;
                party.members[kind].at = h.place;
                return;
            }
            Deal::Follow => {
                if party.full() {
                    s.invalidate();
                    notice(p, s, h, Msg::RpgAskTitle, Msg::RpgPartyFull, None);
                    s.invalidate();
                } else {
                    party.members[kind].present = true;
                    return;
                }
            }
            Deal::Back => return,
        }
    }
}

/// Everybody you know, where they are, and a way into each of them.
///
/// One village line instead of one per companion. With four of them the square
/// -- five roads, two people, four standing verbs -- ran out of rows, and the
/// guard that kept the list inside twelve was dropping companions off the end.
fn company(p: &mut dyn Platform, s: &mut Screen, h: &mut Hero, party: &mut Party, c: Clock) {
    let mut sel = 0usize;
    loop {
        let mut who = [0usize; party::ROSTER];
        let mut items = [Line::plain("", Attr::Normal); LIST_MAX];
        let mut n = 0usize;

        for (kind, m) in party.members.iter().enumerate() {
            if !m.known || n >= LIST_MAX - 1 {
                continue;
            }
            who[n] = kind;
            items[n] = Line {
                text: Text::Plain(party::Member::spec(kind).name),
                attr: if m.present {
                    Attr::Bright
                } else if m.at == h.place {
                    Attr::Accent
                } else {
                    Attr::Frame
                },
                // The second column, which the roads taught the eye to read as
                // "where", says where somebody you left behind is standing.
                via: if m.present {
                    None
                } else {
                    Some((Text::Plain(PLACES[m.at as usize].name), 0))
                },
            };
            n += 1;
        }
        items[n] = Line::plain(i18n::t(Msg::RpgActBack), Attr::Frame);
        let back = n;
        n += 1;
        if sel >= n {
            sel = 0;
        }

        let hero_snapshot = *h;
        let travelling = party.count();
        let mut redraw = |s: &mut Screen| {
            s.clear();
            chrome(s, i18n::t(Msg::RpgCompanyTitle), "");
            draw_clock(s, c, hero_snapshot.place);
            s.wrap(TEXT_X, TEXT_Y, TEXT_W, 2, i18n::t(Msg::RpgCompanyBody), Attr::Frame);
            let mut x = 2 + s.text(2, TEXT_Y + 3, i18n::t(Msg::RpgDWith), Attr::Normal) + 1;
            x += num(s, x, TEXT_Y + 3, travelling, Attr::Bright);
            x += s.text(x, TEXT_Y + 3, "/", Attr::Frame);
            num(s, x, TEXT_Y + 3, party::TRAVELLING, Attr::Frame);
            s.rule(MID_Y, Attr::Frame);
            status(s, &hero_snapshot);
        };

        let Some(i) = choose(p, s, &items[..n], &mut sel, &mut redraw) else {
            return;
        };
        if i == back {
            return;
        }
        companion(p, s, h, party, who[i], c);
        s.invalidate();
    }
}

/// Anybody who asked to be taken somewhere, and has been.
///
/// Called on arrival rather than on a timer, because that is the only moment it
/// can become true. Returns the companion, so the caller can say so.
fn arrived_for(party: &mut Party, place: u8) -> Option<usize> {
    for (kind, m) in party.members.iter_mut().enumerate() {
        if m.known
            && m.present
            && m.quest == party::QUEST_ASKED
            && party::KINDS[kind].quest_at == place
        {
            m.quest = party::QUEST_DONE;
            m.morale = 100;
            m.hp = m.hp.saturating_add(2);
            return Some(kind);
        }
    }
    None
}

// ------------------------------------------------------------------- village

/// What the currently selected line will do.
#[derive(Clone, Copy)]
enum Act {
    /// Which road, and where along it you come out.
    Go(u8, u8),
    Talk(u8),
    Company,
    Rest,
    Spar,
    Hunt,
    Glean,
    Bag,
    Sheet,
    Save,
    Quit,
}

fn fight(
    p: &mut dyn Platform,
    s: &mut Screen,
    h: &mut Hero,
    party: &mut Party,
    rng: &mut Rng,
    foes: &[u8],
) -> combat::Outcome {
    let outcome = combat::battle(p, s, h, party, rng, foes);
    // Any full-screen thing that follows must repaint: the battle left its own
    // frame on the terminal and on the panel.
    s.invalidate();
    match outcome {
        combat::Outcome::Won => {
            let xp: u16 = foes.iter().map(|&f| world::FOES[f as usize].xp).sum();
            let levelled = h.grant_xp(xp);
            h.coin = h.coin.saturating_add(xp / 10);
            // Costache has lost four sheep and says it was not wolves. Killing
            // one is the only way to find out whether he is right, and it is
            // what he wants to see before he will walk anywhere with you.
            if foes.contains(&world::F_WOLF) {
                h.flags[0] |= party::DEED_WOLF;
            }
            notice(p, s, h, Msg::RpgWonTitle, Msg::RpgWon, Some(xp as usize));
            // What they were carrying. A boar is meat, which is the only reason
            // hunger has an answer that is not paying Nikos three coin.
            let mut taken = 0usize;
            for &f in foes {
                let loot = world::FOES[f as usize].loot;
                if item::valid(loot) && h.bag.add(loot, 1) == 0 {
                    taken += 1;
                }
            }
            if taken > 0 {
                s.invalidate();
                notice(p, s, h, Msg::RpgLootTitle, Msg::RpgLoot, Some(taken));
            }
            if levelled {
                s.invalidate();
                notice(p, s, h, Msg::RpgLevelTitle, Msg::RpgLevel, Some(h.level as usize));
            }
        }
        combat::Outcome::Lost => notice(p, s, h, Msg::RpgLostTitle, Msg::RpgLost, None),
        combat::Outcome::Fled => notice(p, s, h, Msg::RpgFledTitle, Msg::RpgFled, None),
    }
    s.invalidate();
    outcome
}

fn talk(
    p: &mut dyn Platform,
    s: &mut Screen,
    h: &mut Hero,
    party: &mut Party,
    mind: &mut Village,
    npc: u8,
    c: Clock,
) {
    let who = &NPCS[npc as usize];
    let trades = !who.stock.is_empty();
    // Three of the fourteen will travel, and only before they have ever agreed.
    // Afterwards they are a companion and are handled on their own screen.
    let joins = companion_of(npc).filter(|&k| !party.members[k].known);

    // Speaking to somebody is what forms the memory, and it forms two: they
    // see your face every time and are told your name once, so a face gains
    // more per meeting and fades faster. The name only starts going in after
    // the first conversation -- before that they have not heard it.
    let first = mind.of(npc).strength[mind::M_FACE] == 0;
    mind.touch(npc, mind::M_FACE, h);
    if !first {
        mind.touch(npc, mind::M_NAME, h);
    }
    // The short-term trace is written **last**, so the spacing penalty it
    // carries applies to the *next* conversation rather than to this one. The
    // other way round, the first meeting of a lifetime is discounted for having
    // just happened, which is nonsense and read as "a stranger" on a screen
    // where somebody had just spent five minutes talking to you.
    mind.touch(npc, mind::M_PASSING, h);
    let knows = mind.known(npc);

    s.clear();
    chrome(s, who.name, i18n::t(who.role));
    s.wrap(TEXT_X, TEXT_Y, TEXT_W, TEXT_H, i18n::t(who.line), Attr::Normal);
    s.rule(MID_Y, Attr::Frame);
    // How well they know you, on the screen where you find out. A bare
    // number would be a stat; the four words are what a person would say.
    let mut kx = 4 + s.text(4, LIST_Y, i18n::t(Msg::RpgDKnown), Attr::Normal) + 1;
    kx += s.text(kx, LIST_Y, i18n::t(knows.msg()), Attr::Accent) + 3;
    if knows.goodwill() > 0 {
        kx += s.text(kx, LIST_Y, i18n::t(Msg::RpgDGoodwill), Attr::Normal) + 1;
        kx += num(s, kx, LIST_Y, knows.goodwill() as usize, Attr::Accent);
        s.text(kx, LIST_Y, "%", Attr::Frame);
    }
    // Trade and company are both offered by the person, not by the place, so
    // the prompt is here rather than on the village menu -- and the menu does
    // not grow a line per shop or per recruit.
    let prompt = match (trades, joins.is_some()) {
        (true, true) => Msg::RpgBothHint,
        (true, false) => Msg::RpgTradeHint,
        (false, true) => Msg::RpgJoinHint,
        (false, false) => Msg::RpgAnyKey,
    };
    s.text(4, LIST_Y + 2, i18n::t(prompt), Attr::Accent);
    status(s, h);
    s.flush(p);
    let key = read_key(p);

    match key {
        Key::Char(b'T') if trades => {
            mind.touch(npc, mind::M_DEED, h);
            bag::shop(p, s, h, mind, npc, c);
            s.invalidate();
        }
        Key::Char(b'J') => {
            if let Some(kind) = joins {
                s.invalidate();
                let before = party.members[kind].known;
                ask_along(p, s, h, party, kind);
                // Agreeing to walk out of the village with somebody is the
                // kind of thing nobody forgets.
                if party.members[kind].known && !before {
                    mind.touch(npc, mind::M_BOND, h);
                }
                s.invalidate();
            }
        }
        _ => {}
    }

    // The captain is the one person who asks something of you, and the one
    // who does not forget the answer.
    if who.kind == Role::Captain && h.oath == 0 {
        swear(p, s, h);
        if h.oath > 0 {
            mind.touch(npc, mind::M_BOND, h);
        }
    }
}

/// Advance the clock and charge the time to everyone who feels it.
///
/// Hunger is counted by **boundaries the clock crossed**, not by dividing this
/// step's minutes. Per-step division throws away everything below the rate --
/// a ten-minute walk is `10 / 40 = 0` -- so the village could be crossed four
/// hundred times without anybody getting hungry. Counting boundaries is exact
/// whatever size the steps are, and needs no leftover to carry.
fn pass(h: &mut Hero, party: &mut Party, mind: &mut Village, c: &mut Clock, minutes: u32) {
    let r = h.rules();
    // How many boundaries of a given size the clock crossed. The same trick
    // three times now, and the reason is the same each time: dividing this
    // step's minutes throws away everything below the rate.
    let crossed = |per: u32, before: &Clock, after: &Clock| -> u8 {
        if per == 0 {
            return 0;
        }
        (after.at / per).saturating_sub(before.at / per).min(255) as u8
    };
    let before = *c;
    c.advance(minutes);

    let fed = crossed(r.hunger_minutes, &before, c);
    h.hunger = h.hunger.saturating_add(fed).min(100);
    // Companions eat on the same clock the hero does. They do not thirst: two
    // bars per person is bookkeeping, and the one the player can act on is the
    // one in their own bag.
    party.feed_time(fed);

    if r.thirst {
        let dry = crossed(r.thirst_minutes, &before, c);
        h.thirst = h.thirst.saturating_add(dry).min(100);
    }
    // The village forgets on the day boundary. Once a day is the right
    // grain: it is the shortest span over which anybody notices they have
    // stopped being recognised, and it means the walk happens on a tick
    // that already does a great deal of other work.
    let days = c.days_between(before);
    if days > 0 {
        mind.fade(days, h.modifier(hero::CHA));
    }
    if h.poison > 0 {
        // Poison works itself off on the same boundaries, and takes a health
        // for each one. Standing still is not a cure; it is just a slower way
        // of paying.
        let ticks = crossed(r.hunger_minutes, &before, c);
        h.poison = h.poison.saturating_sub(ticks);
        h.hp = h
            .hp
            .saturating_sub(ticks.saturating_mul(hero::POISON_BITE))
            .max(1);
    }
}

/// The light where you are standing, which is not always the light the clock
/// says: under the canopy only full day gets through.
fn light_at(place: u8, c: Clock) -> Light {
    if PLACES[place as usize].canopy {
        c.light().under_trees()
    } else {
        c.light()
    }
}

/// Walk a road. Returns where you end up, which is not always where you meant.
///
/// The order matters. The clock advances **before** the encounter is rolled, so
/// a long road that sets out in the afternoon is ambushed at the hour it
/// actually arrives -- which is the entire reason the bridge road is a hundred
/// and ten minutes. Losing puts you back inside the gate rather than at the far
/// end of the road, because that is what the defeat text has always said
/// happened, and a hero waking alone two hours from the village at one hit
/// point is a dead end rather than a setback.
// Eight arguments, and every one of them is a different thing this has to
// touch: the terminal, the frame, the hero, the party, the clock, the dice, and
// which road to where. Bundling any two would make a struct that exists only to
// satisfy a lint. `combat::attack` made the same call.
#[allow(clippy::too_many_arguments)]
fn journey(
    p: &mut dyn Platform,
    s: &mut Screen,
    h: &mut Hero,
    party: &mut Party,
    mind: &mut Village,
    c: &mut Clock,
    rng: &mut Rng,
    which: u8,
    to: u8,
) {
    let r: &Road = &ROADS[which as usize];

    if r.wild && !c.light().travel_safe() {
        // The torch rule, which used to belong to the gate and now belongs to
        // every road that leaves the village.
        if h.bag.take_one(item::I_TORCH) {
            notice(p, s, h, Msg::RpgRoadTitle, Msg::RpgTorchLit, None);
        } else {
            notice(p, s, h, Msg::RpgRoadTitle, Msg::RpgTooDark, None);
            return;
        }
    }

    pass(h, party, mind, c, r.minutes);
    h.place = to;

    // Been there. Two of the three companions ask for proof rather than coin,
    // and this is where the proof is written down.
    h.flags[0] |= match to {
        world::P_FOREST => party::DEED_FOREST,
        world::P_FIELDS => party::DEED_FIELDS,
        world::P_BRIDGE => party::DEED_BRIDGE,
        _ => 0,
    };
    if let Some(kind) = arrived_for(party, to) {
        notice(p, s, h, Msg::RpgTaskTitle, party::Member::spec(kind).thanks, None);
        s.invalidate();
    }

    if r.danger == 0 || r.foes.is_empty() {
        return;
    }
    // Arriving in the dark is worse than setting out in it, and this is where
    // the hunter's advice stops being advice.
    let chance = rules::scaled(r.danger as i32, h.rules().danger) as u32
        + if c.light().travel_safe() { 0 } else { 25 };
    if rng.next() % 100 >= chance {
        return;
    }

    let mut foes = [0u8; road::MAX_PACK];
    let n = (r.pack as usize).clamp(1, road::MAX_PACK);
    for f in foes.iter_mut().take(n) {
        *f = r.foes[(rng.next() as usize) % r.foes.len()];
    }
    notice(p, s, h, Msg::RpgAmbushTitle, Msg::RpgAmbush, None);
    if fight(p, s, h, party, rng, &foes[..n]) == combat::Outcome::Lost {
        h.place = world::P_GATE;
    }
}

/// The forest, and the one thing it is for.
///
/// A perception check rather than a flat yield, because `VIS`, `HEA` and `SEN`
/// have until now fed exactly one derived number that nothing rolled against.
/// This is that roll: whichever of the three senses is sharpest decides whether
/// you find the game before it finds you.
#[allow(clippy::too_many_arguments)]
fn hunt(
    p: &mut dyn Platform,
    s: &mut Screen,
    h: &mut Hero,
    party: &mut Party,
    mind: &mut Village,
    c: &mut Clock,
    rng: &mut Rng,
) {
    if !light_at(h.place, *c).travel_safe() {
        notice(p, s, h, Msg::RpgHuntTitle, Msg::RpgNoLight, None);
        return;
    }
    pass(h, party, mind, c, cost::HUNT);
    let roll = rng.d20() + h.perception() - 10;
    let got = match roll {
        r if r >= 16 => 2u8,
        r if r >= 11 => 1,
        r if r <= 4 => {
            // You found something. It found you first.
            notice(p, s, h, Msg::RpgHuntTitle, Msg::RpgHuntFound, None);
            fight(p, s, h, party, rng, &[world::F_WOLF]);
            return;
        }
        _ => 0,
    };
    if got == 0 {
        notice(p, s, h, Msg::RpgHuntTitle, Msg::RpgHuntPoor, None);
        return;
    }
    let left = h.bag.add(item::I_MEAT, got);
    if left == got {
        notice(p, s, h, Msg::RpgHuntTitle, Msg::RpgBagFull, None);
    } else {
        notice(p, s, h, Msg::RpgHuntTitle, Msg::RpgHuntGood, Some((got - left) as usize));
    }
}

/// The fields, and the one thing they are for.
///
/// It is Brumar and the harvest is in, so this is gleaning rather than
/// harvesting: what is left after the carts have gone. An apple is certain and
/// grain is not, which keeps a broke hero fed without making the four shops
/// that sell food decorative.
#[allow(clippy::too_many_arguments)]
fn glean(
    p: &mut dyn Platform,
    s: &mut Screen,
    h: &mut Hero,
    party: &mut Party,
    mind: &mut Village,
    c: &mut Clock,
    rng: &mut Rng,
) {
    if !light_at(h.place, *c).travel_safe() {
        notice(p, s, h, Msg::RpgGleanTitle, Msg::RpgNoLight, None);
        return;
    }
    pass(h, party, mind, c, cost::GLEAN);
    h.bag.add(item::I_APPLES, 1);
    // Endurance, not luck: gleaning is bending over for an hour.
    if rng.d20() + h.modifier(hero::END) >= 13 {
        h.bag.add(item::I_BREAD, 1);
        notice(p, s, h, Msg::RpgGleanTitle, Msg::RpgGleanGood, None);
    } else {
        notice(p, s, h, Msg::RpgGleanTitle, Msg::RpgGleanSome, None);
    }
}

#[allow(clippy::too_many_arguments)]
fn village(
    p: &mut dyn Platform,
    s: &mut Screen,
    h: &mut Hero,
    party: &mut Party,
    mind: &mut Village,
    c: &mut Clock,
    rng: &mut Rng,
) {
    let mut sel = 0usize;
    loop {
        let place = &PLACES[h.place as usize];

        // Build the line-up for this location. Text first, actions second, so
        // the numbers stay where the player expects them.
        let mut acts = [Act::Quit; LIST_MAX];
        let mut items = [Line::plain("", Attr::Normal); LIST_MAX];
        let mut n = 0usize;

        // Everything that varies with the location has to leave room for the
        // four verbs that are always there. One guard, one number, and the
        // worst case is the square at exactly LIST_MAX.
        let mut ways = [(0u8, 0u8); road::MAX_EXITS];
        let out = road::exits(h.place, &mut ways);
        for &(which, to) in ways.iter().take(out) {
            if n >= LIST_MAX - RESERVED {
                break;
            }
            let r: &Road = &ROADS[which as usize];
            acts[n] = Act::Go(which, to);
            items[n] = Line {
                text: Text::Plain(PLACES[to as usize].name),
                // A road that leaves the village is marked as one. Two hours of
                // walking should not look like the crooked lane.
                attr: if r.wild { Attr::Warn } else { Attr::Normal },
                via: Some((Text::Plain(r.name), r.minutes)),
            };
            n += 1;
        }
        for &npc in place.npcs {
            if n >= LIST_MAX - RESERVED {
                break;
            }
            // Somebody who is on the road with you is not also standing in
            // their own kitchen. Without this the hunter appears twice at the
            // Hearth the moment she joins, once as a villager and once as a
            // companion, and both would answer.
            if party.is_away(npc) {
                continue;
            }
            acts[n] = Act::Talk(npc);
            items[n] = Line::plain(NPCS[npc as usize].name, Attr::Accent);
            n += 1;
        }
        // Companions used to be a line each on every village screen. With four
        // of them that does not fit: the square already spends five lines on
        // roads and two on people, and the guard that kept the list inside
        // twelve was silently dropping the second companion -- which is the
        // worst possible thing to drop, because it is the only way to reach
        // them. One standing entry instead, beside the bag.
        // Location-specific verbs. Four places do something; the rest talk.
        let extra: Option<(Act, Msg, u32)> = match h.place {
            world::P_STAG => Some((Act::Rest, Msg::RpgActRest, 0)),
            world::P_YARD => Some((Act::Spar, Msg::RpgActSpar, cost::SPAR)),
            world::P_FOREST => Some((Act::Hunt, Msg::RpgActHunt, cost::HUNT)),
            world::P_FIELDS => Some((Act::Glean, Msg::RpgActGlean, cost::GLEAN)),
            _ => None,
        };
        if let Some((a, m, minutes)) = extra {
            if n < LIST_MAX - RESERVED {
                acts[n] = a;
                items[n] = Line {
                    text: i18n::t(m),
                    attr: Attr::Warn,
                    // Work costs hours out here, and the same column that says
                    // how far a road is says how long a job is.
                    via: if minutes > 0 { Some((Text::Plain(""), minutes)) } else { None },
                };
                n += 1;
            }
        }
        for (a, m) in [
            (Act::Company, Msg::RpgActCompany),
            (Act::Bag, Msg::RpgActBag),
            (Act::Sheet, Msg::RpgActSheet),
            (Act::Save, Msg::RpgActSave),
            (Act::Quit, Msg::RpgActQuit),
        ] {
            if n < LIST_MAX {
                acts[n] = a;
                items[n] = Line::plain(i18n::t(m), Attr::Frame);
                n += 1;
            }
        }

        if sel >= n {
            sel = 0;
        }

        let name = place.name;
        let place_id = h.place;
        let about = place.about;
        let hero_snapshot = *h;
        let now = *c;
        let mut redraw = |s: &mut Screen| {
            s.clear();
            // The game's name is on the title screen and in HELP. On the
            // village screen the right-hand side is worth more as a clock.
            chrome(s, name, "");
            draw_clock(s, now, place_id);
            s.wrap(TEXT_X, TEXT_Y, TEXT_W, TEXT_H, i18n::t(about), Attr::Normal);
            s.rule(MID_Y, Attr::Frame);
            status(s, &hero_snapshot);
        };

        let i = match choose_or(p, s, &items[..n], &mut sel, &mut redraw, b"M") {
            Pick::Item(i) => i,
            Pick::Hot(b'M') => {
                map::run(p, s, h.place);
                s.invalidate();
                continue;
            }
            Pick::Hot(_) | Pick::Quit => return,
        };

        match acts[i] {
            Act::Go(which, to) => {
                journey(p, s, h, party, mind, c, rng, which, to);
                sel = 0;
            }
            Act::Talk(npc) => {
                talk(p, s, h, party, mind, npc, *c);
                pass(h, party, mind, c, cost::TALK);
            }
            Act::Company => {
                company(p, s, h, party, *c);
                s.invalidate();
            }
            Act::Rest => {
                if h.coin >= 3 {
                    h.coin -= 3;
                    h.hp = h.hp_max();
                    party.rest();
                    // A bed no longer feeds you. It used to, because nothing
                    // else could; now Nikos sells bread two paces from the
                    // stairs, and a night that repaired both hunger and health
                    // for three coin would make the whole shop decorative.
                    //
                    // A night is charged like any other stretch of time, which
                    // it was not before -- sleeping used to be the one action
                    // that moved the clock without anybody getting hungrier.
                    let slept = {
                        let mut probe = *c;
                        probe.sleep_until_morning()
                    };
                    pass(h, party, mind, c, slept);
                    notice(p, s, h, Msg::RpgRestTitle, Msg::RpgRested, None);
                } else {
                    notice(p, s, h, Msg::RpgRestTitle, Msg::RpgNoCoin, None);
                }
            }
            // The yard is safe: the captain stops it before anyone is hurt, so
            // the fight teaches the controls without teaching them expensively.
            Act::Spar => {
                fight(p, s, h, party, rng, &[world::F_BANDIT]);
                pass(h, party, mind, c, cost::SPAR);
            }
            Act::Hunt => hunt(p, s, h, party, mind, c, rng),
            Act::Glean => glean(p, s, h, party, mind, c, rng),
            Act::Bag => {
                bag::carry(p, s, h, party, *c);
                s.invalidate();
            }
            Act::Sheet => {
                loop {
                    match sheet(p, s, h, party) {
                        Key::Esc | Key::Enter => break,
                        _ => {}
                    }
                }
                s.invalidate();
            }
            Act::Save => match save::store(p, h, party, mind, *c) {
                Ok(()) => notice(p, s, h, Msg::RpgSaveTitle, Msg::RpgSaved, None),
                // The code says which step, because "it failed" is not
                // actionable on a driver with two address spaces and a
                // half-second erase.
                Err(e) => notice(p, s, h, Msg::RpgSaveTitle, Msg::RpgSaveFail, Some(e.code())),
            },
            Act::Quit => return,
        }
    }
}

// ------------------------------------------------------------------- entry

/// Who you are, then what you are made of, then what you are called.
fn create(p: &mut dyn Platform, s: &mut Screen, rng: &mut Rng) -> Option<Hero> {
    let mut h = Hero::fresh();
    if !difficulty(p, s, &mut h) {
        return None;
    }
    if !identity(p, s, &mut h) {
        return None;
    }
    if !allocate(p, s, &mut h) {
        return None;
    }
    if !ask_name(p, s, &mut h) {
        return None;
    }
    // The body follows the stats, once they have stopped moving.
    h.settle(rng);
    h.kit();
    Some(h)
}

/// `RPG` -- the command.
pub fn run(p: &mut dyn Platform) {
    // 8 KB of buffers on the stack. Static RAM here is 40 bytes and the shell
    // is shallow at this point; `#![forbid(unsafe_code)]` rules out a static.
    let mut s = Screen::new();
    let mut rng = Rng::new(p.uptime_ms() as u32);

    vt::alt_enter(p);
    vt::hide_cursor(p);
    vt::cls_normal(p);

    let saved = save::load(p);
    let can_save = save::available(p);

    let mut sel = 0usize;
    loop {
        let mut labels: [Text; 4] = [Text::Plain(""); 4];
        let mut acts: [u8; 4] = [0; 4];
        let mut n = 0;
        if saved.is_some() {
            labels[n] = i18n::t(Msg::RpgMenuContinue);
            acts[n] = 0;
            n += 1;
        }
        labels[n] = i18n::t(Msg::RpgMenuNew);
        acts[n] = 1;
        n += 1;
        if saved.is_some() {
            labels[n] = i18n::t(Msg::RpgMenuErase);
            acts[n] = 2;
            n += 1;
        }
        labels[n] = i18n::t(Msg::RpgMenuQuit);
        acts[n] = 3;
        n += 1;

        let mut items = [Line::plain("", Attr::Normal); LIST_MAX];
        for i in 0..n {
            items[i] = Line::plain(labels[i], Attr::Normal);
        }
        if sel >= n {
            sel = 0;
        }

        let mut redraw = |s: &mut Screen| {
            s.clear();
            chrome(s, i18n::t(Msg::RpgTitle), "");
            s.wrap(TEXT_X, TEXT_Y, TEXT_W, TEXT_H, i18n::t(Msg::RpgIntro), Attr::Normal);
            s.rule(MID_Y, Attr::Frame);
            s.fill(1, STATUS_Y, screen::COLS - 2, 1, b' ', Attr::Normal);
            if !can_save {
                s.text(2, STATUS_Y, i18n::t(Msg::RpgNoStore), Attr::Warn);
            }
            keys_line(s, STATUS_Y, if can_save { 1 } else { 40 }, Msg::RpgKeys);
        };

        let Some(i) = choose(p, &mut s, &items[..n], &mut sel, &mut redraw) else {
            break;
        };

        match acts[i] {
            0 => {
                if let Some((mut h, mut party, mut clock, mut mind)) = saved {
                    village(p, &mut s, &mut h, &mut party, &mut mind, &mut clock, &mut rng);
                    break;
                }
            }
            1 => {
                if let Some(mut h) = create(p, &mut s, &mut rng) {
                    // You did not arrive alone, and the game never asks whether
                    // you want him.
                    let mut party = Party::starting();
                    let mut clock = Clock::new();
                    // Nobody in Vadul Alb has ever seen you. That is the whole
                    // premise, and it is now a table rather than a sentence.
                    let mut mind = Village::new();
                    village(p, &mut s, &mut h, &mut party, &mut mind, &mut clock, &mut rng);
                }
                break;
            }
            2 => {
                let _ = save::erase(p);
                break;
            }
            _ => break,
        }
    }

    vt::show_cursor(p);
    vt::alt_leave(p);
}
