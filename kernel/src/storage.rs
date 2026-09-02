use crate::{ata, fat32, partition, vfs};

const MAX_ROOT_FILES: usize = 8;
const DIRECTORY_ATTRIBUTE: u8 = 0x10;

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
            Ok(Some(partition)) => {
                let Ok(mut view) = partition::PartitionDevice::new(disk, partition) else {
                    return MountStatus::Failed;
                };
                mount_device(&mut view)
            }
            Ok(None) => MountStatus::NotFat32,
            Err(_) => MountStatus::Failed,
        }
    })
    .unwrap_or(MountStatus::NoDevice)
}
fn mount_device(device: &mut impl crate::block::BlockDevice) -> MountStatus {
    let volume = match fat32::mount(device) {
        Ok(volume) => volume,
        Err(fat32::Error::InvalidBootSector | fat32::Error::UnsupportedGeometry) => {
            return MountStatus::NotFat32;
        }
        Err(_) => return MountStatus::Failed,
    };

    let mut entries = [None; MAX_ROOT_FILES];
    let count = match fat32::list_root(device, volume, &mut entries) {
        Ok(count) => count,
        Err(_) => return MountStatus::Failed,
    };
    let mut mounted = 0;
    for entry in entries[..count].iter().flatten().copied() {
        if entry.attributes & DIRECTORY_ATTRIBUTE != 0 || entry.size as usize > vfs::NODE_CAPACITY {
            continue;
        }
        let Some((path, path_length)) = mount_path(entry.short_name) else {
            continue;
        };
        let Ok(path) = core::str::from_utf8(&path[..path_length]) else {
            return MountStatus::Failed;
        };
        let mut data = [0_u8; vfs::NODE_CAPACITY];
        let length = match fat32::read_file(device, volume, entry, &mut data) {
            Ok(length) => length,
            Err(_) => return MountStatus::Failed,
        };
        if vfs::create_read_only(path, &data[..length]).is_err() {
            return MountStatus::Failed;
        }
        mounted += 1;
    }
    MountStatus::Mounted(mounted)
}
fn mount_path(short_name: [u8; 11]) -> Option<([u8; 24], usize)> {
    let mut path = [0_u8; 24];
    path[..5].copy_from_slice(b"/mnt/");
    let mut length = 5;
    for byte in short_name[..8]
        .iter()
        .copied()
        .take_while(|byte| *byte != b' ')
    {
        path[length] = normalize(byte)?;
        length += 1;
    }
    if length == 5 {
        return None;
    }
    if short_name[8..].iter().any(|byte| *byte != b' ') {
        path[length] = b'.';
        length += 1;
        for byte in short_name[8..]
            .iter()
            .copied()
            .take_while(|byte| *byte != b' ')
        {
            path[length] = normalize(byte)?;
            length += 1;
        }
    }
    Some((path, length))
}

fn normalize(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte + (b'a' - b'A')),
        b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' => Some(byte),
        _ => None,
    }
}

pub fn self_test() -> bool {
    let Some((path, length)) = mount_path(*b"KERNEL  BIN") else {
        return false;
    };
    &path[..length] == b"/mnt/kernel.bin"
        && mount_path(*b"README     ")
            .is_some_and(|(path, length)| &path[..length] == b"/mnt/readme")
        && mount_path(*b"BAD?    TXT").is_none()
}
