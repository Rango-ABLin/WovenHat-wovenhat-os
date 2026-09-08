//! Bounded xAPIC SMP startup and lock-free, acknowledged TLB invalidation.
use crate::{gdt, interrupts, memory, paging, serial, task};
use bootloader_api::info::{MemoryRegion, MemoryRegionKind};
use core::{
    arch::{asm, global_asm, x86_64::__cpuid},
    cell::UnsafeCell,
    sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering},
};
use x86_64::{registers::model_specific::Msr, structures::paging::PageTable};

pub const MAX_CPUS: usize = 4;
pub const TIMER_VECTOR: u8 = 0xe0;
pub const SPURIOUS_VECTOR: u8 = 0xff;
static IDS: [AtomicU32; MAX_CPUS] = [const { AtomicU32::new(u32::MAX) }; MAX_CPUS];
static ONLINE: [AtomicBool; MAX_CPUS] = [const { AtomicBool::new(false) }; MAX_CPUS];
static ACK: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];
static GENERATION: AtomicU64 = AtomicU64::new(0);
static SHOOT_LOCK: AtomicBool = AtomicBool::new(false);
static LAPIC: AtomicU64 = AtomicU64::new(0);
static BOOT_PAGE: AtomicU64 = AtomicU64::new(0);
static COUNT: AtomicUsize = AtomicUsize::new(1);
static IOAPIC: AtomicU64 = AtomicU64::new(0);
static TIMER_COUNT: AtomicU32 = AtomicU32::new(1_000_000);
#[repr(align(16))]
struct Stack(UnsafeCell<[u8; 131072]>);
// Each AP exclusively owns its bootstrap stack for its entire lifetime.
unsafe impl Sync for Stack {}
static STACKS: [Stack; MAX_CPUS] = [const { Stack(UnsafeCell::new([0; 131072])) }; MAX_CPUS];
global_asm!(include_str!("ap_start.S"));
unsafe extern "C" {
    static ap_start: u8;
    static ap_end: u8;
    static ap_root: u8;
    static ap_stack: u8;
    static ap_entry: u8;
    static ap_gdt_ptr: u8;
    static ap_far: u8;
    static ap_long: u8;
    static ap_gdt: u8;
}

pub fn cpu_index() -> usize {
    let id = __cpuid(1).ebx >> 24;
    IDS.iter()
        .position(|entry| entry.load(Ordering::Relaxed) == id)
        .expect("unregistered CPU identity")
}
pub fn online_count() -> usize {
    COUNT.load(Ordering::Acquire)
}
pub fn prepare(regions: &[MemoryRegion]) {
    IDS[0].store(__cpuid(1).ebx >> 24, Ordering::Relaxed);
    ONLINE[0].store(true, Ordering::Release);
    // The frame allocator excludes the low MiB. Never overwrite firmware/reserved RAM.
    for region in regions {
        let Some(aligned) = region.start.max(0x1000).checked_add(4095) else {
            continue;
        };
        let start = aligned & !4095;
        if region.kind == MemoryRegionKind::Usable
            && start
                .checked_add(4096)
                .is_some_and(|end| end <= region.end.min(0xa0000))
        {
            BOOT_PAGE.store(start, Ordering::Relaxed);
            break;
        }
    }
}
fn read(reg: u64) -> u32 {
    unsafe { ((LAPIC.load(Ordering::Relaxed) + reg) as *const u32).read_volatile() }
}
fn write(reg: u64, value: u32) {
    unsafe {
        ((LAPIC.load(Ordering::Relaxed) + reg) as *mut u32).write_volatile(value);
    }
    let _ = read(0x20);
}
pub fn eoi() {
    write(0xb0, 0);
}
fn enable_local() {
    unsafe {
        let mut msr = Msr::new(0x1b);
        let value = msr.read();
        assert_eq!(value & (1 << 10), 0, "x2APIC is unsupported");
        msr.write(value | (1 << 11));
    }
    write(0x80, 0);
    write(0xf0, 0x100 | u32::from(SPURIOUS_VECTOR));
    write(0x350, 1 << 16);
    write(0x360, 1 << 16);
    write(0x320, 1 << 16);
}
fn send(id: u32, command: u32) {
    let start = unsafe { core::arch::x86_64::_rdtsc() };
    while read(0x300) & (1 << 12) != 0 {
        assert!(
            unsafe { core::arch::x86_64::_rdtsc() }.wrapping_sub(start) < 5_000_000_000,
            "APIC delivery timeout"
        );
        core::hint::spin_loop();
    }
    write(0x310, id << 24);
    write(0x300, command);
}
// PIT channel 2 provides a hardware interval independent of IRQ enable state.
fn delay_10ms() {
    use x86_64::instructions::port::Port;
    unsafe {
        let mut speaker = Port::<u8>::new(0x61);
        let saved = speaker.read();
        speaker.write(saved & !3);
        Port::<u8>::new(0x43).write(0xb0);
        Port::<u8>::new(0x42).write(11932u16 as u8);
        Port::<u8>::new(0x42).write((11932u16 >> 8) as u8);
        speaker.write((saved & !2) | 1);
        let start = core::arch::x86_64::_rdtsc();
        while speaker.read() & 0x20 == 0 {
            assert!(
                core::arch::x86_64::_rdtsc().wrapping_sub(start) < 5_000_000_000,
                "PIT calibration timeout"
            );
            core::hint::spin_loop();
        }
        speaker.write(saved);
    }
}
fn start_timer() {
    write(0x3e0, 3); // divide by 16
    write(0x320, (1 << 17) | u32::from(TIMER_VECTOR));
    write(0x380, TIMER_COUNT.load(Ordering::Relaxed));
}
fn io_read(base: u64, register: u32) -> u32 {
    unsafe {
        (base as *mut u32).write_volatile(register);
        ((base + 16) as *const u32).read_volatile()
    }
}
fn io_write(base: u64, register: u32, value: u32) {
    unsafe {
        (base as *mut u32).write_volatile(register);
        ((base + 16) as *mut u32).write_volatile(value);
    }
}
pub fn routed_irq() -> bool {
    IOAPIC.load(Ordering::Acquire) != 0
}

