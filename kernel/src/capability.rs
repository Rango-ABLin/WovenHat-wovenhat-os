#[derive(Clone, Copy)]
#[repr(u8)]
pub enum Capability {
    Console = 0,
    TimerRead = 1,
    TaskInspect = 2,
    DeviceIo = 3,
    InterruptControl = 4,
}

#[derive(Clone, Copy)]
pub struct CapabilitySet {
    bits: u64,
}

impl CapabilitySet {
    pub const fn empty() -> Self {
        Self { bits: 0 }
    }

    pub const fn kernel_bootstrap() -> Self {
        Self::empty()
            .with(Capability::Console)
            .with(Capability::TimerRead)
            .with(Capability::TaskInspect)
            .with(Capability::DeviceIo)
            .with(Capability::InterruptControl)
    }

    pub const fn with(self, capability: Capability) -> Self {
        Self {
            bits: self.bits | capability.bit(),
        }
    }

    pub const fn contains(self, capability: Capability) -> bool {
        self.bits & capability.bit() != 0
    }
}

impl Capability {
    const fn bit(self) -> u64 {
        1 << self as u8
    }
}
