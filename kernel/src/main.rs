#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]
#![feature(alloc_error_handler)]

extern crate alloc;

mod ata;
mod block;
mod capability;
mod console;
mod device;
mod elf;
mod fat32;
mod gdt;
mod graphics;
mod gui;
mod hal;
mod heap;
mod interrupts;
mod ipc;
mod keyboard;
mod memory;
mod paging;
mod panic;
mod pic;
mod serial;
mod shell;
mod storage;
mod syscall;
mod task;
mod timer;
mod userspace;
mod vfs;

use bootloader_api::{config::Mapping, entry_point, info::Optional, BootInfo, BootloaderConfig};

use console::Console;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use core::{alloc::Layout, panic::PanicInfo};
use keyboard::Keyboard;
use shell::Shell;

use x86_64::instructions::interrupts::int3;

static BOOTLOADER_CONFIG: BootloaderConfig = {
    let mut config = BootloaderConfig::new_default();
    config.mappings.physical_memory = Some(Mapping::Dynamic);
    config
};

entry_point!(kernel_main, config = &BOOTLOADER_CONFIG);

static PREEMPTION_PROBE_BLOCKED: AtomicBool = AtomicBool::new(false);
static PREEMPTION_PROBE_COMPLETED: AtomicBool = AtomicBool::new(false);
static FAIR_TASK_A_RUNS: AtomicU64 = AtomicU64::new(0);
static FAIR_TASK_B_RUNS: AtomicU64 = AtomicU64::new(0);
static FAIR_TASKS_COMPLETED: AtomicU64 = AtomicU64::new(0);

