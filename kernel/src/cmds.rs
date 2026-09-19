//! Built-in commands.
//!
//! The table is dispatched through `&mut dyn Platform` rather than a generic
//! parameter. That costs one indirect call per command and saves monomorphising
//! every handler per platform -- worth it when the budget is 240 KB of flash.
//!
//! Help text and every user-visible message are [`Msg`] ids, not literals, so
//! the whole shell follows `LANG`.

use crate::i18n::{self, Chars, Msg};
use crate::marquee::{Marquee, Mode};
use crate::{kprint, kprintln, romfs, vt, Platform};

type Handler = fn(&mut dyn Platform, &mut Marquee, &str);

/// Where a command's output actually turns up.
///
/// This machine has three displays -- the terminal you are typing in, a 12x8
/// LED panel on the board, and an optional TFT -- plus a second chip that
/// answers on its own. "Print text" is ambiguous across those, so the listing
/// groups commands by destination and says so in the heading.
#[derive(Clone, Copy, PartialEq)]
enum Group {
    System,
    Term,
    Files,
    Matrix,
    Pins,
    Tft,
    Radio,
}

impl Group {
    fn title(self) -> Msg {
        match self {
            Group::System => Msg::HelpGroupSystem,
            Group::Term => Msg::HelpGroupTerm,
            Group::Files => Msg::HelpGroupFiles,
            Group::Matrix => Msg::HelpGroupMatrix,
            Group::Pins => Msg::HelpGroupPins,
            Group::Tft => Msg::HelpGroupTft,
            Group::Radio => Msg::HelpGroupRadio,
        }
    }
}

struct Cmd {
    name: &'static str,
    args: &'static str,
    group: Group,
    /// One-line summary for the listing.
    help: Msg,
    /// Concrete invocation. Command syntax is not translated, so this stays a
    /// literal rather than a Msg.
    example: &'static str,
    /// Long description printed by `HELP <name>`: syntax, accepted values,
    /// limits, and what it does *not* do. `None` where the summary and the
    /// example genuinely say everything -- `CLS` needs no essay.
    detail: Option<Msg>,
    run: Handler,
}

