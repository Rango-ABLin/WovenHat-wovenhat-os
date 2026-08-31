use core::sync::atomic::{AtomicBool, Ordering};

use spin::Once;

use x86_64::{
    registers::control::Cr2,
    structures::idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode},
};

use crate::{gdt, keyboard, pic, serial, timer};

static BREAKPOINT_REACHED: AtomicBool = AtomicBool::new(false);

static IDT: Once<InterruptDescriptorTable> = Once::new();

pub fn init() {
    let idt = IDT.call_once(|| {
        let mut idt = InterruptDescriptorTable::new();

        idt.breakpoint.set_handler_fn(breakpoint_handler);
        idt.divide_error.set_handler_fn(divide_error_handler);
        idt.invalid_opcode.set_handler_fn(invalid_opcode_handler);
        // SAFETY: The selected IST entry is initialized with a dedicated,
        // statically allocated stack by `gdt::init` before this IDT is loaded.
        unsafe {
            idt.double_fault
                .set_handler_fn(double_fault_handler)
                .set_stack_index(gdt::DOUBLE_FAULT_IST_INDEX);
        }
        idt.general_protection_fault
            .set_handler_fn(general_protection_fault_handler);
        idt.page_fault.set_handler_fn(page_fault_handler);
        idt[pic::MASTER_OFFSET + timer::IRQ].set_handler_fn(timer_interrupt_handler);
        idt[pic::MASTER_OFFSET + keyboard::IRQ].set_handler_fn(keyboard_interrupt_handler);

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

extern "x86-interrupt" fn divide_error_handler(stack_frame: InterruptStackFrame) {
    serial::write_fmt(format_args!(
        "\nEXCEPTION: DIVIDE ERROR\n{stack_frame:#?}\n"
    ));
    halt();
}

extern "x86-interrupt" fn invalid_opcode_handler(stack_frame: InterruptStackFrame) {
    serial::write_fmt(format_args!(
        "\nEXCEPTION: INVALID OPCODE\n{stack_frame:#?}\n"
    ));
    halt();
}

extern "x86-interrupt" fn double_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: u64,
) -> ! {
    serial::write_fmt(format_args!(
        "\nEXCEPTION: DOUBLE FAULT\nERROR CODE: {error_code:#x}\n{stack_frame:#?}\n"
    ));
    halt();
}

extern "x86-interrupt" fn general_protection_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: u64,
) {
    serial::write_fmt(format_args!(
        "\nEXCEPTION: GENERAL PROTECTION FAULT\nERROR CODE: {error_code:#x}\n{stack_frame:#?}\n"
    ));
    halt();
}

extern "x86-interrupt" fn page_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: PageFaultErrorCode,
) {
    serial::write_fmt(format_args!(
        "\nEXCEPTION: PAGE FAULT\nADDRESS: {:?}\nERROR CODE: {error_code:?}\n{stack_frame:#?}\n",
        Cr2::read()
    ));
    halt();
}

extern "x86-interrupt" fn timer_interrupt_handler(_stack_frame: InterruptStackFrame) {
    timer::record_tick();
    pic::notify_end_of_interrupt(timer::IRQ);
}

extern "x86-interrupt" fn keyboard_interrupt_handler(_stack_frame: InterruptStackFrame) {
    keyboard::handle_interrupt();
    pic::notify_end_of_interrupt(keyboard::IRQ);
}

fn halt() -> ! {
    loop {
        x86_64::instructions::hlt();
    }
}
