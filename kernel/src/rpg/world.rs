//! The village of **Vadul Alb**, its people, and the things on the road.
//!
//! # On the names
//!
//! Romanian given names and surnames against Greek ones, because a river
//! crossing on a trade road is exactly where those two would have met and not
//! quite merged. `Nikos Bălan`, `Anghel Petrakis`, `Căpitan Vasile Kondos` --
//! each one is a Greek half and a Romanian half, and the mixture is the
//! village's history stated without a paragraph of exposition.
//!
//! **Proper nouns are not translated.** `Vadul Alb` is `Vadul Alb` in English,
//! the way `VER` is `VER` in Romanian: a name is not a word. What is translated
//! is everything said *about* them.
//!
//! # On the thirty-five houses
//!
//! They exist and they are not thirty-five rooms. Walking a tile map of
//! identical cottages is a chore the 1980s invented and the 1990s abandoned;
//! what a village needs is that it be *the size it says it is*, which is a
//! matter of who lives there, not of how long it takes to cross. So the houses
//! are one location -- the lanes -- and their inhabitants are the ten roles
//! below.
//!
//! # On the coordinates
//!
//! Every place has one, in tenths of a minute of walking from the square. They
//! are not decoration: `MAP` draws to scale, so the straight line between two
//! neighbours has to come out as the minutes their road already charges, and
//! the positions were solved against [`super::road::ROADS`] rather than
//! sketched. Every street lands within 7% of its stated length.
//!
//! The exception is deliberate. `Drumul Vadului` measures 10.6 minutes across
//! the map and charges 12, because it follows the water round -- which is what
//! `road` says about it, and a map that agreed exactly would be contradicting
//! the reason the road is long.
//!
//! North is up and the gate is north, which is why the road out of the top of
//! the map is the one the gate stands on.

use super::item::{
    NONE, I_APPLES, I_ARROWS, I_AXE, I_BOW, I_BREAD, I_CHEESE, I_HAMMER, I_KNIFE, I_LEATHER,
    I_MAIL, I_MEAT, I_SALVE, I_SHIELD, I_SOUP, I_SPEAR, I_SWORD, I_TEA, I_TORCH,
    I_WATER,
};
use crate::i18n::Msg;

// ------------------------------------------------------------------- places

pub const P_SQUARE: u8 = 0;
pub const P_LANES: u8 = 1;
pub const P_STAG: u8 = 2;
pub const P_OLD: u8 = 3;
pub const P_HEARTH: u8 = 4;
pub const P_FORGE_BIG: u8 = 5;
pub const P_FORGE_FORD: u8 = 6;
pub const P_YARD: u8 = 7;
pub const P_GATE: u8 = 8;
/// The three at the far end of the three roads out.
pub const P_FOREST: u8 = 9;
pub const P_FIELDS: u8 = 10;
pub const P_BRIDGE: u8 = 11;

/// How many of [`PLACES`] are inside the village.
///
/// The rest are the far ends of the three roads out, which is a real
/// distinction and not just an ordering: they are hours away, they are the
/// only places `MAP` keeps drawing once the village has shrunk to a dot, and
/// nothing about them is walkable before dark.
pub const VILLAGE: usize = 9;

/// Proper noun, untranslated, like every other name here. It is the village
/// itself rather than a place in it, so it has no entry in [`PLACES`].
pub const VILLAGE_NAME: &str = "Vadul Alb";

/// Where you can walk from here is **not** here -- it lives in
/// [`super::road`], as an ordered list of the places along each street. A
/// neighbour list on the place and a street list on the road would be the same
/// facts written twice, and the two would drift.
pub struct Place {
    /// Proper noun, untranslated.
    pub name: &'static str,
    /// Where it stands, in tenths of a minute of walking from the square, x
    /// east and y south -- see the module header.
    pub at: (i16, i16),
    pub about: Msg,
    pub npcs: &'static [u8],
    /// Under trees rather than under the sky, so it is one step darker than
    /// the hour says. The one place worth carrying a torch before dusk.
    pub canopy: bool,
}

