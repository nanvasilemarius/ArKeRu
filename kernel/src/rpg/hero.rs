//! The hero: twelve stats, and the arithmetic that turns them into a person.
//!
//! # Where the formulas come from
//!
//! Three systems, each borrowed where it is strongest, and named so the
//! numbers can be argued with rather than taken on trust.
//!
//! **D&D 5e** supplies the spine. A score of 3-18, a modifier of
//! `(score - 10) / 2` rounded down, a d20 against a target number. That single
//! curve is why a 14 feels different from a 10 without anyone consulting a
//! table, and every check in this game is the same shape as every other.
//!
//! **Fallout** supplies the derived stats — the idea that carry weight and
//! reaction and resilience are *consequences* of the primaries rather than
//! separate dials. SPECIAL's carry formula is linear in Strength and so is
//! this one.
//!
//! **Skyrim** supplies the pools: a flat base plus a per-point increment, so a
//! low-Endurance character is fragile rather than unplayable.
//!
//! # Why twelve
//!
//! D&D uses six because six fits on a character sheet somebody is holding at a
//! table. Twelve fits here because the screen is 80 columns and the stats sit
//! in two columns of six with room for the modifier beside each. Every one of
//! them is read by something: nothing is on the sheet purely to be admired.

use super::item::{self, Bag, Effect, Gear, Slot, SLOTS};
use super::rules::{self, Rules};
use crate::i18n::Msg;

pub const STATS: usize = 12;
pub const NAME_MAX: usize = 12;

/// Point-buy limits.
///
/// Every stat starts at [`STAT_MIN`] and you spend [`POINTS`] raising them, one
/// point per step, to a ceiling of [`STAT_MAX`]. So the total is
/// `12 * 3 + 90 = 126`, an average of **10.5** -- right on the D&D baseline,
/// where the modifier crosses from negative to positive.
///
/// That average is the whole design. A flat spread gives a competent, unheroic
/// person; every stat pushed to 18 costs fifteen points and has to come out of
/// somewhere else. There is no allocation that is good at everything, and none
/// that is bad at everything either.
///
/// Rolling was the earlier version and it is gone. Rolling is more characterful
/// and it is also a slot machine: the interesting decision at that screen was
/// "again or not", which is not a decision about the character.
pub const STAT_MIN: u8 = 3;
pub const STAT_MAX: u8 = 18;
pub const POINTS: u16 = 90;

/// Who the hero is. Prose is second person throughout, so this changes almost
/// nothing on screen -- but the brother has to have something to call you.
pub const GENDER_SHE: u8 = 0;
pub const GENDER_HE: u8 = 1;

/// Hunger at which it starts to matter. Shared with companions, because one
/// number the player learns once is worth more than two that are nearly the
/// same.
pub const HUNGER_HUNGRY: u8 = 60;

/// Thirst at which it starts to matter. Lower than hunger's threshold, because
/// thirst is meant to be the thing that catches you first.
pub const THIRST_DRY: u8 = 50;

/// How much of a point of poison one clock boundary works off, and what a
/// poisoned person loses in the meantime.
///
/// One health per tick, and on `Realistic` one point off every ability score
/// while it lasts -- which is what "poison affects many stats" has to mean if
/// it is to mean anything the player can feel. A drain that only takes health
/// is a slower kind of damage; a drain that takes the modifiers changes what
/// you can do.
pub const POISON_BITE: u8 = 1;
pub const POISON_STAT_PENALTY: i32 = 1;

pub const STR: usize = 0;
pub const END: usize = 1;
pub const AGI: usize = 2;
pub const REA: usize = 3;
pub const FAT: usize = 4;
pub const COLD: usize = 5;
pub const VIS: usize = 6;
pub const HEA: usize = 7;
pub const SEN: usize = 8;
pub const INT: usize = 9;
pub const WIL: usize = 10;
pub const CHA: usize = 11;

pub static STAT_NAME: [Msg; STATS] = [
    Msg::RpgStatStr,
    Msg::RpgStatEnd,
    Msg::RpgStatAgi,
    Msg::RpgStatRea,
    Msg::RpgStatFat,
    Msg::RpgStatCold,
    Msg::RpgStatVis,
    Msg::RpgStatHea,
    Msg::RpgStatSen,
    Msg::RpgStatInt,
    Msg::RpgStatWil,
    Msg::RpgStatCha,
];

