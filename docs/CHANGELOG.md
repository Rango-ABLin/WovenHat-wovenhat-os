# WovenHat OS — Changelog

Consolidates the incident write-ups previously scattered across
`docs/STAGE*-FIX.md` and `docs/STAGE9_BUILD_FIX.md`. Full original text for
any entry is preserved in `docs/archive/` if you need the complete
diagnostic detail; this file keeps the summary that matters going forward.

## Stage 9 — standard file-descriptor stability fix
Every process started with an empty descriptor table, so the first
successful `open()` was assigned descriptor 0 — the slot `read(0)`
hard-codes as keyboard stdin. A program like `/bin/cat` would then read
from the keyboard instead of the file it opened, and appeared to hang or
silently eat shell keystrokes. Fixed by reserving descriptor 0 for stdin
at process creation instead of leaving it assignable.

## Stage 9 — shell startup fix
`/bin/sh` called the mmap syscall before printing its banner or prompt, so
if the anonymous-mapping path was slow or stalled, `sh` looked stuck at
"starting userspace shell..." even though other Ring-3 commands worked
fine. Fixed by having the shell carve its scratch buffer out of its
already-mapped stack instead of a separate mmap call before first output.

## Stage 9 — runtime console / userspace-shell fix
Two related issues: (1) `cmd_sh()` synchronously waited for the shell
process to exit from within the kernel's main event loop, blocking the
whole kernel while the shell ran interactively; (2) a related console
ownership issue during that same wait. Fixed by making the shell command
cooperate with the kernel loop instead of blocking it.

## Stage 9 — Ring-3 transition fix
`sh` cleared the screen and appeared to hang; other `/bin/*` programs
never became interactive either. Root cause: the GDT's user code/data
descriptors are DPL3, but the selectors handed to the `iretq` user-mode
transition still carried RPL0 — a CPL0→CPL3 return requires RPL3
selectors. Fixed by correcting the selector RPL bits used for the
transition.

## Stage 9 — foreground shell and explicit `/bin` path fix
`/bin/sh` was given foreground ownership but the kernel only yielded to it
once, so the kernel task could resume immediately and starve the
interactive shell. Fixed by having `cmd_sh()` repeatedly yield while the
shell process is alive, matching the pattern already used by other
working foreground commands.

## Stage 9 — command audit fix
Consolidated patch fixing: Ring-3 CS/SS selectors not using RPL3 (see
above), user processes not entering with the prepared argv stack, and
ordinary external commands being forced through fork/copy-on-write
instead of the dedicated `SpawnCommand` syscall.

## Stages 7–9 — consolidated fix
The ELF loader built a correct C-style userspace stack (`argc`/`argv`) and
stored the resulting pointer in `UserImage::stack_top`, but both the
initial spawn path and the exec path entered Ring 3 using `UserStack::top`
instead — skipping the prepared argument frame, so argument-driven
programs saw an invalid or zero `argc`. Fixed by using `stack_top`
consistently on both paths.

## Stage 9 — build fix
Compiler failures after consolidating Stages 7–9: literal `\n` tokens
left in source around `global_asm!` blocks instead of real newlines, and a
duplicate `process_count()` definition. Fixed; retained the Stage 8
process-observation implementation and the Ring-3 argv stack fix above.

## Stage 6 — clean-build fix
Addressed diagnostics surfaced after enabling `warnings = "deny"` in CI.
`network::endpoint_to_packed` used a `let ... else` on `IpAddress` where,
with only IPv4 compiled in, the pattern is irrefutable and the `else`
branch unreachable — replaced with a direct irrefutable binding.

---

*(2026-09-04 addition, not from an archived doc)* — the network stack's
`Config::random_seed` was a hardcoded constant, making TCP initial sequence
numbers predictable on every boot. Fixed by adding `kernel/src/entropy.rs`
(RDRAND-backed, documented fallback) and using it to seed the interface
config. See `docs/AUDIT-2026-09-04.md`.

*(2026-09-04 addition)* — added a boot self-test (`mmap_w_xor_x_self_test`
in `userspace.rs`) that pins down the existing W^X guarantee on anonymous
mmap: a writable mapping can never be executable. This was previously true
by inspection only.
