use core::arch::global_asm;

use crate::config::{MAX_ANONYMOUS_MAPPINGS, MAX_ELF_SEGMENTS};
use crate::paging;

pub use crate::config::MAX_ANONYMOUS_MAPPINGS;

const USER_REGION_START: u64 = 0x0000_4000_0000_0000;
const USER_STACK_OFFSET: u64 = 0x1f_0000;
const USER_CODE_SIZE: usize = 4096;

global_asm!(
    ".global wovenhat_user_program_start",
    ".global wovenhat_user_program_end",
    "wovenhat_user_program_start:",
    "mov eax, 16",
    "int 0x80",
    "cmp rax, -1",
    "je wovenhat_user_failure",
    "test rax, rax",
    "jz wovenhat_fork_child",
    "mov r12, rax",
    "wovenhat_fork_wait:",
    "mov eax, 5",
    "mov rdi, r12",
    "int 0x80",
    "cmp rax, -2",
    "jne wovenhat_fork_reaped",
    "mov eax, 7",
    "int 0x80",
    "jmp wovenhat_fork_wait",
    "wovenhat_fork_reaped:",
    "cmp rax, 42",
    "jne wovenhat_user_failure",
    "jmp wovenhat_fork_parent",
    "wovenhat_fork_child:",
    "mov edi, 42",
    "mov eax, 3",
    "int 0x80",
    "wovenhat_fork_parent:",
    "mov eax, 0",
    "xor edi, edi",
    "mov rsi, 0x4000001f0000",
    "mov edx, 1",
    "int 0x80",
    "cmp rax, 1",
    "jne wovenhat_user_failure",
    "cmp byte ptr [0x4000001f0000], 97",
    "jne wovenhat_user_failure",
    "mov eax, 1",
    "mov edi, 1",
    "lea rsi, [rip + wovenhat_user_message]",
    "mov edx, 27",
    "int 0x80",
    "mov eax, 1",
    "mov edi, 2",
    "lea rsi, [rip + wovenhat_user_message]",
    "xor edx, edx",
    "int 0x80",
    "test rax, rax",
    "jne wovenhat_user_failure",
    "mov eax, 2",
    "lea rdi, [rip + wovenhat_user_path]",
    "mov esi, 9",
    "int 0x80",
    "mov r12, rax",
    "mov eax, 0",
    "mov esi, 0x1f0000",
    "mov ebx, 0x40000",
    "shl rbx, 32",
    "or rsi, rbx",
    "mov rdi, r12",
    "mov edx, 64",
    "int 0x80",
    "mov r13, rax",
    "mov eax, 1",
    "mov edi, 1",
    "mov esi, 0x1f0000",
    "mov ebx, 0x40000",
    "shl rbx, 32",
    "or rsi, rbx",
    "mov eax, 6",
    "mov rdi, r12",
    "int 0x80",
    "mov eax, 8",
    "mov edi, 4096",
    "mov esi, 1",
    "int 0x80",
    "mov r14, rax",
    "mov r15d, 0x4e484154",
    "mov ebx, 0x574f5645",
    "shl rbx, 32",
    "or r15, rbx",
    "mov qword ptr [r14], r15",
    "cmp qword ptr [r14], r15",
    "jne wovenhat_user_failure",
    "mov eax, 9",
    "mov rdi, r14",
    "mov esi, 4096",
    "int 0x80",
    "test rax, rax",
    "jne wovenhat_user_failure",
    "mov eax, 13",
    "int 0x80",
    "cmp rax, 1000",
    "jne wovenhat_user_failure",
    "mov eax, 14",
    "int 0x80",
    "cmp rax, 1000",
    "jne wovenhat_user_failure",
    "mov eax, 4",
    "int 0x80",
    "xor edi, edi",
    "mov eax, 3",
    "int 0x80",
    "wovenhat_user_failure:",
    "mov edi, 1",
    "mov eax, 3",
    "int 0x80",
    "2:",
    "jmp 2b",
    "wovenhat_user_message:",
    ".ascii \"[USER] syscall I/O online!\\n\"",
    "wovenhat_user_path:",
    ".ascii \"/etc/motd\"",
    "wovenhat_user_program_end:",
);

