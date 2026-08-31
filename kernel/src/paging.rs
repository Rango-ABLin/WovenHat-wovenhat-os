use spin::Mutex;
use x86_64::{
    VirtAddr,
    registers::control::Cr3,
    structures::paging::{OffsetPageTable, PageTable, Translate},
};

static PAGING: Mutex<PagingState> = Mutex::new(PagingState::empty());

pub struct Stats {
    pub physical_memory_offset: u64,
    pub level_4_frame: u64,
    pub successful_translations: usize,
    pub tested_translations: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum InitError {
    MissingPhysicalMemoryMapping,
    AlreadyInitialized,
    AddressOverflow,
}

struct PagingState {
    mapper: Option<OffsetPageTable<'static>>,
    physical_memory_offset: u64,
    level_4_frame: u64,
    successful_translations: usize,
    tested_translations: usize,
}

impl PagingState {
    const fn empty() -> Self {
        Self {
            mapper: None,
            physical_memory_offset: 0,
            level_4_frame: 0,
            successful_translations: 0,
            tested_translations: 0,
        }
    }

    fn stats(&self) -> Stats {
        Stats {
            physical_memory_offset: self.physical_memory_offset,
            level_4_frame: self.level_4_frame,
            successful_translations: self.successful_translations,
            tested_translations: self.tested_translations,
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

pub fn stats() -> Stats {
    PAGING.lock().stats()
}
