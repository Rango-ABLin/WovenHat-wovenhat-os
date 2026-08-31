# WovenHat OS Improvement Roadmap

## Current state

This kernel already has a strong x86_64 foundation:

- bootloader entry and framebuffer initialization in [kernel/src/main.rs](../kernel/src/main.rs)
- GDT/TSS initialization and double-fault handling in [kernel/src/gdt.rs](../kernel/src/gdt.rs)
- physical memory frame allocation in [kernel/src/memory.rs](../kernel/src/memory.rs)
- paging setup and mapping validation in [kernel/src/paging.rs](../kernel/src/paging.rs)
- interrupt routing and handlers in [kernel/src/interrupts.rs](../kernel/src/interrupts.rs)
- task scheduling and capability-based access in [kernel/src/task.rs](../kernel/src/task.rs)
- keyboard + shell interaction in [kernel/src/keyboard.rs](../kernel/src/keyboard.rs) and [kernel/src/shell.rs](../kernel/src/shell.rs)

That means the project is far beyond a minimal boot stub. The right direction is to evolve it into a real OS architecture instead of continuing with prototype-only features.

---

## Primary goal

Turn WovenHat into a secure, usable operating system with:

- preemptive multitasking
- user mode and syscall interface
- proper memory isolation
- a filesystem abstraction
- drivers and device model
- recovery and diagnostics for faults

---

## Recommended roadmap

### Phase 1: Stabilize the kernel foundation

This phase should improve reliability before adding major features.

#### Goals

- improve panic handling and crash reporting
- add structured kernel logging
- support CPU feature detection and safe boot validation
- harden memory management and page-fault handling
- create a better debug/diagnostic story

#### Tasks

1. Replace the current hard-stop panic behavior in [kernel/src/main.rs](../kernel/src/main.rs) with a structured kernel panic path that prints:
   - RIP/RSP/CR2
   - registers and flags
   - stack trace information when available
   - an optional reboot or halt policy

2. Upgrade [kernel/src/interrupts.rs](../kernel/src/interrupts.rs) to support:
   - better exception classification
   - panic context capture
   - recoverable fault handling where appropriate
   - logging to serial and framebuffer

3. Extend [kernel/src/memory.rs](../kernel/src/memory.rs) and [kernel/src/paging.rs](../kernel/src/paging.rs) with:
   - kernel heap allocation
   - page-fault recovery logic
   - guard pages and protection metadata
   - a separate user-space mapping model

4. Add ACPI/PCI discovery and board feature detection so drivers are not hard-coded to one environment.

#### Definition of done

- the kernel does not silently freeze on unknown faults
- errors are diagnosable from serial output
- memory state is better tracked and validated
- boot is reproducible across environment changes

---

### Phase 2: Real multitasking and process abstraction

The existing scheduler in [kernel/src/task.rs](../kernel/src/task.rs) is a great starting point, but it is still cooperative and simplistic.

#### Goals

- add preemptive scheduling
- create a true task/process model
- support waiting, blocking, and wakeups
- separate kernel tasks from user tasks

#### Tasks

1. Replace the current cooperative switch pattern with timer-driven preemption.
2. Introduce task states:
   - new
   - ready
   - running
   - blocked
   - sleeping
   - dead
3. Add per-process memory context and a process control block.
4. Add process IDs, parent-child relationships, and process exit handling.
5. Add a scheduler policy:
   - round robin
   - priority-based scheduling
   - time slices

#### Definition of done

- multiple tasks can run without explicit yield calls
- blocked tasks do not starve the scheduler
- each task has its own lifecycle and state transitions
- user processes can be created and scheduled cleanly

---

### Phase 3: User mode and syscall interface

This is the biggest missing milestone. Without user mode, WovenHat remains a kernel test harness instead of an OS.

#### Goals

- move code into ring 3 user space
- expose a minimal syscall ABI
- create userspace process execution
- support privilege boundaries

#### Tasks

