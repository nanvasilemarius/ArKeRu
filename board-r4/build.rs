//! Puts `memory.x` and `device.x` where the cortex-m-rt linker script can
//! find them.

use std::env;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

fn main() {
    let out = &PathBuf::from(env::var("OUT_DIR").unwrap());
    File::create(out.join("memory.x"))
        .unwrap()
        .write_all(include_bytes!("memory.x"))
        .unwrap();
    // With cortex-m-rt's `device` feature its linker script does
    // `INCLUDE device.x`, which normally comes from a PAC. There is no PAC
    // here: the vector table is `__INTERRUPTS` in main.rs, and every handler
    // is named in Rust rather than PROVIDEd by the linker, so this file has
    // nothing to say -- but it has to exist.
    File::create(out.join("device.x"))
        .unwrap()
        .write_all(b"/* No PAC: see __INTERRUPTS in src/main.rs. */
")
        .unwrap();

    println!("cargo:rustc-link-search={}", out.display());
    println!("cargo:rerun-if-changed=memory.x");
    println!("cargo:rerun-if-changed=build.rs");
}
