//! Things you can carry, the three slots you can fill, and the arithmetic of
//! weight.
//!
//! # Why an item is eighteen entries and not eighty
//!
//! Every item here is read by something. A sword changes the damage die, mail
//! changes the number an enemy has to roll, bread changes hunger, a torch
//! changes whether the north gate lets you out after dark. Nothing is in the
//! catalogue to pad a shop list, because an item that only has a price is a
//! number wearing a name.
//!
//! # Weight in grams
//!
//! A `u16` of grams reaches 65 kg, which is more than anybody is carrying one
//! of, and it makes an arrow weigh forty grams rather than nought. The earlier
//! sketch used hundred-gram units to fit a byte; that rounds a quiver of
//! twenty-four arrows to zero, and an encumbrance system whose lightest item is
//! free teaches the player to hoard it.
//!
//! Totals are `u32` grams. The carry limit comes from
//! [`super::hero::Hero::carry_kg`], which is Fallout's linear-in-Strength
//! formula, so the two ends of the comparison were designed apart and meet
//! here.
//!
//! # Stacks and slots
//!
//! A bag is a fixed array of `(item, count)` pairs. Adding tops up open stacks
//! before opening a new one, so a bag can never hold two half-stacks of bread
//! and refuse a third loaf -- which is the bug every inventory written in a
//! hurry has.

use crate::i18n::Msg;

/// Slots in the hero's bag.
pub const SLOTS: usize = 12;
/// Slots in a companion's pack. Smaller on purpose: he is a person who came
/// with you, not a second rucksack.
pub const PACK_SLOTS: usize = 4;

/// An empty slot, and "no item" wherever one is optional.
///
/// Item 0 is a real item, so the empty marker has to be out of range rather
/// than zero -- a mistake that costs one knife and is invisible for a week.
pub const NONE: u8 = 0xff;

/// Where an item goes when it is in use.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    /// Carried only. Food, remedies, arrows, torches.
    Loose,
    Weapon,
    Body,
    Hand,
}

/// What an item does. The variant *is* the item's purpose, so a thing that does
/// nothing cannot be described.
#[derive(Clone, Copy)]
pub enum Effect {
    /// A weapon. `die` is the damage die, `hit` adds to the attack roll, and
    /// `reach` is how many squares away it works from -- 1 for anything swung,
    /// 2 for a spear, and further for a bow.
    Strike { die: u8, hit: i8, reach: u8 },
    /// Worn or held. Adds to armour class.
    Guard { ac: i8 },
    /// Eaten. Takes points off hunger.
    Eat { hunger: u8 },
    /// Used. Puts health back.
    Mend { heal: u8 },
    /// Burned to walk the road after dark. One per trip.
    Burn,
    /// Spent by a weapon that names it as ammunition.
    Spend,
}

pub struct Item {
    pub name: Msg,
    pub slot: Slot,
    /// Grams.
    pub weight: u16,
    /// What a vendor asks for it. They buy back at half.
    pub value: u16,
    /// How many fit in one slot.
    pub stack: u8,
    /// Both hands, so no shield. The one reason to keep a sword over an axe.
    pub heavy: bool,
    /// What it spends when used, or [`NONE`].
    pub ammo: u8,
    /// Thirst it takes away. A field rather than an effect, because being
    /// wet is orthogonal to what a thing is *for*: soup feeds you and also
    /// slakes, tea mends you and also slakes, and water only slakes.
    /// Read only under rules that track thirst.
    pub wet: u8,
    pub effect: Effect,
}

pub const I_KNIFE: u8 = 0;
pub const I_HAMMER: u8 = 1;
pub const I_SPEAR: u8 = 2;
pub const I_AXE: u8 = 3;
pub const I_SWORD: u8 = 4;
pub const I_BOW: u8 = 5;
pub const I_ARROWS: u8 = 6;
pub const I_LEATHER: u8 = 7;
pub const I_MAIL: u8 = 8;
pub const I_SHIELD: u8 = 9;
pub const I_BREAD: u8 = 10;
pub const I_CHEESE: u8 = 11;
pub const I_MEAT: u8 = 12;
pub const I_APPLES: u8 = 13;
pub const I_SOUP: u8 = 14;
pub const I_SALVE: u8 = 15;
pub const I_TEA: u8 = 16;
pub const I_WATER: u8 = 17;
pub const I_TORCH: u8 = 18;

