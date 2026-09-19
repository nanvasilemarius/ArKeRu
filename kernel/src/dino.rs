//! A side-scrolling endless runner, rendered in the terminal with colour.
//!
//! Jump over obstacles; they arrive faster as your score climbs. The sprites
//! and layout are this project's own — it is the genre that is being borrowed,
//! not anyone's artwork.
//!
//! # Why the playfield is 40x8
//!
//! Everything is drawn over a 115200 baud serial line, so the frame *is* the
//! bandwidth budget. A 40x8 field with run-length colour changes costs roughly
//! 400 bytes, about 35 ms on the wire, which sustains ~20 fps. Doubling the
//! width would halve the frame rate. The field is redrawn from cursor-home
//! rather than cleared, because clearing makes it flicker.
//!
//! No allocation: one `[[u8; W]; H]` grid on the stack.

use crate::i18n::{self, Msg};
use crate::{kprint, kprintln, vt, Platform};

const W: usize = 40;
const H: usize = 8;
/// Row holding the ground line. The runner stands on top of it.
const GROUND: usize = 7;
/// Column the runner occupies (it is 2 wide).
const PX: usize = 4;

const MAX_OBSTACLES: usize = 4;

// Cell kinds. Kept as one grid rather than separate char/colour planes so the
// whole frame is 320 bytes.
const C_EMPTY: u8 = 0;
const C_GROUND: u8 = 1;
const C_PLAYER: u8 = 2;
const C_ROCK: u8 = 3;
const C_CLOUD: u8 = 4;

struct Obstacle {
    /// Signed so an obstacle can walk off the left edge before being reused.
    x: i16,
    h: u8,
}

pub fn play(p: &mut dyn Platform) {
    // Entering and leaving once, around the whole session: `run` calls itself
    // for "play again", and nested 1049h/1049l pairs lose the saved screen.
    vt::alt_enter(p);
    run(p);
    vt::alt_leave(p);
}

fn run(p: &mut dyn Platform) {
    let mut obstacles: [Obstacle; MAX_OBSTACLES] =
        [(); MAX_OBSTACLES].map(|_| Obstacle { x: -1, h: 0 });
    let mut y: i16 = 0; // height above ground, in rows
    let mut vel: i16 = 0;
    let mut score: u32 = 0;
    let mut frame: u32 = 0;
    // Cheap deterministic spread for obstacle spacing and height.
    let mut rng: u32 = 0x1357_9BDF;

    // Default colours, not whatever the previous screen left set: the field is
    // drawn as coloured glyphs against a `RESET`, so the background it clears
    // to has to be the same one those glyphs land on.
    vt::cls_normal(p);
    vt::hide_cursor(p);
    i18n::putln(p, Msg::DinoHowto);

    let mut next_ms = p.uptime_ms();

    loop {
        // --- input (non-blocking; the game never waits on a key) ---
        while let Some(k) = p.get() {
            match k {
                b'q' | b'Q' | 0x03 => {
                    vt::show_cursor(p);
                    kprintln!(p);
                    i18n::putln(p, Msg::DinoQuit);
                    return;
                }
                // Anything else jumps, but only from the ground -- no
                // mid-air double jumps.
                _ => {
                    if y == 0 {
                        vel = 3;
                    }
                }
            }
        }

        // --- frame pacing ---
        let now = p.uptime_ms();
        if now < next_ms {
            continue;
        }
        // Speeds up with score, floored so it stays playable.
        let interval = 90u64.saturating_sub((score / 12) as u64).max(45);
        next_ms = now + interval;
        frame = frame.wrapping_add(1);

        // --- physics ---
        y += vel;
        vel -= 1;
        if y <= 0 {
            y = 0;
            vel = 0;
        }

        // --- world ---
        for o in obstacles.iter_mut() {
            if o.h > 0 {
                o.x -= 1;
                if o.x < -1 {
                    o.h = 0; // retire it
                    score += 1;
                }
            }
        }
        // Spawn when the rightmost obstacle has moved far enough in, so there
        // is always room to react.
        let rightmost = obstacles.iter().filter(|o| o.h > 0).map(|o| o.x).max();
        if rightmost.is_none_or(|x| x < (W as i16 - 14)) {
            rng = rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            if let Some(slot) = obstacles.iter_mut().find(|o| o.h == 0) {
                slot.x = W as i16 - 1;
                slot.h = 1 + ((rng >> 17) & 1) as u8; // 1 or 2 tall
            }
        }

        // --- collision ---
        // The runner occupies rows (5-y)..(6-y); an obstacle of height h
        // occupies rows (7-h)..6. They overlap exactly when y <= h-1.
        for o in obstacles.iter() {
            if o.h > 0 && (o.x == PX as i16 || o.x == PX as i16 + 1) && y <= o.h as i16 - 1 {
                render(p, &obstacles, y, score, frame);
                vt::show_cursor(p);
                kprintln!(p);
                i18n::putln1(p, Msg::DinoOver, score);
                kprintln!(p);
                i18n::put(p, Msg::DinoAgain);
                loop {
                    if let Some(k) = p.get() {
                        kprintln!(p);
                        if k == b'y' || k == b'Y' {
                            return run(p);
                        }
                        return;
                    }
                }
            }
        }

        render(p, &obstacles, y, score, frame);
    }
}

