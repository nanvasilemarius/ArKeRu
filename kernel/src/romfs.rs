//! A read-only file table baked into flash.
//!
//! There is no writable filesystem: the RA4M1 has 8 KB of data flash and
//! wearing it out from a shell would be a poor trade. `DIR` and `TYPE` read
//! from here. When the ESP32-S3's 8 MB flash is wired up as a block device this
//! is the interface that would grow a second backend.
//!
//! # Why the contents live in `i18n/`
//!
//! The bodies are message keys, not literals, so a file follows `LANG` like
//! everything else. Without that, `AUTOEXEC.BAT` printed `System ready.` in
//! English at the end of an otherwise fully Romanian boot -- the last line on
//! screen, and the one that undid the illusion. `TYPE README.TXT` had the same
//! problem in a bigger way.
//!
//! Only the prose moves. `CONFIG.SYS` keeps its `DEVICE=` lines and
//! `AUTOEXEC.BAT` keeps `VER` and `LED HEART` verbatim in every language:
//! those are commands, and a command name is not a word.

use crate::i18n::{self, Chars, Msg, Text};

/// One entry in the ROM volume.
pub struct File {
    pub name: &'static str,
    pub ext: &'static str,
    /// The contents, as a translatable message.
    pub text: Msg,
    /// Fake timestamp, purely for the DIR listing's period feel.
    pub date: &'static str,
    pub time: &'static str,
}

impl File {
    /// The contents in the active language.
    pub fn body(&self) -> Text {
        i18n::t(self.text)
    }

    /// Size in bytes, which genuinely differs between languages -- so `DIR`
    /// reports a different number under `LANG RO`. That is not a bug: the file
    /// really is a different length.
    pub fn size(&self) -> usize {
        self.body().len()
    }
}

pub const VOLUME: &str = "ARDUINODOS";

pub static FILES: &[File] = &[
    File {
        name: "README",
        ext: "TXT",
        date: "07-29-26",
        time: "10:24a",
        text: Msg::FileReadme,
    },
    File {
        name: "CONFIG",
        ext: "SYS",
        date: "07-29-26",
        time: "10:24a",
        text: Msg::FileConfig,
    },
    File {
        name: "AUTOEXEC",
        ext: "BAT",
        date: "07-29-26",
        time: "10:24a",
        text: Msg::FileAutoexec,
    },
    File {
        name: "HARDWARE",
        ext: "TXT",
        date: "07-29-26",
        time: "10:24a",
        text: Msg::FileHardware,
    },
];

/// Look up a file by `NAME.EXT`, case-insensitively.
pub fn find(spec: &str) -> Option<&'static File> {
    let (want_name, want_ext) = match spec.find('.') {
        Some(i) => (&spec[..i], &spec[i + 1..]),
        None => (spec, ""),
    };
    FILES.iter().find(|f| {
        eq_ignore_case(f.name, want_name) && (want_ext.is_empty() || eq_ignore_case(f.ext, want_ext))
    })
}

fn eq_ignore_case(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .all(|(x, y)| x.to_ascii_uppercase() == y.to_ascii_uppercase())
}

/// Total bytes occupied by the volume.
pub fn total_bytes() -> usize {
    FILES.iter().map(|f| f.size()).sum()
}
