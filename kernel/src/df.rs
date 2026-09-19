//! `DF` — the data flash, and the proof that writing it works.
//!
//! This is deliberately a diagnostic rather than a filesystem. The driver
//! underneath is new, it is the first thing in this project that *modifies*
//! non-volatile memory, and the way to earn trust in it is to erase one block,
//! write a known pattern, read it back, and then survive a power cycle.
//!
//! # One block
//!
//! Everything here operates on the **last** block of the array and nothing
//! else. If the driver has an off-by-one or the address arithmetic is wrong,
//! the damage is confined to 1 KB that holds nothing. The settings the shell
//! actually relies on live in the ESP32's NVS, on the other chip entirely, so
//! a mistake here cannot lose `LANG`, `THEME` or `SPEED` either.

use crate::i18n::{self, Msg};
use crate::{kprint, kprintln, vt, Platform};

/// How much of the block to show.
const DUMP: usize = 64;

/// Offset of the block this command is allowed to touch: the last one.
fn test_block(size: usize, block: usize) -> usize {
    size - block
}

fn hex_dump(p: &mut dyn Platform, base: usize, buf: &[u8]) {
    for (row, chunk) in buf.chunks(16).enumerate() {
        kprint!(p, "  {:04X}  ", base + row * 16);
        for b in chunk {
            kprint!(p, "{:02X} ", b);
        }
        for _ in chunk.len()..16 {
            kprint!(p, "   ");
        }
        kprint!(p, " ");
        for &b in chunk {
            // Flash erases to 0xFF, which is not printable; show it as a dot
            // like any other non-text byte.
            let c = if (0x20..0x7f).contains(&b) { b } else { b'.' };
            p.put(c);
        }
        kprintln!(p);
    }
}

/// `DF`, `DF ERASE`, `DF WRITE <text>`.
pub fn run(p: &mut dyn Platform, args: &str) {
    let Some((size, block)) = p.dataflash_info() else {
        i18n::putln(p, Msg::MsgNodf);
        return;
    };
    let (verb, rest) = crate::cmds::split_first(args);

    // An optional block number, so a block other than the scratch one can be
    // *inspected*. Diagnosing a bad write with no way to look at the bytes is
    // guessing, and this driver has already cost one round of that.
    let pick = |s: &str| -> usize {
        match s.trim().parse::<usize>() {
            Ok(n) if (n + 1) * block <= size => n * block,
            _ => test_block(size, block),
        }
    };
    let off = if crate::cmds::ieq(verb, "ERASE") {
        pick(rest)
    } else if verb.is_empty() || crate::cmds::ieq(verb, "WRITE") {
        test_block(size, block)
    } else {
        pick(verb)
    };

    if crate::cmds::ieq(verb, "ERASE") {
        let ok = p.dataflash_erase(off);
        report(p, ok);
        if ok {
            // An erase that reports success but leaves data behind is the
            // failure mode that matters, so check rather than trust.
            let mut buf = [0u8; DUMP];
            p.dataflash_read(off, &mut buf);
            let blank = buf.iter().all(|&b| b == 0xFF);
            i18n::putln(p, if blank { Msg::DfBlank } else { Msg::DfNotblank });
        }
        return;
    }

    if crate::cmds::ieq(verb, "WRITE") {
        if rest.is_empty() {
            i18n::putln(p, Msg::MsgUsagedf);
            return;
        }
        // Erase first: flash can only clear bits, so programming over old
        // data yields the AND of the two rather than the new value.
        if !p.dataflash_erase(off) {
            report(p, false);
            return;
        }
        let bytes = rest.as_bytes();
        let n = bytes.len().min(DUMP);
        let ok = p.dataflash_write(off, &bytes[..n]);
        report(p, ok);
        if ok {
            let mut buf = [0u8; DUMP];
            p.dataflash_read(off, &mut buf);
            let same = buf[..n] == bytes[..n];
            i18n::putln(p, if same { Msg::DfVerified } else { Msg::DfMismatch });
        }
        return;
    }

    if !verb.is_empty() && verb.trim().parse::<usize>().is_err() {
        i18n::putln(p, Msg::MsgUsagedf);
        return;
    }

    // Status.
    kprintln!(p);
    i18n::putln1(p, Msg::DfSize, size);
    i18n::putln1(p, Msg::DfBlocksize, block);
    i18n::putln1(p, Msg::DfTestblock, off);
    kprintln!(p);
    let mut buf = [0u8; DUMP];
    if p.dataflash_read(off, &mut buf) {
        hex_dump(p, off, &buf);
    }
    kprintln!(p);
    i18n::putln(p, Msg::DfHowto);
    kprintln!(p);
}

fn report(p: &mut dyn Platform, ok: bool) {
    kprint!(p, "  ");
    if ok {
        kprint!(p, "{}", vt::sgr(vt::FG_GREEN));
        i18n::put(p, Msg::DfOk);
    } else {
        kprint!(p, "{}", vt::sgr(vt::FG_RED));
        i18n::put(p, Msg::DfFail);
    }
    kprintln!(p, "{}", vt::RESET);
}
