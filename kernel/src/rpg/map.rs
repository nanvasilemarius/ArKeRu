//! `MAP` -- Vadul Alb and its roads, drawn to scale.
//!
//! # Why it has to have a scale
//!
//! Because the village and the roads out of it are not the same size. `Ulita
//! Stramba` is four minutes end to end and `Drumul Podului` is a hundred and
//! ten, so any single picture is either a village with three enormous lines
//! leaving it or a region with an unreadable dot in the middle. A player
//! deciding whether to set out before dark needs both -- the streets to see
//! where they are, the region to see what leaving costs.
//!
//! So the map zooms, and [`screen::View`] does the arithmetic. Everything here
//! is in tenths of a minute of walking, which is the unit the rest of the game
//! already thinks in: a road's length is a number of minutes, and on this map
//! it is also a distance.
//!
//! # Level of detail, and what it actually does
//!
//! Two things, both of which are dropping information that would otherwise be
//! a smudge rather than a fact.
//!
//! **Labels are dropped when they collide.** Names are placed in order of how
//! much the player needs them -- where you are standing first, then the places
//! outside the village, then the rest -- and one that would land on a name
//! already written is not written. So zooming out thins the labels smoothly
//! instead of stacking them into a stripe, and no threshold had to be invented
//! to make that happen.
//!
//! **The village becomes one marker** once it is too small to have parts. At
//! six columns across, nine markers and nine names are not nine places; they
//! are a blot with a caption. Below that width the streets are not drawn at
//! all and `Vadul Alb` is a single point on three long roads, which is what
//! the village *is* from a day's walk away.
//!
//! # What it costs on the wire
//!
//! Panning is the worst thing that can be asked of a cell-diffing screen:
//! every cell moves, so the diff saves nothing and the frame costs a full
//! repaint. That is why a keypress pans a *quarter of the window* rather than
//! one column -- if a frame is going to cost two kilobytes, it should buy a
//! useful distance. The same argument sets the zoom ladder at powers of two.

use super::road::ROADS;
use super::world::{PLACES, VILLAGE, VILLAGE_NAME};
use super::{chrome, duration, keys_line};
use crate::i18n::{self, Chars, Msg, Text};
use crate::screen::{Attr, Screen, View};
use crate::setup::{read_key, Key};
use crate::Platform;

/// A place's position, widened for the viewport's arithmetic.
fn pt(at: (i16, i16)) -> (i32, i32) {
    (at.0 as i32, at.1 as i32)
}

/// The window, in cells. Rows 3..20 between the title rule and the legend.
const VIEW_X: usize = 1;
const VIEW_Y: usize = 3;
const VIEW_W: usize = 78;
const VIEW_H: usize = 18;
const FOOT_Y: usize = 21;
const SCALE_Y: usize = 22;

/// Tenths of a minute to a cell column. Powers of two, because a pan and a
/// zoom cost the same full repaint and a ladder finer than this would spend
/// several of them getting anywhere.
const ZOOMS: [i32; 6] = [2, 4, 8, 16, 32, 64];

/// Opens showing the whole village and nothing beyond it, which is the
/// question `MAP` is usually being asked.
const START: usize = 2;

/// How wide Vadul Alb is, corner to corner, in tenths of a minute.
const VILLAGE_WIDE: i32 = 215;

/// The most things that can hold a piece of a row at once: a marker and a name
/// for every place, plus a street name for every road.
const LABELS: usize = PLACES.len() * 2 + ROADS.len();

/// What the current scale can carry.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Detail {
    /// Streets named as well as places.
    Streets,
    /// Places, and as many of their names as fit.
    Places,
    /// The village is one marker on three long roads.
    Village,
}

impl Detail {
    /// Chosen by measuring the village, not by indexing the zoom ladder -- the
    /// question is whether there is room for the parts, and room is cells.
    fn of(v: &View) -> Self {
        match v.cells(VILLAGE_WIDE) {
            n if n >= 50 => Detail::Streets,
            n if n >= 12 => Detail::Places,
            _ => Detail::Village,
        }
    }
}

/// Names already written, as the row and columns they occupy.
///
/// A label that would touch one of these is dropped. Twelve places and ten
/// roads is a small enough list that comparing against all of it beats any
/// cleverer structure.
struct Taken {
    at: [(i32, i32, i32); LABELS],
    n: usize,
}

