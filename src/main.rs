#![no_std]
#![no_main]
#![feature(asm)]

#[macro_use]
extern crate alloc;

#[macro_use]
extern crate log;

use uefi::prelude::*;
use uefi::proto::media::file::*;
use uefi::proto::media::fs::SimpleFileSystem;
use uefi::table::boot::*;
use xmas_elf::ElfFile;

mod cpu;
mod page_table;

#[no_mangle]
pub extern "C" fn efi_main(_image: uefi::Handle, st: SystemTable<Boot>) -> Status {
    // Initialize utilities (logging, memory allocation...)
    uefi_services::init(&st).expect_success("failed to initialize utilities");
    info!("Welcome to JudgeDuck OS 64!");

    page_table::enable_page_table_editing();
    cpu::setup_gdt();

    let (entry, stacktop) = load_user_program(st.boot_services(), "main");
    go_to_user(entry, stacktop);
    unimplemented!();
}

/// Go to user mode with `rip` and `rsp`
fn go_to_user(rip: usize, rsp: usize) {
    struct TrapFrame {
        // Pushed by CPU
        pub rip: usize,
        pub cs: usize,
        pub rflags: usize,

        // Pushed by CPU when Ring3->0
        pub rsp: usize,
        pub ss: usize,
    }
    let tf = TrapFrame {
        rip,
        cs: 0x28 | 3,
        rflags: 0x002, // to enable interrupt: |= 0x200
        rsp,
        ss: 0x20 | 3,
    };
    unsafe {
        asm!(r#"
            mov rsp, $0
            iretq
        "# :: "r"(&tf) :: "intel");
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

/// Workaround for Rust compiler bug:
/// https://github.com/rust-lang/rust/issues/62785
#[used]
#[no_mangle]
pub static _fltused: i32 = 0;
