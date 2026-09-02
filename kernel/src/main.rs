#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]
#![feature(alloc_error_handler)]

extern crate alloc;

mod capability;
mod console;
mod elf;
mod gdt;
mod graphics;
mod gui;
mod hal;
mod heap;
mod interrupts;
mod keyboard;
mod memory;
mod paging;
mod panic;
mod pic;
mod serial;
mod shell;
mod syscall;
mod task;
mod timer;
mod userspace;
mod vfs;

use bootloader_api::{config::Mapping, entry_point, info::Optional, BootInfo, BootloaderConfig};

use console::Console;
use core::{alloc::Layout, panic::PanicInfo};
use keyboard::Keyboard;

use x86_64::instructions::interrupts::int3;

static BOOTLOADER_CONFIG: BootloaderConfig = {
    let mut config = BootloaderConfig::new_default();
    config.mappings.physical_memory = Some(Mapping::Dynamic);
    config
};

entry_point!(kernel_main, config = &BOOTLOADER_CONFIG);

fn kernel_main(boot_info: &'static mut BootInfo) -> ! {
    let memory_init = memory::init(&boot_info.memory_regions);
    let paging_init = match &boot_info.physical_memory_offset {
        Optional::Some(offset) => paging::init(*offset),
        Optional::None => Err(paging::InitError::MissingPhysicalMemoryMapping),
    };
    let boot_info_address = boot_info as *const BootInfo as u64;

    let framebuffer = match &mut boot_info.framebuffer {
        Optional::Some(framebuffer) => framebuffer,
        Optional::None => halt(),
    };

    let info = framebuffer.info();
    let buffer = framebuffer.buffer_mut();
    let framebuffer_address = buffer.as_ptr() as u64;
    let stack_probe = &info as *const _ as u64;

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

    if paging_init.is_err() {
        console.println("PAGING: INITIALIZATION FAILED");
        halt();
    }

    let translation_probes = [
        kernel_main as *const () as u64,
        boot_info_address,
        framebuffer_address,
        stack_probe,
    ];
    if paging::self_test(&translation_probes) {
        console.println("PAGING TRANSLATION: 4/4 OK");
    } else {
        console.println("PAGING TRANSLATION: FAILED");
        halt();
    }

    if paging::mapping_self_test() {
        console.println("PAGING MAP/WRITE/UNMAP: OK");
    } else {
        console.println("PAGING MAP/WRITE/UNMAP: FAILED");
        halt();
    }

    serial::init();

    let hardware = hal::init();
    let vendor = match hardware.cpu_vendor {
        hal::CpuVendor::Intel => "INTEL",
        hal::CpuVendor::Amd => "AMD",
        hal::CpuVendor::Unknown => "UNKNOWN",
    };
    serial::write_fmt(format_args!(
        "HARDWARE: CPU={} LOGICAL_CPUS={} TSC={} RDRAND={} AES_NI={} AVX={} PAE={} SSE4.2={}\n",
        vendor,
        hardware.logical_cpus,
        hardware.cpu_features.has_tsc as u8,
        hardware.cpu_features.has_rdrand as u8,
        hardware.cpu_features.has_aes_ni as u8,
        hardware.cpu_features.has_avx as u8,
        hardware.cpu_features.has_pae as u8,
        hardware.cpu_features.has_sse4_2 as u8,
    ));

    if heap::init().is_err() {
        console.println("KERNEL HEAP: INITIALIZATION FAILED");
        halt();
    }

    if heap::self_test() {
        console.println("KERNEL HEAP: 256 KIB OK");
    } else {
        console.println("KERNEL HEAP: SELF TEST FAILED");
        halt();
    }

    if vfs::self_test() {
        console.println("VFS READ/WRITE: OK");
    } else {
        console.println("VFS READ/WRITE: FAILED");
        halt();
    }

    if gui::self_test() {
        console.println("GUI INPUT: OK");
    } else {
        console.println("GUI INPUT: FAILED");
        halt();
    }

    gdt::init();
    let _user_segments = gdt::user_segments();
    console.println("GDT/TSS: INSTALLED");
    console.println("USER MODE SEGMENTS: READY");

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
    let mut desktop = gui::Desktop::new(graphics::Color::DARK_BLUE);
    let mut window = gui::Window::new(gui::Rect::new(80, 80, 480, 280), "WOVENHAT DESKTOP");
    window.add_button(gui::Button::new(
        gui::Rect::new(120, 180, 180, 48),
        "ACTIVATE",
        graphics::Color::CYAN,
    ));
    window.add_button(gui::Button::new(
        gui::Rect::new(320, 180, 180, 48),
        "SECOND",
        graphics::Color::CYAN,
    ));
    desktop.add_window(window);
    console.render_desktop(&desktop);

    //
    // Interrupt-driven keyboard input. The IRQ handler only queues raw
    // scancodes; decoding and rendering remain in the main loop.
    //

    let mut keyboard = Keyboard::new();

    loop {
        if let Some(key) = keyboard.poll() {
            let event = match key {
                keyboard::Key::Char(character) => gui::InputEvent::Key(character),
                keyboard::Key::Enter => gui::InputEvent::Key('\n'),
                keyboard::Key::Backspace => gui::InputEvent::Key('\u{8}'),
                keyboard::Key::Tab => gui::InputEvent::Key('\t'),
            };
            desktop.handle(&event);
            console.render_desktop(&desktop);
        }

        syscall::service_pending();
        task::preemption_point();
        x86_64::instructions::hlt();
    }
}

fn halt() -> ! {
    x86_64::instructions::interrupts::disable();
    loop {
        x86_64::instructions::hlt();
    }
}

#[alloc_error_handler]
fn alloc_error_handler(layout: Layout) -> ! {
    serial::write_fmt(format_args!(
        "\nKERNEL ALLOC ERROR: layout size={} align={}\n",
        layout.size(),
        layout.align()
    ));
    halt()
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    panic::kernel_panic(info)
}