pub fn start(topology: Option<crate::hal::acpi::Summary>, offset: u64) {
    let Some(topology) = topology else {
        serial::write_line(format_args!(
            "[SMP] online=1 expected=1 legacy PIC fallback"
        ));
        return;
    };
    if !topology.apic || topology.processor_count == 0 {
        return;
    }
    assert!(
        !topology.truncated && topology.processor_count <= MAX_CPUS,
        "unsupported CPU topology"
    );
    assert!(
        topology.processor_ids[..topology.processor_count]
            .iter()
            .all(|id| *id < 256),
        "x2APIC IDs unsupported"
    );
    LAPIC.store(
        paging::map_mmio(topology.local_apic_address)
            .ok()
            .expect("LAPIC mapping"),
        Ordering::Relaxed,
    );
    enable_local();
    write(0x3e0, 3);
    write(0x380, u32::MAX);
    delay_10ms();
    TIMER_COUNT.store(u32::MAX - read(0x390), Ordering::Relaxed);
    assert!(
        TIMER_COUNT.load(Ordering::Relaxed) > 0,
        "LAPIC timer calibration"
    );
    // Route only keyboard input through IOAPIC; each CPU has its own LAPIC timer.
    assert_eq!(topology.io_apics, 1, "one IOAPIC required");
    let io = paging::map_mmio(u64::from(topology.io_apic_address))
        .ok()
        .expect("IOAPIC mapping");
    let max = (io_read(io, 1) >> 16) & 0xff;
    for pin in 0..=max {
        io_write(io, 0x10 + pin * 2, 1 << 16);
    }
    let (gsi, flags) = topology.isa_gsi[1].unwrap_or((1, 0));
    assert!(
        gsi >= topology.io_apic_gsi_base && gsi - topology.io_apic_gsi_base <= max,
        "keyboard GSI"
    );
    assert!(
        flags & 3 != 2 && (flags >> 2) & 3 != 2,
        "reserved ISO flags"
    );
    let pin = gsi - topology.io_apic_gsi_base;
    io_write(io, 0x11 + pin * 2, IDS[0].load(Ordering::Relaxed) << 24);
    io_write(
        io,
        0x10 + pin * 2,
        33 | if flags & 3 == 3 { 1 << 13 } else { 0 }
            | if (flags >> 2) & 3 == 3 { 1 << 15 } else { 0 },
    );
    IOAPIC.store(io, Ordering::Release);
    if topology.processor_count > 1 {
        let low = BOOT_PAGE.load(Ordering::Relaxed);
        assert!(low != 0, "no usable AP trampoline page below 1 MiB");
        let root = memory::allocate_frame()
            .expect("AP root")
            .start_address()
            .as_u64();
        let pdpt = memory::allocate_frame()
            .expect("AP PDPT")
            .start_address()
            .as_u64();
        let pd = memory::allocate_frame()
            .expect("AP PD")
            .start_address()
            .as_u64();
        assert!(root < 1u64 << 32, "AP root above 4 GiB");
        unsafe {
            let real_root = paging::kernel_address_space().unwrap().root_address();
            core::ptr::copy_nonoverlapping(
                (offset + real_root) as *const u8,
                (offset + root) as *mut u8,
                4096,
            );
            let table = &mut *((offset + root) as *mut PageTable);
            let flags = x86_64::structures::paging::PageTableFlags::PRESENT
                | x86_64::structures::paging::PageTableFlags::WRITABLE;
            table[0].set_addr(x86_64::PhysAddr::new(pdpt), flags);
            core::ptr::write_bytes((offset + pdpt) as *mut u8, 0, 4096);
            core::ptr::write_bytes((offset + pd) as *mut u8, 0, 4096);
            (&mut *((offset + pdpt) as *mut PageTable))[0]
                .set_addr(x86_64::PhysAddr::new(pd), flags);
            (&mut *((offset + pd) as *mut PageTable))[0].set_addr(
                x86_64::PhysAddr::new(0),
                flags | x86_64::structures::paging::PageTableFlags::HUGE_PAGE,
            );
            let base = &ap_start as *const u8 as usize;
            let size = &ap_end as *const u8 as usize - base;
            assert!(size < 4096);
            core::ptr::copy_nonoverlapping(base as *const u8, (offset + low) as *mut u8, size);
            let patch = |symbol: *const u8| (offset + low + symbol as u64 - base as u64) as *mut u8;
            patch(&ap_root).cast::<u64>().write_unaligned(root);
            patch(&ap_entry)
                .cast::<u64>()
                .write_unaligned(ap_main as *const () as u64);
            patch(&ap_gdt_ptr)
                .add(2)
                .cast::<u32>()
                .write_unaligned((low + &ap_gdt as *const u8 as u64 - base as u64) as u32);
            patch(&ap_far)
                .cast::<u32>()
                .write_unaligned((low + &ap_long as *const u8 as u64 - base as u64) as u32);
            let mut cpu = 1;
            for id in &topology.processor_ids[..topology.processor_count] {
                if *id == IDS[0].load(Ordering::Relaxed) {
                    continue;
                }
                IDS[cpu].store(*id, Ordering::Release);
                patch(&ap_stack)
                    .cast::<u64>()
                    .write_unaligned(STACKS[cpu].0.get() as u64 + 131072);
                send(*id, 0xc500);
                delay_10ms();
                send(*id, 0x8500);
                delay_10ms();
                send(*id, 0x600 | (low >> 12) as u32);
                delay_10ms();
                if !ONLINE[cpu].load(Ordering::Acquire) {
                    send(*id, 0x600 | (low >> 12) as u32);
                }
                for _ in 0..100 {
                    if ONLINE[cpu].load(Ordering::Acquire) {
                        break;
                    }
                    delay_10ms();
                }
                assert!(ONLINE[cpu].load(Ordering::Acquire), "AP startup timeout");
                cpu += 1;
            }
            COUNT.store(cpu, Ordering::Release);
        }
    }
    start_timer();
    serial::write_line(format_args!(
        "[SMP] online={} expected={} LAPIC/IOAPIC enabled",
        online_count(),
        topology.processor_count
    ));
}
extern "C" fn ap_main() -> ! {
    paging::switch_to(paging::kernel_address_space().unwrap());
    gdt::init();
    interrupts::init();
    enable_local();
    let cpu = cpu_index();
    task::init_ap(cpu);
    start_timer();
    ONLINE[cpu].store(true, Ordering::Release);
    loop {
        task::yield_now();
        x86_64::instructions::interrupts::enable_and_hlt();
    }
}
/// NMI handler never acquires a lock, allocates, or schedules.
pub fn acknowledge_tlb() {
    let generation = GENERATION.load(Ordering::Acquire);
    unsafe {
        let cr4: u64;
        asm!("mov {}, cr4", out(reg) cr4, options(nomem, nostack));
        asm!("mov cr4, {}", in(reg) cr4 & !(1 << 7), options(nostack));
        let cr3: u64;
        asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack));
        asm!("mov cr3, {}", in(reg) cr3, options(nostack));
        asm!("mov cr4, {}", in(reg) cr4, options(nostack));
    }
    ACK[cpu_index()].store(generation, Ordering::Release);
}
pub fn shootdown() {
    x86_64::instructions::interrupts::without_interrupts(shootdown_inner);
}
fn shootdown_inner() {
    if online_count() == 1 {
        return;
    }
    while SHOOT_LOCK
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
    let generation = GENERATION.fetch_add(1, Ordering::AcqRel) + 1;
    let current = cpu_index();
    acknowledge_tlb();
    for (cpu, id) in IDS.iter().enumerate().take(online_count()) {
        if cpu != current {
            send(id.load(Ordering::Relaxed), 0x400);
        }
    }
    let start = unsafe { core::arch::x86_64::_rdtsc() };
    while (0..online_count()).any(|cpu| ACK[cpu].load(Ordering::Acquire) < generation) {
        assert!(
            unsafe { core::arch::x86_64::_rdtsc() }.wrapping_sub(start) < 5_000_000_000,
            "TLB shootdown timeout"
        );
        core::hint::spin_loop();
    }
    SHOOT_LOCK.store(false, Ordering::Release);
}

