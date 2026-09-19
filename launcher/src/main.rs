//! `akterm` — find the board and be its terminal.
//!
//! The kernel runs on the RA4M1, not here. This is only the glass teletype on
//! the end of the wire: it locates the board by USB VID/PID, opens the port and
//! shuttles bytes. Ship one of these per OS and a user needs neither Rust nor
//! this repository.
//!
//! ```text
//!   akterm                 auto-detect and connect
//!   akterm --list          show every serial port and which ones match
//!   akterm --port COM7     connect to a specific port
//!   akterm --baud 9600     non-default speed
//!   akterm --send x.dlc    compile a story pack and load it onto the board
//! ```
//!
//! `Ctrl-]` quits. If the board resets — which reflashing does — the port
//! disappears and comes back; `akterm` reconnects on its own rather than
//! making you restart it.

mod dlc;

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::time::Duration;

/// Arduino's USB vendor ID, and the UNO R4 WiFi's product ID. This is the
/// ESP32-S3 bridge presenting the CDC interface, not the RA4M1 — see README.
const VID: u16 = 0x2341;
const PID: u16 = 0x1002;

const DEFAULT_BAUD: u32 = 115_200;
/// Ctrl-] , the classic telnet escape.
const QUIT: u8 = 0x1D;

fn main() {
    let mut baud = DEFAULT_BAUD;
    let mut want_port: Option<String> = None;
    let mut list_only = false;
    let mut watch = false;
    let mut post = false;
    let mut send: Option<String> = None;

    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--list" | "-l" => list_only = true,
            "--watch" | "-w" => watch = true,
            "--post" => post = true,
            "--send" | "-s" => send = args.next(),
            "--port" | "-p" => want_port = args.next(),
            "--baud" | "-b" => {
                if let Some(v) = args.next() {
                    baud = v.parse().unwrap_or(DEFAULT_BAUD);
                }
            }
            "--help" | "-h" => {
                println!("{}", HELP);
                return;
            }
            other => {
                eprintln!("unknown argument: {other}\n\n{HELP}");
                std::process::exit(2);
            }
        }
    }

    if list_only {
        list_ports();
        return;
    }

    // Sending is not a terminal session: it opens the port, does one thing and
    // says how it went. Doing it before raw mode is entered keeps the output
    // readable when it is run from a script.
    if let Some(path) = send {
        let Some(port) = want_port.clone().or_else(autodetect) else {
            eprintln!("No ArduinoKernel board found (looking for USB {VID:04x}:{PID:04x}).");
            std::process::exit(1);
        };
        std::process::exit(dlc::send(&port, baud, &path));
    }

    if !watch && want_port.is_none() && autodetect().is_none() {
        eprintln!("No ArduinoKernel board found (looking for USB {VID:04x}:{PID:04x}).\n");
        list_ports();
        eprintln!("\nIs the board plugged in? Use --port, or --watch to wait for it.");
        std::process::exit(1);
    }

    let restore = term::enter_raw();

    // stdin is read on its own thread and funnelled through a channel: a
    // blocking read cannot be cancelled, so it must not live on the thread
    // that also has to notice a disconnect.
    let (tx, rx) = mpsc::channel::<u8>();
    std::thread::spawn(move || {
        let mut buf = [0u8; 256];
        let mut stdin = std::io::stdin();
        while let Ok(n) = stdin.read(&mut buf) {
            if n == 0 {
                break;
            }
            for &b in &buf[..n] {
                if tx.send(b).is_err() {
                    return;
                }
            }
        }
    });

    let code = run(&want_port, baud, watch, post, &rx);
    term::restore(&restore);
    println!();
    std::process::exit(code);
}

