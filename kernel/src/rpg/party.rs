//! Companions: people who travel with you, fight beside you, have to be fed,
//! and eventually want something back.
//!
//! # The roster, not the party
//!
//! Every companion who has ever agreed to come is remembered here, whether or
//! not they are walking with you. That is the point of the change: leaving
//! somebody behind has to be reversible without forgetting who they were, and
//! two slots that get overwritten would mean a hunter who rejoins at full
//! morale with an empty pack and no memory of the task she asked you for.
//!
//! So [`Party::members`] is indexed by *kind* -- one entry per person in
//! [`KINDS`] -- and `present` says which of them is on the road with you.
//! [`TRAVELLING`] caps that at two, because four units plus the hero on a
//! twelve-by-eight field is a column rather than a party.
//!
//! # The brother
//!
//! He is there from the first minute rather than recruited, which is a
//! deliberate difference from the other three. A companion you *earn* is a
//! reward; a companion you *start with* is a responsibility, and the story is
//! about waking with no memory and finding somebody already depending on you.
//!
//! # What a companion is, mechanically
//!
//! Four stats rather than the hero's twelve. A companion is fought *alongside*,
//! not *played*, so the numbers that matter are the ones the battle reads: how
//! hard they hit, how hard they are to hit, how much they take, how far they
//! can strike from. The other eight would be a character sheet nobody opens.
//!
//! Each carries **morale** and **hunger**, both of which the village and the
//! clock drive, and a **stance**, which is the one thing about them the player
//! commands directly.

use super::item::{self, Bag, Effect, PACK_SLOTS};
use super::world;
use crate::i18n::Msg;

/// How many can walk with you at once.
pub const TRAVELLING: usize = 2;

/// One entry per person in [`KINDS`], travelling or not.
pub const ROSTER: usize = 4;

// ------------------------------------------------------------------- stance
//
// The one lever the player has over somebody else's behaviour, and the answer
// to "protect the hero, or attack". Three settings rather than a toggle,
// because a bow that charges is wrong in a different way from a shepherd who
// hangs back.

/// Stay beside the hero and hit whatever comes near them.
pub const GUARD: u8 = 0;
/// Close with the nearest enemy.
pub const PRESS: u8 = 1;
/// Do not move. Strike what comes into range.
pub const HOLD: u8 = 2;
pub const STANCES: u8 = 3;

pub static STANCE_NAME: [Msg; 3] = [
    Msg::RpgStanceGuard,
    Msg::RpgStancePress,
    Msg::RpgStanceHold,
];

// ------------------------------------------------------------------ recruiting

/// Deeds the hero has to their name, as bits in `Hero::flags[0]`.
///
/// A companion asks for proof rather than coin. Nobody in a village a hundred
/// and ten minutes from a bridge nobody crosses is persuaded by money.
pub const DEED_WOLF: u8 = 1 << 0;
pub const DEED_FOREST: u8 = 1 << 1;
pub const DEED_BRIDGE: u8 = 1 << 2;
pub const DEED_FIELDS: u8 = 1 << 3;

/// What somebody wants before they will come.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Want {
    /// Nothing. He was already there when you woke up.
    Nothing,
    /// A deed done, as one of the `DEED_` bits.
    Deed(u8),
    /// Any of the four vows sworn to Kondos.
    ///
    /// The oath has been in the game since 0.20.0 and has never once been read
    /// by anything. This is the first thing that checks it.
    Oath,
}

/// A companion's fixed description. The mutable part lives in [`Member`].
pub struct Kind {
    /// Proper noun, untranslated -- a name is not a word.
    pub name: &'static str,
    pub about: Msg,
    /// What they say when spoken to on the road.
    pub line: Msg,
    /// What they say when they will not come yet.
    pub refuse: Msg,
    /// What they say when they agree.
    pub join: Msg,
    /// What they ask of you, once they trust you enough to ask.
    pub ask: Msg,
    /// What they say when it is done.
    pub thanks: Msg,
    /// STR, END, AGI, REA -- the four the battle actually reads.
    pub stats: [u8; 4],
    pub hp: u8,
    /// How they draw on the battle grid.
    pub glyph: u8,
    /// Damage die.
    pub die: u32,
    /// Squares they can strike from. A bow is six.
    pub reach: u32,
    /// Health they put back on the hero, once a fight. Zero for everyone but
    /// the priest, which is the whole reason to take him instead of a bow.
    pub mend: u8,
    /// The villager they are when they are not travelling, or [`NO_NPC`].
    pub npc: u8,
    /// Where they are standing the first time you meet them.
    pub home: u8,
    pub want: Want,
    /// Where they want taking, once they ask.
    pub quest_at: u8,
    /// Stance they keep unless told otherwise.
    pub stance: u8,
}