pub static ITEMS: [Item; 19] = [
    // Cheap, light, and better than nothing -- which is what the hero wakes up
    // with, so it is also the tutorial for the damage die.
    Item {
        name: Msg::RpgIKnife,
        slot: Slot::Weapon,
        weight: 300,
        value: 6,
        stack: 1,
        heavy: false,
        ammo: NONE,
        wet: 0,
        effect: Effect::Strike {
            die: 4,
            hit: 0,
            reach: 1,
        },
    },
    // A tool that fights. Anghel sells it as a tool and does not pretend
    // otherwise.
    Item {
        name: Msg::RpgIHammer,
        slot: Slot::Weapon,
        weight: 1900,
        value: 12,
        stack: 1,
        heavy: false,
        ammo: NONE,
        wet: 0,
        effect: Effect::Strike {
            die: 6,
            hit: 0,
            reach: 1,
        },
    },
    // Reach 2 is the whole point: it strikes a square before anything closes,
    // and it costs the shield to do it.
    Item {
        name: Msg::RpgISpear,
        slot: Slot::Weapon,
        weight: 1800,
        value: 16,
        stack: 1,
        heavy: true,
        ammo: NONE,
        wet: 0,
        effect: Effect::Strike {
            die: 6,
            hit: 0,
            reach: 2,
        },
    },
    Item {
        name: Msg::RpgIAxe,
        slot: Slot::Weapon,
        weight: 2200,
        value: 24,
        stack: 1,
        heavy: true,
        ammo: NONE,
        wet: 0,
        effect: Effect::Strike {
            die: 8,
            hit: 0,
            reach: 1,
        },
    },
    // The same die as the axe, one hand, and a plus to hit. It costs twice as
    // much, and that is the trade.
    Item {
        name: Msg::RpgISword,
        slot: Slot::Weapon,
        weight: 1500,
        value: 45,
        stack: 1,
        heavy: false,
        ammo: NONE,
        wet: 0,
        effect: Effect::Strike {
            die: 8,
            hit: 1,
            reach: 1,
        },
    },
    Item {
        name: Msg::RpgIBow,
        slot: Slot::Weapon,
        weight: 1000,
        value: 38,
        stack: 1,
        heavy: true,
        ammo: I_ARROWS,
        wet: 0,
        effect: Effect::Strike {
            die: 6,
            hit: 1,
            reach: 6,
        },
    },
    Item {
        name: Msg::RpgIArrows,
        slot: Slot::Loose,
        weight: 40,
        value: 1,
        stack: 24,
        heavy: false,
        ammo: NONE,
        wet: 0,
        effect: Effect::Spend,
    },
    Item {
        name: Msg::RpgILeather,
        slot: Slot::Body,
        weight: 4000,
        value: 22,
        stack: 1,
        heavy: false,
        ammo: NONE,
        wet: 0,
        effect: Effect::Guard { ac: 1 },
    },
    // Nine kilograms. On a Strength 8 hero that is a quarter of everything they
    // can carry, and the sheet says so before the road does.
    Item {
        name: Msg::RpgIMail,
        slot: Slot::Body,
        weight: 9000,
        value: 95,
        stack: 1,
        heavy: false,
        ammo: NONE,
        wet: 0,
        effect: Effect::Guard { ac: 3 },
    },
    Item {
        name: Msg::RpgIShield,
        slot: Slot::Hand,
        weight: 3000,
        value: 28,
        stack: 1,
        heavy: false,
        ammo: NONE,
        wet: 0,
        effect: Effect::Guard { ac: 2 },
    },
    Item {
        name: Msg::RpgIBread,
        slot: Slot::Loose,
        weight: 400,
        value: 2,
        stack: 6,
        heavy: false,
        ammo: NONE,
        wet: 0,
        effect: Effect::Eat { hunger: 20 },
    },
    Item {
        name: Msg::RpgICheese,
        slot: Slot::Loose,
        weight: 300,
        value: 3,
        stack: 6,
        heavy: false,
        ammo: NONE,
        wet: 0,
        effect: Effect::Eat { hunger: 25 },
    },
    // The best food per gram in the game, which is why a boar is worth killing.
    Item {
        name: Msg::RpgIMeat,
        slot: Slot::Loose,
        weight: 250,
        value: 5,
        stack: 6,
        heavy: false,
        ammo: NONE,
        wet: 0,
        effect: Effect::Eat { hunger: 35 },
    },
    Item {
        name: Msg::RpgIApples,
        slot: Slot::Loose,
        weight: 150,
        value: 1,
        stack: 10,
        heavy: false,
        ammo: NONE,
        wet: 8,
        effect: Effect::Eat { hunger: 12 },
    },
    // Heavy, and it does not keep -- two to a slot. Eat it where it is sold.
    Item {
        name: Msg::RpgISoup,
        slot: Slot::Loose,
        weight: 800,
        value: 4,
        stack: 2,
        heavy: false,
        ammo: NONE,
        wet: 30,
        effect: Effect::Eat { hunger: 50 },
    },
    Item {
        name: Msg::RpgISalve,
        slot: Slot::Loose,
        weight: 200,
        value: 14,
        stack: 4,
        heavy: false,
        ammo: NONE,
        wet: 0,
        effect: Effect::Mend { heal: 8 },
    },
    Item {
        name: Msg::RpgITea,
        slot: Slot::Loose,
        weight: 300,
        value: 6,
        stack: 4,
        heavy: false,
        ammo: NONE,
        wet: 30,
        effect: Effect::Mend { heal: 4 },
    },
    // Weight for weight the worst thing in the bag and the only answer to a
    // dry mouth. Seven hundred grams a skin, which on a Strength 3 hero is
    // four percent of everything they can carry -- and `Realistic` is
    // exactly the setting where that arithmetic starts to hurt.
    Item {
        name: Msg::RpgIWater,
        slot: Slot::Loose,
        weight: 700,
        value: 1,
        stack: 3,
        heavy: false,
        ammo: NONE,
        wet: 50,
        effect: Effect::Eat { hunger: 0 },
    },
    // Three coins buys one night on the road. Lenuta sells them and also tells
    // you not to need them.
    Item {
        name: Msg::RpgITorch,
        slot: Slot::Loose,
        weight: 900,
        value: 3,
        stack: 4,
        heavy: false,
        ammo: NONE,
        wet: 0,
        effect: Effect::Burn,
    },
];

