use crate::{ata, fat32, gpt, partition, vfs};

const DIRECTORY_ATTRIBUTE: u8 = 0x10;
const MAX_IMPORT_DEPTH: usize = 2;
const MAX_DIR_ENTRIES: usize = 32;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MountStatus {
    NoDevice,
    NotFat32,
    Mounted(usize),
    Failed,
}

pub fn mount_ata_root() -> MountStatus {
    ata::with_primary_master(|disk| {
        let direct = mount_device(disk);
        if direct != MountStatus::NotFat32 {
            return direct;
        }
        match partition::find_fat32(disk) {
            Ok(Some(partition)) => mount_partition(disk, partition),
            Ok(None) => match gpt::find_fat_partition(disk) {
                Ok(Some(partition)) => mount_partition(disk, partition),
                Ok(None) | Err(gpt::Error::MissingProtectiveMbr) => MountStatus::NotFat32,
                Err(_) => MountStatus::Failed,
            },
            Err(_) => MountStatus::Failed,
        }
    })
    .unwrap_or(MountStatus::NoDevice)
}

fn mount_partition(
    device: &mut impl crate::block::BlockDevice,
    partition: partition::Partition,
) -> MountStatus {
    let Ok(mut view) = partition::PartitionDevice::new(device, partition) else {
        return MountStatus::Failed;
    };
    mount_device(&mut view)
}

fn mount_device(device: &mut impl crate::block::BlockDevice) -> MountStatus {
    let volume = match fat32::mount(device) {
        Ok(volume) => volume,
        Err(fat32::Error::InvalidBootSector | fat32::Error::UnsupportedGeometry) => {
            return MountStatus::NotFat32;
        }
        Err(_) => return MountStatus::Failed,
    };

    let _ = vfs::mkdir("/mnt");

    match import_directory(device, volume, volume.root_cluster, "/mnt", 0) {
        Ok(count) => MountStatus::Mounted(count),
        Err(_) => MountStatus::Failed,
    }
}

fn import_directory(
    device: &mut impl crate::block::BlockDevice,
    volume: fat32::Volume,
    dir_cluster: u32,
    vfs_prefix: &str,
    depth: usize,
) -> Result<usize, fat32::Error> {
    let mut entries = [None; MAX_DIR_ENTRIES];
    let count = fat32::list_directory(device, volume, dir_cluster, &mut entries)?;
    let mut mounted = 0usize;

    for entry in entries[..count].iter().flatten().copied() {
        if entry.attributes & 0x08 != 0 {
            continue;
        }
        // Skip . and ..
        if entry.short_name[0] == b'.' {
            continue;
        }

        let mut name = [0u8; 12];
        let Some(name_len) = short_name_to_str(&entry.short_name, &mut name) else {
            continue;
        };
        let Ok(name_str) = core::str::from_utf8(&name[..name_len]) else {
            continue;
        };

        let mut path_buf = [0u8; 64];
        let Some(path_len) = join_path(vfs_prefix, name_str, &mut path_buf) else {
            continue;
        };
        let Ok(path) = core::str::from_utf8(&path_buf[..path_len]) else {
            continue;
        };

        let is_dir = entry.attributes & DIRECTORY_ATTRIBUTE != 0;
        if is_dir {
            match vfs::mkdir(path) {
                Ok(()) | Err(vfs::Error::AlreadyExists) => mounted += 1,
                Err(_) => return Err(fat32::Error::DirectoryFull),
            }
            if depth + 1 < MAX_IMPORT_DEPTH && entry.first_cluster >= 2 {
                mounted += import_directory(device, volume, entry.first_cluster, path, depth + 1)?;
            }
            continue;
        }

        if entry.size as usize > vfs::NODE_CAPACITY {
            continue;
        }
        let mut data = [0_u8; vfs::NODE_CAPACITY];
        let length = fat32::read_file(device, volume, entry, &mut data)?;
        match vfs::create_read_only(path, &data[..length]) {
            Ok(()) => mounted += 1,
            Err(vfs::Error::AlreadyExists) => {}
            Err(vfs::Error::Full) => return Err(fat32::Error::DirectoryFull),
            Err(_) => {}
        }
    }
    Ok(mounted)
}

fn join_path(prefix: &str, name: &str, out: &mut [u8; 64]) -> Option<usize> {
    let slash = !prefix.ends_with('/');
    let need = prefix.len() + usize::from(slash) + name.len();
    if need > out.len() {
        return None;
    }
    out[..prefix.len()].copy_from_slice(prefix.as_bytes());
    let mut len = prefix.len();
    if slash {
        out[len] = b'/';
        len += 1;
    }
    out[len..len + name.len()].copy_from_slice(name.as_bytes());
    Some(len + name.len())
}

fn short_name_to_str(short_name: &[u8; 11], out: &mut [u8; 12]) -> Option<usize> {
    let mut len = 0usize;
    for byte in short_name[..8].iter().copied().take_while(|b| *b != b' ') {
        out[len] = to_lower(byte)?;
        len += 1;
    }
    if len == 0 {
        return None;
    }
    if short_name[8..].iter().any(|b| *b != b' ') {
        out[len] = b'.';
        len += 1;
        for byte in short_name[8..].iter().copied().take_while(|b| *b != b' ') {
            out[len] = to_lower(byte)?;
            len += 1;
        }
    }
    Some(len)
}

fn to_lower(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte + (b'a' - b'A')),
        b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' => Some(byte),
        _ => None,
    }
}

