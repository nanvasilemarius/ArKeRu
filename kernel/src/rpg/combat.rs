//! The battle grid: twelve squares by eight, on the terminal **and on the LED
//! panel at the same time**.
//!
//! # Why twelve by eight
//!
//! Because the panel is twelve by eight. That is the whole argument. Every
//! other size would have meant the physical display showing an abstraction of
//! the battle; this size means it shows *the battle*, one LED per square, and
//! the board on the desk becomes a second view of the same state rather than a
//! decoration that reacts to it.
//!
//! It costs nothing on the wire, which on a machine whose binding constraint is
//! 11 520 bytes per second is the other half of the argument.
//!
//! # The arithmetic
//!
//! D&D 5e, unmodified, because it is the part of D&D that most deserves
//! copying: `d20 + attack bonus` against the target's armour class, then a
//! damage die plus the ability modifier. One curve, one comparison, and the
//! player can do it in their head after two fights.
//!
//! Initiative is `d20 + Reaction`, rolled once at the start, which is why
//! Reaction is a stat rather than a derived value.

use super::hero::{Hero, Rng, AGI, STR};
use super::item;
use super::party::{self, Party};
use super::rules;
use super::world::{Foe, FOES};
use crate::i18n::{self, Chars, Msg, Piece};
use crate::screen::{self, Attr, Screen};
use crate::setup::{read_key, Key};
use crate::Platform;

pub const W: usize = 12;
pub const H: usize = 8;

/// The hero, up to two companions, and up to four enemies.
const MAX_UNITS: usize = 7;

/// Points of poison one bad bite leaves.
///
/// Six, against a hunger rate of twenty-five to ninety minutes a point, so it
/// takes between two and nine hours to work off -- long enough to be a reason
/// to turn back, short enough that it is not a second wound.
const POISON_DOSE: u8 = 6;

/// Top-left of the grid on screen. Each square is two columns wide, so the
/// aspect ratio comes out roughly square in a terminal cell.
const GRID_X: usize = 4;
const GRID_Y: usize = 5;
/// Where the combatant list starts.
const PANEL_X: usize = 34;
/// First row of the four-line log.
const LOG_Y: usize = 15;
const LOG_LINES: usize = 4;
const STATUS_Y: usize = 23;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Won,
    Lost,
    Fled,
}

/// Which side a unit is on, and where its numbers come from.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Hero,
    /// Index into the party.
    Ally(usize),
    /// Index into [`FOES`].
    Foe(u8),
}

#[derive(Clone, Copy)]
struct Unit {
    side: Side,
    x: i32,
    y: i32,
    hp: i32,
    init: i32,
    alive: bool,
    /// A once-a-fight thing already used. Only the priest has one, and
    /// tracking it on the unit rather than the member means it resets
    /// when the fight does without anybody having to remember to clear it.
    spent: bool,
}

impl Unit {
    /// `None` is the hero, whose name is borrowed from `Hero` and so cannot be
    /// stored here without borrowing the hero for the whole battle.
    fn name(&self) -> Option<&'static str> {
        match self.side {
            Side::Foe(i) => Some(FOES[i as usize].name),
            Side::Ally(i) => Some(party::Member::spec(i).name),
            Side::Hero => None,
        }
    }
    fn glyph(&self) -> u8 {
        match self.side {
            Side::Foe(i) => FOES[i as usize].glyph,
            Side::Ally(i) => party::Member::spec(i).glyph,
            Side::Hero => b'@',
        }
    }
    fn hostile(&self) -> bool {
        matches!(self.side, Side::Foe(_))
    }
}

/// One line of the fight log. Held as a message id plus at most one number, so
/// nothing has to be formatted into a buffer that does not exist.
#[derive(Clone, Copy)]
struct Line {
    /// `None` is the hero. Their name lives in `Hero` and is therefore
    /// borrowed from it -- storing it here would borrow the hero for the whole
    /// battle, which is precisely the value the battle has to mutate.
    who: Option<&'static str>,
    msg: Msg,
    n: i32,
    /// True when the number should be printed. `misses` has no number.
    numbered: bool,
}

struct Log {
    lines: [Option<Line>; LOG_LINES],
}