1. Add a user-mode task setup in the GDT/segmentation layer.
2. Build a syscall entry point and IDT handlers for system calls.
3. Define a minimal ABI:
   - read
   - write
   - open
   - close
   - fork/exec
   - exit
   - mmap
   - yield
4. Add a userspace ELF loader.
5. Ensure kernel permissions are not trivially bypassed by user tasks.

#### Suggested initial syscall set

- `sys_write`
- `sys_read`
- `sys_open`
- `sys_close`
- `sys_fork`
- `sys_exec`
- `sys_exit`
- `sys_yield`
- `sys_getpid`

#### Definition of done

- the kernel can start a user task in ring 3
- syscall entry works from user mode
- user code cannot directly access kernel memory or hardware

---

### Phase 4: Filesystem and device layers

A usable OS needs a virtual filesystem and a clean device abstraction.

#### Goals

- expose storage and files to userspace
- abstract device access behind a common interface
- keep shell commands and apps independent from hardware details

#### Tasks

1. Introduce a VFS layer.
2. Implement a minimal read/write filesystem or mountable FAT/EXT-like system.
3. Add file descriptors and standard streams.
4. Add device drivers for:
   - console
   - keyboard
   - timer
   - serial
   - block devices when available
5. Build a simple device registry for IRQ and driver registration.

#### Definition of done

- processes can read and write files through the kernel API
- drivers are not hard-coded into shell behavior
- basic resource isolation for devices exists

---

### Phase 5: Security and reliability maturity

The capability model in [kernel/src/task.rs](../kernel/src/task.rs) is a good sign, but it should be expanded into a more robust security model.

#### Goals

- privilege separation
- validation on all kernel interfaces
- hardening against invalid pointers and corrupted state

#### Tasks

1. Add user IDs and group checks.
2. Validate pointers and buffers in every syscall boundary.
3. Use stack guards and kernel page protections.
4. Introduce privilege checks for device I/O and memory access.
5. Add kernel audit logs for privileged actions.

#### Definition of done

- unprivileged tasks cannot access kernel memory or privileged hardware
- invalid syscall arguments fail safely and deterministically
- the system remains stable under malformed inputs

---

### Phase 6: Performance and ecosystem

Once the OS is stable, optimize and expand usability.

#### Goals

- better performance and throughput
- cleaner developer workflows
- easier experimentation and testing

#### Tasks

1. Add benchmarking hooks for scheduler and memory performance.
2. Add QEMU-based CI validation.
3. Build a small userland test suite.
4. Add documentation for kernel architecture and build flow.
5. Support a standard shell, utilities, and app launcher.

---

## Best next milestone for this repo

The most important next milestone is not “more commands.” It is:

1. preemptive scheduling
2. user mode + syscalls
3. a basic filesystem abstraction

If those three are completed well, this project becomes a real operating system instead of a kernel demo platform.

---

## Recommended order of implementation

1. Improve fault handling in [kernel/src/interrupts.rs](../kernel/src/interrupts.rs)
2. Build a process model in [kernel/src/task.rs](../kernel/src/task.rs)
3. Add user-mode setup and kernel entry transitions
4. Create syscall handlers and a minimal ABI
5. Add a VFS and device registry
6. Expand the shell into a real command environment

---

## Suggested architecture direction

A good direction for this codebase is a hybrid kernel or microkernel-inspired design:

- kernel handles memory, scheduling, interrupt routing, and protection
- drivers run as isolated services where possible
- shell and utilities run in userspace
- inter-process communication is explicit and capability-based

This fits the current design philosophy and scales better than a monolithic “everything in kernel space” model.

---

## Final recommendation

The code already shows strong engineering instincts. The next move should be to stop optimizing the boot splash and start building the OS model that makes the system useful.

The best OS version for this repository would be:

- stable kernel
- preemptive scheduler
- user mode with syscalls
- filesystem abstraction
- driver model
- security boundaries

This is the clearest path from a strong prototype to a genuinely impressive OS.
