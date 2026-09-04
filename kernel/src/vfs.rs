use spin::Mutex;

use crate::config::{
    MAX_OPEN_FILES, MAX_PATH_SIZE as PATH_CAPACITY, MAX_VFS_NODES as MAX_NODES, VFS_NODE_CAPACITY,
};

pub const NODE_CAPACITY: usize = VFS_NODE_CAPACITY;

/// Maximum bytes returned for a single directory entry name (excluding NUL).
pub const MAX_DIR_NAME: usize = 64;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    File,
    Directory,
}

#[derive(Clone, Copy)]
struct Node {
    path: [u8; PATH_CAPACITY],
    path_length: usize,
    data: [u8; NODE_CAPACITY],
    length: usize,
    writable: bool,
    kind: NodeKind,
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
            kind: NodeKind::File,
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
        node.kind = NodeKind::File;
        node.occupied = true;
        node
    }

    const fn directory(path: &[u8]) -> Self {
        let mut node = Self::empty();
        let mut index = 0;
        while index < path.len() {
            node.path[index] = path[index];
            index += 1;
        }
        node.path_length = path.len();
        node.kind = NodeKind::Directory;
        node.writable = true;
        node.occupied = true;
        node
    }

    fn matches(&self, path: &str) -> bool {
        self.occupied && &self.path[..self.path_length] == path.as_bytes()
    }

    fn path_str(&self) -> &str {
        core::str::from_utf8(&self.path[..self.path_length]).unwrap_or("")
    }
}

/// Metadata returned by `stat`.
#[derive(Clone, Copy)]
pub struct Stat {
    pub kind: NodeKind,
    pub size: usize,
    pub writable: bool,
}

/// One directory entry returned by `readdir`.
#[derive(Clone, Copy)]
pub struct DirEntry {
    pub name: [u8; MAX_DIR_NAME],
    pub name_length: usize,
    pub kind: NodeKind,
}

impl DirEntry {
    pub fn name_str(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_length]).unwrap_or("")
    }
}

struct Registry {
    nodes: [Node; MAX_NODES],
}

impl Registry {
    const fn boot() -> Self {
        let mut nodes = [Node::empty(); MAX_NODES];
        // Directories first so path walking can find parents.
        nodes[0] = Node::directory(b"/");
        nodes[1] = Node::directory(b"/etc");
        nodes[2] = Node::directory(b"/tmp");
        nodes[3] = Node::directory(b"/mnt");
        nodes[4] = Node::directory(b"/bin");
        nodes[5] = Node::with_data(b"/etc/motd", b"Welcome to WovenHat OS.\n", false);
        nodes[6] = Node::with_data(b"/etc/version", b"WovenHat kernel 0.2.0 Stage 4\n", false);
        nodes[7] = Node::with_data(b"/tmp/vfs-self-test", b"", true);
        Self { nodes }
    }

    const fn empty() -> Self {
        Self {
            nodes: [Node::empty(); MAX_NODES],
        }
    }