impl Log {
    fn new() -> Self {
        Self {
            lines: [None; LOG_LINES],
        }
    }
    fn push(&mut self, who: Option<&'static str>, msg: Msg, n: i32, numbered: bool) {
        for i in 0..LOG_LINES - 1 {
            self.lines[i] = self.lines[i + 1];
        }
        self.lines[LOG_LINES - 1] = Some(Line {
            who,
            msg,
            n,
            numbered,
        });
    }
}

/// Draw a message with its single `{}` replaced by a number.
///
/// `i18n::put1` does this straight to the console, which is exactly what a
/// composed screen must not do. Same rule, different destination.
fn msg1(s: &mut Screen, x: usize, y: usize, m: Msg, n: i32, attr: Attr) -> usize {
    let mut w = 0;
    i18n::spell(i18n::t(m), &mut |piece| match piece {
        Piece::Ch(c) => {
            if x + w < screen::COLS {
                s.put(x + w, y, screen::code(c as char), attr);
                w += 1;
            }
        }
        Piece::Hole(_) => w += super::num(s, x + w, y, n.max(0) as usize, attr),
    });
    w
}

fn chebyshev(a: &Unit, b: &Unit) -> i32 {
    (a.x - b.x).abs().max((a.y - b.y).abs())
}

/// Draw the grid, the combatant panel and the log.
fn draw(
    s: &mut Screen,
    hero: &Hero,
    units: &[Unit; MAX_UNITS],
    n: usize,
    log: &Log,
    moves: u32,
) {
    s.clear();
    s.border(Attr::Frame);
    s.rule(2, Attr::Frame);
    s.rule(14, Attr::Frame);
    s.text(2, 1, i18n::t(Msg::RpgBattle), Attr::Bright);

    // A dot per empty square rather than a blank, so the extent of the board is
    // visible -- an eight-row void with two glyphs in it does not read as a grid.
    for y in 0..H {
        for x in 0..W {
            s.put(GRID_X + x * 2, GRID_Y + y, screen::code('\u{00b7}'), Attr::Frame);
        }
    }
    for u in units.iter().take(n).filter(|u| u.alive) {
        let attr = match u.side {
            Side::Hero => Attr::Bright,
            Side::Ally(_) => Attr::Accent,
            Side::Foe(_) => Attr::Warn,
        };
        s.put(
            GRID_X + u.x as usize * 2,
            GRID_Y + u.y as usize,
            u.glyph(),
            attr,
        );
    }

    // Who is on the field, and how hurt. Your side first, then theirs, so the
    // split is readable without reading the names.
    let mut row = GRID_Y;
    for u in units.iter().take(n).filter(|u| !u.hostile()) {
        let who = u.name().unwrap_or_else(|| hero.name_str());
        let attr = if u.alive { Attr::Bright } else { Attr::Frame };
        s.text(PANEL_X, row, who, attr);
        let x = PANEL_X + who.chars().count() + 2;
        if u.alive {
            s.put(x, row, screen::HEART, Attr::Warn);
            super::num(s, x + 2, row, u.hp.max(0) as usize, Attr::Normal);
        } else {
            s.text(x, row, i18n::t(Msg::RpgDown), Attr::Frame);
        }
        row += 1;
    }
    row += 1;
    for u in units.iter().take(n).filter(|u| u.hostile()) {
        let who = u.name().unwrap_or("");
        let attr = if u.alive { Attr::Normal } else { Attr::Frame };
        s.text(PANEL_X, row, who, attr);
        let x = PANEL_X + who.chars().count() + 2;
        if u.alive {
            s.put(x, row, screen::HEART, Attr::Warn);
            super::num(s, x + 2, row, u.hp.max(0) as usize, attr);
        } else {
            s.text(x, row, i18n::t(Msg::RpgDown), Attr::Frame);
        }
        row += 1;
    }

    for (i, line) in log.lines.iter().enumerate() {
        let Some(l) = line else { continue };
        let y = LOG_Y + i;
        let w = s.text(2, y, l.who.unwrap_or_else(|| hero.name_str()), Attr::Accent);
        let at = if w == 0 { 2 } else { 2 + w + 1 };
        if l.numbered {
            msg1(s, at, y, l.msg, l.n, Attr::Normal);
        } else {
            s.text(at, y, i18n::t(l.msg), Attr::Normal);
        }
    }

    s.fill(1, STATUS_Y, screen::COLS - 2, 1, b' ', Attr::Normal);
    let mut sx = 2;
    sx += msg1(s, sx, STATUS_Y, Msg::RpgMovesLeft, moves as i32, Attr::Accent);
    sx += 2;
    let keys = i18n::t(Msg::RpgBattleKeys);
    let kw = keys.len();
    s.text(
        (screen::COLS - 2).saturating_sub(kw).max(sx + 1),
        STATUS_Y,
        keys,
        Attr::Frame,
    );
}

