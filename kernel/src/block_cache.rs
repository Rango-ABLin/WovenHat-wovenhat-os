//! Small write-back sector cache used between filesystems and block devices.
//!
//! This deliberately stays allocation-free so it can be used during early boot.
//! Entries are replaced with a clock/LRU approximation; dirty entries are written
//! back before reuse and by `flush()`/drop-time callers.

use crate::block::{BlockDevice, Error, SECTOR_SIZE};

#[derive(Clone, Copy)]
struct CacheEntry {
    lba: u64,
    data: [u8; SECTOR_SIZE],
    valid: bool,
    dirty: bool,
    age: u32,
}

impl CacheEntry {
    const fn empty() -> Self {
        Self { lba: 0, data: [0; SECTOR_SIZE], valid: false, dirty: false, age: 0 }
    }
}

pub struct CachedDevice<'a, D: BlockDevice, const N: usize = 16> {
    inner: &'a mut D,
    entries: [CacheEntry; N],
    tick: u32,
    hits: u64,
    misses: u64,
}

impl<'a, D: BlockDevice, const N: usize> CachedDevice<'a, D, N> {
    pub fn new(inner: &'a mut D) -> Self {
        Self { inner, entries: [CacheEntry::empty(); N], tick: 1, hits: 0, misses: 0 }
    }

    pub fn hits(&self) -> u64 { self.hits }
    pub fn misses(&self) -> u64 { self.misses }

    fn touch(&mut self, index: usize) {
        self.tick = self.tick.wrapping_add(1).max(1);
        self.entries[index].age = self.tick;
    }

    fn find(&self, lba: u64) -> Option<usize> {
        self.entries.iter().position(|e| e.valid && e.lba == lba)
    }

    fn victim(&self) -> usize {
        if let Some(index) = self.entries.iter().position(|e| !e.valid) { return index; }
        self.entries.iter().enumerate().min_by_key(|(_, e)| e.age).map(|(i, _)| i).unwrap_or(0)
    }

    fn write_back(&mut self, index: usize) -> Result<(), Error> {
        if self.entries[index].valid && self.entries[index].dirty {
            let lba = self.entries[index].lba;
            let data = self.entries[index].data;
            self.inner.write_sector(lba, &data)?;
            self.entries[index].dirty = false;
        }
        Ok(())
    }

    fn load(&mut self, lba: u64) -> Result<usize, Error> {
        if let Some(index) = self.find(lba) {
            self.hits = self.hits.saturating_add(1);
            self.touch(index);
            return Ok(index);
        }
        self.misses = self.misses.saturating_add(1);
        let index = self.victim();
        self.write_back(index)?;
        let mut data = [0u8; SECTOR_SIZE];
        self.inner.read_sector(lba, &mut data)?;
        self.entries[index] = CacheEntry { lba, data, valid: true, dirty: false, age: 0 };
        self.touch(index);
        Ok(index)
    }

    pub fn flush(&mut self) -> Result<(), Error> {
        for index in 0..N { self.write_back(index)?; }
        Ok(())
    }
}

impl<D: BlockDevice, const N: usize> BlockDevice for CachedDevice<'_, D, N> {
    fn sector_count(&self) -> u64 { self.inner.sector_count() }

    fn read_sector(&mut self, lba: u64, sector: &mut [u8]) -> Result<(), Error> {
        if sector.len() != SECTOR_SIZE { return Err(Error::InvalidBuffer); }
        if lba >= self.sector_count() { return Err(Error::OutOfBounds); }
        let index = self.load(lba)?;
        sector.copy_from_slice(&self.entries[index].data);
        Ok(())
    }

    fn write_sector(&mut self, lba: u64, sector: &[u8]) -> Result<(), Error> {
        if sector.len() != SECTOR_SIZE { return Err(Error::InvalidBuffer); }
        if lba >= self.sector_count() { return Err(Error::OutOfBounds); }
        let index = if let Some(index) = self.find(lba) {
            self.hits = self.hits.saturating_add(1);
            index
        } else {
            self.misses = self.misses.saturating_add(1);
            let index = self.victim();
            self.write_back(index)?;
            self.entries[index] = CacheEntry { lba, data: [0; SECTOR_SIZE], valid: true, dirty: false, age: 0 };
            index
        };
        self.entries[index].data.copy_from_slice(sector);
        self.entries[index].dirty = true;
        self.touch(index);
        Ok(())
    }
}

pub fn self_test() -> bool {
    use crate::block::RamDisk;
    let mut disk = RamDisk::<4>::new();
    let mut cache = CachedDevice::<_, 2>::new(&mut disk);
    let mut a = [0u8; SECTOR_SIZE];
    a[..5].copy_from_slice(b"cache");
    if cache.write_sector(1, &a).is_err() { return false; }
    let mut out = [0u8; SECTOR_SIZE];
    if cache.read_sector(1, &mut out).is_err() || out != a || cache.hits() == 0 { return false; }
    cache.flush().is_ok()
}