fn render(p: &mut dyn Platform, obstacles: &[Obstacle; MAX_OBSTACLES], y: i16, score: u32, frame: u32) {
    let mut g = [[C_EMPTY; W]; H];

    // Ground line.
    for x in 0..W {
        g[GROUND][x] = C_GROUND;
    }

    // Two clouds drifting at a third of the scroll speed, for parallax.
    for (i, row) in [(0usize, 1usize), (1, 2)] {
        let cx = (W - 1) - (((frame / 3) as usize + i * 17) % W);
        g[row][cx] = C_CLOUD;
        if cx + 1 < W {
            g[row][cx + 1] = C_CLOUD;
        }
    }

    // Obstacles sit on the ground and grow upward.
    for o in obstacles.iter() {
        if o.h == 0 || o.x < 0 || o.x >= W as i16 {
            continue;
        }
        for k in 0..o.h as usize {
            g[GROUND - 1 - k][o.x as usize] = C_ROCK;
        }
    }

    // Runner: 2x2, standing on the ground unless jumping.
    let bottom = (GROUND - 1) as i16 - y;
    for dy in 0..2i16 {
        let r = bottom - dy;
        if r >= 0 && (r as usize) < H {
            g[r as usize][PX] = C_PLAYER;
            g[r as usize][PX + 1] = C_PLAYER;
        }
    }

    // Home the cursor rather than clearing: clearing flickers.
    kprint!(p, "\x1b[H");
    kprint!(p, "  {}SCORE {:04}{}   ", vt::sgr(vt::FG_BRIGHT), score, vt::RESET);
    i18n::put(p, Msg::DinoKeys);
    kprintln!(p);

    let mut cur = 255u8; // force a colour write on the first cell
    for row in g.iter() {
        kprint!(p, "  ");
        for &cell in row.iter() {
            if cell != cur {
                kprint!(
                    p,
                    "{}",
                    match cell {
                        C_PLAYER => vt::sgr(vt::FG_CYAN),
                        C_ROCK => vt::sgr(vt::FG_GREEN),
                        C_GROUND => vt::sgr(vt::FG_YELLOW),
                        C_CLOUD => vt::sgr(vt::FG_BLUE),
                        _ => vt::RESET,
                    }
                );
                cur = cell;
            }
            kprint!(
                p,
                "{}",
                match cell {
                    C_PLAYER => "\u{2588}",
                    C_ROCK => "\u{2588}",
                    C_GROUND => "\u{2500}",
                    C_CLOUD => "\u{2591}",
                    _ => " ",
                }
            );
        }
        kprint!(p, "{}", vt::RESET);
        cur = 255;
        kprintln!(p);
    }
}