global_asm!(
    ".section .rodata.wovenhat_exec_stub, \"a\"",
    ".global wovenhat_exec_program_start",
    ".global wovenhat_exec_program_end",
    "wovenhat_exec_program_start:",
    "mov eax, 15",
    "lea rdi, [rip + wovenhat_exec_path]",
    "mov esi, 13",
    "int 0x80",
    "mov edi, 1",
    "mov eax, 3",
    "int 0x80",
    "1:",
    "jmp 1b",
    "wovenhat_exec_path:",
    ".ascii \"/bin/selftest\"",
    "wovenhat_exec_program_end:",
    ".previous",
);

// Userspace init: read argc/argv from the stack, print a banner, exit 0.
global_asm!(
    ".section .rodata.wovenhat_init_stub, \"a\"",
    ".global wovenhat_init_program_start",
    ".global wovenhat_init_program_end",
    "wovenhat_init_program_start:",
    // crt0: argc in [rsp], argv at [rsp+8] (kept on stack for later use)
    "mov r12, [rsp]",
    "lea r13, [rsp + 8]",
    // write(1, banner, len)
    "mov eax, 1",
    "mov edi, 1",
    "lea rsi, [rip + wovenhat_init_banner]",
    "mov edx, 28",
    "int 0x80",
    // exit(0)
    "xor edi, edi",
    "mov eax, 3",
    "int 0x80",
    "1:",
    "jmp 1b",
    "wovenhat_init_banner:",
    ".ascii \"[INIT] userspace online\\n\"",
    "wovenhat_init_program_end:",
    ".previous",
);

unsafe extern "C" {
    static wovenhat_user_program_start: u8;
    static wovenhat_user_program_end: u8;
    static wovenhat_exec_program_start: u8;
    static wovenhat_exec_program_end: u8;
    static wovenhat_init_program_start: u8;
    static wovenhat_init_program_end: u8;
}
#[derive(Clone, Copy, Debug)]
pub struct UserImage {
    pub entry: u64,
    pub stack_top: u64,
    pub image_size: u64,
    pub load_segments: usize,
}

impl UserImage {
    pub fn is_valid(self) -> bool {
        self.entry >= USER_REGION_START
            && self.stack_top != 0
            && self.stack_top.is_multiple_of(16)
            && self.image_size != 0
            && self.load_segments != 0
    }
}

#[derive(Clone, Copy, Debug)]
pub struct UserStack {
    pub guard_base: u64,
    pub base: u64,
    pub top: u64,
    pub size: usize,
}

impl UserStack {
    pub const GUARD_SIZE: usize = 4096;
    pub const SIZE: usize = 4096 * 2;

    pub fn new(base: u64) -> Self {
        Self {
            guard_base: base - Self::GUARD_SIZE as u64,
            base,
            top: base + Self::SIZE as u64,
            size: Self::SIZE,
        }
    }

    pub fn is_aligned(self) -> bool {
        self.guard_base.is_multiple_of(4096)
            && self.base == self.guard_base + Self::GUARD_SIZE as u64
            && self.base.is_multiple_of(16)
            && self.top.is_multiple_of(16)
    }
}

/// Maximum number of argv strings placed on the initial user stack.
pub const MAX_ARGV: usize = 8;
/// Maximum total bytes for all argv string data (including NULs).
pub const MAX_ARGV_BYTES: usize = 512;

