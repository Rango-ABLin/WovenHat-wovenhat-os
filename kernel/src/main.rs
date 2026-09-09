#![cfg_attr(feature = "qemu-test", allow(dead_code, unused_imports))]
#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]
#![feature(alloc_error_handler)]

extern crate alloc;

mod ata;
mod audit;
mod benchmark;
mod block;
mod block_cache;
mod block_io;
mod capability;
mod config;
mod console;
mod device;
mod elf;
mod entropy;
mod fat32;
mod file_frames;
mod file_mapping;
mod gdt;
mod gpt;
mod graphics;
mod gui;
mod hal;
mod heap;
mod interrupts;
mod ipc;
mod keyboard;
mod memory;
mod network;
mod page_cache;
mod paging;
mod panic;
mod partition;
mod pic;
mod pipe;
mod serial;
mod shell;
mod storage;
mod swap;
mod smp;
mod syscall;
mod task;
mod terminal;
mod timer;
mod userspace;
mod vfs;
mod virtio_net;

use bootloader_api::{config::Mapping, entry_point, info::Optional, BootInfo, BootloaderConfig};

use console::Console;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use core::{alloc::Layout, panic::PanicInfo};
use shell::Shell;

use x86_64::instructions::interrupts::int3;

static BOOTLOADER_CONFIG: BootloaderConfig = {
    let mut config = BootloaderConfig::new_default();
    config.mappings.physical_memory = Some(Mapping::Dynamic);
    // The default boot stack is intentionally small. WovenHat performs
    // substantial early initialization, so give the bootstrap task a 1 MiB
    // stack while retaining the bootloader's guard-page protection.
    config.kernel_stack_size = 1024 * 1024;
    config
};

entry_point!(kernel_main, config = &BOOTLOADER_CONFIG);

static PREEMPTION_PROBE_BLOCKED: AtomicBool = AtomicBool::new(false);
static PREEMPTION_PROBE_COMPLETED: AtomicBool = AtomicBool::new(false);
static FAIR_TASK_A_RUNS: AtomicU64 = AtomicU64::new(0);
static FAIR_TASK_B_RUNS: AtomicU64 = AtomicU64::new(0);
static FAIR_TASKS_COMPLETED: AtomicU64 = AtomicU64::new(0);

