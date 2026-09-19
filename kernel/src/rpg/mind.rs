//! What the village remembers about you, and how it forgets.
//!
//! # Why this is the one system the story needed
//!
//! The hero wakes at a ford with no memory and nobody in Vadul Alb knows them.
//! Zoe says it in her first line -- *"I have seen four people come down that
//! road. You are the fourth."* Until now that was scenery: every villager said
//! the same words on the hundredth conversation as on the first. This is the
//! other half of it. They learn who you are, and if you stay away long enough
//! they stop knowing.
//!
//! # Ebbinghaus, not a timer
//!
//! Hermann Ebbinghaus measured his own forgetting in 1885 and found retention
//! falling **exponentially** with time, and each recall flattening the curve --
//! spaced repetition. That is the model here, and it is cited for the same
//! reason D&D and Fallout are cited in [`super::hero`]: so the numbers can be
//! argued with rather than taken on trust.
//!
//! Exponential decay without floats is a fraction taken off once a day:
//! `strength -= strength / divisor`. A bigger divisor is a longer half-life.
//! Repetition raises the divisor, so the fourth conversation is worth far more
//! than the first -- which is what spacing does in the real curve.
//!
//! # No timestamps
//!
//! The obvious shape is `(strength, last_touched)` and decay computed on read.
//! That costs four bytes a trace and puts a clock read in every price lookup.
//! Decaying **eagerly**, once per day boundary the clock crosses, costs one
//! walk of seventy bytes on a day that already did a great deal of work, and
//! leaves a trace that is a single byte.
//!
//! # Presence, at last
//!
//! `CHA` has been on the character sheet since 0.22.0 and nothing has ever read
//! it. It reads it here: a memorable person is forgotten more slowly, which is
//! what Presence *is*. And the condition you were in when a memory formed sets
//! how strongly it forms -- turn up starving, thirsty or poisoned and you make
//! a poor impression, because you were not yourself.

use super::hero::Hero;
use super::world::NPCS;
use crate::i18n::Msg;

/// Kinds of memory, shortest-lived first.
///
/// The five map onto the five spans a person actually keeps things for: what
/// happened just now, a face, a name, a thing you did, and the one that does
/// not fade.
pub const M_PASSING: usize = 0;
pub const M_FACE: usize = 1;
pub const M_NAME: usize = 2;
pub const M_DEED: usize = 3;
pub const M_BOND: usize = 4;
pub const KINDS: usize = 5;

/// The divisor of the daily loss, before repetition and Presence lengthen it.
///
/// `strength -= strength / d` each day gives a half-life of about `0.7 * d`
/// days, so: a passing glance is gone overnight, a face lasts about a week, a
/// name a month, a deed most of a year, and a bond does not go.
static DECAY: [u16; KINDS] = [1, 10, 43, 256, 0];

/// What one reinforcement is worth. A conversation is worth more to a face than
/// to a name, because you see somebody every time and are told their name once.
static GAIN: [u8; KINDS] = [90, 45, 30, 20, 80];

/// How much repetition can lengthen a memory. Capped, because a curve that
/// flattens without limit is a memory that is simply permanent, and there is
/// already a kind for that.
const SEEN_MAX: u8 = 7;

/// What one villager holds about you.
///
/// Ten bytes: five strengths and five counts of how often each was renewed.
/// The counts could be three bits each and fit in two, and the packing would
/// cost more to read than the eight bytes are worth on a part with 32 KB.
#[derive(Clone, Copy)]
pub struct Mind {
    pub strength: [u8; KINDS],
    pub seen: [u8; KINDS],
}

impl Default for Mind {
    fn default() -> Self {
        Self::blank()
    }
}

impl Mind {
    pub const fn blank() -> Self {
        Self {
            strength: [0; KINDS],
            seen: [0; KINDS],
        }
    }

    /// Everything they hold, added up. What "how well do they know me" means.
    ///
    /// The short-term trace is **not** in the total. "You were just here" is not
    /// a thing you know about somebody, and counting it would mean the first
    /// conversation of a visit reporting that they knew your name.
    pub fn standing(&self) -> u32 {
        self.strength
            .iter()
            .enumerate()
            .filter(|(k, _)| *k != M_PASSING)
            .map(|(_, &s)| s as u32)
            .sum()
    }

