# Userspace ABI

## Syscall entry

- Interrupt vector `0x80` (DPL 3)
- Arguments: `RAX` = call number, `RDI`, `RSI`, `RDX`
- Return: `RAX` (or `u64::MAX` on error)

See [syscall-api.md](syscall-api.md) for the full table including `stat` (17),
`readdir` (18), and `mkdir` (19).

## Initial process stack (argc / argv)

When the kernel loads an ELF via `load_elf` / `load_elf_with_argv` / `exec`, it
builds a System V-style argument area on the user stack:

```text
high addresses
  argv string bytes (NUL-terminated)
  padding to 16-byte alignment
  NULL                 ← argv[argc]
  argv[argc-1]
  ...
  argv[0]
  argc                 ← RSP at entry
low addresses
```

A minimal crt0:

```asm
.global _start
_start:
    mov rdi, [rsp]          # argc
    lea rsi, [rsp + 8]      # argv
    # call main(argc, argv)  — language runtime specific
    mov eax, 3              # exit
    mov rdi, 0
    int 0x80
```

`exec("/bin/sh")` sets `argv[0]` to the path string. `load_elf` defaults to
`argv = ["a.out"]`.

## Capabilities and credentials

New processes start as UID/GID 1000 with the restricted userspace capability set.
File and process syscalls enforce capabilities at the kernel boundary.

## Memory

- User region begins at `0x0000_4000_0000_0000`
- Stack is placed near the top of that region with a guard page
- `mmap` / `munmap` manage anonymous mappings
- After `fork`, writable pages are copy-on-write until first write