static ARRIVED: AtomicUsize = AtomicUsize::new(0);
static FINISHED: AtomicUsize = AtomicUsize::new(0);
static CPU_MASK: AtomicUsize = AtomicUsize::new(0);
static PROBE_SUM: AtomicU64 = AtomicU64::new(0);
fn parallel_probe() -> ! {
    let cpu = cpu_index();
    CPU_MASK.fetch_or(1 << cpu, Ordering::AcqRel);
    ARRIVED.fetch_add(1, Ordering::AcqRel);
    let start = unsafe { core::arch::x86_64::_rdtsc() };
    while ARRIVED.load(Ordering::Acquire) < online_count() {
        assert!(
            unsafe { core::arch::x86_64::_rdtsc() }.wrapping_sub(start) < 10_000_000_000,
            "parallel barrier timeout"
        );
        core::hint::spin_loop();
    }
    for _ in 0..32 {
        PROBE_SUM.fetch_add(1, Ordering::AcqRel);
        task::yield_now();
    }
    FINISHED.fetch_add(1, Ordering::Release);
    task::exit_current_task();
}
pub fn self_test() {
    static TESTED: AtomicBool = AtomicBool::new(false);
    if TESTED.swap(true, Ordering::AcqRel) {
        return;
    }
    for cpu in 0..online_count() {
        unsafe { task::spawn_on(cpu, "smp-probe", parallel_probe) }
            .ok()
            .expect("SMP probe slots");
    }
    let deadline = crate::timer::ticks() + 500;
    while FINISHED.load(Ordering::Acquire) < online_count() {
        assert!(
            crate::timer::ticks() < deadline,
            "multicore scheduling timeout"
        );
        task::yield_now();
    }
    assert_eq!(CPU_MASK.load(Ordering::Acquire), (1 << online_count()) - 1);
    assert_eq!(
        PROBE_SUM.load(Ordering::Acquire),
        (32 * online_count()) as u64
    );
    // SAFETY: This job touches only an atomic and the CPU-owned scheduler.
    unsafe { task::spawn_parallel("balanced-probe", balanced_probe) }
        .ok()
        .expect("balanced probe slot");
    let deadline = crate::timer::ticks() + 500;
    while !BALANCED_DONE.load(Ordering::Acquire) {
        assert!(crate::timer::ticks() < deadline, "balanced job timeout");
        task::yield_now();
    }
    stale_translation_test();
    preemption_test();
    for _ in 0..32 {
        shootdown();
    }
    serial::write_line(format_args!(
        "[SMP] scheduler/barrier: PASSED cpus={} mask={:#x}",
        online_count(),
        CPU_MASK.load(Ordering::Acquire)
    ));
    serial::write_line(format_args!("[SMP] acknowledged TLB shootdowns: PASSED"));
}

