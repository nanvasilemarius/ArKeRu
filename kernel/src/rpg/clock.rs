//! The village calendar: a 24-hour clock, seven-day weeks, twelve months.
//!
//! # One number
//!
//! Everything is minutes since the start of the year, in a `u32`. Hours, days,
//! weekdays, months and the part of the day are all derived, so there is no way
//! for two fields to disagree — no "it is Tuesday the 31st of a thirty-day
//! month" — and the save carries four bytes rather than five fields that have
//! to be validated against each other on load.
//!
//! # Why the months are not translated
//!
//! `Gerar`, `Faurar`, `Martisor`, `Cuptor`, `Brumar` are the **old Romanian
//! folk names** for the months, and most of them say what the month is *for*:
//! `Cuptor` is the oven, `Brumar` is the frost-bringer, `Cireșar` is when the
//! cherries come. They are names, and this project does not translate names —
//! the same rule that keeps `Nikos Balan` and `Vadul Alb` in one spelling.
//!
//! An English-speaking player therefore reads the village's own word for the
//! month, which is the point. The weekdays *are* translated, because they are
//! ordinary words and a player needs to know when the week turns.
//!
//! # Thirty days
//!
//! Twelve months of thirty, so a year is 360 days. No leap year, no Gregorian
//! irregularity. A village calendar does not need to survive an astronomer, and
//! the arithmetic being exact means the date can be recomputed from the minute
//! count on every frame without a table.

use crate::i18n::Msg;

pub const MINUTES_PER_HOUR: u32 = 60;
pub const HOURS_PER_DAY: u32 = 24;
pub const MINUTES_PER_DAY: u32 = MINUTES_PER_HOUR * HOURS_PER_DAY;
pub const DAYS_PER_WEEK: u32 = 7;
pub const DAYS_PER_MONTH: u32 = 30;
pub const MONTHS: u32 = 12;
pub const DAYS_PER_YEAR: u32 = DAYS_PER_MONTH * MONTHS;

/// Folk names, in order. Proper nouns, so not translated.
pub static MONTH_NAMES: [&str; MONTHS as usize] = [
    "Gerar",     // the frost month
    "Faurar",    // the smith
    "Martisor",  // the little March, when the red-and-white thread is worn
    "Prier",     // the opening
    "Florar",    // the flowering
    "Ciresar",   // the cherries
    "Cuptor",    // the oven
    "Gustar",    // the tasting, when the harvest is first eaten
    "Rapciune",  // the wine month
    "Brumarel",  // the little frost
    "Brumar",    // the frost-bringer
    "Undrea",    // the needle, when the days are shortest
];

pub static WEEKDAY_NAMES: [Msg; DAYS_PER_WEEK as usize] = [
    Msg::RpgDayMon,
    Msg::RpgDayTue,
    Msg::RpgDayWed,
    Msg::RpgDayThu,
    Msg::RpgDayFri,
    Msg::RpgDaySat,
    Msg::RpgDaySun,
];

/// Roughly what the light is doing. Read by the gate, which will not let you
/// onto the north road in the dark, and by the location descriptions.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Light {
    Dawn,
    Morning,
    Afternoon,
    Evening,
    Night,
}

impl Light {
    pub fn msg(self) -> Msg {
        match self {
            Light::Dawn => Msg::RpgLightDawn,
            Light::Morning => Msg::RpgLightMorning,
            Light::Afternoon => Msg::RpgLightAfternoon,
            Light::Evening => Msg::RpgLightEvening,
            Light::Night => Msg::RpgLightNight,
        }
    }

    /// Whether the road out of the village is walkable.
    ///
    /// The hunter says it plainly the first time you speak to her -- *"go in
    /// the morning and come back before the light turns"* -- and this is that
    /// sentence made true. A warning the game does not enforce is scenery.
    pub fn travel_safe(self) -> bool {
        !matches!(self, Light::Night)
    }

    /// What the same hour looks like under a canopy.
    ///
    /// Only full day gets through. First light and dusk are both simply dark in
    /// there, which is why the forest wants a torch two hours before the
    /// village does and why coming home late from it is a different decision
    /// from coming home late from the fields.
    ///
    /// Not "one step darker": this enum is a time of day, and morning and
    /// afternoon are equally bright, so stepping along it would have said
    /// something about the clock rather than about the light.
    pub fn under_trees(self) -> Light {
        match self {
            Light::Morning | Light::Afternoon => self,
            _ => Light::Night,
        }
    }
}

