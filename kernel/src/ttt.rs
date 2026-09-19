//! Tic-tac-toe. You are X and move first; the machine is O.
//!
//! The opponent plays **perfectly** — full minimax over the game tree, no
//! heuristics and no randomness. Tic-tac-toe is a draw under optimal play, so
//! the best you can do is force a draw; if you win, that is a bug.
//!
//! There is no allocation and no recursion beyond the 9 plies the board
//! allows, so the whole game costs one `[u8; 9]` on the stack.
//!
//! The board is mirrored onto the 12x8 LED matrix when one is fitted: each
//! cell is 3x2 pixels with a one-pixel gutter, **X solid (6 pixels) and O a
//! single pixel**. An outlined O was tried first and read as an X at this
//! size; six-versus-one cannot be misread.
//!
//! # Rendering
//!
//! The screen is laid out at fixed rows and drawn once. A move rewrites the
//! one cell that changed — three bytes of cursor address and a glyph — and
//! messages rewrite a single reserved line. Nothing scrolls and nothing is
//! cleared mid-game, so the board stays put on screen instead of being wiped
//! and repainted from the top on every keypress. Same reasoning as `setup`:
//! over a 115200 baud line the frame *is* the budget.

use crate::i18n::{self, Msg};
use crate::{kprint, vt, Platform};

const EMPTY: u8 = 0;
const X: u8 = 1; // human
const O: u8 = 2; // machine

/// The eight ways to win.
const LINES: [[usize; 3]; 8] = [
    [0, 1, 2], [3, 4, 5], [6, 7, 8], // rows
    [0, 3, 6], [1, 4, 7], [2, 5, 8], // columns
    [0, 4, 8], [2, 4, 6],            // diagonals
];

/// `X`, `O`, or `EMPTY` if nobody has three in a row yet.
fn winner(b: &[u8; 9]) -> u8 {
    for l in LINES {
        let v = b[l[0]];
        if v != EMPTY && v == b[l[1]] && v == b[l[2]] {
            return v;
        }
    }
    EMPTY
}

fn full(b: &[u8; 9]) -> bool {
    b.iter().all(|&c| c != EMPTY)
}

/// Score a position from O's point of view.
///
/// `depth` is folded into the result so the machine prefers winning sooner and
/// losing later — without it, a forced loss looks identical whether it happens
/// next move or in four, and the machine plays listlessly.
fn minimax(b: &mut [u8; 9], turn: u8, depth: i32) -> i32 {
    match winner(b) {
        O => return 10 - depth,
        X => return depth - 10,
        _ => {}
    }
    if full(b) {
        return 0;
    }

    let mut best = if turn == O { i32::MIN } else { i32::MAX };
    for i in 0..9 {
        if b[i] != EMPTY {
            continue;
        }
        b[i] = turn;
        let s = minimax(b, if turn == O { X } else { O }, depth + 1);
        b[i] = EMPTY;
        if turn == O {
            if s > best {
                best = s;
            }
        } else if s < best {
            best = s;
        }
    }
    best
}

fn best_move(b: &mut [u8; 9]) -> usize {
    let mut best_score = i32::MIN;
    let mut best_cell = 0;
    for i in 0..9 {
        if b[i] != EMPTY {
            continue;
        }
        b[i] = O;
        let s = minimax(b, X, 0);
        b[i] = EMPTY;
        if s > best_score {
            best_score = s;
            best_cell = i;
        }
    }
    best_cell
}

/// Pack the board into a 12x8 frame: 3x2 cells, one-pixel gutters,
/// X solid and O hollow.
fn matrix_frame(b: &[u8; 9]) -> [u16; 8] {
    let mut f = [0u16; 8];
    for (cell, &v) in b.iter().enumerate() {
        let x0 = (cell % 3) * 4; // 0, 4, 8
        let y0 = (cell / 3) * 3; // 0, 3, 6
        match v {
            X => {
                for dy in 0..2 {
                    for dx in 0..3 {
                        f[y0 + dy] |= 1 << (11 - (x0 + dx));
                    }
                }
            }
            // A hollow outline was too easily confused with the solid X at
            // this size. One lit pixel against six is unmistakable.
            O => f[y0] |= 1 << (11 - (x0 + 1)),
            _ => {}
        }
    }
    f
}

// ---------------------------------------------------------------- layout
//
// Fixed 1-based screen rows. Everything the game draws has a reserved place,
// which is what lets a move touch one cell instead of the whole screen.

/// Screen row of the top board row. The three rows sit two apart, with the
/// rules between them.
const TOP: usize = 3;
/// Screen column of the leftmost cell's glyph.
const LEFT: usize = 8;
const HOWTO_ROW: usize = 9;
/// Where the game asks for a key.
const PROMPT_ROW: usize = 11;
/// Where it answers: "taken", "thinking", the result.
const MSG_ROW: usize = 12;



/// Screen position of cell `i`'s glyph.
fn cell_pos(i: usize) -> (usize, usize) {
    (TOP + (i / 3) * 2, LEFT + (i % 3) * 4)
}

