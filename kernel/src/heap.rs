use core::{
    alloc::{GlobalAlloc, Layout},
    ptr::null_mut,
    sync::atomic::{AtomicUsize, Ordering},
};

use alloc::{boxed::Box, vec::Vec};

use crate::paging;

pub const START: u64 = 0x4444_5000_0000;
pub const SIZE: usize = 256 * 1024;

static NEXT: AtomicUsize = AtomicUsize::new(0);
static END: AtomicUsize = AtomicUsize::new(0);
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
static ALLOCATED_BYTES: AtomicUsize = AtomicUsize::new(0);

#[global_allocator]
static ALLOCATOR: BumpAllocator = BumpAllocator;

struct BumpAllocator;

pub struct Stats {
    pub start: u64,
    pub size: usize,
    pub allocated_bytes: usize,
    pub allocations: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum InitError {
    Paging,
    AlreadyInitialized,
    AddressOverflow,
}

// SAFETY: Allocations are handed out from an atomically advanced cursor. Each
// successful caller receives a unique, suitably aligned range in the mapped
// heap. Deallocation is intentionally deferred in this bootstrap allocator.
unsafe impl GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let mut current = NEXT.load(Ordering::Acquire);
        if current == 0 {
            return null_mut();
        }

        loop {
            let Some(start) = align_up(current, layout.align()) else {
                return null_mut();
            };
            let Some(next) = start.checked_add(layout.size()) else {
                return null_mut();
            };
            if next > END.load(Ordering::Relaxed) {
                return null_mut();
            }

            match NEXT.compare_exchange_weak(current, next, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => {
                    ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
                    ALLOCATED_BYTES.fetch_add(layout.size(), Ordering::Relaxed);
                    return start as *mut u8;
                }
                Err(updated) => current = updated,
            }
        }
    }

    unsafe fn dealloc(&self, _pointer: *mut u8, _layout: Layout) {
        // The first heap is monotonic. A reclaiming allocator will replace it
        // once allocation behavior is established and measured.
    }
}

pub fn init() -> Result<(), InitError> {
    if NEXT.load(Ordering::Acquire) != 0 {
        return Err(InitError::AlreadyInitialized);
    }

    paging::map_range(START, SIZE).map_err(|_| InitError::Paging)?;

    let start = usize::try_from(START).map_err(|_| InitError::AddressOverflow)?;
    let end = start.checked_add(SIZE).ok_or(InitError::AddressOverflow)?;
    END.store(end, Ordering::Relaxed);
    NEXT.store(start, Ordering::Release);
    Ok(())
}

pub fn self_test() -> bool {
    let boxed = Box::new(0x574F_5645_4E48_4154_u64);
    let mut values = Vec::with_capacity(64);
    for value in 0..64_u64 {
        values.push(value * value);
    }

    *boxed == 0x574F_5645_4E48_4154
        && values.len() == 64
        && values[0] == 0
        && values[7] == 49
        && values[63] == 3969
}

pub fn stats() -> Stats {
    Stats {
        start: START,
        size: SIZE,
        allocated_bytes: ALLOCATED_BYTES.load(Ordering::Relaxed),
        allocations: ALLOCATIONS.load(Ordering::Relaxed),
    }
}

fn align_up(value: usize, alignment: usize) -> Option<usize> {
    value
        .checked_add(alignment - 1)
        .map(|address| address & !(alignment - 1))
}
