use core::sync::atomic::{AtomicBool, Ordering};

use spin::Once;

use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame};

static BREAKPOINT_REACHED: AtomicBool = AtomicBool::new(false);

static IDT: Once<InterruptDescriptorTable> = Once::new();

pub fn init() {
    let idt = IDT.call_once(|| {
        let mut idt = InterruptDescriptorTable::new();

        idt.breakpoint.set_handler_fn(breakpoint_handler);

        idt
    });

    idt.load();
}

pub fn breakpoint_reached() -> bool {
    BREAKPOINT_REACHED.load(Ordering::SeqCst)
}

extern "x86-interrupt" fn breakpoint_handler(_stack_frame: InterruptStackFrame) {
    BREAKPOINT_REACHED.store(true, Ordering::SeqCst);
}