/// A companion who is nobody in the village -- the brother, who exists only as
/// a companion and is never an NPC you can walk up to.
pub const NO_NPC: u8 = 0xff;

pub const C_BROTHER: usize = 0;
pub const C_HUNTER: usize = 1;
pub const C_SHEPHERD: usize = 2;
pub const C_PRIEST: usize = 3;

pub static KINDS: [Kind; ROSTER] = [
    // Young, quick, and not yet strong. He is worse than you at everything
    // except getting out of the way, which is the correct shape for somebody
    // you are supposed to worry about.
    Kind {
        name: "Andrei",
        about: Msg::RpgCompBrother,
        line: Msg::RpgSayBrother,
        refuse: Msg::RpgSayBrother,
        join: Msg::RpgJoinBrother,
        ask: Msg::RpgAskBrother,
        thanks: Msg::RpgThanksBrother,
        stats: [8, 9, 13, 12],
        hp: 14,
        glyph: b'a',
        die: 6,
        reach: 1,
        mend: 0,
        npc: NO_NPC,
        home: world::P_STAG,
        want: Want::Nothing,
        quest_at: world::P_BRIDGE,
        stance: PRESS,
    },
    // A bow, and the only thing on your side that can hit something before it
    // arrives. Fragile in exchange, and she starts on Hold because a bow that
    // charges is a worse knife.
    Kind {
        name: "Lenuta Kalfas",
        about: Msg::RpgCompHunter,
        line: Msg::RpgSayKHunter,
        refuse: Msg::RpgRefuseHunter,
        join: Msg::RpgJoinHunter,
        ask: Msg::RpgAskHunter,
        thanks: Msg::RpgThanksHunter,
        stats: [10, 10, 15, 14],
        hp: 12,
        glyph: b'k',
        die: 6,
        reach: 6,
        mend: 0,
        npc: world::N_HUNTER,
        home: world::P_HEARTH,
        // She said not to go past the second bridge. Somebody who has been up
        // there and come back is somebody she will walk with.
        want: Want::Deed(DEED_BRIDGE),
        quest_at: world::P_FOREST,
        stance: HOLD,
    },
    // Slow and hard to knock down. Twenty-two health on a field where the hero
    // has eighteen, and enemies go for whoever is nearest -- so on Guard he is
    // a wall you can stand behind.
    Kind {
        name: "Costache Mavros",
        about: Msg::RpgCompShepherd,
        line: Msg::RpgSayKShepherd,
        refuse: Msg::RpgRefuseShepherd,
        join: Msg::RpgJoinShepherd,
        ask: Msg::RpgAskShepherd,
        thanks: Msg::RpgThanksShepherd,
        stats: [14, 15, 8, 9],
        hp: 22,
        glyph: b'o',
        die: 6,
        reach: 1,
        mend: 0,
        npc: world::N_SHEPHERD,
        home: world::P_FIELDS,
        // "Lost four sheep this month and not one of them to wolves." Bring him
        // a wolf and he will find out whether he was right.
        want: Want::Deed(DEED_WOLF),
        quest_at: world::P_BRIDGE,
        stance: GUARD,
    },
    // A bad fighter who keeps you alive, which is a different job. Once a
    // fight, when the hero is below half, he mends instead of swinging.
    Kind {
        name: "Parintele Damian",
        about: Msg::RpgCompPriest,
        line: Msg::RpgSayKPriest,
        refuse: Msg::RpgRefusePriest,
        join: Msg::RpgJoinPriest,
        ask: Msg::RpgAskPriest,
        thanks: Msg::RpgThanksPriest,
        stats: [9, 11, 10, 11],
        hp: 16,
        glyph: b'p',
        die: 4,
        reach: 1,
        mend: 6,
        npc: world::N_PRIEST,
        home: world::P_LANES,
        want: Want::Oath,
        quest_at: world::P_FOREST,
        stance: GUARD,
    },
];

