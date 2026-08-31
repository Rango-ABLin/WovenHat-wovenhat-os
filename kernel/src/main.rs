#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]

mod capability;
mod console;
mod gdt;
mod interrupts;
mod keyboard;
mod memory;
mod pic;
mod serial;
mod shell;
mod task;
mod timer;

use bootloader_api::{BootInfo, entry_point, info::Optional};

use console::Console;
use core::panic::PanicInfo;
use keyboard::Keyboard;
use shell::Shell;

use x86_64::instructions::interrupts::int3;

entry_point!(kernel_main);

fn kernel_main(boot_info: &'static mut BootInfo) -> ! {
    let memory_init = memory::init(&boot_info.memory_regions);

    let framebuffer = match &mut boot_info.framebuffer {
        Optional::Some(framebuffer) => framebuffer,
        Optional::None => halt(),
    };

    let info = framebuffer.info();
    let buffer = framebuffer.buffer_mut();

    let mut console = Console::new(buffer, info, 40, 40, 2);

    console.clear();

    console.println("WOVENHAT OS");
    console.println("SECURE INTELLIGENCE PLATFORM");
    console.println("");

    console.println("WOVENHAT KERNEL 0.0.7");
    console.println("ARCHITECTURE: X86_64");
    console.println("KERNEL BOOT SUCCESSFUL.");
    console.println("");

    if memory_init.is_err() {
        console.println("FRAME ALLOCATOR: INITIALIZATION FAILED");
        halt();
    }

    if memory::self_test() {
        console.println("FRAME ALLOCATOR: OK");
    } else {
        console.println("FRAME ALLOCATOR: SELF TEST FAILED");
        halt();
    }

    serial::init();
    gdt::init();
    console.println("GDT/TSS: INSTALLED");

    //
    // Interrupt Descriptor Table
    //

    interrupts::init();

    console.println("IDT: INSTALLED");

    task::init();
    console.println("SCHEDULER: INITIALIZED");

    if task::capability_policy_valid() {
        console.println("CAPABILITY POLICY: ONLINE");
    } else {
        console.println("CAPABILITY POLICY: FAILED");
        halt();
    }

    if task::capability_delegation_valid() {
        console.println("CAPABILITY DELEGATION: OK");
    } else {
        console.println("CAPABILITY DELEGATION: FAILED");
        halt();
    }

    pic::init();
    console.println("PIC: INITIALIZED (ALL IRQS MASKED)");

    timer::init();
    pic::unmask(timer::IRQ);
    pic::unmask(keyboard::IRQ);
    x86_64::instructions::interrupts::enable();

    while timer::ticks() < 3 {
        task::yield_now();
    }

    console.println("TIMER IRQ: OK");
    console.println("KEYBOARD IRQ: READY");

    //
    // Breakpoint exception test
    //

    console.println("TESTING BREAKPOINT INTERRUPT...");

    int3();

    if interrupts::breakpoint_reached() {
        console.println("BREAKPOINT HANDLER: OK");
        console.println("INTERRUPT SYSTEM: ONLINE");
    } else {
        console.println("BREAKPOINT HANDLER: FAILED");
    }

    console.println("");
    let mut shell = Shell::new();
    shell.print_prompt(&mut console);

    //
    // Interrupt-driven keyboard input. The IRQ handler only queues raw
    // scancodes; decoding and rendering remain in the main loop.
    //

    let mut keyboard = Keyboard::new();

    loop {
        if let Some(key) = keyboard.poll() {
            shell.handle_key(key, &mut console);
        }

        core::hint::spin_loop();
    }
}

fn halt() -> ! {
    loop {
        core::hint::spin_loop();
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    halt()
}
