# Userspace ABI

## Overview

WovenHat userspace is ring-3 code entered via `iretq` with a System V-style
stack. Programs are ELF images loaded by the kernel (`load_elf` /
`load_elf_with_argv`) or by `exec`.

## Syscall entry

- Vector: `int 0x80` (DPL 3)
- Args: `RAX` = number, `RDI`, `RSI`, `RDX`
- Return: `RAX` (`u64::MAX` on error)

| # | Name | Notes |
|---|------|--------|
| 0 | read | fd 0 = keyboard (non-blocking) |
| 1 | write | fd 1/2 = serial console |
| 2 | open | path, path_len → fd |
| 3 | exit | status |
| 4 | getpid | |
| 5 | waitpid | child pid; `-2` still running |
| 6 | close | |
| 7 | yield | |
| 8 | mmap | length, writable |
| 9 | munmap | |
| 10 | file_write | fd, buf, len |
| 11–12 | message_send / receive | IPC |
| 13–14 | getuid / getgid | |
| 15 | exec | path, path_len (replaces image) |
| 16 | fork | parent→child pid, child→0 |
| 17 | stat | packed metadata |
| 18 | readdir | path, len\|(index≪16), name buf |
| 19 | mkdir | |
| 20 | chdir | |
| 21 | getcwd | |
| 22 | dup | |
| 23 | pipe | read\|(write≪32) |
| 24 | dup2 | |
| 25 | getppid | |
| 26 | kill | 0/9/15 |

## Initial stack

```text
  [ argv strings ]
  NULL          ← envp terminator (empty env)
  NULL          ← argv terminator
  argv[n-1] … argv[0]
  argc          ← RSP
```

## Default capabilities (userspace)

Console, FileRead, FileWrite, Ipc, ProcessCreate.

## Installed programs

| Path | Role |
|------|------|
| `/bin/sh` | Interactive shell |
| `/bin/init` | Banner then `exec /bin/sh` |
| `/bin/selftest` | Boot / regression self-test |

## Shell builtins (`/bin/sh`)

`help`, `echo`, `cat`, `ls`, `mkdir`, `cd`, `pwd`, `exit`/`quit`.  
Any other line is treated as a path: `fork` → `exec` → parent `waitpid`.

## Boot flow

After kernel self-tests, boot schedules `init`, which replaces itself with
`/bin/sh`. The kernel diagnostic shell remains available via F1.
