//! The seven streets of Vadul Alb, and the three roads that leave it.
//!
//! # A road, not an edge
//!
//! Until now movement was an untyped list of neighbours on each place: the
//! square simply knew it was next to five other locations. That is a graph, and
//! a graph is not a village. A village is a handful of named streets, each of
//! which runs *past* several things — which is why a road here holds an ordered
//! list of the places along it and the edges are derived from that. `Ulita
//! Fierarilor` runs from the square past the big forge to the ford forge, and
//! saying so once is better than writing the same adjacency into three places
//! and hoping they stay in agreement.
//!
//! It also puts a name on the screen. Choosing where to go now says which
//! street you would walk down and how long it takes, and both of those are
//! things a person standing in a village would know.
//!
//! # Length
//!
//! Every street has its own length in minutes, replacing the flat ten that
//! crossing the village used to cost. The crooked lane to the Old Tavern is
//! four minutes; the smiths' lane is ten; the ford road is twelve because it
//! follows the water round. The clock was built in 0.23.0 and until now had
//! only one number to read.
//!
//! # Wild roads
//!
//! Three of the ten leave the village, and those are the ones the dark and the
//! danger fields are for. They are also the only roads long enough that *when
//! you set out* matters: the bridge road is nearly two hours, so leaving at
//! seven in the evening means arriving in the dark whatever the light was when
//! you started.

use super::world::{
    F_BANDIT, F_BOAR, F_RIDER, F_SOLDIER, F_STRIGOI, F_WOLF, P_BRIDGE, P_FIELDS, P_FOREST,
    P_FORGE_BIG, P_FORGE_FORD, P_GATE, P_HEARTH, P_LANES, P_OLD, P_SQUARE, P_STAG, P_YARD,
};

/// The most roads that touch any one place. The square has five.
pub const MAX_EXITS: usize = 6;

/// The most that can be waiting on a road. The battle grid holds seven units
/// and three of them are yours.
pub const MAX_PACK: usize = 4;

pub struct Road {
    /// Proper noun, untranslated.
    pub name: &'static str,
    /// The places along it, in order. Adjacent entries are one segment apart.
    pub along: &'static [u8],
    /// Minutes to walk one segment.
    pub minutes: u32,
    /// Leaves the village. The dark matters on these and not on the streets.
    pub wild: bool,
    /// Chance in a hundred of meeting something, per journey.
    pub danger: u32,
    /// The pool an encounter is drawn from, independently per body -- so a
    /// forest ambush can be two wolves, or a wolf and a boar.
    pub foes: &'static [u8],
    /// How many are drawn.
    pub pack: u8,
}

pub static ROADS: [Road; 10] = [
    // The high street. Runs the length of the village and carries on north as
    // the road out, which is why the gate is on it.
    Road {
        name: "Ulita Mare",
        along: &[P_GATE, P_SQUARE, P_STAG],
        minutes: 8,
        wild: false,
        danger: 0,
        foes: &[],
        pack: 0,
    },
    Road {
        name: "Ulita Fierarilor",
        along: &[P_SQUARE, P_FORGE_BIG, P_FORGE_FORD],
        minutes: 10,
        wild: false,
        danger: 0,
        foes: &[],
        pack: 0,
    },
    Road {
        name: "Ulita Ograzii",
        along: &[P_SQUARE, P_YARD],
        minutes: 6,
        wild: false,
        danger: 0,
        foes: &[],
        pack: 0,
    },
    Road {
        name: "Ulita Caselor",
        along: &[P_SQUARE, P_LANES],
        minutes: 7,
        wild: false,
        danger: 0,
        foes: &[],
        pack: 0,
    },
    Road {
        name: "Poteca Vetrei",
        along: &[P_LANES, P_HEARTH],
        minutes: 5,
        wild: false,
        danger: 0,
        foes: &[],
        pack: 0,
    },
    // The shortest thing in the village, and the reason the Old Tavern is where
    // the men who do not want to be seen going somewhere go.
    Road {
        name: "Ulita Stramba",
        along: &[P_LANES, P_OLD],
        minutes: 4,
        wild: false,
        danger: 0,
        foes: &[],
        pack: 0,
    },
    // Follows the water round, so it is longer than the map suggests. It closes
    // the one loop in the village: square, forge, ford, gate, square.
    Road {
        name: "Drumul Vadului",
        along: &[P_FORGE_FORD, P_GATE],
        minutes: 12,
        wild: false,
        danger: 0,
        foes: &[],
        pack: 0,
    },
    // ------------------------------------------------------------ out
    Road {
        name: "Drumul Padurii",
        along: &[P_GATE, P_FOREST],
        minutes: 75,
        wild: true,
        danger: 55,
        foes: &[F_WOLF, F_BOAR],
        pack: 2,
    },
    // The safest of the three and the shortest, because it never leaves sight
    // of the village. Nothing can lie in wait in a cut field.
    Road {
        name: "Drumul Holdelor",
        along: &[P_FORGE_FORD, P_FIELDS],
        minutes: 55,
        wild: true,
        danger: 35,
        foes: &[F_BANDIT],
        pack: 2,
    },
    // Nearly two hours. Set out after five and you arrive in the dark whatever
    // the light was when you left, which is the whole point of a long road.
    Road {
        name: "Drumul Podului",
        along: &[P_GATE, P_BRIDGE],
        minutes: 110,
        wild: true,
        danger: 70,
        foes: &[F_SOLDIER, F_RIDER, F_STRIGOI],
        pack: 1,
    },
];

/// Where you can get to from `place`, and by which road.
///
/// Fills `(road, destination)` pairs and returns how many. Derived from
/// [`ROADS`] on every call rather than cached: it is ten short slices, the
/// alternative is a table that can disagree with the roads it came from, and
/// the caller does this once per screen.
pub fn exits(place: u8, out: &mut [(u8, u8); MAX_EXITS]) -> usize {
    let mut n = 0usize;
    for (i, road) in ROADS.iter().enumerate() {
        for (j, &at) in road.along.iter().enumerate() {
            if at != place {
                continue;
            }
            // A place in the middle of a street has a neighbour either side.
            for step in [j.wrapping_sub(1), j + 1] {
                if let Some(&to) = road.along.get(step) {
                    if n < MAX_EXITS {
                        out[n] = (i as u8, to);
                        n += 1;
                    }
                }
            }
        }
    }
    n
}

/// Minutes as a person would say them: `45 min`, `1 h`, `1 h 50`.
///
/// Minutes alone would put `110 min` beside a place two hours away, and a
/// number nobody converts in their head is a number nobody reads.
pub fn split(minutes: u32) -> (u32, u32) {
    (minutes / 60, minutes % 60)
}