/// Ordered by group: `cmd_help` walks it once and prints a heading whenever
/// the group changes, so this order *is* the layout of the help screen.
#[rustfmt::skip]
static TABLE: &[Cmd] = &[
    Cmd { name: "HELP",   args: "[command]",      group: Group::System, help: Msg::HelpHelp,   example: "HELP AT",              detail: Some(Msg::HelpDHelp),   run: cmd_help },
    Cmd { name: "VER",    args: "",               group: Group::System, help: Msg::HelpVer,    example: "",                     detail: None,                   run: cmd_ver },
    Cmd { name: "MEM",    args: "",               group: Group::System, help: Msg::HelpMem,    example: "",                     detail: None,                   run: cmd_mem },
    Cmd { name: "TIME",   args: "",               group: Group::System, help: Msg::HelpTime,   example: "",                     detail: None,                   run: cmd_time },
    Cmd { name: "LANG",   args: "[code]",         group: Group::System, help: Msg::HelpLang,   example: "LANG RO",              detail: Some(Msg::HelpDLang),   run: cmd_lang },
    Cmd { name: "THEME",  args: "[name]",         group: Group::System, help: Msg::HelpTheme,  example: "THEME BIOS",           detail: Some(Msg::HelpDTheme),  run: cmd_theme },
    Cmd { name: "PAGE",   args: "[rows|OFF]",     group: Group::System, help: Msg::HelpPage,   example: "PAGE 24",              detail: Some(Msg::HelpDPage),   run: cmd_page },
    Cmd { name: "SETUP",  args: "",               group: Group::System, help: Msg::HelpSetup,  example: "",                     detail: None,                   run: cmd_setup },
    Cmd { name: "POST",   args: "",               group: Group::System, help: Msg::HelpPost,   example: "",                     detail: None,                   run: cmd_post },
    Cmd { name: "REBOOT", args: "",               group: Group::System, help: Msg::HelpReboot, example: "",                     detail: None,                   run: cmd_reboot },
    Cmd { name: "EXIT",   args: "",               group: Group::System, help: Msg::HelpExit,   example: "",                     detail: None,                   run: cmd_reboot },

    Cmd { name: "CLS",    args: "",               group: Group::Term,   help: Msg::HelpCls,    example: "",                     detail: None,                   run: cmd_cls },
    Cmd { name: "ECHO",   args: "<text>",         group: Group::Term,   help: Msg::HelpEcho,   example: "ECHO hello world",     detail: Some(Msg::HelpDEcho),   run: cmd_echo },
    Cmd { name: "DINO",   args: "",               group: Group::Term,   help: Msg::HelpDino,   example: "",                     detail: None,                   run: cmd_dino },
    Cmd { name: "TTT",    args: "",               group: Group::Term,   help: Msg::HelpTtt,    example: "",                     detail: None,                   run: cmd_ttt },
    Cmd { name: "RPG",    args: "",               group: Group::Term,   help: Msg::HelpRpg,    example: "",                     detail: Some(Msg::HelpDRpg),    run: cmd_rpg },

    Cmd { name: "DIR",    args: "",               group: Group::Files,  help: Msg::HelpDir,    example: "",                     detail: Some(Msg::HelpDDir),    run: cmd_dir },
    Cmd { name: "TYPE",   args: "<file>",         group: Group::Files,  help: Msg::HelpType,   example: "TYPE HARDWARE.TXT",    detail: Some(Msg::HelpDType),   run: cmd_type },
    Cmd { name: "RUN",    args: "<file>",         group: Group::Files,  help: Msg::HelpRun,    example: "RUN AUTOEXEC.BAT",     detail: Some(Msg::HelpDRun),    run: cmd_run },
    Cmd { name: "DF",     args: "[ERASE|WRITE t]", group: Group::Files,  help: Msg::HelpDf,     example: "DF WRITE HELLO",       detail: Some(Msg::HelpDDf),     run: cmd_df },
    Cmd { name: "DLC",    args: "[LOAD|PLAY|ERASE]", group: Group::Files, help: Msg::HelpDlc,  example: "DLC PLAY",             detail: Some(Msg::HelpDDlc),    run: cmd_dlc },

    Cmd { name: "LED",    args: "<pattern>",      group: Group::Matrix, help: Msg::HelpLed,    example: "LED HEART",            detail: Some(Msg::HelpDLed),    run: cmd_led },
    Cmd { name: "TEXT",   args: "<word>",         group: Group::Matrix, help: Msg::HelpText,   example: "TEXT OK",              detail: Some(Msg::HelpDText),   run: cmd_text },
    Cmd { name: "SCROLL", args: "<dir> <text>",   group: Group::Matrix, help: Msg::HelpScroll, example: "SCROLL L HELLO",       detail: Some(Msg::HelpDScroll), run: cmd_scroll },
    Cmd { name: "SPEED",  args: "[ms]",           group: Group::Matrix, help: Msg::HelpSpeed,  example: "SPEED 40",             detail: Some(Msg::HelpDSpeed),  run: cmd_speed },

    Cmd { name: "PINS",   args: "[WATCH [p.n]]",  group: Group::Pins,   help: Msg::HelpPins,   example: "PINS WATCH",           detail: Some(Msg::HelpDPins),   run: cmd_pins },

    Cmd { name: "TFT",    args: "<mode>",         group: Group::Tft,    help: Msg::HelpTft,    example: "TFT ID",               detail: Some(Msg::HelpDTft),    run: cmd_tft },

    Cmd { name: "ESP",    args: "",               group: Group::Radio,  help: Msg::HelpEsp,    example: "",                     detail: Some(Msg::HelpDEsp),    run: cmd_esp },
    Cmd { name: "AT",     args: "<command>",      group: Group::Radio,  help: Msg::HelpAt,     example: "AT+PREFSTAT",          detail: Some(Msg::HelpDAt),     run: cmd_at },
    Cmd { name: "DEBUG",  args: "[ON|OFF]",       group: Group::Radio,  help: Msg::HelpDebug,  example: "DEBUG ON",             detail: Some(Msg::HelpDDebug),  run: cmd_debug },
];

pub fn dispatch<P: Platform>(p: &mut P, m: &mut Marquee, line: &str) {
    dispatch_dyn(p, m, line)
}

