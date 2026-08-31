use core::cell::UnsafeCell;

use spin::Once;
use x86_64::{
    VirtAddr,
    instructions::{
        segmentation::{CS, DS, ES, SS, Segment},
        tables::load_tss,
    },
    structures::{
        gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector},
        tss::TaskStateSegment,
    },
};

pub const DOUBLE_FAULT_IST_INDEX: u16 = 0;

const DOUBLE_FAULT_STACK_SIZE: usize = 4096 * 5;

#[repr(align(16))]
struct KernelStack(UnsafeCell<[u8; DOUBLE_FAULT_STACK_SIZE]>);

// SAFETY: The stack is never accessed through Rust references after startup.
// The CPU exclusively writes to it while handling a double fault on this core.
unsafe impl Sync for KernelStack {}

static DOUBLE_FAULT_STACK: KernelStack = KernelStack(UnsafeCell::new([0; DOUBLE_FAULT_STACK_SIZE]));
static TSS: Once<TaskStateSegment> = Once::new();
static GDT: Once<(GlobalDescriptorTable, Selectors)> = Once::new();

struct Selectors {
    code: SegmentSelector,
    data: SegmentSelector,
    user_code: SegmentSelector,
    user_data: SegmentSelector,
    tss: SegmentSelector,
}

pub fn init() {
    let tss = TSS.call_once(|| {
        let mut tss = TaskStateSegment::new();
        let stack_start = VirtAddr::from_ptr(DOUBLE_FAULT_STACK.0.get());
        tss.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] =
            stack_start + DOUBLE_FAULT_STACK_SIZE as u64;
        tss
    });

    let (gdt, selectors) = GDT.call_once(|| {
        let mut gdt = GlobalDescriptorTable::new();
        let selectors = Selectors {
            code: gdt.append(Descriptor::kernel_code_segment()),
            data: gdt.append(Descriptor::kernel_data_segment()),
            user_code: gdt.append(Descriptor::user_code_segment()),
            user_data: gdt.append(Descriptor::user_data_segment()),
            tss: gdt.append(Descriptor::tss_segment(tss)),
        };
        (gdt, selectors)
    });

    gdt.load();

    // SAFETY: All selectors refer to live entries in the static GDT. The TSS
    // and its double-fault stack also have static storage and are initialized
    // before the task register is loaded.
    unsafe {
        CS::set_reg(selectors.code);
        SS::set_reg(selectors.data);
        DS::set_reg(selectors.data);
        ES::set_reg(selectors.data);
        load_tss(selectors.tss);
    }
}

pub fn user_segments() -> (SegmentSelector, SegmentSelector) {
    let (_, selectors) = GDT
        .get()
        .expect("GDT must be initialized before user mode is configured");
    (selectors.user_code, selectors.user_data)
}
