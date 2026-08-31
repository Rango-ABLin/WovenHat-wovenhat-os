#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]

mod console;
mod interrupts;
mod keyboard;

use bootloader_api::{BootInfo, entry_point, info::Optional};

use console::Console;
use core::panic::PanicInfo;
use keyboard::{Key, Keyboard};

use x86_64::instructions::interrupts::int3;

entry_point!(kernel_main);

fn kernel_main(boot_info: &'static mut BootInfo) -> ! {
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

    console.println("WOVENHAT KERNEL 0.0.3");
    console.println("ARCHITECTURE: X86_64");
    console.println("KERNEL BOOT SUCCESSFUL.");
    console.println("");

    //
    // Interrupt Descriptor Table
    //

    interrupts::init();

    console.println("IDT: INSTALLED");

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
    console.print("WOVENHAT> ");

    //
    // Temporary polling keyboard.
    //
    // We will replace this after IRQ support is working.
    //

    let mut keyboard = Keyboard::new();

    loop {
        if let Some(key) = keyboard.poll() {
            match key {
                Key::Char(character) => {
                    console.put_char(character);
                }

                Key::Backspace => {
                    console.backspace();
                }

                Key::Enter => {
                    console.newline();
                    console.print("WOVENHAT> ");
                }
            }
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
