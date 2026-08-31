use core::sync::atomic::{AtomicBool, Ordering};

use spin::Once;

use x86_64::{
    registers::control::Cr2,
    structures::idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode},
};

use crate::{gdt, keyboard, pic, serial, syscall, task, timer};

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
        idt[0x80].set_handler_fn(syscall_handler);
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

fn dump_exception_context(label: &str, stack_frame: &InterruptStackFrame, error_code: Option<u64>) {
    serial::write_fmt(format_args!("\nEXCEPTION: {label}\n"));
    if let Some(code) = error_code {
        serial::write_fmt(format_args!("ERROR CODE: {code:#x}\n"));
    }

    serial::write_fmt(format_args!(
        "RIP: {:#x}\nCS: {:#x}\nRFLAGS: {:#x}\nRSP: {:#x}\nSS: {:#x}\n",
        stack_frame.instruction_pointer.as_u64(),
        stack_frame.code_segment.0,
        stack_frame.cpu_flags,
        stack_frame.stack_pointer.as_u64(),
        stack_frame.stack_segment.0,
    ));
}

extern "x86-interrupt" fn divide_error_handler(stack_frame: InterruptStackFrame) {
    dump_exception_context("DIVIDE ERROR", &stack_frame, None);
    halt();
}

extern "x86-interrupt" fn invalid_opcode_handler(stack_frame: InterruptStackFrame) {
    dump_exception_context("INVALID OPCODE", &stack_frame, None);
    halt();
}

extern "x86-interrupt" fn double_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: u64,
) -> ! {
    dump_exception_context("DOUBLE FAULT", &stack_frame, Some(error_code));
    halt();
}

extern "x86-interrupt" fn general_protection_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: u64,
) {
    dump_exception_context("GENERAL PROTECTION FAULT", &stack_frame, Some(error_code));
    halt();
}

extern "x86-interrupt" fn page_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: PageFaultErrorCode,
) {
    let address = Cr2::read();
    dump_exception_context("PAGE FAULT", &stack_frame, Some(error_code.bits()));
    if let Ok(addr) = address {
        serial::write_fmt(format_args!("FAULT ADDRESS: {:#x}\n", addr.as_u64()));
    } else {
        serial::write_fmt(format_args!("FAULT ADDRESS: UNAVAILABLE\n"));
    }
    serial::write_fmt(format_args!("PAGE FAULT DETAILS: {error_code:?}\n"));
    halt();
}

extern "x86-interrupt" fn timer_interrupt_handler(_stack_frame: InterruptStackFrame) {
    timer::record_tick();
    task::tick();
    pic::notify_end_of_interrupt(timer::IRQ);
}

extern "x86-interrupt" fn syscall_handler(_stack_frame: InterruptStackFrame) {
    syscall::handle_interrupt();
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
