#[derive(Clone, Copy)]
struct Node {
    path: &'static str,
    data: &'static [u8],
}

const NODES: &[Node] = &[
    Node {
        path: "/etc/motd",
        data: b"Welcome to WovenHat OS.\n",
    },
    Node {
        path: "/etc/version",
        data: b"WovenHat kernel 0.0.7\n",
    },
];

#[derive(Clone, Copy)]
pub struct OpenFile {
    node: usize,
    offset: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Error {
    NotFound,
    InvalidDescriptor,
}

pub fn open(path: &str) -> Result<OpenFile, Error> {
    NODES
        .iter()
        .position(|node| node.path == path)
        .map(|node| OpenFile { node, offset: 0 })
        .ok_or(Error::NotFound)
}

pub fn read(file: &mut OpenFile, buffer: &mut [u8]) -> Result<usize, Error> {
    let node = NODES.get(file.node).ok_or(Error::InvalidDescriptor)?;
    let remaining = node
        .data
        .get(file.offset..)
        .ok_or(Error::InvalidDescriptor)?;
    let count = core::cmp::min(remaining.len(), buffer.len());
    buffer[..count].copy_from_slice(&remaining[..count]);
    file.offset += count;
    Ok(count)
}

pub const fn node_count() -> usize {
    NODES.len()
}
