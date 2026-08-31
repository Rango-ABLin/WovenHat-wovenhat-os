use core::arch::asm;

pub const MASTER_OFFSET: u8 = 32;
pub const SLAVE_OFFSET: u8 = MASTER_OFFSET + 8;

const MASTER_COMMAND: u16 = 0x20;
const MASTER_DATA: u16 = 0x21;
const SLAVE_COMMAND: u16 = 0xa0;
const SLAVE_DATA: u16 = 0xa1;

const INITIALIZE: u8 = 0x11;
const MODE_8086: u8 = 0x01;
const ALL_IRQS_MASKED: u8 = 0xff;

/// Remap both legacy 8259 PICs and leave every hardware IRQ masked.
///
/// This controller is a bootstrap path for QEMU. WovenHat will move to an
/// APIC/x2APIC implementation as the interrupt architecture matures.
pub fn init() {
    // SAFETY: Interrupts remain disabled and the IDT is already loaded. The
    // constants below are the standard 8259 command/data ports, and masking
    // every line prevents delivery before individual IRQ handlers are ready.
    unsafe {
        outb(MASTER_COMMAND, INITIALIZE);
        io_wait();
        outb(SLAVE_COMMAND, INITIALIZE);
        io_wait();

        outb(MASTER_DATA, MASTER_OFFSET);
        io_wait();
        outb(SLAVE_DATA, SLAVE_OFFSET);
        io_wait();

        outb(MASTER_DATA, 1 << 2);
        io_wait();
        outb(SLAVE_DATA, 2);
        io_wait();

        outb(MASTER_DATA, MODE_8086);
        io_wait();
        outb(SLAVE_DATA, MODE_8086);
        io_wait();

        outb(MASTER_DATA, ALL_IRQS_MASKED);
        outb(SLAVE_DATA, ALL_IRQS_MASKED);
    }
}

/// Write one byte to an x86 I/O port.
///
/// # Safety
///
/// The caller must ensure that `port` is valid and that WovenHat owns the
/// associated device.
unsafe fn outb(port: u16, value: u8) {
    // SAFETY: The caller upholds the port-I/O requirements above.
    unsafe {
        asm!(
            "out dx, al",
            in("dx") port,
            in("al") value,
            options(nomem, nostack, preserves_flags)
        );
    }
}

/// Delay long enough for legacy hardware to observe the previous port write.
unsafe fn io_wait() {
    // SAFETY: Port 0x80 is the conventional POST delay port and the written
    // value has no effect on the PIC or other kernel state.
    unsafe { outb(0x80, 0) };
}