const TEST_PAGE: u64 = 0x5555_6000_0000;
static READ_PHASE: AtomicUsize = AtomicUsize::new(0);
static READ_ACK: [AtomicUsize; MAX_CPUS] = [const { AtomicUsize::new(0) }; MAX_CPUS];
fn translation_reader() -> ! {
    let cpu = cpu_index();
    for phase in 1..=8 {
        while READ_PHASE.load(Ordering::Acquire) < phase {
            core::hint::spin_loop();
        }
        let value = unsafe { (TEST_PAGE as *const u64).read_volatile() };
        assert_eq!(value, phase as u64, "stale remote TLB translation");
        READ_ACK[cpu].store(phase, Ordering::Release);
    }
    task::exit_current_task();
}
fn stale_translation_test() {
    paging::map_range(TEST_PAGE, 4096)
        .ok()
        .expect("SMP test mapping");
    for cpu in 1..online_count() {
        unsafe { task::spawn_on(cpu, "tlb-reader", translation_reader) }
            .ok()
            .expect("TLB reader slot");
    }
    for phase in 1..=8 {
        paging::replace_smp_test_page(TEST_PAGE, phase as u64);
        assert_eq!(
            unsafe { (TEST_PAGE as *const u64).read_volatile() },
            phase as u64
        );
        READ_PHASE.store(phase, Ordering::Release);
        let deadline = crate::timer::ticks() + 500;
        while READ_ACK
            .iter()
            .take(online_count())
            .skip(1)
            .any(|ack| ack.load(Ordering::Acquire) != phase)
        {
            assert!(crate::timer::ticks() < deadline, "TLB reader timeout");
            task::yield_now();
        }
    }
    paging::remove_smp_test_page(TEST_PAGE);
    serial::write_line(format_args!(
        "[SMP] remote stale-translation/refree: PASSED"
    ));
}
static PREEMPT_STARTED: [AtomicBool; MAX_CPUS] = [const { AtomicBool::new(false) }; MAX_CPUS];
static PREEMPT_PEER: [AtomicBool; MAX_CPUS] = [const { AtomicBool::new(false) }; MAX_CPUS];
static PREEMPT_DONE: AtomicUsize = AtomicUsize::new(0);
fn cpu_bound_probe() -> ! {
    let cpu = cpu_index();
    PREEMPT_STARTED[cpu].store(true, Ordering::Release);
    let start = unsafe { core::arch::x86_64::_rdtsc() };
    // No yield: only a hardware timer can schedule the peer on this CPU.
    while !PREEMPT_PEER[cpu].load(Ordering::Acquire) {
        assert!(
            unsafe { core::arch::x86_64::_rdtsc() }.wrapping_sub(start) < 10_000_000_000,
            "AP timer preemption timeout"
        );
        core::hint::spin_loop();
    }
    PREEMPT_DONE.fetch_add(1, Ordering::Release);
    task::exit_current_task();
}
fn preemption_peer() -> ! {
    let cpu = cpu_index();
    while !PREEMPT_STARTED[cpu].load(Ordering::Acquire) {
        task::yield_now();
    }
    PREEMPT_PEER[cpu].store(true, Ordering::Release);
    task::exit_current_task();
}
fn preemption_test() {
    for cpu in 0..online_count() {
        unsafe { task::spawn_on(cpu, "cpu-bound", cpu_bound_probe) }
            .ok()
            .expect("CPU-bound slot");
        unsafe { task::spawn_on(cpu, "preempt-peer", preemption_peer) }
            .ok()
            .expect("preemption peer slot");
    }
    let deadline = crate::timer::ticks() + 500;
    while PREEMPT_DONE.load(Ordering::Acquire) < online_count() {
        assert!(
            crate::timer::ticks() < deadline,
            "per-CPU preemption timeout"
        );
        task::yield_now();
    }
    serial::write_line(format_args!("[SMP] per-CPU timer preemption: PASSED"));
}

static BALANCED_DONE: AtomicBool = AtomicBool::new(false);
fn balanced_probe() -> ! {
    BALANCED_DONE.store(true, Ordering::Release);
    task::exit_current_task();
}
pub fn diagnostic(console: &mut crate::console::Console<'_>) {
    console.print("Online CPUs: ");
    // Console itself remains BSP-owned; diagnostic output never runs in IRQ/NMI.
    let count = online_count();
    console.println(match count {
        1 => "1",
        2 => "2",
        3 => "3",
        _ => "4",
    });
    console.println("CPU-owned kernel scheduling; userspace and I/O run on CPU 0.");
    serial::write_line(format_args!(
        "[SMP] online={} shootdowns={}",
        count,
        GENERATION.load(Ordering::Acquire)
    ));
}