/// Hour at which a night at the inn ends.
pub const WAKE_HOUR: u32 = 7;

/// The moment the game starts.
///
/// **Brumar, the fourth, seven in the morning.** Not arbitrary: Despina in the
/// Old Tavern already says *"They will tell you the pass is closed for the
/// snow. It is the fourth of the month and there is no snow."* That line was
/// written before there was a calendar. Now it is checkable, and it is true.
pub const START: u32 = ((10 * DAYS_PER_MONTH + 3) * MINUTES_PER_DAY) + WAKE_HOUR * MINUTES_PER_HOUR;

/// How long things take, in minutes.
///
/// Walking is **not** here. Every road has its own length in
/// [`super::road::ROADS`] -- the crooked lane is four minutes and the bridge
/// road is a hundred and ten -- and a single `WALK` constant beside them would
/// be a second answer to a question that already has one.
pub mod cost {
    /// A conversation.
    pub const TALK: u32 = 5;
    /// A bout in the yard, plus getting your breath back.
    pub const SPAR: u32 = 45;
    /// Working a wood over for game.
    pub const HUNT: u32 = 90;
    /// Working a cut field over for what the carts left.
    pub const GLEAN: u32 = 60;
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Clock {
    /// Minutes since the first midnight of the year.
    pub at: u32,
}

impl Default for Clock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock {
    pub const fn new() -> Self {
        Self { at: START }
    }

    pub const fn from_minutes(at: u32) -> Self {
        Self { at }
    }

    pub fn advance(&mut self, minutes: u32) {
        self.at = self.at.saturating_add(minutes);
    }

    /// Sleep until [`WAKE_HOUR`] tomorrow. Returns the minutes that passed, so
    /// the caller can charge hunger for them.
    pub fn sleep_until_morning(&mut self) -> u32 {
        let target = (self.day_index() + 1) * MINUTES_PER_DAY + WAKE_HOUR * MINUTES_PER_HOUR;
        // Waking *before* the wake hour would otherwise sleep for a negative
        // time and do nothing, which is exactly the case of dozing off at 3am.
        let target = if target <= self.at {
            target + MINUTES_PER_DAY
        } else {
            target
        };
        let slept = target - self.at;
        self.at = target;
        slept
    }

    fn day_index(&self) -> u32 {
        self.at / MINUTES_PER_DAY
    }

    pub fn hour(&self) -> u32 {
        (self.at % MINUTES_PER_DAY) / MINUTES_PER_HOUR
    }

    pub fn minute(&self) -> u32 {
        self.at % MINUTES_PER_HOUR
    }

    /// 1-based, as a person would say it.
    pub fn day_of_month(&self) -> u32 {
        (self.day_index() % DAYS_PER_MONTH) + 1
    }

    pub fn month(&self) -> usize {
        ((self.day_index() / DAYS_PER_MONTH) % MONTHS) as usize
    }

    pub fn month_name(&self) -> &'static str {
        MONTH_NAMES[self.month()]
    }

    /// Which day of the week. The year starts on a Monday, because something
    /// has to and nothing depends on it being otherwise.
    pub fn weekday(&self) -> usize {
        (self.day_index() % DAYS_PER_WEEK) as usize
    }

    pub fn weekday_msg(&self) -> Msg {
        WEEKDAY_NAMES[self.weekday()]
    }

    /// 1-based week of the year, for anything that wants a coarser tick than a
    /// day -- market days, a companion's patience, wages.
    pub fn week(&self) -> u32 {
        (self.day_index() % DAYS_PER_YEAR) / DAYS_PER_WEEK + 1
    }

    pub fn light(&self) -> Light {
        match self.hour() {
            5..=7 => Light::Dawn,
            8..=11 => Light::Morning,
            12..=16 => Light::Afternoon,
            17..=20 => Light::Evening,
            _ => Light::Night,
        }
    }

    /// Whether a day boundary lies between two readings. Used to charge the
    /// things that happen daily rather than continuously.
    pub fn days_between(&self, earlier: Clock) -> u32 {
        self.day_index().saturating_sub(earlier.day_index())
    }
}
