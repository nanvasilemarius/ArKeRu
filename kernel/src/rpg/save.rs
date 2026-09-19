//! Saved games, in the RA4M1's data flash.
//!
//! # Why this is now possible
//!
//! When the save question was first asked, the only persistent store was the
//! ESP32's NVS through an AT command that writes **one byte per key**. A save
//! is tens of bytes, so it would have meant designing a typed blob protocol
//! against a bridge firmware whose command set had to be discovered by probing.
//!
//! 0.18.0 changed that. The data flash driver -- 8 KB, 1 KB blocks, 100 000
//! erase cycles, recovered from FSP because the User's Manual withholds the
//! interface -- is on the board and proven. A save is now one erase and one
//! short program of local flash.
//!
//! # Where it lives
//!
//! **Block 6**, at offset 6144. Deliberately not the last block: `DF` operates
//! on that one, and a diagnostic that erases the block your save is in would be
//! a trap laid for the person most likely to type `DF ERASE`.
//!
//! # What makes a save valid
//!
//! A magic word, a format version, and a CRC over everything before it. All
//! three are needed and each catches a different failure:
//!
//! - **magic** -- the block is erased, or held something else. Reads `0xFF`.
//! - **version** -- the record is from an older layout. The fields would load
//!   into the wrong places and produce a character that is subtly wrong rather
//!   than obviously broken, which is worse.
//! - **CRC** -- the write was interrupted. A save that dies halfway through
//!   leaves a record whose magic and version are perfectly good.
//!
//! Erased flash reads `0xFF`, so `0xFFFF` must not be a valid magic, and it is
//! not.

use super::clock::Clock;
use super::hero::{Hero, NAME_MAX, STATS};
use super::item::{self, Bag, Gear, Stack, PACK_SLOTS, SLOTS};
use super::mind::{Mind, Village, KINDS, NPC_COUNT};
use super::party::{self, Member, Party, ROSTER};
use crate::Platform;

/// Byte offset of the save block. Not the last block -- `DF` owns that.
const SLOT_OFF: usize = 6 * 1024;

const MAGIC: u16 = 0x5641; // "VA", for Vadul Alb
const VERSION: u8 = 7;

/// Bytes per slot: what it holds, and how many.
const SLOT: usize = 2;

/// Bytes per saved companion: known, present, where they wait, hp, morale,
/// hunger, stance, task, then the pack.
///
/// Every companion is written, not only the ones travelling. Two slots that
/// got overwritten would mean a hunter who rejoins at full morale with an
/// empty pack and no memory of what she asked you for.
const MEMBER: usize = 8 + PACK_SLOTS * SLOT;

/// Bytes in one record, CRC included. Everything is a byte or a little-endian
/// `u16`; there is no padding and no alignment to reason about.
pub const RECORD: usize = 3
    + NAME_MAX
    + 1
    + STATS
    + 6
    + 3
    + 1
    + 1
    + 2
    + 2
    + 3
    + 8
    + SLOTS * SLOT
    + 3
    + ROSTER * MEMBER
    + NPC_COUNT * KINDS * 2
    + 4
    + 2;

/// CRC-16/CCITT-FALSE. Twenty lines, catches every single-bit error and every
/// burst up to sixteen bits, which is exactly the shape of a flash failure.
fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0xFFFFu16;
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

struct Cursor<'a> {
    buf: &'a mut [u8; RECORD],
    at: usize,
}

impl Cursor<'_> {
    fn u8(&mut self, v: u8) {
        self.buf[self.at] = v;
        self.at += 1;
    }
    fn u16(&mut self, v: u16) {
        self.u8(v as u8);
        self.u8((v >> 8) as u8);
    }
    fn bytes(&mut self, v: &[u8]) {
        for &b in v {
            self.u8(b);
        }
    }
    fn bag<const N: usize>(&mut self, b: &Bag<N>) {
        for s in b.slots.iter() {
            self.u8(s.item);
            self.u8(s.count);
        }
    }
}

struct Reader<'a> {
    buf: &'a [u8; RECORD],
    at: usize,
}

impl Reader<'_> {
    fn u8(&mut self) -> u8 {
        let v = self.buf[self.at];
        self.at += 1;
        v
    }
    fn u16(&mut self) -> u16 {
        let lo = self.u8() as u16;
        let hi = self.u8() as u16;
        lo | (hi << 8)
    }
    /// Read a bag, dropping anything the current catalogue does not know.
    ///
    /// An item index from a future layout would index [`item::ITEMS`] out of
    /// range on every screen that draws a weight, so an unknown one becomes an
    /// empty slot rather than a fault.
    fn bag<const N: usize>(&mut self) -> Bag<N> {
        let mut b = Bag::new();
        for s in b.slots.iter_mut() {
            let it = self.u8();
            let count = self.u8();
            *s = if item::valid(it) && count > 0 {
                Stack {
                    item: it,
                    count: count.min(item::spec(it).stack),
                }
            } else {
                Stack::EMPTY
            };
        }
        b
    }
    fn worn(&mut self) -> u8 {
        let it = self.u8();
        if item::valid(it) {
            it
        } else {
            item::NONE
        }
    }
}