    /// Reinforce one kind. `impression` is a percent, and it is the hero's
    /// condition: somebody who turns up starving is remembered less well.
    ///
    /// Presence is not read here. How memorable you are changes how slowly they
    /// forget, not how much goes in at the time -- which is the distinction
    /// Ebbinghaus's curve actually makes, between the height it starts at and
    /// how fast it falls.
    pub fn touch(&mut self, kind: usize, impression: u32) {
        if kind >= KINDS {
            return;
        }
        // Massed repetition is worth far less than spaced repetition -- the
        // other half of what Ebbinghaus found, and the job the short-term trace
        // does. It is full straight after a conversation and gone by morning,
        // so speaking to somebody four times in an afternoon is worth barely
        // more than speaking to them once, and coming back tomorrow is worth
        // the full amount again.
        let spacing = if kind == M_PASSING {
            255
        } else {
            255 - self.strength[M_PASSING] as u32
        };
        let gain = (((GAIN[kind] as u32 * impression) / 100) * spacing / 255).min(255) as u8;
        self.strength[kind] = self.strength[kind].saturating_add(gain);
        self.seen[kind] = (self.seen[kind] + 1).min(SEEN_MAX);
    }

    /// One day of forgetting.
    fn fade_one_day(&mut self, presence: i32) {
        for k in 0..KINDS {
            if DECAY[k] == 0 || self.strength[k] == 0 {
                continue;
            }
            // Repetition and Presence both lengthen the curve, and they do it
            // the same way -- by making the daily loss a smaller fraction.
            let stretch = 1 + self.seen[k] as u32 + presence.max(0) as u32;
            let divisor = DECAY[k] as u32 * stretch;
            // A loss that rounds to nothing is a memory that never fades, so a
            // live trace always loses at least one point a day.
            let loss = (self.strength[k] as u32 / divisor).max(1) as u8;
            self.strength[k] = self.strength[k].saturating_sub(loss);
        }
    }
}

/// How well somebody knows you, in the four bands the game actually acts on.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Known {
    Stranger,
    Face,
    Name,
    Trusted,
}

impl Known {
    pub fn of(standing: u32) -> Known {
        match standing {
            0..=29 => Known::Stranger,
            30..=89 => Known::Face,
            90..=189 => Known::Name,
            _ => Known::Trusted,
        }
    }

    pub fn msg(self) -> Msg {
        match self {
            Known::Stranger => Msg::RpgKnowStranger,
            Known::Face => Msg::RpgKnowFace,
            Known::Name => Msg::RpgKnowName,
            Known::Trusted => Msg::RpgKnowTrusted,
        }
    }

    /// Percent off what they ask, and percent on top of what they pay.
    ///
    /// Fifteen percent at the top is deliberately small. A village that halves
    /// its prices for a regular is a village whose economy the player solves by
    /// talking to everybody twice; this is a thank-you, not a strategy.
    pub fn goodwill(self) -> u32 {
        match self {
            Known::Stranger => 0,
            Known::Face => 5,
            Known::Name => 10,
            Known::Trusted => 15,
        }
    }
}

/// Everybody's memory of you.
#[derive(Clone, Copy)]
pub struct Village {
    pub minds: [Mind; NPC_COUNT],
}

pub const NPC_COUNT: usize = NPCS.len();

impl Default for Village {
    fn default() -> Self {
        Self::new()
    }
}

impl Village {
    pub const fn new() -> Self {
        Self {
            minds: [Mind::blank(); NPC_COUNT],
        }
    }

    pub fn of(&self, npc: u8) -> &Mind {
        &self.minds[(npc as usize).min(NPC_COUNT - 1)]
    }

    pub fn known(&self, npc: u8) -> Known {
        Known::of(self.of(npc).standing())
    }

    /// Reinforce one villager's memory of one thing.
    pub fn touch(&mut self, npc: u8, kind: usize, h: &Hero) {
        let i = (npc as usize).min(NPC_COUNT - 1);
        self.minds[i].touch(kind, impression(h));
    }

    /// Let `days` pass. Everybody forgets a little, and the people who have
    /// only seen you once forget most.
    pub fn fade(&mut self, days: u32, presence: i32) {
        if days == 0 {
            return;
        }
        // Capped because a save loaded after a very long sleep should not spend
        // a second walking a table whose entries are already zero. Thirty days
        // takes every decaying trace to nothing.
        for _ in 0..days.min(30) {
            for m in self.minds.iter_mut() {
                m.fade_one_day(presence);
            }
        }
    }
}

/// How well you come across right now, as a percent.
///
/// The "lifestyle" input: hunger, thirst and poison each take a quarter off the
/// impression you make. Three of them at once is less than half. This is state
/// the game already tracks, so nothing new has to be measured -- and it means
/// the sensible thing to do before asking a favour is to eat first.
pub fn impression(h: &Hero) -> u32 {
    let mut p = 100u32;
    if h.starving() {
        p = p * 3 / 4;
    }
    if h.parched() {
        p = p * 3 / 4;
    }
    if h.poisoned() {
        p = p * 3 / 4;
    }
    p
}