pub fn spec(item: u8) -> &'static Item {
    &ITEMS[(item as usize).min(ITEMS.len() - 1)]
}

pub fn valid(item: u8) -> bool {
    (item as usize) < ITEMS.len()
}

pub fn name(item: u8) -> Msg {
    spec(item).name
}

/// What a vendor pays for one. Half, floored, but never nothing -- a shop that
/// takes an item for zero coin is a shop that eats it.
pub fn resale(item: u8) -> u16 {
    (spec(item).value / 2).max(1)
}

// -------------------------------------------------------------------- stacks

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Stack {
    pub item: u8,
    pub count: u8,
}

impl Stack {
    pub const EMPTY: Stack = Stack {
        item: NONE,
        count: 0,
    };

    pub fn is_empty(&self) -> bool {
        self.item == NONE || self.count == 0
    }
}

/// A fixed set of slots. Const-generic so the hero's twelve and a companion's
/// four are the same code with different arithmetic.
#[derive(Clone, Copy)]
pub struct Bag<const N: usize> {
    pub slots: [Stack; N],
}

impl<const N: usize> Default for Bag<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> Bag<N> {
    pub const fn new() -> Self {
        Self {
            slots: [Stack::EMPTY; N],
        }
    }

    /// Put `count` of `item` in. Returns however many did not fit.
    ///
    /// Open stacks are topped up before a new slot is opened, which is the
    /// difference between a bag that holds six loaves in one slot and one that
    /// holds three, three, and refuses the seventh.
    pub fn add(&mut self, item: u8, mut count: u8) -> u8 {
        if !valid(item) || count == 0 {
            return count;
        }
        let max = spec(item).stack;
        for s in self.slots.iter_mut() {
            if count == 0 {
                break;
            }
            if s.item == item && s.count < max {
                let take = (max - s.count).min(count);
                s.count += take;
                count -= take;
            }
        }
        for s in self.slots.iter_mut() {
            if count == 0 {
                break;
            }
            if s.is_empty() {
                let take = max.min(count);
                *s = Stack { item, count: take };
                count -= take;
            }
        }
        count
    }

    /// Whether `count` of `item` would fit, without putting anything in.
    pub fn room_for(&self, item: u8, count: u8) -> bool {
        if !valid(item) {
            return false;
        }
        let max = spec(item).stack;
        let mut room = 0u32;
        for s in self.slots.iter() {
            room += if s.is_empty() {
                max as u32
            } else if s.item == item {
                (max - s.count.min(max)) as u32
            } else {
                0
            };
            if room >= count as u32 {
                return true;
            }
        }
        false
    }

