#[derive(Clone, Copy, Debug)]
pub struct UserImage {
    pub magic: [u8; 4],
    pub class: u8,
    pub endian: u8,
    pub version: u8,
    pub entry: u64,
    pub stack_top: u64,
    pub image_size: u64,
}

impl UserImage {
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

#[derive(Clone, Copy, Debug)]
pub struct UserStack {
    pub base: u64,
    pub top: u64,
    pub size: usize,
}

impl UserStack {
    pub const SIZE: usize = 4096 * 2;

    pub fn new(base: u64) -> Self {
        Self {
            base,
            top: base + Self::SIZE as u64,
            size: Self::SIZE,
        }
    }

    pub fn is_aligned(self) -> bool {
        self.base % 16 == 0 && self.top % 16 == 0
    }
}

pub fn allocate_user_stack() -> Option<UserStack> {
    let layout = core::alloc::Layout::from_size_align(UserStack::SIZE, 16).ok()?;
    let ptr = unsafe { alloc::alloc::alloc(layout) };
    if ptr.is_null() {
        return None;
    }
    let base = ptr as u64;
    Some(UserStack::new(base))
}

pub fn stub_program(entry: usize, stack_top: usize) -> UserImage {
    UserImage::new(entry as u64, stack_top as u64)
}

pub fn validate_stub(entry: usize, stack_top: usize) -> bool {
    stub_program(entry, stack_top).is_valid()
}

pub fn describe(entry: usize, stack_top: usize) -> (u64, u64, u64) {
    let image = stub_program(entry, stack_top);
    (image.entry, image.stack_top, image.image_size)
}

unsafe extern "C" {
    pub fn wovenhat_user_stub() -> !;
}