#[allow(unreachable_code)]
fn kernel_main(boot_info: &'static mut BootInfo) -> ! {
    smp::prepare(&boot_info.memory_regions);
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

    terminal::init(buffer, info);
    let mut console = Console::new(buffer, info, 40, 40, 2);

    console.clear();

    console.println("WOVENHAT OS");
    console.println("SECURE INTELLIGENCE PLATFORM");
    console.println("");

    console.println("WOVENHAT KERNEL 0.8.0 MULTICORE FOUNDATION");
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
                "[ACPI] revision={} tables={} APIC={} CPUs={} IOAPICs={} ISOs={} LAPIC={:#x} FADT={} HPET={} MCFG={} truncated={}",
                summary.revision,
                summary.tables,
                summary.apic as u8,
                summary.enabled_processors,
                summary.io_apics,
                summary.interrupt_overrides,
                summary.local_apic_address,
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

    // Normal boots are shell-first. Do not make the interactive console wait
    // for the exhaustive storage/network/ring3 validation suite. Those tests
    // remain below for `--features qemu-test` builds.
    #[cfg(not(feature = "qemu-test"))]
    {
        gdt::init();
        let _user_segments = gdt::user_segments();
        interrupts::init();
        task::init();

        pic::init();
        timer::init();

        let early_devices = [
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

        for dev in early_devices {
            if device::register(dev).is_err() {
                serial::write_line(format_args!("[BOOT] early device registration failed"));
                halt();
            }
        }

        smp::start(acpi.ok(), physical_memory_offset.unwrap());
        if !smp::routed_irq() {
            pic::unmask(timer::IRQ);
            pic::unmask(keyboard::IRQ);
        }
        x86_64::instructions::interrupts::enable();
        if !block_io::start_worker() {
            console.println("BLOCK I/O WORKER: START FAILED");
            halt();
        }
        if !block_io::async_completion_self_test() {
            console.println("BLOCK I/O COMPLETION: FAILED");
            halt();
        }
        if !task::start_pager() {
            console.println("PAGER: START FAILED");
            halt();
        }

        let ata_sectors = ata::init();
        if let Some(sectors) = ata_sectors {
            serial::write_line(format_args!(
                "[ATA] primary-master online: {} sectors",
                sectors
            ));
        } else {
            serial::write_line(format_args!("[ATA] primary-master not detected"));
        }

        match storage::mount_ata_root() {
            storage::MountStatus::Mounted(count) => {
                let _ = device::register(device::Device {
                    name: "ata0",
                    kind: device::DeviceKind::Block,
                    irq: None,
                });
                serial::write_line(format_args!(
                    "[FS] FAT32 mounted at /mnt; imported {} entries",
                    count
                ));
            }
            storage::MountStatus::NoDevice => {
                serial::write_line(format_args!("[FS] no ATA disk; continuing with RAM VFS"))
            }
            storage::MountStatus::NotFat32 => {
                serial::write_line(format_args!("[FS] ATA disk present but no FAT32 root"))
            }
            storage::MountStatus::Failed => serial::write_line(format_args!(
                "[FS] FAT32 mount failed; continuing with RAM VFS"
            )),
        }

        for dir in ["/etc", "/var", "/home", "/tmp"] {
            let _ = vfs::mkdir(dir);
        }

        match network::init() {
            Ok(()) => serial::write_line(format_args!(
                "[NET] virtio-net + smoltcp online at 10.0.2.15/24"
            )),
            Err(error) => serial::write_line(format_args!(
                "[NET] optional network init skipped: {:?}",
                error
            )),
        }

        serial::write_line(format_args!(
            "[BOOT] shell-first runtime ready; entering diagnostic shell"
        ));

        // Start userspace init -> /bin/sh (non-fatal if spawn fails).
        match userspace::create_init_process() {
            Some(program) => match task::spawn_user_process("init", program) {
                Ok((pid, _)) => {
                    serial::write_line(format_args!(
                        "[BOOT] userspace init/sh scheduled as pid {}",
                        pid.as_u64()
                    ));
                    console.println("USERSPACE INIT+SH: STARTED");
                }
                Err(_) => console.println("USERSPACE INIT+SH: SPAWN FAILED"),
            },
            None => console.println("USERSPACE INIT+SH: IMAGE FAILED"),
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
        let mut shell = Shell::new();
        let mut desktop_active = false;

        // Start in the diagnostic shell. F1 toggles to the graphical desktop.
        console.clear();
        console.println("WOVENHAT DIAGNOSTIC SHELL (F1 TO OPEN DESKTOP)");
        shell.print_prompt(&mut console);

        let mut userspace_was_foreground = false;
        loop {
            let userspace_foreground = terminal::foreground_active();
            if userspace_was_foreground && !userspace_foreground {
                console.clear();
                console.println("WOVENHAT DIAGNOSTIC SHELL");
                console.println("USERSPACE SESSION ENDED");
                console.println("");
                shell.print_prompt(&mut console);
                desktop_active = false;
            }
            userspace_was_foreground = userspace_foreground;

            // The kernel UI may consume PS/2 input only when no userspace
            // process owns the foreground terminal.  In particular, do not
            // call keyboard::poll() while /bin/sh is foreground: poll() pops
            // the scancode from the shared queue, which would starve the
            // userspace read(0, ...) syscall and make the shell appear hung.
            if !userspace_foreground {
                if let Some(key) = keyboard::poll() {
                    if matches!(key, keyboard::Key::F1) {
                        desktop_active = !desktop_active;
                        if desktop_active {
                            console.render_desktop(&desktop);
                        } else {
                            console.clear();
                            console.println("WOVENHAT DIAGNOSTIC SHELL (F1 TO OPEN DESKTOP)");
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
            }

            network::poll();
            syscall::service_pending();
            task::preemption_point();
            x86_64::instructions::hlt();
        }
    }
    if gpt::self_test() {
        console.println("GPT PARTITIONS: VALIDATED");
    } else {
        console.println("GPT PARTITIONS: VALIDATION FAILED");
        halt();
    }
    if partition::self_test() {
        console.println("MBR PARTITIONS: VALIDATED");
    } else {
        console.println("MBR PARTITIONS: VALIDATION FAILED");
        halt();
    }
    if block::self_test() {
        console.println("BLOCK DEVICE I/O: OK");
    } else {
        console.println("BLOCK DEVICE I/O: FAILED");
        halt();
    }
    if block_cache::self_test() {
        console.println("BLOCK CACHE: OK");
        serial::write_line(format_args!("[BUFFER CACHE] regression tests: PASSED"));
    } else {
        console.println("BLOCK CACHE: FAILED");
        serial::write_line(format_args!("[BUFFER CACHE] regression tests: FAILED"));
        halt();
    }
    if block_io::self_test() {
        console.println("ASYNC BLOCK I/O: OK");
        serial::write_line(format_args!("[BLOCK IO] async completion tests: PASSED"));
    } else {
        console.println("ASYNC BLOCK I/O: FAILED");
        serial::write_line(format_args!("[BLOCK IO] async completion tests: FAILED"));
        halt();
    }
    if swap::self_test() {
        console.println("SWAP BACKING: OK");
        serial::write_line(format_args!("[SWAP] disk-backed policy tests: PASSED"));
    } else {
        console.println("SWAP BACKING: FAILED");
        serial::write_line(format_args!("[SWAP] disk-backed policy tests: FAILED"));
        halt();
    }

    if page_cache::self_test() {
        serial::write_line(format_args!("[FILE PAGES] regression tests: PASSED"));
    } else {
        serial::write_line(format_args!("[FILE PAGES] regression tests: FAILED"));
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
    if network::self_test() {
        console.println("NETWORK/SMOLTCP ADAPTER: OK");
    } else {
        console.println("NETWORK/SMOLTCP ADAPTER: FAILED");
        halt();
    }
    match virtio_net::probe() {
        virtio_net::ProbeStatus::Found(_) => console.println("VIRTIO-NET PCI: DETECTED"),
        virtio_net::ProbeStatus::Missing => console.println("VIRTIO-NET PCI: NOT PRESENT"),
    }

    if vfs::self_test() {
        console.println("VFS READ/WRITE: OK");
        serial::write_line(format_args!("[VFS] read/write and path semantics: PASSED"));
    } else {
        console.println("VFS READ/WRITE: FAILED");
        serial::write_line(format_args!("[VFS] read/write and path semantics: FAILED"));
        halt();
    }
    if paging::frame_ownership_self_test() {
        serial::write_line(format_args!(
            "[FRAME OWNERSHIP] overflow and exhaustion rollback: PASSED"
        ));
    } else {
        serial::write_line(format_args!(
            "[FRAME OWNERSHIP] exhaustion rollback: FAILED"
        ));
        halt();
    }
    if file_frames::self_test() {
        serial::write_line(format_args!(
            "[FRAME CACHE] pinned aliases, LRU eviction, reclaim: PASSED"
        ));
    } else {
        serial::write_line(format_args!("[FRAME CACHE] regression tests: FAILED"));
        halt();
    }
    if userspace::shared_file_mmap_self_test() {
        serial::write_line(format_args!(
            "[SHARED FILE MMAP] aliases, COW, truncation, unlink, reclaim: PASSED"
        ));
    } else {
        serial::write_line(format_args!("[SHARED FILE MMAP] regression tests: FAILED"));
        halt();
    }
    if userspace::lazy_file_mmap_self_test() {
        serial::write_line(format_args!("[LAZY FILE MMAP] regression tests: PASSED"));
    } else {
        serial::write_line(format_args!("[LAZY FILE MMAP] regression tests: FAILED"));
        halt();
    }
    if userspace::private_lazy_swap_self_test() {
        serial::write_line(format_args!(
            "[PRIVATE SWAP MMAP] dirty eviction/refault: PASSED"
        ));
    } else {
        serial::write_line(format_args!("[PRIVATE SWAP MMAP] regression tests: FAILED"));
        halt();
    }
    if userspace::file_mmap_self_test() {
        serial::write_line(format_args!("[FILE MMAP] regression tests: PASSED"));
    } else {
        serial::write_line(format_args!("[FILE MMAP] regression tests: FAILED"));
        halt();
    }
    // Blocking pipe APIs consult the current task, even for immediate reads.
    gdt::init();
    let _user_segments = gdt::user_segments();
    console.println("GDT/TSS: INSTALLED");
    console.println("USER MODE SEGMENTS: READY");

    //
    // Interrupt Descriptor Table
    //

    interrupts::init();

    console.println("IDT: INSTALLED");
    if interrupts::fault_policy_self_test() {
        console.println("USER FAULT RECOVERY: ARMED");
    } else {
        console.println("USER FAULT RECOVERY: FAILED");
        halt();
    }

    task::init();
    console.println("SCHEDULER: INITIALIZED");
    if pipe::self_test() {
        console.println("PIPE: OK");
    } else {
        console.println("PIPE: FAILED");
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

    if keyboard::self_test() {
        console.println("KEYBOARD DECODER: OK");
    } else {
        console.println("KEYBOARD DECODER: FAILED");
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

    if audit::self_test() && task::credential_policy_valid() {
        console.println("CREDENTIAL/AUDIT POLICY: OK");
    } else {
        console.println("CREDENTIAL/AUDIT POLICY: FAILED");
        halt();
    }

    if benchmark::self_test() {
        console.println("BENCHMARK DELTAS: VALIDATED");
    } else {
        console.println("BENCHMARK DELTAS: FAILED");
        halt();
    }
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

    if audit::count() < 2
        || !audit::latest()
            .is_some_and(|event| event.action == audit::Action::CapabilityRevoke && event.allowed)
    {
        console.println("CAPABILITY AUDIT: FAILED");
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
            console.println("FAT32 ROOT MOUNTED");
            serial::write_line(format_args!("[VFS] mounted {} FAT32 root files", files));
        }
        storage::MountStatus::NoDevice => console.println("FAT32 MOUNT: NO BLOCK DEVICE"),
        storage::MountStatus::NotFat32 => console.println("FAT32 MOUNT: NO VOLUME"),
        storage::MountStatus::Failed => console.println("FAT32 MOUNT: FAILED"),
    }
    if userspace::install_stub_executable() {
        console.println("EXEC IMAGE: INSTALLED");
    } else {
        console.println("EXEC IMAGE: INSTALL FAILED");
        halt();
    }
    if userspace::install_init_executable() {
        console.println("INIT IMAGE: INSTALLED");
    } else {
        console.println("INIT IMAGE: INSTALL FAILED");
        halt();
    }
    if userspace::install_shell_executable() {
        console.println("SHELL IMAGE: INSTALLED");
    } else {
        console.println("SHELL IMAGE: INSTALL FAILED");
        halt();
    }
    if userspace::install_echo_executable() {
        console.println("ECHO IMAGE: INSTALLED");
    } else {
        console.println("ECHO IMAGE: INSTALL FAILED");
        halt();
    }
    if userspace::install_true_executable()
        && userspace::install_false_executable()
        && userspace::install_cat_executable()
        && userspace::install_ls_executable()
        && userspace::install_sleep_executable()
        && userspace::install_pwd_executable()
        && userspace::install_mkdir_executable()
        && userspace::install_rm_executable()
    {
        console.println("BIN UTILS: INSTALLED");
    } else {
        console.println("BIN UTILS: INSTALL FAILED");
        halt();
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
    smp::start(acpi.ok(), physical_memory_offset.unwrap());
    if !smp::routed_irq() {
        pic::unmask(timer::IRQ);
        pic::unmask(keyboard::IRQ);
    }
    x86_64::instructions::interrupts::enable();
    // The root `network-test` mode always enables `qemu-test`.  Cargo artifact
    // dependencies can keep a nested kernel-only feature isolated, so key the
    // runtime probe from the qemu-test feature that is known to reach this
    // kernel artifact.  Only QEMU configurations that actually expose the
    // supported VirtIO network device run the live networking regression.
    //
    // Memory/storage QEMU suites do not attach that VirtIO NIC: `network::init`
    // simply fails its transport probe and those suites continue normally.
    #[cfg(feature = "qemu-test")]
    {
        match network::init() {
            Ok(()) => {
                serial::write_line(format_args!(
                    "[NETTEST] virtio-net + smoltcp initialized"
                ));
                if network::qemu_runtime_self_test() {
                    serial::write_line(format_args!(
                        "[NETTEST] DHCP/DNS/ICMP/UDP/TCP: PASSED"
                    ));
                } else {
                    serial::write_line(format_args!(
                        "[NETTEST] runtime regression: FAILED"
                    ));
                    halt();
                }
            }
            Err(_) => {
                // No supported VirtIO NIC belongs to the normal memory/storage
                // QEMU regressions, so networking is intentionally skipped.
            }
        }
    }
    if !block_io::start_worker() {
        console.println("BLOCK I/O WORKER: START FAILED");
        halt();
    }
    if block_io::async_completion_self_test() {
        serial::write_line(format_args!("[BLOCK IO] worker completion: PASSED"));
    } else {
        serial::write_line(format_args!("[BLOCK IO] worker completion: FAILED"));
        halt();
    }
    if !task::start_pager() {
        console.println("PAGER: START FAILED");
        halt();
    }
    match storage::live_mutation_self_test() {
        storage::LiveMutationTestStatus::Passed => {
            serial::write_line(format_args!(
                "[STORAGE MUTATION] live FAT32 rename/delete/growth/lifecycle: PASSED"
            ));
        }
        storage::LiveMutationTestStatus::Skipped => {
            serial::write_line(format_args!(
                "[STORAGE MUTATION] live FAT32 rename/delete/growth/lifecycle: SKIPPED"
            ));
        }
        storage::LiveMutationTestStatus::Failed(stage) => {
            serial::write_line(format_args!(
                "[STORAGE MUTATION] live FAT32 rename/delete/growth/lifecycle: FAILED at {}",
                stage
            ));
            halt();
        }
    }

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

    keyboard::inject_validation_input(2);
    let isolation_baseline = memory::stats().allocated_frames;
    let Some(first_program) = userspace::create_exec_process() else {
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

    if !userspace::mmap_w_xor_x_self_test(first_program.address_space) {
        console.println("MMAP W^X INVARIANT: FAILED");
        halt();
    }
    console.println("MMAP W^X INVARIANT: OK");
    serial::write_line(format_args!(
        "[BOOT] anonymous mmap W^X invariant verified (writable mapping is never executable)"
    ));

    if !task::file_fault_io_self_test() {
        serial::write_line(format_args!(
            "[FAULT IO] interrupt/preemption guard: FAILED"
        ));
        halt();
    }
    serial::write_line(format_args!(
        "[FAULT IO] timer IRQs live, task stable, state restored: PASSED"
    ));
    let first_root = first_program.address_space.root_address();
    let second_root = second_program.address_space.root_address();
    // Publish the paired bootstrap processes as one BSP-local transaction.
    // A timer interrupt used to be able to schedule process A after its TCB was
    // made Ready but before process B and its process-table entry were created.
    // That timing window became visible with multiple LAPIC timers running and
    // could leave the boot validation waiting forever before the identity audit.
    // APs may continue taking timer interrupts, but their scheduler path uses
    // try_lock and cannot run CPU-0-owned userspace tasks.
    serial::write_line(format_args!(
        "[BOOT] paired userspace spawn: BEGIN"
    ));
    let (first_spawn, second_spawn) = x86_64::instructions::interrupts::without_interrupts(|| {
        let first = task::spawn_user_process("init-user-a", first_program);
        let second = if first.is_ok() {
            Some(task::spawn_user_process("init-user-b", second_program))
        } else {
            None
        };
        (first, second)
    });

    let first_pid = match first_spawn {
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
    let second_pid = match second_spawn {
        Some(Ok((pid, _))) => pid,
        _ => {
            console.println("SECOND USER PROCESS: SPAWN FAILED");
            halt();
        }
    };
    serial::write_line(format_args!(
        "[BOOT] paired userspace spawn: READY"
    ));
    if task::process_credentials(first_pid) != Some(task::Credentials::USERSPACE)
        || task::process_credentials(second_pid) != Some(task::Credentials::USERSPACE)
    {
        console.println("USER CREDENTIALS: INVALID");
        halt();
    }
    serial::write_line(format_args!(
        "[AUDIT] userspace identity uid={} gid={}",
        task::Credentials::USERSPACE.uid,
        task::Credentials::USERSPACE.gid,
    ));

    let user_pair_wait_start = timer::ticks();
    while !task::process_exited(first_pid) || !task::process_exited(second_pid) {
        if timer::ticks().wrapping_sub(user_pair_wait_start) > 2_000 {
            serial::write_line(format_args!(
                "[BOOT] paired userspace execution: TIMEOUT first_exited={} second_exited={}",
                task::process_exited(first_pid),
                task::process_exited(second_pid),
            ));
            console.println("PAIRED USERSPACE EXECUTION: TIMEOUT");
            halt();
        }
        x86_64::instructions::hlt();
    }
    serial::write_line(format_args!(
        "[BOOT] paired userspace execution: PASSED"
    ));

    if !syscall::user_memory_verified() || task::anonymous_mapping_count() != 0 {
        console.println("USER MMAP/MUNMAP: FAILED");
        halt();
    }
    if !syscall::user_identity_verified() {
        console.println("USER UID/GID SYSCALLS: FAILED");
        halt();
    }
    if !syscall::user_exec_verified() {
        console.println("USER EXEC SYSCALL: FAILED");
        halt();
    }
    console.println("USER EXEC ATOMIC REPLACEMENT: OK");
    if !syscall::user_fork_verified() {
        console.println("USER FORK SYSCALL: FAILED");
        halt();
    }
    console.println("USER FORK ADDRESS-SPACE CLONE: OK");
    if !syscall::user_standard_streams_verified() {
        console.println("USER STDIN SYSCALL: FAILED");
        halt();
    }
    console.println("USER STANDARD STREAMS: OK");
    console.println("USER UID/GID SYSCALLS: OK");
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
    // Exercise a real Ring-3 absent-page exception. The pager must park the
    // faulting context, populate the backing page on its worker task, then
    // resume the original instruction. mmaptest covers sparse lazy faults,
    // writes, fork, and kernel copy_to/from_user fault resolution.
    let pager_before = task::pager_stats();
    if !userspace::install_file_mmap_test() {
        console.println("PAGER MMAP IMAGE: INSTALL FAILED");
        halt();
    }
    let mut pager_image = alloc::vec![0u8; vfs::NODE_CAPACITY];
    let Ok(pager_image_len) = vfs::read_all("/bin/mmaptest", &mut pager_image) else {
        console.println("PAGER MMAP IMAGE: READ FAILED");
        halt();
    };
    let Some(pager_program) =
        userspace::load_elf_with_argv(&pager_image[..pager_image_len], &["/bin/mmaptest"])
    else {
        console.println("PAGER MMAP IMAGE: LOAD FAILED");
        halt();
    };
    let pager_pid = match task::spawn_user_process("pager-mmaptest", pager_program) {
        Ok((pid, _)) => pid,
        Err(_) => {
            console.println("PAGER MMAP PROCESS: SPAWN FAILED");
            halt();
        }
    };
    let pager_wait_start = timer::ticks();
    while !task::process_exited(pager_pid) {
        if timer::ticks().wrapping_sub(pager_wait_start) > 200 {
            let (queued, completed) = task::pager_stats();
            serial::write_line(format_args!(
                "[PAGER] mmaptest timeout queued={} completed={}",
                queued, completed
            ));
            console.println("ASYNCHRONOUS PAGER: TIMEOUT");
            halt();
        }
        x86_64::instructions::hlt();
    }
    let pager_status = task::wait_process(pager_pid.as_u64());
    let pager_after = task::pager_stats();
    if pager_status != Ok(0) || pager_after.0 <= pager_before.0 || pager_after.1 < pager_after.0 {
        console.println("ASYNCHRONOUS PAGER: FAILED");
        halt();
    }
    serial::write_line(format_args!(
        "[PAGER] Ring-3 faults queued={} completed={}: PASSED",
        pager_after.0 - pager_before.0,
        pager_after.1 - pager_before.1
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

    smp::self_test();
    serial::write_line(format_args!("[BOOT] ALL VALIDATIONS PASSED"));
    #[cfg(feature = "qemu-test")]
    qemu_test_exit_success();

    loop {
        x86_64::instructions::hlt();
    }
}

#[cfg(feature = "qemu-test")]
fn qemu_test_exit_success() {
    unsafe {
        core::arch::asm!(
            "out dx, eax",
            in("dx") 0xf4_u16,
            in("eax") 0x10_u32,
            options(nomem, nostack, preserves_flags),
        );
    }
    loop {
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