    /// Remove one from a slot, emptying it when the last goes.
    pub fn take_at(&mut self, at: usize) -> Option<u8> {
        let s = self.slots.get_mut(at)?;
        if s.is_empty() {
            return None;
        }
        let item = s.item;
        s.count -= 1;
        if s.count == 0 {
            *s = Stack::EMPTY;
        }
        Some(item)
    }

    /// Remove one of `item` wherever it is. Returns whether there was one.
    pub fn take_one(&mut self, item: u8) -> bool {
        match self.slots.iter().position(|s| s.item == item && s.count > 0) {
            Some(i) => self.take_at(i).is_some(),
            None => false,
        }
    }

    pub fn count_of(&self, item: u8) -> u32 {
        self.slots
            .iter()
            .filter(|s| s.item == item)
            .map(|s| s.count as u32)
            .sum()
    }

    pub fn weight(&self) -> u32 {
        self.slots
            .iter()
            .filter(|s| !s.is_empty())
            .map(|s| spec(s.item).weight as u32 * s.count as u32)
            .sum()
    }

    pub fn used(&self) -> usize {
        self.slots.iter().filter(|s| !s.is_empty()).count()
    }

    /// The first slot holding something edible, if any. What "ask your brother
    /// for food" resolves to.
    pub fn first_food(&self) -> Option<usize> {
        self.slots.iter().position(|s| {
            !s.is_empty() && matches!(spec(s.item).effect, Effect::Eat { .. })
        })
    }
}

// ---------------------------------------------------------------------- gear

/// What is in hand and on the body. Items here are **not** also in the bag --
/// equipping takes one out and unequipping puts it back -- but their weight
/// still counts, because a sword on your hip is a sword you are carrying.
#[derive(Clone, Copy)]
pub struct Gear {
    pub weapon: u8,
    pub body: u8,
    pub hand: u8,
}

impl Default for Gear {
    fn default() -> Self {
        Self::none()
    }
}

/// Bare hands: `d4`, no bonus, one square. Deliberately worse than the cheapest
/// knife, so the first six coins the player spends are obviously well spent.
pub const UNARMED: (u8, i32, u32) = (4, 0, 1);

impl Gear {
    pub const fn none() -> Self {
        Self {
            weapon: NONE,
            body: NONE,
            hand: NONE,
        }
    }

    pub fn weight(&self) -> u32 {
        [self.weapon, self.body, self.hand]
            .iter()
            .filter(|&&i| valid(i))
            .map(|&i| spec(i).weight as u32)
            .sum()
    }

    /// Armour class from what is worn and held.
    pub fn ac(&self) -> i32 {
        [self.body, self.hand]
            .iter()
            .filter(|&&i| valid(i))
            .map(|&i| match spec(i).effect {
                Effect::Guard { ac } => ac as i32,
                _ => 0,
            })
            .sum()
    }

    /// Damage die, attack bonus, and reach.
    pub fn strike(&self) -> (u8, i32, u32) {
        if !valid(self.weapon) {
            return UNARMED;
        }
        match spec(self.weapon).effect {
            Effect::Strike { die, hit, reach } => (die, hit as i32, reach as u32),
            _ => UNARMED,
        }
    }

    /// What the equipped weapon spends per attack, or [`NONE`].
    pub fn ammo(&self) -> u8 {
        if valid(self.weapon) {
            spec(self.weapon).ammo
        } else {
            NONE
        }
    }

    /// Whether the weapon needs both hands, which is what keeps a shield off it.
    pub fn two_handed(&self) -> bool {
        valid(self.weapon) && spec(self.weapon).heavy
    }

    /// The slot an item would occupy, and what is currently in it.
    pub fn in_slot(&self, slot: Slot) -> u8 {
        match slot {
            Slot::Weapon => self.weapon,
            Slot::Body => self.body,
            Slot::Hand => self.hand,
            Slot::Loose => NONE,
        }
    }

    pub fn set_slot(&mut self, slot: Slot, item: u8) {
        match slot {
            Slot::Weapon => self.weapon = item,
            Slot::Body => self.body = item,
            Slot::Hand => self.hand = item,
            Slot::Loose => {}
        }
    }

    pub fn holds(&self, item: u8) -> bool {
        valid(item) && (self.weapon == item || self.body == item || self.hand == item)
    }
}