    fn insert(&mut self, path: &str, data: &[u8], writable: bool) -> Result<usize, Error> {
        validate_absolute_path(path)?;
        if data.len() > NODE_CAPACITY {
            return Err(Error::Full);
        }
        if self.nodes.iter().any(|node| node.matches(path)) {
            return Err(Error::AlreadyExists);
        }
        // Parent directory must exist (except for root itself).
        if path != "/" {
            let parent = parent_path(path).ok_or(Error::InvalidPath)?;
            if !self.nodes.iter().any(|n| n.matches(parent) && n.kind == NodeKind::Directory)
            {
                return Err(Error::NotFound);
            }
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
        node.kind = NodeKind::File;
        node.occupied = true;
        Ok(index)
    }

    fn mkdir(&mut self, path: &str) -> Result<usize, Error> {
        validate_absolute_path(path)?;
        if path == "/" {
            return Err(Error::AlreadyExists);
        }
        if self.nodes.iter().any(|node| node.matches(path)) {
            return Err(Error::AlreadyExists);
        }
        let parent = parent_path(path).ok_or(Error::InvalidPath)?;
        if !self
            .nodes
            .iter()
            .any(|n| n.matches(parent) && n.kind == NodeKind::Directory)
        {
            return Err(Error::NotFound);
        }
        let index = self
            .nodes
            .iter()
            .position(|node| !node.occupied)
            .ok_or(Error::Full)?;
        let node = &mut self.nodes[index];
        *node = Node::directory(path.as_bytes());
        // directory() copies from a slice; path may be longer than what const fn saw.
        node.path[..path.len()].copy_from_slice(path.as_bytes());
        node.path_length = path.len();
        Ok(index)
    }

    fn count(&self) -> usize {
        self.nodes.iter().filter(|node| node.occupied).count()
    }

    fn remove(&mut self, path: &str) -> Result<(), Error> {
        if path == "/" {
            return Err(Error::ReadOnly);
        }
        let index = self
            .nodes
            .iter()
            .position(|node| node.matches(path))
            .ok_or(Error::NotFound)?;
        let node = &self.nodes[index];
        // Refuse to remove non-empty directories.
        if node.kind == NodeKind::Directory {
            let dir_path = node.path_str();
            for other in self.nodes.iter().filter(|n| n.occupied) {
                if immediate_child_name(dir_path, other.path_str()).is_some() {
                    return Err(Error::Full); // reuse as "not empty"
                }
            }
        }
        self.nodes[index] = Node::empty();
        Ok(())
    }

    fn rename(&mut self, old: &str, new: &str) -> Result<(), Error> {
        validate_absolute_path(old)?;
        validate_absolute_path(new)?;
        if old == "/" || new == "/" {
            return Err(Error::ReadOnly);
        }
        if self.nodes.iter().any(|n| n.matches(new)) {
            return Err(Error::AlreadyExists);
        }
        let index = self
            .nodes
            .iter()
            .position(|n| n.matches(old))
            .ok_or(Error::NotFound)?;
        let node = &mut self.nodes[index];
        let bytes = new.as_bytes();
        if bytes.len() >= PATH_CAPACITY {
            return Err(Error::Full);
        }
        node.path = [0; PATH_CAPACITY];
        node.path[..bytes.len()].copy_from_slice(bytes);
        node.path_length = bytes.len();
        Ok(())
    }

    fn write_file(&mut self, path: &str, data: &[u8]) -> Result<(), Error> {
        validate_absolute_path(path)?;
        if data.len() > NODE_CAPACITY {
            return Err(Error::Full);
        }
        if let Some(index) = self.nodes.iter().position(|n| n.matches(path)) {
            let node = &mut self.nodes[index];
            if node.kind != NodeKind::File {
                return Err(Error::AlreadyExists);
            }
            if !node.writable {
                return Err(Error::ReadOnly);
            }
            node.data[..data.len()].copy_from_slice(data);
            node.length = data.len();
            return Ok(());
        }
        // Create new writable file (parent must exist).
        self.insert(path, data, true).map(|_| ())
    }

    fn stat(&self, path: &str) -> Result<Stat, Error> {
        let node = self
            .nodes
            .iter()
            .find(|node| node.matches(path))
            .ok_or(Error::NotFound)?;
        Ok(Stat {
            kind: node.kind,
            size: node.length,
            writable: node.writable,
        })
    }

    /// Return the `index`-th direct child of `dir_path` (0-based).
    fn readdir(&self, dir_path: &str, index: usize) -> Result<DirEntry, Error> {
        let dir = self
            .nodes
            .iter()
            .find(|node| node.matches(dir_path) && node.kind == NodeKind::Directory)
            .ok_or(Error::NotFound)?;
        let dir_path = dir.path_str();

        let mut seen = 0usize;
        for node in self.nodes.iter().filter(|n| n.occupied) {
            let child_path = node.path_str();
            let Some(name) = immediate_child_name(dir_path, child_path) else {
                continue;
            };
            if seen == index {
                let mut entry = DirEntry {
                    name: [0; MAX_DIR_NAME],
                    name_length: 0,
                    kind: node.kind,
                };
                let len = core::cmp::min(name.len(), MAX_DIR_NAME);
                entry.name[..len].copy_from_slice(name.as_bytes());
                entry.name_length = len;
                return Ok(entry);
            }
            seen += 1;
        }
        Err(Error::NotFound)
    }
}

fn validate_absolute_path(path: &str) -> Result<(), Error> {
    if path.is_empty() || !path.starts_with('/') || path.len() > PATH_CAPACITY {
        return Err(Error::InvalidPath);
    }
    if path.as_bytes().contains(&0) {
        return Err(Error::InvalidPath);
    }
    // Reject trailing slash except for root.
    if path.len() > 1 && path.ends_with('/') {
        return Err(Error::InvalidPath);
    }
    Ok(())
}

fn parent_path(path: &str) -> Option<&str> {
    if path == "/" {
        return None;
    }
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(0) => Some("/"),
        Some(pos) => Some(&trimmed[..pos]),
        None => None,
    }
}