pub static PLACES: [Place; 12] = [
    Place {
        name: "Piata Vadului",
        at: (0, 0),
        about: Msg::RpgPSquare,
        npcs: &[N_ELDER, N_CHILD],
        canopy: false,
    },
    Place {
        name: "Ulitele",
        at: (-70, 0),
        about: Msg::RpgPLanes,
        npcs: &[N_HEALER, N_PRIEST],
        canopy: false,
    },
    Place {
        name: "Hanul Cerbului",
        at: (0, 80),
        about: Msg::RpgPStag,
        npcs: &[N_INNKEEP],
        canopy: false,
    },
    Place {
        name: "Carciuma Veche",
        at: (-100, 30),
        about: Msg::RpgPOld,
        npcs: &[N_TAVERNER],
        canopy: false,
    },
    Place {
        name: "Vatra",
        at: (-110, -30),
        about: Msg::RpgPHearth,
        npcs: &[N_COOK, N_HUNTER],
        canopy: false,
    },
    Place {
        name: "Fieraria Mare",
        at: (100, 0),
        about: Msg::RpgPForgeBig,
        npcs: &[N_SMITH],
        canopy: false,
    },
    Place {
        name: "Fieraria de la Vad",
        at: (105, -95),
        about: Msg::RpgPForgeFord,
        npcs: &[N_FARRIER],
        canopy: false,
    },
    Place {
        name: "Ograda de Arme",
        at: (-40, 50),
        about: Msg::RpgPYard,
        npcs: &[N_CAPTAIN],
        canopy: false,
    },
    Place {
        name: "Poarta de Miazanoapte",
        at: (0, -80),
        about: Msg::RpgPGate,
        npcs: &[N_MERCHANT],
        canopy: false,
    },
    // Nobody lives out here. What the forest has instead of people is meat,
    // and the two things that also want it.
    Place {
        name: "Padurea Neagra",
        at: (-550, -580),
        about: Msg::RpgPForest,
        npcs: &[],
        canopy: true,
    },
    Place {
        name: "Holdele",
        at: (620, 100),
        about: Msg::RpgPFields,
        npcs: &[N_SHEPHERD],
        canopy: false,
    },
    Place {
        name: "Podul de Piatra",
        at: (100, -1170),
        about: Msg::RpgPBridge,
        npcs: &[N_WATCH],
        canopy: false,
    },
];

// -------------------------------------------------------------------- people

/// What an NPC will do for you, beyond talking. One role each, so the village
/// has ten *functions* and not ten flavours of the same conversation.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Elder,
    Child,
    Healer,
    Priest,
    Shepherd,
    Innkeeper,
    Taverner,
    Cook,
    Hunter,
    Smith,
    Farrier,
    Captain,
    Merchant,
    Watch,
}

pub const N_ELDER: u8 = 0;
pub const N_CHILD: u8 = 1;
pub const N_HEALER: u8 = 2;
pub const N_PRIEST: u8 = 3;
pub const N_SHEPHERD: u8 = 4;
pub const N_INNKEEP: u8 = 5;
pub const N_TAVERNER: u8 = 6;
pub const N_COOK: u8 = 7;
pub const N_HUNTER: u8 = 8;
pub const N_SMITH: u8 = 9;
pub const N_FARRIER: u8 = 10;
pub const N_CAPTAIN: u8 = 11;
pub const N_MERCHANT: u8 = 12;
pub const N_WATCH: u8 = 13;

pub struct Npc {
    /// Proper noun, untranslated.
    pub name: &'static str,
    pub role: Msg,
    /// What they say when you first speak to them.
    pub line: Msg,
    pub kind: Role,
    /// What they will sell. Empty for the eight who only talk.
    ///
    /// Trade hangs off the person rather than the place, which is both truer
    /// and cheaper: buying from Dimitrie means walking to the forge *and*
    /// speaking to him, and the village menu does not grow a line per shop.
    pub stock: &'static [u8],
}