/// Connect, pump bytes, and reconnect if the board goes away.
///
/// In `--watch` mode the port is re-detected on every attempt, so unplugging
/// and replugging — even into a different USB socket, which can change the
/// port name — reconnects on its own. That is what makes this usable as a
/// login item.
fn run(want: &Option<String>, baud: u32, watch: bool, post: bool, rx: &Receiver<u8>) -> i32 {
    let mut waiting = false;
    loop {
        let port_name = match want.clone().or_else(autodetect) {
            Some(p) => p,
            None => {
                if !watch {
                    return 1;
                }
                if !waiting {
                    eprint!("\r\nwaiting for a board...\r\n");
                    waiting = true;
                }
                if wait_or_quit(rx, Duration::from_millis(1000)) {
                    return 0;
                }
                continue;
            }
        };
        waiting = false;
        eprint!("\r\nakterm — {port_name} @ {baud} 8N1.  Ctrl-] to quit.\r\n");

        let port = serialport::new(&port_name, baud)
            .timeout(Duration::from_millis(50))
            .open();

        let mut port = match port {
            Ok(p) => p,
            Err(e) => {
                // The board is mid-reset, or someone else holds the port.
                eprint!("\r\nwaiting for {port_name} ({e})...\r\n");
                if wait_or_quit(rx, Duration::from_millis(800)) {
                    return 0;
                }
                continue;
            }
        };

        // Assert DTR/RTS. The ESP32-S3 bridge does not relay the RA4M1's UART
        // until the host raises DTR, so without this the port opens cleanly and
        // then sits in complete silence -- which looks like a dead board.
        let _ = port.write_data_terminal_ready(true);
        let _ = port.write_request_to_send(true);
        std::thread::sleep(Duration::from_millis(200));

        // Nudge the kernel so a prompt appears instead of a blank screen.
        //
        // With --post, ask for the boot screen instead. The board prints POST
        // the moment it powers up, long before a terminal can be launched and
        // a port opened, so those bytes are always lost. POST re-runs the same
        // code and produces the same screen -- deterministically, rather than
        // racing the boot.
        let _ = port.write_all(if post { b"POST\r".as_slice() } else { b"\r".as_slice() });
        let _ = port.flush();

        let alive = Arc::new(AtomicBool::new(true));
        let mut reader = match port.try_clone() {
            Ok(r) => r,
            Err(e) => {
                eprint!("\r\ncannot clone port: {e}\r\n");
                return 1;
            }
        };

        let alive_rx = alive.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 1024];
            let mut out = std::io::stdout();
            while alive_rx.load(Ordering::Relaxed) {
                match reader.read(&mut buf) {
                    Ok(0) => {}
                    Ok(n) => {
                        let _ = out.write_all(&buf[..n]);
                        let _ = out.flush();
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {}
                    Err(_) => break, // unplugged or reset
                }
            }
            alive_rx.store(false, Ordering::Relaxed);
        });

        // Keyboard -> board, until the link drops or the user quits.
        //
        // `Disconnected` means stdin reached EOF, which happens when input is
        // piped rather than typed. Exiting immediately would cut off the
        // board's reply mid-sentence, so linger briefly and keep printing --
        // that is what makes `echo VER | akterm` usable in a script.
        let mut drain_until: Option<std::time::Instant> = None;
        while alive.load(Ordering::Relaxed) {
            if let Some(deadline) = drain_until {
                if std::time::Instant::now() >= deadline {
                    alive.store(false, Ordering::Relaxed);
                    return 0;
                }
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(QUIT) => {
                    alive.store(false, Ordering::Relaxed);
                    return 0;
                }
                Ok(b) => {
                    // Coalesce whatever else is already queued into one write.
                    //
                    // Writing a byte at a time and flushing after each one
                    // makes every keystroke a separate USB CDC transaction,
                    // costing milliseconds apiece. An arrow key is three bytes
                    // (`ESC [ A`), so it arrived at the board spread over tens
                    // of milliseconds -- long enough for the firmware to give
                    // up waiting and read the ESC as a lone keypress. Sent as
                    // one burst, the sequence lands intact.
                    let mut buf = [0u8; 64];
                    buf[0] = b;
                    let mut n = 1;
                    while n < buf.len() {
                        match rx.try_recv() {
                            Ok(QUIT) => {
                                let _ = port.write_all(&buf[..n]);
                                let _ = port.flush();
                                alive.store(false, Ordering::Relaxed);
                                return 0;
                            }
                            Ok(nb) => {
                                buf[n] = nb;
                                n += 1;
                            }
                            Err(_) => break,
                        }
                    }
                    if port.write_all(&buf[..n]).is_err() || port.flush().is_err() {
                        break;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    drain_until = Some(std::time::Instant::now() + Duration::from_millis(1500));
                }
            }
        }

        alive.store(false, Ordering::Relaxed);
        eprint!("\r\n-- link lost --\r\n");
        if wait_or_quit(rx, Duration::from_millis(500)) {
            return 0;
        }
        if !watch && want.is_none() {
            return 0; // one-shot mode: the board is gone, so are we
        }
    }
}

