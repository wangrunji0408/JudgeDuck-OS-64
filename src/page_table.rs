//! This file is modified from 'page_table.rs' in 'rust-osdev/bootloader'

use uefi::prelude::*;
use uefi::table::boot::*;
use x86_64::registers::control::{Cr0, Cr0Flags, Cr3};
use x86_64::registers::model_specific::{Efer, EferFlags};
use x86_64::structures::paging::{
    mapper::*, FrameAllocator, Mapper, Page, PageSize, PageTable, PageTableFlags, PhysFrame,
    Size4KiB,
};
use x86_64::{align_up, PhysAddr, VirtAddr};
use xmas_elf::{program, ElfFile};

pub fn map_elf(bs: &BootServices, elf: &ElfFile) -> Result<(), MapToError> {
    info!("mapping ELF");
    let mut page_table = current_page_table();
    let mut frame_allocator = UEFIFrameAllocator(bs);
    let kernel_start = PhysAddr::new(elf.input.as_ptr() as u64);
    for segment in elf.program_iter() {
        map_segment(
            &segment,
            kernel_start,
            &mut page_table,
            &mut frame_allocator,
        )?;
    }
    Ok(())
}

/// By default the page of CR3 have write protect.
/// We have to remove that before editing page table.
pub fn enable_page_table_editing() {
    unsafe {
        Efer::update(|f| f.insert(EferFlags::NO_EXECUTE_ENABLE));

        Cr0::update(|f| f.remove(Cr0Flags::WRITE_PROTECT));
        //        let cr3_page = Page::<Size4KiB>::from_start_address(VirtAddr::new(
        //            Cr3::read().0.start_address().as_u64(),
        //        ))
        //        .unwrap();
        //        current_page_table().update_flags(cr3_page, PageTableFlags::PRESENT | PageTableFlags::WRITABLE);
        //        Cr0::update(|f| f.insert(Cr0Flags::WRITE_PROTECT));
    }
}

/// Get current page table from CR3
fn current_page_table() -> OffsetPageTable<'static> {
    let p4_table_addr = Cr3::read().0.start_address().as_u64();
    let p4_table = unsafe { &mut *(p4_table_addr as *mut PageTable) };
    unsafe { OffsetPageTable::new(p4_table, VirtAddr::new(0)) }
}

pub fn map_stack(bs: &BootServices, addr: u64, pages: u64) -> Result<(), MapToError> {
    info!("mapping stack at {:#x}", addr);
    let mut page_table = current_page_table();
    let mut frame_allocator = UEFIFrameAllocator(bs);

    // create a stack
    let stack_start = Page::containing_address(VirtAddr::new(addr));
    let stack_end = stack_start + pages;

    let flags =
        PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE;

    for page in Page::range(stack_start, stack_end) {
        let frame = frame_allocator
            .allocate_frame()
            .ok_or(MapToError::FrameAllocationFailed)?;
        unsafe {
            page_table
                .map_to(page, frame, flags, &mut frame_allocator)?
                .flush();
        }
        set_user_bit(&page);
    }

    Ok(())
}