pub fn self_test() -> bool {
    let mut name = [0u8; 12];
    let nlen = short_name_to_str(b"KERNEL  BIN", &mut name).unwrap_or(0);
    let kernel_ok = &name[..nlen] == b"kernel.bin";
    let nlen = short_name_to_str(b"README     ", &mut name).unwrap_or(0);
    let readme_ok = &name[..nlen] == b"readme";
    let bad_ok = short_name_to_str(b"BAD?    TXT", &mut name).is_none();
    let encode_ok = fat32::encode_short_name("kernel.bin") == Some(*b"KERNEL  BIN")
        && fat32::encode_short_name("readme") == Some(*b"README     ");
    kernel_ok && readme_ok && bad_ok && encode_ok
}

/// Ensure a path under `/mnt` exists in the VFS by resolving it on the live ATA volume.
///
/// If the path is already present, returns success immediately. Otherwise opens the
/// primary ATA master, mounts FAT32 (superfloppy / MBR / GPT), resolves the relative
/// path, and imports the file or directory into the VFS.
pub fn ensure_path(path: &str) -> Result<(), EnsureError> {
    if vfs::stat(path).is_ok() {
        return Ok(());
    }
    if path == "/mnt" {
        return match vfs::mkdir("/mnt") {
            Ok(()) | Err(vfs::Error::AlreadyExists) => Ok(()),
            Err(_) => Err(EnsureError::Vfs),
        };
    }
    if !path.starts_with("/mnt/") {
        return Err(EnsureError::NotUnderMount);
    }

    let relative = &path[5..]; // strip "/mnt/"
    ata::with_primary_master(|disk| ensure_on_disk(disk, relative, path))
        .unwrap_or(Err(EnsureError::NoDevice))
}

fn ensure_on_disk(
    device: &mut impl crate::block::BlockDevice,
    relative: &str,
    full_path: &str,
) -> Result<(), EnsureError> {
    // Prefer superfloppy at LBA 0.
    match fat32::mount(device) {
        Ok(volume) => return import_resolved(device, volume, relative, full_path),
        Err(fat32::Error::InvalidBootSector | fat32::Error::UnsupportedGeometry) => {}
        Err(_) => return Err(EnsureError::Failed),
    }

    if let Ok(Some(part)) = partition::find_fat32(device) {
        let mut view =
            partition::PartitionDevice::new(device, part).map_err(|_| EnsureError::Failed)?;
        let volume = fat32::mount(&mut view).map_err(map_fat_err)?;
        return import_resolved(&mut view, volume, relative, full_path);
    }

    match gpt::find_fat_partition(device) {
        Ok(Some(part)) => {
            let mut view =
                partition::PartitionDevice::new(device, part).map_err(|_| EnsureError::Failed)?;
            let volume = fat32::mount(&mut view).map_err(map_fat_err)?;
            import_resolved(&mut view, volume, relative, full_path)
        }
        Ok(None) | Err(gpt::Error::MissingProtectiveMbr) => Err(EnsureError::NotFat32),
        Err(_) => Err(EnsureError::Failed),
    }
}

fn import_resolved(
    device: &mut impl crate::block::BlockDevice,
    volume: fat32::Volume,
    relative: &str,
    full_path: &str,
) -> Result<(), EnsureError> {
    let entry = fat32::resolve_path(device, volume, relative).map_err(map_fat_err)?;
    let is_dir = entry.attributes & DIRECTORY_ATTRIBUTE != 0;

    create_parent_dirs(full_path)?;

    if is_dir {
        return match vfs::mkdir(full_path) {
            Ok(()) | Err(vfs::Error::AlreadyExists) => Ok(()),
            Err(_) => Err(EnsureError::Vfs),
        };
    }
    if entry.size as usize > vfs::NODE_CAPACITY {
        return Err(EnsureError::TooLarge);
    }
    let mut data = [0_u8; vfs::NODE_CAPACITY];
    let length = fat32::read_file(device, volume, entry, &mut data).map_err(map_fat_err)?;
    match vfs::create_read_only(full_path, &data[..length]) {
        Ok(()) | Err(vfs::Error::AlreadyExists) => Ok(()),
        Err(_) => Err(EnsureError::Vfs),
    }
}

fn create_parent_dirs(path: &str) -> Result<(), EnsureError> {
    let bytes = path.as_bytes();
    if !path.starts_with('/') {
        return Err(EnsureError::InvalidPath);
    }
    let mut i = 1;
    while i < bytes.len() {
        while i < bytes.len() && bytes[i] != b'/' {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let prefix = core::str::from_utf8(&bytes[..i]).map_err(|_| EnsureError::InvalidPath)?;
        match vfs::mkdir(prefix) {
            Ok(()) | Err(vfs::Error::AlreadyExists) => {}
            Err(_) => return Err(EnsureError::Vfs),
        }
        i += 1;
    }
    Ok(())
}

fn map_fat_err(err: fat32::Error) -> EnsureError {
    match err {
        fat32::Error::NotFound => EnsureError::NotFound,
        fat32::Error::InvalidBootSector | fat32::Error::UnsupportedGeometry => {
            EnsureError::NotFat32
        }
        _ => EnsureError::Failed,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EnsureError {
    NoDevice,
    NotFat32,
    NotFound,
    NotUnderMount,
    InvalidPath,
    TooLarge,
    Vfs,
    Failed,
}