pub(crate) fn dispatch_dyn(p: &mut dyn Platform, m: &mut Marquee, line: &str) {
    let line = line.trim();
    if line.is_empty() {
        return;
    }
    let (name, args) = split_first(line);

    // `AT+PREFSTAT` has no space in it, so the tokeniser sees one unknown word.
    // Anything beginning `AT+` or `AT&` is a modem command; hand over the whole
    // line untouched.
    let n = name.as_bytes();
    if n.len() > 2 && (n[0] | 32) == b'a' && (n[1] | 32) == b't' && (n[2] == b'+' || n[2] == b'&') {
        cmd_at(p, m, line);
        return;
    }

    for c in TABLE {
        if ieq(c.name, name) {
            (c.run)(p, m, args);
            return;
        }
    }

    // Not a command -- but DOS ran a program when you typed its name, and the
    // nearest thing here is a file. `DIR` lists four of them and says nothing
    // about how to open one, so `README` was a dead end until you happened to
    // know `TYPE`. Now the obvious thing works: a `.BAT` runs, anything else
    // is printed.
    if let Some(f) = romfs::find(name) {
        let is_batch = ieq(f.ext, "BAT");
        if is_batch {
            crate::batch::run(p, m, b'A', name);
        } else {
            cmd_type(p, m, name);
        }
        return;
    }

    i18n::putln1(p, Msg::ShellBadcmd, name);
    // If a word they typed *is* a file, say so rather than leaving them to
    // guess which of `OPEN`, `VIEW` or `CAT` this shell happens to use.
    let (_, rest) = split_first(line);
    let (arg, _) = split_first(rest);
    if !arg.is_empty() && romfs::find(arg).is_some() {
        i18n::putln1(p, Msg::ShellTryfile, arg);
    }
}

// ---------------------------------------------------------------- helpers

/// ASCII case-insensitive equality. No allocation, no locale.
pub(crate) fn ieq(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .all(|(x, y)| x.to_ascii_uppercase() == y.to_ascii_uppercase())
}

/// Split off the first whitespace-delimited token.
pub(crate) fn split_first(s: &str) -> (&str, &str) {
    match s.find(char::is_whitespace) {
        Some(i) => (&s[..i], s[i..].trim_start()),
        None => (s, ""),
    }
}

// ---------------------------------------------------------------- commands

/// Column layout for the listing, chosen so the longest row -- a Romanian
/// description beside `SCROLL <dir> <text>` -- still lands inside 80 columns.
const NAME_COL: usize = 21;
const DESC_COL: usize = 33;

/// `-- Heading -----------------------------------` above each group.
fn group_head(p: &mut dyn Platform, g: Group) {
    let title = i18n::t(g.title());
    kprintln!(p);
    kprint!(p, "  ");
    vt::repeat(p, vt::LH, 2);
    kprint!(p, " {}{}{} ", vt::sgr(vt::FG_BRIGHT), title, vt::RESET);
    let used = 7 + title.len();
    vt::repeat(p, vt::LH, 78usize.saturating_sub(used));
    kprintln!(p);
}

fn cmd_help(p: &mut dyn Platform, _m: &mut Marquee, a: &str) {
    let a = a.trim();
    if !a.is_empty() {
        help_one(p, a);
        return;
    }

    // Forty-odd lines is more than a screen, which is exactly the case the
    // pager exists for.
    let mut pg = crate::pager::Pager::new();
    let mut shown: Option<Group> = None;
    for c in TABLE {
        if shown != Some(c.group) {
            group_head(p, c.group);
            // The heading costs two lines: a blank and the rule.
            if !pg.line(p) || !pg.line(p) {
                return;
            }
            shown = Some(c.group);
        }

        let mut n = c.name.len();
        kprint!(p, "  {}", c.name);
        if !c.args.is_empty() {
            kprint!(p, " {}", c.args);
            n += 1 + c.args.len();
        }
        for _ in n..NAME_COL {
            kprint!(p, " ");
        }

        let desc = i18n::t(c.help);
        kprint!(p, "{}", desc);
        if !c.example.is_empty() {
            // Right-hand example column. A description that overruns just
            // pushes the example along rather than being truncated.
            for _ in desc.len()..DESC_COL {
                kprint!(p, " ");
            }
            kprint!(p, "{}{}", i18n::t(Msg::HelpEg), c.example);
        }
        kprintln!(p);
        if !pg.line(p) {
            return;
        }
    }

    kprintln!(p);
    i18n::putln(p, Msg::HelpHint);
    kprintln!(p);
}