fn map_segment(
    segment: &program::ProgramHeader,
    kernel_start: PhysAddr,
    page_table: &mut impl Mapper<Size4KiB>,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
) -> Result<(), MapToError> {
    match segment.get_type().unwrap() {
        program::Type::Load => {
            debug!("mapping segment: {:#x?}", segment);
            let mem_size = segment.mem_size();
            let file_size = segment.file_size();
            let file_offset = segment.offset() & !0xfff;
            let phys_start_addr = kernel_start + file_offset;
            let virt_start_addr = VirtAddr::new(segment.virtual_addr());

            let start_page: Page = Page::containing_address(virt_start_addr);
            let start_frame = PhysFrame::containing_address(phys_start_addr);
            let end_frame = PhysFrame::containing_address(phys_start_addr + file_size - 1u64);

            let flags = segment.flags();
            let mut page_table_flags = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE;
            if !flags.is_execute() {
                page_table_flags |= PageTableFlags::NO_EXECUTE
            };
            if flags.is_write() {
                page_table_flags |= PageTableFlags::WRITABLE
            };

            for frame in PhysFrame::range_inclusive(start_frame, end_frame) {
                let offset = frame - start_frame;
                let page = start_page + offset;
                unsafe {
                    page_table
                        .map_to(page, frame, page_table_flags, frame_allocator)?
                        .flush();
                }
                set_user_bit(&page);
            }

            if mem_size > file_size {
                // .bss section (or similar), which needs to be zeroed
                let zero_start = virt_start_addr + file_size;
                let zero_end = virt_start_addr + mem_size;
                if zero_start.as_u64() & 0xfff != 0 {
                    // A part of the last mapped frame needs to be zeroed. This is
                    // not possible since it could already contains parts of the next
                    // segment. Thus, we need to copy it before zeroing.

                    let new_frame = frame_allocator
                        .allocate_frame()
                        .ok_or(MapToError::FrameAllocationFailed)?;

                    type PageArray = [u64; Size4KiB::SIZE as usize / 8];

                    let last_page = Page::containing_address(virt_start_addr + file_size - 1u64);
                    let last_page_ptr = end_frame.start_address().as_u64() as *mut PageArray;
                    let temp_page_ptr = new_frame.start_address().as_u64() as *mut PageArray;

                    unsafe {
                        // copy contents
                        temp_page_ptr.write(last_page_ptr.read());
                    }

                    // remap last page
                    if let Err(e) = page_table.unmap(last_page.clone()) {
                        return Err(match e {
                            UnmapError::ParentEntryHugePage => MapToError::ParentEntryHugePage,
                            UnmapError::PageNotMapped => unreachable!(),
                            UnmapError::InvalidFrameAddress(_) => unreachable!(),
                        });
                    }

                    unsafe {
                        page_table
                            .map_to(last_page, new_frame, page_table_flags, frame_allocator)?
                            .flush();
                        set_user_bit(&last_page);
                    }
                }

                // Map additional frames.
                let start_page: Page = Page::containing_address(VirtAddr::new(align_up(
                    zero_start.as_u64(),
                    Size4KiB::SIZE,
                )));
                let end_page = Page::containing_address(zero_end);
                for page in Page::range_inclusive(start_page, end_page) {
                    let frame = frame_allocator
                        .allocate_frame()
                        .ok_or(MapToError::FrameAllocationFailed)?;
                    unsafe {
                        page_table
                            .map_to(page, frame, page_table_flags, frame_allocator)?
                            .flush();
                        set_user_bit(&page);
                    }
                }

                // zero bss
                unsafe {
                    core::ptr::write_bytes(
                        zero_start.as_mut_ptr::<u8>(),
                        0,
                        (mem_size - file_size) as usize,
                    );
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// Set user bit for 4-level PDEs of the `page`.
/// This is a workaround since `x86_64` crate does not set user bit for PDEs.
fn set_user_bit(page: &Page) {
    let mut page_table = Cr3::read().0.start_address().as_u64() as *mut PageTable;
    for level in 0..4 {
        let index = (page.start_address().as_u64() as usize >> (12 + (3 - level) * 9)) & 0o777;
        let entry = unsafe { &mut (&mut *page_table)[index] };
        entry.set_flags(entry.flags() | PageTableFlags::USER_ACCESSIBLE);
        if level == 3 {
            return;
        }
        page_table = entry.frame().unwrap().start_address().as_u64() as *mut PageTable;
    }
}

/// Use `BootServices::allocate_pages()` as frame allocator
struct UEFIFrameAllocator<'a>(&'a BootServices);

unsafe impl FrameAllocator<Size4KiB> for UEFIFrameAllocator<'_> {
    fn allocate_frame(&mut self) -> Option<PhysFrame<Size4KiB>> {
        let addr = self
            .0
            .allocate_pages(AllocateType::AnyPages, MemoryType::LOADER_DATA, 1)
            .expect_success("failed to allocate frame");
        Some(PhysFrame::containing_address(PhysAddr::new(addr)))
    }
}