/// Sleep, but stay responsive to Ctrl-] while disconnected.
fn wait_or_quit(rx: &Receiver<u8>, d: Duration) -> bool {
    matches!(rx.recv_timeout(d), Ok(QUIT))
}

fn autodetect() -> Option<String> {
    let ports = serialport::available_ports().ok()?;
    ports
        .into_iter()
        .find(|p| matches!(&p.port_type,
            serialport::SerialPortType::UsbPort(u) if u.vid == VID && u.pid == PID))
        .map(|p| p.port_name)
}

fn list_ports() {
    match serialport::available_ports() {
        Ok(ports) if !ports.is_empty() => {
            eprintln!("Serial ports:");
            for p in ports {
                match &p.port_type {
                    serialport::SerialPortType::UsbPort(u) => {
                        let hit = if u.vid == VID && u.pid == PID {
                            "  <- ArduinoKernel board"
                        } else {
                            ""
                        };
                        eprintln!(
                            "  {:<12} USB {:04x}:{:04x}  {}{}",
                            p.port_name,
                            u.vid,
                            u.pid,
                            u.product.clone().unwrap_or_else(|| "-".into()),
                            hit
                        );
                    }
                    other => eprintln!("  {:<12} {:?}", p.port_name, other),
                }
            }
        }
        Ok(_) => eprintln!("No serial ports found."),
        Err(e) => eprintln!("Cannot enumerate serial ports: {e}"),
    }
}

const HELP: &str = "\
akterm — terminal for an ArduinoKernel board

USAGE:
    akterm [OPTIONS]

OPTIONS:
    -l, --list          List serial ports and mark any matching board
    -p, --port <NAME>   Connect to a specific port (COM3, /dev/ttyACM0, ...)
    -w, --watch         Wait for a board instead of failing, and keep waiting
                        after it is unplugged. Use this as a login item.
        --post          Ask the board to draw its BIOS screen on connect
    -s, --send <FILE>   Compile a story source and send it with DLC LOAD
    -b, --baud <RATE>   Baud rate (default 115200)
    -h, --help          This text

Ctrl-] quits. Reconnects automatically if the board resets.";

// ---------------------------------------------------------------- raw mode

/// Windows: raw input, ANSI output, UTF-8 code page. Degrades quietly when
/// stdio is redirected, so the tool stays scriptable.
#[cfg(windows)]
mod term {
    use std::ffi::c_void;

    const STD_INPUT: u32 = 0xFFFF_FFF6;
    const STD_OUTPUT: u32 = 0xFFFF_FFF5;
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

/// Unix: cfmakeraw on stdin, restored on exit.
///
/// NOTE: written but not tested — there was no Linux or macOS machine to hand.
/// The Windows path is verified.
#[cfg(unix)]
mod term {
    pub struct Restore(Option<libc::termios>);

    pub fn enter_raw() -> Restore {
        unsafe {
            let mut t: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(libc::STDIN_FILENO, &mut t) != 0 {
                return Restore(None); // not a tty, e.g. piped
            }
            let saved = t;
            libc::cfmakeraw(&mut t);
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &t);
            Restore(Some(saved))
        }
    }

    pub fn restore(r: &Restore) {
        if let Some(t) = r.0 {
            unsafe {
                libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &t);
            }
        }
    }
}