/// `HELP <command>` -- the full page for one command.
fn help_one(p: &mut dyn Platform, arg: &str) {
    // `HELP AT+PREFSTAT` should still land on AT, and a stray second word
    // should not turn the lookup into a miss.
    let (word, _) = split_first(arg);
    let word = match word.find(['+', '&']) {
        Some(i) if i > 0 => &word[..i],
        _ => word,
    };

    for c in TABLE {
        if !ieq(c.name, word) {
            continue;
        }

        kprintln!(p);
        kprint!(p, "  {}{}", vt::sgr(vt::FG_BRIGHT), c.name);
        if !c.args.is_empty() {
            kprint!(p, " {}", c.args);
        }
        kprintln!(p, "{}", vt::RESET);
        kprint!(p, "  ");
        vt::repeat(p, vt::LH, 68);
        kprintln!(p);
        kprintln!(p, "  {}", i18n::t(c.help));
        kprintln!(p);

        if let Some(d) = c.detail {
            // Detail strings carry their own indentation and trailing newline,
            // so the pager has to split them rather than count `kprintln!`s.
            let mut pg = crate::pager::Pager::new();
            crate::pager::put_text(p, &mut pg, i18n::t(d));
            if pg.stopped() {
                return;
            }
            kprintln!(p);
        }
        if !c.example.is_empty() {
            kprint!(p, "{}", i18n::t(Msg::HelpExample));
            kprintln!(p, "{}", c.example);
        }
        kprintln!(p);
        return;
    }

    i18n::putln1(p, Msg::HelpNohelp, word);
}

fn cmd_ver(p: &mut dyn Platform, _m: &mut Marquee, _a: &str) {
    // Bound first: the print macros take `&mut *p`, so reading through `p` in
    // the argument list would overlap that borrow.
    let (board, cpu, mhz) = (p.board_name(), p.cpu_name(), p.cpu_mhz());
    kprintln!(p);
    i18n::putln1(p, Msg::ShellVersion, crate::VERSION);
    kprintln!(p, "{} -- {} @ {} MHz", board, cpu, mhz);
    i18n::putln1(p, Msg::MsgLangis, i18n::lang_name());
    kprintln!(p);
}

fn cmd_cls(p: &mut dyn Platform, _m: &mut Marquee, _a: &str) {
    // Back to default colours, not whatever the last full-screen thing left
    // set -- otherwise CLS after POST paints the whole screen blue.
    vt::cls_normal(p);
}

fn cmd_echo(p: &mut dyn Platform, _m: &mut Marquee, a: &str) {
    kprintln!(p, "{}", a);
}

fn cmd_mem(p: &mut dyn Platform, _m: &mut Marquee, _a: &str) {
    let total = p.ram_total();
    let used = p.ram_used();
    let free = total.saturating_sub(used);
    let rom = p.rom_total();
    let vol = romfs::total_bytes();

    kprintln!(p);
    i18n::putln(p, Msg::MemHeader);
    kprintln!(p, "  ----------------  --------   --------   --------");
    kprintln!(p, "  {:<16}  {:>8}   {:>8}   {:>8}", i18n::t(Msg::MemConv), total, used, free);
    kprintln!(p, "  {:<16}  {:>8}   {:>8}   {:>8}", i18n::t(Msg::MemFlash), rom, rom - vol, vol);
    kprintln!(p, "  {:<16}  {:>8}   {:>8}   {:>8}", i18n::t(Msg::MemRom), vol, vol, 0);
    kprintln!(p);
    // Under DEBUG ON only, and English on purpose: a diagnostic is for pasting
    // into a bug report. Worth having permanently because wiring an interrupt
    // on this part takes three steps that all fail silently -- a count of zero
    // beside a working console means the polled fallback is carrying it.
    if crate::esp::trace() {
        let d = p.rx_irq_debug();
        kprintln!(p, "  RX irq {}  pending {:08X}  SCR {:02X}", d[0], d[1], d[2]);
        kprint!(p, "  ICU:");
        for v in d.iter().skip(3) {
            kprint!(p, " {:05X}", v);
        }
        kprintln!(p);
    }
    i18n::putln1(p, Msg::MemLargest, free);
    i18n::putln(p, Msg::MemNoheap);
    kprintln!(p);
}