/// Eye colours. Cosmetic, except that `SEN` checks in low light read better on
/// a character the village has decided is strange-looking.
pub static EYES: [Msg; 6] = [
    Msg::RpgEyeBrown,
    Msg::RpgEyeGrey,
    Msg::RpgEyeGreen,
    Msg::RpgEyeBlue,
    Msg::RpgEyeBlack,
    Msg::RpgEyeAmber,
];

/// Xorshift32. Not cryptographic and not trying to be -- it is a dice cup.
///
/// Seeded from the uptime at the moment the player pressed a key, which on a
/// machine with no clock and no entropy source is the only unpredictable
/// number available.
pub struct Rng(u32);

impl Rng {
    pub fn new(seed: u32) -> Self {
        // Zero is a fixed point of xorshift and would return zero forever.
        Self(if seed == 0 { 0x9E37_79B9 } else { seed })
    }

    pub fn next(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    /// One die of `sides`, numbered from 1.
    pub fn roll(&mut self, sides: u32) -> u32 {
        1 + self.next() % sides
    }

    pub fn d20(&mut self) -> i32 {
        self.roll(20) as i32
    }

    /// 4d6, drop the lowest -- the classic D&D ability roll.
    ///
    /// It produces a mean near 12 rather than 10.5, which is the point: heroes
    /// are meant to be above the run of people, and a village of 3d6 farmers is
    /// what they are above.
    pub fn ability(&mut self) -> u8 {
        let mut d = [0u32; 4];
        let mut lowest = 0usize;
        for i in 0..4 {
            d[i] = self.roll(6);
            if d[i] < d[lowest] {
                lowest = i;
            }
        }
        let total: u32 = d.iter().sum::<u32>() - d[lowest];
        total as u8
    }
}

/// A saved character. Every field is a byte or two, because this goes into
/// data flash whole and the record has to stay small enough that a save is one
/// erase and one short program.
#[derive(Clone, Copy)]
pub struct Hero {
    pub name: [u8; NAME_MAX],
    pub name_len: u8,
    pub stats: [u8; STATS],
    pub age: u8,
    /// Centimetres above one metre, so it fits a byte.
    pub height: u8,
    pub weight: u8,
    pub eyes: u8,
    pub gender: u8,
    /// 0..=100, rising. Time is the only thing that moves it up.
    pub hunger: u8,
    /// 0..=100, rising. Only tracked when the rules say so.
    pub thirst: u8,
    /// Points of poison left to work through. Each one costs a health
    /// and, on `Realistic`, holds every ability score down while it lasts.
    pub poison: u8,
    /// Which of the six ways to play. Chosen once, at the start.
    pub rules: u8,
    pub hp: u8,
    pub level: u8,
    pub xp: u16,
    pub coin: u16,
    /// 0 means no oath sworn. 1..=4 index the vows.
    pub oath: u8,
    pub oath_state: u8,
    /// Where in the village the hero is standing.
    pub place: u8,
    pub flags: [u8; 8],
    pub bag: Bag<SLOTS>,
    pub gear: Gear,
}

/// What pressing ENTER on a bag row did. Reported rather than assumed, because
/// two of the five outcomes are refusals and a screen that silently does
/// nothing is indistinguishable from one that has crashed.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Use {
    /// Nothing happened, and nothing was wrong -- an empty slot, or a salve at
    /// full health.
    Nothing,
    Equipped,
    Stowed,
    Ate,
    Mended,
    /// The weapon takes both hands, so the shield cannot also be held.
    NeedsHands,
    /// Whatever came off had nowhere to go.
    NoRoom,
}

impl Default for Hero {
    fn default() -> Self {
        Self::blank()
    }
}

impl Hero {
    pub const fn blank() -> Self {
        Self {
            name: [0; NAME_MAX],
            name_len: 0,
            stats: [10; STATS],
            age: 25,
            height: 70,
            weight: 70,
            eyes: 0,
            gender: GENDER_SHE,
            hunger: 0,
            thirst: 0,
            poison: 0,
            rules: super::rules::MEDIUM,
            hp: 10,
            level: 1,
            xp: 0,
            coin: 12,
            oath: 0,
            oath_state: 0,
            place: 0,
            flags: [0; 8],
            bag: Bag::new(),
            gear: Gear::none(),
        }
    }