fn encode(h: &Hero, party: &Party, mind: &Village, clock: Clock) -> [u8; RECORD] {
    let mut buf = [0u8; RECORD];
    {
        let mut c = Cursor {
            buf: &mut buf,
            at: 0,
        };
        c.u16(MAGIC);
        c.u8(VERSION);
        c.bytes(&h.name);
        c.u8(h.name_len);
        c.bytes(&h.stats);
        c.u8(h.age);
        c.u8(h.height);
        c.u8(h.weight);
        c.u8(h.eyes);
        c.u8(h.gender);
        c.u8(h.hunger);
        c.u8(h.thirst);
        c.u8(h.poison);
        c.u8(h.rules);
        c.u8(h.hp);
        c.u8(h.level);
        c.u16(h.xp);
        c.u16(h.coin);
        c.u8(h.oath);
        c.u8(h.oath_state);
        c.u8(h.place);
        c.bytes(&h.flags);
        c.bag(&h.bag);
        c.u8(h.gear.weapon);
        c.u8(h.gear.body);
        c.u8(h.gear.hand);
        c.u16(clock.at as u16);
        c.u16((clock.at >> 16) as u16);
        for m in party.members.iter() {
            c.u8(u8::from(m.known));
            c.u8(u8::from(m.present));
            c.u8(m.at);
            c.u8(m.hp);
            c.u8(m.morale);
            c.u8(m.hunger);
            c.u8(m.stance);
            c.u8(m.quest);
            c.bag(&m.pack);
        }
        // What the village holds about you: a strength and a count of
        // renewals per kind, per villager. Two bytes each, and there is no
        // timestamp because the forgetting is applied on the day boundary
        // rather than computed on read.
        for m in mind.minds.iter() {
            c.bytes(&m.strength);
            c.bytes(&m.seen);
        }
    }
    let crc = crc16(&buf[..RECORD - 2]);
    buf[RECORD - 2] = crc as u8;
    buf[RECORD - 1] = (crc >> 8) as u8;
    buf
}

fn decode(buf: &[u8; RECORD]) -> Option<(Hero, Party, Clock, Village)> {
    let stored = buf[RECORD - 2] as u16 | ((buf[RECORD - 1] as u16) << 8);
    if stored != crc16(&buf[..RECORD - 2]) {
        return None;
    }
    let mut r = Reader { buf, at: 0 };
    if r.u16() != MAGIC || r.u8() != VERSION {
        return None;
    }
    let mut h = Hero::blank();
    for b in h.name.iter_mut() {
        *b = r.u8();
    }
    h.name_len = r.u8().min(NAME_MAX as u8);
    for s in h.stats.iter_mut() {
        *s = r.u8();
    }
    h.age = r.u8();
    h.height = r.u8();
    h.weight = r.u8();
    h.eyes = r.u8();
    h.gender = r.u8().min(1);
    h.hunger = r.u8();
    h.thirst = r.u8();
    h.poison = r.u8();
    // A setting from a build with more of them would index `RULES` out of
    // range on the first frame drawn, and `rules::of` clamps -- but storing
    // the clamped value keeps the save honest about what is being played.
    h.rules = r.u8().min(super::rules::LEVELS - 1);
    h.hp = r.u8();
    h.level = r.u8().max(1);
    h.xp = r.u16();
    h.coin = r.u16();
    h.oath = r.u8();
    h.oath_state = r.u8();
    // A place index from a layout with more locations than this build knows
    // would index `PLACES` out of range on the first frame drawn. The village
    // grew from nine to twelve in 0.25.0, so this is a real direction of travel
    // and not a hypothetical one.
    h.place = r.u8();
    if h.place as usize >= super::world::PLACES.len() {
        h.place = 0;
    }
    for f in h.flags.iter_mut() {
        *f = r.u8();
    }
    h.bag = r.bag::<SLOTS>();
    h.gear = Gear {
        weapon: r.worn(),
        body: r.worn(),
        hand: r.worn(),
    };
    let lo = r.u16() as u32;
    let hi = r.u16() as u32;
    let clock = Clock::from_minutes(lo | (hi << 16));
    let mut party = Party::new();
    // The roster is positional now -- slot `i` is `KINDS[i]` -- so there is no
    // kind byte to validate. What can still be wrong is a stance or a place
    // from a build that had more of either, and both are clamped rather than
    // rejected: a companion standing in the wrong square of a village that has
    // shrunk is recoverable, and refusing the whole save is not.
    for m in party.members.iter_mut() {
        let known = r.u8() != 0;
        let present = r.u8() != 0;
        let at = r.u8();
        let hp = r.u8();
        let morale = r.u8();
        let hunger = r.u8();
        let stance = r.u8();
        let quest = r.u8();
        let pack = r.bag::<PACK_SLOTS>();
        *m = if known {
            Member {
                known,
                present,
                at: if (at as usize) < super::world::PLACES.len() { at } else { 0 },
                hp,
                morale,
                hunger,
                stance: stance % party::STANCES,
                quest: quest.min(party::QUEST_DONE),
                pack,
            }
        } else {
            Member::unknown()
        };
    }
    let mut mind = Village::new();
    for m in mind.minds.iter_mut() {
        let mut fresh = Mind::blank();
        for v in fresh.strength.iter_mut() {
            *v = r.u8();
        }
        for v in fresh.seen.iter_mut() {
            *v = r.u8();
        }
        *m = fresh;
    }
    // Two is as many as will keep together, and a record claiming more would
    // put a fourth body on a field sized for seven.
    let mut walking = 0usize;
    for m in party.members.iter_mut() {
        if m.known && m.present {
            walking += 1;
            if walking > party::TRAVELLING {
                m.present = false;
            }
        }
    }
    // A name that is not UTF-8 would panic every screen that draws it, so the
    // record is rejected here rather than trusted and printed.
    core::str::from_utf8(&h.name[..h.name_len as usize]).ok()?;
    if h.eyes as usize >= super::hero::EYES.len() {
        return None;
    }
    Some((h, party, clock, mind))
}

