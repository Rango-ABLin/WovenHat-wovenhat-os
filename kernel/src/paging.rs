use spin::Mutex;
use x86_64::{
    registers::control::{Cr3, Cr3Flags},
    registers::model_specific::{Efer, EferFlags},
    structures::paging::{
        mapper::TranslateResult, FrameAllocator, Mapper, OffsetPageTable, Page, PageSize,
        PageTable, PageTableFlags, PhysFrame, Size4KiB, Translate,
    },
    VirtAddr,
};

use crate::memory;

const TEST_PAGE_ADDRESS: u64 = 0x4444_4444_0000;
const TEST_VALUE: u64 = 0x574F_5645_4E48_4154;

static PAGING: Mutex<PagingState> = Mutex::new(PagingState::empty());

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct AddressSpace {
    level_4_frame: PhysFrame<Size4KiB>,
}

impl AddressSpace {
    pub const fn root_address(self) -> u64 {
        self.level_4_frame.start_address().as_u64()
    }
}
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
    NotMapped,
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
    // SAFETY: NXE is enabled before any no-execute mappings are created.
    unsafe { Efer::update(|flags| *flags |= EferFlags::NO_EXECUTE_ENABLE) };

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
    map_range_with_flags(
        start,
        size,
        PageTableFlags::PRESENT | PageTableFlags::WRITABLE,
    )
}

fn user_flags(writable: bool, executable: bool) -> PageTableFlags {
    let mut flags = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE;
    if writable {
        flags |= PageTableFlags::WRITABLE;
    }
    if !executable {
        flags |= PageTableFlags::NO_EXECUTE;
    }
    flags
}

fn page_range(start: u64, size: usize) -> Result<(Page<Size4KiB>, Page<Size4KiB>), MapRangeError> {
    if size == 0
        || !start.is_multiple_of(Size4KiB::SIZE)
        || !size.is_multiple_of(Size4KiB::SIZE as usize)
    {
        return Err(MapRangeError::InvalidRange);
    }
    let size = u64::try_from(size).map_err(|_| MapRangeError::InvalidRange)?;
    let last = start
        .checked_add(size - 1)
        .ok_or(MapRangeError::InvalidRange)?;
    Ok((
        Page::containing_address(VirtAddr::new(start)),
        Page::containing_address(VirtAddr::new(last)),
    ))
}

fn map_range_with_flags(
    start: u64,
    size: usize,
    flags: PageTableFlags,
) -> Result<(), MapRangeError> {
    let (start_page, end_page) = page_range(start, size)?;

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

        // SAFETY: Each page is checked to be unmapped and each frame comes
        // uniquely from the physical allocator. Both allocators are locked.
        let flush = unsafe { mapper.map_to(page, frame, flags, &mut *allocator) }
            .map_err(|_| MapRangeError::MappingFailed)?;
        flush.flush();
    }

    Ok(())
}

pub fn kernel_address_space() -> Option<AddressSpace> {
    let paging = PAGING.lock();
    paging.mapper.as_ref()?;
    PhysFrame::from_start_address(x86_64::PhysAddr::new(paging.level_4_frame))
        .ok()
        .map(|level_4_frame| AddressSpace { level_4_frame })
}

pub fn create_user_address_space(user_address: u64) -> Option<AddressSpace> {
    let paging = PAGING.lock();
    paging.mapper.as_ref()?;
    let root_frame = memory::allocate_frame()?;
    let kernel_table = page_table_at(paging.physical_memory_offset, paging.level_4_frame)?;
    let new_table = page_table_at_mut(
        paging.physical_memory_offset,
        root_frame.start_address().as_u64(),
    )?;
    *new_table = kernel_table.clone();
    new_table[Page::<Size4KiB>::containing_address(VirtAddr::new(user_address)).p4_index()]
        .set_unused();
    Some(AddressSpace {
        level_4_frame: root_frame,
    })
}

