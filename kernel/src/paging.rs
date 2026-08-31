use spin::Mutex;
use x86_64::{
    VirtAddr,
    registers::control::Cr3,
    structures::paging::{
        FrameAllocator, Mapper, OffsetPageTable, Page, PageSize, PageTable, PageTableFlags,
        Size4KiB, Translate,
    },
};

use crate::memory;

const TEST_PAGE_ADDRESS: u64 = 0x4444_4444_0000;
const TEST_VALUE: u64 = 0x574F_5645_4E48_4154;

static PAGING: Mutex<PagingState> = Mutex::new(PagingState::empty());

pub struct Stats {
    pub physical_memory_offset: u64,
    pub level_4_frame: u64,
    pub successful_translations: usize,
    pub tested_translations: usize,
    pub mapping_test_passed: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum InitError {
    MissingPhysicalMemoryMapping,
    AlreadyInitialized,
    AddressOverflow,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MapRangeError {
    NotInitialized,
    InvalidRange,
    AlreadyMapped,
    OutOfFrames,
    MappingFailed,
}

struct PagingState {
    mapper: Option<OffsetPageTable<'static>>,
    physical_memory_offset: u64,
    level_4_frame: u64,
    successful_translations: usize,
    tested_translations: usize,
    mapping_test_passed: bool,
}

impl PagingState {
    const fn empty() -> Self {
        Self {
            mapper: None,
            physical_memory_offset: 0,
            level_4_frame: 0,
            successful_translations: 0,
            tested_translations: 0,
            mapping_test_passed: false,
        }
    }

    fn stats(&self) -> Stats {
        Stats {
            physical_memory_offset: self.physical_memory_offset,
            level_4_frame: self.level_4_frame,
            successful_translations: self.successful_translations,
            tested_translations: self.tested_translations,
            mapping_test_passed: self.mapping_test_passed,
        }
    }
}

pub fn init(physical_memory_offset: u64) -> Result<(), InitError> {
    let mut paging = PAGING.lock();
    if paging.mapper.is_some() {
        return Err(InitError::AlreadyInitialized);
    }

    let offset = VirtAddr::new(physical_memory_offset);
    let (level_4_frame, _) = Cr3::read();
    let table_address = physical_memory_offset
        .checked_add(level_4_frame.start_address().as_u64())
        .ok_or(InitError::AddressOverflow)?;

    // SAFETY: The bootloader maps all physical memory at `offset`. CR3 names
    // the active level-4 table, and PAGING creates the only mutable Rust view
    // of that table for the remainder of kernel execution.
    let level_4_table = unsafe { &mut *(table_address as *mut PageTable) };

    // SAFETY: `level_4_table` is the uniquely borrowed active table and
    // `offset` is the bootloader-provided physical-memory mapping base.
    let mapper = unsafe { OffsetPageTable::new(level_4_table, offset) };

    paging.level_4_frame = level_4_frame.start_address().as_u64();
    paging.physical_memory_offset = physical_memory_offset;
    paging.mapper = Some(mapper);
    Ok(())
}

pub fn self_test(addresses: &[u64]) -> bool {
    let mut paging = PAGING.lock();
    let Some(mapper) = paging.mapper.as_ref() else {
        return false;
    };

    let successful = addresses
        .iter()
        .filter(|address| mapper.translate_addr(VirtAddr::new(**address)).is_some())
        .count();

    paging.successful_translations = successful;
    paging.tested_translations = addresses.len();
    successful == addresses.len()
}

pub fn mapping_self_test() -> bool {
    let mut paging = PAGING.lock();
    let Some(mapper) = paging.mapper.as_mut() else {
        return false;
    };

    let page = Page::<Size4KiB>::containing_address(VirtAddr::new(TEST_PAGE_ADDRESS));
    if mapper.translate_addr(page.start_address()).is_some() {
        return false;
    }

    let mut allocator = memory::allocator();
    let Some(frame) = allocator.allocate_frame() else {
        return false;
    };
    let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE;

    // SAFETY: `frame` was freshly allocated and `page` was verified unmapped.
    // The paging mutex provides exclusive access to the active page tables.
    let mapping = unsafe { mapper.map_to(page, frame, flags, &mut *allocator) };
    let Ok(flush) = mapping else {
        return false;
    };
    flush.flush();

    let pointer = page.start_address().as_mut_ptr::<u64>();
    // SAFETY: The page is present and writable for this test, and the pointer
    // is naturally aligned within that mapping.
    unsafe { pointer.write_volatile(TEST_VALUE) };
    // SAFETY: The same live mapping and aligned location are read back before
    // the page is unmapped.
    let value = unsafe { pointer.read_volatile() };

    let Ok((_frame, flush)) = mapper.unmap(page) else {
        return false;
    };
    flush.flush();

    let passed = value == TEST_VALUE && mapper.translate_addr(page.start_address()).is_none();
    paging.mapping_test_passed = passed;
    passed
}

pub fn map_range(start: u64, size: usize) -> Result<(), MapRangeError> {
    if size == 0 || start % Size4KiB::SIZE != 0 || size % Size4KiB::SIZE as usize != 0 {
        return Err(MapRangeError::InvalidRange);
    }

    let size = u64::try_from(size).map_err(|_| MapRangeError::InvalidRange)?;
    let last_address = start
        .checked_add(size - 1)
        .ok_or(MapRangeError::InvalidRange)?;
    let start_page = Page::<Size4KiB>::containing_address(VirtAddr::new(start));
    let end_page = Page::<Size4KiB>::containing_address(VirtAddr::new(last_address));

    let mut paging = PAGING.lock();
    let Some(mapper) = paging.mapper.as_mut() else {
        return Err(MapRangeError::NotInitialized);
    };
    let mut allocator = memory::allocator();

    for page in Page::range_inclusive(start_page, end_page) {
        if mapper.translate_addr(page.start_address()).is_some() {
            return Err(MapRangeError::AlreadyMapped);
        }

        let frame = allocator
            .allocate_frame()
            .ok_or(MapRangeError::OutOfFrames)?;
        let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE;

        // SAFETY: Each page is checked to be unmapped and each frame comes
        // uniquely from the physical allocator. Both allocators are locked.
        let flush = unsafe { mapper.map_to(page, frame, flags, &mut *allocator) }
            .map_err(|_| MapRangeError::MappingFailed)?;
        flush.flush();
    }

    Ok(())
}

pub fn stats() -> Stats {
    PAGING.lock().stats()
}
