//! The two screens that move things: the bag, and a shop counter.
//!
//! # One list, three kinds of row
//!
//! What you are wearing, what you are holding and what is in the bag are the
//! same list. Gear rows come first and carry a tag; selecting one takes the
//! thing off. That is why there is no separate "equipment" screen and no key
//! for unequipping -- the list already knows which rows are which, and the
//! player only has to learn `ENTER`.
//!
//! # Why the shop is two lists and not one
//!
//! Buying and selling are the same gesture with the price reversed, and a
//! single list with both would need a column saying which direction each row
//! goes in. Two modes on the left/right keys costs one line of chrome and
//! removes the ambiguity entirely: whatever is on screen, `ENTER` does the
//! thing the header says.

use super::clock::Clock;
use super::hero::{Hero, Use};
use super::item::{self, Effect, Slot};
use super::mind::{Known, Village};
use super::party::Party;
use super::{chrome, draw_clock, keys_line, kg, num, snum, STATUS_Y};
use crate::i18n::{self, Msg, Text};
use crate::screen::{self, Attr, Screen};
use crate::setup::{read_key, Key};
use crate::Platform;

const HEAD_Y: usize = 3;
const LIST_Y: usize = 5;
/// Three gear slots plus twelve bag slots, so a full inventory never scrolls.
const LIST_H: usize = 15;
/// The counter carries one more line of chrome than the bag -- what they
/// think of you -- so its rows start one lower. Two screens sharing a row
/// origin would mean the rule landing on the first item.
const SHOP_Y: usize = LIST_Y + 1;
const DETAIL_Y: usize = 21;
const NOTE_Y: usize = 22;

const NAME_X: usize = 7;
const COUNT_X: usize = 32;
const WEIGHT_X: usize = 38;
const RIGHT_X: usize = 48;
const TAG_X: usize = 58;

/// Whose bag is on screen.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Who {
    You,
    Comp(usize),
}

/// One line of the list.
#[derive(Clone, Copy)]
enum Row {
    /// Worn or held, so not in the bag. Selecting it takes it off.
    Worn(Slot),
    /// An occupied slot in whichever bag is shown.
    Held(usize),
}

// ------------------------------------------------------------------ drawing

/// The one-line description under the list: what the thing does, what it
/// weighs, what it is worth.
///
/// Generated from the item rather than written as prose. A hand-written line
/// for eighteen items in two languages is thirty-six strings that go stale the
/// first time a damage die is re-balanced; these numbers cannot disagree with
/// the ones the fight uses because they are the same numbers.
fn detail(s: &mut Screen, y: usize, id: u8, price: Option<u16>, thirsty: bool) {
    s.fill(1, y, screen::COLS - 2, 1, b' ', Attr::Normal);
    if !item::valid(id) {
        return;
    }
    let spec = item::spec(id);
    let mut x = 2 + s.text(2, y, i18n::t(spec.name), Attr::Bright) + 2;
    match spec.effect {
        Effect::Strike { die, hit, reach } => {
            s.put(x, y, b'd', Attr::Accent);
            x += 1 + num(s, x + 1, y, die as usize, Attr::Accent) + 2;
            if hit != 0 {
                x += snum(s, x, y, hit as i32, Attr::Accent) + 1;
                x += s.text(x, y, i18n::t(Msg::RpgDHit), Attr::Normal) + 2;
            }
            if reach > 1 {
                x += s.text(x, y, i18n::t(Msg::RpgDReach), Attr::Normal) + 1;
                x += num(s, x, y, reach as usize, Attr::Accent) + 2;
            }
        }
        Effect::Guard { ac } => {
            x += snum(s, x, y, ac as i32, Attr::Accent) + 1;
            x += s.text(x, y, i18n::t(Msg::RpgDAc), Attr::Normal) + 2;
        }
        // Water feeds nobody, so a bare `feeds 0` would be the whole line.
        Effect::Eat { hunger } if hunger > 0 => {
            x += s.text(x, y, i18n::t(Msg::RpgDFeeds), Attr::Normal) + 1;
            x += num(s, x, y, hunger as usize, Attr::Accent) + 2;
        }
        Effect::Mend { heal } => {
            x += s.text(x, y, i18n::t(Msg::RpgDHeal), Attr::Normal) + 1;
            x += num(s, x, y, heal as usize, Attr::Accent) + 2;
        }
        Effect::Eat { .. } | Effect::Burn | Effect::Spend => {}
    }
    // Only where it is read. Under the five settings that do not track thirst,
    // a number for it on the shop line would be a promise the game does not
    // keep.
    if spec.wet > 0 && thirsty {
        x += s.text(x, y, i18n::t(Msg::RpgDSlakes), Attr::Normal) + 1;
        x += num(s, x, y, spec.wet as usize, Attr::Accent) + 2;
    }
    if spec.heavy {
        x += s.text(x, y, i18n::t(Msg::RpgTwoHands), Attr::Frame) + 2;
    }
    x += kg(s, x, y, spec.weight as u32, Attr::Normal);
    x += s.text(x, y, " kg", Attr::Normal) + 2;
    if let Some(p) = price {
        x += num(s, x, y, p as usize, Attr::Accent) + 1;
        s.text(x, y, i18n::t(Msg::RpgDCoin), Attr::Normal);
    }
}

