#![no_std]
#![no_main]
#![feature(asm)]
#![feature(abi_efiapi)]
#![feature(abi_x86_interrupt)]

#[macro_use]
extern crate alloc;

#[macro_use]
extern crate log;

use trapframe::{GeneralRegs, TrapFrame, UserContext};
use uefi::prelude::*;
use uefi::proto::media::file::*;
use uefi::proto::media::fs::SimpleFileSystem;
use uefi::table::boot::*;
use xmas_elf::ElfFile;

mod page_table;

#[entry]
fn efi_main(_image: uefi::Handle, st: SystemTable<Boot>) -> Status {
    // Initialize utilities (logging, memory allocation...)
    uefi_services::init(&st).expect_success("failed to initialize utilities");
    info!("Welcome to JudgeDuck OS 64!");

    page_table::enable_page_table_editing();
    unsafe {
        trapframe::init();
    }

    let (entry, stacktop) = load_user_program(st.boot_services(), "main");

    let mut context = UserContext {
        vector: Default::default(),
        general: GeneralRegs {
            rip: entry,
            rsp: stacktop,
            ..Default::default()
        },
        trap_num: 0,
        error_code: 0,
    };
    let start_tsc = unsafe { core::arch::x86_64::_rdtsc() };
    context.run();
    let end_tsc = unsafe { core::arch::x86_64::_rdtsc() };
    info!("start tsc: {:#x}", start_tsc);
    info!("end   tsc: {:#x}", end_tsc);
    info!("time  tsc: {:#x}", end_tsc - start_tsc);
    unimplemented!();
}

#[no_mangle]
extern "sysv64" fn trap_handler(tf: &mut TrapFrame) {
    match tf.trap_num {
        0x68 => {} // UEFI timer
        _ => panic!("TRAP: {:#x?}", tf),
    }
}

/// Load user program at `path` into memory
/// return (entry, stacktop)
fn load_user_program(bs: &BootServices, path: &str) -> (usize, usize) {
    let mut file = open_file(bs, path);
    let elf_data = load_file(bs, &mut file);
    let elf = ElfFile::new(elf_data).expect("failed to open ELF");
    page_table::map_elf(bs, &elf).unwrap();
    page_table::map_stack(bs, 0xffff9000_00000000, 0x100).unwrap();
    let entry = elf.header.pt2.entry_point() as usize;
    let stacktop = 0xffff9000_00100000;
    (entry, stacktop)
}

/// Open file at `path`
fn open_file(bs: &BootServices, path: &str) -> RegularFile {
    info!("opening file: {}", path);
    // FIXME: use LoadedImageProtocol to get the FileSystem of this image
    let fs = bs
        .locate_protocol::<SimpleFileSystem>()
        .expect_success("failed to get FileSystem");
    let fs = unsafe { &mut *fs.get() };

    let mut root = fs.open_volume().expect_success("failed to open volume");
    let handle = root
        .open(path, FileMode::Read, FileAttribute::empty())
        .expect_success("failed to open file");

    match handle.into_type().expect_success("failed to into_type") {
        FileType::Regular(regular) => regular,
        _ => panic!("Invalid file type"),
    }
}

/// Load file to new allocated pages
fn load_file(bs: &BootServices, file: &mut RegularFile) -> &'static mut [u8] {
    info!("loading file to memory");
    let mut info_buf = [0u8; 0x100];
    let info = file
        .get_info::<FileInfo>(&mut info_buf)
        .expect_success("failed to get file info");
    let pages = info.file_size() as usize / 0x1000 + 1;
    let mem_start = bs
        .allocate_pages(AllocateType::AnyPages, MemoryType::LOADER_DATA, pages)
        .expect_success("failed to allocate pages");
    let buf = unsafe { core::slice::from_raw_parts_mut(mem_start as *mut u8, pages * 0x1000) };
    let len = file.read(buf).expect_success("failed to read file");
    &mut buf[..len]
}