/// Minimal crt0 contract expected by the kernel stack layout:
///
/// ```asm
/// _start:
///     mov rdi, [rsp]       ; argc
///     lea rsi, [rsp+8]     ; argv
///     call main
///     mov eax, 3           ; exit
///     mov rdi, rax
///     int 0x80
/// ```
///
/// Build a System V-style initial stack for a new user process.
///
/// Layout (high → low addresses):
/// ```text
///   [ string area: argv[0]\0 argv[1]\0 ... ]
///   [ padding to 16-byte alignment ]
///   NULL          (argv terminator)
///   argv[n-1]
///   ...
///   argv[0]
///   argc
///   ← rsp (16-byte aligned)
/// ```
///
/// On entry the process may read argc from `[rsp]` and argv from `[rsp+8]`.
/// Registers are not set; a minimal crt0 can load them from the stack.
///
/// Returns the new stack pointer (value to place in RSP), or `None` on failure.
pub fn setup_argv_stack(
    address_space: paging::AddressSpace,
    stack: UserStack,
    args: &[&str],
) -> Option<u64> {
    if args.len() > MAX_ARGV {
        return None;
    }
    let mut string_bytes = 0usize;
    for arg in args {
        string_bytes = string_bytes.checked_add(arg.len().checked_add(1)?)?;
    }
    if string_bytes > MAX_ARGV_BYTES {
        return None;
    }

    // Write strings just below stack.top, growing downward conceptually but we
    // place them in a contiguous region ending at stack.top.
    let strings_start = stack.top.checked_sub(string_bytes as u64)?;
    if strings_start < stack.base {
        return None;
    }

    let mut cursor = strings_start;
    let mut argv_ptrs = [0u64; MAX_ARGV];
    for (i, arg) in args.iter().enumerate() {
        argv_ptrs[i] = cursor;
        let mut buf = [0u8; 256];
        if arg.len() >= buf.len() {
            return None;
        }
        buf[..arg.len()].copy_from_slice(arg.as_bytes());
        // include trailing NUL
        paging::write_user_bytes(address_space, cursor, &buf[..arg.len() + 1]).ok()?;
        cursor = cursor.checked_add(arg.len() as u64 + 1)?;
    }

    // Build the pointer vector below the string area.
    // argc + (n+1) pointers, then align rsp to 16 bytes.
    let argc = args.len() as u64;
    let pointer_words = 1 + args.len() + 1; // argc + argv... + NULL
    let pointer_bytes = pointer_words * 8;
    let mut rsp = strings_start.checked_sub(pointer_bytes as u64)?;
    // 16-byte align
    rsp &= !0xfu64;
    if rsp < stack.base {
        return None;
    }

    // Write argc
    paging::write_user_bytes(address_space, rsp, &argc.to_le_bytes()).ok()?;
    // Write argv pointers
    for i in 0..args.len() {
        let addr = rsp + 8 + (i as u64) * 8;
        paging::write_user_bytes(address_space, addr, &argv_ptrs[i].to_le_bytes()).ok()?;
    }
    // NULL terminator
    let null_addr = rsp + 8 + (args.len() as u64) * 8;
    paging::write_user_bytes(address_space, null_addr, &0u64.to_le_bytes()).ok()?;

    Some(rsp)
}

const USER_MMAP_START: u64 = USER_REGION_START + 0x10_0000;
const USER_MMAP_STRIDE: u64 = 0x10_000;
const USER_MMAP_MAX_SIZE: usize = USER_MMAP_STRIDE as usize;

#[derive(Clone, Copy)]
pub struct AnonymousMapping {
    pub address: u64,
    pub size: usize,
    pub writable: bool,
}
#[derive(Clone, Copy)]
struct UserMapping {
    start: u64,
    size: usize,
    writable: bool,
    executable: bool,
}

impl UserMapping {
    const EMPTY: Self = Self {
        start: 0,
        size: 0,
        writable: false,
        executable: false,
    };
}

#[derive(Clone, Copy)]
pub struct AddressSpace {
    paging: paging::AddressSpace,
    stack_base: u64,
    mappings: [UserMapping; MAX_ELF_SEGMENTS],
    mapping_count: usize,
}