/// Health, load and purse on one line.
///
/// These three are here rather than in the usual status bar because the bar is
/// where the key legend lives, and the legend on this screen is long enough
/// that the two would fight. Health belongs on it anyway: whether to spend
/// fourteen coin on a salve is the question this screen exists to answer.
fn burden(s: &mut Screen, h: &Hero, y: usize, x0: usize) {
    let over = h.over_laden();
    let mut x = x0;
    s.put(x, y, screen::HEART, Attr::Warn);
    x += 2;
    x += num(s, x, y, h.hp as usize, Attr::Normal);
    x += s.text(x, y, "/", Attr::Frame);
    x += num(s, x, y, h.hp_max() as usize, Attr::Frame) + 3;
    x += s.text(x, y, i18n::t(Msg::RpgDWeight), Attr::Normal) + 1;
    x += kg(
        s,
        x,
        y,
        h.load_g(),
        if over { Attr::Warn } else { Attr::Bright },
    );
    x += s.text(x, y, " / ", Attr::Frame);
    x += num(s, x, y, h.carry_kg() as usize, Attr::Frame);
    x += s.text(x, y, " kg", Attr::Frame) + 3;
    x += num(s, x, y, h.coin as usize, Attr::Accent) + 1;
    x += s.text(x, y, i18n::t(Msg::RpgDCoin), Attr::Normal);
    if over {
        s.text(x + 3, y, i18n::t(Msg::RpgOverLaden), Attr::Warn);
    }
}

fn row_line(s: &mut Screen, y: usize, i: usize, on: bool, id: u8, count: u8, tag: Option<Msg>) {
    s.put(2, y, if on { screen::ARROW } else { b' ' }, Attr::Bright);
    let attr = if on { Attr::Bright } else { Attr::Normal };
    if i < 9 {
        s.put(4, y, b'1' + i as u8, if on { Attr::Bright } else { Attr::Frame });
        s.put(5, y, b'.', Attr::Frame);
    }
    s.text(NAME_X, y, i18n::t(item::name(id)), attr);
    if count > 1 {
        s.put(COUNT_X, y, b'x', Attr::Frame);
        num(s, COUNT_X + 1, y, count as usize, Attr::Accent);
    }
    let total = item::spec(id).weight as u32 * count.max(1) as u32;
    let w = kg(s, WEIGHT_X, y, total, Attr::Frame);
    s.text(WEIGHT_X + w, y, " kg", Attr::Frame);
    if let Some(t) = tag {
        s.text(TAG_X, y, i18n::t(t), Attr::Accent);
    }
}

// ---------------------------------------------------------------- the bag

/// Collect the rows for whoever is on screen. Empty slots are skipped, so the
/// list is as long as the inventory is rather than as long as the array.
fn rows(h: &Hero, party: &Party, who: Who, out: &mut [Row; LIST_H]) -> usize {
    let mut n = 0usize;
    match who {
        Who::You => {
            for slot in [Slot::Weapon, Slot::Body, Slot::Hand] {
                if item::valid(h.gear.in_slot(slot)) && n < LIST_H {
                    out[n] = Row::Worn(slot);
                    n += 1;
                }
            }
            for (i, st) in h.bag.slots.iter().enumerate() {
                if !st.is_empty() && n < LIST_H {
                    out[n] = Row::Held(i);
                    n += 1;
                }
            }
        }
        Who::Comp(c) => {
            for (i, st) in party.members[c].pack.slots.iter().enumerate() {
                if !st.is_empty() && n < LIST_H {
                    out[n] = Row::Held(i);
                    n += 1;
                }
            }
        }
    }
    n
}