/// Mirror the field onto the panel. Bit 11 is column 0, matching `ttt`.
///
/// The panel is **no longer the battlefield**. Twelve by eight cannot show a
/// companion, a bow's reach or a poisoned unit, and keeping the fight that size
/// made the smallest display in the system the limit on the whole design. It
/// keeps the job it is actually good at: a glance that says how the fight is
/// going, in a place you can see without looking away from the screen.
fn led(p: &mut dyn Platform, units: &[Unit; MAX_UNITS], n: usize) {
    if !p.has_led_matrix() {
        return;
    }
    let mut rows = [0u16; H];
    for u in units.iter().take(n).filter(|u| u.alive) {
        let (x, y) = (u.x as usize, u.y as usize);
        if x < W && y < H {
            rows[y] |= 1 << (11 - x);
        }
    }
    p.led_matrix(&rows);
}

fn occupant(units: &[Unit; MAX_UNITS], n: usize, x: i32, y: i32) -> Option<usize> {
    (0..n).find(|&i| units[i].alive && units[i].x == x && units[i].y == y)
}

/// The nearest living unit on the other side, if any.
fn nearest(units: &[Unit; MAX_UNITS], n: usize, from: usize, hostile: bool) -> Option<usize> {
    let mut best: Option<(i32, usize)> = None;
    for i in 0..n {
        if !units[i].alive || i == from || units[i].hostile() != hostile {
            continue;
        }
        let d = chebyshev(&units[from], &units[i]);
        if best.map(|(bd, _)| d < bd).unwrap_or(true) {
            best = Some((d, i));
        }
    }
    best.map(|(_, i)| i)
}

/// One attack: `d20 + bonus` against armour class, then damage on a hit.
///
/// A natural 20 always hits and a natural 1 always misses, as in 5e. Without
/// those two rules an armoured enemy becomes literally unbeatable by a weak
/// character rather than merely hard, and the player cannot tell the difference
/// between "unlikely" and "impossible".
#[allow(clippy::too_many_arguments)]
fn attack(
    rng: &mut Rng,
    log: &mut Log,
    who: Option<&'static str>,
    bonus: i32,
    die: u32,
    dmg_bonus: i32,
    target_ac: i32,
    target_hp: &mut i32,
    target_name: Option<&'static str>,
    // Percent of the damage that lands: the difficulty, and the only thing
    // it changes about a fight. The rolls, the armour classes and the dice
    // are the same on every setting, so what the player works out about the
    // arithmetic on Easy is still true on Realistic.
    scale: u32,
) -> bool {
    let natural = rng.d20();
    let hit = natural == 20 || (natural != 1 && natural + bonus >= target_ac);
    if !hit {
        log.push(who, Msg::RpgMisses, 0, false);
        return false;
    }
    let mut dmg = rng.roll(die) as i32 + dmg_bonus;
    if natural == 20 {
        dmg += rng.roll(die) as i32;
    }
    let dmg = rules::scaled(dmg.max(1), scale);
    *target_hp -= dmg;
    log.push(who, Msg::RpgHitsFor, dmg, true);
    if *target_hp <= 0 {
        log.push(target_name, Msg::RpgFalls, 0, false);
    }
    true
}