pub static NPCS: [Npc; 14] = [
    Npc {
        name: "Batrana Zoe Vlahos",
        role: Msg::RpgRoleElder,
        line: Msg::RpgSayElder,
        kind: Role::Elder,
        stock: &[],
    },
    Npc {
        name: "Mitrut",
        role: Msg::RpgRoleChild,
        line: Msg::RpgSayChild,
        kind: Role::Child,
        stock: &[],
    },
    Npc {
        name: "Ileana Sotiriu",
        role: Msg::RpgRoleHealer,
        line: Msg::RpgSayHealer,
        kind: Role::Healer,
        stock: &[I_SALVE, I_TEA],
    },
    Npc {
        name: "Parintele Damian",
        role: Msg::RpgRolePriest,
        line: Msg::RpgSayPriest,
        kind: Role::Priest,
        stock: &[],
    },
    Npc {
        name: "Costache Mavros",
        role: Msg::RpgRoleShepherd,
        line: Msg::RpgSayShepherd,
        kind: Role::Shepherd,
        stock: &[],
    },
    Npc {
        name: "Nikos Balan",
        role: Msg::RpgRoleInnkeep,
        line: Msg::RpgSayInnkeep,
        kind: Role::Innkeeper,
        stock: &[I_BREAD, I_CHEESE, I_SOUP, I_WATER],
    },
    Npc {
        name: "Despina Radu",
        role: Msg::RpgRoleTaverner,
        line: Msg::RpgSayTaverner,
        kind: Role::Taverner,
        stock: &[I_BREAD, I_CHEESE, I_TEA, I_WATER],
    },
    Npc {
        name: "Iorgu Stamatis",
        role: Msg::RpgRoleCook,
        line: Msg::RpgSayCook,
        kind: Role::Cook,
        stock: &[I_SOUP, I_MEAT, I_APPLES, I_BREAD, I_WATER],
    },
    Npc {
        name: "Lenuta Kalfas",
        role: Msg::RpgRoleHunter,
        line: Msg::RpgSayHunter,
        kind: Role::Hunter,
        stock: &[I_BOW, I_ARROWS, I_MEAT, I_TORCH],
    },
    Npc {
        name: "Dimitrie Ursu",
        role: Msg::RpgRoleSmith,
        line: Msg::RpgSaySmith,
        kind: Role::Smith,
        stock: &[I_SWORD, I_AXE, I_MAIL, I_SHIELD, I_HAMMER],
    },
    Npc {
        name: "Anghel Petrakis",
        role: Msg::RpgRoleFarrier,
        line: Msg::RpgSayFarrier,
        kind: Role::Farrier,
        stock: &[I_KNIFE, I_SPEAR, I_HAMMER, I_LEATHER],
    },
    Npc {
        name: "Capitan Vasile Kondos",
        role: Msg::RpgRoleCaptain,
        line: Msg::RpgSayCaptain,
        kind: Role::Captain,
        stock: &[],
    },
    Npc {
        name: "Yannis Cel Scurt",
        role: Msg::RpgRoleMerchant,
        line: Msg::RpgSayMerchant,
        kind: Role::Merchant,
        stock: &[I_KNIFE, I_LEATHER, I_TORCH, I_ARROWS, I_SALVE, I_APPLES],
    },
    // Two hours from the village and the only person out here. Kondos posts
    // him because of what walks down that road, and he sells what somebody
    // who has walked two hours actually wants.
    Npc {
        name: "Grigore Anastas",
        role: Msg::RpgRoleWatch,
        line: Msg::RpgSayWatch,
        kind: Role::Watch,
        stock: &[I_TORCH, I_ARROWS, I_BREAD, I_SALVE, I_WATER],
    },
];

// -------------------------------------------------------------------- enemies