/// What a row is showing: the item, how many, and the tag beside it.
fn row_of(h: &Hero, party: &Party, who: Who, row: Row) -> (u8, u8, Option<Msg>) {
    match (who, row) {
        (Who::You, Row::Worn(slot)) => (
            h.gear.in_slot(slot),
            1,
            Some(match slot {
                Slot::Body => Msg::RpgTagWorn,
                _ => Msg::RpgTagHeld,
            }),
        ),
        (Who::You, Row::Held(i)) => (h.bag.slots[i].item, h.bag.slots[i].count, None),
        (Who::Comp(c), Row::Held(i)) => {
            let st = party.members[c].pack.slots[i];
            (st.item, st.count, None)
        }
        _ => (item::NONE, 0, None),
    }
}

/// The owners you can page through: you, then whoever is travelling with you.
fn owners(party: &Party, out: &mut [Who; 1 + super::party::TRAVELLING]) -> usize {
    out[0] = Who::You;
    let mut n = 1;
    for (i, _) in party.travelling() {
        if n < out.len() {
            out[n] = Who::Comp(i);
            n += 1;
        }
    }
    n
}

/// The bag. `ENTER` uses or takes off, `G` hands one across, `X` drops one.
pub fn carry(p: &mut dyn Platform, s: &mut Screen, h: &mut Hero, party: &mut Party, c: Clock) {
    let mut sel = 0usize;
    let mut list = [Who::You; 1 + super::party::TRAVELLING];
    let owners_n = owners(party, &mut list);
    let mut owner = 0usize;
    let mut note: Option<Msg> = None;

    loop {
        let who = list[owner];
        let mut rows_buf = [Row::Held(0); LIST_H];
        let n = rows(h, party, who, &mut rows_buf);
        if sel >= n {
            sel = n.saturating_sub(1);
        }

        s.clear();
        chrome(s, i18n::t(Msg::RpgBagTitle), "");
        draw_clock(s, c, h.place);
        s.rule(2, Attr::Frame);

        // Owner tabs. Left and right page through them, which is also how the
        // shop switches between buying and selling -- one habit, two screens.
        let mut x = 2usize;
        for (i, w) in list[..owners_n].iter().enumerate() {
            let name = match w {
                Who::You => i18n::t(Msg::RpgYou),
                Who::Comp(ci) => Text::Plain(super::party::Member::spec(*ci).name),
            };
            let on = i == owner;
            s.put(x, HEAD_Y, if on { b'[' } else { b' ' }, Attr::Frame);
            x += 1;
            x += s.text(x, HEAD_Y, name, if on { Attr::Bright } else { Attr::Frame });
            s.put(x, HEAD_Y, if on { b']' } else { b' ' }, Attr::Frame);
            x += 3;
        }
        burden(s, h, HEAD_Y, x.max(28));
        s.rule(HEAD_Y + 1, Attr::Frame);

        if n == 0 {
            s.text(NAME_X, LIST_Y, i18n::t(Msg::RpgBagEmpty), Attr::Frame);
        }
        for i in 0..n {
            let (id, count, tag) = row_of(h, party, who, rows_buf[i]);
            row_line(s, LIST_Y + i, i, i == sel, id, count, tag);
        }

        s.rule(DETAIL_Y - 1, Attr::Frame);
        if n > 0 {
            let (id, _, _) = row_of(h, party, who, rows_buf[sel]);
            detail(s, DETAIL_Y, id, None, h.rules().thirst);
        }
        s.fill(1, NOTE_Y, screen::COLS - 2, 1, b' ', Attr::Normal);
        if let Some(m) = note {
            s.text(2, NOTE_Y, i18n::t(m), Attr::Warn);
        }
        s.fill(1, STATUS_Y, screen::COLS - 2, 1, b' ', Attr::Normal);
        keys_line(s, STATUS_Y, 1, Msg::RpgBagKeys);
        s.flush(p);

        note = None;
        match read_key(p) {
            // A digit jumps to a row but does not act on it. Everywhere
            // else in this game a number is a menu choice; here the verbs
            // spend coin and destroy things, so the second key is the point.
            Key::Digit(d) if (d as usize) <= n => sel = d as usize - 1,
            Key::Up if n > 0 => sel = (sel + n - 1) % n,
            Key::Down if n > 0 => sel = (sel + 1) % n,
            Key::Left => {
                owner = (owner + owners_n - 1) % owners_n;
                sel = 0;
            }
            Key::Right => {
                owner = (owner + 1) % owners_n;
                sel = 0;
            }
            Key::Enter if n > 0 => note = use_row(h, party, who, rows_buf[sel]),
            Key::Char(b'G') if n > 0 => note = hand_over(h, party, who, rows_buf[sel], &list[..owners_n]),
            Key::Char(b'X') if n > 0 => drop_row(h, party, who, rows_buf[sel]),
            Key::Esc => return,
            _ => {}
        }
    }
}