impl AddressSpace {
    pub fn paging(self) -> paging::AddressSpace {
        self.paging
    }

    pub fn root_address(self) -> u64 {
        self.paging.root_address()
    }

    /// Returns whether `address` falls in a logically writable mapping
    /// (ELF segment, user stack, or anonymous region tracked by the process).
    pub fn is_logically_writable(
        self,
        address: u64,
        anonymous: &[Option<AnonymousMapping>; MAX_ANONYMOUS_MAPPINGS],
    ) -> bool {
        if address >= self.stack_base && address < self.stack_base + UserStack::SIZE as u64 {
            return true;
        }
        for mapping in &self.mappings[..self.mapping_count] {
            if mapping.writable
                && address >= mapping.start
                && address < mapping.start + mapping.size as u64
            {
                return true;
            }
        }
        for mapping in anonymous.iter().flatten() {
            if mapping.writable
                && address >= mapping.address
                && address < mapping.address + mapping.size as u64
            {
                return true;
            }
        }
        false
    }
}

#[derive(Clone, Copy)]
pub struct UserProgram {
    pub image: UserImage,
    pub stack: UserStack,
    pub address_space: AddressSpace,
}

pub fn elf_loader_self_test() -> bool {
    let stub = unsafe {
        let start = &wovenhat_user_program_start as *const u8;
        let end = &wovenhat_user_program_end as *const u8;
        core::slice::from_raw_parts(start, end.offset_from(start) as usize)
    };
    let Some(valid) = build_stub_elf(stub) else {
        return false;
    };
    let Ok(image) = crate::elf::parse(&valid) else {
        return false;
    };
    let valid_segment = image.segment_count() == 1
        && image.entry == USER_REGION_START
        && image
            .segments()
            .next()
            .is_some_and(|segment| segment.memory_size == USER_CODE_SIZE && segment.executable);

    let mut bad_magic = valid.clone();
    bad_magic[0] = 0;
    let mut writable_executable = valid;
    writable_executable[68] = 7;
    valid_segment
        && crate::elf::parse(&bad_magic).is_err()
        && crate::elf::parse(&writable_executable).is_err()
}
pub fn create_stub_process() -> Option<UserProgram> {
    let stub = unsafe {
        let start = &wovenhat_user_program_start as *const u8;
        let end = &wovenhat_user_program_end as *const u8;
        core::slice::from_raw_parts(start, end.offset_from(start) as usize)
    };
    let elf = build_stub_elf(stub)?;
    load_elf(&elf)
}

pub fn install_stub_executable() -> bool {
    let stub = unsafe {
        let start = &wovenhat_user_program_start as *const u8;
        let end = &wovenhat_user_program_end as *const u8;
        core::slice::from_raw_parts(start, end.offset_from(start) as usize)
    };
    build_stub_elf(stub)
        .is_some_and(|elf| crate::vfs::create_read_only("/bin/selftest", &elf).is_ok())
}

pub fn create_exec_process() -> Option<UserProgram> {
    let stub = unsafe {
        let start = &wovenhat_exec_program_start as *const u8;
        let end = &wovenhat_exec_program_end as *const u8;
        core::slice::from_raw_parts(start, end.offset_from(start) as usize)
    };
    let elf = build_stub_elf(stub)?;
    load_elf(&elf)
}

/// Spawnable userspace init process (`argv[0] == "/init"`).
pub fn create_init_process() -> Option<UserProgram> {
    let stub = unsafe {
        let start = &wovenhat_init_program_start as *const u8;
        let end = &wovenhat_init_program_end as *const u8;
        core::slice::from_raw_parts(start, end.offset_from(start) as usize)
    };
    let elf = build_stub_elf(stub)?;
    load_elf_with_argv(&elf, &["/init"])
}

