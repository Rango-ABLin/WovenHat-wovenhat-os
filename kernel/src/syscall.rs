use core::arch::asm;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Number {
    Yield = 0,
    Read = 1,
    Write = 2,
    Exit = 3,
}

pub fn emit(number: Number) {
    unsafe {
        asm!(
            "int 0x80",
            in("rax") number as u64,
            options(nomem, nostack, preserves_flags)
        );
    }
}

pub fn test() {
    emit(Number::Yield);
}
