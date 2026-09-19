//! Compile a readable story source and send it to the board over XMODEM.
//!
//! # Why the compiler is here
//!
//! The board reads a pack; it does not parse one. A scene is thirty bytes at a
//! computed offset and a string is an offset and a length, so there is no text
//! format on the RA4M1 at all -- no tokeniser, no line buffer, no error
//! messages nobody can read on a 25-row screen. All of that lives on the
//! machine with the memory to do it and a place to print a line number.
//!
//! # The source format
//!
//! ```text
//! # The Salt Road            pack title, once, first
//! ; anything after a semicolon at the start of a line is a comment
//!
//! : start                    a scene label
//! = The Ferryman             the scene's own title
//! The river is wider than the map said.
//!
//! He has been watching you come down the bank for a while.
//!
//! - "How much?"       > price
//! - Wade it           > across  +wet
//! - Ask about the pass > pass   ?paid
//! ```
//!
//! A choice is `- label > target`, optionally `+flag` to set one and `?flag` to
//! require one. `> .` ends the pack. Flags are named here and become bits.
//!
//! # The binary
//!
//! ```text
//!  0  magic  u16   0x4B44 "DK"
//!  2  ver    u8    1
//!  3  scenes u8
//!  4  crc    u16   CCITT-FALSE over 12..len
//!  6  len    u16
//!  8  title  off u16, len u16
//! 11  pad
//! 12  scene[n]      36 bytes each
//!     text blob
//! ```

use std::collections::HashMap;
use std::io::{Read, Write};
use std::time::{Duration, Instant};

const MAGIC: u16 = 0x4B44;
const VERSION: u8 = 1;
const HEADER: usize = 12;
const SCENE: usize = 36;
const CHOICES: usize = 4;
const MAX_SCENES: usize = 24;
const PACK_MAX: usize = 6 * 1024;
const END: u8 = 0xff;

const SOH: u8 = 0x01;
const EOT: u8 = 0x04;
const ACK: u8 = 0x06;
const NAK: u8 = 0x15;
const CAN: u8 = 0x18;
const BLOCK: usize = 128;

// ------------------------------------------------------------------ compile

#[derive(Default)]
struct Choice {
    label: String,
    target: String,
    set: String,
    need: String,
    line: usize,
}

#[derive(Default)]
struct Scene {
    label: String,
    title: String,
    body: String,
    choices: Vec<Choice>,
    line: usize,
}