/// `ENTER`: eat it, drink it, wear it, wield it, or take it off.
fn use_row(h: &mut Hero, party: &mut Party, who: Who, row: Row) -> Option<Msg> {
    let outcome = match (who, row) {
        (Who::You, Row::Worn(slot)) => h.unequip(slot),
        (Who::You, Row::Held(i)) => h.use_slot(i),
        // A companion using something from his own pack is him eating it, which
        // is the same act as being fed -- the transfer key put it there.
        (Who::Comp(c), Row::Held(i)) => {
            if party.members[c].eat(i) {
                Use::Ate
            } else {
                Use::Nothing
            }
        }
        _ => Use::Nothing,
    };
    match outcome {
        Use::NeedsHands => Some(Msg::RpgTwoHands),
        Use::NoRoom => Some(Msg::RpgBagFull),
        _ => None,
    }
}

/// `G`: one unit across to the other pack, in whichever direction makes sense.
fn hand_over(h: &mut Hero, party: &mut Party, who: Who, row: Row, list: &[Who]) -> Option<Msg> {
    // With nobody travelling with you there is nowhere for it to go.
    let other = list.iter().find(|w| **w != who).copied()?;
    match (who, row, other) {
        (Who::You, Row::Held(i), Who::Comp(c)) => {
            let id = h.bag.slots[i].item;
            if !party.members[c].pack.room_for(id, 1) {
                return Some(Msg::RpgBagFull);
            }
            h.bag.take_at(i);
            party.members[c].pack.add(id, 1);
            None
        }
        (Who::Comp(c), Row::Held(i), Who::You) => {
            let id = party.members[c].pack.slots[i].item;
            if !h.bag.room_for(id, 1) {
                return Some(Msg::RpgBagFull);
            }
            party.members[c].pack.take_at(i);
            h.bag.add(id, 1);
            None
        }
        // Take it off first. Handing over the sword in your hand without
        // sheathing it is the kind of thing an interface should make you mean.
        _ => None,
    }
}

/// `X`: one unit on the ground, and gone. There is no floor to pick it up from.
fn drop_row(h: &mut Hero, party: &mut Party, who: Who, row: Row) {
    match (who, row) {
        (Who::You, Row::Held(i)) => {
            h.bag.take_at(i);
        }
        (Who::Comp(c), Row::Held(i)) => {
            party.members[c].pack.take_at(i);
        }
        _ => {}
    }
}

// --------------------------------------------------------------- the counter