/// Strike without moving, at whatever range the weapon has.
///
/// Returns whether the turn was spent. A refusal -- nothing in reach, or an
/// empty quiver -- says so in the log and hands the turn back, because losing a
/// turn to a key that did nothing is the cruellest thing an interface can do in
/// a fight the player is losing.
fn loose(
    hero: &mut Hero,
    rng: &mut Rng,
    units: &mut [Unit; MAX_UNITS],
    n: usize,
    log: &mut Log,
) -> bool {
    let (die, bonus, reach) = hero.strike();
    let Some(t) = nearest(units, n, 0, true) else {
        return false;
    };
    let Side::Foe(f) = units[t].side else {
        return false;
    };
    let spec = &FOES[f as usize];
    if chebyshev(&units[0], &units[t]) > reach as i32 {
        log.push(Some(spec.name), Msg::RpgOutOfReach, 0, false);
        return false;
    }
    // A missile weapon spends what it names as ammunition. A spear does not,
    // which is why `ammo` is a field rather than a guess from the reach.
    let ammo = hero.gear.ammo();
    if item::valid(ammo) && !hero.bag.take_one(ammo) {
        log.push(None, Msg::RpgNoArrows, 0, false);
        return false;
    }
    // 5e's rule: a thrown or loosed weapon adds Agility, a swung one Strength.
    let dmg_bonus = if item::valid(ammo) {
        hero.modifier(AGI)
    } else {
        hero.modifier(STR)
    };
    let mut hp = units[t].hp;
    attack(
        rng,
        log,
        None,
        bonus,
        die as u32,
        dmg_bonus,
        spec.ac,
        &mut hp,
        Some(spec.name),
        100,
    );
    units[t].hp = hp;
    if hp <= 0 {
        units[t].alive = false;
    }
    true
}

/// Walk `actor` towards `target` for at most `steps` squares.
///
/// Deliberately greedy. A pathfinder would be twenty times the code to solve a
/// problem an open field does not pose.
fn approach(units: &mut [Unit; MAX_UNITS], n: usize, actor: usize, target: usize, steps: u32) {
    let mut left = steps;
    while left > 0 {
        if chebyshev(&units[actor], &units[target]) <= 1 {
            break;
        }
        let sx = (units[target].x - units[actor].x).signum();
        let sy = (units[target].y - units[actor].y).signum();
        let (nx, ny) = (units[actor].x + sx, units[actor].y + sy);
        if occupant(units, n, nx, ny).is_some() {
            // Blocked: slide along one axis before giving up.
            let alt = [
                (units[actor].x + sx, units[actor].y),
                (units[actor].x, units[actor].y + sy),
            ];
            let free = alt.iter().find(|&&(ax, ay)| {
                ax >= 0
                    && ay >= 0
                    && ax < W as i32
                    && ay < H as i32
                    && occupant(units, n, ax, ay).is_none()
                    && (ax, ay) != (units[actor].x, units[actor].y)
            });
            match free {
                Some(&(ax, ay)) => {
                    units[actor].x = ax;
                    units[actor].y = ay;
                }
                None => break,
            }
        } else {
            units[actor].x = nx;
            units[actor].y = ny;
        }
        left -= 1;
    }
}