impl Taken {
    fn new() -> Self {
        Self {
            at: [(0, 0, 0); LABELS],
            n: 0,
        }
    }

    /// Claim `row`, columns `from..to`, unless something is already there.
    fn claim(&mut self, row: i32, from: i32, to: i32) -> bool {
        if self.n == LABELS || row < 0 || row >= VIEW_H as i32 || from < 0 || to > VIEW_W as i32 {
            return false;
        }
        // A one-column gap either side, so two names never read as one word.
        if self.at[..self.n]
            .iter()
            .any(|&(r, a, b)| r == row && from <= b && a <= to)
        {
            return false;
        }
        self.at[self.n] = (row, from - 1, to + 1);
        self.n += 1;
        true
    }
}

/// Draw a marker and reserve the cell it is in.
///
/// Reserving matters: a name is wider than the thing it names, so without this
/// the first label written across a row rubs out every marker under it, and
/// the place stops existing rather than merely going unlabelled.
fn mark(s: &mut Screen, v: &View, taken: &mut Taken, w: (i32, i32), ch: u8, attr: Attr) {
    let (col, row) = v.cell_of(w);
    taken.claim(row, col, col);
    v.mark(s, w, ch, attr);
}

/// Write `text` beside a world point, on whichever side it fits, or not at all.
fn label(s: &mut Screen, v: &View, taken: &mut Taken, w: (i32, i32), text: impl Chars, attr: Attr) {
    // A name whose marker fell outside the window has nothing to point at, and
    // reads as a word floating against the border rather than as a label. It
    // goes wherever the marker went.
    if v.at(w).is_none() {
        return;
    }
    let (col, row) = v.cell_of(w);
    let len = text.len() as i32;
    // Six places to try, in the order a person drawing this by hand would:
    // beside the marker first, then the rows above and below it. Without the
    // last four, a place in a crowded row goes unnamed even when the space
    // immediately over it is empty -- which is how the square you are standing
    // in ended up as an unlabelled `@` at the default zoom.
    for (dx, dy) in [
        (2, 0),
        (-2 - len, 0),
        (0, -1),
        (0, 1),
        (-len, -1),
        (-len, 1),
    ] {
        let from = col + dx;
        if taken.claim(row + dy, from, from + len) {
            v.label(s, w, (dx, dy), text, attr);
            return;
        }
    }
}

fn draw(s: &mut Screen, v: &View, here: u8) {
    s.clear();
    chrome(s, Text::Plain(VILLAGE_NAME), i18n::t(Msg::RpgMap));
    let detail = Detail::of(v);
    let mut taken = Taken::new();

    // Roads first, so a marker is never painted over by the road it is on.
    for r in ROADS.iter() {
        if detail == Detail::Village && !r.wild {
            continue;
        }
        let attr = if r.wild { Attr::Warn } else { Attr::Frame };
        for pair in r.along.windows(2) {
            let (a, b) = (
                pt(PLACES[pair[0] as usize].at),
                pt(PLACES[pair[1] as usize].at),
            );
            v.line(s, a, b, v.stroke(a, b), attr);
        }
    }

    // Every marker before any name. A name is wider than the thing it names,
    // so a marker drawn afterwards lands in the middle of somebody else's
    // label -- which is how `Vadul Alb` came to be spelled `Vadul Aob`.
    let village = detail == Detail::Village;
    if village {
        // Nine places inside six columns is not nine places. The square stands
        // for all of them, and the three roads out keep their far ends.
        mark(s, v, &mut taken, pt(PLACES[0].at), b'@', Attr::Bright);
    } else {
        for (i, place) in PLACES.iter().enumerate().take(VILLAGE) {
            let you = i as u8 == here;
            mark(
                s,
                v,
                &mut taken,
                pt(place.at),
                if you { b'@' } else { b'o' },
                if you { Attr::Bright } else { Attr::Normal },
            );
        }
    }
    // Outside the village, always. These are the whole reason the map zooms.
    for place in PLACES.iter().skip(VILLAGE) {
        mark(s, v, &mut taken, pt(place.at), b'o', Attr::Warn);
    }

    // Names, most wanted first: where you are standing, then what is out
    // there, then everything else. Whatever does not fit is dropped by
    // `claim`, so the order is the priority.
    let (whose, name) = if village {
        (PLACES[0].at, Text::Plain(VILLAGE_NAME))
    } else {
        let p = &PLACES[here as usize];
        (p.at, Text::Plain(p.name))
    };
    label(s, v, &mut taken, pt(whose), name, Attr::Bright);
    for place in PLACES.iter().skip(VILLAGE) {
        label(s, v, &mut taken, pt(place.at), place.name, Attr::Warn);
    }
    if detail != Detail::Village {
        for (i, place) in PLACES.iter().enumerate().take(VILLAGE) {
            if i as u8 != here {
                label(s, v, &mut taken, pt(place.at), place.name, Attr::Normal);
            }
        }
    }
    if detail == Detail::Streets {
        for r in ROADS.iter().filter(|r| !r.wild) {
            // Halfway along the *first* segment, not halfway from end to end.
            // `Ulita Mare` runs north from the gate and south to the inn, so
            // its two ends average out to the square -- which put its name on
            // the wrong street, beside a road going the other way.
            let (a, b) = (
                pt(PLACES[r.along[0] as usize].at),
                pt(PLACES[r.along[1] as usize].at),
            );
            label(
                s,
                v,
                &mut taken,
                ((a.0 + b.0) / 2, (a.1 + b.1) / 2),
                r.name,
                Attr::Frame,
            );
        }
    }

    // How much ground the window covers, in the unit everything else uses.
    s.rule(FOOT_Y, Attr::Frame);
    let mut x = 2 + s.text(2, SCALE_Y, i18n::t(Msg::RpgMapAcross), Attr::Normal) + 1;
    x += duration(s, x, SCALE_Y, (v.span().0 / 10) as u32, Attr::Bright);
    keys_line(s, SCALE_Y, x, Msg::RpgMapKeys);
}

