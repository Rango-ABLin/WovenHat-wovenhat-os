use core::{
    alloc::{GlobalAlloc, Layout},
    ptr::null_mut,
};

use alloc::{boxed::Box, vec::Vec};
use spin::Mutex;

use crate::paging;

pub const START: u64 = 0x4444_5000_0000;
pub const SIZE: usize = 256 * 1024;
const MAX_ALLOCATIONS: usize = 256;

#[global_allocator]
static ALLOCATOR: TrackedBumpAllocator = TrackedBumpAllocator;
static HEAP: Mutex<HeapState> = Mutex::new(HeapState::empty());

#[derive(Clone, Copy)]
struct Allocation {
    ptr: usize,
    size: usize,
}

struct HeapState {
    start: usize,
    end: usize,
    next: usize,
    total_allocations: usize,
    total_allocated_bytes: usize,
    allocations: [Option<Allocation>; MAX_ALLOCATIONS],
}

impl HeapState {
    const fn empty() -> Self {
        Self {
            start: 0,
            end: 0,
            next: 0,
            total_allocations: 0,
            total_allocated_bytes: 0,
            allocations: [None; MAX_ALLOCATIONS],
        }
    }

    fn init(&mut self) -> Result<(), InitError> {
        if self.start != 0 {
            return Err(InitError::AlreadyInitialized);
        }

        paging::map_range(START, SIZE).map_err(|_| InitError::Paging)?;

        let start = usize::try_from(START).map_err(|_| InitError::AddressOverflow)?;
        let end = start.checked_add(SIZE).ok_or(InitError::AddressOverflow)?;
        self.start = start;
        self.end = end;
        self.next = start;
        Ok(())
    }

    fn alloc(&mut self, layout: Layout) -> *mut u8 {
        if self.start == 0 {
            return null_mut();
        }

        let aligned = match align_up(self.next, layout.align()) {
            Some(value) => value,
            None => return null_mut(),
        };

        let end = match aligned.checked_add(layout.size()) {
            Some(value) => value,
            None => return null_mut(),
        };

        if end > self.end {
            return null_mut();
        }

        let slot = match self.allocations.iter_mut().position(|slot| slot.is_none()) {
            Some(index) => index,
            None => return null_mut(),
        };

        self.allocations[slot] = Some(Allocation {
            ptr: aligned,
            size: layout.size(),
        });
        self.next = end;
        self.total_allocations += 1;
        self.total_allocated_bytes += layout.size();
        aligned as *mut u8
    }

    fn dealloc(&mut self, pointer: *mut u8, _layout: Layout) {
        let ptr = pointer as usize;
        let Some(slot) = self
            .allocations
            .iter_mut()
            .position(|slot| matches!(slot, Some(allocation) if allocation.ptr == ptr))
        else {
            return;
        };

        let Some(allocation) = self.allocations[slot].take() else {
            return;
        };

        self.total_allocations = self.total_allocations.saturating_sub(1);
        self.total_allocated_bytes = self.total_allocated_bytes.saturating_sub(allocation.size);
    }

    fn stats(&self) -> Stats {
        Stats {
            start: START,
            size: SIZE,
            allocated_bytes: self.total_allocated_bytes,
            allocations: self.total_allocations,
        }
    }
}

struct TrackedBumpAllocator;

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

unsafe impl GlobalAlloc for TrackedBumpAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        HEAP.lock().alloc(layout)
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        HEAP.lock().dealloc(pointer, layout)
    }
}

pub fn init() -> Result<(), InitError> {
    HEAP.lock().init()
}

pub fn self_test() -> bool {
    let boxed = Box::new(0x574F_5645_4E48_4154_u64);
    let value = *boxed;
    drop(boxed);

    let mut values = Vec::with_capacity(64);
    for value in 0..64_u64 {
        values.push(value * value);
    }

    let result = value == 0x574F_5645_4E48_4154
        && values.len() == 64
        && values[0] == 0
        && values[7] == 49
        && values[63] == 3969;

    drop(values);
    result
}

pub fn stats() -> Stats {
    HEAP.lock().stats()
}

fn align_up(value: usize, alignment: usize) -> Option<usize> {
    if alignment == 0 || !alignment.is_power_of_two() {
        return None;
    }

    value
        .checked_add(alignment - 1)
        .map(|address| address & !(alignment - 1))
}
