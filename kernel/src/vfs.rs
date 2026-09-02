use spin::Mutex;

const NODE_CAPACITY: usize = 256;

struct Node {
    path: &'static str,
    data: [u8; NODE_CAPACITY],
    length: usize,
}

static NODES: Mutex<[Node; 2]> = Mutex::new([
    Node {
        path: "/etc/motd",
        data: padded(b"Welcome to WovenHat OS.\n"),
        length: 24,
    },
    Node {
        path: "/etc/version",
        data: padded(b"WovenHat kernel 0.0.7\n"),
        length: 22,
    },
    Node {
        path: "/tmp/vfs-self-test",
        data: padded(b""),
        length: 0,
    },
]);

#[derive(Clone, Copy)]
pub struct OpenFile {
    node: usize,
    offset: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Error {
    NotFound,
    InvalidDescriptor,
    Full,
}

pub fn open(path: &str) -> Result<OpenFile, Error> {
    let nodes = NODES.lock();
    nodes
        .iter()
        .position(|node| node.path == path)
        .map(|node| OpenFile { node, offset: 0 })
        .ok_or(Error::NotFound)
}

pub fn read(file: &mut OpenFile, buffer: &mut [u8]) -> Result<usize, Error> {
    let nodes = NODES.lock();
    let node = nodes.get(file.node).ok_or(Error::InvalidDescriptor)?;
    if file.offset > node.length {
        return Err(Error::InvalidDescriptor);
    }
    let count = core::cmp::min(node.length - file.offset, buffer.len());
    buffer[..count].copy_from_slice(&node.data[file.offset..file.offset + count]);
    file.offset += count;
    Ok(count)
}

pub fn write(file: &mut OpenFile, buffer: &[u8]) -> Result<usize, Error> {
    let mut nodes = NODES.lock();
    let node = nodes.get_mut(file.node).ok_or(Error::InvalidDescriptor)?;
    if file.offset > node.length {
        return Err(Error::InvalidDescriptor);
    }
    let count = core::cmp::min(buffer.len(), NODE_CAPACITY - file.offset);
    node.data[file.offset..file.offset + count].copy_from_slice(&buffer[..count]);
    file.offset += count;
    node.length = core::cmp::max(node.length, file.offset);
    if count < buffer.len() {
        return Err(Error::Full);
    }
    Ok(count)
}

pub const fn node_count() -> usize {
    3
}

pub fn self_test() -> bool {
    let Ok(mut writer) = open("/tmp/vfs-self-test") else {
        return false;
    };
    let payload = b"wovenhat-vfs";
    if write(&mut writer, payload) != Ok(payload.len()) {
        return false;
    }

    let Ok(mut reader) = open("/tmp/vfs-self-test") else {
        return false;
    };
    let mut buffer = [0; 12];
    read(&mut reader, &mut buffer) == Ok(payload.len()) && buffer == *payload
}

const fn padded(bytes: &[u8]) -> [u8; NODE_CAPACITY] {
    let mut data = [0; NODE_CAPACITY];
    let mut index = 0;
    while index < bytes.len() {
        data[index] = bytes[index];
        index += 1;
    }
    data
}
