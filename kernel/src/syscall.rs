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

pub fn emit(number: Number) {
    REQUEST.store(number as u64, Ordering::Release);

    // SAFETY: IDT vector 0x80 is installed before shell commands run. The
    // current handler only records the request and returns through the CPU's
    // interrupt frame; it never schedules or changes privilege level.
    unsafe {
        asm!(
            "int 0x80",
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

pub fn handle_interrupt() {
    let request = REQUEST.swap(NO_REQUEST, Ordering::AcqRel);
    if request == NO_REQUEST {
        return;
    }

    if request > Number::Getpid as u64 {
        return;
    }

    LAST_COMPLETED.store(request, Ordering::Release);
    COMPLETIONS.fetch_add(1, Ordering::Release);
}