/// The corners of everything there is: `(west, east, north, south)`.
fn bounds() -> (i32, i32, i32, i32) {
    let mut b = (i32::MAX, i32::MIN, i32::MAX, i32::MIN);
    for p in PLACES.iter() {
        b.0 = b.0.min(p.at.0 as i32);
        b.1 = b.1.max(p.at.0 as i32);
        b.2 = b.2.min(p.at.1 as i32);
        b.3 = b.3.max(p.at.1 as i32);
    }
    b
}

/// Keep the window over the world.
///
/// Two cases, and getting the second wrong is what left the stone bridge
/// eighteen units off the top of the screen at the widest zoom: when the world
/// is *smaller* than the window there is nothing to pan, so the window centres
/// on the world rather than on wherever the player happened to be standing.
/// When it is larger, the centre is held far enough in for the window to stay
/// over the map, which is also what stops panning from wandering into nothing.
fn keep_in_view(v: &mut View) {
    let (west, east, north, south) = bounds();
    let (span_x, span_y) = v.span();
    v.cx = if east - west <= span_x {
        (west + east) / 2
    } else {
        v.cx.clamp(west + span_x / 2, east - span_x / 2)
    };
    v.cy = if south - north <= span_y {
        (north + south) / 2
    } else {
        v.cy.clamp(north + span_y / 2, south - span_y / 2)
    };
}

pub fn run(p: &mut dyn Platform, s: &mut Screen, here: u8) {
    let start = pt(PLACES[here as usize].at);
    let mut v = View {
        x: VIEW_X,
        y: VIEW_Y,
        w: VIEW_W,
        h: VIEW_H,
        cx: start.0,
        cy: start.1,
        per: ZOOMS[START],
    };
    let mut rung = START;

    loop {
        draw(s, &v, here);
        s.flush(p);
        let (span_x, span_y) = v.span();
        match read_key(p) {
            Key::Esc => return,
            // Recentre. Panning is cheap to do and easy to get lost with, so
            // there has to be a way back to yourself that is one key.
            Key::Enter => {
                v.cx = start.0;
                v.cy = start.1;
            }
            Key::Left => v.cx -= span_x / 4,
            Key::Right => v.cx += span_x / 4,
            Key::Up => v.cy -= span_y / 4,
            Key::Down => v.cy += span_y / 4,
            Key::Plus => rung = rung.saturating_sub(1),
            Key::Minus => rung = (rung + 1).min(ZOOMS.len() - 1),
            // A rung by number, the way every other list here is answerable by
            // typing one. Six of them, closest first.
            Key::Digit(n) if (n as usize) <= ZOOMS.len() => rung = n as usize - 1,
            _ => continue,
        }
        v.per = ZOOMS[rung];
        keep_in_view(&mut v);
    }
}
