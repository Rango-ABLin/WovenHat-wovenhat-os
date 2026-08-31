#![no_std]
#![no_main]

mod console;
mod keyboard;

use bootloader_api::{
    entry_point,
    info::Optional,
    BootInfo,
};

use console::Console;
use core::panic::PanicInfo;
use keyboard::{Key, Keyboard};

entry_point!(kernel_main);

fn kernel_main(boot_info: &'static mut BootInfo) -> ! {
    let framebuffer = match &mut boot_info.framebuffer {
        Optional::Some(framebuffer) => framebuffer,
        Optional::None => halt(),
    };

    let info = framebuffer.info();
    let buffer = framebuffer.buffer_mut();

    let mut console = Console::new(
        buffer,
        info,
        40,
        40,
        2,
    );

    console.clear();

    console.println("WOVENHAT OS");
    console.println("SECURE INTELLIGENCE PLATFORM");
    console.println("");
    console.println("WOVENHAT KERNEL 0.0.1");
    console.println("ARCHITECTURE: X86_64");
    console.println("KERNEL BOOT SUCCESSFUL.");
    console.println("");

    console.print("WOVENHAT> ");

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