    /// A character at the start of point-buy: every stat at its floor.
    pub fn fresh() -> Self {
        let mut h = Self::blank();
        h.stats = [STAT_MIN; STATS];
        h.age = 24;
        h
    }

    /// Points already spent raising stats above the floor.
    pub fn spent(&self) -> u16 {
        self.stats
            .iter()
            .map(|&s| (s.saturating_sub(STAT_MIN)) as u16)
            .sum()
    }

    pub fn points_left(&self) -> u16 {
        POINTS.saturating_sub(self.spent())
    }

    /// Move one stat by one point, refusing anything that breaks the budget or
    /// the range. Returns whether it moved.
    pub fn adjust(&mut self, stat: usize, up: bool) -> bool {
        if stat >= STATS {
            return false;
        }
        let v = self.stats[stat];
        if up {
            if v >= STAT_MAX || self.points_left() == 0 {
                return false;
            }
            self.stats[stat] = v + 1;
        } else {
            if v <= STAT_MIN {
                return false;
            }
            self.stats[stat] = v - 1;
        }
        true
    }

    /// Fix the body to the stats once they are settled.
    ///
    /// Derived rather than chosen: a Strength 17 hero who weighs 55 kg is a
    /// contradiction the sheet should not be able to state.
    pub fn settle(&mut self, rng: &mut Rng) {
        self.height = 60 + rng.roll(20) as u8 + self.stats[STR] / 3;
        self.weight = 45 + self.stats[STR] * 2 + self.stats[END] + rng.roll(6) as u8;
        self.hp = self.hp_max();
    }