// --------------------------------------------------------------------- member

/// A companion as they currently are.
#[derive(Clone, Copy)]
pub struct Member {
    /// Whether they have ever agreed to come. False means you have not asked,
    /// or they said no.
    pub known: bool,
    /// Whether they are on the road with you right now.
    pub present: bool,
    /// Where they are waiting, when they are not with you.
    pub at: u8,
    pub hp: u8,
    /// 0..=100. Below [`MORALE_SHAKEN`] they hang back in a fight.
    pub morale: u8,
    /// 0..=100, rising. Above [`HUNGER_HUNGRY`] they will say so.
    pub hunger: u8,
    pub stance: u8,
    /// 0 not asked, 1 asked and waiting, 2 done.
    pub quest: u8,
    /// What they are carrying.
    pub pack: Bag<PACK_SLOTS>,
}

pub const MORALE_SHAKEN: u8 = 30;
/// The same threshold the hero uses -- see [`super::hero::HUNGER_HUNGRY`].
pub const HUNGER_HUNGRY: u8 = super::hero::HUNGER_HUNGRY;

pub const QUEST_NONE: u8 = 0;
pub const QUEST_ASKED: u8 = 1;
pub const QUEST_DONE: u8 = 2;

/// Morale at which somebody trusts you enough to ask for something.
pub const MORALE_ASKS: u8 = 60;

impl Member {
    pub const fn unknown() -> Self {
        Self {
            known: false,
            present: false,
            at: 0,
            hp: 0,
            morale: 0,
            hunger: 0,
            stance: GUARD,
            quest: QUEST_NONE,
            pack: Bag::new(),
        }
    }

    pub fn spec(kind: usize) -> &'static Kind {
        &KINDS[kind.min(ROSTER - 1)]
    }

    /// Bring somebody into the roster for the first time.
    pub fn recruit(&mut self, kind: usize) {
        let k = Self::spec(kind);
        self.known = true;
        self.present = true;
        self.at = k.home;
        self.hp = k.hp;
        self.morale = 70;
        self.hunger = 10;
        self.stance = k.stance;
        self.quest = QUEST_NONE;
    }

    pub fn name(&self, kind: usize) -> &'static str {
        Self::spec(kind).name
    }

    /// Whether the task they asked for is finished.
    ///
    /// The one permanent change a companion undergoes. It is small on purpose:
    /// two health and one to hit, plus a morale floor. A companion who becomes
    /// twice as good is a companion you stop worrying about, and worrying about
    /// them is the mechanic.
    pub fn bonded(&self) -> bool {
        self.quest == QUEST_DONE
    }

    pub fn hp_max(&self, kind: usize) -> u8 {
        Self::spec(kind).hp + if self.bonded() { 2 } else { 0 }
    }

    /// D&D modifier, same curve as the hero's. One rule for everybody on the
    /// field is worth more than a bespoke one for each.
    pub fn modifier(&self, kind: usize, stat: usize) -> i32 {
        (Self::spec(kind).stats[stat] as i32 - 10).div_euclid(2)
    }

    pub fn armour(&self, kind: usize) -> i32 {
        10 + self.modifier(kind, 2)
    }

    pub fn attack_bonus(&self, kind: usize) -> i32 {
        self.modifier(kind, 0) + 2 + if self.bonded() { 1 } else { 0 }
    }

    pub fn speed(&self, kind: usize) -> u32 {
        (3 + self.modifier(kind, 2).max(0)).min(6) as u32
    }

    pub fn initiative_bonus(&self, kind: usize) -> i32 {
        self.modifier(kind, 3)
    }

    /// Whether they will close with an enemy, or keep their distance.
    ///
    /// Morale still overrides the stance: somebody who is shaken guards
    /// whatever you told them to do, because the point of the number is that
    /// it takes the decision away from you.
    pub fn will_engage(&self) -> bool {
        self.morale >= MORALE_SHAKEN
    }

    /// The stance actually used this turn, morale included.
    pub fn footing(&self) -> u8 {
        if self.will_engage() {
            self.stance
        } else {
            GUARD
        }
    }

    /// Eat whatever is in pack slot `at`. Returns whether anything was eaten.
    ///
    /// A companion feeding himself is the same act as being fed: the transfer
    /// key puts food in his pack, and this takes it out again. Morale moves
    /// because being given food by somebody who is also hungry is the whole
    /// relationship.
    pub fn eat(&mut self, at: usize) -> bool {
        let Some(st) = self.pack.slots.get(at).copied() else {
            return false;
        };
        if st.is_empty() {
            return false;
        }
        let Effect::Eat { hunger } = item::spec(st.item).effect else {
            return false;
        };
        self.pack.take_at(at);
        self.hunger = self.hunger.saturating_sub(hunger);
        self.morale = self.morale.saturating_add(5).min(100);
        true
    }

    /// The floor morale will not fall below. Somebody who has been somewhere
    /// with you and come back does not break the way a stranger does.
    fn morale_floor(&self) -> u8 {
        if self.bonded() {
            50
        } else {
            0
        }
    }
}