fn cmd_dir(p: &mut dyn Platform, _m: &mut Marquee, _a: &str) {
    kprintln!(p);
    i18n::putln1(p, Msg::DirVolume, romfs::VOLUME);
    i18n::putln(p, Msg::DirDirof);
    kprintln!(p);

    for f in romfs::FILES {
        kprintln!(
            p,
            "{:<8} {:<3} {:>10}  {}  {}",
            f.name,
            f.ext,
            f.size(),
            f.date,
            f.time
        );
    }

    kprintln!(p);
    i18n::put1(p, Msg::DirFiles, romfs::FILES.len());
    kprint!(p, " {:>10} ", romfs::total_bytes());
    i18n::putln(p, Msg::MsgBytes);
    i18n::putln(p, Msg::DirNodirs);
    kprintln!(p);
    // The listing used to end here, which told you what exists and nothing
    // about how to look inside it.
    i18n::putln(p, Msg::DirHowto);
    kprintln!(p);
}

fn cmd_type(p: &mut dyn Platform, _m: &mut Marquee, a: &str) {
    if a.is_empty() {
        i18n::putln(p, Msg::MsgParammissing);
        return;
    }
    match romfs::find(a) {
        Some(f) => {
            let body = f.body();
            kprintln!(p);
            let mut pg = crate::pager::Pager::new();
            crate::pager::put_text(p, &mut pg, body);
            kprintln!(p);
        }
        None => i18n::putln1(p, Msg::MsgFilenotfound, a),
    }
}

fn cmd_time(p: &mut dyn Platform, _m: &mut Marquee, _a: &str) {
    let ms = p.uptime_ms();
    let total_s = ms / 1000;
    let (h, m, s, cs) = (
        total_s / 3600,
        (total_s % 3600) / 60,
        total_s % 60,
        (ms % 1000) / 10,
    );
    kprintln!(p);
    i18n::put(p, Msg::TimeSince);
    kprintln!(p, "{:02}:{:02}:{:02}.{:02}", h, m, s, cs);
    kprintln!(p);
}

fn cmd_led(p: &mut dyn Platform, m: &mut Marquee, a: &str) {
    if !p.has_led_matrix() {
        i18n::putln(p, Msg::MsgNomatrix);
        return;
    }
    if a.is_empty() {
        i18n::putln(p, Msg::MsgUsageled);
        return;
    }
    // A static pattern and a running marquee cannot share the panel -- the
    // marquee would overwrite the pattern on its next tick.
    m.stop();

    // 12 columns per row, packed into the low 12 bits. Bit 11 is the leftmost
    // column, matching the row-major MSB-first layout Arduino's frames use.
    let frame: [u16; 8] = if ieq(a, "ON") {
        [0x0FFF; 8]
    } else if ieq(a, "OFF") {
        [0x000; 8]
    } else if ieq(a, "HEART") {
        // Straight from the Arduino LED matrix documentation, whose frame
        // 0x3184a444 / 0x42081100 / 0xa0040000 unpacks to exactly these rows.
        // A known-good reference: if this renders as a heart, the pin table,
        // the scan order and the bit mapping are all correct.
        [
            0b0011_0001_1000,
            0b0100_1010_0100,
            0b0100_0100_0100,
            0b0010_0000_1000,
            0b0001_0001_0000,
            0b0000_1010_0000,
            0b0000_0100_0000,
            0b0000_0000_0000,
        ]
    } else if ieq(a, "TEST") {
        [0xAAA, 0x555, 0xAAA, 0x555, 0xAAA, 0x555, 0xAAA, 0x555]
    } else if ieq(a, "SMILE") {
        [
            0b0000_0000_0000,
            0b0001_1001_1000,
            0b0001_1001_1000,
            0b0000_0000_0000,
            0b0100_0000_0010,
            0b0011_0000_1100,
            0b0000_1111_0000,
            0b0000_0000_0000,
        ]
    } else {
        i18n::putln(p, Msg::MsgUsageled);
        return;
    };
    p.led_matrix(&frame);
    i18n::putln(p, Msg::MsgMatrixupdated);
}

