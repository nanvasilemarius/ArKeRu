//! Host harness: runs the exact same `kernel` crate in a PC terminal.
//!
//! This exists so the shell, the POST screen and every command can be developed
//! and tested with no board attached. The only thing that differs from the
//! RA4M1 build is this file -- `kernel` itself is byte-identical.
//!
//!   cargo run -p host
//!
//! REBOOT quits. The reported RAM/ROM figures are the RA4M1's, so the POST
//! screen shows what the board will show.

use kernel::{Kernel, Platform};
use std::io::{Read, Write};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

// The board's real figures, mirrored so the POST screen is representative.
const TARGET_RAM: usize = 32 * 1024;
const TARGET_ROM: usize = 240 * 1024; // 256K flash less the 16K bootloader

fn main() {
    let restore = term::enter_raw();

    let (tx, rx) = mpsc::channel::<u8>();
    // stdin is read on its own thread so `Platform::get` can stay non-blocking,
    // matching the interrupt-driven UART on the real board.
    std::thread::spawn(move || {
        let mut buf = [0u8; 64];
        let mut stdin = std::io::stdin();
        loop {
            match stdin.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    for &b in &buf[..n] {
                        if tx.send(b).is_err() {
                            return;
                        }
                    }
                }
            }
        }
    });

    let mut plat = HostPlatform {
        rx,
        out: Vec::with_capacity(4096),
        start: Instant::now(),
        restore,
        alt: false,
        alt_match: 0,
    };

    let mut k = Kernel::new();
    k.run(&mut plat);
}

struct HostPlatform {
    rx: Receiver<u8>,
    out: Vec<u8>,
    start: Instant,
    restore: term::Restore,
    /// Whether the kernel has switched to the terminal's alternate screen.
    ///
    /// On the board the LED panel is 96 physical pixels and shares nothing with
    /// the terminal. Here it is emulated *on the same screen*, in a fixed
    /// region at the top right -- which lands squarely inside a full-screen
    /// view's layout and corrupts it. The panel is a convenience for developing
    /// LED commands, so it yields to whatever owns the whole screen.
    alt: bool,
    /// How much of the alternate-screen sequence has matched so far.
    alt_match: usize,
}

/// `ESC [ ? 1 0 4 9` -- the common prefix of enter and leave; the final byte
/// decides which.
const ALT_PREFIX: &[u8] = b"\x1b[?1049";

impl HostPlatform {
    /// Track the alternate-screen sequences as they go past on the way out.
    fn watch_alt(&mut self, b: u8) {
        if self.alt_match == ALT_PREFIX.len() {
            if b == b'h' || b == b'l' {
                self.alt = b == b'h';
            }
            self.alt_match = 0;
            return;
        }
        if b == ALT_PREFIX[self.alt_match] {
            self.alt_match += 1;
        } else {
            // Restart rather than reset: ESC may begin the next sequence.
            self.alt_match = usize::from(b == 0x1b);
        }
    }

    fn flush(&mut self) {
        if !self.out.is_empty() {
            let mut so = std::io::stdout();
            let _ = so.write_all(&self.out);
            let _ = so.flush();
            self.out.clear();
        }
    }
}

impl Platform for HostPlatform {
    fn put(&mut self, b: u8) {
        self.watch_alt(b);
        self.out.push(b);
        // Keep the buffer bounded in case a command produces a lot at once.
        if self.out.len() >= 4096 {
            self.flush();
        }
    }

    fn get(&mut self) -> Option<u8> {
        // Anything queued for output is flushed before we block for input --
        // otherwise the prompt would not appear until after the keystroke.
        self.flush();
        self.rx.recv_timeout(Duration::from_millis(20)).ok()
    }

    fn uptime_ms(&mut self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    fn delay_ms(&mut self, ms: u32) {
        self.flush();
        std::thread::sleep(Duration::from_millis(ms as u64));
    }

    fn cpu_name(&self) -> &'static str {
        "Renesas RA4M1 Cortex-M4F"
    }

    fn cpu_mhz(&self) -> u32 {
        48
    }

    fn ram_total(&self) -> usize {
        TARGET_RAM
    }

    fn ram_used(&self) -> usize {
        // What the kernel itself occupies, plus a nominal stack reservation.
        core::mem::size_of::<Kernel>() + 2048
    }

    fn rom_total(&self) -> usize {
        TARGET_ROM
    }

