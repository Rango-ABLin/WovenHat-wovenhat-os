use spin::Mutex;

pub const NODE_CAPACITY: usize = 8192;
const PATH_CAPACITY: usize = 64;
const MAX_NODES: usize = 16;

#[derive(Clone, Copy)]
struct Node {
    path: [u8; PATH_CAPACITY],
    path_length: usize,
    data: [u8; NODE_CAPACITY],
    length: usize,
    writable: bool,
    occupied: bool,
}

impl Node {
    const fn empty() -> Self {
        Self {
            path: [0; PATH_CAPACITY],
            path_length: 0,
            data: [0; NODE_CAPACITY],
            length: 0,
            writable: false,
            occupied: false,
        }
    }

    const fn with_data(path: &[u8], data: &[u8], writable: bool) -> Self {
        let mut node = Self::empty();
        let mut index = 0;
        while index < path.len() {
            node.path[index] = path[index];
            index += 1;
        }
        index = 0;
        while index < data.len() {
            node.data[index] = data[index];
            index += 1;
        }
        node.path_length = path.len();
        node.length = data.len();
        node.writable = writable;
        node.occupied = true;
        node
    }

    fn matches(&self, path: &str) -> bool {
        self.occupied && &self.path[..self.path_length] == path.as_bytes()
    }
}

struct Registry {
    nodes: [Node; MAX_NODES],
}

impl Registry {
    const fn boot() -> Self {
        let mut nodes = [Node::empty(); MAX_NODES];
        nodes[0] = Node::with_data(b"/etc/motd", b"Welcome to WovenHat OS.\n", false);
        nodes[1] = Node::with_data(b"/etc/version", b"WovenHat kernel 0.0.7\n", false);
        nodes[2] = Node::with_data(b"/tmp/vfs-self-test", b"", true);
        Self { nodes }
    }

    const fn empty() -> Self {
        Self {
            nodes: [Node::empty(); MAX_NODES],
        }
    }

    fn insert(&mut self, path: &str, data: &[u8], writable: bool) -> Result<usize, Error> {
        if path.is_empty() || !path.starts_with('/') || path.len() > PATH_CAPACITY {
            return Err(Error::InvalidPath);
        }
        if data.len() > NODE_CAPACITY {
            return Err(Error::Full);
        }
        if self.nodes.iter().any(|node| node.matches(path)) {
            return Err(Error::AlreadyExists);
        }
        let index = self
            .nodes
            .iter()
            .position(|node| !node.occupied)
            .ok_or(Error::Full)?;
        let node = &mut self.nodes[index];
        node.path[..path.len()].copy_from_slice(path.as_bytes());
        node.path_length = path.len();
        node.data[..data.len()].copy_from_slice(data);
        node.length = data.len();
        node.writable = writable;
        node.occupied = true;
        Ok(index)
    }

    fn count(&self) -> usize {
        self.nodes.iter().filter(|node| node.occupied).count()
    }
}

static REGISTRY: Mutex<Registry> = Mutex::new(Registry::boot());

#[derive(Clone, Copy)]
pub struct OpenFile {
    node: usize,
    offset: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Error {
    NotFound,
    InvalidDescriptor,
    InvalidPath,
    AlreadyExists,
    ReadOnly,
    Full,
}

pub fn create_read_only(path: &str, data: &[u8]) -> Result<(), Error> {
    REGISTRY.lock().insert(path, data, false).map(|_| ())
}

pub fn open(path: &str) -> Result<OpenFile, Error> {
    let registry = REGISTRY.lock();
    registry
        .nodes
        .iter()
        .position(|node| node.matches(path))
        .map(|node| OpenFile { node, offset: 0 })
        .ok_or(Error::NotFound)
}

pub fn read(file: &mut OpenFile, buffer: &mut [u8]) -> Result<usize, Error> {
    let registry = REGISTRY.lock();
    let node = registry
        .nodes
        .get(file.node)
        .filter(|node| node.occupied)
        .ok_or(Error::InvalidDescriptor)?;
    if file.offset > node.length {
        return Err(Error::InvalidDescriptor);
    }
    let count = core::cmp::min(node.length - file.offset, buffer.len());
    buffer[..count].copy_from_slice(&node.data[file.offset..file.offset + count]);
    file.offset += count;
    Ok(count)
}

pub fn read_all(path: &str, buffer: &mut [u8]) -> Result<usize, Error> {
    let registry = REGISTRY.lock();
    let node = registry
        .nodes
        .iter()
        .find(|node| node.matches(path))
        .ok_or(Error::NotFound)?;
    if node.length > buffer.len() {
        return Err(Error::Full);
    }
    buffer[..node.length].copy_from_slice(&node.data[..node.length]);
    Ok(node.length)
}
pub fn write(file: &mut OpenFile, buffer: &[u8]) -> Result<usize, Error> {
    let mut registry = REGISTRY.lock();
    let node = registry
        .nodes
        .get_mut(file.node)
        .filter(|node| node.occupied)
        .ok_or(Error::InvalidDescriptor)?;
    if !node.writable {
        return Err(Error::ReadOnly);
    }
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

pub fn node_count() -> usize {
    REGISTRY.lock().count()
}

pub fn self_test() -> bool {
    let mut scratch = Registry::empty();
    let inserted = scratch.insert("/mnt/test.txt", b"mounted", false) == Ok(0);
    let duplicate = scratch.insert("/mnt/test.txt", b"again", false) == Err(Error::AlreadyExists);
    let invalid_path = scratch.insert("relative", b"bad", false) == Err(Error::InvalidPath);

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
    let round_trip = read(&mut reader, &mut buffer) == Ok(payload.len()) && buffer == *payload;

    let Ok(mut protected) = open("/etc/motd") else {
        return false;
    };
    let mut complete = [0; 32];
    inserted
        && duplicate
        && invalid_path
        && round_trip
        && write(&mut protected, b"x") == Err(Error::ReadOnly)
        && read_all("/etc/motd", &mut complete) == Ok(24)
        && &complete[..24] == b"Welcome to WovenHat OS.\n"
        && node_count() == 3
}