/// Turn the source into a pack, or say which line is wrong.
pub fn compile(src: &str) -> Result<Vec<u8>, String> {
    let mut title = String::new();
    let mut scenes: Vec<Scene> = Vec::new();

    for (n, raw) in src.lines().enumerate() {
        let line = n + 1;
        let s = raw.trim_end();
        let t = s.trim_start();
        if t.starts_with(';') {
            continue;
        }
        if let Some(rest) = t.strip_prefix('#') {
            if title.is_empty() {
                title = rest.trim().to_string();
            }
            continue;
        }
        if let Some(rest) = t.strip_prefix(':') {
            scenes.push(Scene {
                label: rest.trim().to_string(),
                line,
                ..Default::default()
            });
            continue;
        }
        let Some(sc) = scenes.last_mut() else {
            if t.is_empty() {
                continue;
            }
            return Err(format!("line {line}: text before the first `: scene`"));
        };
        if let Some(rest) = t.strip_prefix('=') {
            sc.title = rest.trim().to_string();
            continue;
        }
        if let Some(rest) = t.strip_prefix("- ") {
            sc.choices.push(parse_choice(rest, line)?);
            continue;
        }
        // Anything else is prose. Blank lines are kept, because a paragraph
        // break is the only formatting the wrap on the board understands.
        if !sc.body.is_empty() || !t.is_empty() {
            sc.body.push_str(s.trim());
            sc.body.push('\n');
        }
    }

    if title.is_empty() {
        return Err("no `# title` line".into());
    }
    if scenes.is_empty() {
        return Err("no scenes".into());
    }
    if scenes.len() > MAX_SCENES {
        return Err(format!(
            "{} scenes; the board holds {MAX_SCENES}",
            scenes.len()
        ));
    }

    let mut index: HashMap<&str, u8> = HashMap::new();
    for (i, sc) in scenes.iter().enumerate() {
        if index.insert(sc.label.as_str(), i as u8).is_some() {
            return Err(format!("line {}: duplicate scene `{}`", sc.line, sc.label));
        }
    }

    // Flags are named in the source and are bits here. Eight, because the board
    // holds them in a byte for the length of one visit.
    let mut flags: HashMap<&str, u8> = HashMap::new();
    for sc in &scenes {
        for c in &sc.choices {
            for name in [c.set.as_str(), c.need.as_str()] {
                if name.is_empty() || flags.contains_key(name) {
                    continue;
                }
                let bit = flags.len();
                if bit >= 8 {
                    return Err(format!("line {}: more than eight flags", c.line));
                }
                flags.insert(name, 1 << bit);
            }
        }
    }

    // Strings go into one blob, deduplicated -- two scenes that offer "Go on"
    // should not cost the text twice.
    let mut blob: Vec<u8> = Vec::new();
    let mut seen: HashMap<String, (u16, u16)> = HashMap::new();
    let base = HEADER + scenes.len() * SCENE;

    let mut intern = |text: &str, line: usize, what: &str| -> Result<(u16, u16), String> {
        let text = text.trim_end_matches('\n');
        if !text.is_ascii() {
            return Err(format!(
                "line {line}: {what} is not ASCII -- the board's font has no room for accents"
            ));
        }
        // The board reads a body into a 640-byte buffer. Refusing here beats
        // truncating there, because here there is a line number to print.
        if text.len() > 600 {
            return Err(format!(
                "line {line}: {what} is {} bytes; 600 is the most that fits on the board",
                text.len()
            ));
        }
        if let Some(&hit) = seen.get(text) {
            return Ok(hit);
        }
        let at = (base + blob.len()) as u16;
        blob.extend_from_slice(text.as_bytes());
        let hit = (at, text.len() as u16);
        seen.insert(text.to_string(), hit);
        Ok(hit)
    };

    let title_str = intern(&title, 1, "the title")?;

    let mut table = Vec::with_capacity(scenes.len() * SCENE);
    for sc in &scenes {
        let t = intern(&sc.title, sc.line, "a scene title")?;
        let b = intern(&sc.body, sc.line, "a scene body")?;
        table.extend_from_slice(&t.0.to_le_bytes());
        table.extend_from_slice(&t.1.to_le_bytes());
        table.extend_from_slice(&b.0.to_le_bytes());
        table.extend_from_slice(&b.1.to_le_bytes());
        if sc.choices.len() > CHOICES {
            return Err(format!(
                "line {}: {} choices; {CHOICES} is the most a scene can offer",
                sc.line,
                sc.choices.len()
            ));
        }
        for c in 0..CHOICES {
            match sc.choices.get(c) {
                Some(ch) => {
                    let l = intern(&ch.label, ch.line, "a choice")?;
                    let to = if ch.target == "." {
                        END
                    } else {
                        *index.get(ch.target.as_str()).ok_or_else(|| {
                            format!("line {}: no scene called `{}`", ch.line, ch.target)
                        })?
                    };
                    table.extend_from_slice(&l.0.to_le_bytes());
                    table.extend_from_slice(&l.1.to_le_bytes());
                    table.push(to);
                    table.push(*flags.get(ch.set.as_str()).unwrap_or(&0));
                    table.push(*flags.get(ch.need.as_str()).unwrap_or(&0));
                }
                None => table.extend_from_slice(&[0, 0, 0, 0, END, 0, 0]),
            }
        }
    }

    let len = HEADER + table.len() + blob.len();
    if len > PACK_MAX {
        return Err(format!("{len} bytes; the board holds {PACK_MAX}"));
    }

    let mut out = vec![0u8; HEADER];
    out[0..2].copy_from_slice(&MAGIC.to_le_bytes());
    out[2] = VERSION;
    out[3] = scenes.len() as u8;
    out[6..8].copy_from_slice(&(len as u16).to_le_bytes());
    out[8..10].copy_from_slice(&title_str.0.to_le_bytes());
    out[10..12].copy_from_slice(&title_str.1.to_le_bytes());
    out.extend_from_slice(&table);
    out.extend_from_slice(&blob);

    // CCITT-FALSE over everything after the header -- the same seed the RPG's
    // save records use, and deliberately *not* the zero seed XMODEM wants for
    // its per-block check.
    let crc = crc16(0xFFFF, &out[HEADER..]);
    out[4..6].copy_from_slice(&crc.to_le_bytes());
    Ok(out)
}

fn parse_choice(rest: &str, line: usize) -> Result<Choice, String> {
    let Some((label, tail)) = rest.split_once('>') else {
        return Err(format!("line {line}: a choice needs `> target`"));
    };
    let mut c = Choice {
        label: label.trim().to_string(),
        line,
        ..Default::default()
    };
    if c.label.is_empty() {
        return Err(format!("line {line}: a choice needs words"));
    }
    for (i, word) in tail.split_whitespace().enumerate() {
        if let Some(f) = word.strip_prefix('+') {
            c.set = f.to_string();
        } else if let Some(f) = word.strip_prefix('?') {
            c.need = f.to_string();
        } else if i == 0 {
            c.target = word.to_string();
        } else {
            return Err(format!("line {line}: `{word}` is not a flag or a target"));
        }
    }
    if c.target.is_empty() {
        return Err(format!("line {line}: a choice needs `> target`"));
    }
    Ok(c)
}

