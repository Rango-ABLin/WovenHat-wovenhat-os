#[derive(Clone, Copy, Debug)]
pub struct ElfStub {
    pub magic: [u8; 4],
    pub class: u8,
    pub endian: u8,
    pub version: u8,
    pub entry: u64,
    pub stack_top: u64,
    pub image_size: u64,
}

impl ElfStub {
    pub const ELF_MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];

    pub const fn new(entry: u64, stack_top: u64) -> Self {
        Self {
            magic: Self::ELF_MAGIC,
            class: 2,
            endian: 1,
            version: 1,
            entry,
            stack_top,
            image_size: 4096,
        }
    }

    pub fn is_valid(self) -> bool {
        self.magic == Self::ELF_MAGIC && self.class == 2 && self.endian == 1 && self.version == 1
    }
}

pub fn stub_program(entry: usize, stack_top: usize) -> ElfStub {
    ElfStub::new(entry as u64, stack_top as u64)
}

pub fn validate_stub(entry: usize, stack_top: usize) -> bool {
    stub_program(entry, stack_top).is_valid()
}