/// If `child` is an immediate child of `dir`, return the final path component.
fn immediate_child_name<'a>(dir: &str, child: &'a str) -> Option<&'a str> {
    if child == dir {
        return None;
    }
    if dir == "/" {
        if !child.starts_with('/') || child.len() < 2 {
            return None;
        }
        let rest = &child[1..];
        if rest.contains('/') {
            return None;
        }
        return Some(rest);
    }
    if !child.starts_with(dir) {
        return None;
    }
    let rest = &child[dir.len()..];
    if !rest.starts_with('/') {
        return None;
    }
    let name = &rest[1..];
    if name.is_empty() || name.contains('/') {
        return None;
    }
    Some(name)
}

static REGISTRY: Mutex<Registry> = Mutex::new(Registry::boot());

/// Handle to a shared open-file description.
///
/// Process file-descriptor tables store these IDs. Multiple descriptors
/// (including across fork) can refer to the same description so that the
/// file offset is shared, matching POSIX semantics.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct OpenFileId(usize);

struct OpenFileDescription {
    node: usize,
    offset: usize,
    refcount: u32,
    occupied: bool,
}

impl OpenFileDescription {
    const fn empty() -> Self {
        Self {
            node: 0,
            offset: 0,
            refcount: 0,
            occupied: false,
        }
    }
}

struct OpenFileTable {
    entries: [OpenFileDescription; MAX_OPEN_FILES],
}

impl OpenFileTable {
    const fn empty() -> Self {
        Self {
            entries: [const { OpenFileDescription::empty() }; MAX_OPEN_FILES],
        }
    }

    fn alloc(&mut self, node: usize) -> Result<OpenFileId, Error> {
        let slot = self
            .entries
            .iter()
            .position(|entry| !entry.occupied)
            .ok_or(Error::Full)?;
        self.entries[slot] = OpenFileDescription {
            node,
            offset: 0,
            refcount: 1,
            occupied: true,
        };
        Ok(OpenFileId(slot))
    }

    fn get_mut(&mut self, id: OpenFileId) -> Result<&mut OpenFileDescription, Error> {
        self.entries
            .get_mut(id.0)
            .filter(|entry| entry.occupied && entry.refcount > 0)
            .ok_or(Error::InvalidDescriptor)
    }

    fn clone_id(&mut self, id: OpenFileId) -> Result<OpenFileId, Error> {
        let entry = self.get_mut(id)?;
        entry.refcount = entry.refcount.saturating_add(1);
        Ok(id)
    }

    fn drop_id(&mut self, id: OpenFileId) -> Result<(), Error> {
        let entry = self
            .entries
            .get_mut(id.0)
            .filter(|entry| entry.occupied)
            .ok_or(Error::InvalidDescriptor)?;
        if entry.refcount == 0 {
            return Err(Error::InvalidDescriptor);
        }
        entry.refcount -= 1;
        if entry.refcount == 0 {
            *entry = OpenFileDescription::empty();
        }
        Ok(())
    }

    fn live_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.occupied)
            .count()
    }
}

static OPEN_FILES: Mutex<OpenFileTable> = Mutex::new(OpenFileTable::empty());

/// Backwards-compatible alias used by older call sites during the transition.
/// Prefer `OpenFileId` for new code.
pub type OpenFile = OpenFileId;

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

/// Create or overwrite a writable file.
pub fn write_file(path: &str, data: &[u8]) -> Result<(), Error> {
    REGISTRY.lock().write_file(path, data)
}

/// Remove a file or empty directory.
pub fn remove(path: &str) -> Result<(), Error> {
    REGISTRY.lock().remove(path)
}

