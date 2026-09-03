# Userspace Syscall ABI

WovenHat exposes interrupt vector 0x80 to ring 3. Arguments use RDI, RSI, and RDX;
RAX selects the call and receives either a non-negative result or the all-ones error
sentinel. The assembly entry preserves general registers and returns with iretq.

| RAX | Call | Arguments |
| ---: | --- | --- |
| 0 | read | fd, user buffer, length |
| 1 | write | stdout/stderr fd, user buffer, length |
| 2 | open | user path, path length |
| 3 | exit | status |
| 4 | getpid | none |
| 5 | waitpid | child PID |
| 6 | close | fd |
| 7 | yield | none |
| 8 | mmap | length, writable flag |
| 9 | munmap | address, length |
| 10 | file_write | fd, user buffer, length |
| 11 | message_send | receiver PID, user buffer, length |
| 12 | message_receive | user buffer, capacity, sender output pointer |
| 13 | getuid | none |
| 14 | getgid | none |
| 15 | exec | user path, path length |
| 16 | fork | none |
| 17 | stat | user path, path length → packed kind/size/writable |
| 18 | readdir | user path, path_len|(index<<16), user name buffer → name_len|(kind<<8) |
| 19 | mkdir | user path, path length |
| 20 | chdir | user path, path length |
| 21 | getcwd | user buffer, capacity → length |
| 22 | dup | fd → new fd |
| 23 | pipe | → read_fd | (write_fd≪32) |
| 24 | dup2 | oldfd, newfd |
| 25 | getppid | |
| 26 | kill | pid, sig (0/9/15) |

Descriptor 0 reads the nonblocking PS/2 byte stream; descriptors 1 and 2 write to COM1. The reserved standard descriptors cannot be closed, and VFS handles begin at 3. See [standard streams](standard-streams.md).

All paths and I/O payloads have fixed upper bounds. Pointer-bearing calls translate and
validate each user page before copying. File and IPC calls additionally pass capability,
credential, descriptor, and VFS/queue checks.

The embedded ring-3 validation program exercises fork, exec, write, open, read, close, mmap,
munmap, yield, getpid, getuid, getgid, and exit. Boot also validates waitpid and process
reclamation from the kernel parent.

`exec` requires FileRead and ProcessCreate. The kernel copies the bounded path, reads and
validates the complete ELF into a fresh address space, and only then commits the process
and task records. It switches CR3 before reclaiming the previous image and anonymous
mappings, preserves the process ID, credentials, capabilities, and open descriptors, and
enters the new image directly. A failed load returns the error sentinel without changing
the caller.

`fork` requires ProcessCreate. It deep-copies the executable segments, user stack, and
anonymous mappings into a distinct CR3 root before publishing the child. The child
inherits the parent's credentials, capabilities, and descriptor snapshots. The syscall
returns the child PID to the parent and zero to the child by resuming the copied register
frame through the common interrupt-return epilogue. If cloning or table publication
fails, no child is exposed and the parent receives the error sentinel.

The boot fixture checks both return paths, child exit status 42, parent wait/reap, and
subsequent process-image cleanup. Fork and exec use bounded process, mapping, ELF, and
descriptor tables and do not overcommit memory.

## Current boundary

Fork uses copy-on-write page sharing; open-file descriptions are reference-counted so
offsets are shared across parent and child. There is no `argv`/environment transfer yet.
Directories are first-class VFS nodes; `stat`, `readdir`, and `mkdir` are available.
