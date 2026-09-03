#[derive(Clone, Copy)]
#[repr(u8)]
pub enum Capability {
    Console = 0,
    TimerRead = 1,
    TaskInspect = 2,
    TaskControl = 3,
    DeviceIo = 4,
    InterruptControl = 5,
    MemoryInspect = 6,
    FileRead = 7,
    FileWrite = 8,
    Ipc = 9,
    ProcessCreate = 10,
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
            .with(Capability::TaskControl)
            .with(Capability::DeviceIo)
            .with(Capability::InterruptControl)
            .with(Capability::MemoryInspect)
            .with(Capability::FileRead)
            .with(Capability::FileWrite)
            .with(Capability::Ipc)
            .with(Capability::ProcessCreate)
    }

    pub const fn userspace() -> Self {
        Self::empty()
            .with(Capability::Console)
            .with(Capability::FileRead)
            .with(Capability::FileWrite)
            .with(Capability::Ipc)
            .with(Capability::ProcessCreate)
    }

    pub const fn with(self, capability: Capability) -> Self {
        Self {
            bits: self.bits | capability.bit(),
        }
    }

    pub const fn contains(self, capability: Capability) -> bool {
        self.bits & capability.bit() != 0
    }

    pub const fn without(self, capability: Capability) -> Self {
        Self {
            bits: self.bits & !capability.bit(),
        }
    }
}

impl Capability {
    const fn bit(self) -> u64 {
        1 << self as u8
    }
}