/// `TEXT <words>` -- hold a message still on the panel.
fn cmd_text(p: &mut dyn Platform, m: &mut Marquee, a: &str) {
    if !p.has_led_matrix() {
        i18n::putln(p, Msg::MsgNomatrix);
        return;
    }
    if a.is_empty() {
        i18n::putln(p, Msg::MsgUsagetext);
        return;
    }
    let n = m.show(a, Mode::Still);
    i18n::putln1(p, Msg::MsgShowing, n);
}

/// `SCROLL [L|R|U|D] <text>` -- animate a message. Direction defaults to left.
fn cmd_scroll(p: &mut dyn Platform, m: &mut Marquee, a: &str) {
    if !p.has_led_matrix() {
        i18n::putln(p, Msg::MsgNomatrix);
        return;
    }
    if a.is_empty() {
        i18n::putln(p, Msg::MsgUsagescroll);
        i18n::putln(p, Msg::MsgUsagescrolloff);
        return;
    }
    if ieq(a, "OFF") {
        m.stop();
        i18n::putln(p, Msg::MsgScrollstopped);
        return;
    }

    // A leading direction token is optional; without one, scroll left.
    let (head, rest) = split_first(a);
    let (mode, text) = if ieq(head, "L") || ieq(head, "LEFT") {
        (Mode::Left, rest)
    } else if ieq(head, "R") || ieq(head, "RIGHT") {
        (Mode::Right, rest)
    } else if ieq(head, "U") || ieq(head, "UP") {
        (Mode::Up, rest)
    } else if ieq(head, "D") || ieq(head, "DOWN") {
        (Mode::Down, rest)
    } else {
        (Mode::Left, a)
    };

    if text.is_empty() {
        i18n::putln(p, Msg::MsgNotextafterdir);
        return;
    }

    let n = m.show(text, mode);
    let dir = match mode {
        Mode::Left => Msg::DirLeft,
        Mode::Right => Msg::DirRight,
        Mode::Up => Msg::DirUp,
        Mode::Down => Msg::DirDown,
        _ => Msg::DirStill,
    };
    let speed = m.speed();
    i18n::put1(p, Msg::MsgScrolling, n);
    kprint!(p, " {} ", i18n::t(dir));
    i18n::putln1(p, Msg::MsgAtms, speed);
}

/// `SPEED <ms>` -- milliseconds between animation steps.
fn cmd_speed(p: &mut dyn Platform, m: &mut Marquee, a: &str) {
    if a.is_empty() {
        i18n::putln1(p, Msg::MsgSpeedis, m.speed());
        return;
    }
    match a.parse::<u32>() {
        Ok(v) => {
            m.set_speed(v);
            i18n::putln1(p, Msg::MsgSpeedset, m.speed());
        }
        Err(_) => i18n::putln1(p, Msg::MsgInvalidnum, a),
    }
}

/// `LANG <code>` -- switch the interface language.
fn cmd_lang(p: &mut dyn Platform, _m: &mut Marquee, a: &str) {
    if a.is_empty() {
        i18n::putln(p, Msg::MsgUsagelang);
        i18n::putln1(p, Msg::MsgCurrent, i18n::lang_name());
        return;
    }
    match i18n::lang_by_name(a) {
        Some(idx) => {
            i18n::set_lang(idx);
            i18n::putln1(p, Msg::MsgLangset, i18n::lang_name());
            // Persisting is best-effort: the language still changes for this
            // session even if the store is unreachable.
            if p.settings_set_u8(i18n::LANG_KEY, idx) {
                i18n::putln(p, Msg::MsgSaved);
            } else {
                i18n::putln(p, Msg::MsgNotsaved);
            }
        }
        None => i18n::putln(p, Msg::MsgUsagelang),
    }
}