fn crc16(seed: u16, data: &[u8]) -> u16 {
    let mut crc = seed;
    for &b in data {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

// --------------------------------------------------------------------- send

/// Compile `path` and push it at the board: type `DLC LOAD`, wait for `C`, and
/// XMODEM the result.
///
/// Typing the command for the user is the point. The alternative is telling
/// somebody to run `DLC LOAD` in one window and a sender in another and get the
/// ten-second window right, which is three chances to go wrong for no gain.
pub fn send(port_name: &str, baud: u32, path: &str) -> i32 {
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("cannot read {path}: {e}");
            return 1;
        }
    };
    let pack = match compile(&src) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{path}: {e}");
            return 1;
        }
    };
    eprintln!("{path}: {} bytes", pack.len());

    let mut port = match serialport::new(port_name, baud)
        .timeout(Duration::from_millis(50))
        .open()
    {
        Ok(p) => p,
        Err(e) => {
            eprintln!("cannot open {port_name}: {e}");
            return 1;
        }
    };
    let _ = port.write_data_terminal_ready(true);
    let _ = port.write_request_to_send(true);
    // The bridge needs a moment after DTR before the RA4M1's output reaches us.
    std::thread::sleep(Duration::from_millis(400));
    let _ = port.clear(serialport::ClearBuffer::Input);

    if port.write_all(b"DLC LOAD\r").is_err() {
        eprintln!("the board is not listening");
        return 1;
    }
    let _ = port.flush();

    // Let the echo and the ready line go by before looking for the invitation.
    //
    // This is not politeness. The board echoes what it is typed, and the thing
    // it was typed was `DLC LOAD` -- which contains a `C`. Scanning the raw
    // stream for the XMODEM invitation finds that `C` and starts sending into a
    // shell that is not listening yet. The echo arrives in one burst, and the
    // board is then silent for a second before its first real `C`, so a quarter
    // of a second of quiet is an unambiguous end of prose.
    let mut one = [0u8; 1];
    let mut last = Instant::now();
    let settle = Instant::now() + Duration::from_secs(3);
    while Instant::now() < settle {
        match port.read(&mut one) {
            Ok(1) => last = Instant::now(),
            Ok(_) => {}
            Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {
                if last.elapsed() > Duration::from_millis(250) {
                    break;
                }
            }
            Err(e) => {
                eprintln!("read failed: {e}");
                return 1;
            }
        }
    }

    // Now the only `C` that can arrive is the invitation. The board sends one a
    // second for ten.
    let deadline = Instant::now() + Duration::from_secs(12);
    let mut invited = false;
    while Instant::now() < deadline {
        match port.read(&mut one) {
            Ok(1) if one[0] == b'C' => {
                invited = true;
                break;
            }
            Ok(_) => {}
            Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(e) => {
                eprintln!("read failed: {e}");
                return 1;
            }
        }
    }
    if !invited {
        eprintln!("the board never asked for the file");
        return 1;
    }

    let blocks = pack.len().div_ceil(BLOCK);
    let mut frame = [0u8; 3 + BLOCK + 2];
    for i in 0..blocks {
        let n = (i + 1) as u8;
        frame[0] = SOH;
        frame[1] = n;
        frame[2] = !n;
        // The tail of the last block is padded with SUB, which is what XMODEM
        // has always used and what tells a reader the file ended mid-block. The
        // board does not care -- it has the length in the header -- but a pack
        // that is a valid XMODEM file can be sent by any other terminal too.
        frame[3..3 + BLOCK].fill(0x1a);
        let from = i * BLOCK;
        let take = BLOCK.min(pack.len() - from);
        frame[3..3 + take].copy_from_slice(&pack[from..from + take]);
        let crc = crc16(0, &frame[3..3 + BLOCK]);
        frame[3 + BLOCK] = (crc >> 8) as u8;
        frame[4 + BLOCK] = crc as u8;

        let mut tries = 0;
        loop {
            if port.write_all(&frame).is_err() || port.flush().is_err() {
                eprintln!("\nthe link dropped at block {}", i + 1);
                return 1;
            }
            match reply(&mut *port, Duration::from_secs(4)) {
                Some(ACK) => break,
                Some(CAN) => {
                    eprintln!("\nthe board cancelled at block {}", i + 1);
                    return 1;
                }
                Some(NAK) | None => {
                    tries += 1;
                    if tries >= 6 {
                        eprintln!("\nblock {} would not go through", i + 1);
                        return 1;
                    }
                }
                Some(_) => {}
            }
        }
        eprint!("\r  {}/{} blocks", i + 1, blocks);
    }
    eprintln!();

    let _ = port.write_all(&[EOT]);
    let _ = port.flush();
    let _ = reply(&mut *port, Duration::from_secs(4));

    // Whatever the board says about what it received. It is the only opinion
    // that counts, and it is the one the person watching wants to see.
    std::thread::sleep(Duration::from_millis(600));
    let mut tail = [0u8; 512];
    if let Ok(n) = port.read(&mut tail) {
        let said = String::from_utf8_lossy(&tail[..n]);
        for line in said.lines().filter(|l| !l.trim().is_empty()) {
            eprintln!("{}", line.trim_end());
        }
    }
    0
}

fn reply(port: &mut dyn serialport::SerialPort, within: Duration) -> Option<u8> {
    let deadline = Instant::now() + within;
    let mut one = [0u8; 1];
    while Instant::now() < deadline {
        match port.read(&mut one) {
            Ok(1) => return Some(one[0]),
            Ok(_) => {}
            Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(_) => return None,
        }
    }
    None
}
