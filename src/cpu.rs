use alloc::boxed::Box;

use x86_64::instructions::tables::{load_tss};
use x86_64::structures::gdt::{Descriptor, SegmentSelector};
use x86_64::structures::tss::TaskStateSegment;
use x86_64::structures::{DescriptorTablePointer};
use x86_64::{VirtAddr};

/// Add GDT entries for user mode
pub fn setup_gdt() {
    let ucode = match Descriptor::user_code_segment() {
        Descriptor::UserSegment(x) => x,
        _ => unreachable!(),
    };
    let udata = match Descriptor::user_data_segment() {
        Descriptor::UserSegment(x) => x,
        _ => unreachable!(),
    };

    // allocate stack for trap from user
    // set the stack top to TSS
    // so that when trap from ring3 to ring0, CPU can switch stack correctly
    let mut tss = Box::new(TaskStateSegment::new());
    let trap_stack_top = Box::leak(Box::new([0u8; 0x1000])).as_ptr() as u64 + 0x1000;
    tss.privilege_stack_table[0] = VirtAddr::new(trap_stack_top);
    let tss: &'static _ = Box::leak(tss);
    let (tss0, tss1) = match Descriptor::tss_segment(tss) {
        Descriptor::SystemSegment(tss0, tss1) => (tss0, tss1),
        _ => unreachable!(),
    };

    unsafe {
        let gdtp = sgdt();
        let gdt_base = gdtp.base as *mut u64;
        // overwrite GDT entries: 2,3,4,5
        gdt_base.add(2).write(tss0);
        gdt_base.add(3).write(tss1);
        gdt_base.add(4).write(udata);
        gdt_base.add(5).write(ucode);
        load_tss(SegmentSelector(0x10));
    }
    // TODO: backup and recover
}

/// Get current GDT register
#[inline]
unsafe fn sgdt() -> DescriptorTablePointer {
    let mut gdt = DescriptorTablePointer { limit: 0, base: 0 };
    asm!("sgdt ($0)" :: "r" (&mut gdt) : "memory");
    gdt
}