pub fn map_user_range_in(
    address_space: AddressSpace,
    start: u64,
    size: usize,
    writable: bool,
    executable: bool,
) -> Result<(), MapRangeError> {
    let (start_page, end_page) = page_range(start, size)?;
    let paging = PAGING.lock();
    let mut mapper = mapper_for(&paging, address_space)?;
    let mut allocator = memory::allocator();
    let flags = user_flags(writable, executable);

    for page in Page::range_inclusive(start_page, end_page) {
        if mapper.translate_addr(page.start_address()).is_some() {
            return Err(MapRangeError::AlreadyMapped);
        }
        let frame = allocator
            .allocate_frame()
            .ok_or(MapRangeError::OutOfFrames)?;
        unsafe { mapper.map_to(page, frame, flags, &mut *allocator) }
            .map_err(|_| MapRangeError::MappingFailed)?
            .ignore();
    }
    Ok(())
}

pub fn protect_user_range_in(
    address_space: AddressSpace,
    start: u64,
    size: usize,
    writable: bool,
    executable: bool,
) -> Result<(), MapRangeError> {
    let (start_page, end_page) = page_range(start, size)?;
    let paging = PAGING.lock();
    let mut mapper = mapper_for(&paging, address_space)?;
    let flags = user_flags(writable, executable);

    for page in Page::range_inclusive(start_page, end_page) {
        unsafe { mapper.update_flags(page, flags) }
            .map_err(|_| MapRangeError::NotMapped)?
            .ignore();
    }
    Ok(())
}

pub fn write_user_bytes(
    address_space: AddressSpace,
    start: u64,
    bytes: &[u8],
) -> Result<(), MapRangeError> {
    let paging = PAGING.lock();
    let mapper = mapper_for(&paging, address_space)?;
    let mut copied = 0;
    while copied < bytes.len() {
        let virtual_address = start
            .checked_add(copied as u64)
            .ok_or(MapRangeError::InvalidRange)?;
        let physical_address = mapper
            .translate_addr(VirtAddr::new(virtual_address))
            .ok_or(MapRangeError::NotMapped)?;
        let page_remaining =
            Size4KiB::SIZE as usize - virtual_address as usize % Size4KiB::SIZE as usize;
        let count = core::cmp::min(page_remaining, bytes.len() - copied);
        let destination = paging
            .physical_memory_offset
            .checked_add(physical_address.as_u64())
            .ok_or(MapRangeError::InvalidRange)? as *mut u8;
        unsafe {
            core::ptr::copy_nonoverlapping(bytes[copied..].as_ptr(), destination, count);
        }
        copied += count;
    }
    Ok(())
}