/// Buy from and sell to one of the eight people in the village who trade.
pub fn shop(
    p: &mut dyn Platform,
    s: &mut Screen,
    h: &mut Hero,
    mind: &Village,
    npc: u8,
    c: Clock,
) {
    // What they charge is what they think of you, fixed for the visit so a
    // price cannot move between the line and the purchase.
    let knows = mind.known(npc);
    let who = &super::world::NPCS[npc as usize];
    let stock = who.stock;
    let mut selling = false;
    let mut sel = 0usize;
    let mut note: Option<Msg> = None;

    loop {
        // The sell list is whatever is loose in the bag. Gear is deliberately
        // not sellable from here: parting with the sword in your hand should
        // take the extra keystroke of putting it away first.
        let mut ids = [item::NONE; 16];
        let mut n = 0usize;
        if selling {
            for st in h.bag.slots.iter() {
                if !st.is_empty() && n < ids.len() {
                    ids[n] = st.item;
                    n += 1;
                }
            }
        } else {
            for &id in stock.iter() {
                if n < ids.len() {
                    ids[n] = id;
                    n += 1;
                }
            }
        }
        if sel >= n {
            sel = n.saturating_sub(1);
        }

        s.clear();
        chrome(s, who.name, "");
        draw_clock(s, c, h.place);
        s.rule(2, Attr::Frame);

        let mut x = 2usize;
        for (i, m) in [Msg::RpgShopBuy, Msg::RpgShopSell].iter().enumerate() {
            let on = (i == 1) == selling;
            s.put(x, HEAD_Y, if on { b'[' } else { b' ' }, Attr::Frame);
            x += 1;
            x += s.text(x, HEAD_Y, i18n::t(*m), if on { Attr::Bright } else { Attr::Frame });
            s.put(x, HEAD_Y, if on { b']' } else { b' ' }, Attr::Frame);
            x += 3;
        }
        burden(s, h, HEAD_Y, x.max(28));
        if knows.goodwill() > 0 {
            let mut gx = 2 + s.text(2, HEAD_Y + 1, i18n::t(Msg::RpgDGoodwill), Attr::Normal) + 1;
            gx += num(s, gx, HEAD_Y + 1, knows.goodwill() as usize, Attr::Accent);
            gx += s.text(gx, HEAD_Y + 1, "%", Attr::Frame) + 2;
            s.text(gx, HEAD_Y + 1, i18n::t(knows.msg()), Attr::Frame);
        }
        s.rule(HEAD_Y + 2, Attr::Frame);

        if n == 0 {
            let m = if selling { Msg::RpgBagEmpty } else { Msg::RpgShopBare };
            s.text(NAME_X, SHOP_Y, i18n::t(m), Attr::Frame);
        }
        for i in 0..n {
            let id = ids[i];
            let on = i == sel;
            let count = if selling { h.bag.count_of(id).min(99) as u8 } else { 1 };
            row_line(s, SHOP_Y + i, i, on, id, count, None);
            let price = price_of(id, selling, knows);
            let w = num(s, RIGHT_X, SHOP_Y + i, price as usize, Attr::Accent);
            s.text(RIGHT_X + w + 1, SHOP_Y + i, i18n::t(Msg::RpgDCoin), Attr::Frame);
        }

        s.rule(DETAIL_Y - 1, Attr::Frame);
        if n > 0 {
            detail(s, DETAIL_Y, ids[sel], Some(price_of(ids[sel], selling, knows)), h.rules().thirst);
        }
        s.fill(1, NOTE_Y, screen::COLS - 2, 1, b' ', Attr::Normal);
        if let Some(m) = note {
            s.text(2, NOTE_Y, i18n::t(m), Attr::Warn);
        }
        s.fill(1, STATUS_Y, screen::COLS - 2, 1, b' ', Attr::Normal);
        keys_line(s, STATUS_Y, 1, Msg::RpgShopKeys);
        s.flush(p);

        note = None;
        match read_key(p) {
            // A digit jumps to a row but does not act on it. Everywhere
            // else in this game a number is a menu choice; here the verbs
            // spend coin and destroy things, so the second key is the point.
            Key::Digit(d) if (d as usize) <= n => sel = d as usize - 1,
            Key::Up if n > 0 => sel = (sel + n - 1) % n,
            Key::Down if n > 0 => sel = (sel + 1) % n,
            Key::Left | Key::Right => {
                selling = !selling;
                sel = 0;
            }
            Key::Enter if n > 0 => note = trade(h, ids[sel], selling, knows),
            Key::Esc => return,
            _ => {}
        }
    }
}

/// What this row costs or fetches, goodwill included.
///
/// Fifteen percent at the very top. A village that halves its prices for a
/// regular is a village whose economy is solved by talking to everybody
/// twice; this is a thank-you, not a strategy.
fn price_of(id: u8, selling: bool, knows: Known) -> u16 {
    let g = knows.goodwill();
    if selling {
        // They pay a little over the half they would give a stranger.
        let base = item::spec(id).value as u32;
        ((base * (50 + g)) / 100).max(1) as u16
    } else {
        let base = item::spec(id).value as u32;
        ((base * (100 - g)) / 100).max(1) as u16
    }
}

fn trade(h: &mut Hero, id: u8, selling: bool, knows: Known) -> Option<Msg> {
    if selling {
        let i = h.bag.slots.iter().position(|st| st.item == id && st.count > 0)?;
        h.bag.take_at(i);
        h.coin = h.coin.saturating_add(price_of(id, true, knows));
        return None;
    }
    let price = price_of(id, false, knows);
    if h.coin < price {
        return Some(Msg::RpgCantAfford);
    }
    if !h.bag.room_for(id, 1) {
        return Some(Msg::RpgBagFull);
    }
    h.coin -= price;
    h.bag.add(id, 1);
    None
}
