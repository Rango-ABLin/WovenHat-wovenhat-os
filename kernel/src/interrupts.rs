use core::sync::atomic::{AtomicBool, Ordering};

use spin::Once;

use x86_64::{
    registers::control::Cr2,
    structures::idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode},
};

use crate::serial;

static BREAKPOINT_REACHED: AtomicBool = AtomicBool::new(false);

static IDT: Once<InterruptDescriptorTable> = Once::new();

pub fn init() {
    let idt = IDT.call_once(|| {
        let mut idt = InterruptDescriptorTable::new();

        idt.breakpoint.set_handler_fn(breakpoint_handler);
        idt.divide_error.set_handler_fn(divide_error_handler);
        idt.invalid_opcode.set_handler_fn(invalid_opcode_handler);
        idt.general_protection_fault
            .set_handler_fn(general_protection_fault_handler);
        idt.page_fault.set_handler_fn(page_fault_handler);

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

fn halt() -> ! {
    loop {
        x86_64::instructions::hlt();
    }
}