pub fn zero_user_range_in(
    address_space: AddressSpace,
    start: u64,
    size: usize,
) -> Result<(), MapRangeError> {
    let _ = page_range(start, size)?;
    let paging = PAGING.lock();
    let mapper = mapper_for(&paging, address_space)?;
    let mut cleared = 0;
    while cleared < size {
        let virtual_address = start
            .checked_add(cleared as u64)
            .ok_or(MapRangeError::InvalidRange)?;
        let physical_address = mapper
            .translate_addr(VirtAddr::new(virtual_address))
            .ok_or(MapRangeError::NotMapped)?;
        let page_remaining =
            Size4KiB::SIZE as usize - virtual_address as usize % Size4KiB::SIZE as usize;
        let count = core::cmp::min(page_remaining, size - cleared);
        let destination = paging
            .physical_memory_offset
            .checked_add(physical_address.as_u64())
            .ok_or(MapRangeError::InvalidRange)? as *mut u8;
        unsafe { core::ptr::write_bytes(destination, 0, count) };
        cleared += count;
    }
    Ok(())
}
pub fn unmap_user_range_in(
    address_space: AddressSpace,
    start: u64,
    size: usize,
) -> Result<(), MapRangeError> {
    let (start_page, end_page) = page_range(start, size)?;
    let active = Cr3::read().0 == address_space.level_4_frame;
    let paging = PAGING.lock();
    let mut mapper = mapper_for(&paging, address_space)?;
    for page in Page::range_inclusive(start_page, end_page) {
        let (frame, flush) = mapper.unmap(page).map_err(|_| MapRangeError::NotMapped)?;
        if active {
            flush.flush();
        } else {
            flush.ignore();
        }
        if !memory::deallocate_frame(frame) {
            return Err(MapRangeError::MappingFailed);
        }
    }
    Ok(())
}
pub fn destroy_user_address_space(
    address_space: AddressSpace,
    ranges: &[(u64, usize)],
) -> Result<(), MapRangeError> {
    if ranges.is_empty() || Cr3::read().0 == address_space.level_4_frame {
        return Err(MapRangeError::MappingFailed);
    }

    let paging = PAGING.lock();
    let mut mapper = mapper_for(&paging, address_space)?;
    for &(start, size) in ranges {
        let (start_page, end_page) = page_range(start, size)?;
        for page in Page::range_inclusive(start_page, end_page) {
            let (frame, flush) = mapper.unmap(page).map_err(|_| MapRangeError::NotMapped)?;
            flush.ignore();
            if !memory::deallocate_frame(frame) {
                return Err(MapRangeError::MappingFailed);
            }
        }
    }

    let first_page = Page::<Size4KiB>::containing_address(VirtAddr::new(ranges[0].0));
    let root = page_table_at_mut(
        paging.physical_memory_offset,
        address_space.level_4_frame.start_address().as_u64(),
    )
    .ok_or(MapRangeError::MappingFailed)?;
    let p3_frame = root[first_page.p4_index()]
        .frame()
        .map_err(|_| MapRangeError::MappingFailed)?;
    let p3 = page_table_at_mut(
        paging.physical_memory_offset,
        p3_frame.start_address().as_u64(),
    )
    .ok_or(MapRangeError::MappingFailed)?;
    let p2_frame = p3[first_page.p3_index()]
        .frame()
        .map_err(|_| MapRangeError::MappingFailed)?;
    let p2 = page_table_at_mut(
        paging.physical_memory_offset,
        p2_frame.start_address().as_u64(),
    )
    .ok_or(MapRangeError::MappingFailed)?;
    let p1_frame = p2[first_page.p2_index()]
        .frame()
        .map_err(|_| MapRangeError::MappingFailed)?;

    root[first_page.p4_index()].set_unused();
    for frame in [p1_frame, p2_frame, p3_frame, address_space.level_4_frame] {
        if !memory::deallocate_frame(frame) {
            return Err(MapRangeError::MappingFailed);
        }
    }
    Ok(())
}

pub fn discard_empty_user_address_space(address_space: AddressSpace) -> bool {
    if Cr3::read().0 == address_space.level_4_frame {
        return false;
    }
    memory::deallocate_frame(address_space.level_4_frame)
}
pub fn switch_to(address_space: AddressSpace) {
    if Cr3::read().0 == address_space.level_4_frame {
        return;
    }
    unsafe { Cr3::write(address_space.level_4_frame, Cr3Flags::empty()) };
}

fn mapper_for(
    paging: &PagingState,
    address_space: AddressSpace,
) -> Result<OffsetPageTable<'static>, MapRangeError> {
    let table = page_table_at_mut(
        paging.physical_memory_offset,
        address_space.level_4_frame.start_address().as_u64(),
    )
    .ok_or(MapRangeError::MappingFailed)?;
    Ok(unsafe { OffsetPageTable::new(table, VirtAddr::new(paging.physical_memory_offset)) })
}

fn page_table_at(offset: u64, physical: u64) -> Option<&'static PageTable> {
    let address = offset.checked_add(physical)?;
    Some(unsafe { &*(address as *const PageTable) })
}