/// Install `/bin/init` into the VFS for later `exec`.
pub fn install_init_executable() -> bool {
    let stub = unsafe {
        let start = &wovenhat_init_program_start as *const u8;
        let end = &wovenhat_init_program_end as *const u8;
        core::slice::from_raw_parts(start, end.offset_from(start) as usize)
    };
    build_stub_elf(stub).is_some_and(|elf| crate::vfs::create_read_only("/bin/init", &elf).is_ok())
}
pub fn load_elf(bytes: &[u8]) -> Option<UserProgram> {
    let image = crate::elf::parse(bytes).ok()?;
    let page_table = paging::create_user_address_space(image.entry)?;
    let mut mappings = [UserMapping::EMPTY; MAX_ELF_SEGMENTS];
    let mut mapping_count = 0;

    let stack_base = USER_REGION_START.checked_add(USER_STACK_OFFSET)?;
    let stack = UserStack::new(stack_base);
    for segment in image.segments() {
        let mapping_end = segment
            .mapping_start
            .checked_add(segment.mapping_size as u64)?;
        if segment.mapping_start < USER_REGION_START || mapping_end > stack.guard_base {
            release_partial(page_table, &mappings[..mapping_count]);
            return None;
        }
        if mapping_count == mappings.len()
            || paging::map_user_range_in(
                page_table,
                segment.mapping_start,
                segment.mapping_size,
                true,
                false,
            )
            .is_err()
        {
            release_partial(page_table, &mappings[..mapping_count]);
            return None;
        }
        mappings[mapping_count] = UserMapping {
            start: segment.mapping_start,
            size: segment.mapping_size,
            writable: segment.writable,
            executable: segment.executable,
        };
        mapping_count += 1;

        let file_end = segment.file_offset.checked_add(segment.file_size)?;
        let memory_end = segment
            .virtual_address
            .checked_add(segment.memory_size as u64)?;
        if memory_end
            > segment
                .mapping_start
                .checked_add(segment.mapping_size as u64)?
            || paging::zero_user_range_in(page_table, segment.mapping_start, segment.mapping_size)
                .is_err()
            || paging::write_user_bytes(
                page_table,
                segment.virtual_address,
                bytes.get(segment.file_offset..file_end)?,
            )
            .is_err()
            || paging::protect_user_range_in(
                page_table,
                segment.mapping_start,
                segment.mapping_size,
                segment.writable,
                segment.executable,
            )
            .is_err()
        {
            release_partial(page_table, &mappings[..mapping_count]);
            return None;
        }
    }

    if paging::map_user_range_in(page_table, stack_base, UserStack::SIZE, true, false).is_err() {
        release_partial(page_table, &mappings[..mapping_count]);
        return None;
    }
    if paging::zero_user_range_in(page_table, stack_base, UserStack::SIZE).is_err() {
        release_with_stack(page_table, stack_base, &mappings[..mapping_count]);
        return None;
    }

    if !paging::user_range_is_unmapped_in(page_table, stack.guard_base, UserStack::GUARD_SIZE)
        || !paging::user_range_has_protection_in(page_table, stack.base, stack.size, true, false)
    {
        release_with_stack(page_table, stack_base, &mappings[..mapping_count]);
        return None;
    }

    // Default argv so every loaded image starts with a valid C-style stack.
    let stack_top = setup_argv_stack(page_table, stack, &["a.out"])?;

    Some(UserProgram {
        image: UserImage {
            entry: image.entry,
            stack_top,
            image_size: bytes.len() as u64,
            load_segments: image.segment_count(),
        },
        stack,
        address_space: AddressSpace {
            paging: page_table,
            stack_base,
            mappings,
            mapping_count,
        },
    })
}

/// Load an ELF and place a custom argv vector on the user stack.
pub fn load_elf_with_argv(bytes: &[u8], args: &[&str]) -> Option<UserProgram> {
    let mut program = load_elf(bytes)?;
    // Rebuild argv over the default one.
    let stack_top = setup_argv_stack(program.address_space.paging(), program.stack, args)?;
    program.image.stack_top = stack_top;
    Some(program)
}