/// One kind of thing that will fight you.
///
/// The first five each punish a different weakness rather than being a bigger
/// number than the last: speed, numbers, armour, raw damage, and cold.
///
/// The last two are the exception, and they are meant to be. `Ostas` and
/// `Calaret` are ranks, and the road to the bridge is where they are found,
/// because Kondos drills thirty-five houses that have never been attacked *for
/// a reason* -- and the reason has to be something the player can meet. They are
/// harder than everything else on purpose: two hours of walking should not end
/// in a wolf.
pub struct Foe {
    /// Proper noun, untranslated.
    pub name: &'static str,
    pub hp: u8,
    /// D&D armour class -- the target number an attack roll must reach.
    pub ac: i32,
    pub attack: i32,
    /// Damage die. `d6` is 6.
    pub damage: u32,
    pub damage_bonus: i32,
    /// Squares per turn.
    pub speed: u32,
    pub initiative: i32,
    /// How it draws on the grid and on the LED panel.
    pub glyph: u8,
    pub xp: u16,
    /// What is left on the field, or [`NONE`].
    ///
    /// A boar carries meat, which is the one loop the clock made necessary:
    /// hunger has to be answerable by going out, not only by paying Nikos.
    pub loot: u8,
    /// Chance in a hundred that a landed blow leaves poison in it.
    ///
    /// Two of the seven, and both earn it: a wolf bite goes bad, and a
    /// `Strigoi` is the reason the folklore exists. A poison every enemy
    /// carried would be a second health bar rather than a hazard.
    pub venom: u8,
}

pub const F_WOLF: u8 = 0;
pub const F_BANDIT: u8 = 1;
pub const F_BOAR: u8 = 2;
pub const F_SELLSWORD: u8 = 3;
pub const F_STRIGOI: u8 = 4;
pub const F_SOLDIER: u8 = 5;
pub const F_RIDER: u8 = 6;

pub static FOES: [Foe; 7] = [
    // Fast, fragile, and arrives before you are ready. Punishes low Reaction.
    Foe {
        name: "Lup",
        hp: 7,
        ac: 13,
        attack: 4,
        damage: 6,
        damage_bonus: 1,
        speed: 5,
        initiative: 2,
        glyph: b'l',
        xp: 40,
        loot: NONE,
        venom: 15,
    },
    // Ordinary, and never alone. Punishes fighting in the open.
    Foe {
        name: "Talhar",
        hp: 11,
        ac: 12,
        attack: 3,
        damage: 6,
        damage_bonus: 1,
        speed: 4,
        initiative: 1,
        glyph: b't',
        xp: 60,
        loot: I_KNIFE,
        venom: 0,
    },
    // Slow, heavy, and hits once for a great deal. Punishes standing still.
    Foe {
        name: "Mistret",
        hp: 18,
        ac: 11,
        attack: 5,
        damage: 10,
        damage_bonus: 2,
        speed: 3,
        initiative: -1,
        glyph: b'm',
        xp: 90,
        loot: I_MEAT,
        venom: 0,
    },
    // Armoured. Punishes a low attack bonus -- you simply miss.
    Foe {
        name: "Mercenar",
        hp: 14,
        ac: 16,
        attack: 5,
        damage: 8,
        damage_bonus: 2,
        speed: 4,
        initiative: 2,
        glyph: b'M',
        xp: 110,
        loot: I_LEATHER,
        venom: 0,
    },
    // Out of the folklore, and the reason COLD is a stat.
    Foe {
        name: "Strigoi",
        hp: 22,
        ac: 14,
        attack: 6,
        damage: 8,
        damage_bonus: 3,
        speed: 3,
        initiative: 0,
        glyph: b'S',
        xp: 160,
        loot: I_SALVE,
        venom: 45,
    },
    // A soldier off a company that is no longer paid. Shield, discipline and
    // nothing to go back to: high armour, steady damage, and he does not
    // break. Punishes a bad weapon rather than a bad stat.
    Foe {
        name: "Ostas",
        hp: 20,
        ac: 17,
        attack: 6,
        damage: 8,
        damage_bonus: 3,
        speed: 4,
        initiative: 2,
        glyph: b'O',
        xp: 200,
        loot: I_SHIELD,
        venom: 0,
    },
    // The rank above, and the hardest thing in the game. Fast enough to reach
    // you on the first turn and heavy enough that it matters when he does.
    Foe {
        name: "Calaret",
        hp: 30,
        ac: 18,
        attack: 7,
        damage: 10,
        damage_bonus: 4,
        speed: 6,
        initiative: 3,
        glyph: b'C',
        xp: 320,
        loot: I_MAIL,
        venom: 0,
    },
];
