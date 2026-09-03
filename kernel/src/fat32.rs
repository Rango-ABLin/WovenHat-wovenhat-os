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
    NoSpace,
    NameTooLong,
    WriteFailed,
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

/// Look up a short name in the volume root directory.
pub fn find_root(
    device: &mut impl BlockDevice,
    volume: Volume,
    short_name: &[u8; 11],
) -> Result<DirectoryEntry, Error> {
    find_in_directory(device, volume, volume.root_cluster, short_name)
}

/// Look up a short name in any directory starting at `dir_cluster`.
pub fn find_in_directory(
    device: &mut impl BlockDevice,
    volume: Volume,
    dir_cluster: u32,
    short_name: &[u8; 11],
) -> Result<DirectoryEntry, Error> {
    let mut cluster = dir_cluster;
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

/// List entries in the volume root directory.
pub fn list_root(
    device: &mut impl BlockDevice,
    volume: Volume,
    output: &mut [Option<DirectoryEntry>],
) -> Result<usize, Error> {
    list_directory(device, volume, volume.root_cluster, output)
}

/// List entries in any directory starting at `dir_cluster`.
pub fn list_directory(
    device: &mut impl BlockDevice,
    volume: Volume,
    dir_cluster: u32,
    output: &mut [Option<DirectoryEntry>],
) -> Result<usize, Error> {
    let mut cluster = dir_cluster;
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

/// Resolve a Unix-style absolute path of 8.3 components against a FAT32 volume.
///
/// Example: `"/BIN/SH"` or `"BIN/SH"` (leading slash optional). Each component is
/// converted to a FAT 8.3 short name. Returns the final directory entry.
pub fn resolve_path(
    device: &mut impl BlockDevice,
    volume: Volume,
    path: &str,
) -> Result<DirectoryEntry, Error> {
    let path = path.trim_start_matches('/');
    if path.is_empty() {
        return Err(Error::NotFound);
    }

    // Collect up to 8 path components without allocation.
    let mut parts = [""; 8];
    let mut part_count = 0usize;
    for component in path.split('/') {
        if component.is_empty() || component == "." {
            continue;
        }
        if component == ".." || part_count == parts.len() {
            return Err(Error::NotFound);
        }
        parts[part_count] = component;
        part_count += 1;
    }
    if part_count == 0 {
        return Err(Error::NotFound);
    }

    let mut cluster = volume.root_cluster;
    let mut current = None;
    for (i, component) in parts[..part_count].iter().enumerate() {
        let short = encode_short_name(component).ok_or(Error::NotFound)?;
        let entry = find_in_directory(device, volume, cluster, &short)?;
        let is_last = i + 1 == part_count;
        if !is_last {
            if entry.attributes & 0x10 == 0 || entry.first_cluster < 2 {
                return Err(Error::NotFound);
            }
            cluster = entry.first_cluster;
        }
        current = Some(entry);
    }
    current.ok_or(Error::NotFound)
}

/// Encode a path component into a FAT 8.3 short name (space-padded).
pub fn encode_short_name(name: &str) -> Option<[u8; 11]> {
    if name.is_empty() || name.len() > 12 {
        return None;
    }
    let mut out = [b' '; 11];
    let (base, ext) = match name.find('.') {
        Some(pos) => (&name[..pos], &name[pos + 1..]),
        None => (name, ""),
    };
    if base.is_empty() || base.len() > 8 || ext.len() > 3 {
        return None;
    }
    if base.contains('.') || ext.contains('.') {
        return None;
    }
    for (i, byte) in base.bytes().enumerate() {
        out[i] = to_fat_char(byte)?;
    }
    for (i, byte) in ext.bytes().enumerate() {
        out[8 + i] = to_fat_char(byte)?;
    }
    Some(out)
}

fn to_fat_char(byte: u8) -> Option<u8> {
    match byte {
        b'a'..=b'z' => Some(byte - (b'a' - b'A')),
        b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'-' => Some(byte),
        _ => None,
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

fn write_u16(bytes: &mut [u8], offset: usize, value: u16) {
    let le = value.to_le_bytes();
    bytes[offset] = le[0];
    bytes[offset + 1] = le[1];
}

fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
    let le = value.to_le_bytes();
    bytes[offset] = le[0];
    bytes[offset + 1] = le[1];
    bytes[offset + 2] = le[2];
    bytes[offset + 3] = le[3];
}

fn read_fat_entry(
    device: &mut impl BlockDevice,
    volume: Volume,
    cluster: u32,
) -> Result<u32, Error> {
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
    Ok(read_u32(&sector, offset) & FAT32_ENTRY_MASK)
}

fn write_fat_entry(
    device: &mut impl BlockDevice,
    volume: Volume,
    cluster: u32,
    value: u32,
) -> Result<(), Error> {
    let fat_offset = (cluster as u64).checked_mul(4).ok_or(Error::CorruptChain)?;
    let sector_index = fat_offset / SECTOR_SIZE as u64;
    let offset = (fat_offset % SECTOR_SIZE as u64) as usize;
    let masked = value & FAT32_ENTRY_MASK;

    for fat in 0..volume.fat_count as u64 {
        let fat_sector = volume
            .first_fat_sector
            .checked_add(fat * volume.fat_size as u64)
            .and_then(|base| base.checked_add(sector_index))
            .ok_or(Error::CorruptChain)?;
        if fat_sector >= volume.first_data_sector {
            return Err(Error::CorruptChain);
        }
        let mut sector = [0_u8; SECTOR_SIZE];
        device
            .read_sector(fat_sector, &mut sector)
            .map_err(Error::Block)?;
        // Preserve high nibble reserved bits if any were set.
        let existing = read_u32(&sector, offset);
        let new_value = (existing & !FAT32_ENTRY_MASK) | masked;
        write_u32(&mut sector, offset, new_value);
        device
            .write_sector(fat_sector, &sector)
            .map_err(Error::Block)?;
    }
    Ok(())
}

fn allocate_cluster(device: &mut impl BlockDevice, volume: Volume) -> Result<u32, Error> {
    // Clusters 0 and 1 are reserved; search from 2.
    let max = volume.cluster_count.saturating_add(2);
    for cluster in 2..max {
        let entry = read_fat_entry(device, volume, cluster)?;
        if entry == 0 {
            write_fat_entry(device, volume, cluster, FAT32_END_MIN)?;
            return Ok(cluster);
        }
    }
    Err(Error::NoSpace)
}

/// Create or overwrite a file in the volume root directory.
///
/// `name` must be a valid 8.3 short name component (e.g. `"FOO.TXT"` or `"readme"`).
/// Only the root directory is supported; the directory is not extended if full.
/// Data larger than one cluster chain that fits in available free clusters is accepted
/// up to the caller's buffer; allocation is sequential.
pub fn create_root_file(
    device: &mut impl BlockDevice,
    volume: Volume,
    name: &str,
    data: &[u8],
) -> Result<(), Error> {
    let short = encode_short_name(name).ok_or(Error::NameTooLong)?;
    if data.len() > u32::MAX as usize {
        return Err(Error::NoSpace);
    }

    // Locate an existing entry to overwrite, or a free/deleted slot.
    let mut slot_cluster = volume.root_cluster;
    let mut slot_sector_lba: u64 = 0;
    let mut slot_offset = 0usize;
    let mut found_slot = false;
    let mut existing_first: Option<u32> = None;
    let mut visited = [0_u32; MAX_READ_CLUSTERS];
    let mut visited_count = 0;
    let mut sector = [0_u8; SECTOR_SIZE];
    let mut cluster = volume.root_cluster;

    'search: loop {
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
            let lba = cluster_lba + sector_index;
            device.read_sector(lba, &mut sector).map_err(Error::Block)?;
            for index in 0..DIRECTORY_ENTRIES_PER_SECTOR {
                let offset = index * DIRECTORY_ENTRY_SIZE;
                let first = sector[offset];
                if first == 0 {
                    // End of directory — free slot.
                    slot_cluster = cluster;
                    slot_sector_lba = lba;
                    slot_offset = offset;
                    found_slot = true;
                    break 'search;
                }
                let attributes = sector[offset + 11];
                if attributes == 0x0f || attributes & 0x08 != 0 {
                    continue;
                }
                let mut entry_name = [0_u8; 11];
                entry_name.copy_from_slice(&sector[offset..offset + 11]);
                if entry_name == short {
                    // Overwrite existing file entry.
                    let high = read_u16(&sector, offset + 20) as u32;
                    let low = read_u16(&sector, offset + 26) as u32;
                    existing_first = Some((high << 16) | low);
                    slot_cluster = cluster;
                    slot_sector_lba = lba;
                    slot_offset = offset;
                    found_slot = true;
                    break 'search;
                }
                if first == 0xe5 && !found_slot {
                    // Remember first deleted slot as candidate.
                    slot_cluster = cluster;
                    slot_sector_lba = lba;
                    slot_offset = offset;
                    found_slot = true;
                }
            }
        }
        match next_cluster(device, volume, cluster)? {
            ClusterLink::Next(next) => cluster = next,
            ClusterLink::End => {
                if found_slot {
                    break;
                }
                // Root directory full and not extended.
                return Err(Error::DirectoryFull);
            }
        }
    }

    if !found_slot {
        return Err(Error::DirectoryFull);
    }

    // Free previous cluster chain if overwriting.
    if let Some(mut c) = existing_first {
        if c >= 2 {
            let mut guard = 0;
            while guard < MAX_READ_CLUSTERS {
                guard += 1;
                let next = match next_cluster(device, volume, c)? {
                    ClusterLink::Next(n) => n,
                    ClusterLink::End => {
                        write_fat_entry(device, volume, c, 0)?;
                        break;
                    }
                };
                write_fat_entry(device, volume, c, 0)?;
                c = next;
            }
        }
    }

    // Allocate clusters for the new data.
    let bytes_per_cluster = volume.sectors_per_cluster as usize * SECTOR_SIZE;
    let needed = if data.is_empty() {
        0
    } else {
        (data.len() + bytes_per_cluster - 1) / bytes_per_cluster
    };
    let mut first_cluster = 0u32;
    let mut prev = 0u32;
    for i in 0..needed {
        let c = allocate_cluster(device, volume)?;
        if i == 0 {
            first_cluster = c;
        } else {
            write_fat_entry(device, volume, prev, c)?;
        }
        prev = c;
    }
    if needed > 0 {
        write_fat_entry(device, volume, prev, FAT32_END_MIN)?;
    }

    // Write file data.
    if needed > 0 {
        let mut remaining = data;
        let mut c = first_cluster;
        for _ in 0..needed {
            let cluster_lba = volume.cluster_lba(c)?;
            for s in 0..volume.sectors_per_cluster as u64 {
                let mut buf = [0_u8; SECTOR_SIZE];
                let take = core::cmp::min(SECTOR_SIZE, remaining.len());
                if take > 0 {
                    buf[..take].copy_from_slice(&remaining[..take]);
                    remaining = &remaining[take..];
                }
                device
                    .write_sector(cluster_lba + s, &buf)
                    .map_err(Error::Block)?;
            }
            if remaining.is_empty() {
                break;
            }
            c = match next_cluster(device, volume, c)? {
                ClusterLink::Next(n) => n,
                ClusterLink::End => break,
            };
        }
    }

    // Write / update the directory entry.
    device
        .read_sector(slot_sector_lba, &mut sector)
        .map_err(Error::Block)?;
    sector[slot_offset..slot_offset + 11].copy_from_slice(&short);
    sector[slot_offset + 11] = 0x20; // archive attribute
    // zero reserved / timestamps for simplicity
    for i in 12..26 {
        if i != 20 && i != 21 {
            sector[slot_offset + i] = 0;
        }
    }
    write_u16(&mut sector, slot_offset + 20, (first_cluster >> 16) as u16);
    write_u16(&mut sector, slot_offset + 26, (first_cluster & 0xffff) as u16);
    write_u32(&mut sector, slot_offset + 28, data.len() as u32);
    device
        .write_sector(slot_sector_lba, &sector)
        .map_err(Error::Block)?;

    let _ = slot_cluster; // silence unused when not extending
    Ok(())
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
