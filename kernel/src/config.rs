//! Central compile-time configuration and resource limits for WovenHat.
//!
//! Raising these values is the first step of Phase A (scalability).
//! Keep them power-of-two friendly where it helps alignment, but the
//! primary goal is to remove the artificial 8-slot ceilings that
//! currently prevent real multi-process workloads.

/// Maximum number of tasks the scheduler can track (including kernel + idle).
pub const MAX_TASKS: usize = 32;

/// Maximum number of processes (user + kernel).
pub const MAX_PROCESSES: usize = 32;

/// Maximum open file descriptors per process.
/// Descriptors 0, 1, 2 are reserved for stdin / stdout / stderr.
pub const MAX_FILE_DESCRIPTORS: usize = 16;

/// Kernel stack size reserved for each task (bytes).
pub const TASK_STACK_SIZE: usize = 4096 * 2;

/// Maximum IPC endpoints (one per process is typical).
pub const MAX_IPC_ENDPOINTS: usize = 32;

/// Maximum messages queued on a single IPC endpoint.
pub const IPC_QUEUE_DEPTH: usize = 16;

/// Maximum payload size of an IPC message (bytes).
pub const MAX_MESSAGE_SIZE: usize = 256;

/// Maximum path length accepted by VFS / syscalls (bytes).
pub const MAX_PATH_SIZE: usize = 128;

/// Maximum size of a single read/write syscall buffer (bytes).
pub const MAX_IO_SIZE: usize = 1024;

/// Maximum VFS nodes (files) that can exist simultaneously.
pub const MAX_VFS_NODES: usize = 64;

/// Capacity of each VFS node data buffer (bytes).
pub const VFS_NODE_CAPACITY: usize = 64 * 1024;

/// Maximum ELF loadable segments accepted by the loader.
pub const MAX_ELF_SEGMENTS: usize = 8;

/// Maximum anonymous (mmap) mappings per process.
pub const MAX_ANONYMOUS_MAPPINGS: usize = 16;

/// Maximum devices registered in the device table.
pub const MAX_DEVICES: usize = 32;

/// Maximum allocations tracked by the simple kernel heap.
pub const MAX_HEAP_ALLOCATIONS: usize = 512;

/// Global open-file description table capacity (refcount-shared across processes).
pub const MAX_OPEN_FILES: usize = 64;