/// Fight. `foes` names which entries of [`FOES`] turn up.
pub fn battle(
    p: &mut dyn Platform,
    s: &mut Screen,
    hero: &mut Hero,
    party: &mut Party,
    rng: &mut Rng,
    foes: &[u8],
) -> Outcome {
    let mut units = [Unit {
        side: Side::Hero,
        x: 0,
        y: 0,
        hp: 0,
        init: 0,
        alive: false,
        spent: false,
    }; MAX_UNITS];
    let mut n = 0usize;

    units[n] = Unit {
        side: Side::Hero,
        x: 1,
        y: (H / 2) as i32,
        hp: hero.hp as i32,
        init: hero.initiative(rng),
        alive: true,
        spent: false,
    };
    n += 1;

    // Companions in roster order, placed either side of the hero.
    let mut side = 0i32;
    for idx in 0..party::ROSTER {
        let m = party.members[idx];
        if !m.known || !m.present || m.hp == 0 || n >= MAX_UNITS {
            continue;
        }
        units[n] = Unit {
            side: Side::Ally(idx),
            x: 0,
            y: ((H / 2) as i32 + if side == 0 { -1 } else { 1 }).clamp(0, H as i32 - 1),
            hp: m.hp as i32,
            init: rng.d20() + m.initiative_bonus(idx),
            alive: true,
            spent: false,
        };
        side += 1;
        n += 1;
    }

    for (i, &f) in foes.iter().enumerate() {
        if n >= MAX_UNITS {
            break;
        }
        let spec: &Foe = &FOES[f as usize];
        units[n] = Unit {
            side: Side::Foe(f),
            x: (W - 2) as i32,
            y: (1 + i * 2).min(H - 1) as i32,
            hp: spec.hp as i32,
            init: rng.d20() + spec.initiative,
            alive: true,
            spent: false,
        };
        n += 1;
    }

    // Turn order, highest initiative first. Seven elements, so an insertion
    // sort is both the simplest and the fastest thing available.
    let mut order = [0usize; MAX_UNITS];
    for (i, o) in order.iter_mut().enumerate() {
        *o = i;
    }
    for i in 1..n {
        let mut j = i;
        while j > 0 && units[order[j - 1]].init < units[order[j]].init {
            order.swap(j - 1, j);
            j -= 1;
        }
    }

    let mut log = Log::new();
    let mut turn = 0usize;
    let mut moves = hero.speed();

    let outcome = loop {
        let live_foes = units
            .iter()
            .take(n)
            .filter(|u| u.alive && u.hostile())
            .count();
        if live_foes == 0 {
            break Outcome::Won;
        }
        if !units[0].alive || units[0].hp <= 0 {
            // Not death. A hero who loses wakes somewhere with one hit point and
            // a story about it; permadeath in a game with one save slot is a way
            // of deleting somebody's afternoon.
            break Outcome::Lost;
        }

        let actor = order[turn % n];
        if !units[actor].alive {
            turn += 1;
            continue;
        }

        match units[actor].side {
            Side::Hero => {
                draw(s, hero, &units, n, &log, moves);
                s.flush(p);
                led(p, &units, n);

                let (dx, dy) = match read_key(p) {
                    Key::Up => (0, -1),
                    Key::Down => (0, 1),
                    Key::Left => (-1, 0),
                    Key::Right => (1, 0),
                    Key::Enter => {
                        moves = hero.speed();
                        turn += 1;
                        continue;
                    }
                    // Strike at reach: a spear at two squares, a bow at six.
                    // Without this a reach weapon is a worse melee weapon, and
                    // the number on the shop line would be a lie.
                    Key::Char(b'F') => {
                        if !loose(hero, rng, &mut units, n, &mut log) {
                            continue;
                        }
                        moves = hero.speed();
                        turn += 1;
                        continue;
                    }
                    Key::Esc => break Outcome::Fled,
                    _ => (0, 0),
                };
                if dx == 0 && dy == 0 {
                    continue;
                }

                let (nx, ny) = (units[0].x + dx, units[0].y + dy);
                if nx < 0 || ny < 0 || nx >= W as i32 || ny >= H as i32 {
                    continue;
                }

                match occupant(&units, n, nx, ny) {
                    Some(t) if units[t].hostile() => {
                        // Moving into an enemy is the attack. One less key to
                        // explain, and it is what the player reaches for anyway.
                        let Side::Foe(f) = units[t].side else { continue };
                        let spec = &FOES[f as usize];
                        let mut hp = units[t].hp;
                        let (die, bonus, _) = hero.strike();
                        attack(
                            rng,
                            &mut log,
                            None,
                            bonus,
                            die as u32,
                            hero.modifier(STR),
                            spec.ac,
                            &mut hp,
                            Some(spec.name),
                            100,
                        );
                        units[t].hp = hp;
                        if hp <= 0 {
                            units[t].alive = false;
                        }
                        moves = hero.speed();
                        turn += 1;
                    }
                    // A companion is not walked through; you go round.
                    Some(_) => continue,
                    None => {
                        units[0].x = nx;
                        units[0].y = ny;
                        moves -= 1;
                        if moves == 0 {
                            moves = hero.speed();
                            turn += 1;
                        }
                    }
                }
                continue;
            }

            Side::Ally(idx) => {
                let m = party.members[idx];
                let kind = party::Member::spec(idx);
                let reach = kind.reach as i32;

                // The priest, once a fight, when it is actually worth doing.
                // Checked before anything else because a companion whose job is
                // to keep you standing should not be swinging at a wolf while
                // you are on four health.
                //
                // Against `units[0].hp`, not `hero.hp`. The hero's health is
                // only written back into `Hero` when the fight ends, so reading
                // it here would compare against whatever it was when the fight
                // started -- which is full, which is why the first version of
                // this never fired once.
                let cap = hero.hp_max() as i32;
                if kind.mend > 0 && !units[actor].spent && units[0].hp * 2 <= cap {
                    units[actor].spent = true;
                    units[0].hp = (units[0].hp + kind.mend as i32).min(cap);
                    log.push(Some(kind.name), Msg::RpgMends, kind.mend as i32, true);
                } else {
                    // Morale overrides the stance rather than sitting beside it:
                    // the point of the number is that at some point it takes the
                    // decision away from you.
                    let footing = m.footing();
                    let target = nearest(&units, n, actor, true);
                    match target {
                        Some(t) => {
                            match footing {
                                party::PRESS => {
                                    approach(&mut units, n, actor, t, m.speed(idx))
                                }
                                // Stand with the hero. Enemies go for whoever is
                                // nearest, so a body beside you is a body between
                                // you and the next blow.
                                party::GUARD
                                    if chebyshev(&units[actor], &units[0]) > 1 =>
                                {
                                    approach(&mut units, n, actor, 0, m.speed(idx));
                                }
                                // HOLD: do not move. The only sensible thing for
                                // somebody whose weapon reaches six squares.
                                _ => {}
                            }
                            if chebyshev(&units[actor], &units[t]) <= reach {
                                if let Side::Foe(f) = units[t].side {
                                    let spec = &FOES[f as usize];
                                    let mut hp = units[t].hp;
                                    // Same rule as the hero's: a loosed weapon
                                    // adds Agility, a swung one Strength.
                                    let bonus = if kind.reach > 1 { 2 } else { 0 };
                                    attack(
                                        rng,
                                        &mut log,
                                        Some(kind.name),
                                        m.attack_bonus(idx),
                                        kind.die,
                                        m.modifier(idx, bonus),
                                        spec.ac,
                                        &mut hp,
                                        Some(spec.name),
                                        100,
                                    );
                                    units[t].hp = hp;
                                    if hp <= 0 {
                                        units[t].alive = false;
                                    }
                                }
                            }
                        }
                        // Nothing worth doing but staying near the hero.
                        None => approach(&mut units, n, actor, 0, m.speed(idx)),
                    }
                }
            }

            Side::Foe(f) => {
                let spec = &FOES[f as usize];
                // Enemies go for whoever is closest, which means a companion
                // standing in front genuinely draws blows away from the hero.
                let target = nearest(&units, n, actor, false).unwrap_or(0);
                approach(&mut units, n, actor, target, spec.speed);

                if chebyshev(&units[actor], &units[target]) <= 1 {
                    let (ac, who) = match units[target].side {
                        Side::Hero => (hero.armour(), None),
                        Side::Ally(i) => (
                            party.members[i].armour(i),
                            Some(party::Member::spec(i).name),
                        ),
                        Side::Foe(_) => (0, None),
                    };
                    if ac > 0 {
                        let mut hp = units[target].hp;
                        let landed = attack(
                            rng,
                            &mut log,
                            Some(spec.name),
                            spec.attack,
                            spec.damage,
                            spec.damage_bonus,
                            ac,
                            &mut hp,
                            who,
                            hero.rules().harm,
                        );
                        units[target].hp = hp;
                        if hp <= 0 {
                            units[target].alive = false;
                        }
                        // Poison is the hero's alone. A companion who could be
                        // poisoned would need a second bar the player cannot
                        // reach, since the cure is a salve out of *your* bag.
                        if landed
                            && spec.venom > 0
                            && units[target].side == Side::Hero
                            && rng.next() % 100 < spec.venom as u32
                        {
                            hero.poison = hero.poison.saturating_add(POISON_DOSE).min(20);
                            log.push(None, Msg::RpgPoisoned, 0, false);
                        }
                    }
                }
            }
        }

        // Show what just happened before the player is asked to respond to it.
        draw(s, hero, &units, n, &log, moves);
        s.flush(p);
        led(p, &units, n);
        p.delay_ms(350);

        turn += 1;
    };

    // Write the field back into the party. A companion who went down is not
    // dead -- they are out of this fight, and they remember it.
    for u in units.iter().take(n) {
        if let Side::Ally(i) = u.side {
            let m = &mut party.members[i];
            if u.alive {
                m.hp = u.hp.max(1) as u8;
                if outcome == Outcome::Won {
                    m.morale = m.morale.saturating_add(5).min(100);
                }
            } else {
                m.hp = 1;
                m.morale = m.morale.saturating_sub(25);
            }
        }
    }

    hero.hp = match outcome {
        // How much of you is left is the difficulty's last say. Very Easy
        // puts you back on your feet whole; Realistic leaves you the one
        // point the defeat text has always promised. It is floored at one
        // on every setting, because losing has never been death here.
        Outcome::Lost => {
            let cap = hero.hp_max() as u32;
            ((cap * hero.rules().revive) / 100).max(1) as u8
        }
        _ => units[0].hp.max(1) as u8,
    };
    outcome
}