fn release_with_stack(page_table: paging::AddressSpace, stack_base: u64, mappings: &[UserMapping]) {
    let mut ranges = [(0, 0); MAX_ELF_SEGMENTS + 1];
    ranges[0] = (stack_base, UserStack::SIZE);
    for (range, mapping) in ranges[1..].iter_mut().zip(mappings) {
        *range = (mapping.start, mapping.size);
    }
    let _ = paging::destroy_user_address_space(page_table, &ranges[..mappings.len() + 1]);
}
fn release_partial(page_table: paging::AddressSpace, mappings: &[UserMapping]) {
    if mappings.is_empty() {
        let _ = paging::discard_empty_user_address_space(page_table);
        return;
    }
    let mut ranges = [(0, 0); MAX_ELF_SEGMENTS];
    for (range, mapping) in ranges.iter_mut().zip(mappings) {
        *range = (mapping.start, mapping.size);
    }
    let _ = paging::destroy_user_address_space(page_table, &ranges[..mappings.len()]);
}

pub fn map_anonymous(
    address_space: AddressSpace,
    slot: usize,
    size: usize,
    writable: bool,
) -> Option<AnonymousMapping> {
    if slot >= MAX_ANONYMOUS_MAPPINGS
        || size == 0
        || size > USER_MMAP_MAX_SIZE
        || !size.is_multiple_of(4096)
    {
        return None;
    }
    let address = USER_MMAP_START.checked_add((slot as u64).checked_mul(USER_MMAP_STRIDE)?)?;
    if paging::map_user_range_in(address_space.paging, address, size, writable, false).is_err() {
        return None;
    }
    if paging::zero_user_range_in(address_space.paging, address, size).is_err() {
        let _ = paging::unmap_user_range_in(address_space.paging, address, size);
        return None;
    }
    Some(AnonymousMapping {
        address,
        size,
        writable,
    })
}

pub fn unmap_anonymous(address_space: AddressSpace, mapping: AnonymousMapping) -> bool {
    paging::unmap_user_range_in(address_space.paging, mapping.address, mapping.size).is_ok()
}
pub fn destroy_process_address_space(
    address_space: AddressSpace,
    anonymous: [Option<AnonymousMapping>; MAX_ANONYMOUS_MAPPINGS],
) -> bool {
    for mapping in anonymous.into_iter().flatten() {
        if !unmap_anonymous(address_space, mapping) {
            return false;
        }
    }
    destroy(address_space)
}
pub fn destroy(address_space: AddressSpace) -> bool {
    let mut ranges = [(0, 0); MAX_ELF_SEGMENTS + 1];
    ranges[0] = (address_space.stack_base, UserStack::SIZE);
    for (range, mapping) in ranges[1..]
        .iter_mut()
        .zip(&address_space.mappings[..address_space.mapping_count])
    {
        *range = (mapping.start, mapping.size);
    }
    paging::destroy_user_address_space(
        address_space.paging,
        &ranges[..address_space.mapping_count + 1],
    )
    .is_ok()
}

