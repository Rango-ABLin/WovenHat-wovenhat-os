use crate::block::{BlockDevice, Error as BlockError, SECTOR_SIZE};

const FAT32_MIN_CLUSTERS: u32 = 65_525;
const DIRECTORY_ENTRY_SIZE: usize = 32;
const DIRECTORY_ENTRIES_PER_SECTOR: usize = SECTOR_SIZE / DIRECTORY_ENTRY_SIZE;
const FAT32_ENTRY_MASK: u32 = 0x0fff_ffff;
const FAT32_BAD_CLUSTER: u32 = 0x0fff_fff7;
const FAT32_END_MIN: u32 = 0x0fff_fff8;
const MAX_READ_CLUSTERS: usize = 64;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Block(BlockError),
    InvalidBootSector,
    UnsupportedGeometry,
    CorruptDirectory,
    DirectoryFull,
    CorruptChain,
    ChainLoop,
    ChainTooLong,
    TruncatedFile,
    NotFound,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Volume {
    pub total_sectors: u32,
    pub sectors_per_cluster: u8,
    pub fat_count: u8,
    pub fat_size: u32,
    pub root_cluster: u32,
    pub first_fat_sector: u64,
    pub first_data_sector: u64,
    cluster_count: u32,
}

impl Volume {
    pub fn cluster_lba(&self, cluster: u32) -> Result<u64, Error> {
        if cluster < 2 || cluster >= self.cluster_count.saturating_add(2) {
            return Err(Error::CorruptChain);
        }
        self.first_data_sector
            .checked_add((cluster - 2) as u64 * self.sectors_per_cluster as u64)
            .ok_or(Error::UnsupportedGeometry)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct DirectoryEntry {
    pub short_name: [u8; 11],
    pub first_cluster: u32,
    pub size: u32,
    pub attributes: u8,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ClusterLink {
    Next(u32),
    End,
}

pub fn mount(device: &mut impl BlockDevice) -> Result<Volume, Error> {
    let mut sector = [0_u8; SECTOR_SIZE];
    device.read_sector(0, &mut sector).map_err(Error::Block)?;

    if sector[510] != 0x55 || sector[511] != 0xaa {
        return Err(Error::InvalidBootSector);
    }
    let bytes_per_sector = read_u16(&sector, 11);
    let sectors_per_cluster = sector[13];
    let reserved_sectors = read_u16(&sector, 14);
    let fat_count = sector[16];
    let root_entries = read_u16(&sector, 17);
    let total_sectors_16 = read_u16(&sector, 19);
    let fat_size_16 = read_u16(&sector, 22);
    let total_sectors = read_u32(&sector, 32);
    let fat_size = read_u32(&sector, 36);
    let root_cluster = read_u32(&sector, 44);

    if bytes_per_sector as usize != SECTOR_SIZE
        || sectors_per_cluster == 0
        || !sectors_per_cluster.is_power_of_two()
        || sectors_per_cluster > 128
        || reserved_sectors == 0
        || !matches!(fat_count, 1 | 2)
        || root_entries != 0
        || total_sectors_16 != 0
        || fat_size_16 != 0
        || total_sectors == 0
        || fat_size == 0
        || root_cluster < 2
    {
        return Err(Error::UnsupportedGeometry);
    }
    if total_sectors as u64 > device.sector_count() {
        return Err(Error::UnsupportedGeometry);
    }

    let fat_sectors = (fat_count as u32)
        .checked_mul(fat_size)
        .ok_or(Error::UnsupportedGeometry)?;
    let first_data = (reserved_sectors as u32)
        .checked_add(fat_sectors)
        .ok_or(Error::UnsupportedGeometry)?;
    let data_sectors = total_sectors
        .checked_sub(first_data)
        .ok_or(Error::UnsupportedGeometry)?;
    let cluster_count = data_sectors / sectors_per_cluster as u32;
    if cluster_count < FAT32_MIN_CLUSTERS || root_cluster >= cluster_count.saturating_add(2) {
        return Err(Error::UnsupportedGeometry);
    }

    Ok(Volume {
        total_sectors,
        sectors_per_cluster,
        fat_count,
        fat_size,
        root_cluster,
        first_fat_sector: reserved_sectors as u64,
        first_data_sector: first_data as u64,
        cluster_count,
    })
}

pub fn find_root(
    device: &mut impl BlockDevice,
    volume: Volume,
    short_name: &[u8; 11],
) -> Result<DirectoryEntry, Error> {
    let mut cluster = volume.root_cluster;
    let mut visited = [0_u32; MAX_READ_CLUSTERS];
    let mut visited_count = 0;
    let mut sector = [0_u8; SECTOR_SIZE];

    loop {
        if visited_count == visited.len() {
            return Err(Error::ChainTooLong);
        }
        if visited[..visited_count].contains(&cluster) {
            return Err(Error::ChainLoop);
        }
        visited[visited_count] = cluster;
        visited_count += 1;

        let cluster_lba = volume.cluster_lba(cluster)?;
        for sector_index in 0..volume.sectors_per_cluster as u64 {
            device
                .read_sector(cluster_lba + sector_index, &mut sector)
                .map_err(Error::Block)?;
            match scan_directory_sector(&sector, short_name)? {
                DirectoryScan::Found(entry) => return Ok(entry),
                DirectoryScan::End => return Err(Error::NotFound),
                DirectoryScan::Continue => {}
            }
        }

        cluster = match next_cluster(device, volume, cluster)? {
            ClusterLink::Next(next) => next,
            ClusterLink::End => return Err(Error::NotFound),
        };
    }
}

pub fn list_root(
    device: &mut impl BlockDevice,
    volume: Volume,
    output: &mut [Option<DirectoryEntry>],
) -> Result<usize, Error> {
    let mut cluster = volume.root_cluster;
    let mut visited = [0_u32; MAX_READ_CLUSTERS];
    let mut visited_count = 0;
    let mut count = 0;
    let mut sector = [0_u8; SECTOR_SIZE];

    loop {
        if visited_count == visited.len() {
            return Err(Error::ChainTooLong);
        }
        if visited[..visited_count].contains(&cluster) {
            return Err(Error::ChainLoop);
        }
        visited[visited_count] = cluster;
        visited_count += 1;

        let cluster_lba = volume.cluster_lba(cluster)?;
        for sector_index in 0..volume.sectors_per_cluster as u64 {
            device
                .read_sector(cluster_lba + sector_index, &mut sector)
                .map_err(Error::Block)?;
            for index in 0..DIRECTORY_ENTRIES_PER_SECTOR {
                let offset = index * DIRECTORY_ENTRY_SIZE;
                let first = sector[offset];
                if first == 0 {
                    return Ok(count);
                }
                let attributes = sector[offset + 11];
                if first == 0xe5 || attributes == 0x0f || attributes & 0x08 != 0 {
                    continue;
                }
                let slot = output.get_mut(count).ok_or(Error::DirectoryFull)?;
                let mut short_name = [0_u8; 11];
                short_name.copy_from_slice(&sector[offset..offset + 11]);
                let first_cluster = ((read_u16(&sector, offset + 20) as u32) << 16)
                    | read_u16(&sector, offset + 26) as u32;
                let size = read_u32(&sector, offset + 28);
                if first_cluster < 2 && size != 0 {
                    return Err(Error::CorruptDirectory);
                }
                *slot = Some(DirectoryEntry {
                    short_name,
                    first_cluster,
                    size,
                    attributes,
                });
                count += 1;
            }
        }
        cluster = match next_cluster(device, volume, cluster)? {
            ClusterLink::Next(next) => next,
            ClusterLink::End => return Ok(count),
        };
    }
}

enum DirectoryScan {
    Found(DirectoryEntry),
    Continue,
    End,
}

fn scan_directory_sector(
    sector: &[u8; SECTOR_SIZE],
    short_name: &[u8; 11],
) -> Result<DirectoryScan, Error> {
    for index in 0..DIRECTORY_ENTRIES_PER_SECTOR {
        let offset = index * DIRECTORY_ENTRY_SIZE;
        let first = sector[offset];
        if first == 0 {
            return Ok(DirectoryScan::End);
        }
        let attributes = sector[offset + 11];
        if first == 0xe5 || attributes == 0x0f || attributes & 0x08 != 0 {
            continue;
        }
        let mut entry_name = [0_u8; 11];
        entry_name.copy_from_slice(&sector[offset..offset + 11]);
        if &entry_name != short_name {
            continue;
        }

        let high_cluster = read_u16(sector, offset + 20) as u32;
        let low_cluster = read_u16(sector, offset + 26) as u32;
        let first_cluster = (high_cluster << 16) | low_cluster;
        if first_cluster < 2 && read_u32(sector, offset + 28) != 0 {
            return Err(Error::CorruptDirectory);
        }
        return Ok(DirectoryScan::Found(DirectoryEntry {
            short_name: entry_name,
            first_cluster,
            size: read_u32(sector, offset + 28),
            attributes,
        }));
    }
    Ok(DirectoryScan::Continue)
}
pub fn next_cluster(
    device: &mut impl BlockDevice,
    volume: Volume,
    cluster: u32,
) -> Result<ClusterLink, Error> {
    volume.cluster_lba(cluster)?;
    let fat_offset = (cluster as u64).checked_mul(4).ok_or(Error::CorruptChain)?;
    let fat_sector = volume
        .first_fat_sector
        .checked_add(fat_offset / SECTOR_SIZE as u64)
        .ok_or(Error::CorruptChain)?;
    if fat_sector >= volume.first_data_sector {
        return Err(Error::CorruptChain);
    }

    let mut sector = [0_u8; SECTOR_SIZE];
    device
        .read_sector(fat_sector, &mut sector)
        .map_err(Error::Block)?;
    let offset = (fat_offset % SECTOR_SIZE as u64) as usize;
    let value = read_u32(&sector, offset) & FAT32_ENTRY_MASK;
    if value >= FAT32_END_MIN {
        return Ok(ClusterLink::End);
    }
    if value < 2 || value == FAT32_BAD_CLUSTER {
        return Err(Error::CorruptChain);
    }
    volume.cluster_lba(value)?;
    Ok(ClusterLink::Next(value))
}

pub fn read_file(
    device: &mut impl BlockDevice,
    volume: Volume,
    entry: DirectoryEntry,
    buffer: &mut [u8],
) -> Result<usize, Error> {
    let file_size = entry.size as usize;
    let target = core::cmp::min(file_size, buffer.len());
    if target == 0 {
        return Ok(0);
    }

    let mut cluster = entry.first_cluster;
    let mut visited = [0_u32; MAX_READ_CLUSTERS];
    let mut visited_count = 0;
    let mut copied = 0;
    let mut sector = [0_u8; SECTOR_SIZE];

    while copied < target {
        if visited_count == visited.len() {
            return Err(Error::ChainTooLong);
        }
        if visited[..visited_count].contains(&cluster) {
            return Err(Error::ChainLoop);
        }
        visited[visited_count] = cluster;
        visited_count += 1;

        let cluster_lba = volume.cluster_lba(cluster)?;
        for sector_index in 0..volume.sectors_per_cluster as u64 {
            if copied == target {
                break;
            }
            device
                .read_sector(cluster_lba + sector_index, &mut sector)
                .map_err(Error::Block)?;
            let count = core::cmp::min(SECTOR_SIZE, target - copied);
            buffer[copied..copied + count].copy_from_slice(&sector[..count]);
            copied += count;
        }
        if copied == target {
            return Ok(copied);
        }

        cluster = match next_cluster(device, volume, cluster)? {
            ClusterLink::Next(next) => next,
            ClusterLink::End => return Err(Error::TruncatedFile),
        };
    }
    Ok(copied)
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

struct TestDisk {
    valid_signature: bool,
    cyclic_chain: bool,
}

impl TestDisk {
    const TOTAL_SECTORS: u64 = 70_000;
    const FAT_LBA: u64 = 32;
    const ROOT_LBA: u64 = 1_232;
    const FILE_LBA: u64 = 1_233;
}

impl BlockDevice for TestDisk {
    fn sector_count(&self) -> u64 {
        Self::TOTAL_SECTORS
    }

    fn read_sector(&mut self, lba: u64, sector: &mut [u8]) -> Result<(), BlockError> {
        if sector.len() != SECTOR_SIZE || lba >= Self::TOTAL_SECTORS {
            return Err(BlockError::OutOfBounds);
        }
        sector.fill(0);
        if lba == 0 {
            sector[11..13].copy_from_slice(&(SECTOR_SIZE as u16).to_le_bytes());
            sector[13] = 1;
            sector[14..16].copy_from_slice(&32_u16.to_le_bytes());
            sector[16] = 2;
            sector[32..36].copy_from_slice(&(Self::TOTAL_SECTORS as u32).to_le_bytes());
            sector[36..40].copy_from_slice(&600_u32.to_le_bytes());
            sector[44..48].copy_from_slice(&2_u32.to_le_bytes());
            if self.valid_signature {
                sector[510] = 0x55;
                sector[511] = 0xaa;
            }
        } else if lba == Self::FAT_LBA {
            sector[8..12].copy_from_slice(&5_u32.to_le_bytes());
            if self.cyclic_chain {
                sector[12..16].copy_from_slice(&3_u32.to_le_bytes());
            } else {
                sector[12..16].copy_from_slice(&4_u32.to_le_bytes());
                sector[16..20].copy_from_slice(&FAT32_END_MIN.to_le_bytes());
            }
            sector[20..24].copy_from_slice(&FAT32_END_MIN.to_le_bytes());
        } else if lba == Self::ROOT_LBA {
            for index in 0..DIRECTORY_ENTRIES_PER_SECTOR {
                sector[index * DIRECTORY_ENTRY_SIZE] = 0xe5;
            }
        } else if lba == Self::ROOT_LBA + 3 {
            sector[..11].copy_from_slice(b"KERNEL  BIN");
            sector[11] = 0x20;
            sector[26..28].copy_from_slice(&3_u16.to_le_bytes());
            let size = if self.cyclic_chain { 1024_u32 } else { 600_u32 };
            sector[28..32].copy_from_slice(&size.to_le_bytes());
        } else if lba == Self::FILE_LBA {
            sector.fill(b'A');
        } else if lba == Self::FILE_LBA + 1 {
            sector.fill(b'B');
        }
        Ok(())
    }

    fn write_sector(&mut self, _lba: u64, _sector: &[u8]) -> Result<(), BlockError> {
        Err(BlockError::ReadOnly)
    }
}

struct RootCycleDisk(TestDisk);

impl BlockDevice for RootCycleDisk {
    fn sector_count(&self) -> u64 {
        self.0.sector_count()
    }

    fn read_sector(&mut self, lba: u64, sector: &mut [u8]) -> Result<(), BlockError> {
        self.0.read_sector(lba, sector)?;
        if lba == TestDisk::FAT_LBA {
            sector[8..12].copy_from_slice(&2_u32.to_le_bytes());
        }
        Ok(())
    }

    fn write_sector(&mut self, lba: u64, sector: &[u8]) -> Result<(), BlockError> {
        self.0.write_sector(lba, sector)
    }
}

pub fn self_test() -> bool {
    let mut disk = TestDisk {
        valid_signature: true,
        cyclic_chain: false,
    };
    let Ok(volume) = mount(&mut disk) else {
        return false;
    };
    let Ok(entry) = find_root(&mut disk, volume, b"KERNEL  BIN") else {
        return false;
    };
    let mut entries = [None; 2];
    let listed = list_root(&mut disk, volume, &mut entries) == Ok(1)
        && entries[0].is_some_and(|listed| listed.short_name == entry.short_name);
    let listing_capacity_enforced =
        list_root(&mut disk, volume, &mut []) == Err(Error::DirectoryFull);
    let mut payload = [0_u8; 600];
    let valid = listed
        && listing_capacity_enforced
        && volume.total_sectors == TestDisk::TOTAL_SECTORS as u32
        && volume.fat_count == 2
        && volume.fat_size == 600
        && volume.cluster_lba(volume.root_cluster) == Ok(TestDisk::ROOT_LBA)
        && next_cluster(&mut disk, volume, volume.root_cluster) == Ok(ClusterLink::Next(5))
        && entry.short_name == *b"KERNEL  BIN"
        && entry.first_cluster == 3
        && entry.size == 600
        && entry.attributes == 0x20
        && next_cluster(&mut disk, volume, 3) == Ok(ClusterLink::Next(4))
        && next_cluster(&mut disk, volume, 4) == Ok(ClusterLink::End)
        && read_file(&mut disk, volume, entry, &mut payload) == Ok(payload.len())
        && payload[..SECTOR_SIZE].iter().all(|byte| *byte == b'A')
        && payload[SECTOR_SIZE..].iter().all(|byte| *byte == b'B')
        && find_root(&mut disk, volume, b"MISSING TXT") == Err(Error::NotFound);

    let mut invalid = TestDisk {
        valid_signature: false,
        cyclic_chain: false,
    };
    let invalid_rejected = mount(&mut invalid) == Err(Error::InvalidBootSector);

    let mut cyclic = TestDisk {
        valid_signature: true,
        cyclic_chain: true,
    };
    let Ok(cyclic_volume) = mount(&mut cyclic) else {
        return false;
    };
    let Ok(cyclic_entry) = find_root(&mut cyclic, cyclic_volume, b"KERNEL  BIN") else {
        return false;
    };
    let mut oversized = [0_u8; 1024];
    let cycle_rejected = read_file(&mut cyclic, cyclic_volume, cyclic_entry, &mut oversized)
        == Err(Error::ChainLoop);

    let mut root_cycle = RootCycleDisk(TestDisk {
        valid_signature: true,
        cyclic_chain: false,
    });
    let root_cycle_rejected = mount(&mut root_cycle).is_ok_and(|volume| {
        find_root(&mut root_cycle, volume, b"MISSING TXT") == Err(Error::ChainLoop)
    });

    valid && invalid_rejected && cycle_rejected && root_cycle_rejected
}
