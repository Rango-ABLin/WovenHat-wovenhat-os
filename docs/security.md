# Kernel Security Model

WovenHat combines per-task capabilities with process credentials. Kernel execution uses
UID 0 and GID 0. Every newly loaded ring-3 process starts with UID 1000 and GID 1000
and the restricted userspace capability set; this prevents a kernel-created process from
implicitly inheriting root identity.

## Authorization

- Capabilities gate console output, file access, IPC, process creation, task control, device I/O,
  interrupt control, and memory inspection.
- Capability grant requires both TaskControl and the capability being delegated.
- Capability revoke requires TaskControl.
- IPC send requires the Ipc capability and either root identity, a matching UID, or a
  matching GID.
- File writes require FileWrite and are also constrained by the VFS node write policy.
- Fork and exec require ProcessCreate; exec also requires FileRead.
- Syscall user pointers are translated page by page and rejected when unmapped or when
  the requested access conflicts with page permissions.

Boot validates root override, same-UID access, same-GID access, cross-identity denial,
capability grant/revoke behavior, and the credentials of two real ring-3 processes.

## Audit ring

A bounded 64-record kernel audit ring stores a monotonic sequence number, timer tick,
actor process ID, action, target, and allow/deny result. Capability grant/revoke, IPC
send, file-write, process-fault, fork, and exec outcomes are recorded. When full, the ring overwrites its oldest
record instead of allocating or blocking. Boot verifies wraparound and confirms the
capability delegation test produced ordered audit evidence.

## Current boundary

Credentials currently contain one UID and one primary GID; supplementary groups,
credential-changing syscalls, executable ownership metadata, and persistent audit
storage are not implemented. Audit records remain kernel-resident and are not yet
exposed to unprivileged processes.
