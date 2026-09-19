//! AT client for the ESP32-S3 bridge, over SCI1 (P501/P502 = Arduino `Serial2`).
//!
//! The bridge exposes a key/value store backed by the ESP32's own NVS — the
//! same one Arduino's `Preferences` library uses on this board. That is how
//! settings persist without ever writing RA4M1 flash.
//!
//! # Safety rules, learned the hard way
//!
//! The ESP32 owns the USB console *and* the sketch upload path. Wedging it
//! costs a physical unplug, so this client is deliberately defensive:
//!
//! - **Handshake first.** `AT+SOFTRESETWIFI` puts the bridge in a known state.
//!   Skipping it and firing commands at a bridge that was not listening is what
//!   wedged it once already.
//! - **Every read is bounded.** No loop here can spin waiting on a bridge that
//!   has stopped answering.
//! - **Stale input is drained before every command**, so a late reply from a
//!   previous command is never mistaken for this one's.
//! - **No bursts.** One command, one bounded reply, then return.

use crate::{millis, ra4m1::Sci};
use core::fmt::Write;

/// Preferences type codes, from the core's `Preferences.h` enum.
const PT_U8: u8 = 1;

/// Namespace for our settings inside the ESP32's NVS.
const NAMESPACE: &str = "akernel";

/// Per-command reply budget. Generous next to the bridge's own 500 ms
/// handshake timeout, but still bounded.
const REPLY_MS: u32 = 600;
/// Handshake budget per attempt, matching what `ModemClass::begin` uses.
const SYNC_MS: u32 = 400;
const SYNC_TRIES: u8 = 2;

/// Fixed-capacity string builder, so commands can be formatted without alloc.
struct Buf {
    b: [u8; 96],
    n: usize,
}

impl Buf {
    const fn new() -> Self {
        Self { b: [0; 96], n: 0 }
    }
    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.b[..self.n]).unwrap_or("")
    }
}

impl Write for Buf {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for &c in s.as_bytes() {
            if self.n == self.b.len() {
                return Err(core::fmt::Error);
            }
            self.b[self.n] = c;
            self.n += 1;
        }
        Ok(())
    }
}

/// Outcome of one command.
pub enum Reply {
    /// Bridge answered `OK`. `data` holds the payload after `+CMD:` if any.
    Ok,
    Error,
    Timeout,
}

pub struct Modem {
    sci: Sci,
    synced: bool,
}

impl Modem {
    pub const fn new(sci: Sci) -> Self {
        Self { sci, synced: false }
    }

    /// Throw away anything the bridge left in the pipe.
    fn drain(&self) {
        // Bounded: at 115200 a byte arrives every ~87 us, so 4096 iterations
        // cannot be outrun by the sender.
        for _ in 0..4096 {
            if self.sci.get().is_none() {
                return;
            }
        }
    }

    fn send_line(&self, s: &str) {
        for b in s.as_bytes() {
            if !self.sci.put(*b) {
                return; // transmitter wedged; give up rather than spin
            }
        }
        let _ = self.sci.put(b'\r');
        let _ = self.sci.put(b'\n');
    }

    /// Read until `OK`/`ERROR` or the deadline. Any `+CMD:payload` line is
    /// copied into `data`, and its length returned alongside the verdict.
    fn read_reply(&self, data: &mut [u8], timeout_ms: u32) -> (Reply, usize) {
        let start = millis();
        let mut line = [0u8; 96];
        let mut n = 0usize;
        let mut data_len = 0usize;

        while millis().wrapping_sub(start) < timeout_ms {
            let Some(b) = self.sci.get() else { continue };

            if b == b'\n' || b == b'\r' {
                if n == 0 {
                    continue; // blank line between records
                }
                let s = core::str::from_utf8(&line[..n]).unwrap_or("");

                if eq_ignore_case(s, "OK") {
                    return (Reply::Ok, data_len);
                }
                if eq_ignore_case(s, "ERROR") {
                    return (Reply::Error, data_len);
                }
                // `+PREFGET:value` -- keep whatever follows the colon.
                if let Some(i) = s.find(':') {
                    let v = s[i + 1..].trim();
                    let take = v.len().min(data.len());
                    data[..take].copy_from_slice(&v.as_bytes()[..take]);
                    data_len = take;
                }
                n = 0;
                continue;
            }

            if n < line.len() {
                line[n] = b;
                n += 1;
            }
        }
        (Reply::Timeout, data_len)
    }

    fn command(&self, cmd: &str, data: &mut [u8]) -> (Reply, usize) {
        self.drain();
        self.send_line(cmd);
        self.read_reply(data, REPLY_MS)
    }

    /// Put the bridge in a known state. Cheap to call repeatedly; only the
    /// first success matters.
    pub fn sync(&mut self) -> bool {
        if self.synced {
            return true;
        }
        let mut scratch = [0u8; 32];
        for _ in 0..SYNC_TRIES {
            self.drain();
            self.send_line("AT+SOFTRESETWIFI");
            if let (Reply::Ok, _) = self.read_reply(&mut scratch, SYNC_MS) {
                self.synced = true;
                return true;
            }
        }
        false
    }

    /// Read a `u8` setting from the ESP32's NVS.
    pub fn get_u8(&mut self, key: &str) -> Option<u8> {
        if !self.sync() {
            return None;
        }
        let mut open = Buf::new();
        write!(open, "AT+PREFBEGIN={NAMESPACE},1,").ok()?;
        let mut scratch = [0u8; 64];
        if !matches!(self.command(open.as_str(), &mut scratch).0, Reply::Ok) {
            return None;
        }

        let mut get = Buf::new();
        write!(get, "AT+PREFGET={key},{PT_U8},0").ok()?;
        let (r, n) = self.command(get.as_str(), &mut scratch);

        let mut end = Buf::new();
        let _ = write!(end, "AT+PREFEND");
        let mut ignore = [0u8; 16];
        let _ = self.command(end.as_str(), &mut ignore);

        match r {
            Reply::Ok if n > 0 => parse_u8(core::str::from_utf8(&scratch[..n]).ok()?),
            _ => None,
        }
    }

    /// Persist a `u8` setting. Returns whether the bridge acknowledged it.
    pub fn set_u8(&mut self, key: &str, val: u8) -> bool {
        if !self.sync() {
            return false;
        }
        let mut open = Buf::new();
        if write!(open, "AT+PREFBEGIN={NAMESPACE},0,").is_err() {
            return false;
        }
        let mut scratch = [0u8; 64];
        if !matches!(self.command(open.as_str(), &mut scratch).0, Reply::Ok) {
            return false;
        }

        let mut put = Buf::new();
        let ok = write!(put, "AT+PREFPUT={key},{PT_U8},{val}").is_ok()
            && matches!(self.command(put.as_str(), &mut scratch).0, Reply::Ok);

        let mut end = Buf::new();
        let _ = write!(end, "AT+PREFEND");
        let mut ignore = [0u8; 16];
        let _ = self.command(end.as_str(), &mut ignore);
        ok
    }
}

fn eq_ignore_case(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .all(|(x, y)| x.to_ascii_uppercase() == y.to_ascii_uppercase())
}

fn parse_u8(s: &str) -> Option<u8> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    let mut v: u32 = 0;
    for c in t.bytes() {
        if !c.is_ascii_digit() {
            return None;
        }
        v = v.checked_mul(10)?.checked_add((c - b'0') as u32)?;
        if v > 255 {
            return None;
        }
    }
    Some(v as u8)
}