/// `THEME <name>` -- how the full-screen views colour the terminal.
///
/// Only the terminal. The TFT has its own palette and keeps the firmware blue
/// regardless, because there the kernel owns every pixel of the panel rather
/// than painting a rectangle inside somebody else's window.
fn cmd_theme(p: &mut dyn Platform, _m: &mut Marquee, a: &str) {
    if a.is_empty() {
        i18n::putln(p, Msg::MsgUsagetheme);
        i18n::putln1(p, Msg::MsgCurrent, vt::theme_name());
        return;
    }
    match vt::theme_by_name(a) {
        Some(idx) => {
            vt::set_theme_index(idx);
            i18n::putln1(p, Msg::MsgThemeset, vt::theme_name());
            // Index+1, so an unwritten key reading back as 0 stays telling
            // apart from a deliberate PLAIN.
            if p.settings_set_u8(vt::THEME_KEY, idx + 1) {
                i18n::putln(p, Msg::MsgSaved);
            } else {
                i18n::putln(p, Msg::MsgNotsaved);
            }
        }
        None => i18n::putln(p, Msg::MsgUsagetheme),
    }
}

/// `PINS [WATCH [<port>.<pin>]]` -- what every GPIO pin is doing.
fn cmd_pins(p: &mut dyn Platform, _m: &mut Marquee, a: &str) {
    let (head, rest) = split_first(a);
    let watch = ieq(head, "WATCH") || ieq(head, "W");
    // `3.1` names port 3 pin 1. Only meaningful with WATCH, and only ever one
    // pin -- see the note in `pins`.
    let pull = rest.split_once('.').and_then(|(a, b)| {
        Some((a.trim().parse::<usize>().ok()?, b.trim().parse::<usize>().ok()?))
    });
    crate::pins::show(p, watch, pull);
}

/// `PAGE [rows|OFF]` -- how many rows before `-- More --`.
fn cmd_page(p: &mut dyn Platform, _m: &mut Marquee, a: &str) {
    if !a.is_empty() {
        let rows = if ieq(a, "OFF") {
            Some(0u8)
        } else {
            a.parse::<u8>().ok()
        };
        match rows {
            Some(n) => crate::pager::set_rows(n),
            None => {
                i18n::putln(p, Msg::MsgUsagepage);
                return;
            }
        }
        // Rows + 1: zero rows means "never page" and must stay apart from a
        // key that was never written.
        let n = crate::pager::rows();
        if p.settings_set_u8(crate::pager::PAGE_KEY, n + 1) {
            i18n::putln(p, Msg::MsgSaved);
        } else {
            i18n::putln(p, Msg::MsgNotsaved);
        }
    }
    let n = crate::pager::rows();
    if n == 0 {
        i18n::putln(p, Msg::MsgPageoff);
    } else {
        i18n::putln1(p, Msg::MsgPageis, n);
    }
}

/// `TFT ID | ON | OFF | TEST | TEXT <words>` -- SPI display.
fn cmd_tft(p: &mut dyn Platform, _m: &mut Marquee, a: &str) {
    if !p.tft_present() {
        i18n::putln(p, Msg::MsgNotft);
        return;
    }
    let (head, rest) = split_first(a);

    if head.is_empty() || ieq(head, "ID") {
        // Read the controller ID. A wired ILI9341 answers 0x009341; anything
        // else means absent, miswired, or a different controller. This is the
        // one command that tells you whether the panel is really there.
        let id = p.tft_init();
        i18n::put1(p, Msg::MsgTftid, id);
        kprintln!(p, " (0x{:06X}) {}", id, if id == 0x9341 { "ILI9341 OK" } else { "unrecognised" });
        return;
    }

    if ieq(head, "ON") {
        p.tft_init();
        crate::tft::banner(p);
    } else if ieq(head, "OFF") {
        p.tft_init();
        crate::tft::clear(p, crate::tft::BLACK);
    } else if ieq(head, "TEST") {
        p.tft_init();
        crate::tft::test_pattern(p);
    } else if ieq(head, "TEXT") {
        if rest.is_empty() {
            i18n::putln(p, Msg::MsgUsagetft);
            return;
        }
        crate::tft::clear(p, crate::tft::POST_BG);
        crate::tft::text_centred(p, 100, rest, 2, crate::tft::WHITE, None);
    } else {
        i18n::putln(p, Msg::MsgUsagetft);
        return;
    }
    i18n::putln(p, Msg::MsgTftok);
}