/// Whether this platform can hold a save at all.
pub fn available(p: &mut dyn Platform) -> bool {
    match p.dataflash_info() {
        Some((size, block)) => SLOT_OFF + block <= size && block >= RECORD,
        None => false,
    }
}

/// Read the saved character, if there is a valid one.
pub fn load(p: &mut dyn Platform) -> Option<(Hero, Party, Clock, Village)> {
    if !available(p) {
        return None;
    }
    let mut buf = [0u8; RECORD];
    if !p.dataflash_read(SLOT_OFF, &mut buf) {
        return None;
    }
    decode(&buf)
}

/// Which step of a save went wrong.
///
/// A single bool would have said only that the save failed, which on a driver
/// with two address spaces and a 504 ms erase is not enough to act on. Each
/// variant points at a different suspect.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    NoStore,
    Erase,
    Write,
    Read,
    /// Carries the offset of the first byte that read back wrong. "It did not
    /// match" says nothing; index 0 means nothing was written at all, and a
    /// late index means the write stopped part way.
    Verify(usize, u8),
}

impl Fault {
    pub fn code(self) -> usize {
        match self {
            Fault::NoStore => 1,
            Fault::Erase => 2,
            Fault::Write => 3,
            Fault::Read => 4,
            Fault::Verify(..) => 5,
        }
    }
}

/// Erase the block and write the character into it.
///
/// The erase is unconditional. Flash can only clear bits, so programming over
/// an existing record without erasing yields the bitwise AND of the old and
/// new -- which passes the magic check, passes the version check, and fails the
/// CRC, so it would look exactly like a corrupted save rather than a bug.
pub fn store(
    p: &mut dyn Platform,
    h: &Hero,
    party: &Party,
    mind: &Village,
    clock: Clock,
) -> Result<(), Fault> {
    if !available(p) {
        return Err(Fault::NoStore);
    }
    if !p.dataflash_erase(SLOT_OFF) {
        return Err(Fault::Erase);
    }
    let buf = encode(h, party, mind, clock);
    if !p.dataflash_write(SLOT_OFF, &buf) {
        return Err(Fault::Write);
    }
    // Read it back. An erase that silently did nothing is exactly the failure
    // this driver had on its first day, and it reported success throughout.
    let mut check = [0u8; RECORD];
    if !p.dataflash_read(SLOT_OFF, &mut check) {
        return Err(Fault::Read);
    }
    if let Some(i) = (0..RECORD).find(|&i| check[i] != buf[i]) {
        return Err(Fault::Verify(i, check[i]));
    }
    Ok(())
}

/// Forget the saved character.
pub fn erase(p: &mut dyn Platform) -> bool {
    available(p) && p.dataflash_erase(SLOT_OFF)
}
