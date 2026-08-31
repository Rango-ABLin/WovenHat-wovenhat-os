use core::{
    arch::asm,
    sync::atomic::{AtomicU64, Ordering},
};

const NO_REQUEST: u64 = u64::MAX;

static REQUEST: AtomicU64 = AtomicU64::new(NO_REQUEST);
static LAST_COMPLETED: AtomicU64 = AtomicU64::new(NO_REQUEST);
static COMPLETIONS: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Number {
    Yield = 0,
    Read = 1,
    Write = 2,
    Exit = 3,
    Getpid = 4,
}

impl Number {
    pub fn from_u64(value: u64) -> Option<Self> {
        match value {
            0 => Some(Self::Yield),
            1 => Some(Self::Read),
            2 => Some(Self::Write),
            3 => Some(Self::Exit),
            4 => Some(Self::Getpid),
            _ => None,
        }
    }
}

pub fn emit(number: Number) {
    REQUEST.store(number as u64, Ordering::Release);

    unsafe {
        asm!(
            "int 0x80",
            in("rax") number as u64,
            options(nomem, nostack, preserves_flags)
        );
    }
}

pub fn test() -> bool {
    let before = COMPLETIONS.load(Ordering::Acquire);
    emit(Number::Yield);
    COMPLETIONS.load(Ordering::Acquire) == before + 1
        && LAST_COMPLETED.load(Ordering::Acquire) == Number::Yield as u64
}

pub fn dispatch(number: Number) {
    match number {
        Number::Yield => {
            crate::serial::write_fmt(format_args!("SYSCALL: YIELD\n"));
            crate::task::yield_now();
        }
        Number::Read => {
            crate::serial::write_fmt(format_args!("SYSCALL: READ\n"));
        }
        Number::Write => {
            crate::serial::write_fmt(format_args!("SYSCALL: WRITE\n"));
        }
        Number::Exit => {
            crate::serial::write_fmt(format_args!("SYSCALL: EXIT\n"));
            crate::task::exit_current_process();
        }
        Number::Getpid => {
            crate::serial::write_fmt(format_args!("SYSCALL: GETPID\n"));
            if let Some(process) = crate::task::current_process() {
                crate::serial::write_fmt(format_args!("PID: {}\n", process.id.as_u64()));
            }
        }
    }
}

pub fn handle_interrupt() {
    let request = REQUEST.swap(NO_REQUEST, Ordering::AcqRel);
    if request == NO_REQUEST {
        return;
    }

    let Some(number) = Number::from_u64(request) else {
        crate::serial::write_fmt(format_args!("SYSCALL: UNKNOWN NUMBER {request:#x}\n"));
        return;
    };

    LAST_COMPLETED.store(request, Ordering::Release);
    COMPLETIONS.fetch_add(1, Ordering::Release);
    dispatch(number);
}