pub fn rename(old: &str, new: &str) -> Result<(), Error> {
    REGISTRY.lock().rename(old, new)
}

/// Create a directory. Parent must already exist.
pub fn mkdir(path: &str) -> Result<(), Error> {
    REGISTRY.lock().mkdir(path).map(|_| ())
}

/// Query metadata for a path.
pub fn stat(path: &str) -> Result<Stat, Error> {
    REGISTRY.lock().stat(path)
}

/// Read the `index`-th entry of a directory (0-based). Returns `NotFound` when exhausted.
pub fn readdir(path: &str, index: usize) -> Result<DirEntry, Error> {
    REGISTRY.lock().readdir(path, index)
}

/// Open a **file** path and return a new open-file description (refcount = 1).
/// Directories cannot be opened for read/write; use `readdir` / `stat` instead.
pub fn open(path: &str) -> Result<OpenFileId, Error> {
    let registry = REGISTRY.lock();
    let node = registry
        .nodes
        .iter()
        .position(|node| node.matches(path) && node.kind == NodeKind::File)
        .ok_or(Error::NotFound)?;
    drop(registry);
    OPEN_FILES.lock().alloc(node)
}

/// Increase the reference count of an existing open-file description.
/// Used by fork (and future dup) so parent and child share the offset.
pub fn clone_open_file(id: OpenFileId) -> Result<OpenFileId, Error> {
    OPEN_FILES.lock().clone_id(id)
}

/// Decrease the reference count. Frees the description when it reaches zero.
pub fn close_open_file(id: OpenFileId) -> Result<(), Error> {
    OPEN_FILES.lock().drop_id(id)
}