fn kernel_main(boot_info: &'static mut BootInfo) -> ! {
    let memory_init = memory::init(&boot_info.memory_regions);
    let physical_memory_offset = match &boot_info.physical_memory_offset {
        Optional::Some(offset) => Some(*offset),
        Optional::None => None,
    };
    let paging_init = match physical_memory_offset {
        Some(offset) => paging::init(offset),
        None => Err(paging::InitError::MissingPhysicalMemoryMapping),
    };
    let rsdp_address = match &boot_info.rsdp_addr {
        Optional::Some(address) => Some(*address),
        Optional::None => None,
    };
    let acpi = physical_memory_offset
        .ok_or(hal::acpi::Error::OutOfRange)
        .and_then(|offset| hal::acpi::discover(offset, rsdp_address, &boot_info.memory_regions));
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
    if !hal::acpi::self_test() {
        console.println("ACPI PARSER: VALIDATION FAILED");
        halt();
    }
    match acpi {
        Ok(summary) => {
            console.println("ACPI TABLES: VALIDATED");
            serial::write_line(format_args!(
                "[ACPI] revision={} tables={} APIC={} FADT={} HPET={} MCFG={} truncated={}",
                summary.revision,
                summary.tables,
                summary.apic as u8,
                summary.fadt as u8,
                summary.hpet as u8,
                summary.mcfg as u8,
                summary.truncated as u8,
            ));
        }
        Err(_) => console.println("ACPI TABLES: UNAVAILABLE"),
    }
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
    if !hal::pci::self_test() {
        console.println("PCI DISCOVERY: VALIDATION FAILED");
        halt();
    }
    serial::write_line(format_args!(
        "[PCI] devices={} recorded={} storage={} network={} display={} bridges={} truncated={}",
        hardware.pci.discovered,
        hardware.pci.recorded,
        hardware.pci.storage,
        hardware.pci.network,
        hardware.pci.display,
        hardware.pci.bridges,
        hardware.pci.truncated as u8,
    ));
    console.println("PCI CONFIGURATION: ENUMERATED");

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
    if block::self_test() {
        console.println("BLOCK DEVICE I/O: OK");
    } else {
        console.println("BLOCK DEVICE I/O: FAILED");
        halt();
    }

    if ata::self_test() {
        console.println("ATA IDENTIFY PARSER: OK");
    } else {
        console.println("ATA IDENTIFY PARSER: FAILED");
        halt();
    }

    if fat32::self_test() {
        console.println("FAT32 CHAIN READS: OK");
    } else {
        console.println("FAT32 VALIDATION: FAILED");
        halt();
    }

    if vfs::self_test() {
        console.println("VFS READ/WRITE: OK");
    } else {
        console.println("VFS READ/WRITE: FAILED");
        halt();
    }
    if storage::self_test() {
        console.println("STORAGE MOUNT PATHS: OK");
    } else {
        console.println("STORAGE MOUNT PATHS: FAILED");
        halt();
    }
    if userspace::elf_loader_self_test() {
        console.println("ELF64 W^X + STACK GUARD: OK");
    } else {
        console.println("ELF64 LOADER VALIDATION: FAILED");
        halt();
    }

    if gui::self_test() {
        console.println("GUI INPUT: OK");
    } else {
        console.println("GUI INPUT: FAILED");
        halt();
    }
    if ipc::self_test() && ipc::endpoint_count() == 0 {
        console.println("IPC QUEUES: VALIDATED");
    } else {
        console.println("IPC QUEUES: VALIDATION FAILED");
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
    if syscall::test() {
        console.println("SYSCALL GATE: GETPID OK");
    } else {
        console.println("SYSCALL GATE: FAILED");
        halt();
    }

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
    let ata_sectors = ata::init();
    if ata_sectors.is_some()
        && !ata::with_primary_master(|disk| {
            let mut sector = [0_u8; block::SECTOR_SIZE];
            block::BlockDevice::read_sector(disk, 0, &mut sector).is_ok()
        })
        .unwrap_or(false)
    {
        console.println("ATA LBA0 READ: FAILED");
        halt();
    }
    let storage_status = storage::mount_ata_root();
    match storage_status {
        storage::MountStatus::Mounted(files) => {
            console.println("FAT32 ROOT MOUNTED READ-ONLY");
            serial::write_line(format_args!("[VFS] mounted {} FAT32 root files", files));
        }
        storage::MountStatus::NoDevice => console.println("FAT32 MOUNT: NO BLOCK DEVICE"),
        storage::MountStatus::NotFat32 => console.println("FAT32 MOUNT: NO VOLUME"),
        storage::MountStatus::Failed => console.println("FAT32 MOUNT: FAILED"),
    }
    let vfs_nodes_before_userspace = vfs::node_count();
    let boot_devices = [
        device::Device {
            name: "framebuffer-console",
            kind: device::DeviceKind::Console,
            irq: None,
        },
        device::Device {
            name: "com1",
            kind: device::DeviceKind::Serial,
            irq: None,
        },
        device::Device {
            name: "pit",
            kind: device::DeviceKind::Timer,
            irq: Some(timer::IRQ),
        },
        device::Device {
            name: "ps2-keyboard",
            kind: device::DeviceKind::Keyboard,
            irq: Some(keyboard::IRQ),
        },
    ];
    for device in boot_devices {
        if device::register(device).is_err() {
            console.println("DEVICE REGISTRATION: FAILED");
            halt();
        }
    }
    if ata_sectors.is_some()
        && device::register(device::Device {
            name: "ata0",
            kind: device::DeviceKind::Block,
            irq: None,
        })
        .is_err()
    {
        console.println("ATA DEVICE REGISTRATION: FAILED");
        halt();
    }
    if device::self_test(ata_sectors.is_some()) {
        if let Some(sectors) = ata_sectors {
            console.println("DEVICE REGISTRY: 5 DEVICES ONLINE");
            serial::write_line(format_args!("[ATA] primary master: {} sectors", sectors));
        } else {
            console.println("DEVICE REGISTRY: 4 DEVICES ONLINE (NO ATA)");
        }
    } else {
        console.println("DEVICE REGISTRY: VALIDATION FAILED");
        halt();
    }
    pic::unmask(timer::IRQ);
    pic::unmask(keyboard::IRQ);
    x86_64::instructions::interrupts::enable();

    while timer::ticks() < 3 {
        task::yield_now();
    }

    let probe_id = match task::spawn("preemption-probe", preemption_probe_task) {
        Ok(id) => id,
        Err(_) => {
            console.println("PREEMPTION TEST: SPAWN FAILED");
            halt();
        }
    };

    while !PREEMPTION_PROBE_BLOCKED.load(Ordering::Acquire) {
        x86_64::instructions::hlt();
    }

    if !task::wake_task(probe_id) {
        console.println("TASK WAKEUP: FAILED");
        halt();
    }

    while !PREEMPTION_PROBE_COMPLETED.load(Ordering::Acquire) {
        x86_64::instructions::hlt();
    }

    console.println("TIMER IRQ: OK");
    console.println("TIMER PREEMPTION: OK");
    console.println("TASK SLEEP/BLOCK/WAKE: OK");
    serial::write_line(format_args!(
        "[BOOT] timer preemption and task lifecycle verified"
    ));

    let preemptions_before_fairness = task::summary().preemption_switches;
    if task::spawn("fair-peer-a", fairness_probe_a).is_err() {
        console.println("SCHEDULER FAIRNESS A: SPAWN FAILED");
        serial::write_line(format_args!("[SCHED] peer A spawn failed"));
        halt();
    }
    if task::spawn("fair-peer-b", fairness_probe_b).is_err() {
        console.println("SCHEDULER FAIRNESS B: SPAWN FAILED");
        serial::write_line(format_args!("[SCHED] peer B spawn failed"));
        halt();
    }
    while FAIR_TASKS_COMPLETED.load(Ordering::Acquire) != 2 {
        x86_64::instructions::hlt();
    }
    let fairness_summary = task::summary();
    if FAIR_TASK_A_RUNS.load(Ordering::Acquire) == 0
        || FAIR_TASK_B_RUNS.load(Ordering::Acquire) == 0
        || fairness_summary.preemption_switches < preemptions_before_fairness + 2
    {
        console.println("SCHEDULER FAIRNESS/QUANTUM: FAILED");
        halt();
    }
    console.println("PRIORITY ROUND-ROBIN/TIME SLICES: OK");
    serial::write_line(format_args!(
        "[BOOT] priority round-robin fairness and per-task quanta verified"
    ));

    console.println("TIMER IRQ: OK");

    let isolation_baseline = memory::stats().allocated_frames;
    let Some(first_program) = userspace::create_stub_process() else {
        console.println("USER PROCESS IMAGE: MAPPING FAILED");
        halt();
    };
    let Some(second_program) = userspace::create_stub_process() else {
        console.println("SECOND USER ADDRESS SPACE: FAILED");
        halt();
    };
    if !first_program.stack.is_aligned()
        || first_program.stack.size != userspace::UserStack::SIZE
        || !paging::user_range_is_unmapped_in(
            first_program.address_space.paging(),
            first_program.stack.guard_base,
            userspace::UserStack::GUARD_SIZE,
        )
        || !paging::user_range_has_protection_in(
            first_program.address_space.paging(),
            first_program.stack.base,
            first_program.stack.size,
            true,
            false,
        )
        || !paging::user_range_is_unmapped_in(
            second_program.address_space.paging(),
            second_program.stack.guard_base,
            userspace::UserStack::GUARD_SIZE,
        )
        || first_program.image.entry != second_program.image.entry
        || first_program.stack.top != second_program.stack.top
        || first_program.address_space.root_address() == second_program.address_space.root_address()
    {
        console.println("USER ADDRESS-SPACE ISOLATION: INVALID");
        halt();
    }
    serial::write_line(format_args!(
        "[BOOT] independent user roots, unmapped stack guards, and RW/NX stacks verified"
    ));

    let first_root = first_program.address_space.root_address();
    let second_root = second_program.address_space.root_address();
    let first_pid = match task::spawn_user_process("init-user-a", first_program) {
        Ok((pid, context)) => {
            serial::write_line(format_args!(
                "[BOOT] ring3 frame CS={:#x} SS={:#x} RIP={:#x} RSP={:#x}",
                context.code_segment, context.data_segment, context.entry, context.stack_top,
            ));
            pid
        }
        Err(_) => {
            console.println("FIRST USER PROCESS: SPAWN FAILED");
            halt();
        }
    };
    let second_pid = match task::spawn_user_process("init-user-b", second_program) {
        Ok((pid, _)) => pid,
        Err(_) => {
            console.println("SECOND USER PROCESS: SPAWN FAILED");
            halt();
        }
    };

    while !task::process_exited(first_pid) || !task::process_exited(second_pid) {
        x86_64::instructions::hlt();
    }

    if !syscall::user_memory_verified() || task::anonymous_mapping_count() != 0 {
        console.println("USER MMAP/MUNMAP: FAILED");
        halt();
    }
    serial::write_line(format_args!(
        "[BOOT] user mmap/write/read/munmap and frame return verified"
    ));

    if !syscall::user_io_verified()
        || task::open_file_count() != 0
        || vfs::node_count() != vfs_nodes_before_userspace
    {
        console.println("USER VFS/DESCRIPTOR SYSCALLS: FAILED");
        halt();
    }
    serial::write_line(format_args!(
        "[BOOT] user write/open/read/close and pointer validation verified"
    ));

    if !syscall::last_completed(syscall::Number::Getpid) {
        console.println("USER SYSCALL ABI: FAILED");
        halt();
    }
    if task::zombie_count() != 2 {
        console.println("PROCESS ZOMBIE RETENTION: FAILED");
        halt();
    }
    let first_status = syscall::invoke(syscall::Number::Waitpid, first_pid.as_u64(), 0, 0);
    let second_status = syscall::invoke(syscall::Number::Waitpid, second_pid.as_u64(), 0, 0);
    if first_status != 0 || second_status != 0 || task::zombie_count() != 0 {
        console.println("PROCESS WAIT/REAP: FAILED");
        halt();
    }
    if syscall::invoke(syscall::Number::Waitpid, first_pid.as_u64(), 0, 0) != u64::MAX {
        console.println("PROCESS DOUBLE-WAIT REJECTION: FAILED");
        halt();
    }
    serial::write_line(format_args!(
        "[BOOT] parent-child waitpid and zombie reaping verified"
    ));
    if memory::stats().allocated_frames != isolation_baseline || ipc::endpoint_count() != 0 {
        console.println("USER ADDRESS-SPACE RECLAMATION: FAILED");
        halt();
    }

    console.println("USER CR3 ISOLATION/W^X/RECLAMATION: OK");
    serial::write_line(format_args!(
        "[BOOT] isolated user CR3 roots {:#x} and {:#x} verified",
        first_root, second_root,
    ));
    serial::write_line(format_args!(
        "[BOOT] ring3 W^X mapping cleanup and frame reuse verified"
    ));
    console.println("RING3 PROCESS GETPID/EXIT: OK");
    serial::write_line(format_args!(
        "[BOOT] ring3 process and syscall ABI verified"
    ));
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
    let mut shell = Shell::new();
    let mut desktop_active = true;

    //
    // F1 switches between the desktop and diagnostic shell. Keyboard IRQ
    // decoding stays outside both interfaces.
    //
    // Interrupt-driven keyboard input. The IRQ handler only queues raw
    // scancodes; decoding and rendering remain in the main loop.
    //

    let mut keyboard = Keyboard::new();

    loop {
        if let Some(key) = keyboard.poll() {
            if matches!(key, keyboard::Key::F1) {
                desktop_active = !desktop_active;
                if desktop_active {
                    console.render_desktop(&desktop);
                } else {
                    console.clear();
                    console.println("WOVENHAT DIAGNOSTIC SHELL (F1 TO RETURN)");
                    shell.print_prompt(&mut console);
                }
            } else if desktop_active {
                let event = match key {
                    keyboard::Key::Char(character) => gui::InputEvent::Key(character),
                    keyboard::Key::Enter => gui::InputEvent::Key('\n'),
                    keyboard::Key::Backspace => gui::InputEvent::Key('\u{8}'),
                    keyboard::Key::Tab => gui::InputEvent::Key('\t'),
                    keyboard::Key::F1 => unreachable!(),
                };
                desktop.handle(&event);
                console.render_desktop(&desktop);
            } else {
                shell.handle_key(key, &mut console);
            }
        }

        syscall::service_pending();
        task::preemption_point();
        x86_64::instructions::hlt();
    }
}

fn fairness_probe_a() -> ! {
    while FAIR_TASK_B_RUNS.load(Ordering::Acquire) == 0 {
        FAIR_TASK_A_RUNS.fetch_add(1, Ordering::Relaxed);
        core::hint::spin_loop();
    }
    FAIR_TASK_A_RUNS.fetch_add(1, Ordering::Release);
    FAIR_TASKS_COMPLETED.fetch_add(1, Ordering::Release);
    task::exit_current_task()
}

fn fairness_probe_b() -> ! {
    while FAIR_TASK_A_RUNS.load(Ordering::Acquire) == 0 {
        FAIR_TASK_B_RUNS.fetch_add(1, Ordering::Relaxed);
        core::hint::spin_loop();
    }
    FAIR_TASK_B_RUNS.fetch_add(1, Ordering::Release);
    FAIR_TASKS_COMPLETED.fetch_add(1, Ordering::Release);
    task::exit_current_task()
}
fn preemption_probe_task() -> ! {
    task::sleep_current(2);
    PREEMPTION_PROBE_BLOCKED.store(true, Ordering::Release);
    task::block_current();
    PREEMPTION_PROBE_COMPLETED.store(true, Ordering::Release);
    task::exit_current_task()
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