/// `DINO` -- side-scrolling endless runner.
fn cmd_dino(p: &mut dyn Platform, m: &mut Marquee, _a: &str) {
    m.stop();
    crate::dino::play(p);
}

/// `TTT` -- tic-tac-toe against a perfect opponent.
fn cmd_ttt(p: &mut dyn Platform, m: &mut Marquee, _a: &str) {
    // The game owns the console and the panel until it exits.
    m.stop();
    crate::ttt::play(p);
}

fn cmd_rpg(p: &mut dyn Platform, m: &mut Marquee, _a: &str) {
    // Same rule: the panel becomes the health bar for the duration, so nothing
    // else may be scrolling text across it.
    m.stop();
    crate::rpg::run(p);
}

/// `AT <cmd>` -- raw passthrough to the ESP32-S3 modem.
///
/// Deliberately dumb: it is the tool for discovering what the bridge firmware
/// actually answers, rather than assuming. `ESP` is the safe way to use the
/// commands that discovery has already confirmed.
fn cmd_at(p: &mut dyn Platform, _m: &mut Marquee, a: &str) {
    if !p.modem_present() {
        i18n::putln(p, Msg::MsgNomodem);
        return;
    }
    crate::esp::converse(p, a, crate::esp::RAW_FIRST_MS);
}

/// `ESP` -- the menu of commands known to work on this bridge.
fn cmd_esp(p: &mut dyn Platform, _m: &mut Marquee, _a: &str) {
    crate::esp::menu(p);
}

/// `DEBUG [ON|OFF]` -- trace the modem link.
fn cmd_debug(p: &mut dyn Platform, _m: &mut Marquee, a: &str) {
    if a.is_empty() {
        i18n::putln1(p, Msg::MsgDebugis, if crate::esp::trace() { "ON" } else { "OFF" });
        return;
    }
    if ieq(a, "ON") {
        crate::esp::set_trace(true);
    } else if ieq(a, "OFF") {
        crate::esp::set_trace(false);
    } else {
        i18n::putln(p, Msg::MsgUsagedebug);
        return;
    }
    let on = crate::esp::trace();
    i18n::putln1(p, Msg::MsgDebugis, if on { "ON" } else { "OFF" });
    // Stored as 2 for on and 1 for off, so an unwritten key reading back as
    // zero stays distinguishable from a deliberate OFF.
    if p.settings_set_u8(crate::esp::TRACE_KEY, if on { 2 } else { 1 }) {
        i18n::putln(p, Msg::MsgSaved);
    } else {
        i18n::putln(p, Msg::MsgNotsaved);
    }
}

/// `RUN <file>` -- execute a batch file from the ROM volume.
fn cmd_run(p: &mut dyn Platform, m: &mut Marquee, a: &str) {
    if a.is_empty() {
        i18n::putln(p, Msg::MsgParammissing);
        return;
    }
    if !crate::batch::run(p, m, b'A', a) {
        i18n::putln1(p, Msg::MsgFilenotfound, a);
    }
}

/// `DF` -- the data flash diagnostic.
fn cmd_dlc(p: &mut dyn Platform, _m: &mut Marquee, args: &str) {
    crate::dlc::cmd(p, args);
}

fn cmd_df(p: &mut dyn Platform, _m: &mut Marquee, a: &str) {
    crate::df::run(p, a);
}

/// `SETUP` -- the BIOS setup utility. Also reached with DEL at the prompt.
fn cmd_setup(p: &mut dyn Platform, m: &mut Marquee, _a: &str) {
    crate::setup::run(p, m);
}

fn cmd_post(p: &mut dyn Platform, _m: &mut Marquee, _a: &str) {
    crate::post::run(p);
}

fn cmd_reboot(p: &mut dyn Platform, _m: &mut Marquee, _a: &str) {
    i18n::putln(p, Msg::MsgRestarting);
    p.reboot();
}
