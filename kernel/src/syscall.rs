use core::{
    arch::{asm, global_asm},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

global_asm!(
    ".global wovenhat_syscall_entry",
    "wovenhat_syscall_entry:",
    "push rax",
    "push rbx",
    "push rcx",
    "push rdx",
    "push rsi",
    "push rdi",
    "push rbp",
    "push r8",
    "push r9",
    "push r10",
    "push r11",
    "push r12",
    "push r13",
    "push r14",
    "push r15",
    "mov rdi, [rsp + 112]",
    "mov rsi, [rsp + 72]",
    "mov rdx, [rsp + 80]",
    "mov rcx, [rsp + 88]",
    "mov r12, rsp",
    "and rsp, -16",
    "call wovenhat_syscall_dispatch",
    "mov rsp, r12",
    "mov [rsp + 112], rax",
    "pop r15",
    "pop r14",
    "pop r13",
    "pop r12",
    "pop r11",
    "pop r10",
    "pop r9",
    "pop r8",
    "pop rbp",
    "pop rdi",
    "pop rsi",
    "pop rdx",
    "pop rcx",
    "pop rbx",
    "pop rax",
    "iretq",
);

static LAST_COMPLETED: AtomicU64 = AtomicU64::new(u64::MAX);
static COMPLETIONS: AtomicU64 = AtomicU64::new(0);
static IO_COMPLETIONS: AtomicU64 = AtomicU64::new(0);
static YIELD_PENDING: AtomicBool = AtomicBool::new(false);

const IO_WRITE: u64 = 1 << 0;
const IO_OPEN: u64 = 1 << 1;
const IO_READ: u64 = 1 << 2;
const IO_CLOSE: u64 = 1 << 3;
const IO_MMAP: u64 = 1 << 4;
const IO_MUNMAP: u64 = 1 << 5;
const IO_FILE_WRITE: u64 = 1 << 6;
const IO_GETUID: u64 = 1 << 7;
const IO_GETGID: u64 = 1 << 8;
const IO_EXEC: u64 = 1 << 9;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Number {
    Read = 0,
    Write = 1,
    Open = 2,
    Exit = 3,
    Getpid = 4,
    Waitpid = 5,
    Close = 6,
    Yield = 7,
    Mmap = 8,
    Munmap = 9,
    FileWrite = 10,
    MessageSend = 11,
    MessageReceive = 12,
    Getuid = 13,
    Getgid = 14,
    Exec = 15,
}

pub fn entry_address() -> u64 {
    unsafe extern "C" {
        fn wovenhat_syscall_entry();
    }

    wovenhat_syscall_entry as *const () as u64
}

pub fn emit(number: Number) -> u64 {
    invoke(number, 0, 0, 0)
}

pub fn invoke(number: Number, arg0: u64, arg1: u64, arg2: u64) -> u64 {
    let result: u64;

    // SAFETY: Vector 0x80 is a DPL3 interrupt gate whose assembly entry saves
    // all general registers and returns the dispatch result in RAX.
    unsafe {
        asm!(
            "int 0x80",
            inlateout("rax") number as u64 => result,
            in("rdi") arg0,
            in("rsi") arg1,
            in("rdx") arg2,
            options(preserves_flags),
        );
    }

    result
}
pub fn test() -> bool {
    let before = COMPLETIONS.load(Ordering::Acquire);
    let result = emit(Number::Getpid);
    COMPLETIONS.load(Ordering::Acquire) == before + 1
        && LAST_COMPLETED.load(Ordering::Acquire) == Number::Getpid as u64
        && result == crate::task::current_process_id()
}

pub fn user_io_verified() -> bool {
    IO_COMPLETIONS.load(Ordering::Acquire) & (IO_WRITE | IO_OPEN | IO_READ | IO_CLOSE)
        == IO_WRITE | IO_OPEN | IO_READ | IO_CLOSE
}

pub fn user_memory_verified() -> bool {
    IO_COMPLETIONS.load(Ordering::Acquire) & (IO_MMAP | IO_MUNMAP) == IO_MMAP | IO_MUNMAP
}

pub fn user_identity_verified() -> bool {
    IO_COMPLETIONS.load(Ordering::Acquire) & (IO_GETUID | IO_GETGID) == IO_GETUID | IO_GETGID
}

pub fn user_exec_verified() -> bool {
    IO_COMPLETIONS.load(Ordering::Acquire) & IO_EXEC == IO_EXEC
}
pub fn last_completed(number: Number) -> bool {
    LAST_COMPLETED.load(Ordering::Acquire) == number as u64
}

pub fn service_pending() {
    if YIELD_PENDING.swap(false, Ordering::AcqRel) {
        crate::task::yield_now();
    }
}

const MAX_IO_SIZE: usize = 256;
const MAX_PATH_SIZE: usize = 64;
const SYSCALL_ERROR: u64 = u64::MAX;

fn sys_mmap(length: u64, flags: u64) -> u64 {
    if flags & !1 != 0 {
        return SYSCALL_ERROR;
    }
    match crate::task::mmap_current(length, flags & 1 != 0) {
        Ok(address) => {
            IO_COMPLETIONS.fetch_or(IO_MMAP, Ordering::Release);
            address
        }
        Err(_) => SYSCALL_ERROR,
    }
}

fn sys_munmap(address: u64, length: u64) -> u64 {
    match crate::task::munmap_current(address, length) {
        Ok(()) => {
            IO_COMPLETIONS.fetch_or(IO_MUNMAP, Ordering::Release);
            0
        }
        Err(_) => SYSCALL_ERROR,
    }
}
fn sys_write(descriptor: u64, user_buffer: u64, length: u64) -> u64 {
    if !matches!(descriptor, 1 | 2)
        || !crate::task::current_has(crate::capability::Capability::Console)
    {
        return SYSCALL_ERROR;
    }
    let Ok(length) = usize::try_from(length) else {
        return SYSCALL_ERROR;
    };
    if length > MAX_IO_SIZE {
        return SYSCALL_ERROR;
    }
    let mut buffer = [0_u8; MAX_IO_SIZE];
    if crate::paging::copy_from_current_user(user_buffer, &mut buffer[..length]).is_err() {
        return SYSCALL_ERROR;
    }
    crate::serial::write_bytes(&buffer[..length]);
    IO_COMPLETIONS.fetch_or(IO_WRITE, Ordering::Release);
    length as u64
}

fn sys_open(user_path: u64, length: u64) -> u64 {
    let Ok(length) = usize::try_from(length) else {
        return SYSCALL_ERROR;
    };
    if length == 0 || length > MAX_PATH_SIZE {
        return SYSCALL_ERROR;
    }
    let mut path = [0_u8; MAX_PATH_SIZE];
    if crate::paging::copy_from_current_user(user_path, &mut path[..length]).is_err() {
        return SYSCALL_ERROR;
    }
    let Ok(path) = core::str::from_utf8(&path[..length]) else {
        return SYSCALL_ERROR;
    };
    match crate::task::open_current(path) {
        Ok(descriptor) => {
            IO_COMPLETIONS.fetch_or(IO_OPEN, Ordering::Release);
            descriptor
        }
        Err(_) => SYSCALL_ERROR,
    }
}

fn sys_read(descriptor: u64, user_buffer: u64, length: u64) -> u64 {
    let Ok(length) = usize::try_from(length) else {
        return SYSCALL_ERROR;
    };
    if length > MAX_IO_SIZE {
        return SYSCALL_ERROR;
    }
    let mut buffer = [0_u8; MAX_IO_SIZE];
    let Ok(count) = crate::task::read_current(descriptor, &mut buffer[..length]) else {
        return SYSCALL_ERROR;
    };
    if crate::paging::copy_to_current_user(user_buffer, &buffer[..count]).is_err() {
        return SYSCALL_ERROR;
    }
    IO_COMPLETIONS.fetch_or(IO_READ, Ordering::Release);
    count as u64
}

fn sys_file_write(descriptor: u64, user_buffer: u64, length: u64) -> u64 {
    let actor = crate::task::current_process_id();
    let Ok(length) = usize::try_from(length) else {
        return SYSCALL_ERROR;
    };
    if length > MAX_IO_SIZE {
        return SYSCALL_ERROR;
    }
    let mut buffer = [0_u8; MAX_IO_SIZE];
    if crate::paging::copy_from_current_user(user_buffer, &mut buffer[..length]).is_err() {
        return SYSCALL_ERROR;
    }
    let result = crate::task::write_current(descriptor, &buffer[..length]);
    crate::audit::record(
        actor,
        crate::audit::Action::FileWrite,
        descriptor,
        result.is_ok(),
    );
    match result {
        Ok(count) => {
            IO_COMPLETIONS.fetch_or(IO_FILE_WRITE, Ordering::Release);
            count as u64
        }
        Err(_) => SYSCALL_ERROR,
    }
}
fn sys_message_send(receiver: u64, user_buffer: u64, length: u64) -> u64 {
    let actor = crate::task::current_process_id();
    if !crate::task::current_has(crate::capability::Capability::Ipc)
        || !crate::task::may_ipc_with(receiver)
    {
        crate::audit::record(actor, crate::audit::Action::IpcSend, receiver, false);
        return SYSCALL_ERROR;
    }
    let Ok(length) = usize::try_from(length) else {
        return SYSCALL_ERROR;
    };
    if length > crate::ipc::MAX_MESSAGE_SIZE {
        return SYSCALL_ERROR;
    }
    let mut buffer = [0_u8; crate::ipc::MAX_MESSAGE_SIZE];
    if crate::paging::copy_from_current_user(user_buffer, &mut buffer[..length]).is_err() {
        return SYSCALL_ERROR;
    }
    let sender = crate::task::current_process_id();
    let result = crate::ipc::send(sender, receiver, &buffer[..length]);
    crate::audit::record(
        actor,
        crate::audit::Action::IpcSend,
        receiver,
        result.is_ok(),
    );
    result.map_or(SYSCALL_ERROR, |()| 0)
}
fn sys_message_receive(user_buffer: u64, capacity: u64, user_sender: u64) -> u64 {
    if !crate::task::current_has(crate::capability::Capability::Ipc) {
        return SYSCALL_ERROR;
    }
    let Ok(capacity) = usize::try_from(capacity) else {
        return SYSCALL_ERROR;
    };
    let receiver = crate::task::current_process_id();
    let Ok(message) = crate::ipc::peek(receiver) else {
        return SYSCALL_ERROR;
    };
    if message.payload().len() > capacity
        || crate::paging::copy_to_current_user(user_buffer, message.payload()).is_err()
        || crate::paging::copy_to_current_user(user_sender, &message.sender.to_le_bytes()).is_err()
    {
        return SYSCALL_ERROR;
    }
    if crate::ipc::receive(receiver).is_err() {
        return SYSCALL_ERROR;
    }
    message.payload().len() as u64
}

fn sys_close(descriptor: u64) -> u64 {
    match crate::task::close_current(descriptor) {
        Ok(()) => {
            IO_COMPLETIONS.fetch_or(IO_CLOSE, Ordering::Release);
            0
        }
        Err(_) => SYSCALL_ERROR,
    }
}
fn sys_exec(user_path: u64, length: u64) -> u64 {
    let actor = crate::task::current_process_id();
    if !crate::task::current_has(crate::capability::Capability::FileRead)
        || !crate::task::current_has(crate::capability::Capability::ProcessCreate)
    {
        crate::audit::record(actor, crate::audit::Action::ProcessExec, length, false);
        return SYSCALL_ERROR;
    }
    let Ok(length) = usize::try_from(length) else {
        return SYSCALL_ERROR;
    };
    if length == 0 || length > MAX_PATH_SIZE {
        return SYSCALL_ERROR;
    }
    let mut path = [0_u8; MAX_PATH_SIZE];
    if crate::paging::copy_from_current_user(user_path, &mut path[..length]).is_err() {
        return SYSCALL_ERROR;
    }
    let Ok(path) = core::str::from_utf8(&path[..length]) else {
        return SYSCALL_ERROR;
    };
    let mut image = alloc::vec![0_u8; crate::vfs::NODE_CAPACITY];
    let Ok(image_length) = crate::vfs::read_all(path, &mut image) else {
        crate::audit::record(
            actor,
            crate::audit::Action::ProcessExec,
            length as u64,
            false,
        );
        return SYSCALL_ERROR;
    };
    let Some(program) = crate::userspace::load_elf(&image[..image_length]) else {
        crate::audit::record(
            actor,
            crate::audit::Action::ProcessExec,
            length as u64,
            false,
        );
        return SYSCALL_ERROR;
    };
    drop(image);
    IO_COMPLETIONS.fetch_or(IO_EXEC, Ordering::Release);
    crate::audit::record(
        actor,
        crate::audit::Action::ProcessExec,
        length as u64,
        true,
    );
    crate::task::exec_current(program)
}
#[unsafe(no_mangle)]
pub extern "C" fn wovenhat_syscall_dispatch(number: u64, arg0: u64, arg1: u64, arg2: u64) -> u64 {
    let result = match number {
        value if value == Number::Mmap as u64 => sys_mmap(arg0, arg1),
        value if value == Number::Munmap as u64 => sys_munmap(arg0, arg1),
        value if value == Number::Read as u64 => sys_read(arg0, arg1, arg2),
        value if value == Number::FileWrite as u64 => sys_file_write(arg0, arg1, arg2),
        value if value == Number::Write as u64 => sys_write(arg0, arg1, arg2),
        value if value == Number::Open as u64 => sys_open(arg0, arg1),
        value if value == Number::Close as u64 => sys_close(arg0),
        value if value == Number::Exec as u64 => sys_exec(arg0, arg1),
        value if value == Number::MessageSend as u64 => sys_message_send(arg0, arg1, arg2),
        value if value == Number::MessageReceive as u64 => sys_message_receive(arg0, arg1, arg2),
        value if value == Number::Getuid as u64 => {
            IO_COMPLETIONS.fetch_or(IO_GETUID, Ordering::Release);
            u64::from(crate::task::current_credentials().uid)
        }
        value if value == Number::Getgid as u64 => {
            IO_COMPLETIONS.fetch_or(IO_GETGID, Ordering::Release);
            u64::from(crate::task::current_credentials().gid)
        }
        value if value == Number::Yield as u64 => {
            YIELD_PENDING.store(true, Ordering::Release);
            0
        }
        value if value == Number::Exit as u64 => crate::task::exit_current_process(arg0 as i32),
        value if value == Number::Getpid as u64 => crate::task::current_process_id(),
        value if value == Number::Waitpid as u64 => match crate::task::wait_process(arg0) {
            Ok(exit_code) => exit_code as u64,
            Err(crate::task::WaitError::StillRunning) => u64::MAX - 1,
            Err(crate::task::WaitError::NoSuchChild) => u64::MAX,
        },
        _ => u64::MAX,
    };

    LAST_COMPLETED.store(number, Ordering::Release);
    COMPLETIONS.fetch_add(1, Ordering::Release);
    result
}