// ---------------------------------------------------------------------- party

/// Everybody you know, and which of them is walking with you.
#[derive(Clone, Copy)]
pub struct Party {
    /// Indexed by kind, so a companion left at the bridge is still the same
    /// person -- same morale, same pack, same task -- when you go back for them.
    pub members: [Member; ROSTER],
}

impl Default for Party {
    fn default() -> Self {
        Self::new()
    }
}

impl Party {
    pub const fn new() -> Self {
        Self {
            members: [Member::unknown(); ROSTER],
        }
    }

    /// The party you wake up with.
    pub fn starting() -> Self {
        let mut p = Self::new();
        p.members[C_BROTHER].recruit(C_BROTHER);
        // He was sitting on that bank long enough to get cold, and he had the
        // sense to bring food. It is also the first thing the player can ask
        // him for, which is how the transfer key gets discovered.
        p.members[C_BROTHER].pack.add(item::I_BREAD, 1);
        p.members[C_BROTHER].pack.add(item::I_APPLES, 2);
        p
    }

    /// Who is on the road with you, as `(kind, member)`.
    pub fn travelling(&self) -> impl Iterator<Item = (usize, &Member)> {
        self.members
            .iter()
            .enumerate()
            .filter(|(_, m)| m.known && m.present)
    }

    pub fn count(&self) -> usize {
        self.travelling().count()
    }

    pub fn full(&self) -> bool {
        self.count() >= TRAVELLING
    }

    /// Whoever is waiting at `place` and could be picked up again.
    pub fn waiting_at(&self, place: u8) -> impl Iterator<Item = (usize, &Member)> + '_ {
        self.members
            .iter()
            .enumerate()
            .filter(move |(_, m)| m.known && !m.present && m.at == place)
    }

    /// Whether this villager is off travelling, so should not also be standing
    /// in their kitchen. The village list reads this before drawing an NPC.
    pub fn is_away(&self, npc: u8) -> bool {
        self.members.iter().enumerate().any(|(k, m)| {
            m.known && KINDS[k].npc == npc && (m.present || m.at != KINDS[k].home)
        })
    }

    /// Charge already-computed hunger points to everyone travelling.
    ///
    /// Points rather than minutes, and the difference matters. Dividing each
    /// step's minutes by the rate loses everything below it: a ten-minute walk
    /// is `10 / 40 = 0`, so a hero could cross the village four hundred times
    /// and never get hungry. The caller counts boundaries the clock crossed
    /// instead, which is exact at any step size.
    pub fn feed_time(&mut self, points: u8) {
        if points == 0 {
            return;
        }
        for m in self.members.iter_mut().filter(|m| m.known && m.present) {
            m.hunger = m.hunger.saturating_add(points).min(100);
            // Being hungry is not just a status line: it wears at the nerve
            // that decides whether you step forward.
            if m.hunger >= HUNGER_HUNGRY {
                let floor = m.morale_floor();
                m.morale = m.morale.saturating_sub(points).max(floor);
            }
        }
    }

    /// Everyone back on their feet. What a bed at the inn buys.
    pub fn rest(&mut self) {
        for (k, m) in self.members.iter_mut().enumerate() {
            if m.known && m.present {
                m.hp = KINDS[k].hp + if m.quest == QUEST_DONE { 2 } else { 0 };
                m.morale = m.morale.saturating_add(15).min(100);
            }
        }
    }
}