pub fn clone_address_space(
    source: AddressSpace,
    anonymous: &[Option<AnonymousMapping>; MAX_ANONYMOUS_MAPPINGS],
) -> Option<AddressSpace> {
    let destination_paging = paging::create_user_address_space(source.stack_base)?;
    let destination = AddressSpace {
        paging: destination_paging,
        stack_base: source.stack_base,
        mappings: source.mappings,
        mapping_count: source.mapping_count,
    };
    let mut completed = [(0_u64, 0_usize); MAX_ELF_SEGMENTS + 1 + MAX_ANONYMOUS_MAPPINGS];
    let mut completed_count = 0;

    // Copy-on-write: share physical frames and mark writable ranges read-only.
    for mapping in &source.mappings[..source.mapping_count] {
        if paging::share_user_range_in(
            source.paging,
            destination.paging,
            mapping.start,
            mapping.size,
            mapping.writable,
            mapping.executable,
        )
        .is_err()
        {
            release_clone(destination.paging, &completed[..completed_count]);
            return None;
        }
        completed[completed_count] = (mapping.start, mapping.size);
        completed_count += 1;
    }
    if paging::share_user_range_in(
        source.paging,
        destination.paging,
        source.stack_base,
        UserStack::SIZE,
        true,
        false,
    )
    .is_err()
    {
        release_clone(destination.paging, &completed[..completed_count]);
        return None;
    }
    completed[completed_count] = (source.stack_base, UserStack::SIZE);
    completed_count += 1;

    for mapping in anonymous.iter().flatten() {
        if paging::share_user_range_in(
            source.paging,
            destination.paging,
            mapping.address,
            mapping.size,
            mapping.writable,
            false,
        )
        .is_err()
        {
            release_clone(destination.paging, &completed[..completed_count]);
            return None;
        }
        completed[completed_count] = (mapping.address, mapping.size);
        completed_count += 1;
    }
    Some(destination)
}

fn release_clone(address_space: paging::AddressSpace, ranges: &[(u64, usize)]) {
    if ranges.is_empty() {
        let _ = paging::discard_empty_user_address_space(address_space);
    } else {
        let _ = paging::destroy_user_address_space(address_space, ranges);
    }
}
fn build_stub_elf(stub: &[u8]) -> Option<alloc::vec::Vec<u8>> {
    const PAYLOAD_OFFSET: usize = 4096;
    const PROGRAM_HEADER_OFFSET: usize = 64;
    let total_size = PAYLOAD_OFFSET.checked_add(stub.len())?;
    let mut bytes = alloc::vec![0_u8; total_size];
    bytes[..4].copy_from_slice(b"\x7fELF");
    bytes[4] = 2;
    bytes[5] = 1;
    bytes[6] = 1;
    write_u16(&mut bytes, 16, 2)?;
    write_u16(&mut bytes, 18, 0x3e)?;
    write_u32(&mut bytes, 20, 1)?;
    write_u64(&mut bytes, 24, USER_REGION_START)?;
    write_u64(&mut bytes, 32, PROGRAM_HEADER_OFFSET as u64)?;
    write_u16(&mut bytes, 52, 64)?;
    write_u16(&mut bytes, 54, 56)?;
    write_u16(&mut bytes, 56, 1)?;

    write_u32(&mut bytes, PROGRAM_HEADER_OFFSET, 1)?;
    write_u32(&mut bytes, PROGRAM_HEADER_OFFSET + 4, 5)?;
    write_u64(&mut bytes, PROGRAM_HEADER_OFFSET + 8, PAYLOAD_OFFSET as u64)?;
    write_u64(&mut bytes, PROGRAM_HEADER_OFFSET + 16, USER_REGION_START)?;
    write_u64(&mut bytes, PROGRAM_HEADER_OFFSET + 32, stub.len() as u64)?;
    write_u64(
        &mut bytes,
        PROGRAM_HEADER_OFFSET + 40,
        USER_CODE_SIZE as u64,
    )?;
    write_u64(&mut bytes, PROGRAM_HEADER_OFFSET + 48, 4096)?;
    bytes[PAYLOAD_OFFSET..].copy_from_slice(stub);
    Some(bytes)
}

fn write_u16(bytes: &mut [u8], offset: usize, value: u16) -> Option<()> {
    bytes
        .get_mut(offset..offset + 2)?
        .copy_from_slice(&value.to_le_bytes());
    Some(())
}

fn write_u32(bytes: &mut [u8], offset: usize, value: u32) -> Option<()> {
    bytes
        .get_mut(offset..offset + 4)?
        .copy_from_slice(&value.to_le_bytes());
    Some(())
}

fn write_u64(bytes: &mut [u8], offset: usize, value: u64) -> Option<()> {
    bytes
        .get_mut(offset..offset + 8)?
        .copy_from_slice(&value.to_le_bytes());
    Some(())
}