    pub fn name_str(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_len as usize]).unwrap_or("?")
    }

    pub fn set_name(&mut self, s: &[u8]) {
        let n = s.len().min(NAME_MAX);
        self.name[..n].copy_from_slice(&s[..n]);
        self.name_len = n as u8;
    }

    /// The rules this character is being played under.
    pub fn rules(&self) -> &'static Rules {
        rules::of(self.rules)
    }

    /// D&D 5e: `(score - 10) / 2`, rounded **down**, so 9 gives -1 and not 0.
    ///
    /// `div_euclid` rather than `/`: Rust truncates towards zero, which would
    /// round -1 up to 0 and quietly make every below-average stat average.
    ///
    /// Poison comes off here rather than off the scores themselves, so it is
    /// felt everywhere at once -- attack, armour, perception, initiative, carry
    /// -- without anything having to remember to ask about it, and so it can be
    /// worked off without having to remember what the scores used to be.
    pub fn modifier(&self, stat: usize) -> i32 {
        let base = (self.stats[stat] as i32 - 10).div_euclid(2);
        if self.poison > 0 && self.rules().deep_poison {
            base - POISON_STAT_PENALTY
        } else {
            base
        }
    }

    /// Whether thirst has reached the point of costing something. Always false
    /// where the rules do not track it.
    pub fn parched(&self) -> bool {
        self.rules().thirst && self.thirst >= THIRST_DRY
    }

    pub fn poisoned(&self) -> bool {
        self.poison > 0
    }

    /// Skyrim's shape: a floor nobody drops below, plus a per-point increment,
    /// plus D&D's hit die per level.
    pub fn hp_max(&self) -> u8 {
        let base = 10 + self.modifier(END) * 2 + (self.level as i32 - 1) * 5;
        base.clamp(4, 255) as u8
    }

    /// Fallout's carry weight: linear in Strength. In kilograms, because this
    /// is a Romanian village and not Nevada.
    ///
    /// The slope and intercept are fitted to *this* catalogue rather than
    /// copied from SPECIAL. The first version was `20 + STR * 4`, written when
    /// nothing had a weight; against real items it gives a Strength 3 hero
    /// 32 kg, and the heaviest thing anyone can assemble -- mail, shield, axe
    /// and a full bag -- comes to about fifteen. The limit could not be reached
    /// by any play, which is a mechanic that costs code and does nothing.
    ///
    /// At `10 + STR * 2` a weak hero has 16 kg and genuinely cannot wear mail
    /// and carry a week of food; a Strength 18 one has 46 kg and never thinks
    /// about it. That is the right shape: encumbrance should be a decision for
    /// the people who dumped Strength, and invisible to everyone else.
    pub fn carry_kg(&self) -> u32 {
        (10 + self.stats[STR] as i32 * 2).max(8) as u32
    }

    /// Everything on your back and on your hip, in grams.
    pub fn load_g(&self) -> u32 {
        self.bag.weight() + self.gear.weight()
    }

    /// Whether the load has passed what Strength allows.
    ///
    /// A threshold rather than a curve. A gradual penalty that starts at 60% of
    /// capacity is invisible -- the player never learns which loaf did it --
    /// whereas a line you can be told you have crossed is a decision.
    pub fn over_laden(&self) -> bool {
        self.load_g() > self.carry_kg() * 1000
    }

    /// D&D armour class: 10 plus the Agility modifier, plus what is worn.
    pub fn armour(&self) -> i32 {
        10 + self.modifier(AGI) + self.gear.ac()
    }

    /// Squares per turn on the battle grid. Capped so a very fast hero cannot
    /// cross the whole board before anything else acts, and cut by one when
    /// over-laden -- which is where carrying a mail shirt you cannot afford to
    /// carry gets paid for.
    /// Squares per turn, less a square for each thing dragging at you.
    ///
    /// Thirst costs movement rather than a slow health drain because a drain is
    /// something you notice afterwards and a lost square is something you
    /// notice while it matters.
    pub fn speed(&self) -> u32 {
        let base = (3 + self.modifier(AGI).max(0)).min(6) as u32;
        let drag = u32::from(self.over_laden()) + u32::from(self.parched());
        base.saturating_sub(drag).max(1)
    }

    /// The damage die, the full attack bonus and the reach of whatever is in
    /// hand -- or of a bare fist.
    pub fn strike(&self) -> (u8, i32, u32) {
        let (die, hit, reach) = self.gear.strike();
        (die, self.attack_bonus() + hit, reach)
    }

    /// D&D passive perception, taking whichever sense is sharpest. Three senses
    /// with one derived value each would be three numbers nobody reads; one
    /// derived value that asks which of them is best is a number that matters.
    pub fn perception(&self) -> i32 {
        let best = self
            .modifier(VIS)
            .max(self.modifier(HEA))
            .max(self.modifier(SEN));
        10 + best
    }

    /// Whether hunger has reached the point of costing something.
    ///
    /// Not a slow drain: below the threshold it is a status, above it a
    /// penalty. A stat that ticks down invisibly teaches the player nothing.
    pub fn starving(&self) -> bool {
        self.hunger >= HUNGER_HUNGRY
    }

    /// Skyrim's stamina pool, spent on sprinting and heavy blows.
    pub fn stamina(&self) -> u32 {
        (30 + self.stats[FAT] as u32 * 3).max(10)
    }

    /// Initiative: `d20 + REA`. The one place Reaction is the whole roll.
    pub fn initiative(&self, rng: &mut Rng) -> i32 {
        rng.d20() + self.modifier(REA)
    }

    /// The attack bonus: Strength plus proficiency, which in 5e is `2 + level/4`,
    /// less two for a dry mouth.
    pub fn attack_bonus(&self) -> i32 {
        let dry = if self.parched() { 2 } else { 0 };
        self.modifier(STR) + 2 + (self.level as i32 - 1) / 4 - dry
    }

    /// How much cold this hero shrugs off before it starts costing anything.
    /// Read by travel, not by combat.
    pub fn cold_threshold(&self) -> i32 {
        self.modifier(COLD) * 3
    }

    /// D&D 5e's milestone-free table, flattened: level `n` at `n^2 * 100` xp.
    pub fn xp_needed(&self) -> u16 {
        let n = self.level as u32 + 1;
        (n * n * 100).min(u16::MAX as u32) as u16
    }

    /// What you wake up with: the knife in your hand, and one meal between the
    /// two of you.
    ///
    /// A knife rather than nothing, because bare hands are `d4` and a player
    /// who never sees an equipped weapon never learns that the die changed.
    pub fn kit(&mut self) {
        self.gear.weapon = item::I_KNIFE;
        self.bag.add(item::I_BREAD, 1);
        self.bag.add(item::I_APPLES, 1);
    }

    /// Eat, drink, wear or wield whatever is in bag slot `at`.
    pub fn use_slot(&mut self, at: usize) -> Use {
        let Some(st) = self.bag.slots.get(at).copied() else {
            return Use::Nothing;
        };
        if st.is_empty() {
            return Use::Nothing;
        }
        let spec = item::spec(st.item);
        match spec.slot {
            Slot::Loose => match spec.effect {
                Effect::Eat { hunger } => {
                    // Water feeds nobody, so a skin at full stomach still has
                    // to be drinkable -- which it is not if this refuses on
                    // hunger alone.
                    if hunger == 0 && !(self.rules().thirst && self.thirst > 0) {
                        return Use::Nothing;
                    }
                    self.bag.take_at(at);
                    self.hunger = self.hunger.saturating_sub(hunger);
                    self.thirst = self.thirst.saturating_sub(spec.wet);
                    Use::Ate
                }
                Effect::Mend { heal } => {
                    // Refusing at full health is not pedantry: a salve is
                    // fourteen coin and the bag screen is where it is easiest
                    // to press ENTER on the wrong row. Being poisoned or dry is
                    // reason enough to take it anyway.
                    if self.hp >= self.hp_max() && self.poison == 0 && !self.parched() {
                        return Use::Nothing;
                    }
                    self.bag.take_at(at);
                    self.hp = self.hp.saturating_add(heal).min(self.hp_max());
                    self.thirst = self.thirst.saturating_sub(spec.wet);
                    // A salve draws half its healing out of the poison as well.
                    // It is the only cure that fits in a bag, which is why
                    // Ileana is worth the walk when it runs out.
                    self.poison = self.poison.saturating_sub(heal / 2);
                    Use::Mended
                }
                _ => Use::Nothing,
            },
            slot => self.equip(at, slot),
        }
    }

    /// Move a bag slot into gear, putting whatever it displaces back.
    ///
    /// Room is checked *before* anything moves. Doing it the other way needs an
    /// undo path for every failure, and an undo path that runs once a month is
    /// an undo path that is wrong.
    fn equip(&mut self, at: usize, slot: Slot) -> Use {
        let new = self.bag.slots[at].item;
        let spec = item::spec(new);
        let old = self.gear.in_slot(slot);
        if old == new {
            return Use::Nothing;
        }
        // A shield in the off hand and a weapon that needs both cannot both be
        // true. Wielding the weapon stows the shield; picking the shield up
        // while holding one refuses, because that way the player is told which
        // of the two the game thinks they wanted.
        if slot == Slot::Hand && self.gear.two_handed() {
            return Use::NeedsHands;
        }
        let stow_hand = slot == Slot::Weapon && spec.heavy && item::valid(self.gear.hand);

        // Gear stacks one to a slot, so taking the new item out always frees
        // exactly one.
        let need = usize::from(item::valid(old)) + usize::from(stow_hand);
        let free = self.bag.slots.iter().filter(|s| s.is_empty()).count() + 1;
        if need > free {
            return Use::NoRoom;
        }

        self.bag.take_at(at);
        if item::valid(old) {
            self.bag.add(old, 1);
        }
        if stow_hand {
            let hand = self.gear.hand;
            self.bag.add(hand, 1);
            self.gear.hand = item::NONE;
        }
        self.gear.set_slot(slot, new);
        Use::Equipped
    }

    /// Take something off and put it in the bag.
    pub fn unequip(&mut self, slot: Slot) -> Use {
        let held = self.gear.in_slot(slot);
        if !item::valid(held) {
            return Use::Nothing;
        }
        if self.bag.add(held, 1) > 0 {
            return Use::NoRoom;
        }
        self.gear.set_slot(slot, item::NONE);
        Use::Stowed
    }

    /// Returns true if the hero levelled.
    pub fn grant_xp(&mut self, amount: u16) -> bool {
        self.xp = self.xp.saturating_add(amount);
        if self.xp >= self.xp_needed() && self.level < 20 {
            self.level += 1;
            self.hp = self.hp_max();
            return true;
        }
        false
    }
}
