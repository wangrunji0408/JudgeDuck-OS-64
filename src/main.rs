#![no_std]
#![no_main]
#![feature(asm)]

#[macro_use]
extern crate log;

use uefi::prelude::*;

#[no_mangle]
pub extern "C" fn efi_main(image: uefi::Handle, st: SystemTable<Boot>) -> Status {
    // Initialize utilities (logging, memory allocation...)
    uefi_services::init(&st).expect_success("failed to initialize utilities");
    info!("Welcome to JudgeDuck OS 64!");
    unimplemented!();
}

/// Workaround for Rust compiler bug:
/// https://github.com/rust-lang/rust/issues/62785
#[used]
#[no_mangle]
pub static _fltused: i32 = 0;
