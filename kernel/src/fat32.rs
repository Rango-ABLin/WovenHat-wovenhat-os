use crate::block::{BlockDevice, Error as BlockError, SECTOR_SIZE};

const FAT32_MIN_CLUSTERS: u32 = 65_525;
const DIRECTORY_ENTRY_SIZE: usize = 32;
const DIRECTORY_ENTRIES_PER_SECTOR: usize = SECTOR_SIZE / DIRECTORY_ENTRY_SIZE;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Block(BlockError),
    InvalidBootSector,
    UnsupportedGeometry,
    CorruptDirectory,
    NotFound,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Volume {
    pub total_sectors: u32,
    pub sectors_per_cluster: u8,
    pub fat_count: u8,
    pub fat_size: u32,
    pub root_cluster: u32,
    pub first_data_sector: u64,
    cluster_count: u32,
}

impl Volume {
    pub fn cluster_lba(&self, cluster: u32) -> Result<u64, Error> {
        if cluster < 2 || cluster >= self.cluster_count.saturating_add(2) {
            return Err(Error::CorruptDirectory);
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
        first_data_sector: first_data as u64,
        cluster_count,
    })
}

pub fn find_root(
    device: &mut impl BlockDevice,
    volume: Volume,
    short_name: &[u8; 11],
) -> Result<DirectoryEntry, Error> {
    let root_lba = volume.cluster_lba(volume.root_cluster)?;
    let mut sector = [0_u8; SECTOR_SIZE];
    device
        .read_sector(root_lba, &mut sector)
        .map_err(Error::Block)?;

    for index in 0..DIRECTORY_ENTRIES_PER_SECTOR {
        let offset = index * DIRECTORY_ENTRY_SIZE;
        let first = sector[offset];
        if first == 0 {
            break;
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

        let high_cluster = read_u16(&sector, offset + 20) as u32;
        let low_cluster = read_u16(&sector, offset + 26) as u32;
        let first_cluster = (high_cluster << 16) | low_cluster;
        if first_cluster < 2 && read_u32(&sector, offset + 28) != 0 {
            return Err(Error::CorruptDirectory);
        }
        return Ok(DirectoryEntry {
            short_name: entry_name,
            first_cluster,
            size: read_u32(&sector, offset + 28),
            attributes,
        });
    }
    Err(Error::NotFound)
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
}

impl TestDisk {
    const TOTAL_SECTORS: u64 = 70_000;
    const ROOT_LBA: u64 = 1_232;
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
        } else if lba == Self::ROOT_LBA {
            sector[..11].copy_from_slice(b"KERNEL  BIN");
            sector[11] = 0x20;
            sector[26..28].copy_from_slice(&3_u16.to_le_bytes());
            sector[28..32].copy_from_slice(&4096_u32.to_le_bytes());
        }
        Ok(())
    }

    fn write_sector(&mut self, _lba: u64, _sector: &[u8]) -> Result<(), BlockError> {
        Err(BlockError::ReadOnly)
    }
}

pub fn self_test() -> bool {
    let mut disk = TestDisk {
        valid_signature: true,
    };
    let Ok(volume) = mount(&mut disk) else {
        return false;
    };
    let Ok(entry) = find_root(&mut disk, volume, b"KERNEL  BIN") else {
        return false;
    };
    let valid = volume.total_sectors == TestDisk::TOTAL_SECTORS as u32
        && volume.fat_count == 2
        && volume.fat_size == 600
        && volume.cluster_lba(volume.root_cluster) == Ok(TestDisk::ROOT_LBA)
        && entry.short_name == *b"KERNEL  BIN"
        && entry.first_cluster == 3
        && entry.size == 4096
        && entry.attributes == 0x20
        && find_root(&mut disk, volume, b"MISSING TXT") == Err(Error::NotFound);

    let mut invalid = TestDisk {
        valid_signature: false,
    };
    valid && mount(&mut invalid) == Err(Error::InvalidBootSector)
}