fn page_table_at_mut(offset: u64, physical: u64) -> Option<&'static mut PageTable> {
    let address = offset.checked_add(physical)?;
    Some(unsafe { &mut *(address as *mut PageTable) })
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum UserMemoryError {
    AddressOverflow,
    NotMapped,
    PermissionDenied,
}

pub fn copy_from_current_user(start: u64, destination: &mut [u8]) -> Result<(), UserMemoryError> {
    copy_current_user(start, destination, false)
}

pub fn copy_to_current_user(start: u64, source: &[u8]) -> Result<(), UserMemoryError> {
    let paging = PAGING.lock();
    let address_space = AddressSpace {
        level_4_frame: Cr3::read().0,
    };
    let mapper = mapper_for(&paging, address_space).map_err(|_| UserMemoryError::NotMapped)?;
    let mut copied = 0;
    while copied < source.len() {
        let virtual_address = start
            .checked_add(copied as u64)
            .ok_or(UserMemoryError::AddressOverflow)?;
        let (physical_address, count, flags) =
            translated_chunk(&mapper, virtual_address, source.len() - copied)?;
        if !flags.contains(PageTableFlags::USER_ACCESSIBLE)
            || !flags.contains(PageTableFlags::WRITABLE)
        {
            return Err(UserMemoryError::PermissionDenied);
        }
        let destination = paging
            .physical_memory_offset
            .checked_add(physical_address)
            .ok_or(UserMemoryError::AddressOverflow)? as *mut u8;
        unsafe {
            core::ptr::copy_nonoverlapping(source[copied..].as_ptr(), destination, count);
        }
        copied += count;
    }
    Ok(())
}

fn copy_current_user(
    start: u64,
    destination: &mut [u8],
    require_writable: bool,
) -> Result<(), UserMemoryError> {
    let paging = PAGING.lock();
    let address_space = AddressSpace {
        level_4_frame: Cr3::read().0,
    };
    let mapper = mapper_for(&paging, address_space).map_err(|_| UserMemoryError::NotMapped)?;
    let mut copied = 0;
    while copied < destination.len() {
        let virtual_address = start
            .checked_add(copied as u64)
            .ok_or(UserMemoryError::AddressOverflow)?;
        let (physical_address, count, flags) =
            translated_chunk(&mapper, virtual_address, destination.len() - copied)?;
        if !flags.contains(PageTableFlags::USER_ACCESSIBLE)
            || (require_writable && !flags.contains(PageTableFlags::WRITABLE))
        {
            return Err(UserMemoryError::PermissionDenied);
        }
        let source = paging
            .physical_memory_offset
            .checked_add(physical_address)
            .ok_or(UserMemoryError::AddressOverflow)? as *const u8;
        unsafe {
            core::ptr::copy_nonoverlapping(source, destination[copied..].as_mut_ptr(), count);
        }
        copied += count;
    }
    Ok(())
}

fn translated_chunk(
    mapper: &OffsetPageTable<'static>,
    virtual_address: u64,
    remaining: usize,
) -> Result<(u64, usize, PageTableFlags), UserMemoryError> {
    match mapper.translate(VirtAddr::new(virtual_address)) {
        TranslateResult::Mapped {
            frame,
            offset,
            flags,
        } => {
            let available = usize::try_from(frame.size() - offset)
                .map_err(|_| UserMemoryError::AddressOverflow)?;
            let count = core::cmp::min(available, remaining);
            let physical = frame
                .start_address()
                .as_u64()
                .checked_add(offset)
                .ok_or(UserMemoryError::AddressOverflow)?;
            Ok((physical, count, flags))
        }
        TranslateResult::NotMapped | TranslateResult::InvalidFrameAddress(_) => {
            Err(UserMemoryError::NotMapped)
        }
    }
}
pub fn stats() -> Stats {
    PAGING.lock().stats()
}