/// Rewrite one cell. This is the whole cost of a move.
fn draw_cell(p: &mut dyn Platform, b: &[u8; 9], i: usize) {
    let (row, col) = cell_pos(i);
    vt::at(p, row, col);
    match b[i] {
        X => kprint!(p, "{}X{}", vt::sgr(vt::FG_CYAN), vt::RESET),
        O => kprint!(p, "{}O{}", vt::sgr(vt::FG_YELLOW), vt::RESET),
        // Empty cells show their key, so the mapping needs no legend.
        _ => kprint!(p, "{}{}{}", vt::sgr(vt::FG_BLUE), i + 1, vt::RESET),
    }
}

/// Rewrite one reserved line, blanking it when `msg` is `None`.
fn line(p: &mut dyn Platform, row: usize, msg: Option<Msg>) {
    vt::at(p, row, 1);
    vt::clear_eol(p);
    if let Some(m) = msg {
        i18n::put(p, m);
    }
}

/// Paint the grid and every cell. Only needed on entry and on a new game.
fn draw_frame(p: &mut dyn Platform, b: &[u8; 9]) {
    // Default colours, not whatever the last screen left set: the glyphs are
    // drawn against a `RESET`, so the cleared background has to match them.
    vt::cls_normal(p);

    for row in 0..3 {
        vt::at(p, TOP + row * 2, 1);
        kprint!(p, "      ");
        for col in 0..3 {
            kprint!(p, "   ");
            if col < 2 {
                kprint!(p, "{}", vt::LV);
            }
        }
        if row < 2 {
            vt::at(p, TOP + row * 2 + 1, 1);
            kprint!(p, "      ");
            for col in 0..3 {
                vt::repeat(p, vt::LH, 3);
                if col < 2 {
                    kprint!(p, "\u{253c}"); // ┼
                }
            }
        }
    }
    for i in 0..9 {
        draw_cell(p, b, i);
    }

    vt::at(p, HOWTO_ROW, 1);
    i18n::put(p, Msg::TttHowto);
}

fn push_matrix(p: &mut dyn Platform, b: &[u8; 9]) {
    if p.has_led_matrix() {
        let f = matrix_frame(b);
        p.led_matrix(&f);
    }
}

/// Blank the panel, restore the cursor, and leave it below the board.
fn leave(p: &mut dyn Platform) {
    if p.has_led_matrix() {
        p.led_matrix(&[0; 8]);
    }
    vt::show_cursor(p);
    vt::alt_leave(p);
}

/// Block until a byte arrives. The shell is not running during a game, so this
/// is the only input path; `Q` and Ctrl-C both leave.
fn wait_key(p: &mut dyn Platform) -> u8 {
    loop {
        if let Some(b) = p.get() {
            return b;
        }
    }
}

/// Run one session. Returns when the player quits.
///
/// "Another game?" loops rather than recursing. The recursive version read
/// well but grew the stack once per game, and this part has 32 KB of it.
pub fn play(p: &mut dyn Platform) {
    vt::alt_enter(p);
    vt::hide_cursor(p);

    'session: loop {
        let mut board = [EMPTY; 9];
        draw_frame(p, &board);
        push_matrix(p, &board);

        // The result of one game, or `None` if the player asked to leave.
        let outcome: Option<Msg> = 'game: loop {
            // --- player ---
            let placed = loop {
                line(p, PROMPT_ROW, Some(Msg::TttYourturn));
                match wait_key(p) {
                    b'q' | b'Q' | 0x03 => break 'game None,
                    b'n' | b'N' => continue 'session,
                    // Moves take effect on the keypress, but people press
                    // Enter out of habit -- swallow it rather than scolding
                    // them for it.
                    b'\r' | b'\n' => continue,
                    k @ b'1'..=b'9' => {
                        let cell = (k - b'1') as usize;
                        if board[cell] != EMPTY {
                            line(p, MSG_ROW, Some(Msg::TttTaken));
                            continue;
                        }
                        break cell;
                    }
                    _ => {
                        line(p, MSG_ROW, Some(Msg::TttInvalid));
                        continue;
                    }
                }
            };
            board[placed] = X;
            draw_cell(p, &board, placed);
            push_matrix(p, &board);
            line(p, MSG_ROW, None);

            if winner(&board) == X {
                break Some(Msg::TttYouwin);
            }
            if full(&board) {
                break Some(Msg::TttDraw);
            }

            // --- machine ---
            line(p, MSG_ROW, Some(Msg::TttThinking));
            let mv = best_move(&mut board);
            board[mv] = O;
            draw_cell(p, &board, mv);
            push_matrix(p, &board);
            line(p, MSG_ROW, None);

            if winner(&board) == O {
                break Some(Msg::TttIwin);
            }
            if full(&board) {
                break Some(Msg::TttDraw);
            }
        };

        match outcome {
            None => {
                line(p, PROMPT_ROW, None);
                line(p, MSG_ROW, Some(Msg::TttQuit));
                break;
            }
            Some(m) => line(p, MSG_ROW, Some(m)),
        }

        line(p, PROMPT_ROW, Some(Msg::TttAgain));
        match wait_key(p) {
            b'y' | b'Y' => continue 'session,
            _ => break,
        }
    }

    leave(p);
}
