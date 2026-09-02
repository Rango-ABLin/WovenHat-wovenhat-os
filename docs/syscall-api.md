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

All paths and I/O payloads have fixed upper bounds. Pointer-bearing calls translate and
validate each user page before copying. File and IPC calls additionally pass capability,
credential, descriptor, and VFS/queue checks.

The embedded ring-3 validation program exercises exec, write, open, read, close, mmap,
munmap, yield, getpid, getuid, getgid, and exit. Boot also validates waitpid and process
reclamation from the kernel parent.

`exec` requires FileRead and ProcessCreate. The kernel copies the bounded path, reads and
validates the complete ELF into a fresh address space, and only then commits the process
and task records. It switches CR3 before reclaiming the previous image and anonymous
mappings, preserves the process ID, credentials, capabilities, and open descriptors, and
enters the new image directly. A failed load returns the error sentinel without changing
the caller.

## Current boundary

Fork is not yet implemented. Process creation currently occurs through the validated
kernel ELF-loading path, and every new userspace process receives the default
unprivileged credentials and capability set. Adding fork requires copy-on-write or a
bounded address-space and saved-register clone.
