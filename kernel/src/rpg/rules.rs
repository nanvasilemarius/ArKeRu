//! Six ways to play, chosen once and then lived with.
//!
//! # What a difficulty is allowed to change
//!
//! Not enemy hit points. Giving a wolf forty health on Hard makes the fight
//! longer without making it different, and the player learns nothing from it.
//! What changes here are the *rates* the rest of the game already runs on:
//! how fast hunger climbs, how much of a blow actually lands, how often the
//! road has something on it, and how much of you is left after you lose.
//!
//! Every one of those numbers already existed as a constant somewhere. This
//! module is where they stopped being constants.
//!
//! # Medium is the game as it was
//!
//! `hunger` 40, `harm` 100, `danger` 100 -- exactly the numbers 0.23.0 through
//! 0.26.0 shipped. That is deliberate: adding a difficulty setting must not
//! quietly re-balance the game for somebody who does not want one.
//!
//! # Realistic is a different kind of thing
//!
//! The first five are the same game with the dials moved. **Realistic turns two
//! systems on that are otherwise not there at all**: thirst, which is a second
//! clock running faster than hunger, and poison that eats your ability scores
//! rather than only your health. That is why it is last rather than sixth --
//! it is not "very hard plus a bit", it is a harsher world.

use crate::i18n::Msg;

pub const VERY_EASY: u8 = 0;
pub const EASY: u8 = 1;
pub const MEDIUM: u8 = 2;
pub const HARD: u8 = 3;
pub const VERY_HARD: u8 = 4;
pub const REALISTIC: u8 = 5;
pub const LEVELS: u8 = 6;

pub struct Rules {
    pub name: Msg,
    /// One sentence on what actually changes. Shown while choosing, because a
    /// difficulty whose effect you find out by playing it is a coin toss.
    pub about: Msg,
    /// Minutes of clock per point of hunger. Bigger is kinder.
    pub hunger_minutes: u32,
    /// Minutes per point of thirst. Read only when [`Rules::thirst`] is set.
    pub thirst_minutes: u32,
    /// Percent of a blow against your side that actually lands.
    pub harm: u32,
    /// Percent applied to a road's chance of having something on it.
    pub danger: u32,
    /// Percent of your maximum health you wake with after losing. Floored at
    /// one, so this can go to zero and still not kill anybody.
    pub revive: u32,
    /// Whether thirst is tracked at all.
    pub thirst: bool,
    /// Whether poison takes ability scores as well as health.
    pub deep_poison: bool,
}

pub static RULES: [Rules; LEVELS as usize] = [
    // For somebody who wants the village and the people and not the arithmetic.
    Rules {
        name: Msg::RpgDiffVeryeasy,
        about: Msg::RpgDiffVeryeasyAbout,
        hunger_minutes: 90,
        thirst_minutes: 0,
        harm: 50,
        danger: 50,
        revive: 100,
        thirst: false,
        deep_poison: false,
    },
    Rules {
        name: Msg::RpgDiffEasy,
        about: Msg::RpgDiffEasyAbout,
        hunger_minutes: 70,
        thirst_minutes: 0,
        harm: 75,
        danger: 75,
        revive: 60,
        thirst: false,
        deep_poison: false,
    },
    // The game as 0.23.0 through 0.26.0 shipped it, to the number.
    Rules {
        name: Msg::RpgDiffMedium,
        about: Msg::RpgDiffMediumAbout,
        hunger_minutes: 40,
        thirst_minutes: 0,
        harm: 100,
        danger: 100,
        revive: 25,
        thirst: false,
        deep_poison: false,
    },
    Rules {
        name: Msg::RpgDiffHard,
        about: Msg::RpgDiffHardAbout,
        hunger_minutes: 30,
        thirst_minutes: 0,
        harm: 125,
        danger: 125,
        revive: 10,
        thirst: false,
        deep_poison: false,
    },
    Rules {
        name: Msg::RpgDiffVeryhard,
        about: Msg::RpgDiffVeryhardAbout,
        hunger_minutes: 24,
        thirst_minutes: 0,
        harm: 150,
        danger: 150,
        revive: 0,
        thirst: false,
        deep_poison: false,
    },
    // Thirst runs at twenty minutes a point against hunger's twenty-five, so on
    // this setting you are thirsty before you are hungry -- which is the way
    // round it works, and the reason water is sold separately from bread.
    Rules {
        name: Msg::RpgDiffRealistic,
        about: Msg::RpgDiffRealisticAbout,
        hunger_minutes: 25,
        thirst_minutes: 20,
        harm: 175,
        danger: 175,
        revive: 0,
        thirst: true,
        deep_poison: true,
    },
];

pub fn of(level: u8) -> &'static Rules {
    &RULES[(level as usize).min(RULES.len() - 1)]
}

/// Scale a number by a percent, never rounding a real quantity down to nothing.
///
/// A blow reduced to zero on Very Easy would be a blow that did not happen,
/// which is a different promise from "half as much".
pub fn scaled(value: i32, percent: u32) -> i32 {
    if value <= 0 {
        return value;
    }
    ((value as u32 * percent) / 100).max(1) as i32
}