    fn board_name(&self) -> &'static str {
        "Arduino UNO R4 WiFi (host emulation)"
    }

    fn console_name(&self) -> &'static str {
        "stdio (host)"
    }

    fn reboot(&mut self) -> ! {
        self.flush();
        term::restore(&self.restore);
        println!();
        std::process::exit(0);
    }

    fn has_led_matrix(&self) -> bool {
        true
    }

    /// Render the 12x8 matrix as text so the LED commands are visible here too.
    ///
    /// Drawn into a fixed region at the top-right with the cursor saved and
    /// restored around it. A scrolling marquee repaints ~11 times a second, so
    /// printing inline would bury the shell in panel art within seconds.
    fn led_matrix(&mut self, rows: &[u16; 8]) {
        const TOP: usize = 2;
        const LEFT: usize = 52;

        // A full-screen view owns every cell; the emulated panel does not get
        // to scribble in the middle of it. On the board this question does not
        // arise -- the panel is 96 LEDs on a different piece of hardware.
        if self.alt {
            return;
        }

        let mut s = String::from("\x1b7"); // save cursor
        for (i, r) in rows.iter().enumerate() {
            s.push_str(&format!("\x1b[{};{}H", TOP + i, LEFT));
            for c in (0..12).rev() {
                s.push_str(if r & (1 << c) != 0 { "██" } else { "░░" });
            }
        }
        s.push_str("\x1b8"); // restore cursor
        for b in s.into_bytes() {
            self.put(b);
        }
        self.flush();
    }
}

/// Windows console plumbing: raw input, ANSI output, UTF-8 code page.
///
/// All of it degrades quietly when stdin/stdout is a pipe rather than a
/// console, so the harness stays scriptable.
#[cfg(windows)]
mod term {
    use std::ffi::c_void;

    const STD_INPUT: u32 = 0xFFFF_FFF6; // -10
    const STD_OUTPUT: u32 = 0xFFFF_FFF5; // -11

    const ENABLE_PROCESSED_INPUT: u32 = 0x0001;
    const ENABLE_LINE_INPUT: u32 = 0x0002;
    const ENABLE_ECHO_INPUT: u32 = 0x0004;
    const ENABLE_VIRTUAL_TERMINAL_INPUT: u32 = 0x0200;
    const ENABLE_VIRTUAL_TERMINAL_PROCESSING: u32 = 0x0004;

    #[link(name = "kernel32")]
    extern "system" {
        fn GetStdHandle(n: u32) -> *mut c_void;
        fn GetConsoleMode(h: *mut c_void, m: *mut u32) -> i32;
        fn SetConsoleMode(h: *mut c_void, m: u32) -> i32;
        fn SetConsoleOutputCP(cp: u32) -> i32;
        fn SetConsoleCP(cp: u32) -> i32;
    }

    pub struct Restore {
        input: Option<u32>,
        output: Option<u32>,
    }

    pub fn enter_raw() -> Restore {
        unsafe {
            // Box drawing needs UTF-8; ignore failure when not a console.
            SetConsoleOutputCP(65001);
            SetConsoleCP(65001);

            let hin = GetStdHandle(STD_INPUT);
            let hout = GetStdHandle(STD_OUTPUT);

            let mut old_in = 0u32;
            let input = if GetConsoleMode(hin, &mut old_in) != 0 {
                let raw = (old_in
                    & !(ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT | ENABLE_PROCESSED_INPUT))
                    | ENABLE_VIRTUAL_TERMINAL_INPUT;
                SetConsoleMode(hin, raw);
                Some(old_in)
            } else {
                None
            };

            let mut old_out = 0u32;
            let output = if GetConsoleMode(hout, &mut old_out) != 0 {
                SetConsoleMode(hout, old_out | ENABLE_VIRTUAL_TERMINAL_PROCESSING);
                Some(old_out)
            } else {
                None
            };

            Restore { input, output }
        }
    }

    pub fn restore(r: &Restore) {
        unsafe {
            if let Some(m) = r.input {
                SetConsoleMode(GetStdHandle(STD_INPUT), m);
            }
            if let Some(m) = r.output {
                SetConsoleMode(GetStdHandle(STD_OUTPUT), m);
            }
        }
    }
}

#[cfg(not(windows))]
mod term {
    pub struct Restore;
    pub fn enter_raw() -> Restore {
        Restore
    }
    pub fn restore(_: &Restore) {}
}