pub fn read(id: OpenFileId, buffer: &mut [u8]) -> Result<usize, Error> {
    let mut table = OPEN_FILES.lock();
    let entry = table.get_mut(id)?;
    let node_index = entry.node;
    let offset = entry.offset;

    let registry = REGISTRY.lock();
    let node = registry
        .nodes
        .get(node_index)
        .filter(|node| node.occupied)
        .ok_or(Error::InvalidDescriptor)?;
    if offset > node.length {
        return Err(Error::InvalidDescriptor);
    }
    let count = core::cmp::min(node.length - offset, buffer.len());
    buffer[..count].copy_from_slice(&node.data[offset..offset + count]);
    drop(registry);

    let entry = table.get_mut(id)?;
    entry.offset = offset + count;
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

pub fn seek(id: OpenFileId, offset: usize) -> Result<usize, Error> {
    let mut table = OPEN_FILES.lock();
    let entry = table.get_mut(id)?;
    let registry = REGISTRY.lock();
    let node = registry.nodes.get(entry.node).filter(|n| n.occupied).ok_or(Error::NotFound)?;
    let max = node.length;
    drop(registry);
    // Allow seek to end (offset == length) for append; not past EOF beyond length for simplicity.
    let pos = core::cmp::min(offset, max);
    entry.offset = pos;
    Ok(pos)
}

pub fn write(id: OpenFileId, buffer: &[u8]) -> Result<usize, Error> {
    let mut table = OPEN_FILES.lock();
    let entry = table.get_mut(id)?;
    let node_index = entry.node;
    let offset = entry.offset;

    let mut registry = REGISTRY.lock();
    let node = registry
        .nodes
        .get_mut(node_index)
        .filter(|node| node.occupied)
        .ok_or(Error::InvalidDescriptor)?;
    if !node.writable {
        return Err(Error::ReadOnly);
    }
    if offset > node.length {
        return Err(Error::InvalidDescriptor);
    }
    let count = core::cmp::min(buffer.len(), NODE_CAPACITY - offset);
    node.data[offset..offset + count].copy_from_slice(&buffer[..count]);
    let new_offset = offset + count;
    node.length = core::cmp::max(node.length, new_offset);
    let short_write = count < buffer.len();
    drop(registry);

    let entry = table.get_mut(id)?;
    entry.offset = new_offset;
    if short_write {
        return Err(Error::Full);
    }
    Ok(count)
}

/// Invoke `f` for every occupied file node whose path starts with `prefix`.
/// Directories are skipped. Intended for storage layer discovery under `/mnt`.
pub fn for_each_file_with_prefix<F>(prefix: &str, mut f: F)
where
    F: FnMut(&str, &[u8]),
{
    let registry = REGISTRY.lock();
    for node in registry.nodes.iter() {
        if !node.occupied || node.kind != NodeKind::File {
            continue;
        }
        let path = node.path_str();
        if path.starts_with(prefix) {
            f(path, &node.data[..node.length]);
        }
    }
}

pub fn node_count() -> usize {
    REGISTRY.lock().count()
}

pub fn open_file_description_count() -> usize {
    OPEN_FILES.lock().live_count()
}

pub fn self_test() -> bool {
    let mut scratch = Registry::empty();
    // Parent directories required before inserting files.
    let root_ok = scratch.mkdir("/").is_err(); // root cannot be created via mkdir on empty
    scratch.nodes[0] = Node::directory(b"/");
    let mnt_ok = scratch.mkdir("/mnt") == Ok(1);
    let inserted = scratch.insert("/mnt/test.txt", b"mounted", false).is_ok();
    let duplicate = scratch.insert("/mnt/test.txt", b"again", false) == Err(Error::AlreadyExists);
    let invalid_path = scratch.insert("relative", b"bad", false) == Err(Error::InvalidPath);
    let no_parent = scratch.insert("/missing/file", b"x", false) == Err(Error::NotFound);

    // Directory listing on the live registry.
    let root_stat = matches!(stat("/"), Ok(Stat { kind: NodeKind::Directory, .. }));
    let etc_stat = matches!(stat("/etc"), Ok(Stat { kind: NodeKind::Directory, .. }));
    let file_stat = matches!(
        stat("/etc/motd"),
        Ok(Stat {
            kind: NodeKind::File,
            size: 24,
            writable: false,
        })
    );

    // / should contain at least etc and tmp.
    let mut found_etc = false;
    let mut found_tmp = false;
    let mut index = 0;
    while let Ok(entry) = readdir("/", index) {
        if entry.name_str() == "etc" {
            found_etc = true;
        }
        if entry.name_str() == "tmp" {
            found_tmp = true;
        }
        index += 1;
        if index > 16 {
            break;
        }
    }
    let readdir_ok = found_etc && found_tmp;

    let Ok(writer) = open("/tmp/vfs-self-test") else {
        return false;
    };
    let payload = b"wovenhat-vfs";
    if write(writer, payload) != Ok(payload.len()) {
        let _ = close_open_file(writer);
        return false;
    }

    // Shared offset: cloning the description must observe the advanced offset.
    let Ok(shared) = clone_open_file(writer) else {
        let _ = close_open_file(writer);
        return false;
    };
    let mut probe = [0; 4];
    let shared_offset_advanced = read(shared, &mut probe) == Ok(0);

    // Independent open starts at offset 0 again.
    let Ok(reader) = open("/tmp/vfs-self-test") else {
        let _ = close_open_file(shared);
        let _ = close_open_file(writer);
        return false;
    };
    let mut buffer = [0; 12];
    let round_trip = read(reader, &mut buffer) == Ok(payload.len()) && buffer == *payload;

    let Ok(protected) = open("/etc/motd") else {
        let _ = close_open_file(reader);
        let _ = close_open_file(shared);
        let _ = close_open_file(writer);
        return false;
    };
    let mut complete = [0; 32];
    let read_only_ok = write(protected, b"x") == Err(Error::ReadOnly)
        && read_all("/etc/motd", &mut complete) == Ok(24)
        && &complete[..24] == b"Welcome to WovenHat OS.\n";

    // Directories cannot be opened as files.
    let dir_open_rejected = open("/etc").is_err();

    let _ = close_open_file(protected);
    let _ = close_open_file(reader);
    let _ = close_open_file(shared);
    let _ = close_open_file(writer);

    // After closing all clones the description table should release the slots.
    let cleaned = open_file_description_count() == 0;

    // Boot registry: /, /etc, /tmp, motd, version, vfs-self-test
    let boot_nodes = node_count() == 8;

    root_ok
        && mnt_ok
        && inserted
        && duplicate
        && invalid_path
        && no_parent
        && root_stat
        && etc_stat
        && file_stat
        && readdir_ok
        && shared_offset_advanced
        && round_trip
        && read_only_ok
        && dir_open_rejected
        && cleaned
        && boot_nodes
}
